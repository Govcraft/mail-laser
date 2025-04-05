use std::env;
use anyhow::{Result, Context, anyhow}; // Import anyhow macro
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

    /// The address to bind the health check server to
    pub health_check_bind_address: String,

    /// The port to bind the health check server to
    pub health_check_port: u16,
}

impl Config {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        // Load .env file if present (optional)
        let _ = dotenv::dotenv();
        
        let target_email = match env::var("MAIL_LASER_TARGET_EMAIL") {
            Ok(val) => val,
            Err(e) => {
                let err_msg = "MAIL_LASER_TARGET_EMAIL environment variable must be set";
                log::error!("{}: {}", err_msg, e); // Log error immediately
                return Err(anyhow!(e).context(err_msg));
            }
        };
            
        let webhook_url = match env::var("MAIL_LASER_WEBHOOK_URL") {
            Ok(val) => val,
            Err(e) => {
                let err_msg = "MAIL_LASER_WEBHOOK_URL environment variable must be set";
                log::error!("{}: {}", err_msg, e); // Log error immediately
                return Err(anyhow!(e).context(err_msg));
            }
        };
            
        let smtp_bind_address = env::var("MAIL_LASER_BIND_ADDRESS")
            .unwrap_or_else(|_| "0.0.0.0".to_string());
            
        let smtp_port_str = env::var("MAIL_LASER_PORT")
            .unwrap_or_else(|_| "2525".to_string()); // Use a non-privileged port by default
        let smtp_port = match smtp_port_str.parse::<u16>() {
            Ok(port) => port,
            Err(e) => {
                let err_msg = format!("MAIL_LASER_PORT ('{}') must be a valid port number", smtp_port_str);
                log::error!("{}: {}", err_msg, e); // Log error immediately
                return Err(anyhow!(e).context(err_msg));
            }
        };

        let health_check_bind_address = env::var("MAIL_LASER_HEALTH_BIND_ADDRESS")
            .unwrap_or_else(|_| "0.0.0.0".to_string());

        let health_check_port_str = env::var("MAIL_LASER_HEALTH_PORT")
            .unwrap_or_else(|_| "8080".to_string()); // Default health check port
        let health_check_port = match health_check_port_str.parse::<u16>() {
            Ok(port) => port,
            Err(e) => {
                let err_msg = format!("MAIL_LASER_HEALTH_PORT ('{}') must be a valid port number", health_check_port_str);
                log::error!("{}: {}", err_msg, e); // Log error immediately
                return Err(anyhow!(e).context(err_msg));
            }
        };
            
        Ok(Config {
            target_email,
            webhook_url,
            smtp_bind_address,
            smtp_port,
            health_check_bind_address,
            health_check_port,
        })
    }
}
