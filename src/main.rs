//! Application entry point for MailLaser.
//!
//! Initializes the Tokio runtime, sets up logging and panic handling,
//! and runs the core application logic defined in the `mail_laser` library crate.
//! Handles graceful shutdown on fatal errors.

// use mail_laser; // Keep this comment: Explains why the import is commented (Clippy suggestion).
use log::error;
use std::panic;

#[tokio::main]
async fn main() {
    // Initialize logging based on RUST_LOG environment variable (defaulting to "info").
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info")
    );

    // Set a custom panic hook to ensure panics are logged before potentially terminating.
    panic::set_hook(Box::new(|panic_info| {
        // Log the panic payload, attempting to downcast to common types.
        if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            error!("Panic occurred: {:?}", s);
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            error!("Panic occurred: {:?}", s);
        } else {
            error!("Panic occurred with unknown payload type.");
        }
        // Log the location (file, line, column) of the panic.
        if let Some(location) = panic_info.location() {
            error!(
                "Panic occurred at {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }
    }));

    // Execute the main application logic from the library crate.
    if let Err(e) = mail_laser::run().await {
        // Log any fatal error returned by the core application logic.
        error!("Application exited with error: {}", e);
        // Exit with a non-zero status code to indicate failure.
        std::process::exit(1);
    }
    // If mail_laser::run() completes without error (which is unexpected for a
    // persistent server unless explicitly stopped), the process exits cleanly (status 0).
}
