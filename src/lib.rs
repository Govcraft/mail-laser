pub mod smtp;
pub mod webhook;
pub mod config;
pub mod health;

use anyhow::Result;
use log::{info, error}; // Add error for logging select! results
use tokio::select; // Import select! macro

pub async fn run() -> Result<()> {
    info!("Starting MailLaser SMTP server"); 
    
    // Load configuration from environment variables with error logging
    let config = match config::Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return Err(e);
        }
    };
    
    // Start the SMTP server
    let smtp_server = smtp::Server::new(config.clone());
    
    // Clone config for the health server
    let health_config = config.clone();

    // Spawn the health check server task
    let health_handle = tokio::spawn(async move {
        if let Err(e) = health::run_health_server(health_config).await {
            error!("Health check server failed: {}", e);
            // Return the error to be caught by select!
            Err(e)
        } else {
            // Should not happen unless server stops gracefully, which is unexpected here
            Ok(())
        }
    });

    // Spawn the SMTP server task
    let smtp_handle = tokio::spawn(async move {
        if let Err(e) = smtp_server.run().await {
             error!("SMTP server failed: {}", e);
             // Return the error to be caught by select!
             Err(e)
        } else {
             // Should not happen unless server stops gracefully, which is unexpected here
             Ok(())
        }
    });

    // Wait for either server task to complete (likely due to an error)
    select! {
        // Handle result from the health server task
        res = health_handle => {
            error!("Health check server task finished.");
            // Handle potential JoinError first, then the task's Result
            match res {
                Ok(Ok(())) => {
                    // Server exited cleanly - unexpected for a long-running service
                    Err(anyhow::anyhow!("Health check server exited unexpectedly without error."))
                }
                Ok(Err(e)) => {
                    // Server task returned an error
                    error!("Health check server returned error: {}", e);
                    Err(e) // Propagate the error
                }
                Err(join_error) => {
                    // Task panicked or was cancelled
                    error!("Health check server task failed: {}", join_error);
                    Err(anyhow::anyhow!("Health check server task failed: {}", join_error))
                }
            }
        },
        // Handle result from the SMTP server task
        res = smtp_handle => {
            error!("SMTP server task finished.");
            // Handle potential JoinError first, then the task's Result
             match res {
                Ok(Ok(())) => {
                    // Server exited cleanly - unexpected for a long-running service
                    Err(anyhow::anyhow!("SMTP server exited unexpectedly without error."))
                }
                Ok(Err(e)) => {
                    // Server task returned an error
                    error!("SMTP server returned error: {}", e);
                    Err(e) // Propagate the error
                }
                Err(join_error) => {
                    // Task panicked or was cancelled
                    error!("SMTP server task failed: {}", join_error);
                    Err(anyhow::anyhow!("SMTP server task failed: {}", join_error))
                }
            }
        },
    }

    // If select! returns, it means one of the servers exited, likely with an error.
    // The error propagation happens inside the select! block.
    // If somehow both completed without error (unexpected), we could reach here.
    // For robustness, let's return Ok, though this path shouldn't normally be hit
    // in a long-running server setup unless explicitly stopped.
    // Ok(()) // The select! block now handles returning the Result
}
