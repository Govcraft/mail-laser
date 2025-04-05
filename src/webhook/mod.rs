use anyhow::Result;
// Updated imports for hyper 1.x and utility crates
use hyper::Request; // Keep Request from hyper core
// Use hyper-rustls for TLS support without depending on OpenSSL
use hyper_rustls::HttpsConnectorBuilder;
// Import necessary components from hyper-util
use hyper_util::{client::legacy::{connect::HttpConnector, Client}, rt::TokioExecutor};
use http_body_util::Full;               // Utilities for handling request/response bodies (BodyExt removed as it's unused)
use bytes::Bytes;                       // Bytes type for request body data
use log::{info, error};
use serde::{Serialize, Deserialize};
use crate::config::Config;

// Define specific connector and client types using aliases for clarity
// Define the HTTPS connector using rustls
type HttpsConn = hyper_rustls::HttpsConnector<HttpConnector>;
type WebhookHttpClient = Client<HttpsConn, Full<Bytes>>; // Alias for the specific client type using rustls

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailPayload {
    pub sender: String,
    pub subject: String,
    pub body: String,
}

pub struct WebhookClient {
    config: Config,
    /// The underlying HTTP client.
    client: WebhookHttpClient,
    /// The User-Agent string generated from Cargo.toml.
    user_agent: String,
}

impl WebhookClient {
    // Return Self directly, expecting cert loading to succeed or panic.
    pub fn new(config: Config) -> Self {
        // Build the rustls-based HTTPS connector
        // - with_native_roots(): Loads root certificates from the system's native store.
        //                        Requires the 'rustls-native-certs' feature we enabled.
        // - expect(): Panics if loading certificates fails (reasonable at startup).
        // - https_only(): Enforces HTTPS connections.
        // - enable_http1(): Enables HTTP/1.1 support.
        let https = HttpsConnectorBuilder::new()
            .with_native_roots()
            .expect("Failed to load native root certificates for hyper-rustls") // Use expect()
            .https_only()
            .enable_http1()
            .build();

        // Build the hyper-util client using the prepared connector and a Tokio executor
        let client: WebhookHttpClient = Client::builder(TokioExecutor::new()).build(https);

        // Construct the User-Agent string using compile-time environment variables
        let user_agent = format!(
            "{}/{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        );
        
        // Return Self directly
        Self {
            config,
            client,
            user_agent, // Store the generated user agent
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
            .header("user-agent", &self.user_agent) // Use the dynamic user agent
            // Create the request body using http_body_util::Full and bytes::Bytes
            .body(Full::new(Bytes::from(json_body)))?;
            
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
