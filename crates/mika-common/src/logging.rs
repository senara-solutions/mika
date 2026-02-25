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

/// Initialize logging with optional stderr output + daily-rotating file log.
/// Returns a `WorkerGuard` that MUST be held alive for the duration of the program —
/// dropping it flushes and stops the file writer.
///
/// When `suppress_stderr` is true (TUI mode), the stderr pretty layer is omitted
/// to avoid corrupting ratatui's alternate screen (which only covers stdout).
///
/// Note: the four match arms below look duplicative, but tracing_subscriber's
/// type-level layer composition creates distinct types for each combination,
/// preventing extraction of shared setup code.
pub fn init_pretty(
    default_level: &str,
    log_dir: Option<&Path>,
    suppress_stderr: bool,
) -> Option<WorkerGuard> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    match (log_dir, suppress_stderr) {
        (Some(dir), false) => {
            // Both stderr (pretty) + file (JSON) — non-TUI commands
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
        (Some(dir), true) => {
            // File only — TUI mode, no stderr to avoid corrupting alternate screen
            let _ = std::fs::create_dir_all(dir);
            let file_appender = tracing_appender::rolling::daily(dir, "mika.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

            tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .json()
                        .flatten_event(true)
                        .with_writer(non_blocking),
                )
                .init();

            Some(guard)
        }
        (None, false) => {
            // Stderr only — no log dir available, non-TUI
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().pretty())
                .init();

            None
        }
        (None, true) => {
            // TUI mode but no log dir — drop events silently.
            // If home dir is missing, init_for_agent will fail before TUI starts.
            tracing_subscriber::registry().with(filter).init();
            None
        }
    }
}
