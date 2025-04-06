//! Manages application configuration loaded from environment variables.
//!
//! This module defines the `Config` struct which holds all runtime settings
//! and provides the `from_env` function to populate this struct. It supports
//! loading variables from a `.env` file via the `dotenv` crate and provides
//! default values for optional settings.

use std::env;
use anyhow::{Result, anyhow};
use serde::{Serialize, Deserialize};

/// Holds the application's runtime configuration settings.
///
/// These settings are typically loaded from environment variables via `from_env`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The *only* email address MailLaser will accept mail for. (Required: `MAIL_LASER_TARGET_EMAIL`)
    pub target_email: String,

    /// The URL where the extracted email payload will be sent via POST request. (Required: `MAIL_LASER_WEBHOOK_URL`)
    pub webhook_url: String,

    /// The IP address the SMTP server should listen on. (Optional: `MAIL_LASER_BIND_ADDRESS`, Default: "0.0.0.0")
    pub smtp_bind_address: String,

    /// The network port the SMTP server should listen on. (Optional: `MAIL_LASER_PORT`, Default: 2525)
    pub smtp_port: u16,

    /// The IP address the health check HTTP server should listen on. (Optional: `MAIL_LASER_HEALTH_BIND_ADDRESS`, Default: "0.0.0.0")
    pub health_check_bind_address: String,

    /// The network port the health check HTTP server should listen on. (Optional: `MAIL_LASER_HEALTH_PORT`, Default: 8080)
    pub health_check_port: u16,
}

impl Config {
    /// Loads configuration settings from environment variables.
    ///
    /// Reads variables prefixed with `MAIL_LASER_`. Supports loading from a `.env` file
    /// if present. Provides default values for bind addresses and ports if not specified.
    /// Logs the configuration values being used.
    ///
    /// # Errors
    ///
    /// Returns an `Err` if:
    /// - Required environment variables (`MAIL_LASER_TARGET_EMAIL`, `MAIL_LASER_WEBHOOK_URL`) are missing.
    /// - Optional port variables (`MAIL_LASER_PORT`, `MAIL_LASER_HEALTH_PORT`) are set but cannot be parsed as `u16`.
    pub fn from_env() -> Result<Self> {
        // Attempt to load variables from a .env file, if it exists. Ignore errors.
        let _ = dotenv::dotenv();

        // --- Required Variables ---
        let target_email = match env::var("MAIL_LASER_TARGET_EMAIL") {
            Ok(val) => val,
            Err(e) => {
                let err_msg = "MAIL_LASER_TARGET_EMAIL environment variable must be set";
                log::error!("{}: {}", err_msg, e); // Log specific error before returning
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!("Config: Using target_email: {}", target_email);

        let webhook_url = match env::var("MAIL_LASER_WEBHOOK_URL") {
            Ok(val) => val,
            Err(e) => {
                let err_msg = "MAIL_LASER_WEBHOOK_URL environment variable must be set";
                log::error!("{}: {}", err_msg, e); // Log specific error before returning
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!("Config: Using webhook_url: {}", webhook_url);

        // --- Optional Variables with Defaults ---
        let smtp_bind_address = env::var("MAIL_LASER_BIND_ADDRESS")
            .map(|val| {
                log::info!("Config: Using smtp_bind_address from env: {}", val);
                val
            })
            .unwrap_or_else(|_| {
                let default_val = "0.0.0.0".to_string();
                log::info!("Config: Using default smtp_bind_address: {}", default_val);
                default_val // Default: Listen on all interfaces
            });

        let smtp_port_str = env::var("MAIL_LASER_PORT")
            .unwrap_or_else(|_| "2525".to_string()); // Default SMTP port
        let smtp_port = match smtp_port_str.parse::<u16>() {
            Ok(port) => port,
            Err(e) => {
                let err_msg = format!("MAIL_LASER_PORT ('{}') must be a valid u16 port number", smtp_port_str);
                log::error!("{}: {}", err_msg, e); // Log specific error before returning
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!("Config: Using smtp_port: {}", smtp_port);

        let health_check_bind_address = env::var("MAIL_LASER_HEALTH_BIND_ADDRESS")
            .map(|val| {
                log::info!("Config: Using health_check_bind_address from env: {}", val);
                val
            })
            .unwrap_or_else(|_| {
                let default_val = "0.0.0.0".to_string();
                log::info!("Config: Using default health_check_bind_address: {}", default_val);
                default_val // Default: Listen on all interfaces
            });

        let health_check_port_str = env::var("MAIL_LASER_HEALTH_PORT")
            .unwrap_or_else(|_| "8080".to_string()); // Default health check port
        let health_check_port = match health_check_port_str.parse::<u16>() {
            Ok(port) => port,
            Err(e) => {
                let err_msg = format!("MAIL_LASER_HEALTH_PORT ('{}') must be a valid u16 port number", health_check_port_str);
                log::error!("{}: {}", err_msg, e); // Log specific error before returning
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!("Config: Using health_check_port: {}", health_check_port);

        // Construct the final Config object
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;
    use once_cell::sync::Lazy;

    // Static Mutex to serialize tests modifying environment variables, preventing race conditions.
    static ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    // Helper functions set_env_vars and clear_env_vars are removed.
    // Setup and teardown now happen within each test under the ENV_LOCK mutex.

    #[test]
    fn test_config_from_env_mixed() {
        let _lock = ENV_LOCK.lock().unwrap(); // Acquire lock for test duration

        // Setup environment variables for this specific test case
        env::set_var("MAIL_LASER_TARGET_EMAIL", "test@example.com");
        env::set_var("MAIL_LASER_WEBHOOK_URL", "http://localhost:8000/webhook");
        env::set_var("MAIL_LASER_PORT", "3000");
        // Clear others explicitly to ensure defaults are tested correctly
        env::remove_var("MAIL_LASER_BIND_ADDRESS");
        env::remove_var("MAIL_LASER_HEALTH_BIND_ADDRESS");
        env::remove_var("MAIL_LASER_HEALTH_PORT");

        let config_result = Config::from_env();
        assert!(config_result.is_ok(), "Config loading failed when it should succeed: {:?}", config_result.err());
        let config = config_result.unwrap();

        assert_eq!(config.target_email, "test@example.com");
        assert_eq!(config.webhook_url, "http://localhost:8000/webhook");
        assert_eq!(config.smtp_bind_address, "0.0.0.0", "Default SMTP bind address mismatch");
        assert_eq!(config.smtp_port, 3000, "SMTP port mismatch");
        assert_eq!(config.health_check_bind_address, "0.0.0.0", "Default health bind address mismatch");
        assert_eq!(config.health_check_port, 8080, "Default health port mismatch");

        // Teardown: Clear variables set by this test (best practice within lock)
        env::remove_var("MAIL_LASER_TARGET_EMAIL");
        env::remove_var("MAIL_LASER_WEBHOOK_URL");
        env::remove_var("MAIL_LASER_PORT");
        // Lock is released automatically when _lock goes out of scope here
    }

    #[test]
    fn test_config_from_env_missing_required() {
        let _lock = ENV_LOCK.lock().unwrap(); // Acquire lock

        // Teardown/Setup: Ensure required vars are definitely not set
        env::remove_var("MAIL_LASER_TARGET_EMAIL");
        env::remove_var("MAIL_LASER_WEBHOOK_URL");
        // Clear others too for a clean slate
        env::remove_var("MAIL_LASER_PORT");
        env::remove_var("MAIL_LASER_BIND_ADDRESS");
        env::remove_var("MAIL_LASER_HEALTH_BIND_ADDRESS");
        env::remove_var("MAIL_LASER_HEALTH_PORT");

        // Test missing TARGET_EMAIL
        let config_result = Config::from_env();
        assert!(config_result.is_err(), "Expected error for missing TARGET_EMAIL, got Ok");
        assert!(config_result.unwrap_err().to_string().contains("MAIL_LASER_TARGET_EMAIL"), "Error message mismatch for missing TARGET_EMAIL");

        // Test missing WEBHOOK_URL (after setting TARGET_EMAIL)
        env::set_var("MAIL_LASER_TARGET_EMAIL", "test@example.com");
        let config_result_2 = Config::from_env();
        assert!(config_result_2.is_err(), "Expected error for missing WEBHOOK_URL, got Ok");
        assert!(config_result_2.unwrap_err().to_string().contains("MAIL_LASER_WEBHOOK_URL"), "Error message mismatch for missing WEBHOOK_URL");

        // Teardown: Clear variables potentially set by this test
        env::remove_var("MAIL_LASER_TARGET_EMAIL");
        // Lock is released automatically
    }

    #[test]
    fn test_config_from_env_invalid_port() {
        let _lock = ENV_LOCK.lock().unwrap(); // Acquire lock

        // Setup: Set valid required vars first
        env::set_var("MAIL_LASER_TARGET_EMAIL", "test@example.com");
        env::set_var("MAIL_LASER_WEBHOOK_URL", "http://localhost:8000/webhook");
        // Clear optional port vars initially for clean test cases
        env::remove_var("MAIL_LASER_PORT");
        env::remove_var("MAIL_LASER_HEALTH_PORT");

        // Test invalid MAIL_LASER_PORT
        env::set_var("MAIL_LASER_PORT", "not-a-port");
        if let Err(e) = Config::from_env() {
            let err_msg = e.to_string();
            assert!(err_msg.contains("MAIL_LASER_PORT"), "Error message should mention MAIL_LASER_PORT");
            assert!(err_msg.contains("not-a-port"), "Error message should contain the invalid value");
        } else {
            panic!("Expected an error for invalid MAIL_LASER_PORT, but got Ok");
        }

        // Reset MAIL_LASER_PORT to valid for the next check
        env::set_var("MAIL_LASER_PORT", "3000");

        // Test invalid MAIL_LASER_HEALTH_PORT
        env::set_var("MAIL_LASER_HEALTH_PORT", "invalid");
        if let Err(e) = Config::from_env() {
            let err_msg_2 = e.to_string();
            assert!(err_msg_2.contains("MAIL_LASER_HEALTH_PORT"), "Error message should mention MAIL_LASER_HEALTH_PORT");
            assert!(err_msg_2.contains("invalid"), "Error message should contain the invalid value");
        } else {
            panic!("Expected an error for invalid MAIL_LASER_HEALTH_PORT, but got Ok");
        }

        // Teardown: Clear all variables used in this test
        env::remove_var("MAIL_LASER_TARGET_EMAIL");
        env::remove_var("MAIL_LASER_WEBHOOK_URL");
        env::remove_var("MAIL_LASER_PORT");
        env::remove_var("MAIL_LASER_HEALTH_PORT");
        // Lock is released automatically
    }
}
