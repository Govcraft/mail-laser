pub mod config;
pub mod health;
pub mod smtp;
pub mod webhook;

use acton_reactive::prelude::*;
use anyhow::Result;
use log::{error, info};

pub async fn run() -> Result<()> {
    info!(
        "Starting {} v{}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    let config = match config::Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return Err(e);
        }
    };

    let mut runtime = ActonApp::launch_async().await;

    // Create actors in dependency order
    let webhook_handle = webhook::WebhookState::create(&mut runtime, &config).await?;
    let _smtp_handle =
        smtp::SmtpListenerState::create(&mut runtime, &config, webhook_handle).await?;
    let _health_handle = health::HealthState::create(&mut runtime, &config).await?;

    // Wait for shutdown signal (SIGTERM/SIGINT)
    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received, draining in-flight work...");

    // Graceful shutdown: cancels accept loops (before_stop), finishes in-flight webhooks
    runtime.shutdown_all().await?;
    info!("Shutdown complete");

    Ok(())
}
