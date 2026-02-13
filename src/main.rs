use std::panic;
use tracing::error;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("info".parse().unwrap()),
        )
        .init();

    // Bridge log crate macros into tracing (existing code uses log::info! etc.)
    tracing_log::LogTracer::init().ok();

    panic::set_hook(Box::new(|panic_info| {
        if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            error!("Panic occurred: {:?}", s);
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            error!("Panic occurred: {:?}", s);
        } else {
            error!("Panic occurred with unknown payload type.");
        }
        if let Some(location) = panic_info.location() {
            error!(
                "Panic occurred at {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }
    }));

    if let Err(e) = mail_laser::run().await {
        error!("Application exited with error: {}", e);
        std::process::exit(1);
    }
}
