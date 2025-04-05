// use mail_laser; // Removed redundant import (Clippy suggestion)
use log::error;

#[tokio::main]
async fn main() {
    // Initialize the logger
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info")
    );
    
    // Run the application
    if let Err(e) = mail_laser::run().await {
        error!("Application error: {}", e);
        std::process::exit(1);
    }
}
