// use mail_laser; // Removed redundant import (Clippy suggestion)
use log::error;
use std::panic;

#[tokio::main]
async fn main() {
    // Initialize the logger
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info")
    );
        // Set a custom panic hook to log panics
        panic::set_hook(Box::new(|panic_info| {
            if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                error!("Panic occurred: {:?}", s);
            } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                error!("Panic occurred: {:?}", s);
            } else {
                error!("Panic occurred with unknown payload");
            }
            if let Some(location) = panic_info.location() {
                error!(
                    "Panic at {}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                );
            }
        }));
    // Run the application
    if let Err(e) = mail_laser::run().await {
        error!("Application error: {}", e);
        std::process::exit(1);
    }
}
