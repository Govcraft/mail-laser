//! Manages application configuration loaded from environment variables.
//!
//! This module defines the `Config` struct which holds all runtime settings
//! and provides the `from_env` function to populate this struct. It supports
//! loading variables from a `.env` file via the `dotenv` crate and provides
//! default values for optional settings.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::env;

/// Holds the application's runtime configuration settings.
///
/// These settings are typically loaded from environment variables via `from_env`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The list of email addresses MailLaser will accept mail for. (Required: `MAIL_LASER_TARGET_EMAILS`, comma-separated)
    pub target_emails: Vec<String>,

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

    /// Header name prefixes to match and forward in the webhook payload.
    /// (Optional: `MAIL_LASER_HEADER_PREFIX`, comma-separated, Default: empty)
    pub header_prefixes: Vec<String>,

    /// Webhook request timeout in seconds. (Optional: `MAIL_LASER_WEBHOOK_TIMEOUT`, Default: 30)
    pub webhook_timeout_secs: u64,

    /// Max retry attempts on webhook delivery failure. (Optional: `MAIL_LASER_WEBHOOK_MAX_RETRIES`, Default: 3)
    pub webhook_max_retries: u32,

    /// Consecutive failures required to open the circuit breaker. (Optional: `MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD`, Default: 5)
    pub circuit_breaker_threshold: u32,

    /// Seconds before a tripped circuit breaker half-opens. (Optional: `MAIL_LASER_CIRCUIT_BREAKER_RESET`, Default: 60)
    pub circuit_breaker_reset_secs: u64,
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
    /// - Required environment variables (`MAIL_LASER_TARGET_EMAILS`, `MAIL_LASER_WEBHOOK_URL`) are missing or `MAIL_LASER_TARGET_EMAILS` is empty/invalid.
    /// - Optional port variables (`MAIL_LASER_PORT`, `MAIL_LASER_HEALTH_PORT`) are set but cannot be parsed as `u16`.
    pub fn from_env() -> Result<Self> {
        // Attempt to load variables from a .env file, if it exists. Ignore errors.
        let _ = dotenv::dotenv();

        // --- Required Variables ---
        // --- Required Variables ---
        let target_emails_str = match env::var("MAIL_LASER_TARGET_EMAILS") {
            Ok(val) => val,
            Err(e) => {
                let err_msg = "MAIL_LASER_TARGET_EMAILS environment variable must be set";
                log::error!("{}: {}", err_msg, e);
                return Err(anyhow!(e).context(err_msg));
            }
        };

        // Parse the comma-separated string into a Vec<String>, trimming whitespace
        let target_emails: Vec<String> = target_emails_str
            .split(',')
            .map(|email| email.trim().to_string()) // Trim whitespace from each part
            .filter(|email| !email.is_empty()) // Remove any empty strings resulting from extra commas or whitespace
            .collect();

        // Ensure at least one valid email was provided
        if target_emails.is_empty() {
            let err_msg = if target_emails_str.trim().is_empty() {
                "MAIL_LASER_TARGET_EMAILS cannot be empty"
            } else {
                "MAIL_LASER_TARGET_EMAILS must contain at least one valid email after trimming and splitting"
            };
            log::error!("{}", err_msg);
            return Err(anyhow!(err_msg.to_string()));
        }

        log::info!("Config: Using target_emails: {:?}", target_emails);

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

        let smtp_port_str = env::var("MAIL_LASER_PORT").unwrap_or_else(|_| "2525".to_string()); // Default SMTP port
        let smtp_port = match smtp_port_str.parse::<u16>() {
            Ok(port) => port,
            Err(e) => {
                let err_msg = format!(
                    "MAIL_LASER_PORT ('{}') must be a valid u16 port number",
                    smtp_port_str
                );
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
                log::info!(
                    "Config: Using default health_check_bind_address: {}",
                    default_val
                );
                default_val // Default: Listen on all interfaces
            });

        let health_check_port_str =
            env::var("MAIL_LASER_HEALTH_PORT").unwrap_or_else(|_| "8080".to_string()); // Default health check port
        let health_check_port = match health_check_port_str.parse::<u16>() {
            Ok(port) => port,
            Err(e) => {
                let err_msg = format!(
                    "MAIL_LASER_HEALTH_PORT ('{}') must be a valid u16 port number",
                    health_check_port_str
                );
                log::error!("{}: {}", err_msg, e); // Log specific error before returning
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!("Config: Using health_check_port: {}", health_check_port);

        // --- Optional: Header Prefixes ---
        let header_prefixes: Vec<String> = env::var("MAIL_LASER_HEADER_PREFIX")
            .map(|val| {
                val.split(',')
                    .map(|prefix| prefix.trim().to_string())
                    .filter(|prefix| !prefix.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        log::info!("Config: Using header_prefixes: {:?}", header_prefixes);

        // --- Optional: Resilience settings ---
        let webhook_timeout_secs: u64 = env::var("MAIL_LASER_WEBHOOK_TIMEOUT")
            .unwrap_or_else(|_| "30".to_string())
            .parse()
            .map_err(|e| anyhow!("MAIL_LASER_WEBHOOK_TIMEOUT must be a valid u64: {}", e))?;
        log::info!(
            "Config: Using webhook_timeout_secs: {}",
            webhook_timeout_secs
        );

        let webhook_max_retries: u32 = env::var("MAIL_LASER_WEBHOOK_MAX_RETRIES")
            .unwrap_or_else(|_| "3".to_string())
            .parse()
            .map_err(|e| anyhow!("MAIL_LASER_WEBHOOK_MAX_RETRIES must be a valid u32: {}", e))?;
        log::info!("Config: Using webhook_max_retries: {}", webhook_max_retries);

        let circuit_breaker_threshold: u32 = env::var("MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD")
            .unwrap_or_else(|_| "5".to_string())
            .parse()
            .map_err(|e| {
                anyhow!(
                    "MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD must be a valid u32: {}",
                    e
                )
            })?;
        log::info!(
            "Config: Using circuit_breaker_threshold: {}",
            circuit_breaker_threshold
        );

        let circuit_breaker_reset_secs: u64 = env::var("MAIL_LASER_CIRCUIT_BREAKER_RESET")
            .unwrap_or_else(|_| "60".to_string())
            .parse()
            .map_err(|e| {
                anyhow!(
                    "MAIL_LASER_CIRCUIT_BREAKER_RESET must be a valid u64: {}",
                    e
                )
            })?;
        log::info!(
            "Config: Using circuit_breaker_reset_secs: {}",
            circuit_breaker_reset_secs
        );

        // Construct the final Config object
        Ok(Config {
            target_emails,
            webhook_url,
            smtp_bind_address,
            smtp_port,
            health_check_bind_address,
            health_check_port,
            header_prefixes,
            webhook_timeout_secs,
            webhook_max_retries,
            circuit_breaker_threshold,
            circuit_breaker_reset_secs,
        })
    }
}

// The inline tests module has been moved to src/config/tests.rs
// and is included via `mod tests;` below.

// Include the tests defined in tests.rs
mod tests;
