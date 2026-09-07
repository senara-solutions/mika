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
// `MIKA_LOG_LLM_BODIES` — one truth table (mika#2220)
// ---------------------------------------------------------------------------

/// The env var that arms LLM request/response body capture.
pub const LOG_LLM_BODIES_ENV: &str = "MIKA_LOG_LLM_BODIES";

/// How a raw `MIKA_LOG_LLM_BODIES` value reads.
///
/// **mika#2220 — the divergence this type exists to remove.** The daemon reaches
/// the flag through `Settings` (config-rs), which accepts `1 / true / on / yes`
/// **case-insensitively**. Every CLI call site instead open-coded
/// `v == "true" || v == "1"` — byte-exact, lowercase only. So
/// `MIKA_LOG_LLM_BODIES=True` (or `yes`, or `on`) armed mika-spirit and was a
/// **silent no-op** on every `mika` process: exactly the reported shape, "works
/// for the daemon, inert for the agent". Measured, not inferred — see
/// `crates/mika-common/tests/tui_llm_body_capture.rs`, which prints the
/// config-rs truth table next to this one.
///
/// A value neither side recognises is [`Unrecognized`](Self::Unrecognized)
/// rather than `false`. The two halves still differ there, deliberately: config-rs
/// makes it a hard `Settings::load` error (the daemon refuses to boot), while a
/// `mika` invocation only warns — downing the CLI over a typo in a diagnostic
/// variable would cost more than it buys. What is *not* acceptable, and is what
/// this closes, is reading it as "off" without saying so.
///
/// Note the leading-space case: config-rs does **not** trim, so `" true"` is an
/// error there. This does not trim either. Trimming would be a new divergence in
/// the opposite direction — a value the CLI accepts and the daemon refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogLlmBodiesSetting {
    /// The variable is absent from the environment.
    Unset,
    /// Recognised as on.
    Enabled,
    /// Recognised as off.
    Disabled,
    /// Present, but not a value either parser recognises. Carries it verbatim
    /// (quoted when reported) so a stray space is visible in the warning.
    Unrecognized(String),
}

impl LogLlmBodiesSetting {
    /// Whether body capture should be armed. Only [`Enabled`](Self::Enabled) is.
    pub fn enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Emit the operator-visible warning when the value was set to something
    /// neither parser recognises.
    ///
    /// Call this **after** the subscriber is installed — before it, the event has
    /// nowhere to go. At WARN because the failure it reports is invisible by
    /// construction: the operator asked for capture and silently did not get it.
    pub fn warn_if_unrecognized(&self) {
        if let Self::Unrecognized(raw) = self {
            tracing::warn!(
                event = "llm_body_capture_unrecognized_value",
                value = %format!("{raw:?}"),
                "{LOG_LLM_BODIES_ENV} is set to a value that is neither on nor off — \
                 body capture stays OFF. Use one of: 1, true, on, yes (case-insensitive)."
            );
        }
    }
}

/// Read [`LOG_LLM_BODIES_ENV`] from the process environment.
pub fn log_llm_bodies_from_env() -> LogLlmBodiesSetting {
    parse_log_llm_bodies(std::env::var(LOG_LLM_BODIES_ENV).ok().as_deref())
}

/// Classify a raw `MIKA_LOG_LLM_BODIES` value.
///
/// The accepted set is config-rs's, so the CLI and the daemon agree — see
/// [`LogLlmBodiesSetting`] for why that agreement is the whole point.
pub fn parse_log_llm_bodies(raw: Option<&str>) -> LogLlmBodiesSetting {
    let Some(raw) = raw else {
        return LogLlmBodiesSetting::Unset;
    };
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => LogLlmBodiesSetting::Enabled,
        "0" | "false" | "off" | "no" => LogLlmBodiesSetting::Disabled,
        _ => LogLlmBodiesSetting::Unrecognized(raw.to_string()),
    }
}

/// Announce, once per process, that body capture is armed and **where it lands**.
///
/// **mika#2220 — why a sink path and not just a boolean.** The flag is
/// process-global, but an agent's turns are not all served by the same process:
/// since mika#1727 `mika ask` is a thin A2A client, so its agent turns execute in
/// mika-spirit and are logged there, not in `~/.mika/agents/<name>/logs/`. An
/// operator who arms the variable on the agent's own process and then reads the
/// agent's own file finds an empty file and concludes the flag is broken — the
/// founding observation of this ticket. This line makes the sink a fact the
/// operator reads instead of a routing rule they have to know.
///
/// WARN, not INFO, for two independent reasons: the CLI's default level is
/// `warn`, so an INFO line would be filtered out on precisely the surface that
/// needed it; and full prompt bodies on disk is a state worth flagging — they can
/// carry anything the agent was sent.
fn announce_llm_body_capture(sink: &str) {
    tracing::warn!(
        event = "llm_body_capture",
        state = "enabled",
        sink = %sink,
        "{LOG_LLM_BODIES_ENV} is on — full LLM request/response bodies are being \
         written to {sink}. Turns served by mika-spirit (webhooks, callbacks, and \
         `mika ask` since mika#1727) are logged by that process, not this one."
    );
}

/// The file a `log_dir`-style (daily-rolling) sink writes to today.
fn daily_sink_path(dir: &Path) -> String {
    dir.join(format!(
        "mika.log.{}",
        chrono::Local::now().format("%Y-%m-%d")
    ))
    .display()
    .to_string()
}

// ---------------------------------------------------------------------------
// Subscriber initialization
// ---------------------------------------------------------------------------

/// Whether `init` installs a JSON `fmt` layer on **stdout**, given whether a log
/// file is configured.
///
/// **mika#2195 — the invariant this function exists to hold: when a log file is
/// configured, mika writes that file exactly once, and writes JSON nowhere else.**
///
/// Before this fix, the JSON branch of [`init`] composed *two* JSON layers — one
/// on stdout, one on the file. That is harmless only while stdout goes somewhere
/// else, and in production it does not: the OpenRC unit runs
/// `supervise-daemon mika-spirit --stdout /var/log/mika/server.log --stderr
/// /var/log/mika/server.log`, i.e. the launcher redirects stdout into the very
/// file the second layer already writes. Every event therefore landed in
/// `server.log` **twice**, and every consumer that counts events over-counted ×2
/// (measured: `mika-spirit listening` twice per restart, `domain_rebuild_complete`
/// four times per boot, the RT-005 factor-2, and mika#2179's "38 failures" that
/// were 19).
///
/// The fix lives here, at the point that composes the layers, rather than in the
/// launcher's redirection: a producer must not depend on how it was launched in
/// order not to duplicate itself. Any other launcher (systemd, docker, a future
/// init script) that redirects stdout into the log file would otherwise reopen
/// the same hole.
///
/// Scope note: only the **JSON** stdout layer is withheld. `LogFormat::Pretty`
/// keeps its human-readable console layer whether or not a file is configured —
/// Pretty is the local-development format, where console output is the point and
/// no launcher redirection is in play.
fn json_stdout_layer_enabled(log_file_configured: bool) -> bool {
    !log_file_configured
}

/// Initialize structured JSON logging (for server/production).
/// Respects RUST_LOG env var, falls back to the provided default level.
///
/// When `log_file` is `Some`, logs are written **only** to that file (JSON, via
/// `tracing_appender`) — the JSON stdout layer is deliberately not installed, see
/// [`json_stdout_layer_enabled`] (mika#2195). Parent directories are created
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

    // Resolved before the match so the announce below can name it (mika#2220).
    let sink = match log_file {
        Some(path) => path.display().to_string(),
        None => "stdout".to_string(),
    };

    // Note: the match arms look duplicative, but tracing_subscriber's type-level layer
    // composition creates distinct types for each combination, preventing shared setup.
    let guard = match (log_file, log_format) {
        (Some(path), LogFormat::Json) => {
            // JSON file only — no JSON stdout layer (mika#2195).
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
                // Gated on the one decision both JSON arms read (mika#2195):
                // a configured log file means this layer is absent, so a
                // launcher that redirects stdout into that same file cannot
                // double every line.
                .with(
                    json_stdout_layer_enabled(true)
                        .then(|| fmt::layer().json().flatten_event(true)),
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
            // JSON stdout only — no file writer, so nothing to duplicate.
            tracing_subscriber::registry()
                .with(otel_layer)
                .with(filter)
                .with(
                    json_stdout_layer_enabled(false)
                        .then(|| fmt::layer().json().flatten_event(true)),
                )
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
    };

    // After `.init()` — before it, the event has no subscriber to reach.
    if log_llm_bodies {
        announce_llm_body_capture(&sink);
    }

    guard
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

    // Resolved before the match so the announce below can name it (mika#2220).
    let sink = match log_dir {
        Some(dir) => daily_sink_path(dir),
        None => "nowhere (no log directory resolved)".to_string(),
    };

    let guard = match (log_dir, output) {
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
    };

    // After `.init()` — before it, the event has no subscriber to reach.
    if log_llm_bodies {
        announce_llm_body_capture(&sink);
    }

    guard
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    /// Own source, read back for the structural guard below.
    const THIS_FILE: &str = include_str!("logging.rs");

    /// A writer that every layer in a test can share, standing in for the single
    /// destination file that production's stdout redirection and file appender
    /// both end up writing to.
    #[derive(Clone, Default)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl SharedSink {
        fn lines_mentioning(&self, needle: &str) -> usize {
            let bytes = self.0.lock().expect("sink poisoned").clone();
            String::from_utf8(bytes)
                .expect("log output is UTF-8")
                .lines()
                .filter(|line| line.contains(needle))
                .count()
        }
    }

    impl std::io::Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("sink poisoned").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedSink {
        type Writer = SharedSink;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Reproduce production's topology: the JSON *file* layer, plus the JSON
    /// *stdout* layer if [`json_stdout_layer_enabled`] says `init` installs one —
    /// both aimed at a single destination, exactly as OpenRC's
    /// `--stdout /var/log/mika/server.log` aims them today. Emit `event` once and
    /// return how many lines mentioning it reached that destination.
    fn lines_written_for_one_event(log_file_configured: bool, event: &'static str) -> usize {
        let sink = SharedSink::default();

        let subscriber = tracing_subscriber::registry()
            .with(json_stdout_layer_enabled(log_file_configured).then(|| {
                fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_writer(sink.clone())
            }))
            .with(log_file_configured.then(|| {
                fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_writer(sink.clone())
            }));

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "mika::otel", step = 1, "{}", event);
        });

        sink.lines_mentioning(event)
    }

    /// AC1 + AC2 (mika#2195) — one real occurrence must produce exactly one line.
    ///
    /// AC3 rides on the same test by construction: the defect was structural to
    /// the layer composition, never specific to `turn_usage`, so the assertion is
    /// made over the three event names the incident was measured on. Before the
    /// fix each of them yields 2 (the pre-fix predicate returned `true` with a log
    /// file configured), which is the non-vacuity control.
    #[test]
    fn mika2195_one_event_is_one_line_when_a_log_file_is_configured() {
        for event in [
            "turn_usage",
            "mika-spirit listening",
            "domain_rebuild_complete",
        ] {
            assert_eq!(
                lines_written_for_one_event(true, event),
                1,
                "`{event}` must reach the log file exactly once; 2 means the JSON \
                 stdout layer is back and the launcher's stdout redirection is \
                 doubling every line again (mika#2195)"
            );
        }
    }

    /// AC4 — the fix withholds a duplicate, never the output itself. With no log
    /// file configured there is no second writer, so stdout stays the only sink
    /// and must still carry the event.
    #[test]
    fn mika2195_stdout_still_logs_when_no_log_file_is_configured() {
        assert!(
            json_stdout_layer_enabled(false),
            "with no log file there is nothing to duplicate; withholding stdout \
             here would silence the process entirely"
        );
        assert_eq!(lines_written_for_one_event(false, "turn_usage"), 1);
    }

    // -----------------------------------------------------------------------
    // mika#2220 — `MIKA_LOG_LLM_BODIES` truth table
    // -----------------------------------------------------------------------

    /// Every value the daemon's parser (config-rs, string → bool) recognises,
    /// with the verdict it reaches.
    const CONFIG_RS_RECOGNISED: &[(&str, bool)] = &[
        ("1", true),
        ("true", true),
        ("on", true),
        ("yes", true),
        ("0", false),
        ("false", false),
        ("off", false),
        ("no", false),
    ];

    /// AC1 — the CLI and the daemon must reach the same verdict on the same value.
    ///
    /// This is the assertion the pre-fix code could not satisfy: every CLI call
    /// site open-coded `v == "true" || v == "1"`, so `True` / `TRUE` / `on` /
    /// `yes` armed the daemon and were silently inert on the CLI. The reference
    /// side is config-rs itself, queried here rather than transcribed — a
    /// transcribed table would drift the first time config-rs widened its own.
    #[test]
    fn mika2220_cli_parse_agrees_with_the_daemon_parse() {
        for (raw, expected) in CONFIG_RS_RECOGNISED {
            let daemon = config::Config::builder()
                .set_override("v", *raw)
                .expect("override")
                .build()
                .expect("build")
                .get_bool("v")
                .unwrap_or_else(|e| panic!("config-rs no longer accepts {raw:?}: {e}"));
            assert_eq!(
                daemon, *expected,
                "the reference table claims config-rs reads {raw:?} as {expected}"
            );
            assert_eq!(
                parse_log_llm_bodies(Some(raw)).enabled(),
                daemon,
                "{raw:?}: the CLI must reach the daemon's verdict, or arming \
                 MIKA_LOG_LLM_BODIES on one process and not the other is silent (mika#2220)"
            );
        }
    }

    /// Case-insensitivity is the specific half that bit: an operator typing
    /// `True` got a working daemon and an inert CLI.
    #[test]
    fn mika2220_case_variants_are_recognised() {
        for raw in ["TRUE", "True", "tRuE", "ON", "Yes", "YES"] {
            assert_eq!(
                parse_log_llm_bodies(Some(raw)),
                LogLlmBodiesSetting::Enabled,
                "{raw:?} must arm capture"
            );
        }
        for raw in ["FALSE", "False", "Off", "NO"] {
            assert_eq!(
                parse_log_llm_bodies(Some(raw)),
                LogLlmBodiesSetting::Disabled
            );
        }
    }

    /// Absent is not the same as unreadable, and unreadable is not the same as
    /// off — collapsing the third into the second is what made the failure silent.
    #[test]
    fn mika2220_unset_and_unrecognized_are_distinct_from_disabled() {
        assert_eq!(parse_log_llm_bodies(None), LogLlmBodiesSetting::Unset);
        assert!(!parse_log_llm_bodies(None).enabled());

        for raw in ["nope", "", " true", "true ", "2"] {
            assert_eq!(
                parse_log_llm_bodies(Some(raw)),
                LogLlmBodiesSetting::Unrecognized(raw.to_string()),
                "{raw:?} must be reported, not read as off"
            );
            assert!(!parse_log_llm_bodies(Some(raw)).enabled());
        }
    }

    /// The whitespace-padded case is deliberately NOT trimmed: config-rs does not
    /// trim either, so trimming here would make the CLI accept a value the daemon
    /// refuses — the same divergence class, pointing the other way.
    #[test]
    fn mika2220_padded_values_are_not_silently_trimmed() {
        let daemon_verdict = config::Config::builder()
            .set_override("v", " true")
            .expect("override")
            .build()
            .expect("build")
            .get_bool("v");
        assert!(
            daemon_verdict.is_err(),
            "config-rs is expected to refuse a padded value; if it started \
             trimming, parse_log_llm_bodies must follow it"
        );
        assert!(matches!(
            parse_log_llm_bodies(Some(" true")),
            LogLlmBodiesSetting::Unrecognized(_)
        ));
    }

    /// The unrecognized case must actually reach the operator, and must quote the
    /// value so an invisible difference (a stray space) is visible in the log.
    #[test]
    fn mika2220_unrecognized_value_warns_with_the_value_quoted() {
        let sink = SharedSink::default();
        let subscriber = tracing_subscriber::registry().with(
            fmt::layer()
                .json()
                .flatten_event(true)
                .with_writer(sink.clone()),
        );

        tracing::subscriber::with_default(subscriber, || {
            parse_log_llm_bodies(Some(" true")).warn_if_unrecognized();
            parse_log_llm_bodies(Some("true")).warn_if_unrecognized();
            parse_log_llm_bodies(None).warn_if_unrecognized();
        });

        assert_eq!(
            sink.lines_mentioning("llm_body_capture_unrecognized_value"),
            1,
            "exactly the unreadable value warns — a recognised or absent one must stay quiet"
        );
        assert_eq!(
            sink.lines_mentioning(r#"\" true\""#),
            1,
            "the value must be quoted in the warning, or a leading space is invisible"
        );
    }

    /// Structural guard: the JSON+file arm of `init` must keep exactly one file
    /// writer and must route its stdout layer through the shared predicate. A
    /// future edit that re-adds an ungated `fmt::layer().json()` there reopens the
    /// double-write under any launcher that redirects stdout into the log file,
    /// and the runtime tests above cannot see it — they exercise the predicate,
    /// not the arm.
    #[test]
    fn mika2195_json_file_arm_gates_stdout_and_keeps_one_file_writer() {
        let arm = THIS_FILE
            .split_once("(Some(path), LogFormat::Json) => {")
            .expect("the JSON + log-file arm of init must exist")
            .1
            .split_once("(Some(path), LogFormat::Pretty) => {")
            .expect("the Pretty + log-file arm must follow it")
            .0;

        assert!(
            arm.contains("json_stdout_layer_enabled("),
            "the JSON+file arm must read the shared predicate rather than decide \
             for itself whether to install a stdout layer (mika#2195)"
        );
        assert_eq!(
            arm.matches(".json()").count(),
            2,
            "exactly two JSON layers may appear in this arm: the gated stdout one \
             and the file one"
        );
        assert_eq!(
            arm.matches(".with_writer(non_blocking)").count(),
            1,
            "exactly one layer may write the log file"
        );
    }
}
