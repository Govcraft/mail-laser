//! Unit tests for the configuration loading logic (`Config::from_env`).
//! Note: These tests modify environment variables and rely on sequential execution
//! or external locking (like the `ENV_LOCK` mutex previously in `mod.rs`) if run in parallel
//! to avoid interference.

// The tests module was likely separated from mod.rs.
// Assuming `Config` is brought into scope via `super::*` or similar in the actual project structure.
// If `Config` is defined in `src/config/mod.rs`, `use crate::config::Config;` might be needed here,
// but I will proceed based on the provided file content which assumes `Config` is available.
// use crate::config::Config; // Example import if needed

#[cfg(test)]
mod tests {
    // Assuming Config is in the parent module (src/config/mod.rs)
    use crate::config::Config; // Assuming Config is in the parent module
    use std::env;
    use std::sync::Mutex; // Using Mutex to serialize tests modifying env vars
    use once_cell::sync::Lazy; // For static Mutex initialization

    // Static Mutex to ensure tests modifying environment variables run serially.
    static ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    /// Helper function to clear potentially conflicting environment variables before a test.
    fn clear_test_env_vars() {
        env::remove_var("MAIL_LASER_TARGET_EMAILS"); // Use plural form
        env::remove_var("MAIL_LASER_WEBHOOK_URL");
        env::remove_var("MAIL_LASER_BIND_ADDRESS");
        env::remove_var("MAIL_LASER_PORT");
        env::remove_var("MAIL_LASER_HEALTH_BIND_ADDRESS"); // Ensure all relevant vars are cleared
        env::remove_var("MAIL_LASER_HEALTH_PORT");
    }

    #[tokio::test]
    async fn test_config_from_env_all_set() {
        let _lock = ENV_LOCK.lock().unwrap(); // Acquire lock
        clear_test_env_vars(); // Ensure clean state

        // --- Setup ---
        // Set all relevant environment variables for this test case.
        env::set_var("MAIL_LASER_TARGET_EMAILS", "test1@example.com, test2@example.com"); // Set multiple emails
        env::set_var("MAIL_LASER_WEBHOOK_URL", "https://webhook.example.com/endpoint");
        env::set_var("MAIL_LASER_BIND_ADDRESS", "127.0.0.1");
        env::set_var("MAIL_LASER_PORT", "3000"); // Use a non-default port
        env::set_var("MAIL_LASER_HEALTH_BIND_ADDRESS", "192.168.1.1"); // Use non-default address
        env::set_var("MAIL_LASER_HEALTH_PORT", "9090"); // Use non-default port

        // --- Action ---
        // Attempt to load configuration from the set environment variables.
        let config = Config::from_env().expect("Config loading failed when all vars were set");

        // --- Verification ---
        // Verify that all configuration fields match the values set in the environment.
        assert_eq!(config.target_emails, vec!["test1@example.com".to_string(), "test2@example.com".to_string()]); // Check the vector
        assert_eq!(config.webhook_url, "https://webhook.example.com/endpoint");
        assert_eq!(config.smtp_bind_address, "127.0.0.1");
        assert_eq!(config.smtp_port, 3000);
        assert_eq!(config.health_check_bind_address, "192.168.1.1");
        assert_eq!(config.health_check_port, 9090);

        // --- Teardown (implicit via clear_test_env_vars at start of next test) ---
    }

    #[tokio::test]
    async fn test_config_default_values() {
        let _lock = ENV_LOCK.lock().unwrap(); // Acquire lock
        clear_test_env_vars(); // Ensure clean state, removing optional vars

        // --- Setup ---
        // Set only the required environment variables. Optional ones remain unset.
        env::set_var("MAIL_LASER_TARGET_EMAILS", "required@example.com"); // Set required emails
        env::set_var("MAIL_LASER_WEBHOOK_URL", "https://required.example.com/hook");

        // --- Action ---
        // Load configuration. Expect defaults for optional variables.
        let config = Config::from_env().expect("Config loading failed with only required vars set");

        // --- Verification ---
        // Verify required fields are set correctly.
        assert_eq!(config.target_emails, vec!["required@example.com".to_string()]); // Check the vector (single email case)
        assert_eq!(config.webhook_url, "https://required.example.com/hook");
        // Verify that default values are used for the optional fields.
        assert_eq!(config.smtp_bind_address, "0.0.0.0", "Default SMTP bind address mismatch");
        assert_eq!(config.smtp_port, 2525, "Default SMTP port mismatch"); // Default is 2525
        assert_eq!(config.health_check_bind_address, "0.0.0.0", "Default health bind address mismatch");
        assert_eq!(config.health_check_port, 8080, "Default health port mismatch"); // Default is 8080

        // --- Teardown (implicit via clear_test_env_vars at start of next test) ---
    }

    #[tokio::test]
    async fn test_config_missing_required_vars() {
        let _lock = ENV_LOCK.lock().unwrap(); // Acquire lock
        clear_test_env_vars(); // Ensure clean state, including required vars

        // --- Action & Verification ---
        // Attempt to load config when MAIL_LASER_TARGET_EMAIL is missing.
        // Attempt to load config when MAIL_LASER_TARGET_EMAILS is missing.
        let result_missing_target = Config::from_env();
        assert!(result_missing_target.is_err(), "Expected error when TARGET_EMAILS is missing");
        assert!(result_missing_target.unwrap_err().to_string().contains("MAIL_LASER_TARGET_EMAILS"), "Error message should mention TARGET_EMAILS");

        // --- Setup for next sub-case ---
        env::set_var("MAIL_LASER_TARGET_EMAILS", "test@example.com"); // Set target emails

        // --- Action & Verification ---
        // Attempt to load config when MAIL_LASER_WEBHOOK_URL is missing.
        let result_missing_webhook = Config::from_env();
        assert!(result_missing_webhook.is_err(), "Expected error when WEBHOOK_URL is missing");
        assert!(result_missing_webhook.unwrap_err().to_string().contains("MAIL_LASER_WEBHOOK_URL"), "Error message should mention WEBHOOK_URL");

        // --- Teardown (implicit via clear_test_env_vars at start of next test) ---
    }

    #[tokio::test]
    async fn test_config_invalid_port_values() {
        let _lock = ENV_LOCK.lock().unwrap(); // Acquire lock
        clear_test_env_vars(); // Ensure clean state

        // --- Setup ---
        // Set required variables.
        env::set_var("MAIL_LASER_TARGET_EMAILS", "test@example.com");
        env::set_var("MAIL_LASER_WEBHOOK_URL", "https://webhook.example.com");

        // --- Action & Verification (Invalid SMTP Port) ---
        env::set_var("MAIL_LASER_PORT", "not-a-number");
        let result_invalid_smtp = Config::from_env();
        assert!(result_invalid_smtp.is_err(), "Expected error for invalid SMTP port");
        assert!(result_invalid_smtp.unwrap_err().to_string().contains("MAIL_LASER_PORT"), "Error message should mention MAIL_LASER_PORT");

        // --- Setup for next sub-case ---
        env::set_var("MAIL_LASER_PORT", "2525"); // Reset to valid SMTP port

        // --- Action & Verification (Invalid Health Port) ---
        env::set_var("MAIL_LASER_HEALTH_PORT", "also-invalid");
        let result_invalid_health = Config::from_env();
        assert!(result_invalid_health.is_err(), "Expected error for invalid health port");
        assert!(result_invalid_health.unwrap_err().to_string().contains("MAIL_LASER_HEALTH_PORT"), "Error message should mention MAIL_LASER_HEALTH_PORT");

        // --- Teardown (implicit via clear_test_env_vars at start of next test) ---
    }

    #[tokio::test]
    async fn test_config_target_emails_parsing() {
        let _lock = ENV_LOCK.lock().unwrap(); // Acquire lock
        clear_test_env_vars(); // Ensure clean state

        // --- Setup ---
        env::set_var("MAIL_LASER_WEBHOOK_URL", "https://webhook.example.com");

        // --- Test Case 1: Single Email ---
        env::set_var("MAIL_LASER_TARGET_EMAILS", "single@example.com");
        let config1 = Config::from_env().expect("Config loading failed for single email");
        assert_eq!(config1.target_emails, vec!["single@example.com".to_string()]);

        // --- Test Case 2: Multiple Emails with Whitespace ---
        env::set_var("MAIL_LASER_TARGET_EMAILS", "  spaced1@example.com , spaced2@example.com  ,third@here.net");
        let config2 = Config::from_env().expect("Config loading failed for emails with whitespace");
        assert_eq!(config2.target_emails, vec![
            "spaced1@example.com".to_string(),
            "spaced2@example.com".to_string(),
            "third@here.net".to_string(),
        ]);

        // --- Test Case 3: Empty String (Should Error) ---
        env::set_var("MAIL_LASER_TARGET_EMAILS", "");
        let result_empty = Config::from_env();
        assert!(result_empty.is_err(), "Expected error for empty MAIL_LASER_TARGET_EMAILS");
        assert!(result_empty.unwrap_err().to_string().contains("MAIL_LASER_TARGET_EMAILS cannot be empty"), "Error message should indicate non-empty requirement");

        // --- Test Case 4: Only Whitespace/Commas (Should Error) ---
         env::set_var("MAIL_LASER_TARGET_EMAILS", " ,, , ");
         let result_whitespace = Config::from_env();
         assert!(result_whitespace.is_err(), "Expected error for whitespace/comma only MAIL_LASER_TARGET_EMAILS");
         assert!(result_whitespace.unwrap_err().to_string().contains("MAIL_LASER_TARGET_EMAILS must contain at least one valid email"), "Error message should indicate need for valid emails");


        // --- Teardown (implicit via clear_test_env_vars at start of next test) ---
    }

}
