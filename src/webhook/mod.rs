//! Handles sending processed email data to a configured webhook URL via HTTPS POST.
//!
//! This module defines the data structure for the webhook payload (`EmailPayload`)
//! and provides a `WebhookClient` responsible for making the asynchronous HTTP request.
//! It uses `hyper` and `hyper-rustls` for the underlying HTTP/S communication.

use anyhow::Result;
use hyper::Request;
use hyper_rustls::HttpsConnectorBuilder;
// Import necessary components from hyper-util, using aliases for clarity.
use hyper_util::{client::legacy::{connect::HttpConnector, Client}, rt::TokioExecutor};
use http_body_util::Full; // For creating simple, complete request bodies.
use bytes::Bytes; // Bytes type for request body data.
use log::{info, error};
use serde::{Serialize, Deserialize};
use crate::config::Config;

// --- Type Aliases for Hyper Client ---

/// Type alias for the HTTPS connector using `hyper-rustls`.
type HttpsConn = hyper_rustls::HttpsConnector<HttpConnector>;
/// Type alias for the specific Hyper client configuration used for sending webhooks.
/// Uses the `HttpsConn` for TLS and expects/sends `Full<Bytes>` bodies.
type WebhookHttpClient = Client<HttpsConn, Full<Bytes>>;

// --- Public Data Structures ---

/// Represents the data payload sent to the webhook URL.
///
/// Contains the essential extracted information from a received email.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailPayload {
    /// The email address of the original sender.
    pub sender: String,
    /// The specific recipient address this email was accepted for.
    pub recipient: String,
    /// The subject line of the email.
    pub subject: String,
    /// The plain text representation of the body (HTML stripped).
    pub body: String,
    /// The original HTML body content, if the email contained HTML.
    #[serde(skip_serializing_if = "Option::is_none")] // Don't include in JSON if None
    pub html_body: Option<String>,
}

/// A client responsible for sending `EmailPayload` data to a configured webhook URL.
///
/// Encapsulates the `hyper` HTTP client setup with `rustls` for HTTPS support.
pub struct WebhookClient {
    /// Shared application configuration.
    config: Config,
    /// The underlying asynchronous HTTP client instance.
    client: WebhookHttpClient,
    /// The User-Agent string sent with webhook requests, derived from the crate's metadata.
    user_agent: String,
}

impl WebhookClient {
    /// Creates a new `WebhookClient`.
    ///
    /// Initializes an HTTPS client using `hyper-rustls` with native system certificates.
    /// Constructs a User-Agent string based on the crate's name and version from `Cargo.toml`.
    ///
    /// # Arguments
    ///
    /// * `config` - The application configuration, used to get the webhook URL.
    ///
    /// # Panics
    ///
    /// Panics if loading the system's native root TLS certificates fails. This is considered
    /// a fatal error during startup.
    pub fn new(config: Config) -> Self {
        // Configure the HTTPS connector using rustls and native certs.
        let https = HttpsConnectorBuilder::new()
            .with_native_roots()
            // Panic if cert loading fails - essential for HTTPS operation.
            .expect("Failed to load native root certificates for hyper-rustls")
            .https_only() // Ensure only HTTPS connections are made.
            .enable_http1() // Enable HTTP/1.1 support.
            .build();

        // Build the hyper client using the HTTPS connector and Tokio runtime.
        let client: WebhookHttpClient = Client::builder(TokioExecutor::new()).build(https);

        // Create a User-Agent string like "MailLaser/0.1.0".
        let user_agent = format!(
            "{}/{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        );

        Self {
            config,
            client,
            user_agent,
        }
    }

    /// Sends the given `EmailPayload` to the configured webhook URL.
    ///
    /// Serializes the payload to JSON and sends it as an HTTPS POST request.
    /// Logs the outcome (success or failure status code) of the request.
    ///
    /// **Note:** A non-successful HTTP status code from the webhook endpoint (e.g., 4xx, 5xx)
    /// is logged as an error but does *not* cause this function to return an `Err`.
    /// The email is considered successfully processed by MailLaser once the webhook
    /// request is attempted.
    ///
    /// # Arguments
    ///
    /// * `email` - The `EmailPayload` to send.
    ///
    /// # Errors
    ///
    /// Returns an `Err` if:
    /// - Serialization of the `EmailPayload` to JSON fails.
    /// - Building the HTTP request fails.
    /// - The HTTP request itself fails (e.g., network error, DNS resolution failure).
    pub async fn forward_email(&self, email: EmailPayload) -> Result<()> {
        info!("Forwarding email from {} with subject: {}", email.sender, email.subject);

        // Serialize payload to JSON. This can fail if the payload is invalid (unlikely here).
        let json_body = serde_json::to_string(&email)?;

        // Build the POST request.
        let request = Request::builder()
            .method(hyper::Method::POST)
            .uri(&self.config.webhook_url) // Target URL from config.
            .header("content-type", "application/json") // Set JSON content type.
            .header("user-agent", &self.user_agent) // Set the custom User-Agent.
            // Create the request body from the serialized JSON string.
            .body(Full::new(Bytes::from(json_body)))?; // This can fail if headers/URI are invalid.

        // Send the request asynchronously using the hyper client.
        let response = self.client.request(request).await?;

        // Check the HTTP status code of the response.
        let status = response.status();
        if !status.is_success() {
            // Log webhook failures but don't propagate the error, as per design.
            error!(
                "Webhook request to {} failed with status: {}",
                self.config.webhook_url, status
            );
        } else {
            info!(
                "Email successfully forwarded to webhook {}, status: {}",
                self.config.webhook_url, status
            );
        }

        // Return Ok regardless of the webhook's response status code.
        Ok(())
    }
}
