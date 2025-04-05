mod email_parser;
mod smtp_protocol;

use std::sync::Arc;
use anyhow::Result;
use log::{info, error, trace}; // Removed unused debug
use tokio::net::{TcpListener, TcpStream};
use crate::config::Config;
use crate::webhook::{WebhookClient, EmailPayload};
use smtp_protocol::{SmtpProtocol, SmtpCommandResult};
use smtp_protocol::SmtpState; // Import the SmtpState enum
use email_parser::EmailParser;

pub struct Server {
    config: Config,
    webhook_client: Arc<WebhookClient>,
}

impl Server {
    // Original signature returning Self directly
    pub fn new(config: Config) -> Self {
        // WebhookClient::new now returns Self directly (panics on cert error)
        let webhook_client = Arc::new(WebhookClient::new(config.clone()));

        Server {
            config,
            webhook_client,
        }
    }
    
    pub async fn run(&self) -> Result<()> {
        let addr = format!("{}:{}", self.config.smtp_bind_address, self.config.smtp_port);
        let listener = TcpListener::bind(&addr).await?;
        
        info!("SMTP server listening on {}", addr);
        
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("New connection from: {}", addr);
                    
                    // Clone the webhook client for this connection
                    let webhook_client = Arc::clone(&self.webhook_client);
                    
                    // Clone the target email for this connection
                    let target_email = self.config.target_email.clone();
                    
                    // Spawn a new task to handle this connection
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, webhook_client, target_email).await {
                            error!("Error handling SMTP connection: {:?}", e);
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

async fn handle_connection(
    stream: TcpStream,
    webhook_client: Arc<WebhookClient>,
    target_email: String,
) -> Result<()> {
    let mut protocol = SmtpProtocol::new(stream);
    
    // Send greeting
    protocol.send_greeting().await?;
    
    let mut sender = String::new();
    let mut recipient = String::new();
    let mut email_data = String::new();
    let mut collecting_data = false;
    
    loop {
        trace!("Waiting to read next line..."); // Changed to trace
        let line = protocol.read_line().await?;
        trace!("Successfully read line (raw length {}): {:?}", line.len(), line); // Changed to trace
        
        // Check for closed connection *only if not* in DATA state.
        // An empty line during DATA state is just part of the email body.
        // The actual end of DATA is signaled by the "." line, handled by process_command.
        // read_line returns empty string "" on EOF (bytes_read == 0).
        if protocol.get_state() != SmtpState::Data && line.is_empty() {
             info!("Connection closed by client (or EOF) outside DATA state.");
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
                
                // Check if this is for our target email
                if recipient.to_lowercase() == target_email.to_lowercase() {
                    protocol.write_line("250 OK").await?;
                } else {
                    protocol.write_line("550 No such user here").await?;
                }
            },
            SmtpCommandResult::DataStart => {
                collecting_data = true;
                email_data.clear();
                // protocol.reset_state(); // Incorrect: State should remain Data until "." is received
            },
            SmtpCommandResult::DataLine(line) => {
                if collecting_data {
                    email_data.push_str(&line);
                    email_data.push_str("\r\n");
                }
            },
            SmtpCommandResult::DataEnd => {
                collecting_data = false;
                
                // Parse the email to extract subject and body
                let (subject, body) = EmailParser::parse(&email_data)?;
                
                info!("Received email from {} to {}", sender, recipient);
                
                // Forward the email to the webhook
                let email_payload = EmailPayload {
                    sender: sender.clone(),
                    subject,
                    body,
                };
                
                if let Err(e) = webhook_client.forward_email(email_payload).await {
                    // Use anyhow's detailed formatting "{:#}" to include the error chain/context
                    error!("Failed to forward email: {:#}", e);
                    // Optionally, log the specific type as well for debugging
                    // error!("Failed to forward email (type: {}): {:#}", std::any::type_name_of_val(&e), e);
                }
                
                // Reset for next email
                sender.clear();
                recipient.clear();
                email_data.clear();
            },
            SmtpCommandResult::Continue => {
                // Nothing to do, continue processing
            }
        }
    }
    
    Ok(())
}
