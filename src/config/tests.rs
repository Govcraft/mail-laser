#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tokio::test;

    #[test]
    async fn test_config_from_env() {
        // Set environment variables for testing
        env::set_var("MAIL_LASER_TARGET_EMAIL", "test@example.com");
        env::set_var("MAIL_LASER_WEBHOOK_URL", "https://webhook.example.com/endpoint");
        env::set_var("MAIL_LASER_BIND_ADDRESS", "127.0.0.1");
        env::set_var("MAIL_LASER_PORT", "2525");

        // Load config from environment
        let config = Config::from_env().expect("Failed to load config from environment in test");

        // Verify config values
        assert_eq!(config.target_email, "test@example.com");
        assert_eq!(config.webhook_url, "https://webhook.example.com/endpoint");
        assert_eq!(config.smtp_bind_address, "127.0.0.1");
        assert_eq!(config.smtp_port, 2525);

        // Clean up environment variables
        env::remove_var("MAIL_LASER_TARGET_EMAIL");
        env::remove_var("MAIL_LASER_WEBHOOK_URL");
        env::remove_var("MAIL_LASER_BIND_ADDRESS");
        env::remove_var("MAIL_LASER_PORT");
    }

    #[test]
    async fn test_config_default_values() {
        // Set only required environment variables
        env::set_var("MAIL_LASER_TARGET_EMAIL", "test@example.com");
        env::set_var("MAIL_LASER_WEBHOOK_URL", "https://webhook.example.com/endpoint");
        
        // Remove optional variables if they exist
        env::remove_var("MAIL_LASER_BIND_ADDRESS");
        env::remove_var("MAIL_LASER_PORT");

        // Load config from environment
        let config = Config::from_env().expect("Failed to load config from environment in test");

        // Verify default values are used
        assert_eq!(config.smtp_bind_address, "0.0.0.0");
        assert_eq!(config.smtp_port, 25);

        // Clean up environment variables
        env::remove_var("MAIL_LASER_TARGET_EMAIL");
        env::remove_var("MAIL_LASER_WEBHOOK_URL");
    }

    #[test]
    async fn test_config_missing_required_vars() {
        // Remove all environment variables
        env::remove_var("MAIL_LASER_TARGET_EMAIL");
        env::remove_var("MAIL_LASER_WEBHOOK_URL");
        env::remove_var("MAIL_LASER_BIND_ADDRESS");
        env::remove_var("MAIL_LASER_PORT");

        // Attempt to load config
        let result = Config::from_env();
        
        // Verify it returns an error
        assert!(result.is_err());
    }
}
