use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{self, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

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

// ---------------------------------------------------------------------------
// Custom compact dev formatter
// ---------------------------------------------------------------------------

/// Compact, human-friendly log format for local development.
///
/// Output: `HH:MM:SS LEVEL  message field=value field=value`
///
/// - Timestamp: local time HH:MM:SS, dimmed
/// - Level: colored and right-padded (INFO green, WARN yellow, ERROR red, DEBUG dim)
/// - Message: the main log text
/// - Fields: key=value pairs, dimmed
/// - No blank lines, no file paths, no module targets
struct DevFormat;

impl<S, N> FormatEvent<S, N> for DevFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &fmt::FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let ansi = writer.has_ansi_escapes();

        // Timestamp: HH:MM:SS local time, dimmed
        let now = chrono::Local::now();
        if ansi {
            write!(writer, "\x1b[2m{}\x1b[0m ", now.format("%H:%M:%S"))?;
        } else {
            write!(writer, "{} ", now.format("%H:%M:%S"))?;
        }

        // Level: colored, right-padded to 5 chars
        let level = *event.metadata().level();
        let level_str = format!("{:>5}", level);
        if ansi {
            let color = match level {
                tracing::Level::ERROR => "\x1b[31m", // red
                tracing::Level::WARN => "\x1b[33m",  // yellow
                tracing::Level::INFO => "\x1b[32m",  // green
                tracing::Level::DEBUG => "\x1b[2m",  // dim
                tracing::Level::TRACE => "\x1b[2m",  // dim
            };
            write!(writer, "{color}{level_str}\x1b[0m ")?;
        } else {
            write!(writer, "{level_str} ")?;
        }

        // Extract message and fields via visitor
        let mut visitor = EventVisitor::new();
        event.record(&mut visitor);

        // Message
        write!(writer, "{}", visitor.message)?;

        // Fields: dimmed key=value pairs
        if !visitor.fields.is_empty() {
            if ansi {
                write!(writer, " \x1b[2m")?;
            }
            for (i, (key, value)) in visitor.fields.iter().enumerate() {
                if i > 0 {
                    write!(writer, " ")?;
                }
                write!(writer, "{key}={value}")?;
            }
            if ansi {
                write!(writer, "\x1b[0m")?;
            }
        }

        writeln!(writer)
    }
}

/// Visitor that extracts the message and structured fields from a tracing event.
struct EventVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl EventVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: Vec::new(),
        }
    }
}

impl tracing::field::Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // fmt::Arguments implements Debug by formatting the interpolated string
            self.message = format!("{value:?}");
        } else {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
}

// ---------------------------------------------------------------------------
// Startup banner helpers (pretty mode only)
// ---------------------------------------------------------------------------

/// Print the startup banner header: `✦ name vX.Y.Z`
pub fn print_banner(name: &str, version: &str) {
    println!();
    println!("  \x1b[1m✦ {name} v{version}\x1b[0m");
    println!();
}

/// Print the ready indicator and a trailing blank line.
pub fn print_ready() {
    println!();
    println!("  \x1b[32mready\x1b[0m");
    println!();
}

// ---------------------------------------------------------------------------
// Subscriber initialization
// ---------------------------------------------------------------------------

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
    log_llm_bodies: bool,
) -> Option<WorkerGuard>
where
    OL: tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
{
    let mut filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    if log_llm_bodies {
        filter = filter.add_directive("mika::llm_debug=debug".parse().unwrap());
    }

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
            // Compact dev stdout + JSON file
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
                .with(fmt::layer().event_format(DevFormat))
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
            // Compact dev stdout only
            tracing_subscriber::registry()
                .with(otel_layer)
                .with(filter)
                .with(fmt::layer().event_format(DevFormat))
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
    log_llm_bodies: bool,
) -> Option<WorkerGuard>
where
    OL: tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
{
    let mut filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    if log_llm_bodies {
        filter = filter.add_directive("mika::llm_debug=debug".parse().unwrap());
    }

    match (log_dir, output) {
        (Some(dir), LogOutput::PrettyAndFile) => {
            // Both stderr (compact dev) + file (JSON) — non-TUI commands
            let _ = std::fs::create_dir_all(dir);
            let file_appender = tracing_appender::rolling::daily(dir, "mika.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

            tracing_subscriber::registry()
                .with(otel_layer)
                .with(filter)
                .with(
                    fmt::layer()
                        .event_format(DevFormat)
                        .with_writer(std::io::stderr),
                )
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
                .with(fmt::layer().event_format(DevFormat))
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
