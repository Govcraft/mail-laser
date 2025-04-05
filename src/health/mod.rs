use axum::{routing::get, Router, http::StatusCode, response::IntoResponse};
use tokio::net::TcpListener;
use log::{info, error};
use anyhow::Result;
use crate::config::Config;

/// Simple handler for the health check endpoint.
async fn health_check_handler() -> impl IntoResponse {
    StatusCode::OK // Simply return a 200 OK status. Could add more checks later if needed.
}

/// Runs the health check HTTP server.
///
/// Binds to the address specified in the config and serves the `/health` endpoint.
pub async fn run_health_server(config: Config) -> Result<()> {
    // Define the application router with the health check route
    let app = Router::new().route("/health", get(health_check_handler));

    // Construct the bind address string
    let addr_str = format!(
        "{}:{}",
        config.health_check_bind_address, config.health_check_port
    );

    // Create a TCP listener
    let listener = TcpListener::bind(&addr_str).await
        .map_err(|e| {
            error!("Failed to bind health check server to {}: {}", addr_str, e);
            anyhow::anyhow!("Failed to bind health check server: {}", e)
        })?;

    info!("Health check server listening on {}", addr_str);

    // Run the server
    axum::serve(listener, app)
        .await
        .map_err(|e| {
            error!("Health check server error: {}", e);
            anyhow::anyhow!("Health check server failed: {}", e)
        })?;

    Ok(()) // Should ideally not be reached unless server stops gracefully
}

#[cfg(test)]
mod tests {
    use super::*;
    // No need to import StatusCode again here as it's brought in by super::*
    // use axum::http::StatusCode; // Removed redundant import

    #[tokio::test]
    async fn test_health_check_handler() {
        // Call the handler
        let response = health_check_handler().await.into_response();

        // Assert that the status code is OK (200)
        assert_eq!(response.status(), StatusCode::OK);
    }
}