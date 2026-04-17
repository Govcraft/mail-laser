//! Unit tests for the configuration loading logic (`Config::from_env`).
//! Note: These tests modify environment variables and rely on sequential execution
//! or external locking (like the `ENV_LOCK` mutex previously in `mod.rs`) if run in parallel
//! to avoid interference.

use crate::config::{AttachmentDelivery, Config};
use once_cell::sync::Lazy;
use std::env;
use std::path::PathBuf;
use std::sync::Mutex;

// Static Mutex to ensure tests modifying environment variables run serially.
static ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Helper function to clear potentially conflicting environment variables before a test.
fn clear_test_env_vars() {
    env::remove_var("MAIL_LASER_TARGET_EMAILS");
    env::remove_var("MAIL_LASER_WEBHOOK_URL");
    env::remove_var("MAIL_LASER_BIND_ADDRESS");
    env::remove_var("MAIL_LASER_PORT");
    env::remove_var("MAIL_LASER_HEALTH_BIND_ADDRESS");
    env::remove_var("MAIL_LASER_HEALTH_PORT");
    env::remove_var("MAIL_LASER_HEADER_PREFIX");
    env::remove_var("MAIL_LASER_WEBHOOK_TIMEOUT");
    env::remove_var("MAIL_LASER_WEBHOOK_MAX_RETRIES");
    env::remove_var("MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD");
    env::remove_var("MAIL_LASER_CIRCUIT_BREAKER_RESET");
    env::remove_var("MAIL_LASER_CEDAR_POLICIES");
    env::remove_var("MAIL_LASER_CEDAR_ENTITIES");
    env::remove_var("MAIL_LASER_MAX_MESSAGE_SIZE");
    env::remove_var("MAIL_LASER_MAX_ATTACHMENT_SIZE");
    env::remove_var("MAIL_LASER_ATTACHMENT_DELIVERY");
    env::remove_var("MAIL_LASER_S3_BUCKET");
    env::remove_var("MAIL_LASER_S3_REGION");
    env::remove_var("MAIL_LASER_S3_ENDPOINT");
    env::remove_var("MAIL_LASER_S3_KEY_PREFIX");
    env::remove_var("MAIL_LASER_S3_PRESIGN_TTL");
}

/// Sets the minimum variables required for `Config::from_env` to succeed.
fn set_required_env() {
    env::set_var("MAIL_LASER_TARGET_EMAILS", "required@example.com");
    env::set_var(
        "MAIL_LASER_WEBHOOK_URL",
        "https://required.example.com/hook",
    );
    env::set_var("MAIL_LASER_CEDAR_POLICIES", "/tmp/policies.cedar");
}

#[tokio::test]
async fn test_config_from_env_all_set() {
    let _lock = ENV_LOCK.lock().unwrap();
    clear_test_env_vars();

    env::set_var(
        "MAIL_LASER_TARGET_EMAILS",
        "test1@example.com, test2@example.com",
    );
    env::set_var(
        "MAIL_LASER_WEBHOOK_URL",
        "https://webhook.example.com/endpoint",
    );
    env::set_var("MAIL_LASER_BIND_ADDRESS", "127.0.0.1");
    env::set_var("MAIL_LASER_PORT", "3000");
    env::set_var("MAIL_LASER_HEALTH_BIND_ADDRESS", "192.168.1.1");
    env::set_var("MAIL_LASER_HEALTH_PORT", "9090");
    env::set_var("MAIL_LASER_HEADER_PREFIX", "X-Custom, X-My-App");
    env::set_var("MAIL_LASER_WEBHOOK_TIMEOUT", "15");
    env::set_var("MAIL_LASER_WEBHOOK_MAX_RETRIES", "5");
    env::set_var("MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD", "10");
    env::set_var("MAIL_LASER_CIRCUIT_BREAKER_RESET", "120");
    env::set_var("MAIL_LASER_CEDAR_POLICIES", "/etc/mail-laser/policies.cedar");
    env::set_var("MAIL_LASER_CEDAR_ENTITIES", "/etc/mail-laser/entities.json");
    env::set_var("MAIL_LASER_MAX_MESSAGE_SIZE", "1048576");
    env::set_var("MAIL_LASER_MAX_ATTACHMENT_SIZE", "524288");

    let config = Config::from_env().expect("Config loading failed when all vars were set");

    assert_eq!(
        config.target_emails,
        vec![
            "test1@example.com".to_string(),
            "test2@example.com".to_string()
        ]
    );
    assert_eq!(config.webhook_url, "https://webhook.example.com/endpoint");
    assert_eq!(config.smtp_bind_address, "127.0.0.1");
    assert_eq!(config.smtp_port, 3000);
    assert_eq!(config.health_check_bind_address, "192.168.1.1");
    assert_eq!(config.health_check_port, 9090);
    assert_eq!(
        config.header_prefixes,
        vec!["X-Custom".to_string(), "X-My-App".to_string()]
    );
    assert_eq!(config.webhook_timeout_secs, 15);
    assert_eq!(config.webhook_max_retries, 5);
    assert_eq!(config.circuit_breaker_threshold, 10);
    assert_eq!(config.circuit_breaker_reset_secs, 120);
    assert_eq!(
        config.cedar_policies_path,
        PathBuf::from("/etc/mail-laser/policies.cedar")
    );
    assert_eq!(
        config.cedar_entities_path,
        Some(PathBuf::from("/etc/mail-laser/entities.json"))
    );
    assert_eq!(config.max_message_size_bytes, 1_048_576);
    assert_eq!(config.max_attachment_size_bytes, 524_288);
    assert_eq!(config.attachment_delivery, AttachmentDelivery::Inline);
}

#[tokio::test]
async fn test_config_default_values() {
    let _lock = ENV_LOCK.lock().unwrap();
    clear_test_env_vars();
    set_required_env();

    let config = Config::from_env().expect("Config loading failed with only required vars set");

    assert_eq!(
        config.target_emails,
        vec!["required@example.com".to_string()]
    );
    assert_eq!(config.webhook_url, "https://required.example.com/hook");
    assert_eq!(config.smtp_bind_address, "0.0.0.0");
    assert_eq!(config.smtp_port, 2525);
    assert_eq!(config.health_check_bind_address, "0.0.0.0");
    assert_eq!(config.health_check_port, 8080);
    assert!(config.header_prefixes.is_empty());
    assert_eq!(config.webhook_timeout_secs, 30);
    assert_eq!(config.webhook_max_retries, 3);
    assert_eq!(config.circuit_breaker_threshold, 5);
    assert_eq!(config.circuit_breaker_reset_secs, 60);
    assert_eq!(
        config.cedar_policies_path,
        PathBuf::from("/tmp/policies.cedar")
    );
    assert!(config.cedar_entities_path.is_none());
    assert_eq!(config.max_message_size_bytes, 26_214_400);
    assert_eq!(config.max_attachment_size_bytes, 10_485_760);
    assert_eq!(config.attachment_delivery, AttachmentDelivery::Inline);
}

#[tokio::test]
async fn test_config_missing_required_vars() {
    let _lock = ENV_LOCK.lock().unwrap();
    clear_test_env_vars();

    let result_missing_target = Config::from_env();
    assert!(result_missing_target.is_err());
    assert!(result_missing_target
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_TARGET_EMAILS"));

    env::set_var("MAIL_LASER_TARGET_EMAILS", "test@example.com");
    let result_missing_webhook = Config::from_env();
    assert!(result_missing_webhook.is_err());
    assert!(result_missing_webhook
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_WEBHOOK_URL"));

    env::set_var("MAIL_LASER_WEBHOOK_URL", "https://webhook.example.com");
    let result_missing_cedar = Config::from_env();
    assert!(result_missing_cedar.is_err());
    assert!(result_missing_cedar
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_CEDAR_POLICIES"));
}

#[tokio::test]
async fn test_config_invalid_port_values() {
    let _lock = ENV_LOCK.lock().unwrap();
    clear_test_env_vars();
    set_required_env();

    env::set_var("MAIL_LASER_PORT", "not-a-number");
    let result_invalid_smtp = Config::from_env();
    assert!(result_invalid_smtp.is_err());
    assert!(result_invalid_smtp
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_PORT"));

    env::set_var("MAIL_LASER_PORT", "2525");
    env::set_var("MAIL_LASER_HEALTH_PORT", "also-invalid");
    let result_invalid_health = Config::from_env();
    assert!(result_invalid_health.is_err());
    assert!(result_invalid_health
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_HEALTH_PORT"));
}

#[tokio::test]
async fn test_config_target_emails_parsing() {
    let _lock = ENV_LOCK.lock().unwrap();
    clear_test_env_vars();

    env::set_var("MAIL_LASER_WEBHOOK_URL", "https://webhook.example.com");
    env::set_var("MAIL_LASER_CEDAR_POLICIES", "/tmp/policies.cedar");

    env::set_var("MAIL_LASER_TARGET_EMAILS", "single@example.com");
    let config1 = Config::from_env().expect("Config loading failed for single email");
    assert_eq!(
        config1.target_emails,
        vec!["single@example.com".to_string()]
    );

    env::set_var(
        "MAIL_LASER_TARGET_EMAILS",
        "  spaced1@example.com , spaced2@example.com  ,third@here.net",
    );
    let config2 = Config::from_env().expect("Config loading failed for emails with whitespace");
    assert_eq!(
        config2.target_emails,
        vec![
            "spaced1@example.com".to_string(),
            "spaced2@example.com".to_string(),
            "third@here.net".to_string(),
        ]
    );

    env::set_var("MAIL_LASER_TARGET_EMAILS", "");
    let result_empty = Config::from_env();
    assert!(result_empty.is_err());
    assert!(result_empty
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_TARGET_EMAILS cannot be empty"));

    env::set_var("MAIL_LASER_TARGET_EMAILS", " ,, , ");
    let result_whitespace = Config::from_env();
    assert!(result_whitespace.is_err());
    assert!(result_whitespace
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_TARGET_EMAILS must contain at least one valid email"));
}

#[tokio::test]
async fn test_config_header_prefix_parsing() {
    let _lock = ENV_LOCK.lock().unwrap();
    clear_test_env_vars();
    set_required_env();

    env::set_var("MAIL_LASER_HEADER_PREFIX", "X-Custom");
    let config1 = Config::from_env().expect("Config loading failed for single prefix");
    assert_eq!(config1.header_prefixes, vec!["X-Custom".to_string()]);

    env::set_var(
        "MAIL_LASER_HEADER_PREFIX",
        "  X-Custom , X-My-App  , X-Third ",
    );
    let config2 = Config::from_env().expect("Config loading failed for multiple prefixes");
    assert_eq!(
        config2.header_prefixes,
        vec![
            "X-Custom".to_string(),
            "X-My-App".to_string(),
            "X-Third".to_string(),
        ]
    );

    env::set_var("MAIL_LASER_HEADER_PREFIX", "");
    let config3 = Config::from_env().expect("Config loading failed for empty prefix string");
    assert!(config3.header_prefixes.is_empty());

    env::set_var("MAIL_LASER_HEADER_PREFIX", " ,, , ");
    let config4 =
        Config::from_env().expect("Config loading failed for whitespace/comma prefix");
    assert!(config4.header_prefixes.is_empty());

    env::remove_var("MAIL_LASER_HEADER_PREFIX");
    let config5 = Config::from_env().expect("Config loading failed when prefix not set");
    assert!(config5.header_prefixes.is_empty());
}

#[tokio::test]
async fn test_config_resilience_fields() {
    let _lock = ENV_LOCK.lock().unwrap();
    clear_test_env_vars();
    set_required_env();

    env::set_var("MAIL_LASER_WEBHOOK_TIMEOUT", "10");
    env::set_var("MAIL_LASER_WEBHOOK_MAX_RETRIES", "7");
    env::set_var("MAIL_LASER_CIRCUIT_BREAKER_THRESHOLD", "3");
    env::set_var("MAIL_LASER_CIRCUIT_BREAKER_RESET", "90");

    let config = Config::from_env().expect("Config loading failed for resilience fields");
    assert_eq!(config.webhook_timeout_secs, 10);
    assert_eq!(config.webhook_max_retries, 7);
    assert_eq!(config.circuit_breaker_threshold, 3);
    assert_eq!(config.circuit_breaker_reset_secs, 90);

    env::set_var("MAIL_LASER_WEBHOOK_TIMEOUT", "not-a-number");
    let result = Config::from_env();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_WEBHOOK_TIMEOUT"));

    env::set_var("MAIL_LASER_WEBHOOK_TIMEOUT", "30");
    env::set_var("MAIL_LASER_WEBHOOK_MAX_RETRIES", "abc");
    let result = Config::from_env();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_WEBHOOK_MAX_RETRIES"));
}

#[tokio::test]
async fn test_config_size_caps_reject_zero_and_garbage() {
    let _lock = ENV_LOCK.lock().unwrap();
    clear_test_env_vars();
    set_required_env();

    env::set_var("MAIL_LASER_MAX_MESSAGE_SIZE", "0");
    let result = Config::from_env();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_MAX_MESSAGE_SIZE"));

    env::set_var("MAIL_LASER_MAX_MESSAGE_SIZE", "1048576");
    env::set_var("MAIL_LASER_MAX_ATTACHMENT_SIZE", "junk");
    let result = Config::from_env();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_MAX_ATTACHMENT_SIZE"));
}

#[tokio::test]
async fn test_config_attachment_delivery_s3_requires_bucket_and_region() {
    let _lock = ENV_LOCK.lock().unwrap();
    clear_test_env_vars();
    set_required_env();
    env::set_var("MAIL_LASER_ATTACHMENT_DELIVERY", "s3");

    let result_missing_bucket = Config::from_env();
    assert!(result_missing_bucket.is_err());
    assert!(result_missing_bucket
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_S3_BUCKET"));

    env::set_var("MAIL_LASER_S3_BUCKET", "my-bucket");
    let result_missing_region = Config::from_env();
    assert!(result_missing_region.is_err());
    assert!(result_missing_region
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_S3_REGION"));

    env::set_var("MAIL_LASER_S3_REGION", "us-east-1");
    env::set_var("MAIL_LASER_S3_ENDPOINT", "http://minio.local:9000");
    env::set_var("MAIL_LASER_S3_KEY_PREFIX", "inbound/");
    env::set_var("MAIL_LASER_S3_PRESIGN_TTL", "600");

    let config = Config::from_env().expect("S3 config should load with all required fields");
    match config.attachment_delivery {
        AttachmentDelivery::S3(settings) => {
            assert_eq!(settings.bucket, "my-bucket");
            assert_eq!(settings.region, "us-east-1");
            assert_eq!(settings.endpoint.as_deref(), Some("http://minio.local:9000"));
            assert_eq!(settings.key_prefix, "inbound/");
            assert_eq!(settings.presign_ttl_secs, Some(600));
        }
        other => panic!("expected S3 delivery, got {:?}", other),
    }
}

#[tokio::test]
async fn test_config_attachment_delivery_unknown_mode_errors() {
    let _lock = ENV_LOCK.lock().unwrap();
    clear_test_env_vars();
    set_required_env();
    env::set_var("MAIL_LASER_ATTACHMENT_DELIVERY", "ftp");

    let result = Config::from_env();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("MAIL_LASER_ATTACHMENT_DELIVERY"));
}
