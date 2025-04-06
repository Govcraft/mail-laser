//! Orchestrates the MailLaser application startup and component lifecycle.
//!
//! This library crate initializes configuration and concurrently runs the primary
//! services (SMTP, Health Check). It ensures that if any essential service
//! terminates unexpectedly, the entire application will shut down gracefully.

pub mod smtp;
pub mod webhook;
pub mod config;
pub mod health;

use anyhow::Result;
use log::{info, error};
use tokio::select;

/// Runs the main MailLaser application logic.
///
/// Initializes and launches the SMTP and health check servers in separate asynchronous tasks.
/// It then monitors these tasks using `tokio::select!`. The application is designed to run
/// indefinitely. This function will only return if a critical error occurs in configuration
/// loading or if one of the essential server tasks terminates unexpectedly (either by
/// error, panic, or unexpected clean exit).
///
/// # Returns
///
/// - `Ok(())`: Should theoretically never return this in normal operation, as servers run indefinitely.
/// - `Err(anyhow::Error)`: If configuration loading fails, or if either the SMTP or health
///   check server task stops unexpectedly. The error indicates a fatal condition preventing
///   the application from continuing.
pub async fn run() -> Result<()> {
    info!(
        "Starting {} v{} inbound-SMTP server",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    // Load configuration; exit early if configuration is invalid or missing.
    let config = match config::Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return Err(e); // Propagate configuration error to main.rs for process exit.
        }
    };

    let smtp_server = smtp::Server::new(config.clone());
    // Clone config for the health server task, as each task needs its own owned copy.
    let health_config = config.clone();

    // Spawn the health check server task.
    let health_handle = tokio::spawn(async move {
        if let Err(e) = health::run_health_server(health_config).await {
            error!("Health check server encountered a fatal error: {}", e);
            Err(e) // Propagate the error to the select! macro.
        } else {
            // A server task exiting without error is unexpected for a long-running service.
            Ok(()) // Signal this unexpected state to select! for error handling.
        }
    });

    // Spawn the main SMTP server task.
    let smtp_handle = tokio::spawn(async move {
        if let Err(e) = smtp_server.run().await {
             error!("SMTP server encountered a fatal error: {}", e);
             Err(e) // Propagate the error to the select! macro.
        } else {
             // A server task exiting without error is unexpected for a long-running service.
             Ok(()) // Signal this unexpected state to select! for error handling.
        }
    });

    // Monitor both server tasks concurrently. `select!` waits for the first task to complete.
    // For long-running services, completion usually indicates an issue.
    select! {
        // `res` is Result<Result<()>, JoinError>
        // Outer Ok: Task finished normally (returned Ok or Err).
        // Outer Err: Task panicked or was cancelled.
        // Inner Ok: Task function returned Ok(()).
        // Inner Err: Task function returned an Err.
        res = health_handle => {
            error!("Health check server task terminated.");
            match res {
                Ok(Ok(())) => {
                    // Task completed without returning an error. This is unexpected for a
                    // persistent server, so we treat it as an application error.
                    Err(anyhow::anyhow!("Health check server exited cleanly, which is unexpected."))
                }
                Ok(Err(e)) => {
                    // Task completed and returned a specific error. Propagate it.
                    error!("Health check server returned error: {}", e);
                    Err(e)
                }
                Err(join_error) => {
                    // Task panicked or was cancelled. Wrap the JoinError.
                    error!("Health check server task failed (panic or cancellation): {}", join_error);
                    Err(anyhow::anyhow!("Health check server task failed: {}", join_error))
                }
            }
        },
        res = smtp_handle => {
            error!("SMTP server task terminated.");
             match res {
                Ok(Ok(())) => {
                    // Task completed without returning an error. Unexpected for the main server.
                    Err(anyhow::anyhow!("SMTP server exited cleanly, which is unexpected."))
                }
                Ok(Err(e)) => {
                    // Task completed and returned a specific error. Propagate it.
                    error!("SMTP server returned error: {}", e);
                    Err(e)
                }
                Err(join_error) => {
                    // Task panicked or was cancelled. Wrap the JoinError.
                    error!("SMTP server task failed (panic or cancellation): {}", join_error);
                    Err(anyhow::anyhow!("SMTP server task failed: {}", join_error))
                }
             }
        },
    }
    // The Result (Ok or Err) from the completed task's branch in select! is returned.
    // Control should ideally not reach *past* the select! block in this setup.
}
