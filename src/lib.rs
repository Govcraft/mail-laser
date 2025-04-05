pub mod smtp;
pub mod webhook;
pub mod config;

use anyhow::Result;
use log::info;

pub async fn run() -> Result<()> {
    info!("Starting MailLaser SMTP server"); // Update log message
    
    // Load configuration from environment variables
    let config = config::Config::from_env()?;
    
    // Start the SMTP server
    let smtp_server = smtp::Server::new(config.clone());
    
    // Run the server
    smtp_server.run().await?;
    
    Ok(())
}
