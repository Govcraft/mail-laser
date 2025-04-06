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
                    let target_email = self.config.target_email.clone();
                    // Spawn a dedicated asynchronous task for each connection.
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, webhook_client, target_email).await {
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
/// * `target_email` - The configured target email address.
///
/// # Errors
///
/// Returns `Err` if initial greeting fails, reading/processing initial commands fails,
/// or if the STARTTLS handshake fails.
async fn handle_connection(
    mut stream: TcpStream, // Mutable ownership needed for potential TLS upgrade.
    webhook_client: Arc<WebhookClient>,
    target_email: String,
) -> Result<()> {
    let mut needs_tls_upgrade = false; // Flag to track if STARTTLS was requested.

    // Scope for the initial, non-TLS protocol handler.
    // We temporarily split the stream to use BufReader/BufWriter.
    // The split borrows `stream` mutably. When the scope ends, the borrow ends,
    // and we regain full ownership of `stream` for potential TLS upgrade.
    {
        let (read_half, write_half) = tokio::io::split(&mut stream);
        let reader = tokio::io::BufReader::new(read_half);
        let writer = tokio::io::BufWriter::new(write_half);
        let mut initial_protocol = SmtpProtocol::new(reader, writer);

        // Send the initial 220 greeting.
        initial_protocol.send_greeting().await?;

        // Process initial commands (EHLO/HELO, STARTTLS, QUIT).
        loop {
            trace!("SMTP(Initial): Waiting for command...");
            let line = initial_protocol.read_line().await?;
            trace!("SMTP(Initial): Received line (len {}): {:?}", line.len(), line);

            // Handle EOF during initial phase.
            if line.is_empty() {
                 info!("Connection closed by client (EOF) during initial phase.");
                 return Ok(()); // Clean exit.
            }

            // Process the command using the state machine.
            let result = initial_protocol.process_command(&line).await?;

            match result {
                SmtpCommandResult::StartTls => {
                    // Client requested TLS upgrade.
                    info!("Client initiated STARTTLS. Proceeding with handshake.");
                    needs_tls_upgrade = true;
                    break; // Exit this loop to perform the TLS handshake.
                }
                SmtpCommandResult::Quit => {
                    // Client quit before TLS or sending mail.
                    info!("Client quit during initial phase.");
                    return Ok(()); // Clean exit.
                }
                SmtpCommandResult::Continue => {
                    // Typically follows EHLO/HELO. State should be Greeted.
                    // Continue waiting for the next command (e.g., STARTTLS or MAIL FROM).
                    if initial_protocol.get_state() == SmtpState::Greeted {
                        continue;
                    } else {
                        // This state indicates an unexpected sequence, likely an error response was sent.
                        warn!("Unexpected state {:?} after Continue result in initial phase.", initial_protocol.get_state());
                        continue; // Continue waiting for client reaction.
                    }
                }
                 _ => {
                    // Any other command result (MAIL FROM, RCPT TO, etc.) is invalid here.
                    // The protocol handler should have sent a 5xx error.
                    trace!("Received unexpected command result {:?} in initial phase.", result);
                    // Continue loop, wait for client reaction (e.g., QUIT).
                    continue;
                 }
            }
        }
        // `initial_protocol` (and its borrow of `stream`) goes out of scope here.
    }

    // Perform TLS handshake if requested.
    if needs_tls_upgrade {
        // Pass ownership of the original stream to the TLS handler.
        handle_starttls(stream, webhook_client, target_email).await?;
    } else {
        // If the loop exited without STARTTLS or QUIT, it's unexpected.
        // This might happen if the client sends EHLO then disconnects.
        warn!("Connection ended without STARTTLS or QUIT after initial phase.");
    }

    info!("Closing connection after initial/TLS phase.");
    Ok(())
}


/// Performs the TLS handshake using a self-signed certificate.
///
/// If the handshake is successful, passes the encrypted stream to `handle_secure_session`.
///
/// # Arguments
///
/// * `stream` - The raw TCP stream after the `220 Go ahead` response to STARTTLS.
/// * `webhook_client` - Shared `WebhookClient`.
/// * `target_email` - The configured target email address.
///
/// # Errors
///
/// Returns `Err` if certificate generation fails, TLS config creation fails, or the handshake fails.
async fn handle_starttls(
    stream: TcpStream, // Takes ownership of the raw TCP stream.
    webhook_client: Arc<WebhookClient>,
    target_email: String,
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
            handle_secure_session(tls_stream, webhook_client, target_email).await
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
/// * `target_email` - The configured target email address.
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
    target_email: String,
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
    let mut recipient = String::new();
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
                recipient = email;
                // Validate recipient against target.
                if recipient.to_lowercase() == target_email.to_lowercase() {
                    protocol.write_line("250 OK").await?;
                } else {
                    protocol.write_line("550 No such user here").await?;
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
                let (subject, body) = EmailParser::parse(&email_data)?;
                info!("Received email (TLS) from {} to {} (Subject: '{}')", sender, recipient, subject);

                // Prepare and forward the payload.
                let email_payload = EmailPayload {
                    sender: sender.clone(),
                    subject,
                    body,
                };
                if let Err(e) = webhook_client.forward_email(email_payload).await {
                    error!("Failed to forward email (TLS) from {}: {:#}", sender, e);
                    // Log only, do not fail the SMTP session.
                }

                // Reset state for the next email in the session.
                sender.clear();
                recipient.clear();
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
