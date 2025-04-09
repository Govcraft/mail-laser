//! Handles the primary SMTP server logic, including connection handling,
//! optional STARTTLS negotiation, command processing via `smtp_protocol`,
//! email parsing via `email_parser`, and initiating webhook forwarding.

mod email_parser;
mod smtp_protocol;

use std::sync::Arc;
use anyhow::{Result, Context};
use log::{info, error, trace, warn};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncRead, AsyncWrite}; // Required for generic TlsStream handling
use crate::config::Config;
use crate::webhook::{WebhookClient, EmailPayload};
use smtp_protocol::{SmtpProtocol, SmtpCommandResult, SmtpState};
use email_parser::EmailParser;

// TLS related imports
use rustls::ServerConfig as RustlsServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::TlsAcceptor;
use rcgen::{generate_simple_self_signed, CertifiedKey};

/// Represents the main SMTP server instance.
///
/// Holds the application configuration and a shared `WebhookClient` instance
/// used by connection handlers to forward processed emails.
pub struct Server {
    config: Config,
    webhook_client: Arc<WebhookClient>, // Arc allows safe sharing across async tasks.
}

impl Server {
    /// Creates a new SMTP `Server` instance.
    ///
    /// Initializes the shared `WebhookClient`.
    ///
    /// # Arguments
    ///
    /// * `config` - The application configuration.
    pub fn new(config: Config) -> Self {
        // Initialize the webhook client; this might panic if certs can't load.
        let webhook_client = Arc::new(WebhookClient::new(config.clone()));
        Server {
            config,
            webhook_client,
        }
    }

    /// Runs the main SMTP server loop.
    ///
    /// Binds to the configured SMTP address and port, then enters an infinite loop
    /// accepting incoming TCP connections. Each connection is handled in a separate
    /// Tokio task via `handle_connection`.
    ///
    /// # Errors
    ///
    /// Returns an `Err` if the server fails to bind to the specified address and port.
    /// Errors during connection acceptance or handling are logged but do not terminate the server loop.
    pub async fn run(&self) -> Result<()> {
        let addr = format!("{}:{}", self.config.smtp_bind_address, self.config.smtp_port);
        // Attempt to bind the TCP listener to the configured address.
        let listener = TcpListener::bind(&addr).await
            .with_context(|| format!("Failed to bind SMTP server to {}", addr))?;
        info!("SMTP server listening on {}", addr);

        // Main server loop: continuously accept incoming connections.
        loop {
            match listener.accept().await {
                Ok((stream, remote_addr)) => {
                    info!("New connection from: {}", remote_addr);
                    // Clone Arcs for the new task. Cloning Arc is cheap.
                    let webhook_client = Arc::clone(&self.webhook_client);
                    // Clone the Vec of target emails for the new task.
                    let target_emails = self.config.target_emails.clone();
                    // Spawn a dedicated asynchronous task for each connection.
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, webhook_client, target_emails).await {
                            // Log errors from individual connection handlers.
                            // Using {:#?} includes the error source/context from anyhow.
                            error!("Error handling SMTP connection from {}: {:#?}", remote_addr, e);
                        }
                    });
                }
                Err(e) => {
                    // Log errors encountered during connection acceptance but continue loop.
                    error!("Error accepting connection: {:?}", e);
                }
            }
        }
        // This loop is infinite, so Ok(()) is never reached in normal operation.
    }
}

/// Generates a self-signed TLS certificate and private key using `rcgen`.
///
/// Used for establishing TLS sessions via STARTTLS when no certificate is configured.
/// Note: Self-signed certificates are generally not trusted by clients without manual configuration.
///
/// # Returns
///
/// A `Result` containing a tuple of the certificate and private key in DER format.
///
/// # Errors
///
/// Returns an `Err` if certificate generation fails.
fn generate_self_signed_cert() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    // Define Subject Alternative Names (SANs) - typically includes domains/IPs the cert is valid for.
    let subject_alt_names = vec!["localhost".to_string()]; // Example SAN

    // Generate the certificate and key pair.
    let certified_key: CertifiedKey = generate_simple_self_signed(subject_alt_names)
        .context("Failed to generate self-signed certificate using rcgen")?;

    // Extract the certificate and key in DER format required by rustls.
    let cert_der = certified_key.cert.der().to_vec(); // Clone bytes needed for owned CertificateDer.
    let key_der = certified_key.key_pair.serialize_der(); // Key in PKCS#8 format.

    Ok((
        CertificateDer::from(cert_der),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)) // Assume PKCS#8 format from rcgen.
    ))
}


/// Handles the initial phase of a new client connection, including potential STARTTLS negotiation.
///
/// Sends the initial greeting and processes commands like EHLO/HELO and STARTTLS.
/// If STARTTLS is successfully negotiated, it passes the upgraded TLS stream to `handle_secure_session`.
///
/// # Arguments
///
/// * `stream` - The raw TCP stream from the accepted connection.
/// * `webhook_client` - Shared `WebhookClient`.
/// * `target_emails` - The configured list of target email addresses.
///
/// # Errors
///
/// Returns `Err` if initial greeting fails, reading/processing initial commands fails,
/// or if the STARTTLS handshake fails.
async fn handle_connection(
    mut stream: TcpStream, // Mutable ownership needed for potential TLS upgrade.
    webhook_client: Arc<WebhookClient>,
    target_emails: Vec<String>,
) -> Result<()> {
    // Variables to store state during the SMTP transaction.
    // These are needed here because this function handles the full non-TLS flow.
    let mut sender = String::new();
    let mut accepted_recipient = String::new();
    let mut email_data = String::new();
    let mut collecting_data = false;

    // Scope for the initial, non-TLS protocol handler.
    // We temporarily split the stream to use BufReader/BufWriter.
    // The split borrows `stream` mutably. When the scope ends, the borrow ends,
    // and we regain full ownership of `stream` for potential TLS upgrade.
    // Scope for the protocol handler using the stream's reader/writer halves.
    // We need to ensure this scope ends cleanly or the stream is handled correctly on STARTTLS.
    // Let's restructure slightly to avoid dropping the protocol handler too early if no STARTTLS.
    let protocol_result = async {
        let (read_half, write_half) = tokio::io::split(&mut stream);
        let reader = tokio::io::BufReader::new(read_half);
        let writer = tokio::io::BufWriter::new(write_half);
        let mut initial_protocol = SmtpProtocol::new(reader, writer);

        // Send the initial 220 greeting.
        initial_protocol.send_greeting().await?;

        // Process commands for the entire session (unless STARTTLS happens).
        loop {
            trace!("SMTP({:?}): Waiting for command...", initial_protocol.get_state());
            let line = initial_protocol.read_line().await?;
            trace!("SMTP({:?}): Received line (len {}): {:?}", initial_protocol.get_state(), line.len(), line);

            // Handle EOF, except during DATA phase.
            if initial_protocol.get_state() != SmtpState::Data && line.is_empty() {
                 info!("Connection closed by client (EOF). State: {:?}", initial_protocol.get_state());
                 return Ok(()); // Clean exit for this async block
            }

            // Process the command using the state machine.
            let result = initial_protocol.process_command(&line).await?;

            match result {
                SmtpCommandResult::StartTls => {
                    // Client requested TLS upgrade.
                    info!("Client initiated STARTTLS. Proceeding with handshake.");
                    // Signal that TLS should be handled outside this block.
                    return Err(anyhow::anyhow!("STARTTLS")); // Use error to signal STARTTLS needed
                }
                SmtpCommandResult::Quit => {
                    info!("Client quit.");
                    return Ok(()); // Clean exit for this async block
                }
                SmtpCommandResult::MailFrom(email) => {
                    sender = email;
                    // State is updated internally by process_command
                },
                SmtpCommandResult::RcptTo(email) => {
                    let received_email = email; // Rename for clarity
                    // Validate recipient against the list of target emails (case-insensitive).
                    let received_email_lower = received_email.to_lowercase();
                    if target_emails.iter().any(|target| target.to_lowercase() == received_email_lower) {
                        // Store the *actual* accepted recipient address (preserving case)
                        accepted_recipient = received_email;
                        initial_protocol.write_line("250 OK").await?;
                        // State is updated internally by process_command
                    } else {
                        // Reject if not in the list.
                        initial_protocol.write_line("550 No such user here").await?;
                        // Clear any previously accepted recipient if a new, invalid one is provided.
                        accepted_recipient.clear();
                        // State remains MailFrom or RcptTo depending on previous state
                    }
                },
                SmtpCommandResult::DataStart => {
                    // Check if we have a sender and at least one valid recipient before accepting DATA
                    if sender.is_empty() || accepted_recipient.is_empty() {
                         warn!("DATA command received without valid MAIL FROM or RCPT TO. Rejecting.");
                         initial_protocol.write_line("503 Bad sequence of commands (MAIL FROM and RCPT TO required first)").await?;
                         // Reset state? Protocol handler might need adjustment or we reset here.
                         // For now, just continue; the protocol handler should keep state correct.
                    } else {
                        // Proceed with DATA
                        collecting_data = true;
                        email_data.clear();
                        // State is updated internally by process_command (to Data)
                    }
                },
                SmtpCommandResult::DataLine(line_content) => {
                    if collecting_data {
                        // Append line, handling potential dot-stuffing if necessary (basic append here)
                        email_data.push_str(&line_content);
                        email_data.push_str("\r\n"); // Re-add CRLF lost by read_line
                    } else {
                        // Should not happen if state machine is correct, but log if it does.
                        warn!("Received DataLine result when not in Data state.");
                    }
                },
                SmtpCommandResult::DataEnd => {
                    collecting_data = false; // Stop collecting data
                    if sender.is_empty() || accepted_recipient.is_empty() {
                        warn!("DataEnd received but sender or recipient was missing. Message likely not processed.");
                        // State is reset to Greeted internally by protocol handler
                    } else {
                        // Parse the collected email data.
                        match EmailParser::parse(email_data.as_bytes()) {
                            Ok((subject, from_name, text_body, html_body)) => {
                                info!("Received email from {} to {} (Subject: '{}')", sender, accepted_recipient, subject);
                                // Prepare and forward the payload.
                                let email_payload = EmailPayload {
                                    sender: sender.clone(),
                                    sender_name: from_name, // Use the correct field name
                                    recipient: accepted_recipient.clone(),
                                    subject,
                                    body: text_body,
                                    html_body,
                                };
                                // Spawn forwarding in a separate task to avoid blocking the SMTP loop?
                                // For now, await directly. Consider spawning if webhook is slow.
                                if let Err(e) = webhook_client.forward_email(email_payload).await {
                                    error!("Failed to forward email from {}: {:#}", sender, e);
                                    // Log only, do not fail the SMTP session.
                                }
                            },
                            Err(e) => {
                                error!("Failed to parse email data from {}: {:#}", sender, e);
                                // Consider sending a 4xx/5xx SMTP error? Difficult after 250 OK for DATA end.
                            }
                        }
                    }
                    // Reset transaction state variables for the next potential email in the session.
                    sender.clear();
                    accepted_recipient.clear();
                    email_data.clear();
                    // State is reset to Greeted internally by protocol handler after DataEnd.
                },
                SmtpCommandResult::Continue => {
                    // Usually follows EHLO/HELO or error responses. Just continue the loop.
                }
                // STARTTLS is handled above by returning Err
            }
        }
    }.await; // End of async block

    // Check the result of the async block
    match protocol_result {
        Ok(()) => Ok(()), // Session ended normally (QUIT or EOF)
        Err(e) if e.to_string() == "STARTTLS" => {
            // Signal to handle STARTTLS was received
            handle_starttls(stream, webhook_client, target_emails).await
        }
        Err(e) => Err(e), // Propagate other errors
        // `initial_protocol` (and its borrow of `stream`) goes out of scope here.
    }
}


/// Performs the TLS handshake using a self-signed certificate.
///
/// If the handshake is successful, passes the encrypted stream to `handle_secure_session`.
///
/// # Arguments
///
/// * `stream` - The raw TCP stream after the `220 Go ahead` response to STARTTLS.
/// * `webhook_client` - Shared `WebhookClient`.
/// * `target_emails` - The configured list of target email addresses.
///
/// # Errors
///
/// Returns `Err` if certificate generation fails, TLS config creation fails, or the handshake fails.
async fn handle_starttls(
    stream: TcpStream, // Takes ownership of the raw TCP stream.
    webhook_client: Arc<WebhookClient>,
    target_emails: Vec<String>, // Changed from single String to Vec<String>
) -> Result<()> {
    // Generate ephemeral self-signed cert for the TLS session.
    let (cert, key) = generate_self_signed_cert()
        .context("Failed to generate self-signed certificate for STARTTLS")?;

    // Configure the rustls server-side TLS parameters.
    let tls_config = RustlsServerConfig::builder()
        .with_no_client_auth() // We don't require client certificates.
        .with_single_cert(vec![cert], key) // Provide the generated cert and key.
        .map_err(|e| anyhow::anyhow!("Failed to create rustls config: {}", e))?;

    // Create a TLS acceptor based on the configuration.
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    // Perform the TLS handshake over the existing TCP stream.
    match acceptor.accept(stream).await {
        Ok(tls_stream) => {
            // Handshake successful, proceed with the secure session.
            info!("STARTTLS handshake successful.");
            // Pass the list of target emails to the secure session handler.
            handle_secure_session(tls_stream, webhook_client, target_emails).await
        }
        Err(e) => {
            // Handshake failed. Log the error and return it.
            error!("STARTTLS handshake failed: {:?}", e);
            Err(anyhow::Error::new(e).context("STARTTLS handshake failed"))
        }
    }
}

/// Handles the SMTP command sequence over an established secure (TLS) connection.
///
/// This function is similar to the main loop in `handle_connection` but operates
/// on the encrypted `tls_stream`. It processes MAIL FROM, RCPT TO, DATA, etc.
///
/// # Arguments
///
/// * `tls_stream` - The encrypted TLS stream after a successful handshake.
/// * `webhook_client` - Shared `WebhookClient`.
/// * `target_emails` - The configured list of target email addresses.
///
/// # Type Parameters
///
/// * `T` - A type that implements `AsyncRead`, `AsyncWrite`, `Unpin`, `Send`, and `'static`,
///   representing the TLS stream type (e.g., `tokio_rustls::server::TlsStream<TcpStream>`).
///
/// # Errors
///
/// Returns `Err` if reading/writing to the TLS stream fails or if command processing fails.
async fn handle_secure_session<T>(
    tls_stream: T, // Generic over the actual TlsStream type.
    webhook_client: Arc<WebhookClient>,
    target_emails: Vec<String>, // Changed from single String to Vec<String>
) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static, // Traits required by tokio::io::split and SmtpProtocol.
{
    // Split the TLS stream for buffered I/O.
    let (read_half, write_half) = tokio::io::split(tls_stream);
    let reader = tokio::io::BufReader::new(read_half);
    let writer = tokio::io::BufWriter::new(write_half);
    // Create a new protocol handler for the secure stream.
    // Important: The state starts as Initial, expecting EHLO/HELO again after STARTTLS.
    let mut protocol = SmtpProtocol::new(reader, writer);

    // Variables to store state during the SMTP transaction within the secure session.
    let mut sender = String::new();
    let mut accepted_recipient = String::new(); // Store the specific recipient that was accepted
    let mut email_data = String::new();
    let mut collecting_data = false;

    // Main loop for processing commands over the secure connection.
    loop {
        trace!("SMTP(TLS/{:?}): Waiting for command...", protocol.get_state());
        let line = protocol.read_line().await?;
        trace!("SMTP(TLS/{:?}): Received line (len {}): {:?}", protocol.get_state(), line.len(), line);

        // Handle EOF during secure session.
        if protocol.get_state() != SmtpState::Data && line.is_empty() {
             info!("Connection closed by client (EOF) during secure session.");
             break;
        }

        // Process the command using the state machine.
        let result = protocol.process_command(&line).await?;

        match result {
            SmtpCommandResult::Quit => break,
            SmtpCommandResult::MailFrom(email) => {
                sender = email;
            },
            SmtpCommandResult::RcptTo(email) => {
                let received_email = email; // Rename for clarity
                // Validate recipient against the list of target emails (case-insensitive).
                let received_email_lower = received_email.to_lowercase();
                if target_emails.iter().any(|target| target.to_lowercase() == received_email_lower) {
                    // Store the *actual* accepted recipient address (preserving case)
                    accepted_recipient = received_email;
                    protocol.write_line("250 OK").await?;
                } else {
                    // Reject if not in the list.
                    protocol.write_line("550 No such user here").await?;
                    // Clear any previously accepted recipient if a new, invalid one is provided.
                    accepted_recipient.clear();
                }
            },
            SmtpCommandResult::DataStart => {
                collecting_data = true;
                email_data.clear();
            },
            SmtpCommandResult::DataLine(line_content) => {
                if collecting_data {
                    email_data.push_str(&line_content);
                    email_data.push_str("\r\n");
                }
            },
            SmtpCommandResult::DataEnd => {
                collecting_data = false;
                // Parse the collected email data.
                // Parse returns (subject, text_body, html_body) now
                // Pass email_data as bytes to the new parser signature
                // Parse returns (subject, from_name, text_body, html_body)
                let (subject, from_name, text_body, html_body) = EmailParser::parse(email_data.as_bytes())?;
                // Remove duplicate parse call from previous diff attempt
                info!("Received email (TLS) from {} to {} (Subject: '{}')", sender, accepted_recipient, subject);

                // Prepare and forward the payload.
                // Prepare and forward the payload.
                let email_payload = EmailPayload {
                    sender: sender.clone(),
                    sender_name: from_name, // Use the correct field name
                    recipient: accepted_recipient.clone(),
                    subject, // Use the parsed subject
                    body: text_body, // Use the parsed text_body
                    html_body, // Use the parsed html_body
                };
                if let Err(e) = webhook_client.forward_email(email_payload).await {
                    error!("Failed to forward email (TLS) from {}: {:#}", sender, e);
                    // Log only, do not fail the SMTP session.
                }

                // Reset state for the next email in the session.
                sender.clear();
                accepted_recipient.clear();
                email_data.clear();
                // Protocol state is reset to Greeted internally after DataEnd.
            },
            SmtpCommandResult::Continue => {
                // Usually follows EHLO/HELO after STARTTLS.
            }
            SmtpCommandResult::StartTls => {
                // STARTTLS is invalid within an already established TLS session.
                warn!("Received STARTTLS command within secure session. Sending error.");
                protocol.write_line("503 STARTTLS already active").await?;
            }
        }
    }
    Ok(())
}
