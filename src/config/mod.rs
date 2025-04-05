use std::env;
use anyhow::{Result, Context};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The email address to accept mail for
    pub target_email: String,
    
    /// The webhook URL to forward emails to
    pub webhook_url: String,
    
    /// The address to bind the SMTP server to
    pub smtp_bind_address: String,
    
    /// The port to bind the SMTP server to
    pub smtp_port: u16,
}

impl Config {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        // Load .env file if present (optional)
        let _ = dotenv::dotenv();
        
        let target_email = env::var("MAIL_LASER_TARGET_EMAIL")
            .context("MAIL_LASER_TARGET_EMAIL environment variable must be set")?;
            
        let webhook_url = env::var("MAIL_LASER_WEBHOOK_URL")
            .context("MAIL_LASER_WEBHOOK_URL environment variable must be set")?;
            
        let smtp_bind_address = env::var("MAIL_LASER_BIND_ADDRESS")
            .unwrap_or_else(|_| "0.0.0.0".to_string());
            
        let smtp_port = env::var("MAIL_LASER_PORT")
            .unwrap_or_else(|_| "2525".to_string()) // Use a non-privileged port by default
            .parse::<u16>()
            .context("MAIL_LASER_PORT must be a valid port number")?;
            
        Ok(Config {
            target_email,
            webhook_url,
            smtp_bind_address,
            smtp_port,
        })
    }
}
