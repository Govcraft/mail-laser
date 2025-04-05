use anyhow::Result;
use hyper::{Body, Client, Request};
use hyper_tls::HttpsConnector;
use log::{info, error};
use serde::{Serialize, Deserialize};
use crate::config::Config;

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailPayload {
    pub sender: String,
    pub subject: String,
    pub body: String,
}

pub struct WebhookClient {
    config: Config,
    client: Client<HttpsConnector<hyper::client::HttpConnector>>,
}

impl WebhookClient {
    pub fn new(config: Config) -> Self {
        // Create a connector with TLS support
        let https = HttpsConnector::new();
        
        // Build the hyper client
        let client = Client::builder().build::<_, Body>(https);
        
        WebhookClient {
            config,
            client,
        }
    }
    
    pub async fn forward_email(&self, email: EmailPayload) -> Result<()> {
        info!("Forwarding email from {} with subject: {}", email.sender, email.subject);
        
        // Serialize the email payload to JSON
        let json_body = serde_json::to_string(&email)?;
        
        // Build the request
        let request = Request::builder()
            .method(hyper::Method::POST)
            .uri(&self.config.webhook_url)
            .header("content-type", "application/json")
            .header("user-agent", "mail_laser/0.1.0") // Use the snake_case crate name
            .body(Body::from(json_body))?;
            
        // Send the request
        let response = self.client.request(request).await?;
        
        // Check the response status
        if !response.status().is_success() {
            error!("Webhook request failed with status: {}", response.status());
            // We don't return an error here as we don't want to fail the SMTP transaction
            // Just log the error and continue
        } else {
            info!("Email successfully forwarded to webhook");
        }
        
        Ok(())
    }
}
