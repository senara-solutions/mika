use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize structured JSON logging (for server/production).
/// Respects RUST_LOG env var, falls back to the provided default level.
pub fn init(default_level: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().json().flatten_event(true))
        .init();
}

/// Initialize logging with human-readable stderr output + daily-rotating file log.
/// Returns a `WorkerGuard` that MUST be held alive for the duration of the program —
/// dropping it flushes and stops the file writer.
pub fn init_pretty(default_level: &str, log_dir: Option<&Path>) -> Option<WorkerGuard> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    match log_dir {
        Some(dir) => {
            let _ = std::fs::create_dir_all(dir);
            let file_appender = tracing_appender::rolling::daily(dir, "mika.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().pretty().with_writer(std::io::stderr))
                .with(
                    fmt::layer()
                        .json()
                        .flatten_event(true)
                        .with_writer(non_blocking),
                )
                .init();

            Some(guard)
        }
        None => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().pretty())
                .init();

            None
        }
    }
}
