//! Handles the SMTP server logic, including listening for connections,
//! processing SMTP commands, parsing emails, and initiating webhook forwarding.

mod email_parser;
mod smtp_protocol;

use std::sync::Arc;
use anyhow::Result;
use log::{info, error, trace};
use tokio::net::{TcpListener, TcpStream};
use crate::config::Config;
use crate::webhook::{WebhookClient, EmailPayload};
use smtp_protocol::{SmtpProtocol, SmtpCommandResult, SmtpState};
use email_parser::EmailParser;

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
    /// Initializes the shared `WebhookClient`. Panics if the webhook client
    /// fails to initialize (e.g., due to issues loading TLS certificates).
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
        let listener = TcpListener::bind(&addr).await?;

        info!("SMTP server listening on {}", addr);

        // Main server loop: continuously accept incoming connections.
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("New connection from: {}", addr);

                    // Clone the Arc<WebhookClient> for the new task. Cloning Arc is cheap.
                    let webhook_client = Arc::clone(&self.webhook_client);
                    // Clone the target email string for the new task.
                    let target_email = self.config.target_email.clone();

                    // Spawn a dedicated asynchronous task for each connection.
                    // This allows the server to handle multiple clients concurrently.
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, webhook_client, target_email).await {
                            // Log errors from individual connection handlers.
                            // Using {:#?} includes the error source/context from anyhow.
                            error!("Error handling SMTP connection from {}: {:#?}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    // Log errors encountered during connection acceptance.
                    // The loop continues to try accepting new connections.
                    error!("Error accepting connection: {:?}", e);
                }
            }
        }
        // This loop is infinite, so Ok(()) is never reached in normal operation.
        // The function only returns Err if the initial bind fails.
    }
}

/// Handles a single SMTP client connection.
///
/// Processes the SMTP command sequence (HELO, MAIL FROM, RCPT TO, DATA, QUIT),
/// validates the recipient, collects email data, parses it, and forwards
/// the relevant parts via the `WebhookClient`.
///
/// # Arguments
///
/// * `stream` - The TCP stream representing the client connection.
/// * `webhook_client` - A shared `Arc` pointer to the `WebhookClient`.
/// * `target_email` - The configured email address this server accepts mail for.
async fn handle_connection(
    stream: TcpStream,
    webhook_client: Arc<WebhookClient>,
    target_email: String,
) -> Result<()> {
    // Initialize the SMTP protocol state machine for this connection.
    let mut protocol = SmtpProtocol::new(stream);

    // Start the conversation by sending the SMTP greeting.
    protocol.send_greeting().await?;

    // Variables to store state during the SMTP transaction.
    let mut sender = String::new();
    let mut recipient = String::new();
    let mut email_data = String::new(); // Buffer for accumulating DATA content.
    let mut collecting_data = false; // Flag indicating if we are in the DATA phase.

    // Main loop processing commands from the client.
    loop {
        trace!("SMTP({:?}): Waiting for command...", protocol.get_state());
        let line = protocol.read_line().await?;
        trace!("SMTP({:?}): Received line (len {}): {:?}", protocol.get_state(), line.len(), line);

        // Check if the connection closed unexpectedly (empty line read outside DATA state).
        // An empty line *during* DATA is valid content. The end of DATA is marked by ".".
        if protocol.get_state() != SmtpState::Data && line.is_empty() {
             info!("Connection closed by client (EOF) outside DATA state.");
             break; // Exit the loop for this connection.
        }

        // Process the received line based on the current SMTP state.
        let result = protocol.process_command(&line).await?;

        // Handle the outcome of the command processing.
        match result {
            SmtpCommandResult::Quit => break, // Client requested termination.
            SmtpCommandResult::MailFrom(email) => {
                sender = email; // Store the sender address.
            },
            SmtpCommandResult::RcptTo(email) => {
                recipient = email; // Store the recipient address.

                // Validate if the recipient matches the configured target email (case-insensitive).
                if recipient.to_lowercase() == target_email.to_lowercase() {
                    protocol.write_line("250 OK").await?; // Accept the recipient.
                } else {
                    // Reject recipients not matching the target.
                    protocol.write_line("550 No such user here").await?;
                }
            },
            SmtpCommandResult::DataStart => {
                // Transition to collecting email data.
                collecting_data = true;
                email_data.clear(); // Ensure the buffer is empty for new data.
            },
            SmtpCommandResult::DataLine(line_content) => {
                // If in DATA state, append the line to the buffer.
                if collecting_data {
                    email_data.push_str(&line_content);
                    email_data.push_str("\r\n"); // Preserve line endings.
                }
            },
            SmtpCommandResult::DataEnd => {
                // End of DATA phase signaled by ".".
                collecting_data = false;

                // Attempt to parse the collected email data.
                let (subject, body) = EmailParser::parse(&email_data)?;
                info!("Received email from {} to {} (Subject: '{}')", sender, recipient, subject);

                // Prepare the payload for the webhook.
                let email_payload = EmailPayload {
                    sender: sender.clone(), // Clone sender as it's needed for reset later.
                    subject,
                    body,
                };

                // Asynchronously forward the email payload via the webhook client.
                // Log errors but do not fail the SMTP transaction if forwarding fails.
                // The email is considered "accepted" by the SMTP server at this point.
                if let Err(e) = webhook_client.forward_email(email_payload).await {
                    // Use anyhow's detailed formatting "{:#}" to include the error chain/context.
                    error!("Failed to forward email from {}: {:#}", sender, e);
                }

                // Reset state for the next potential email in the same session.
                sender.clear();
                recipient.clear();
                email_data.clear();
                // The protocol state is reset internally by process_command after DataEnd.
            },
            SmtpCommandResult::Continue => {
                // No specific action needed for this command result, just continue the loop.
            }
        }
    }

    info!("Closing connection.");
    Ok(())
}
