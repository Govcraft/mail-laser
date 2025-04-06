//! Handles the SMTP server logic, including listening for connections,
//! processing SMTP commands, parsing emails, and initiating webhook forwarding.

mod email_parser;
mod smtp_protocol;

use std::sync::Arc;
use anyhow::{Result, Context};
use log::{info, error, trace, warn}; // Keep all log levels for now
use tokio::net::{TcpListener, TcpStream};
// Keep only used IO traits/types
#[allow(unused_imports)] // Keep trait in scope for SmtpProtocol generic methods
use tokio::io::AsyncBufReadExt;
use crate::config::Config;
use crate::webhook::{WebhookClient, EmailPayload};
use smtp_protocol::{SmtpProtocol, SmtpCommandResult, SmtpState};
use email_parser::EmailParser;

// TLS related imports
use rustls::ServerConfig as RustlsServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::TlsAcceptor; // Remove unused server::TlsStream
// Correct rcgen imports based on documentation example and compiler warnings
use rcgen::{generate_simple_self_signed, CertifiedKey}; // Remove unused CertificateParams, SanType

/// Represents the main SMTP server instance.
///
/// Holds the application configuration and a shared `WebhookClient` instance
/// used by connection handlers to forward processed emails.
pub struct Server {
    config: Config,
    webhook_client: Arc<WebhookClient>,
}

impl Server {
    /// Creates a new SMTP `Server` instance.
    pub fn new(config: Config) -> Self {
        let webhook_client = Arc::new(WebhookClient::new(config.clone()));
        Server {
            config,
            webhook_client,
        }
    }

    /// Runs the main SMTP server loop.
    pub async fn run(&self) -> Result<()> {
        let addr = format!("{}:{}", self.config.smtp_bind_address, self.config.smtp_port);
        let listener = TcpListener::bind(&addr).await?;
        info!("SMTP server listening on {}", addr);

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("New connection from: {}", addr);
                    let webhook_client = Arc::clone(&self.webhook_client);
                    let target_email = self.config.target_email.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, webhook_client, target_email).await {
                            error!("Error handling SMTP connection from {}: {:#?}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Error accepting connection: {:?}", e);
                }
            }
        }
    }
}

/// Generates a self-signed TLS certificate and private key using rcgen.
/// Corrected based on rcgen docs and compiler feedback.
fn generate_self_signed_cert() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    // 1. Define Subject Alternative Names (SANs)
    //    generate_simple_self_signed takes Vec<String> directly.
    let subject_alt_names = vec!["localhost".to_string()];

    // 2. Generate the certificate and key pair using the helper function
    let certified_key: CertifiedKey = generate_simple_self_signed(subject_alt_names)
        .context("Failed to generate self-signed certificate")?;

    // 3. Extract the certificate and key in DER format for rustls
    //    Access .cert and .key_pair fields of CertifiedKey
    // Use the .der() method on the Certificate struct
    let cert_der = certified_key.cert.der().to_vec(); // Clone the bytes from the slice
    // Get the private key DER from the key_pair field
    let key_der = certified_key.key_pair.serialize_der();

    Ok((
        CertificateDer::from(cert_der),
        // Revert to Pkcs8 as Pkcs1 was incorrect.
        // generate_simple_self_signed likely produces PKCS#8 for its default RSA key.
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der))
    ))
}


// Main entry point for handling a new TCP connection.
// Manages the initial greeting and the STARTTLS handshake if requested.
async fn handle_connection(
    mut stream: TcpStream, // Make mutable for potential TLS upgrade
    webhook_client: Arc<WebhookClient>,
    target_email: String,
) -> Result<()> {
    #[allow(unused_assignments)] // This is assigned but only read if needs_tls_upgrade becomes true
    let mut needs_tls_upgrade = false;
    { // Scope for the initial protocol handler
        // Borrow the stream mutably to split it temporarily.
        // When this scope ends, the borrow ends, and we regain ownership of `stream`.
        let (read_half, write_half) = tokio::io::split(&mut stream);
        let reader = tokio::io::BufReader::new(read_half);
        let writer = tokio::io::BufWriter::new(write_half);
        let mut initial_protocol = SmtpProtocol::new(reader, writer);

        initial_protocol.send_greeting().await?;

        // Loop only for EHLO/HELO, STARTTLS, QUIT, or errors before TLS
        loop {
            trace!("SMTP(Initial): Waiting for command...");
            let line = initial_protocol.read_line().await?;
            trace!("SMTP(Initial): Received line (len {}): {:?}", line.len(), line);

            if line.is_empty() {
                 info!("Connection closed by client (EOF) during initial phase.");
                 return Ok(()); // Connection closed cleanly before TLS/MAIL
            }

            let result = initial_protocol.process_command(&line).await?;

            match result {
                SmtpCommandResult::StartTls => {
                    info!("Client initiated STARTTLS. Proceeding with handshake.");
                    needs_tls_upgrade = true;
                    break; // Exit initial loop to perform handshake
                }
                SmtpCommandResult::Quit => {
                    info!("Client quit during initial phase.");
                    return Ok(()); // Connection closed cleanly
                }
                SmtpCommandResult::Continue => {
                    // This handles EHLO/HELO responses. Any other command before STARTTLS
                    // in a real server might be an error or handled differently,
                    // but here we assume EHLO/HELO is the only valid step before STARTTLS/MAIL.
                    // If MAIL FROM etc. were received, process_command would return 5xx error.
                    // If EHLO/HELO was received, state becomes Greeted, ready for STARTTLS or MAIL.
                    if initial_protocol.get_state() == SmtpState::Greeted {
                        // Ready for next command (potentially STARTTLS)
                        continue;
                    } else {
                        // Should not happen if process_command is correct, but log defensively.
                        warn!("Unexpected state {:?} after Continue result in initial phase.", initial_protocol.get_state());
                        continue; // Or perhaps break with error? For now, continue.
                    }
                }
                // MAIL FROM, RCPT TO, DATA etc. are invalid before STARTTLS (handled by process_command returning 5xx)
                // or will be handled in the secure session.
                 _ => {
                    // process_command should have sent an error response (5xx)
                    trace!("Received unexpected command result {:?} in initial phase, client should disconnect or retry.", result);
                    // Continue loop, wait for client reaction (e.g., QUIT or valid command)
                    continue;
                 }
            }
        }
        // `initial_protocol` goes out of scope here, releasing the reader/writer halves.
        // The borrow of `stream` ends.
    } // End scope for initial_protocol

    if needs_tls_upgrade {
        // Pass the original stream (now owned again) to the TLS handler
        handle_starttls(stream, webhook_client, target_email).await?;
    } else {
        // This case should ideally not be reached if STARTTLS is enforced,
        // but handle it defensively. Could log an error or close.
        warn!("Connection ended without STARTTLS or QUIT.");
    }

    info!("Closing connection post-TLS or early exit.");
    Ok(())
}


/// Handles the TLS handshake process after STARTTLS is received.
async fn handle_starttls(
    stream: TcpStream, // Takes ownership of the raw TCP stream
    webhook_client: Arc<WebhookClient>,
    target_email: String,
) -> Result<()> {
    let (cert, key) = generate_self_signed_cert()
        .context("Failed to generate self-signed certificate for TLS")?;

    let tls_config = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key.clone_key())
        .map_err(|e| anyhow::anyhow!("Failed to create rustls config: {}", e))?;

    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    match acceptor.accept(stream).await {
        Ok(tls_stream) => {
            info!("TLS handshake successful.");
            handle_secure_session(tls_stream, webhook_client, target_email).await
        }
        Err(e) => {
            error!("TLS handshake failed: {:?}", e);
            Err(anyhow::Error::new(e).context("TLS handshake failed"))
        }
    }
}

/// Handles the SMTP command sequence over an established secure (TLS) connection.
async fn handle_secure_session<T>(
    tls_stream: T, // Generic over the TlsStream type
    webhook_client: Arc<WebhookClient>,
    target_email: String,
) -> Result<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, write_half) = tokio::io::split(tls_stream);
    let reader = tokio::io::BufReader::new(read_half);
    let writer = tokio::io::BufWriter::new(write_half);
    let mut protocol = SmtpProtocol::new(reader, writer);

    let mut sender = String::new();
    let mut recipient = String::new();
    let mut email_data = String::new();
    let mut collecting_data = false;

    loop {
        trace!("SMTP(TLS/{:?}): Waiting for command...", protocol.get_state());
        let line = protocol.read_line().await?;
        trace!("SMTP(TLS/{:?}): Received line (len {}): {:?}", protocol.get_state(), line.len(), line);

        if protocol.get_state() != SmtpState::Data && line.is_empty() {
             info!("Connection closed by client (EOF) during secure session.");
             break;
        }

        let result = protocol.process_command(&line).await?;

        match result {
            SmtpCommandResult::Quit => break,
            SmtpCommandResult::MailFrom(email) => {
                sender = email;
            },
            SmtpCommandResult::RcptTo(email) => {
                recipient = email;
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
                let (subject, body) = EmailParser::parse(&email_data)?;
                info!("Received email (TLS) from {} to {} (Subject: '{}')", sender, recipient, subject);

                let email_payload = EmailPayload {
                    sender: sender.clone(),
                    subject,
                    body,
                };

                if let Err(e) = webhook_client.forward_email(email_payload).await {
                    error!("Failed to forward email (TLS) from {}: {:#}", sender, e);
                }

                sender.clear();
                recipient.clear();
                email_data.clear();
            },
            SmtpCommandResult::Continue => {}
            SmtpCommandResult::StartTls => {
                warn!("Received STARTTLS command within secure session. Sending error.");
                protocol.write_line("503 STARTTLS already active").await?;
            }
        }
    }
    Ok(())
}
