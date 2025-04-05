#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config; 

    // Helper to create a default config for testing
    // NOTE: Adjust this based on your actual Config struct definition and required fields.
    // If your Config::load() handles defaults or uses dotenv, you might need a different setup.
    // For simplicity, assuming direct field access is possible here.
    fn test_config() -> Config {
        // Provide minimal valid configuration for testing client creation
        Config {
            webhook_url: "http://example.com/webhook".to_string(),
            // Add default values for any other fields required by Config
            // e.g., smtp_port: 2525, smtp_host: "127.0.0.1".to_string(), etc.
            // If loading from env is mandatory, consider setting test env vars
            // or mocking the config loading process.
        }
    }

    #[test]
    fn test_webhook_client_user_agent() {
        let config = test_config();
        let client = WebhookClient::new(config);

        // Construct the expected user agent string using compile-time env vars
        let expected_user_agent = format!(
            "{}/{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        );

        // Assert that the client's user_agent field matches the expected format
        assert_eq!(client.user_agent, expected_user_agent);
        
        // Optionally, assert against the known current version for extra safety
        assert_eq!(client.user_agent, "mail_laser/0.1.0"); 
    }
}
