//! Manages application configuration loaded from environment variables.
//!
//! This module defines the `Config` struct which holds all runtime settings
//! and provides the `from_env` function to populate this struct. It supports
//! loading variables from a `.env` file via the `dotenv` crate and provides
//! default values for optional settings.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;

const DEFAULT_MAX_MESSAGE_SIZE_BYTES: u64 = 26_214_400; // 25 MiB
const DEFAULT_MAX_ATTACHMENT_SIZE_BYTES: u64 = 10_485_760; // 10 MiB

/// DMARC validation mode for inbound messages.
///
/// * `Off` — no validation (backward-compatible default). No DNS lookups performed.
/// * `Monitor` — validate and log, but always accept. Useful when rolling DMARC out.
/// * `Enforce` — reject with `550 5.7.1` on `fail`; temperror handling controlled by
///   [`DmarcTempErrorAction`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DmarcMode {
    Off,
    Monitor,
    Enforce,
}

/// Policy for DMARC temperror (DNS SERVFAIL, timeout, unreachable resolver) in
/// `Enforce` mode. Ignored in `Off` and `Monitor`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DmarcTempErrorAction {
    /// Return `451 4.7.0` — fail-closed. Sending MTA retries later.
    Reject,
    /// Accept the message and log. Fail-open.
    Accept,
}

/// How attachments are delivered to the webhook consumer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AttachmentDelivery {
    /// Base64-encode attachments inline in the JSON webhook payload.
    Inline,
    /// Upload to an S3-compatible bucket and send a URL in the payload.
    S3(S3Settings),
}

/// Settings for the S3-compatible attachment delivery backend.
///
/// `endpoint` is set for non-AWS S3-compatible stores (MinIO, R2, Wasabi).
/// When `presign_ttl_secs` is `Some`, each uploaded object gets a presigned
/// GET URL in addition to the bare object URL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct S3Settings {
    pub bucket: String,
    pub region: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub key_prefix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presign_ttl_secs: Option<u64>,
}

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

    /// Path to the Cedar policy file. (Required: `MAIL_LASER_CEDAR_POLICIES`)
    pub cedar_policies_path: PathBuf,

    /// Optional path to a Cedar entities JSON file. (Optional: `MAIL_LASER_CEDAR_ENTITIES`)
    pub cedar_entities_path: Option<PathBuf>,

    /// Max total SMTP message size in bytes. (Optional: `MAIL_LASER_MAX_MESSAGE_SIZE`, Default: 26_214_400)
    pub max_message_size_bytes: u64,

    /// Max per-attachment size in bytes. (Optional: `MAIL_LASER_MAX_ATTACHMENT_SIZE`, Default: 10_485_760)
    pub max_attachment_size_bytes: u64,

    /// How attachments are delivered to the webhook consumer.
    /// (Optional: `MAIL_LASER_ATTACHMENT_DELIVERY`, Default: `inline`)
    pub attachment_delivery: AttachmentDelivery,

    /// DMARC validation mode. (Optional: `MAIL_LASER_DMARC_MODE`, Default: `off`)
    pub dmarc_mode: DmarcMode,

    /// Overall DMARC evaluation timeout in seconds (wraps all SPF + DKIM + DMARC DNS
    /// lookups). (Optional: `MAIL_LASER_DMARC_DNS_TIMEOUT`, Default: 5)
    pub dmarc_dns_timeout_secs: u64,

    /// Optional explicit DNS servers to use for DMARC lookups (`ip:port`).
    /// When empty, the system resolver is used.
    /// (Optional: `MAIL_LASER_DMARC_DNS_SERVERS`, comma-separated, Default: empty)
    pub dmarc_dns_servers: Vec<String>,

    /// How to handle DMARC temperror in `Enforce` mode.
    /// (Optional: `MAIL_LASER_DMARC_TEMPERROR_ACTION`, Default: `reject`)
    pub dmarc_temperror_action: DmarcTempErrorAction,
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
    /// - Required environment variables (`MAIL_LASER_TARGET_EMAILS`, `MAIL_LASER_WEBHOOK_URL`,
    ///   `MAIL_LASER_CEDAR_POLICIES`) are missing or `MAIL_LASER_TARGET_EMAILS` is empty/invalid.
    /// - Optional port variables (`MAIL_LASER_PORT`, `MAIL_LASER_HEALTH_PORT`) are set but cannot be parsed as `u16`.
    /// - `MAIL_LASER_ATTACHMENT_DELIVERY=s3` but required S3 fields are missing.
    pub fn from_env() -> Result<Self> {
        // Attempt to load variables from a .env file, if it exists. Ignore errors.
        let _ = dotenv::dotenv();

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
            .map(|email| email.trim().to_string())
            .filter(|email| !email.is_empty())
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
                log::error!("{}: {}", err_msg, e);
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!("Config: Using webhook_url: {}", webhook_url);

        let cedar_policies_path = match env::var("MAIL_LASER_CEDAR_POLICIES") {
            Ok(val) => PathBuf::from(val),
            Err(e) => {
                let err_msg = "MAIL_LASER_CEDAR_POLICIES environment variable must be set";
                log::error!("{}: {}", err_msg, e);
                return Err(anyhow!(e).context(err_msg));
            }
        };
        log::info!(
            "Config: Using cedar_policies_path: {}",
            cedar_policies_path.display()
        );

        let cedar_entities_path = env::var("MAIL_LASER_CEDAR_ENTITIES")
            .ok()
            .map(PathBuf::from);
        if let Some(ref p) = cedar_entities_path {
            log::info!("Config: Using cedar_entities_path: {}", p.display());
        }

        // --- Optional Variables with Defaults ---
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

        let smtp_port_str = env::var("MAIL_LASER_PORT").unwrap_or_else(|_| "2525".to_string());
        let smtp_port = match smtp_port_str.parse::<u16>() {
            Ok(port) => port,
            Err(e) => {
                let err_msg = format!(
                    "MAIL_LASER_PORT ('{}') must be a valid u16 port number",
                    smtp_port_str
                );
                log::error!("{}: {}", err_msg, e);
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
                default_val
            });

        let health_check_port_str =
            env::var("MAIL_LASER_HEALTH_PORT").unwrap_or_else(|_| "8080".to_string());
        let health_check_port = match health_check_port_str.parse::<u16>() {
            Ok(port) => port,
            Err(e) => {
                let err_msg = format!(
                    "MAIL_LASER_HEALTH_PORT ('{}') must be a valid u16 port number",
                    health_check_port_str
                );
                log::error!("{}: {}", err_msg, e);
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

        // --- Optional: Attachment size caps ---
        let max_message_size_bytes: u64 = env::var("MAIL_LASER_MAX_MESSAGE_SIZE")
            .unwrap_or_else(|_| DEFAULT_MAX_MESSAGE_SIZE_BYTES.to_string())
            .parse()
            .map_err(|e| anyhow!("MAIL_LASER_MAX_MESSAGE_SIZE must be a valid u64: {}", e))?;
        if max_message_size_bytes == 0 {
            return Err(anyhow!("MAIL_LASER_MAX_MESSAGE_SIZE must be greater than 0"));
        }
        log::info!(
            "Config: Using max_message_size_bytes: {}",
            max_message_size_bytes
        );

        let max_attachment_size_bytes: u64 = env::var("MAIL_LASER_MAX_ATTACHMENT_SIZE")
            .unwrap_or_else(|_| DEFAULT_MAX_ATTACHMENT_SIZE_BYTES.to_string())
            .parse()
            .map_err(|e| anyhow!("MAIL_LASER_MAX_ATTACHMENT_SIZE must be a valid u64: {}", e))?;
        if max_attachment_size_bytes == 0 {
            return Err(anyhow!(
                "MAIL_LASER_MAX_ATTACHMENT_SIZE must be greater than 0"
            ));
        }
        log::info!(
            "Config: Using max_attachment_size_bytes: {}",
            max_attachment_size_bytes
        );

        // --- Optional: Attachment delivery mode ---
        let attachment_delivery = parse_attachment_delivery()?;
        log::info!(
            "Config: Using attachment_delivery: {:?}",
            attachment_delivery
        );

        // --- Optional: DMARC settings ---
        let dmarc_mode = parse_dmarc_mode()?;
        log::info!("Config: Using dmarc_mode: {:?}", dmarc_mode);

        let dmarc_dns_timeout_secs: u64 = env::var("MAIL_LASER_DMARC_DNS_TIMEOUT")
            .unwrap_or_else(|_| "5".to_string())
            .parse()
            .map_err(|e| anyhow!("MAIL_LASER_DMARC_DNS_TIMEOUT must be a valid u64: {}", e))?;
        if dmarc_dns_timeout_secs == 0 {
            return Err(anyhow!("MAIL_LASER_DMARC_DNS_TIMEOUT must be greater than 0"));
        }
        log::info!(
            "Config: Using dmarc_dns_timeout_secs: {}",
            dmarc_dns_timeout_secs
        );

        let dmarc_dns_servers: Vec<String> = env::var("MAIL_LASER_DMARC_DNS_SERVERS")
            .map(|val| {
                val.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        log::info!("Config: Using dmarc_dns_servers: {:?}", dmarc_dns_servers);

        let dmarc_temperror_action = parse_dmarc_temperror_action()?;
        log::info!(
            "Config: Using dmarc_temperror_action: {:?}",
            dmarc_temperror_action
        );

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
            cedar_policies_path,
            cedar_entities_path,
            max_message_size_bytes,
            max_attachment_size_bytes,
            attachment_delivery,
            dmarc_mode,
            dmarc_dns_timeout_secs,
            dmarc_dns_servers,
            dmarc_temperror_action,
        })
    }
}

fn parse_dmarc_mode() -> Result<DmarcMode> {
    let mode = env::var("MAIL_LASER_DMARC_MODE")
        .unwrap_or_else(|_| "off".to_string())
        .to_lowercase();
    match mode.as_str() {
        "off" => Ok(DmarcMode::Off),
        "monitor" => Ok(DmarcMode::Monitor),
        "enforce" => Ok(DmarcMode::Enforce),
        other => Err(anyhow!(
            "MAIL_LASER_DMARC_MODE must be 'off', 'monitor', or 'enforce' (got '{}')",
            other
        )),
    }
}

fn parse_dmarc_temperror_action() -> Result<DmarcTempErrorAction> {
    let action = env::var("MAIL_LASER_DMARC_TEMPERROR_ACTION")
        .unwrap_or_else(|_| "reject".to_string())
        .to_lowercase();
    match action.as_str() {
        "reject" => Ok(DmarcTempErrorAction::Reject),
        "accept" => Ok(DmarcTempErrorAction::Accept),
        other => Err(anyhow!(
            "MAIL_LASER_DMARC_TEMPERROR_ACTION must be 'reject' or 'accept' (got '{}')",
            other
        )),
    }
}

fn parse_attachment_delivery() -> Result<AttachmentDelivery> {
    let mode = env::var("MAIL_LASER_ATTACHMENT_DELIVERY")
        .unwrap_or_else(|_| "inline".to_string())
        .to_lowercase();

    match mode.as_str() {
        "inline" => Ok(AttachmentDelivery::Inline),
        "s3" => Ok(AttachmentDelivery::S3(parse_s3_settings()?)),
        other => Err(anyhow!(
            "MAIL_LASER_ATTACHMENT_DELIVERY must be 'inline' or 's3' (got '{}')",
            other
        )),
    }
}

fn parse_s3_settings() -> Result<S3Settings> {
    let bucket = env::var("MAIL_LASER_S3_BUCKET").map_err(|e| {
        anyhow!(e).context(
            "MAIL_LASER_S3_BUCKET must be set when MAIL_LASER_ATTACHMENT_DELIVERY=s3",
        )
    })?;
    let region = env::var("MAIL_LASER_S3_REGION").map_err(|e| {
        anyhow!(e).context(
            "MAIL_LASER_S3_REGION must be set when MAIL_LASER_ATTACHMENT_DELIVERY=s3",
        )
    })?;

    let endpoint = env::var("MAIL_LASER_S3_ENDPOINT").ok();
    let key_prefix = env::var("MAIL_LASER_S3_KEY_PREFIX").unwrap_or_default();

    let presign_ttl_secs = match env::var("MAIL_LASER_S3_PRESIGN_TTL") {
        Ok(val) => {
            let parsed: u64 = val
                .parse()
                .map_err(|e| anyhow!("MAIL_LASER_S3_PRESIGN_TTL must be a valid u64: {}", e))?;
            if parsed == 0 {
                return Err(anyhow!("MAIL_LASER_S3_PRESIGN_TTL must be greater than 0"));
            }
            Some(parsed)
        }
        Err(_) => None,
    };

    Ok(S3Settings {
        bucket,
        region,
        endpoint,
        key_prefix,
        presign_ttl_secs,
    })
}

// Include the tests defined in tests.rs
#[cfg(test)]
mod tests;
