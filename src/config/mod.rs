use std::env;
use anyhow::{Result, anyhow}; // Import anyhow macro
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
            Err(e) => { // Keep logging errors as before
                let err_msg = "MAIL_LASER_TARGET_EMAIL environment variable must be set";
                log::error!("{}: {}", err_msg, e); // Log error immediately
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!("Config: Using target_email: {}", target_email); // Log the value used
            
        let webhook_url = match env::var("MAIL_LASER_WEBHOOK_URL") {
            Ok(val) => val,
            Err(e) => { // Keep logging errors as before
                let err_msg = "MAIL_LASER_WEBHOOK_URL environment variable must be set";
                log::error!("{}: {}", err_msg, e); // Log error immediately
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!("Config: Using webhook_url: {}", webhook_url); // Log the value used
            
        let smtp_bind_address = env::var("MAIL_LASER_BIND_ADDRESS")
            .map(|val| {
                log::info!("Config: Using smtp_bind_address from env: {}", val);
                val
            })
            .unwrap_or_else(|_| {
                let default_val = "0.0.0.0".to_string();
                log::info!("Config: Using default smtp_bind_address: {}", default_val);
                default_val
            });
            
        let smtp_port_str = env::var("MAIL_LASER_PORT")
            .unwrap_or_else(|_| "2525".to_string()); // Use a non-privileged port by default
        let smtp_port = match smtp_port_str.parse::<u16>() {
            Ok(port) => port,
            Err(e) => { // Keep logging errors as before
                let err_msg = format!("MAIL_LASER_PORT ('{}') must be a valid port number", smtp_port_str);
                log::error!("{}: {}", err_msg, e); // Log error immediately
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!("Config: Using smtp_port: {}", smtp_port); // Log the value used
 
        let health_check_bind_address = env::var("MAIL_LASER_HEALTH_BIND_ADDRESS")
            .map(|val| {
                log::info!("Config: Using health_check_bind_address from env: {}", val);
                val
            })
            .unwrap_or_else(|_| {
                let default_val = "0.0.0.0".to_string();
                log::info!("Config: Using default health_check_bind_address: {}", default_val);
                default_val
            });

        let health_check_port_str = env::var("MAIL_LASER_HEALTH_PORT")
            .unwrap_or_else(|_| "8080".to_string()); // Default health check port
        let health_check_port = match health_check_port_str.parse::<u16>() {
            Ok(port) => port,
            Err(e) => { // Keep logging errors as before
                let err_msg = format!("MAIL_LASER_HEALTH_PORT ('{}') must be a valid port number", health_check_port_str);
                log::error!("{}: {}", err_msg, e); // Log error immediately
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!("Config: Using health_check_port: {}", health_check_port); // Log the value used
            
        Ok(Config {
            // Log the final constructed config object for a summary view
            // Note: This might be slightly redundant with the individual logs above,
            // but provides a good overview. Consider keeping only this if verbosity is a concern.
            target_email,
            webhook_url,
            smtp_bind_address,
            smtp_port,
            health_check_bind_address,
            health_check_port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // Helper function to set environment variables for a test scope
    fn set_env_vars() {
        env::set_var("MAIL_LASER_TARGET_EMAIL", "test@example.com");
        env::set_var("MAIL_LASER_WEBHOOK_URL", "http://localhost:8000/webhook");
        env::set_var("MAIL_LASER_PORT", "3000");
        // We won't set BIND_ADDRESS, HEALTH_BIND_ADDRESS, or HEALTH_PORT to test defaults
    }

    // Helper function to clear environment variables after test
    fn clear_env_vars() {
        env::remove_var("MAIL_LASER_TARGET_EMAIL");
        env::remove_var("MAIL_LASER_WEBHOOK_URL");
        env::remove_var("MAIL_LASER_PORT");
        env::remove_var("MAIL_LASER_BIND_ADDRESS"); // Ensure cleared even if not set
        env::remove_var("MAIL_LASER_HEALTH_BIND_ADDRESS");
        env::remove_var("MAIL_LASER_HEALTH_PORT");
    }

    #[test]
    fn test_config_from_env_mixed() {
        set_env_vars();

        let config_result = Config::from_env();
        assert!(config_result.is_ok());
        let config = config_result.unwrap();

        assert_eq!(config.target_email, "test@example.com");
        assert_eq!(config.webhook_url, "http://localhost:8000/webhook");
        assert_eq!(config.smtp_bind_address, "0.0.0.0"); // Default
        assert_eq!(config.smtp_port, 3000); // From env
        assert_eq!(config.health_check_bind_address, "0.0.0.0"); // Default
        assert_eq!(config.health_check_port, 8080); // Default

        clear_env_vars(); // Clean up environment variables
    }

    #[test]
    fn test_config_from_env_missing_required() {
        clear_env_vars(); // Ensure required vars are not set

        let config_result = Config::from_env();
        assert!(config_result.is_err());
        // Check if the error message contains the expected variable name
        assert!(config_result.unwrap_err().to_string().contains("MAIL_LASER_TARGET_EMAIL"));

        // Set one required var, check for the other missing one
        env::set_var("MAIL_LASER_TARGET_EMAIL", "test@example.com");
        let config_result_2 = Config::from_env();
        assert!(config_result_2.is_err());
        assert!(config_result_2.unwrap_err().to_string().contains("MAIL_LASER_WEBHOOK_URL"));

        clear_env_vars();
    }

     #[test]
    fn test_config_from_env_invalid_port() {
        set_env_vars(); // Set valid required vars first
        env::set_var("MAIL_LASER_PORT", "not-a-port"); // Set invalid port

        // Use if let Err to handle the error case directly
        if let Err(e) = Config::from_env() {
            let err_msg = e.to_string();
            assert!(err_msg.contains("MAIL_LASER_PORT"));
            assert!(err_msg.contains("not-a-port"));
        } else {
            panic!("Expected an error for invalid MAIL_LASER_PORT, but got Ok");
        }


        env::set_var("MAIL_LASER_PORT", "3000"); // Reset valid port
        env::set_var("MAIL_LASER_HEALTH_PORT", "invalid"); // Set invalid health port

        // Use if let Err again for the second case
        if let Err(e) = Config::from_env() {
            let err_msg_2 = e.to_string();
            assert!(err_msg_2.contains("MAIL_LASER_HEALTH_PORT"));
            assert!(err_msg_2.contains("invalid"));
        } else {
            panic!("Expected an error for invalid MAIL_LASER_HEALTH_PORT, but got Ok");
        }

        clear_env_vars();
    }
}
