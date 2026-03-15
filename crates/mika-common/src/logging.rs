use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Re-export for callers that need a type hint for `None` OTel layers.
pub use tracing_subscriber::layer::Identity as NoopLayer;

/// Re-export so callers can name the log guard type returned by `init_pretty`.
pub use tracing_appender::non_blocking::WorkerGuard as LogGuard;

/// Controls where log output is sent.
pub enum LogOutput {
    /// Pretty stderr + file (non-TUI CLI commands)
    PrettyAndFile,
    /// File only, no stderr (TUI mode)
    FileOnly,
}

/// Controls the stdout log format for server/gateway binaries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// Structured JSON output (default for server/gateway)
    #[default]
    Json,
    /// Human-readable pretty output (convenient for local development)
    Pretty,
}

impl std::str::FromStr for LogFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "json" => Ok(Self::Json),
            "pretty" => Ok(Self::Pretty),
            _ => Err(format!(
                "MIKA_LOG_FORMAT must be 'json' or 'pretty', got '{s}'"
            )),
        }
    }
}

/// Initialize structured JSON logging (for server/production).
/// Respects RUST_LOG env var, falls back to the provided default level.
///
/// When `log_file` is `Some`, logs are written to both stdout (JSON) and the
/// specified file (JSON) via `tracing_appender`. Parent directories are created
/// automatically. Returns `Some(WorkerGuard)` that MUST be held alive for the
/// duration of the program — dropping it flushes and stops the file writer.
///
/// When `log_file` is `None`, logs go to stdout only and returns `None`.
pub fn init<OL>(
    default_level: &str,
    log_file: Option<&Path>,
    log_format: LogFormat,
    otel_layer: Option<OL>,
) -> Option<WorkerGuard>
where
    OL: tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
{
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    // Note: the match arms look duplicative, but tracing_subscriber's type-level layer
    // composition creates distinct types for each combination, preventing shared setup.
    match (log_file, log_format) {
        (Some(path), LogFormat::Json) => {
            // JSON stdout + JSON file
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let file_appender = tracing_appender::rolling::never(
                path.parent().unwrap_or_else(|| Path::new(".")),
                path.file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("mika.log")),
            );
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

            tracing_subscriber::registry()
                .with(otel_layer)
                .with(filter)
                .with(fmt::layer().json().flatten_event(true))
                .with(
                    fmt::layer()
                        .json()
                        .flatten_event(true)
                        .with_writer(non_blocking),
                )
                .init();

            Some(guard)
        }
        (Some(path), LogFormat::Pretty) => {
            // Pretty stdout + JSON file
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let file_appender = tracing_appender::rolling::never(
                path.parent().unwrap_or_else(|| Path::new(".")),
                path.file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("mika.log")),
            );
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

            tracing_subscriber::registry()
                .with(otel_layer)
                .with(filter)
                .with(fmt::layer().pretty())
                .with(
                    fmt::layer()
                        .json()
                        .flatten_event(true)
                        .with_writer(non_blocking),
                )
                .init();

            Some(guard)
        }
        (None, LogFormat::Json) => {
            // JSON stdout only
            tracing_subscriber::registry()
                .with(otel_layer)
                .with(filter)
                .with(fmt::layer().json().flatten_event(true))
                .init();

            None
        }
        (None, LogFormat::Pretty) => {
            // Pretty stdout only
            tracing_subscriber::registry()
                .with(otel_layer)
                .with(filter)
                .with(fmt::layer().pretty())
                .init();

            None
        }
    }
}

/// Initialize logging with optional stderr output + daily-rotating file log.
/// Returns a `WorkerGuard` that MUST be held alive for the duration of the program —
/// dropping it flushes and stops the file writer.
///
/// When `output` is `LogOutput::FileOnly` (TUI mode), the stderr pretty layer is
/// omitted to avoid corrupting ratatui's alternate screen (which only covers stdout).
///
/// Note: the four match arms below look duplicative, but tracing_subscriber's
/// type-level layer composition creates distinct types for each combination,
/// preventing extraction of shared setup code.
pub fn init_pretty<OL>(
    default_level: &str,
    log_dir: Option<&Path>,
    output: LogOutput,
    otel_layer: Option<OL>,
) -> Option<WorkerGuard>
where
    OL: tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
{
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    match (log_dir, output) {
        (Some(dir), LogOutput::PrettyAndFile) => {
            // Both stderr (pretty) + file (JSON) — non-TUI commands
            let _ = std::fs::create_dir_all(dir);
            let file_appender = tracing_appender::rolling::daily(dir, "mika.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

            tracing_subscriber::registry()
                .with(otel_layer)
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
        (Some(dir), LogOutput::FileOnly) => {
            // File only — TUI mode, no stderr to avoid corrupting alternate screen
            let _ = std::fs::create_dir_all(dir);
            let file_appender = tracing_appender::rolling::daily(dir, "mika.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

            tracing_subscriber::registry()
                .with(otel_layer)
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
        (None, LogOutput::PrettyAndFile) => {
            // Stderr only — no log dir available, non-TUI
            tracing_subscriber::registry()
                .with(otel_layer)
                .with(filter)
                .with(fmt::layer().pretty())
                .init();

            None
        }
        (None, LogOutput::FileOnly) => {
            // TUI mode but no log dir — drop events silently.
            // If home dir is missing, init_for_agent will fail before TUI starts.
            tracing_subscriber::registry()
                .with(otel_layer)
                .with(filter)
                .init();
            None
        }
    }
}
