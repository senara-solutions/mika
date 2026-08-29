// READ-ONLY INVARIANT: Skills must never write to their own directory at runtime.
// This is critical for `--link` mode where the skill directory is a symlink to the
// author's source directory. Writing back would silently modify the author's source
// files, creating shared-mutable-state bugs. All skill output goes to stdout/stderr.

use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use base64::Engine;
use regex::Regex;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

use super::index::ResolvedSkillTool;
use super::manifest::ToolHandler;
use crate::async_db::AsyncDatabase;
use crate::db::{self, NewTask};
use crate::github_graphql::{fetch_issue_body, fetch_pr_summary, parse_pr_url};
use crate::task_engine::types::{action_type, trigger_type};
use crate::tools::{GitHubRef, ImageData, ToolOutput, parse_github_ref};

/// Maximum output size from a skill tool (10,000 characters).
const MAX_OUTPUT_LEN: usize = 10_000;

/// Non-`MIKA_*` env vars that must also be scrubbed from child processes.
///
/// `GH_TOKEN` is removed to prevent identity collision: if it leaked from
/// `~/.mika/.env` via dotenvy, it would override the host's `gh auth` identity
/// in ALL child processes. `run_gh` explicitly re-injects the correct platform
/// token AFTER this scrub. See issue #380.
const EXTRA_SCRUB_VARS: &[&str] = &["GH_TOKEN"];

/// Scrub all `MIKA_*` environment variables (and [`EXTRA_SCRUB_VARS`]) from a
/// tokio Command (defense-in-depth).
///
/// Prevents leaking secrets like `MIKA_ANTHROPIC_API_KEY`, `MIKA_INTERNAL_TOKEN`,
/// and `MIKA_OPENAI_API_KEY` to child processes.
pub(crate) fn scrub_mika_env_vars(cmd: &mut tokio::process::Command) {
    for (key, _) in std::env::vars() {
        if key.starts_with("MIKA_") {
            cmd.env_remove(&key);
        }
    }
    for key in EXTRA_SCRUB_VARS {
        cmd.env_remove(key);
    }
}

/// Scrub all `MIKA_*` environment variables (and [`EXTRA_SCRUB_VARS`]) from a
/// std Command (defense-in-depth).
///
/// Same as [`scrub_mika_env_vars`] but for synchronous `std::process::Command`.
pub(crate) fn scrub_mika_env_vars_std(cmd: &mut std::process::Command) {
    for (key, _) in std::env::vars() {
        if key.starts_with("MIKA_") {
            cmd.env_remove(&key);
        }
    }
    for key in EXTRA_SCRUB_VARS {
        cmd.env_remove(key);
    }
}

/// Baseline exact-match allowlist for [`is_sandbox_env_allowed`]. Every well-behaved
/// subprocess needs these — missing `PATH` breaks every exec, missing `HOME` breaks
/// every `~/.config/…` read. Kept narrow: no key here may start with `MIKA_`.
const SANDBOX_ENV_CORE_ALLOWLIST: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "TERM", "LANG", "LC_ALL", "TMPDIR", "HOSTNAME",
];

/// Prefix-match allowlist for [`is_sandbox_env_allowed`]. Wildcard families the
/// pilot subprocess needs to interoperate with system tooling. Deliberately covers
/// only NON-secret families — no `AWS_`, no `OPENAI_`, no `NODE_AUTH_`.
const SANDBOX_ENV_ALLOWED_PREFIXES: &[&str] = &[
    "LC_",     // locale variants (LC_MESSAGES, LC_NUMERIC, …) — user's shell
    "XDG_",    // Claude Code plugin cache, gh config, freedesktop paths
    "NVM_",    // nvm-managed node runtime discovery
    "CARGO_",  // cargo build cache / config for pilot-invoked builds
    "RUSTUP_", // rustup toolchain resolution
];

/// Pure predicate for the [`sandboxed_pilot_env`] allowlist. Extracted for
/// unit-testing so the shape can be verified without spawning a subprocess or
/// mutating the process-wide env.
fn is_sandbox_env_allowed(key: &str) -> bool {
    // Never allow a MIKA_* key through even if it were listed — defense against
    // a future refactor that adds one to the core list. Composed with the
    // debug_assert in `sandboxed_pilot_env` for both dynamic and static checks.
    if key.starts_with("MIKA_") {
        return false;
    }
    if SANDBOX_ENV_CORE_ALLOWLIST.contains(&key) {
        return true;
    }
    SANDBOX_ENV_ALLOWED_PREFIXES
        .iter()
        .any(|p| key.starts_with(p))
}

/// Positive-allowlist environment for the pilot subprocess — used by
/// [`spawn_long_running_exec`] which invokes claude-pilot in the highest-untrust
/// context on the platform (LLM-generated commands running via cpp).
///
/// Strictly stronger than [`scrub_mika_env_vars`]: instead of removing MIKA_\*
/// (negative shape), this clears the entire env and copies only the vars matched
/// by [`is_sandbox_env_allowed`]. Any operator-injected token not in the allowlist
/// (`AWS_*`, `OPENAI_*` without the MIKA_ prefix, `NODE_AUTH_TOKEN`, `NPM_TOKEN`,
/// custom credential vars) cannot leak by inheritance.
///
/// Callers still re-inject `GH_TOKEN` and `ANTHROPIC_LOG_FILE` AFTER this call, same
/// as with the older scrub. See mika#TBD (Phase 1 of dev-pilot containment layer,
/// coupled to bubblewrap fs+network isolation in Phase 2).
pub(crate) fn sandboxed_pilot_env(cmd: &mut tokio::process::Command) {
    cmd.env_clear();
    for (key, value) in std::env::vars() {
        if is_sandbox_env_allowed(&key) {
            cmd.env(&key, &value);
        }
    }
    debug_assert!(
        SANDBOX_ENV_CORE_ALLOWLIST
            .iter()
            .all(|k| !k.starts_with("MIKA_")),
        "SANDBOX_ENV_CORE_ALLOWLIST contains a MIKA_ variable — a secret would leak"
    );
}

/// Environment variable claude-pilot-py honors to append a per-LLM-call JSONL
/// transcript (mika#1705). Deliberately non-`MIKA_*` so it survives the
/// child-env scrub in [`scrub_mika_env_vars`] and flows through dispatch-lib.sh
/// into the claude-pilot subprocess.
const PILOT_TRANSCRIPT_ENV: &str = "ANTHROPIC_LOG_FILE";

/// Read the `MIKA_LOG_PILOT_TRANSCRIPTS` gate from mika-spirit's own environment
/// (mika#1705 committed position 4 — default ON once shipped, gateable). Reads
/// the parent process env directly, before the child-command scrub. `0`,
/// `false`, `no`, `off` (case-insensitive) disable; any other value or absence
/// enables.
pub(crate) fn pilot_transcripts_enabled() -> bool {
    match std::env::var("MIKA_LOG_PILOT_TRANSCRIPTS") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Inject `ANTHROPIC_LOG_FILE` for claude-pilot dispatch skills so the
/// subprocess appends its LLM-call corpus to
/// `{home}/data/pilot-transcripts/<task-id>.jsonl` (mika#1705). The engine
/// ingestion tick imports finished files into the `pilot_transcripts` table.
///
/// MUST be called AFTER [`scrub_mika_env_vars`] so the injected var is not
/// stripped. Best-effort: feature-off, non-pilot skill, home resolution failure,
/// or dir-create failure are all silent no-ops — transcript capture must never
/// block or fail a dispatch.
fn inject_pilot_transcript_env(
    cmd: &mut tokio::process::Command,
    skill_dir: &std::path::Path,
    task_id: &str,
) {
    if !pilot_transcripts_enabled() {
        return;
    }
    // Only the two claude-pilot dispatch skills produce subprocess LLM
    // trajectories worth capturing (both source `_shared/dispatch-lib.sh`).
    let is_pilot_skill = skill_dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "dev-pilot" || n == "dev-groom");
    if !is_pilot_skill {
        return;
    }
    let Ok(home) = mika_common::home::resolve_home_dir() else {
        warn!(task_id = %task_id, "mika#1705: could not resolve home dir; skipping transcript capture");
        return;
    };
    let dir = home.join("data").join("pilot-transcripts");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!(task_id = %task_id, error = %e, "mika#1705: failed to create pilot-transcripts dir; skipping capture");
        return;
    }
    let path = dir.join(format!("{task_id}.jsonl"));
    cmd.env(PILOT_TRANSCRIPT_ENV, &path);
    debug!(task_id = %task_id, path = %path.display(), "mika#1705: pilot transcript capture enabled");
}

/// Maximum raw image file size (5 MB).
const MAX_IMAGE_SIZE: u64 = 5 * 1024 * 1024;

/// Maximum number of images per tool result.
const MAX_IMAGES_PER_RESULT: usize = 5;

// -- Mika envelope protocol for image-bearing tool results --

/// Top-level JSON envelope output by exec handlers that return images.
///
/// Scripts output `{"__mika_v1": {"text": "...", "images": ["/path/to/img.png"]}}`.
#[derive(Deserialize)]
struct MikaEnvelope {
    __mika_v1: MikaOutput,
}

#[derive(Deserialize)]
struct MikaOutput {
    text: String,
    #[serde(default)]
    images: Vec<String>,
}

/// Context for spawning long-running background exec handlers.
///
/// When present and the handler has `long_running: true`, the executor creates
/// a callback task and spawns the subprocess in the background instead of
/// blocking the agent loop.
pub struct LongRunningContext {
    pub db: AsyncDatabase,
    pub agent_name: String,
    pub session_id: String,
    pub trace_id: String,
    /// Per-turn dispatch counter (#583). Only one long-running dispatch is
    /// permitted per agent turn. Atomic for interior mutability through `&self`.
    pub dispatch_count: AtomicU32,
    /// Originating user-message text for this turn, when available.
    ///
    /// Populated in conversation-mode turns (the actual user/webhook input).
    /// `None` for silent triggers (`SilentTrigger::DeferredDispatch`, callback
    /// continuation turns) where there is no fresh user input — those paths
    /// have already passed an upstream gate.
    pub originating_message: Option<String>,
}

/// Validate that all required fields declared in a skill tool's input schema are
/// present and non-null in the supplied input.
///
/// Returns `Some(ToolOutput::error(...))` if validation fails, `None` if all
/// required fields are present (or the schema has no `required` key at all).
///
/// Scope: top-level `required` only. Does NOT validate `enum` constraints,
/// `type` assertions, or nested `properties`.
///
/// Post-#984: if `required` exists but is not a JSON array (malformed schema),
/// returns a structured `malformed_required_schema` error instead of silently passing.
pub fn validate_required_fields(
    skill_tool: &ResolvedSkillTool,
    input: &serde_json::Value,
) -> Option<ToolOutput> {
    let tool_name = &skill_tool.definition.name;
    let required_raw = skill_tool.definition.input_schema.get("required").cloned();
    let input_keys: Vec<&str> = input
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();

    let required_fields: Vec<&str> = required_raw
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .map(|arr| arr.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();

    if required_fields.is_empty() {
        // Check if `required` exists but isn't an array — indicates malformed schema.
        // Step 3.5 (#984): return a structured error instead of silently passing.
        if let Some(raw) = &required_raw
            && !raw.is_array()
        {
            warn!(
                tool = %tool_name,
                required_raw = ?raw,
                ?input_keys,
                "skill_tool_malformed_required_schema: \
                 'required' field exists but is not a JSON array — rejecting dispatch"
            );
            let error = serde_json::json!({
                "error": "malformed_required_schema",
                "tool": tool_name,
                "reason": "The tool's 'required' field in input_schema is not a JSON array. \
                           This indicates a schema configuration error. The dispatch cannot \
                           be validated and is rejected as a safety measure.",
                "required_raw_type": format!("{}", raw)
            });
            return Some(ToolOutput::error(error.to_string()));
        }
        // No `required` key at all — schema intentionally has no required fields.
        tracing::debug!(
            tool = %tool_name,
            ?input_keys,
            "validate_required_fields: no required fields declared in schema"
        );
        return None;
    }

    // F5 instrumentation (#984): log the schema and input state for diagnostics.
    // DEBUG on happy path (silent in production), WARN on any missing field (rare, surfaces immediately).
    let all_present = {
        let input_obj = input.as_object();
        required_fields.iter().all(|field| {
            input_obj
                .and_then(|obj| obj.get(*field))
                .is_some_and(|v| !v.is_null())
        })
    };

    if all_present {
        tracing::debug!(
            tool = %tool_name,
            ?input_keys,
            ?required_fields,
            "validate_required_fields: all required fields present"
        );
    } else {
        warn!(
            tool = %tool_name,
            ?input_keys,
            ?required_fields,
            required_raw = ?required_raw,
            "validate_required_fields: one or more required fields missing — will reject"
        );
    }

    let input_obj = input.as_object();
    for field in &required_fields {
        let is_present = input_obj
            .and_then(|obj| obj.get(*field))
            .is_some_and(|v| !v.is_null());

        if !is_present {
            warn!(
                tool = %tool_name,
                field = %field,
                "skill_tool_missing_required_field: \
                 required field not provided in tool call input"
            );

            // Collect valid_values from the field's `enum` constraint, if any
            let valid_values: Option<Vec<&str>> = skill_tool
                .definition
                .input_schema
                .get("properties")
                .and_then(|p| p.get(*field))
                .and_then(|f| f.get("enum"))
                .and_then(serde_json::Value::as_array)
                .map(|arr| arr.iter().filter_map(serde_json::Value::as_str).collect());

            let mut error = serde_json::json!({
                "error": "missing_required_field",
                "tool": skill_tool.definition.name,
                "field": field,
                "reason": format!(
                    "The '{}' field is required by the tool schema but was not provided in the tool call.",
                    field
                )
            });

            if let Some(values) = valid_values {
                error["valid_values"] = serde_json::json!(values);
            }

            return Some(ToolOutput::error(error.to_string()));
        }
    }

    None
}

/// Execute a skill tool with the appropriate handler.
///
/// Applies a per-skill timeout wrapping the inner execution.
/// If `long_running_ctx` is Some and the handler is `Exec { long_running: true }`,
/// the subprocess is spawned in the background with a callback task.
///
/// `callback_task_id` and `callback_db` enable deferred dispatch registration
/// from callback turns (mika#1058). When both are `Some`, the executor gate
/// intercepts long-running tool calls and registers them as deferred dispatches
/// instead of returning a hard error.
pub async fn execute_skill_tool(
    skill_tool: &ResolvedSkillTool,
    input: serde_json::Value,
    timeout_secs: u64,
    long_running_ctx: Option<&LongRunningContext>,
    github_token: Option<&str>,
    callback_task_id: Option<&str>,
    callback_db: Option<&AsyncDatabase>,
) -> ToolOutput {
    // Validate required fields from tool schema before any execution (#955).
    // Catches the bug class where the LLM omits a required field — the subprocess
    // never spawns, and the LLM gets a structured retry signal in the same turn.
    if let Some(error) = validate_required_fields(skill_tool, &input) {
        return error;
    }

    // Check for long-running exec handler
    if let ToolHandler::Exec {
        command,
        long_running: true,
        estimated_duration_secs,
    } = &skill_tool.handler
        && let Some(ctx) = long_running_ctx
    {
        return execute_long_running(
            skill_tool,
            command,
            input,
            *estimated_duration_secs,
            ctx,
            github_token,
        )
        .await;
    }

    // Refuse long-running tools when no long-running context is available
    // (callback turns, silent mode, CLI test). The sync exec path does not
    // inject __mika_task_id/__mika_agent, so the handler would crash with
    // a cryptic error. Return an explicit error instead (#537).
    //
    // mika#1058: Callback turns with a known task_id can register deferred
    // dispatches instead of receiving a hard error. The deferred callback fires
    // as a DeferredDispatch silent turn which HAS long_running_ctx injected.
    if matches!(
        &skill_tool.handler,
        ToolHandler::Exec {
            long_running: true,
            ..
        }
    ) && long_running_ctx.is_none()
    {
        // Callback turns: attempt deferred dispatch registration instead of hard error.
        if let Some(task_id) = callback_task_id
            && let Some(db) = callback_db
        {
            match check_lineage_cycle(db, task_id, &input).await {
                Ok(()) => {
                    if register_deferred_callback(db, task_id, &input).await {
                        info!(
                            tool = %skill_tool.definition.name,
                            task_id,
                            "callback_deferred_dispatch_registered"
                        );
                        return ToolOutput::success(
                            serde_json::json!({
                                "status": "deferred",
                                "message": "Long-running dispatch registered as deferred callback. \
                                            It will fire automatically when the current dispatch \
                                            slot is free. Do not retry.",
                                "deferred": true
                            })
                            .to_string(),
                        );
                    }
                    // Fall through to original error if registration failed (cap exceeded / DB error)
                }
                Err(cycle_msg) => {
                    warn!(
                        tool = %skill_tool.definition.name,
                        task_id,
                        "deferred_dispatch_cycle_detected"
                    );
                    return ToolOutput::error(
                        serde_json::json!({
                            "error": "deferred_dispatch_cycle_detected",
                            "message": cycle_msg,
                        })
                        .to_string(),
                    );
                }
            }
        }

        // Original error for non-callback contexts (heartbeat, reflection, CLI test)
        warn!(
            tool = %skill_tool.definition.name,
            "long-running tool invoked without long_running_ctx"
        );
        return ToolOutput::error(format!(
            "Tool '{}' is declared long_running but cannot run in the current context \
             (callback turn, silent mode, or CLI test). Long-running tools require a \
             conversation-mode turn with an active task engine.",
            skill_tool.definition.name
        ));
    }

    let timeout = Duration::from_secs(timeout_secs);
    match tokio::time::timeout(timeout, execute_inner(skill_tool, input, github_token)).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            warn!(
                tool = %skill_tool.definition.name,
                error = %e,
                "skill tool execution failed"
            );
            ToolOutput::error(format!("Skill tool error: {e}"))
        }
        Err(_) => {
            warn!(
                tool = %skill_tool.definition.name,
                timeout_secs,
                "skill tool timed out"
            );
            ToolOutput::error(format!(
                "Skill tool '{}' timed out after {timeout_secs}s",
                skill_tool.definition.name
            ))
        }
    }
}

async fn execute_inner(
    skill_tool: &ResolvedSkillTool,
    input: serde_json::Value,
    github_token: Option<&str>,
) -> Result<ToolOutput> {
    tracing::info!(
        tool = %skill_tool.definition.name,
        input = %input,
        "executing skill tool"
    );
    match &skill_tool.handler {
        ToolHandler::Exec { command, .. } => {
            execute_exec(
                command,
                &skill_tool.skill_dir,
                &skill_tool.definition.name,
                input,
                github_token,
            )
            .await
        }
        ToolHandler::Http { url, method } => execute_http(url, method, input).await,
        ToolHandler::Builtin { .. } => {
            // Builtin handlers are dispatched directly from agent.rs, not through executor.
            // This path should never be reached.
            bail!("Builtin handlers must be dispatched from the agent loop, not the executor")
        }
    }
}

/// Attempt to parse exec handler stdout as a `__mika_v1` image envelope.
///
/// Returns `Some(MikaOutput)` if the output is valid JSON with the sentinel key,
/// `None` otherwise (plain text output — backward compatible).
fn try_parse_envelope(stdout: &str) -> Option<MikaOutput> {
    let trimmed = stdout.trim();
    if !trimmed.starts_with('{') || !trimmed.contains(r#""__mika_v1""#) {
        return None;
    }
    serde_json::from_str::<MikaEnvelope>(trimmed)
        .ok()
        .map(|e| e.__mika_v1)
}

/// Read an image file from disk, validate it, and return base64-encoded data.
///
/// Security checks:
/// - Canonicalizes path (resolves symlinks)
/// - Verifies regular file (rejects devices, sockets, etc.)
/// - Enforces 5 MB size limit via metadata pre-check AND capped read (TOCTOU-safe)
/// - Magic-byte validation for supported image types (JPEG, PNG, GIF, WebP)
async fn read_and_validate_image(path: &str) -> Result<ImageData, String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        use std::fs;
        use std::io::Read;

        let canonical = fs::canonicalize(&path)
            .map_err(|e| format!("cannot resolve image path '{}': {}", path, e))?;

        let metadata = fs::metadata(&canonical)
            .map_err(|e| format!("cannot read image '{}': {}", canonical.display(), e))?;

        if !metadata.is_file() {
            return Err(format!("not a regular file: {}", canonical.display()));
        }

        if metadata.len() > MAX_IMAGE_SIZE {
            return Err(format!(
                "image too large: {} bytes (max {} bytes)",
                metadata.len(),
                MAX_IMAGE_SIZE
            ));
        }

        // Use capped read to prevent TOCTOU race (file could grow between metadata and read)
        let file = fs::File::open(&canonical)
            .map_err(|e| format!("cannot open image '{}': {}", canonical.display(), e))?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_IMAGE_SIZE + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("cannot read image '{}': {}", canonical.display(), e))?;

        if bytes.len() as u64 > MAX_IMAGE_SIZE {
            return Err(format!(
                "image too large: {} bytes (max {} bytes)",
                bytes.len(),
                MAX_IMAGE_SIZE
            ));
        }

        let media_type = detect_image_type(&bytes)
            .ok_or_else(|| format!("not a supported image type: {}", canonical.display()))?;

        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);

        Ok(ImageData {
            media_type: media_type.to_string(),
            data,
        })
    })
    .await
    .map_err(|e| format!("image read task panicked: {e}"))?
}

/// Detect image type from magic bytes. Returns MIME type or None.
fn detect_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        Some("image/png")
    } else if bytes.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        Some("image/gif")
    } else if bytes.len() >= 12
        && bytes[..4] == [0x52, 0x49, 0x46, 0x46]
        && bytes[8..12] == [0x57, 0x45, 0x42, 0x50]
    {
        Some("image/webp")
    } else {
        None
    }
}

/// Process image file paths from a Mika envelope, returning validated images
/// and any error notes for paths that couldn't be loaded.
async fn process_envelope_images(image_paths: &[String]) -> (Vec<ImageData>, Vec<String>) {
    let mut images = Vec::new();
    let mut errors = Vec::new();

    for (i, path) in image_paths.iter().enumerate() {
        if i >= MAX_IMAGES_PER_RESULT {
            errors.push(format!(
                "skipped {} image(s): max {} per result",
                image_paths.len() - MAX_IMAGES_PER_RESULT,
                MAX_IMAGES_PER_RESULT
            ));
            break;
        }
        match read_and_validate_image(path).await {
            Ok(img) => images.push(img),
            Err(e) => {
                warn!(path, error = %e, "failed to load envelope image");
                errors.push(e);
            }
        }
    }

    (images, errors)
}

/// Execute an exec-type handler by spawning a subprocess.
///
/// - Resolves the command path relative to the skill directory
/// - Pipes input JSON to stdin
/// - Returns stdout regardless of exit code; prefixes `Exit code: N` on non-zero
/// - Detects `__mika_v1` envelope for image-bearing results (exit 0 only)
async fn execute_exec(
    command: &str,
    skill_dir: &std::path::Path,
    tool_name: &str,
    input: serde_json::Value,
    github_token: Option<&str>,
) -> Result<ToolOutput> {
    // Resolve command relative to skill directory
    let cmd_path = skill_dir.join(command);
    info!(command = %cmd_path.display(), "executing skill command");
    if !cmd_path.exists() {
        bail!(
            "handler command not found: {} (resolved to {})",
            command,
            cmd_path.display()
        );
    }

    let mut child = loop {
        let mut cmd = tokio::process::Command::new(&cmd_path);
        cmd.current_dir(skill_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .kill_on_drop(true);
        scrub_mika_env_vars(&mut cmd);
        // Re-inject agent's GitHub token for platform identity separation.
        // Same pattern as builtin run_gh handler (builtin_handlers.rs).
        if let Some(token) = github_token {
            cmd.env("GH_TOKEN", token);
        }
        match cmd.spawn() {
            Ok(child) => break child,
            Err(e) if e.raw_os_error() == Some(26 /* ETXTBSY */) => {
                // ETXTBSY — another process has the file open for writing.
                // Retry after a brief yield (common fork+exec race).
                tokio::task::yield_now().await;
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    };

    // Write input JSON to stdin and close.
    // Ignore BrokenPipe — the child may exit without reading stdin.
    if let Some(mut stdin) = child.stdin.take() {
        let input_bytes = serde_json::to_vec(&input)?;
        match stdin.write_all(&input_bytes).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(e) => return Err(e.into()),
        }
        // stdin is dropped here, closing the pipe
    }

    let output = child.wait_with_output().await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Log output for debugging (regardless of exit code)
    // Use char-boundary-safe slicing to avoid panics on multi-byte UTF-8
    let stdout_end = {
        let mut b = stdout.len().min(200);
        while b > 0 && !stdout.is_char_boundary(b) {
            b -= 1;
        }
        b
    };
    debug!(
        tool = %tool_name,
        exit_success = output.status.success(),
        stdout_len = stdout.len(),
        stdout_preview = %&stdout[..stdout_end],
        "skill exec output"
    );
    if !stderr.trim().is_empty() {
        let stderr_end = {
            let mut b = stderr.len().min(500);
            while b > 0 && !stderr.is_char_boundary(b) {
                b -= 1;
            }
            b
        };
        debug!(
            tool = %tool_name,
            stderr = %&stderr[..stderr_end],
            "skill exec stderr"
        );
    }

    if output.status.success() {
        // Exit 0: parse envelope and return stdout
        if let Some(envelope) = try_parse_envelope(&stdout) {
            let (images, errors) = process_envelope_images(&envelope.images).await;
            let mut text = truncate_output(&envelope.text);
            if !errors.is_empty() {
                text.push_str("\n[image errors: ");
                text.push_str(&errors.join("; "));
                text.push(']');
            }
            if images.is_empty() {
                Ok(ToolOutput::success(text))
            } else {
                Ok(ToolOutput::success_with_images(text, images))
            }
        } else {
            Ok(ToolOutput::success(truncate_output(&stdout)))
        }
    } else {
        // Non-zero exit: return success with exit code prefix.
        // The agent decides whether the exit code represents a real failure —
        // many tools (grep, linters, health checks) use non-zero to signal
        // status, not errors.
        let code_display = match output.status.code() {
            Some(code) => format!("Exit code: {code}"),
            None => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    match output.status.signal() {
                        Some(sig) => format!("Killed by signal: {sig}"),
                        None => "Exit code: unknown".to_string(),
                    }
                }
                #[cfg(not(unix))]
                {
                    "Exit code: unknown".to_string()
                }
            }
        };

        // Combine stdout and stderr. Append stderr only if it has content
        // not already in stdout (run.sh merges them with 2>&1).
        let mut combined = stdout.to_string();
        let stderr_trimmed = stderr.trim();
        if !stderr_trimmed.is_empty() && stderr_trimmed != stdout.trim() {
            if !combined.is_empty() && !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str(stderr_trimmed);
        }

        let truncated = truncate_output(&combined);
        Ok(ToolOutput::success(format!("{code_display}\n{truncated}")))
    }
}

/// Execute an HTTP-type handler by making an HTTP request.
///
/// - POST/PUT: sends input as JSON body
/// - GET: sends input as query parameters
async fn execute_http(url: &str, method: &str, input: serde_json::Value) -> Result<ToolOutput> {
    let client = reqwest::Client::new();

    let request = match method.to_uppercase().as_str() {
        "GET" => {
            // For GET, serialize input object as query params
            let mut req = client.get(url);
            if let serde_json::Value::Object(map) = &input {
                let params: Vec<(String, String)> = map
                    .iter()
                    .map(|(k, v)| {
                        let val = match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        (k.clone(), val)
                    })
                    .collect();
                req = req.query(&params);
            }
            req
        }
        "POST" => client.post(url).json(&input),
        "PUT" => client.put(url).json(&input),
        other => bail!("unsupported HTTP method: {other}"),
    };

    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await?;

    if status.is_success() {
        Ok(ToolOutput::success(truncate_output(&body)))
    } else {
        Ok(ToolOutput::error(format!(
            "HTTP {}: {}",
            status.as_u16(),
            truncate_output(&body)
        )))
    }
}

/// Truncate output to MAX_OUTPUT_LEN characters.
fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_LEN {
        s.to_string()
    } else {
        let mut boundary = MAX_OUTPUT_LEN;
        while boundary > 0 && !s.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!(
            "{}\n... (truncated at {MAX_OUTPUT_LEN} chars)",
            &s[..boundary]
        )
    }
}

use crate::github_graphql::{
    fetch_issue_labels, fetch_issue_milestone_number, fetch_milestone_issues_by_state,
    fetch_open_blockers, parse_phase_label,
};

/// Derive the dispatch class from a skill name (#1001).
///
/// Used by the per-class dispatch slot split to determine which concurrency
/// slot a dispatch occupies. `"groom"` class allows grooming to run concurrently
/// with implementation; all other skills are `"implement"` class.
// COUPLED PAIR (mika#1175): when adding a new arm here, also update
// `DISPATCH_CLASSES` in `task_engine/engine.rs` AND the probe-list inside
// `test_dispatch_classes_universe_matches_derive_fn` (same file). The drift
// test compares this function's outputs against the slice, so a new class
// is silently lost from the periodic backstop unless all three sites move
// together.
pub(crate) fn derive_dispatch_class(skill: Option<&str>) -> &'static str {
    match skill {
        Some("dev-groom") => "groom",
        _ => "implement", // dev-pilot, deploy_mika, and all others
    }
}

/// Extract the skill name from a tool input JSON value.
fn extract_skill_from_input(input: &serde_json::Value) -> Option<&str> {
    input.get("skill").and_then(|v| v.as_str())
}

/// Check an issue body for the three canonical grooming-marker signals (#919).
///
/// Returns a list of missing signal names. Empty list means all signals present.
/// The three load-bearing substrings match the canonical `/mika-groom-ticket`
/// Phase 5 callout shape. Both this function and the prompt-level check at
/// `skills/bundled/self-dev/system_prompt.md:253` must update together if
/// the callout shape changes.
///
/// The plan callout uses `docs/plans/` as the path-prefix substring rather
/// than `Plan: docs/plans/` because the canonical callout shape in the issue
/// body is `> - **Plan:** \`<repo>/docs/plans/<file>\`` — the bold markdown
/// and backtick-wrapping mean `Plan: docs/plans/` never appears as a
/// contiguous substring. `docs/plans/` is the essential anchoring directory
/// prefix.
///
/// The groomed-verdict check accepts the canonical `second-pass (GROOMED)`,
/// the spec-tolerated paraphrase `second-pass (READY, paraphrased GROOMED`,
/// and parameterized/annotated variants like `second-pass (GROOMED, session ...)`
/// or `second-pass (GROOMED — session-id: ...)` (#1725). The grooming spec's
/// Phase 5 and orchestrator-manual verdict-landing both routinely emit
/// parameterized forms; the gate must accept everything the spec authorizes.
///
/// # Regex shape
///
/// `second-pass \(GROOMED[\s\)\.,;:—-]` — the character class after `GROOMED` is
/// the structural discriminator. It matches:
/// - `)` (canonical strict form: `second-pass (GROOMED)`)
/// - `,` (parameter: `second-pass (GROOMED, session abc)`)
/// - `.` (terminator: `second-pass (GROOMED. Full ratification.)`)
/// - `;` `:` (annotators)
/// - `—` `-` (dash-separated annotation: `second-pass (GROOMED — session-id: uuid)`)
/// - whitespace (any word-boundary follow-on)
///
/// Anchoring to the `second-pass (` prefix + a delimiter after `GROOMED`
/// structurally distinguishes the verdict-line callout from prose like
/// "the ticket was GROOMED yesterday" or "GROOMED status pending".
static GROOMED_VERDICT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"second-pass \(GROOMED[\s\)\.,;:—-]").expect("groomed verdict regex must compile")
});
static PARAPHRASED_GROOMED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"second-pass \(READY, paraphrased GROOMED")
        .expect("paraphrased groomed regex must compile")
});
/// Single-pass grooming verdict (mika#2012).
///
/// The first-pass READY disposition is a legitimate grooming exit — see
/// `/mika-groom-ticket` Phase 3 step 10: "Disposition: READY — plan is sound.
/// Commit the staged plan […] and skip to Phase 5". No second pass runs, so the
/// body must not claim one; `write_canonical_callout`'s `ready-single-pass`
/// stage emits this truthful marker instead.
///
/// Before mika#2012 this shape had no stage and no regex: a ticket groomed in
/// one pass stayed permanently invisible to the dispatch gate, was re-dispatched
/// as `dev-groom`, re-groomed, and looped — 25 measured requeues across 5
/// tickets in 13 h, producing 6 branches containing only markdown plans.
///
/// Anchored on the `first-pass (` prefix for the same reason as
/// `GROOMED_VERDICT_RE`: it distinguishes the verdict-line callout from prose.
static SINGLE_PASS_GROOMED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"first-pass \(READY, single-pass GROOMED")
        .expect("single-pass groomed regex must compile")
});

pub fn check_grooming_markers(issue_body: &str) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !issue_body.contains("> - **Branch:**") {
        missing.push("branch_callout");
    }
    if !issue_body.contains("docs/plans/") {
        missing.push("plan_callout");
    }
    let has_groomed_marker = GROOMED_VERDICT_RE.is_match(issue_body)
        || PARAPHRASED_GROOMED_RE.is_match(issue_body)
        || SINGLE_PASS_GROOMED_RE.is_match(issue_body);
    if !has_groomed_marker {
        missing.push("groomed_verdict");
    }
    missing
}

/// Best-effort write of a dispatch-rejection reason to `tasks.result` (#1108).
///
/// Fire-and-forget: logs a warning on failure but never propagates the error.
/// This surfaces rejection reasons to operator-visible surfaces (`mika tasks list`,
/// dashboard task detail) without requiring DB-level inspection.
async fn record_dispatch_rejection(db: &AsyncDatabase, task_id: &str, reason_json: &str) {
    if let Err(e) = db.write_task_dispatch_rejection(task_id, reason_json).await {
        warn!(
            task_id = task_id,
            error = %e,
            "failed to write dispatch-rejection reason to tasks.result"
        );
    }
}

/// Validate that a task is in a dispatchable state for long-running execution.
///
/// Stricter than `validate_task()` (which also allows `blocked` for delegation).
/// Long-running dispatch only permits `pending` and `in_progress`, and rejects if an
/// active callback child task already exists (double-dispatch prevention).
///
/// Returns `Err(json_error_string)` on rejection, `Ok(status)` with the task's
/// current status if dispatch may proceed. Each rejection site also writes the
/// structured reason to `tasks.result` (#1108) for operator visibility.
//
// DOCTRINE: pre-classifier structural gate (mika#1733 AC2)
// Applies per crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md §AC2:
// "This agent structurally cannot do X" applies to pre-classifier engine gates
// only, NEVER to LLM classifier decisions. This is such a gate — it rejects a
// dispatch before any LLM classifier runs, based on structural task state
// (status, callback children, grooming callouts, blockedBy edges) that the
// classifier is not competent to evaluate.
//
// NOTE: The tier1/tier2/tier3 permission classifier code lives in
// claude-pilot-py; the companion doctrine anchor for those sites is tracked
// as a cross-repo follow-up filed alongside this PR (see PR body §Follow-ups).
// This annotation covers the in-mika-agent structural gate only. See mika#1193
// for the retirement of the in-repo `permission-policy` skill that moved the
// classifier tiers into claude-pilot-py.
pub(crate) async fn validate_dispatch_readiness(
    db: &AsyncDatabase,
    task_id: &str,
    github_token: Option<&str>,
    tool_input: Option<&serde_json::Value>,
    originating_message: Option<&str>,
) -> Result<String, String> {
    // #933 — Tool-boundary gate for unauthorized webhook dispatch. Cheapest check
    // (pure string-prefix match, no DB), runs first. Rejects `run_claude_pilot`
    // when the originating user message is in the Webhook Fallthrough domain.
    if let Some(msg) = originating_message
        && crate::webhook_dispatch::is_unauthorized_webhook_dispatch(msg)
    {
        let rejection = serde_json::json!({
            "error": "unauthorized_webhook_dispatch",
            "task_id": task_id,
            "reason": "This turn was initiated by a [GitHub] webhook event in the \
                       Webhook Fallthrough domain (issue events, comments, or \
                       unknown event types). Only `[GitHub] Issue labeled ready on` \
                       webhooks (authorized dispatch) and PR / Check-suite events \
                       handled by self-dev-webhook-qa / self-dev-webhook-ci skills \
                       may dispatch claude-pilot. All other webhook events must use \
                       Webhook Fallthrough: acknowledge without dispatching \
                       (mika#841 positive-consent contract, mika#933)."
        });
        record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
        return Err(rejection.to_string());
    }

    // mika#2046 — Tool-boundary gate for the dispatchable-repository allowlist.
    // Pure string handling, no DB access, so it sits with the other cheap checks
    // ahead of the task fetch.
    //
    // This is the load-bearing layer. `run_claude_pilot` spawns a subprocess and
    // creates a worktree, so a guard that only fires after the tool has run
    // detects the violation without preventing it
    // (docs/solutions/architecture-patterns/post-hoc-vs-tool-boundary-guard-placement-2026-05-13.md).
    // The pre-LLM ready-label handler refuses the webhook path; this refuses
    // every path, whatever originated the turn.
    if let Some(prompt) = tool_input
        .and_then(|input| input.get("prompt"))
        .and_then(|v| v.as_str())
        && let Some(repo_ref) = crate::webhook_dispatch::parse_repo_ref_from_dispatch_prompt(prompt)
        && !crate::webhook_dispatch::is_dispatchable_repo(repo_ref)
    {
        let owner_repo = crate::webhook_dispatch::normalize_owner_repo(repo_ref);
        let allowed = crate::webhook_dispatch::dispatchable_repos_display();
        let rejection = serde_json::json!({
            "error": "repo_not_dispatchable",
            "task_id": task_id,
            "repo": owner_repo,
            "reason": format!(
                "`{owner_repo}` is not a repository the autonomous loop may dispatch \
                 into. Dispatchable repositories: {allowed}. Repositories outside \
                 this list are Claude Code spawn territory and are never reached by \
                 the loop; this is a structural gate, not a transient failure, so \
                 retrying the dispatch will not clear it (mika#2046)."
            )
        });
        record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
        return Err(rejection.to_string());
    }

    // Re-fetch the task to get the full struct (validate_task confirmed existence)
    let task = match db.get_task(task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            // Should not happen after validate_task, but defense-in-depth
            return Err(serde_json::json!({
                "error": "task_not_found",
                "task_id": task_id,
                "reason": "Task does not exist in the database"
            })
            .to_string());
        }
        Err(e) => {
            return Err(serde_json::json!({
                "error": "dispatch_check_failed",
                "task_id": task_id,
                "reason": format!("Failed to fetch task for dispatch check: {e}")
            })
            .to_string());
        }
    };

    // Only pending and in_progress are dispatchable
    if !matches!(task.status.as_str(), "pending" | "in_progress") {
        let pr_url = extract_pr_url(&task.metadata);
        let rejection = serde_json::json!({
            "error": "task_not_dispatchable",
            "task_id": task_id,
            "current_status": task.status,
            "pr_url": pr_url,
            "reason": format!(
                "Task is in '{}' state and cannot be dispatched. \
                 Only 'pending' and 'in_progress' tasks can be dispatched.",
                task.status
            )
        });
        record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
        return Err(rejection.to_string());
    }

    // Check for active callback children (double-dispatch prevention)
    match db.get_child_tasks(task_id).await {
        Ok(children) => {
            let active_callback = children.iter().find(|c| {
                c.trigger_type == "callback"
                    && matches!(c.status.as_str(), "pending" | "in_progress")
            });
            if let Some(child) = active_callback {
                let pr_url = extract_pr_url(&task.metadata);
                let rejection = serde_json::json!({
                    "error": "task_active_dispatch",
                    "task_id": task_id,
                    "current_status": task.status,
                    "active_child_id": child.id,
                    "active_child_status": child.status,
                    "pr_url": pr_url,
                    "reason": format!(
                        "Task already has an active dispatch (callback task '{}' \
                         in '{}' status). Wait for it to complete or cancel it before \
                         dispatching again.",
                        child.id, child.status
                    )
                });
                record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
                return Err(rejection.to_string());
            }
        }
        Err(e) => {
            // Fail-closed: if we can't check children, reject dispatch
            return Err(serde_json::json!({
                "error": "dispatch_check_failed",
                "task_id": task_id,
                "reason": format!("Failed to check active dispatches for task: {e}")
            })
            .to_string());
        }
    }

    // Per-class dispatch guard (#583, #1001): reject if another task of the
    // SAME dispatch class has an active callback child. The slot split allows
    // one 'implement' + one 'groom' dispatch concurrently per agent.
    let dispatch_class = tool_input.and_then(extract_skill_from_input);
    let class = derive_dispatch_class(dispatch_class);
    match db.has_active_callback_tasks_excluding(task_id, class).await {
        Ok(Some((blocking_parent_id, blocking_callback_id, blocking_label))) => {
            // mika#1011 — Register a deferred-dispatch callback so the engine
            // auto-retries when the blocking dispatch completes. The LLM still
            // sees the rejection (γ composition) and may call send_message;
            // both paths are independent and validate_dispatch_readiness()
            // arbitrates any race on the next dispatch attempt.
            let deferred_registered = if let Some(input) = tool_input {
                register_deferred_callback(db, task_id, input).await
            } else {
                false
            };

            // Derive blocker_kind from the blocking callback's label (#1172 W3).
            let blocker_kind = if blocking_label.ends_with(":deferred") {
                "deferred_wrapper"
            } else {
                "real_callback"
            };

            let mut rejection = serde_json::json!({
                "error": "global_dispatch_active",
                "task_id": task_id,
                "dispatch_class": class,
                "blocking_task_id": blocking_parent_id,
                "blocking_callback_id": blocking_callback_id,
                "blocking_label": blocking_label,
                "blocker_kind": blocker_kind,
                "reason": format!(
                    "Another task ('{}') already has an active {} dispatch \
                     (callback task '{}'). Only one long-running dispatch per class may be \
                     active at a time. Wait for it to complete or cancel it before \
                     dispatching again.",
                    blocking_parent_id, class, blocking_callback_id
                )
            });
            if deferred_registered {
                rejection["deferred_dispatch_registered"] = serde_json::json!(true);
                // W4: audit event for deferred registration (#1172)
                if let Err(e) = db
                    .log_audit_event(
                        "system",
                        "deferred_dispatch_registered",
                        &format!("task:{task_id}"),
                        None,
                        Some("deferred"),
                        Some(&format!(
                            "dispatch_class:{class}, blocking:{blocking_parent_id}"
                        )),
                        None,
                    )
                    .await
                {
                    warn!(error = %e, "failed to write deferred_dispatch_registered audit event");
                }
            }
            record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
            return Err(rejection.to_string());
        }
        Ok(None) => { /* No conflicting dispatch in this class — proceed */ }
        Err(e) => {
            // Fail-closed: if we can't check global state, reject dispatch
            return Err(serde_json::json!({
                "error": "dispatch_check_failed",
                "task_id": task_id,
                "reason": format!("Failed to check global dispatch state: {e}")
            })
            .to_string());
        }
    }

    // Manual re-dispatch state-awareness guard (mika#920): reject `dev-pilot`
    // re-dispatch when the task already has an open PR and the caller did not
    // pass `iteration_context`. The autonomous retry paths (verdict_handler,
    // ci_failure_handler) supply `iteration_context` and bypass this guard;
    // engine-initiated recovery (DeferredDispatch) and operator positive-consent
    // (ready-label webhook) bypass via dedicated predicates.
    if let Some(rejection) =
        check_task_has_open_pr(&task, tool_input, originating_message, github_token).await
    {
        record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
        return Err(rejection.to_string());
    }

    // Hoist the GitHub ref parse above both the grooming-marker check (#919)
    // and the blocked-by check (#713) so they share the binding.
    let github_ref = task.reference_url.as_deref().and_then(parse_github_ref);

    // Grooming-marker check (#919): reject dev-pilot dispatch when the target
    // issue body lacks the three canonical grooming callouts (Branch + Plan +
    // architect verdict). Engine-level gate — all dispatch paths (webhook,
    // CLI ask, sprint, free-text) funnel through here.
    //
    // Coupled pair: the prompt-level check at
    // `skills/bundled/self-dev/system_prompt.md:253` is defense-in-depth.
    // Both must update if the canonical `/mika-groom-ticket` Phase 5 callout
    // shape changes. Load-bearing substrings: `> - **Branch:**`,
    // `docs/plans/`, and a `second-pass` marker (canonical `(GROOMED)` or
    // spec-tolerated `(READY, paraphrased GROOMED ...)` per #1108).
    if let Some(GitHubRef::Issue {
        ref owner,
        ref repo,
        number,
    }) = github_ref
    {
        // Bypass 1: only gate dev-pilot dispatches (dev-groom is the marker
        // producer; other skills are out of scope for #919)
        let skill = tool_input.and_then(extract_skill_from_input);
        let is_dev_pilot = skill == Some("dev-pilot");

        // Bypass 2: milestones/projects don't carry plans on their own bodies
        let is_issue_type = task.r#type == db::TASK_TYPE_ISSUE;

        // Bypass 4: env var emergency override
        let bypass_env = std::env::var("MIKA_DISPATCH_BYPASS_GROOMING_CHECK")
            .map(|v| v.eq_ignore_ascii_case("1") || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if is_dev_pilot && is_issue_type {
            if bypass_env {
                warn!(
                    task_id = task_id,
                    owner = %owner,
                    repo = %repo,
                    number = number,
                    "dispatch grooming marker check bypassed via env var"
                );
            } else {
                match github_token {
                    Some(token) => {
                        match fetch_issue_body(token, owner, repo, number).await {
                            Ok(issue_body) => {
                                let missing = check_grooming_markers(&issue_body);

                                if !missing.is_empty() {
                                    let rejection = serde_json::json!({
                                        "error": "dispatch_no_grooming_marker",
                                        "task_id": task_id,
                                        "issue": format!("{}/{}#{}", owner, repo, number),
                                        "missing_signals": missing,
                                        "predicate": "issue body must contain all three substrings: \
                                                      '> - **Branch:**', 'docs/plans/', and a second-pass \
                                                      marker ('(GROOMED)' or '(READY, paraphrased GROOMED ...)')",
                                        "recovery": "Run /mika-groom-ticket <ref> to produce the canonical \
                                                     callout block, or dispatch dev-groom first via \
                                                     'mika ask --agent mika-dev \"groom <typed-ref>\"', or \
                                                     set MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1 to bypass.",
                                        "reason": format!(
                                            "Cannot dispatch dev-pilot on ticket #{number}: issue body is \
                                             missing one or more grooming-marker signals. The grooming-marker \
                                             gate ensures architect-reviewed plans are committed before \
                                             implementation begins (mika#907, mika#919)."
                                        )
                                    });
                                    record_dispatch_rejection(db, task_id, &rejection.to_string())
                                        .await;
                                    return Err(rejection.to_string());
                                }

                                // Grooming provenance cross-check (#1620): markers are
                                // present but may have been pre-stamped by a manual
                                // /mika-ask-arch session. Verify a completed groom-class
                                // task exists for this issue in the DB.
                                let issue_url = format!(
                                    "https://github.com/{}/{}/issues/{}",
                                    owner, repo, number
                                );
                                match db.has_completed_groom_for_issue(&issue_url).await {
                                    Ok(true) => {
                                        // Autonomous groom confirmed — proceed
                                    }
                                    Ok(false) => {
                                        let rejection = serde_json::json!({
                                            "error": "dispatch_grooming_not_verified",
                                            "task_id": task_id,
                                            "issue": format!("{}/{}#{}", owner, repo, number),
                                            "predicate": "issue body has grooming markers but no \
                                                          completed dispatch_class='groom' task exists \
                                                          for this issue — markers may be pre-stamped \
                                                          from a manual /mika-ask-arch session",
                                            "recovery": "Run /mika-groom-ticket <ref> to groom via \
                                                         the autonomous loop, or set \
                                                         MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1 to bypass.",
                                            "reason": format!(
                                                "Cannot dispatch dev-pilot on ticket #{number}: \
                                                 grooming markers are present in the issue body but \
                                                 no completed autonomous groom task was found. The \
                                                 dispatch-classification gate requires structural \
                                                 proof of grooming (mika#1620)."
                                            )
                                        });
                                        record_dispatch_rejection(
                                            db,
                                            task_id,
                                            &rejection.to_string(),
                                        )
                                        .await;
                                        return Err(rejection.to_string());
                                    }
                                    Err(e) => {
                                        // Fail-open on DB error (consistent with
                                        // no-token fail-open behavior)
                                        warn!(
                                            task_id = task_id,
                                            error = %e,
                                            "grooming provenance cross-check failed, \
                                             allowing dispatch (fail-open)"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                // Fail-closed: token present but API error
                                warn!(
                                    task_id = task_id,
                                    error = %e,
                                    "grooming-marker check failed, rejecting dispatch"
                                );
                                return Err(serde_json::json!({
                                    "error": "dispatch_check_failed",
                                    "task_id": task_id,
                                    "reason": format!("Failed to fetch issue body for grooming-marker check: {e}")
                                })
                                .to_string());
                            }
                        }
                    }
                    None => {
                        // Fail-open: no token configured (mirrors blocked-by behavior)
                        warn!(
                            task_id = task_id,
                            "Skipping grooming-marker check: no GitHub token configured"
                        );
                    }
                }
            }
        }
    }

    // Phase guard (mika#1153 E4): reject dispatch when the target issue has a
    // `phase:N` label (N > 1) and any phase-(N-1) sub-issues in the same
    // milestone are still OPEN. Defense-in-depth over the `blockedBy` GraphQL guard.
    // Position: between grooming-marker (check 5) and blocked-by (check 6).
    // Cost: 1-2 REST API calls (medium), cheaper than GraphQL blocked-by.
    if let Some(GitHubRef::Issue {
        ref owner,
        ref repo,
        number,
    }) = github_ref
    {
        // Only gate issue-type tasks (matching check #5 pattern).
        if task.r#type == db::TASK_TYPE_ISSUE {
            match github_token {
                Some(token) => {
                    match fetch_issue_labels(token, owner, repo, number).await {
                        Ok(labels) => {
                            if let Some(phase) = parse_phase_label(&labels)
                                && phase > 1
                            {
                                // Fetch the issue's milestone number from the GitHub API.
                                let milestone_number =
                                    fetch_issue_milestone_number(token, owner, repo, number)
                                        .await
                                        .ok();

                                if let Some(ms_num) = milestone_number {
                                    // Fetch open issues in the milestone and check for open phase-(N-1) issues.
                                    match fetch_milestone_issues_by_state(
                                        token, owner, repo, ms_num, "open",
                                    )
                                    .await
                                    {
                                        Ok(open_issues) => {
                                            let prior_phase = phase - 1;
                                            let open_in_prior_phase: Vec<u64> = open_issues
                                                .iter()
                                                .filter(|issue| {
                                                    parse_phase_label(&issue.labels)
                                                        == Some(prior_phase)
                                                })
                                                .map(|issue| issue.number)
                                                .collect();

                                            if !open_in_prior_phase.is_empty() {
                                                let rejection = serde_json::json!({
                                                    "error": "dispatch_phase_blocked",
                                                    "task_id": task_id,
                                                    "phase": phase,
                                                    "blocking_phase": prior_phase,
                                                    "open_issues_in_prior_phase": open_in_prior_phase,
                                                    "reason": format!(
                                                        "Cannot dispatch phase-{phase} issue #{number}: \
                                                         {count} phase-{prior_phase} issue(s) are still open \
                                                         ({issues}). Phase-{prior_phase} must complete before \
                                                         phase-{phase} can be dispatched.",
                                                        count = open_in_prior_phase.len(),
                                                        issues = open_in_prior_phase
                                                            .iter()
                                                            .map(|n| format!("#{n}"))
                                                            .collect::<Vec<_>>()
                                                            .join(", ")
                                                    )
                                                });
                                                record_dispatch_rejection(
                                                    db,
                                                    task_id,
                                                    &rejection.to_string(),
                                                )
                                                .await;
                                                return Err(rejection.to_string());
                                            }
                                        }
                                        Err(e) => {
                                            // Fail-closed: reject dispatch on API error
                                            warn!(
                                                task_id = task_id,
                                                error = %e,
                                                "phase guard: failed to fetch milestone issues, rejecting dispatch"
                                            );
                                            return Err(serde_json::json!({
                                                "error": "dispatch_check_failed",
                                                "task_id": task_id,
                                                "reason": format!(
                                                    "Failed to fetch milestone issues for phase guard: {e}"
                                                )
                                            })
                                            .to_string());
                                        }
                                    }
                                }
                                // If we can't determine the milestone number, skip (can't check).
                            }
                            // No phase label or phase == 1: bypass guard.
                        }
                        Err(e) => {
                            // Fail-closed: reject dispatch on API error (matching check #6 pattern)
                            warn!(
                                task_id = task_id,
                                error = %e,
                                "phase guard: failed to fetch issue labels, rejecting dispatch"
                            );
                            return Err(serde_json::json!({
                                "error": "dispatch_check_failed",
                                "task_id": task_id,
                                "reason": format!(
                                    "Failed to fetch issue labels for phase guard: {e}"
                                )
                            })
                            .to_string());
                        }
                    }
                }
                None => {
                    // Fail-open: no token configured (matching check #6 pattern)
                    warn!(
                        task_id = task_id,
                        "Skipping phase guard: no GitHub token configured"
                    );
                }
            }
        }
    }

    // Blocked-by guard (#713): reject dispatch if the ticket's GitHub blockers
    // are still open. This is the most expensive check (external API call) so it
    // runs last, after all cheap DB checks have passed.
    if let Some(GitHubRef::Issue {
        owner,
        repo,
        number,
    }) = github_ref
    {
        match github_token {
            Some(token) => match fetch_open_blockers(token, &owner, &repo, number).await {
                Ok(blockers) if !blockers.is_empty() => {
                    let rejection = serde_json::json!({
                        "error": "dispatch_blocked_by",
                        "task_id": task_id,
                        "blocking_issues": blockers,
                        "message": format!(
                            "ticket #{number} is blocked by {} which {} still open",
                            blockers.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", "),
                            if blockers.len() == 1 { "is" } else { "are" }
                        )
                    });
                    record_dispatch_rejection(db, task_id, &rejection.to_string()).await;
                    return Err(rejection.to_string());
                }
                Ok(_) => { /* No open blockers — proceed */ }
                Err(e) => {
                    // Fail-closed: if we can't verify blocker state, reject dispatch
                    warn!(
                        task_id = task_id,
                        error = %e,
                        "blocked-by check failed, rejecting dispatch"
                    );
                    return Err(serde_json::json!({
                        "error": "dispatch_check_failed",
                        "task_id": task_id,
                        "reason": format!("Failed to check blocked-by status: {e}")
                    })
                    .to_string());
                }
            },
            None => {
                warn!(
                    task_id = task_id,
                    "Skipping blocked-by check: no GitHub token configured"
                );
            }
        }
    }

    Ok(task.status.clone())
}

/// Engine-internal sentinel field name (mika#920 F3 bypass).
///
/// Injected into `original_call` by `register_deferred_callback()` so that the
/// `DeferredDispatch` replay turn bypasses the `dispatch_task_has_open_pr`
/// guard. Without this bypass, deferred recoveries from `global_dispatch_active`
/// rejections would livelock: deferred fires → guard rejects → re-defer → repeat.
///
/// The `__internal_*` prefix marks this as engine-internal — not part of the
/// public `run_claude_pilot` tool schema. The dev-pilot `tools.json` MUST NOT
/// advertise this field; correctness depends on the schema's permissive
/// `additionalProperties` mode (see plan Anchor F).
pub(crate) const INTERNAL_DEFERRED_DISPATCH_FIELD: &str = "__internal_deferred_dispatch";

/// Build the `dispatch_task_has_open_pr` rejection (mika#920) if all guard
/// conditions are met, otherwise return `None` to allow dispatch.
///
/// Bypass conditions (evaluated in order — any one short-circuits to `None`):
/// 1. `iteration_context` is present in `tool_input` — explicit operator/handler
///    re-dispatch with context (autonomous verdict/ci handlers, or manual
///    operator-with-context invocations).
/// 2. Skill is not `dev-pilot` — `dev-groom` is fresh grooming, not an
///    implementation re-run.
/// 3. Task has no `claude_pilot.pr_url` in metadata — fresh dispatch, no prior
///    PR to conflict with.
/// 4. `originating_message` matches the ready-label webhook marker — the
///    operator's positive-consent signal (mika#841). Coupled pair with the
///    `[GitHub] Issue labeled ready on` format in
///    `mika_gateway::github::format_event_text`; changes to either side must
///    update both.
/// 5. `tool_input` carries the `__internal_deferred_dispatch` sentinel —
///    engine-initiated recovery from a prior `global_dispatch_active`
///    rejection (mika#1011/#1058). Without this bypass the deferred replay
///    would livelock against this guard.
///
/// When no bypass fires, the function attempts a best-effort GitHub REST
/// enrichment (PR state, latest mika-qa verdict, mergeable_state). API
/// failures degrade gracefully: the core decision is based on `pr_url`
/// presence in DB metadata, enrichment only fills out the rejection body.
async fn check_task_has_open_pr(
    task: &db::Task,
    tool_input: Option<&serde_json::Value>,
    originating_message: Option<&str>,
    github_token: Option<&str>,
) -> Option<serde_json::Value> {
    let input = tool_input?;

    // Bypass 1: explicit iteration_context — caller already supplied state context.
    if let Some(ctx) = input.get("iteration_context").and_then(|v| v.as_str())
        && !ctx.is_empty()
    {
        return None;
    }

    // Bypass 5 (F3): engine-initiated deferred-dispatch replay.
    if input
        .get(INTERNAL_DEFERRED_DISPATCH_FIELD)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }

    // Bypass 2: only dev-pilot dispatches are in scope.
    let skill = extract_skill_from_input(input);
    if skill != Some("dev-pilot") {
        return None;
    }

    // Bypass 4 (F2): ready-label webhook is the operator's positive-consent path.
    if let Some(msg) = originating_message
        && crate::webhook_dispatch::is_ready_label_dispatch_marker(msg)
    {
        return None;
    }

    // Bypass 3: no prior PR in metadata → fresh dispatch, nothing to conflict with.
    let pr_url = extract_pr_url(&task.metadata)?;

    // Best-effort GitHub REST enrichment. The rejection decision is already
    // made (pr_url present + no bypass) — enrichment only fills out detail
    // fields for the LLM's structured handling.
    let (owner, repo, pr_number) = match parse_pr_url(&pr_url) {
        Some(parsed) => parsed,
        None => {
            warn!(
                task_id = task.id.as_str(),
                pr_url = pr_url.as_str(),
                "dispatch_task_has_open_pr: could not parse pr_url; rejecting without enrichment"
            );
            return Some(build_open_pr_rejection(
                &task.id, &pr_url, None, None, None, None,
            ));
        }
    };

    let summary = match github_token {
        Some(token) => match fetch_pr_summary(token, &owner, &repo, pr_number).await {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(
                    task_id = task.id.as_str(),
                    pr_url = pr_url.as_str(),
                    error = %e,
                    "dispatch_task_has_open_pr: PR API enrichment failed; rejecting without enrichment"
                );
                None
            }
        },
        None => {
            warn!(
                task_id = task.id.as_str(),
                "dispatch_task_has_open_pr: no GitHub token; rejecting without enrichment"
            );
            None
        }
    };

    let (pr_state, latest_verdict, merge_state) = match summary {
        Some(s) => (s.state, s.latest_verdict, s.merge_state),
        None => (None, None, None),
    };

    Some(build_open_pr_rejection(
        &task.id,
        &pr_url,
        Some(pr_number),
        pr_state,
        latest_verdict,
        merge_state,
    ))
}

/// Build the structured `dispatch_task_has_open_pr` rejection body (mika#920).
///
/// Optional fields are omitted when `None`; the `recovery` and `reason`
/// strings are stable so the self-dev LLM prompt can pattern-match on them.
fn build_open_pr_rejection(
    task_id: &str,
    pr_url: &str,
    pr_number: Option<u64>,
    pr_state: Option<String>,
    latest_verdict: Option<String>,
    merge_state: Option<String>,
) -> serde_json::Value {
    let pr_label = pr_number
        .map(|n| format!("#{n}"))
        .unwrap_or_else(|| pr_url.to_string());
    let verdict_clause = latest_verdict
        .as_deref()
        .map(|v| format!(" with QA verdict '{v}'"))
        .unwrap_or_default();
    let reason = format!(
        "Task has an open PR ({pr_label}){verdict_clause}. Re-dispatching without \
         iteration_context would re-run the full pipeline against a mostly-complete \
         branch — likely a no-op."
    );
    let recovery = "This task already has an open PR. Options: (a) re-dispatch with \
                    iteration_context to address specific feedback, (b) wait for the \
                    blocker to resolve, (c) check PR status manually. To bypass: pass \
                    iteration_context in the run_claude_pilot call.";

    let mut obj = serde_json::Map::new();
    obj.insert(
        "error".to_string(),
        serde_json::Value::String("dispatch_task_has_open_pr".to_string()),
    );
    obj.insert(
        "task_id".to_string(),
        serde_json::Value::String(task_id.to_string()),
    );
    obj.insert(
        "pr_url".to_string(),
        serde_json::Value::String(pr_url.to_string()),
    );
    if let Some(n) = pr_number {
        obj.insert("pr_number".to_string(), serde_json::Value::from(n));
    }
    if let Some(s) = pr_state {
        obj.insert("pr_state".to_string(), serde_json::Value::String(s));
    }
    if let Some(v) = latest_verdict {
        obj.insert(
            "latest_qa_verdict".to_string(),
            serde_json::Value::String(v),
        );
    }
    if let Some(m) = merge_state {
        obj.insert("merge_state".to_string(), serde_json::Value::String(m));
    }
    obj.insert(
        "recovery".to_string(),
        serde_json::Value::String(recovery.to_string()),
    );
    obj.insert("reason".to_string(), serde_json::Value::String(reason));
    serde_json::Value::Object(obj)
}

/// Check for cycles in the task lineage before enqueuing a deferred dispatch (mika#1058).
///
/// Walks the `parent_task_id` chain (max 4 hops, bounded by `depth ≤ 3` schema CHECK).
/// Extracts `(repo, issue_number, skill)` from each ancestor's metadata and compares
/// against the proposed dispatch. Returns `Ok(())` if safe, `Err(message)` if cycle detected.
///
/// Fail-open: if metadata extraction fails for an ancestor, that ancestor is skipped.
/// The `depth ≤ 3` schema CHECK is the structural backstop.
async fn check_lineage_cycle(
    db: &AsyncDatabase,
    parent_task_id: &str,
    proposed_input: &serde_json::Value,
) -> Result<(), String> {
    let proposed_skill = proposed_input.get("skill").and_then(|v| v.as_str());
    let proposed_prompt = proposed_input.get("prompt").and_then(|v| v.as_str());
    let (proposed_repo, proposed_issue) = parse_repo_issue(proposed_prompt);

    // If we can't extract what we're proposing, we can't detect a cycle — fail-open.
    if proposed_skill.is_none() || proposed_repo.is_none() || proposed_issue.is_none() {
        return Ok(());
    }

    let mut current_id = parent_task_id.to_string();
    for _depth in 0..4 {
        let task = match db.get_task_unscoped(&current_id).await {
            Ok(Some(t)) => t,
            _ => break, // task not found or DB error → stop walking (fail-open)
        };

        // Extract (repo, issue, skill) from this ancestor
        if let Some((ancestor_repo, ancestor_issue, ancestor_skill)) = extract_dispatch_tuple(&task)
            && proposed_skill == Some(ancestor_skill.as_str())
            && proposed_repo == Some(ancestor_repo.as_str())
            && proposed_issue == Some(ancestor_issue)
        {
            return Err(format!(
                "Cycle detected: ancestor task {} has same dispatch tuple \
                 ({}, #{}, skill={}). Refusing to enqueue.",
                task.id, ancestor_repo, ancestor_issue, ancestor_skill
            ));
        }

        // Walk up
        match task.parent_task_id {
            Some(pid) => current_id = pid,
            None => break,
        }
    }
    Ok(())
}

/// Parse "repo#number" format from a prompt string.
///
/// Handles formats like "mika#159", "mika-skills#42", etc. Returns
/// `(Some("mika"), Some(159))` on success, `(None, None)` on failure.
fn parse_repo_issue(prompt: Option<&str>) -> (Option<&str>, Option<i64>) {
    let prompt = match prompt {
        Some(p) => p,
        None => return (None, None),
    };

    // Search for the "repo#number" pattern anywhere in the prompt.
    // The repo name is alphanumeric with hyphens, followed by # and digits.
    for word in prompt.split_whitespace() {
        if let Some((repo, num_str)) = word.split_once('#')
            && !repo.is_empty()
            && repo
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            && let Ok(num) = num_str.parse::<i64>()
        {
            return (Some(repo), Some(num));
        }
    }
    (None, None)
}

/// Extract `(repo, issue_number, skill)` tuple from a task's metadata/action_config.
///
/// Tries multiple extraction strategies in order:
/// 1. `action_config.original_call` (deferred callbacks store the full tool input)
/// 2. Task metadata fields (manual tasks from `create_task`)
/// 3. `reference_url` parsing (GitHub URL → repo#number) + `source` as skill hint
fn extract_dispatch_tuple(task: &crate::db::Task) -> Option<(String, i64, String)> {
    // Strategy 1: action_config.original_call (deferred callbacks)
    if let Ok(config) = serde_json::from_str::<serde_json::Value>(&task.action_config)
        && let Some(original_call) = config.get("original_call")
    {
        let skill = original_call
            .get("skill")
            .and_then(|v| v.as_str())
            .map(String::from);
        let prompt = original_call.get("prompt").and_then(|v| v.as_str());
        let (repo, issue) = parse_repo_issue(prompt);
        if let (Some(skill), Some(repo), Some(issue)) = (skill, repo, issue) {
            return Some((repo.to_string(), issue, skill));
        }
    }

    // Strategy 2: Parse from task label (long-running dispatch labels encode the skill)
    // and reference_url (GitHub issue URL)
    if let Some(ref_url) = &task.reference_url {
        // Parse GitHub URL: https://github.com/owner/repo/issues/123
        let parts: Vec<&str> = ref_url.rsplitn(3, '/').collect();
        if parts.len() >= 3
            && let Ok(issue_num) = parts[0].parse::<i64>()
        {
            // Extract repo name from URL path
            let repo = parts[2].rsplit('/').next().unwrap_or(parts[2]).to_string();

            // Use source as skill hint, or parse from label
            let skill = task.source.as_deref().unwrap_or(&task.label).to_string();

            return Some((repo, issue_num, skill));
        }
    }

    None
}

/// Maximum number of pending deferred-dispatch callbacks per agent (mika#1011).
/// Prevents unbounded queue growth from buggy or malicious dispatch loops.
const MAX_PENDING_DEFERRED_CALLBACKS: i64 = 10;

/// Repair budget per parent task (mika#2045).
///
/// A deferred wrapper consumed without dispatching leaves its parent orphaned.
/// Re-arming replaces the wrapper instead of throwing the work away, but a
/// parent whose turns never dispatch must still terminate: after this many
/// repairs the reaper expires the task and frees its slot in
/// `idx_tasks_manual_active_ref_url` so the `ready` sweep can create a fresh one.
///
/// The counter is shared by both repair paths — the inline re-arm at wrapper
/// consumption and the reaper's — so the budget bounds the total, not each path.
pub(crate) const MAX_STUCK_REARMS: i64 = 2;

/// Register a deferred-dispatch callback when `global_dispatch_active` fires (mika#1011).
///
/// Creates a `pending` callback task with `label = "long_running:run_claude_pilot:deferred"`
/// linked to the requesting parent task. When the blocking dispatch completes, the
/// dispatcher promotes this to `in_progress` and fires a `SilentTrigger::DeferredDispatch`
/// turn. Returns `true` if registered, `false` if cap exceeded or DB error (fail-open).
///
/// # Precondition (security-load-bearing, mika#1205)
///
/// All callers MUST be downstream of the `unauthorized_webhook_dispatch` guard
/// (`validate_dispatch_readiness` guard 0). The deferred-callback child row
/// created by this function is later read by `execute_long_running` as proof
/// that a prior turn was authorized — that read uses the row's existence to
/// short-circuit duplicate-retry rejection with an idempotent "deferred" success.
///
/// Current call sites (both verified downstream of guard 0):
/// - Callback-turn entry path in `execute_skill_tool` — downstream of
///   `check_lineage_cycle` on a turn already gated by callback semantics.
/// - `validate_dispatch_readiness` `global_dispatch_active` branch — downstream
///   of guards 0, 1, 2.
///
/// Adding a new call site that does NOT pass guard 0 first would let
/// unauthorized dispatches forge authorization. If you add one, document the
/// guard-0 equivalence at the call site and update this comment.
pub(crate) async fn register_deferred_callback(
    db: &AsyncDatabase,
    task_id: &str,
    input: &serde_json::Value,
) -> bool {
    // Flood-cap check: reject without insert if at capacity
    match db.count_pending_deferred_callbacks().await {
        Ok(count) if count >= MAX_PENDING_DEFERRED_CALLBACKS => {
            warn!(
                task_id,
                pending_count = count,
                cap = MAX_PENDING_DEFERRED_CALLBACKS,
                "deferred_dispatch_cap_exceeded — not registering deferred callback"
            );
            return false;
        }
        Err(e) => {
            warn!(task_id, error = %e, "failed to count deferred callbacks — skipping registration");
            return false;
        }
        _ => {}
    }

    // Encode the original dispatch arguments so the deferred turn can replay them.
    // Inject the `__internal_deferred_dispatch` sentinel into the saved
    // original_call so that when the deferred turn replays this dispatch, the
    // `dispatch_task_has_open_pr` guard (mika#920) bypasses on F3 condition.
    // Without the sentinel the deferred replay would livelock against the
    // open-PR guard.
    let mut original_call = input.clone();
    if let Some(obj) = original_call.as_object_mut() {
        obj.insert(
            INTERNAL_DEFERRED_DISPATCH_FIELD.to_string(),
            serde_json::Value::Bool(true),
        );
    }
    let action_config = serde_json::json!({
        "trigger_kind": "deferred_dispatch",
        "original_call": original_call,
    })
    .to_string();

    // Derive dispatch_class from the original call's skill parameter so
    // the deferred callback occupies the correct slot when it fires (#1001).
    let skill = extract_skill_from_input(input);
    let class = derive_dispatch_class(skill);

    let task = NewTask {
        agent_id: db.agent_id().to_string(),
        team_run_id: None,
        parent_task_id: Some(task_id.to_string()),
        depth: 0,
        label: crate::agent::DEFERRED_DISPATCH_LABEL.to_string(),
        trigger_type: trigger_type::CALLBACK.to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: action_type::RESUME_AGENT.to_string(),
        action_config,
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: Some("deferred_dispatch".to_string()),
        metadata: None,
        r#type: None,
        dispatch_class: Some(class.to_string()),
    };

    match db.create_task(task).await {
        Ok(deferred_id) => {
            info!(
                task_id,
                deferred_id = %deferred_id,
                "deferred_dispatch_registered — pending callback created for auto-retry"
            );
            true
        }
        Err(e) => {
            warn!(task_id, error = %e, "failed to register deferred callback — LLM fallback only");
            false
        }
    }
}

/// Result of a repair attempt (mika#2045).
///
/// `NotNow` and `Unrepairable` are both refusals, and collapsing them into one
/// boolean is the bug this enum exists to prevent: the reaper expires a task it
/// cannot repair, and a full deferred-callback queue is not that. It is a
/// transient condition that clears on its own, so a task refused for capacity
/// must be left alone and retried, not destroyed with repair budget still on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RearmOutcome {
    /// A replacement wrapper now exists.
    Rearmed,
    /// Refused for a condition that clears by itself — try again next tick.
    NotNow,
    /// Refused for good: the repair budget is spent, or the dispatch cannot be
    /// reconstructed. Only this warrants expiring the task.
    Unrepairable,
}

/// Re-arm a parent whose deferred wrapper was consumed without dispatching
/// (mika#2045).
///
/// `promote_next_deferred_callback` sets the wrapper `completed`, so promotion
/// is destructive: the wrapper leaves the `pending` queue and nothing brings it
/// back. When the promoted turn produces no real dispatch — it errored, or it
/// ran and called no tool — the parent is left `pending` with nothing
/// representing it, and the partial unique index forbids a replacement.
///
/// This inserts a fresh `pending` wrapper and returns. It deliberately does NOT
/// promote anything inline. That is not timidity about mika#1124's anti-cascade
/// guard, it is the same discipline: the cascade that guard closed promoted the
/// *next* wrapper — another task's — N times in one call stack. Re-arming
/// inserts one row for *this* parent and hands the promotion decision back to
/// `promote_pending_deferred_if_idle`, which checks the class slot first.
///
/// The caller must distinguish the two ways a repair can be refused, because
/// only one of them justifies destroying the task. See [`RearmOutcome`].
pub(crate) async fn rearm_deferred_callback(
    db: &AsyncDatabase,
    parent_task_id: &str,
    action_config: &str,
    dispatch_class: &str,
    cause: &str,
) -> RearmOutcome {
    // Guard: if the parent already has an active non-deferred callback, the
    // turn did dispatch and there is nothing to repair. Fail-closed on a query
    // error — a spurious re-arm would double-dispatch, and the reaper is the
    // backstop either way.
    match db
        .has_non_deferred_active_callback_child(parent_task_id)
        .await
    {
        Ok(false) => {}
        Ok(true) => return RearmOutcome::NotNow, // The turn dispatched — healthy.
        Err(e) => {
            warn!(
                parent_task_id,
                error = %e,
                "failed to check for non-deferred callback children — not re-arming"
            );
            return RearmOutcome::NotNow;
        }
    }

    match db.get_stuck_rearm_count(parent_task_id).await {
        Ok(count) if count >= MAX_STUCK_REARMS => {
            warn!(
                event = "deferred_dispatch_rearm_budget_exhausted",
                parent_task_id,
                rearm_count = count,
                budget = MAX_STUCK_REARMS,
                cause,
                "repair budget exhausted — leaving the task for the reaper to expire"
            );
            return RearmOutcome::Unrepairable;
        }
        Err(e) => {
            warn!(parent_task_id, error = %e, "failed to read stuck_rearm_count — not re-arming");
            return RearmOutcome::NotNow;
        }
        _ => {}
    }

    match db.count_pending_deferred_callbacks().await {
        Ok(count) if count >= MAX_PENDING_DEFERRED_CALLBACKS => {
            warn!(
                parent_task_id,
                pending_count = count,
                cap = MAX_PENDING_DEFERRED_CALLBACKS,
                "deferred_dispatch_cap_exceeded — not re-arming yet"
            );
            return RearmOutcome::NotNow;
        }
        Err(e) => {
            warn!(parent_task_id, error = %e, "failed to count deferred callbacks — not re-arming");
            return RearmOutcome::NotNow;
        }
        _ => {}
    }

    let task = NewTask {
        agent_id: db.agent_id().to_string(),
        team_run_id: None,
        parent_task_id: Some(parent_task_id.to_string()),
        depth: 0,
        label: crate::agent::DEFERRED_DISPATCH_LABEL.to_string(),
        trigger_type: trigger_type::CALLBACK.to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: action_type::RESUME_AGENT.to_string(),
        action_config: action_config.to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: Some("deferred_dispatch_rearm".to_string()),
        metadata: None,
        r#type: None,
        dispatch_class: Some(dispatch_class.to_string()),
    };

    let rearmed_id = match db.create_task(task).await {
        Ok(id) => id,
        Err(e) => {
            warn!(parent_task_id, error = %e, "failed to re-arm deferred callback");
            return RearmOutcome::NotNow;
        }
    };

    let rearm_count = db
        .increment_stuck_rearm_count(parent_task_id)
        .await
        .unwrap_or_else(|e| {
            warn!(parent_task_id, error = %e, "failed to increment stuck_rearm_count");
            0
        });

    info!(
        event = "deferred_dispatch_rearmed",
        parent_task_id,
        rearmed_task_id = %rearmed_id,
        rearm_count,
        dispatch_class,
        cause,
        "deferred wrapper consumed without dispatching — replacement registered"
    );

    if let Err(e) = db
        .log_audit_event(
            "system",
            "deferred_dispatch_rearmed",
            &format!("task:{parent_task_id}"),
            None,
            Some("rearmed"),
            Some(&format!(
                "cause:{cause}, rearmed:{rearmed_id}, rearm_count:{rearm_count}"
            )),
            None,
        )
        .await
    {
        warn!(error = %e, "failed to write deferred_dispatch_rearmed audit event");
    }

    RearmOutcome::Rearmed
}

/// Extract `pr_url` from a task's metadata JSON.
///
/// Looks for `claude_pilot.pr_url` (nested) or `pr_url` (top-level).
fn extract_pr_url(metadata: &Option<String>) -> Option<String> {
    let meta = metadata.as_deref()?;
    let parsed: serde_json::Value = serde_json::from_str(meta).ok()?;

    // Try nested claude_pilot.pr_url first
    if let Some(url) = parsed
        .get("claude_pilot")
        .and_then(|cp| cp.get("pr_url"))
        .and_then(|v| v.as_str())
    {
        return Some(url.to_string());
    }

    // Fallback to top-level pr_url
    parsed
        .get("pr_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Execute a long-running exec handler by creating a callback task and spawning
/// the subprocess in the background. Returns immediately with the task ID.
/// Build the callback `NewTask` for a long-running dispatch.
///
/// Shared by `execute_long_running` (the LLM tool-call path) and the engine-side
/// ready-label dispatch (mika#1572) so both paths create structurally identical
/// callback children. The callback/resume contract depends on this exact shape:
/// `trigger_type=callback`, `action_type=resume_agent`, the `action_config.input`
/// mirror of dispatch fields (#958), and the per-class `dispatch_class`. Drift
/// between the two construction sites would re-introduce the callback-shape bug
/// class this consolidation prevents (plan Risk 1).
pub(crate) fn build_callback_task(
    agent_id: String,
    parent_task_id: Option<String>,
    tool_name: &str,
    input: &serde_json::Value,
    timeout_secs: u64,
    session_id: &str,
    trace_id: &str,
) -> NewTask {
    NewTask {
        agent_id,
        team_run_id: None,
        parent_task_id,
        depth: 0,
        label: format!("long_running:{tool_name}"),
        trigger_type: trigger_type::CALLBACK.to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: Some(crate::timestamp::now_plus(chrono::Duration::seconds(
            timeout_secs as i64,
        ))),
        action_type: action_type::RESUME_AGENT.to_string(),
        action_config: {
            // Populate action_config.input with dispatch fields so child tasks
            // are self-describing without a parent join (#958).
            let mut ac_input = serde_json::Map::new();
            for key in &["prompt", "skill", "task_id", "branch"] {
                if let Some(val) = input.get(*key).filter(|v| !v.is_null()) {
                    ac_input.insert((*key).to_string(), val.clone());
                }
            }
            serde_json::json!({ "input": ac_input }).to_string()
        },
        input_context: Some(serde_json::to_string(input).unwrap_or_default()),
        created_by_session: Some(session_id.to_string()),
        created_trace_id: Some(trace_id.to_string()),
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: Some(
            derive_dispatch_class(input.get("skill").and_then(|v| v.as_str())).to_string(),
        ),
    }
}

async fn execute_long_running(
    skill_tool: &ResolvedSkillTool,
    command: &str,
    input: serde_json::Value,
    estimated_duration_secs: Option<u64>,
    ctx: &LongRunningContext,
    github_token: Option<&str>,
) -> ToolOutput {
    // Validate task_id — long-running tasks require tracked tasks.
    // The agent passes the task UUID via the `task_id` input field.
    // See mika#596 / mika-skills#151.
    let task_id = input.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
    let scoped = match crate::tools::AgentScopedTaskId::from_agent_context(&ctx.db, task_id) {
        Ok(s) => s,
        Err(e) => return ToolOutput::error(e.content),
    };
    if let Some(err) = crate::tools::validate_task(&ctx.db, &scoped).await {
        return ToolOutput::error(err);
    }

    // mika#1205: Idempotent ack when a deferred-dispatch child is already pending
    // for this task on this agent.
    //
    // When an LLM-conversation turn retries `run_claude_pilot` on a task that has
    // a pending deferred-callback child (created by a prior turn that hit
    // `global_dispatch_active`), short-circuit with the same `status: "deferred"`
    // success that the callback-turn entry path returns when
    // `register_deferred_callback` succeeds (see top of `execute_skill_tool`).
    // Without this intercept, guard (0) `unauthorized_webhook_dispatch` can fire
    // on the retry's fresh originating_message (issue comment, label change), and
    // the LLM may hallucinate a supervisor → blocked transition (mika#716).
    //
    // Security: the deferred-callback child can only exist if a prior turn passed
    // guard (0) — `register_deferred_callback` is downstream of guard (0) at both
    // call sites (callback-turn entry path and `validate_dispatch_readiness`).
    // The per-agent filter (`child.agent_id == self_agent`) prevents cross-agent
    // authorization leakage in team-task trees where `db::get_child_tasks` returns
    // children with heterogeneous `agent_id`s.
    match ctx.db.get_child_tasks(task_id).await {
        Ok(children) => {
            let self_agent = ctx.db.agent_id();
            let pending_deferred = children.iter().find(|c| {
                c.label == crate::agent::DEFERRED_DISPATCH_LABEL
                    && c.agent_id == self_agent
                    && matches!(c.status.as_str(), "pending" | "in_progress")
            });
            if let Some(child) = pending_deferred {
                info!(
                    task_id,
                    deferred_callback_id = %child.id,
                    "deferred_dispatch_idempotent_ack — prior dispatch already queued (mika#1205)"
                );
                return ToolOutput::success(
                    serde_json::json!({
                        "status": "deferred",
                        "already_deferred": true,
                        "deferred_callback_id": child.id,
                        "deferred_callback_status": child.status,
                        "message": "Your prior dispatch for this task is queued as a \
                                    deferred callback and will fire automatically when \
                                    the dispatch slot is free. Do not retry; do not \
                                    transition the supervisor task. (mika#1205)"
                    })
                    .to_string(),
                );
            }
        }
        Err(e) => {
            // Fail-closed on DB error: skip the intercept and let the existing
            // guards apply. Worst case is reverting to current behavior (the bug
            // we're fixing), not a security regression — guard (0) still rejects
            // unauthorized retries.
            warn!(
                task_id,
                error = %e,
                "deferred_dispatch_intercept_check_failed — falling through to validate_dispatch_readiness"
            );
        }
    }

    // Dispatch-readiness guard (#525): stricter than validate_task() which also
    // allows `blocked` (needed by delegate_task). Long-running dispatch only permits
    // `pending` and `in_progress`. Returns the current status on success to avoid
    // a redundant DB read in the auto-transition below.
    let wi_status = match validate_dispatch_readiness(
        &ctx.db,
        task_id,
        github_token,
        Some(&input),
        ctx.originating_message.as_deref(),
    )
    .await
    {
        Ok(status) => status,
        Err(err) => return ToolOutput::error(err),
    };

    // Per-turn dispatch cap (#583): only one long-running dispatch per agent turn.
    // Check first without incrementing — the actual increment happens right before
    // spawn to avoid leaving the counter stuck at 1 if create_task or path validation fails.
    if ctx.dispatch_count.load(Ordering::Relaxed) > 0 {
        let rejection = serde_json::json!({
            "error": "dispatch_limit_exceeded",
            "task_id": task_id,
            "dispatches_this_turn": ctx.dispatch_count.load(Ordering::Relaxed),
            "reason": "Only one long-running dispatch is permitted per agent turn. \
                       A dispatch has already been launched in this turn. Wait for the \
                       current dispatch to complete via callback before launching another."
        });
        record_dispatch_rejection(&ctx.db, task_id, &rejection.to_string()).await;
        return ToolOutput::error(rejection.to_string());
    }

    let estimated = estimated_duration_secs.unwrap_or(3600);
    let timeout_secs = (estimated * 3).clamp(600, 7_776_000); // 10min..90days

    // Auto-transition pending tasks to in_progress on dispatch (#525).
    // Closes the TOCTOU window where two dispatches to a pending item both pass.
    if wi_status == "pending" {
        if let Err(e) = ctx
            .db
            .update_manual_task_status(task_id, "in_progress")
            .await
        {
            // Non-fatal: the callback child creation provides a secondary guard
            warn!(
                task_id,
                error = %e,
                "failed to auto-transition task to in_progress"
            );
        } else {
            info!(
                task_id,
                "auto-transitioned task from pending to in_progress on dispatch"
            );
        }
    }

    // Link callback task to parent task via parent_task_id for task tree correlation.
    // Same canonical source as the validation above — agent passes via `task_id`.
    let parent_task_id = input
        .get("task_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // Belt-and-suspenders (#955): validate_required_fields is the runtime guard,
    // but assert that required fields survived into the dispatch input as a
    // development-time safety net before the shared builder serializes it.
    debug_assert!(
        {
            let required: Vec<&str> = skill_tool
                .definition
                .input_schema
                .get("required")
                .and_then(serde_json::Value::as_array)
                .map(|arr| arr.iter().filter_map(serde_json::Value::as_str).collect())
                .unwrap_or_default();
            required
                .iter()
                .all(|f| input.get(f).is_some_and(|v| !v.is_null()))
        },
        "dispatch record serialized without required fields — \
         validate_required_fields should have caught this"
    );

    let task = build_callback_task(
        ctx.db.agent_id.clone(),
        parent_task_id,
        &skill_tool.definition.name,
        &input,
        timeout_secs,
        &ctx.session_id,
        &ctx.trace_id,
    );

    let task_id = match ctx.db.create_task(task).await {
        Ok(id) => id,
        Err(e) => {
            return ToolOutput::error(format!("Failed to create callback task: {e}"));
        }
    };

    let cmd_path = skill_tool.skill_dir.join(command);
    if !cmd_path.exists() {
        let _ = ctx
            .db
            .update_task_failed(
                &task_id,
                &format!("handler not found: {}", cmd_path.display()),
            )
            .await;
        return ToolOutput::error(format!(
            "handler command not found: {} (resolved to {})",
            command,
            cmd_path.display()
        ));
    }

    // Inject task metadata into input for the subprocess
    let mut enriched_input = input;
    if let serde_json::Value::Object(ref mut map) = enriched_input {
        map.insert(
            "__mika_task_id".to_string(),
            serde_json::Value::String(task_id.clone()),
        );
        map.insert(
            "__mika_agent".to_string(),
            serde_json::Value::String(ctx.agent_name.clone()),
        );
    }

    // Increment dispatch counter right before spawn — after all validation
    // and task creation succeeded. This ensures the counter stays at 0 if
    // any early error path returns before we actually launch the subprocess.
    ctx.dispatch_count.fetch_add(1, Ordering::Relaxed);

    spawn_long_running_exec(
        cmd_path,
        skill_tool.skill_dir.clone(),
        enriched_input,
        task_id.clone(),
        ctx.db.clone(),
        github_token.map(|s| s.to_string()),
    );

    ToolOutput::success(format!(
        "Task submitted (long-running). ID: {task_id}\n\
         The subprocess is running in the background. \
         Results will be delivered via callback when complete."
    ))
}

/// Spawn a monitored background task for a long-running exec handler.
///
/// The subprocess runs with `kill_on_drop(false)` so it survives if the
/// parent agent task ends. A monitor task records the PID and handles
/// failure (non-zero exit → task marked failed).
pub(crate) fn spawn_long_running_exec(
    cmd_path: PathBuf,
    skill_dir: PathBuf,
    input: serde_json::Value,
    task_id: String,
    db: AsyncDatabase,
    github_token: Option<String>,
) {
    tokio::spawn(async move {
        let mut cmd = tokio::process::Command::new(&cmd_path);
        cmd.current_dir(&skill_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .kill_on_drop(false)
            .process_group(0); // Make child a process group leader (#855)
        // Positive-allowlist env (Phase 1 of dev-pilot containment). Strictly
        // stronger than the legacy MIKA_* scrub: any operator token outside
        // the allowlist (AWS_*, NODE_AUTH_TOKEN, non-MIKA_ OPENAI_/ANTHROPIC_,
        // etc.) cannot leak by inheritance. Companion to Phase 2 (bubblewrap
        // fs+network isolation).
        sandboxed_pilot_env(&mut cmd);
        if let Some(ref token) = github_token {
            cmd.env("GH_TOKEN", token);
        }
        // mika#1705: enable claude-pilot subprocess LLM-transcript capture.
        // Injected AFTER the env sandbox so the non-allowlisted var survives.
        inject_pilot_transcript_env(&mut cmd, &skill_dir, &task_id);
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "failed to spawn long-running exec");
                if let Err(db_err) = db
                    .update_task_failed(&task_id, &format!("spawn failed: {e}"))
                    .await
                {
                    warn!(task_id = %task_id, error = %db_err, "failed to mark spawn-failed task in DB");
                }
                return;
            }
        };

        // Record PID and process start time for watchdog (#959)
        if let Some(pid) = child.id() {
            if let Err(e) = db.set_task_process_id(&task_id, Some(pid as i64)).await {
                warn!(task_id = %task_id, error = %e, "failed to record process ID for long-running task");
            }
            // Store process start time from /proc/<pid>/stat for PID reuse detection.
            // On non-Linux this returns None and is silently skipped.
            if let Some(start_time) =
                crate::task_engine::process_liveness::read_process_start_time(pid)
                && let Err(e) = db
                    .set_task_metadata_field(
                        &task_id,
                        "process_start_time",
                        &start_time.to_string(),
                    )
                    .await
            {
                warn!(
                    task_id = %task_id,
                    error = %e,
                    "failed to store process start time in task metadata"
                );
            }
        }

        // Write input JSON to stdin
        if let Some(mut stdin) = child.stdin.take() {
            let input_bytes = serde_json::to_vec(&input).unwrap_or_default();
            match stdin.write_all(&input_bytes).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "failed to write stdin to long-running exec");
                }
            }
        }

        // Take stderr handle so we can read it (capped) on failure.
        let stderr_handle = child.stderr.take();

        // Wait for the subprocess to finish — only handle failure here.
        // On success, the script itself should call `mika ask --task-id`
        // to deliver results via the callback mechanism.
        let status = match child.wait().await {
            Ok(s) => s,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "failed to wait on long-running exec");
                match db
                    .update_task_failed(&task_id, &format!("wait failed: {e}"))
                    .await
                {
                    Ok(true) => {
                        warn!(task_id = %task_id, error = %e, "long-running exec wait failed")
                    }
                    Ok(false) => {
                        info!(task_id = %task_id, "long-running exec wait failed but task already in terminal state")
                    }
                    Err(db_err) => {
                        warn!(task_id = %task_id, error = %db_err, "failed to mark wait-failed task in DB")
                    }
                }
                return;
            }
        };

        if !status.success() {
            let mut stderr_text = String::new();
            if let Some(mut stderr) = stderr_handle {
                use tokio::io::AsyncReadExt;
                let mut buf = Vec::with_capacity(MAX_OUTPUT_LEN);
                AsyncReadExt::take(&mut stderr, MAX_OUTPUT_LEN as u64)
                    .read_to_end(&mut buf)
                    .await
                    .ok();
                stderr_text = String::from_utf8_lossy(&buf).to_string();
            }
            let code_display = match status.code() {
                Some(code) => format!("Exit code: {code}"),
                None => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        match status.signal() {
                            Some(sig) => format!("Killed by signal: {sig}"),
                            None => "Exit code: unknown".to_string(),
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        "Exit code: unknown".to_string()
                    }
                }
            };
            let err_msg = format!("Process {code_display}: {}", truncate_output(&stderr_text));
            match db.update_task_failed(&task_id, &err_msg).await {
                Ok(true) => warn!(task_id = %task_id, %code_display, "long-running exec failed"),
                Ok(false) => {
                    info!(task_id = %task_id, %code_display, "long-running exec exited but task already in terminal state")
                }
                Err(db_err) => {
                    warn!(task_id = %task_id, error = %db_err, "failed to mark long-running exec failure in DB")
                }
            }
        }
        // If success, the script called `mika ask --task-id` which completes the task.
        // If the script didn't call it, the task will eventually expire via timeout_at.
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::ToolHandler;
    use mika_common::claude::ToolDefinition;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn sandbox_env_allows_core_vars() {
        for key in [
            "PATH", "HOME", "USER", "SHELL", "LANG", "LC_ALL", "TERM", "TMPDIR",
        ] {
            assert!(
                is_sandbox_env_allowed(key),
                "core var {key} must be allowed"
            );
        }
    }

    #[test]
    fn sandbox_env_allows_prefix_families() {
        for key in [
            "LC_MESSAGES",
            "XDG_CONFIG_HOME",
            "XDG_RUNTIME_DIR",
            "NVM_DIR",
            "CARGO_HOME",
            "RUSTUP_HOME",
        ] {
            assert!(
                is_sandbox_env_allowed(key),
                "prefix-family var {key} must be allowed"
            );
        }
    }

    #[test]
    fn sandbox_env_denies_secret_vars() {
        for key in [
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "NODE_AUTH_TOKEN",
            "NPM_TOKEN",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GH_TOKEN",
            "MIKA_ANTHROPIC_API_KEY",
            "MIKA_INTERNAL_TOKEN",
            "MIKA_GITHUB_APP_PRIVATE_KEY",
        ] {
            assert!(
                !is_sandbox_env_allowed(key),
                "secret-shaped var {key} must NOT be allowed"
            );
        }
    }

    #[test]
    fn sandbox_env_denies_mika_prefixed_vars_even_if_core_listed() {
        // Defense against a future refactor accidentally adding a MIKA_ key
        // to the core allowlist: the MIKA_ prefix check runs first.
        assert!(!is_sandbox_env_allowed("MIKA_PATH"));
        assert!(!is_sandbox_env_allowed("MIKA_"));
    }

    /// Write a script file and make it executable, with fsync to avoid races.
    fn write_script(path: &std::path::Path, content: &str) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(content.as_bytes()).unwrap();
        let file = writer.into_inner().unwrap();
        file.sync_all().unwrap();
        drop(file); // Explicitly close before chmod
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn make_exec_tool(skill_dir: &std::path::Path, command: &str) -> ResolvedSkillTool {
        ResolvedSkillTool {
            definition: ToolDefinition {
                name: "test_tool".to_string(),
                description: "Test tool".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Exec {
                command: command.to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: skill_dir.to_path_buf(),
        }
    }

    // --- validate_required_fields tests (#955) ---

    #[test]
    fn test_validate_required_fields_missing_field_returns_error() {
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "run_claude_pilot".to_string(),
                description: "Dispatch claude-pilot".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["skill", "prompt", "task_id"],
                    "properties": {
                        "skill": {
                            "type": "string",
                            "enum": ["dev-pilot"]
                        },
                        "prompt": { "type": "string" },
                        "task_id": { "type": "string" }
                    }
                }),
            },
            handler: ToolHandler::Exec {
                command: "handler.sh".to_string(),
                long_running: true,
                estimated_duration_secs: Some(3600),
            },
            skill_dir: PathBuf::from("/tmp"),
        };

        // Missing `skill` field entirely
        let input = serde_json::json!({"prompt": "mika#928", "task_id": "abc-123"});
        let result = validate_required_fields(&tool, &input);
        assert!(
            result.is_some(),
            "expected error for missing required field"
        );
        let output = result.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("missing_required_field"));
        assert!(output.content.contains("skill"));
        assert!(output.content.contains("dev-pilot"));
    }

    #[test]
    fn test_validate_required_fields_null_field_returns_error() {
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "run_claude_pilot".to_string(),
                description: "Dispatch claude-pilot".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["skill"],
                    "properties": {
                        "skill": { "type": "string" }
                    }
                }),
            },
            handler: ToolHandler::Exec {
                command: "handler.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: PathBuf::from("/tmp"),
        };

        // `skill` present but null
        let input = serde_json::json!({"skill": null});
        let result = validate_required_fields(&tool, &input);
        assert!(result.is_some(), "expected error for null required field");
        let output = result.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("missing_required_field"));
    }

    #[test]
    fn test_validate_required_fields_all_present_passes() {
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "run_claude_pilot".to_string(),
                description: "Dispatch claude-pilot".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["skill", "prompt"],
                    "properties": {
                        "skill": { "type": "string" },
                        "prompt": { "type": "string" }
                    }
                }),
            },
            handler: ToolHandler::Exec {
                command: "handler.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: PathBuf::from("/tmp"),
        };

        let input = serde_json::json!({"skill": "dev-pilot", "prompt": "mika#928"});
        let result = validate_required_fields(&tool, &input);
        assert!(
            result.is_none(),
            "expected no error when all required fields present"
        );
    }

    #[test]
    fn test_validate_required_fields_no_required_key_passes() {
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "test_tool".to_string(),
                description: "Test tool".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Exec {
                command: "handler.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: PathBuf::from("/tmp"),
        };

        let input = serde_json::json!({"anything": "goes"});
        let result = validate_required_fields(&tool, &input);
        assert!(result.is_none(), "no required fields = no validation error");
    }

    #[test]
    fn test_validate_required_fields_malformed_schema_returns_error() {
        // `required` is a string, not an array — malformed.
        // Post-#984: this must return an error, not silently pass.
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "bad_tool".to_string(),
                description: "Tool with bad schema".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": "skill"
                }),
            },
            handler: ToolHandler::Exec {
                command: "handler.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: PathBuf::from("/tmp"),
        };

        let input = serde_json::json!({});
        let result = validate_required_fields(&tool, &input);
        assert!(
            result.is_some(),
            "malformed schema must return an error, not silently pass"
        );
        let output = result.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("malformed_required_schema"));
    }

    #[test]
    fn test_validate_required_fields_malformed_schema_null_returns_error() {
        // `required` is null — malformed
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "bad_tool".to_string(),
                description: "Tool with bad schema".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": null
                }),
            },
            handler: ToolHandler::Exec {
                command: "handler.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: PathBuf::from("/tmp"),
        };

        let input = serde_json::json!({});
        let result = validate_required_fields(&tool, &input);
        // null is not an array — should reject
        assert!(
            result.is_some(),
            "null 'required' must return an error, not silently pass"
        );
        let output = result.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("malformed_required_schema"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_execute_skill_tool_rejects_missing_required_field() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("handler.sh"),
            "#!/bin/sh\necho 'should not run'",
        );

        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "run_claude_pilot".to_string(),
                description: "Dispatch claude-pilot".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["skill", "prompt", "task_id"],
                    "properties": {
                        "skill": {
                            "type": "string",
                            "enum": ["dev-pilot"]
                        },
                        "prompt": { "type": "string" },
                        "task_id": { "type": "string" }
                    }
                }),
            },
            handler: ToolHandler::Exec {
                command: "handler.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: tmp.path().to_path_buf(),
        };

        // Call with missing `skill` — should get rejected before the handler runs
        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"prompt": "mika#928", "task_id": "abc-123"}),
            30,
            None,
            None,
            None,
            None,
        )
        .await;

        assert!(output.is_error, "expected error: {}", output.content);
        assert!(
            output.content.contains("missing_required_field"),
            "expected structured error, got: {}",
            output.content
        );
        assert!(
            output.content.contains("skill"),
            "error should name the missing field"
        );
        // The handler should NOT have run
        assert!(
            !output.content.contains("should not run"),
            "handler should not execute when required field is missing"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_success() {
        let tmp = tempfile::tempdir().unwrap();
        let handler_dir = tmp.path().join("handlers");
        fs::create_dir_all(&handler_dir).unwrap();
        write_script(
            &handler_dir.join("handler.sh"),
            "#!/bin/sh\necho 'hello from handler'",
        );

        let tool = make_exec_tool(tmp.path(), "handlers/handler.sh");
        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"query": "test"}),
            30,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("hello from handler"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_nonzero_exit_returns_output() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("fail.sh"),
            "#!/bin/sh\necho 'error msg' >&2\nexit 1",
        );

        let tool = make_exec_tool(tmp.path(), "fail.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;
        // Non-zero exit is NOT a tool error — the process ran to completion
        assert!(!output.is_error, "non-zero exit should not be is_error");
        assert!(
            output.content.contains("Exit code: 1"),
            "should contain exit code, got: {}",
            output.content
        );
        assert!(
            output.content.contains("error msg"),
            "should contain stderr output, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_nonzero_exit_with_stdout() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("status.sh"),
            "#!/bin/sh\necho 'CRITICAL: disk usage 95%'\nexit 2",
        );

        let tool = make_exec_tool(tmp.path(), "status.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;
        assert!(
            !output.is_error,
            "non-zero exit should not be is_error, got: {}",
            output.content
        );
        assert!(
            output.content.contains("Exit code: 2"),
            "should contain exit code 2, got: {}",
            output.content
        );
        assert!(
            output.content.contains("CRITICAL: disk usage 95%"),
            "should contain stdout, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_nonzero_exit_empty_output() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("silent_fail.sh"), "#!/bin/sh\nexit 3");

        let tool = make_exec_tool(tmp.path(), "silent_fail.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;
        assert!(!output.is_error, "non-zero exit should not be is_error");
        assert!(
            output.content.contains("Exit code: 3"),
            "should contain exit code 3, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_nonzero_exit_via_run_sh() {
        // Regression test for the double problem: run.sh merges stderr into stdout
        // via 2>&1, so on non-zero exit the executor must read stdout (not stderr).
        let (_tmp, tool) = setup_shell_exec_handler();
        let input = serde_json::json!({"command": "echo 'health check output' && exit 2"});
        let output = execute_skill_tool(&tool, input, 30, None, None, None, None).await;
        assert!(
            !output.is_error,
            "non-zero exit via run.sh should not be is_error, got: {}",
            output.content
        );
        assert!(
            output.content.contains("Exit code: 2"),
            "should contain exit code 2, got: {}",
            output.content
        );
        assert!(
            output.content.contains("health check output"),
            "should contain stdout from run.sh, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_exit_zero_unchanged() {
        // Exit 0 should NOT have an exit code prefix
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("ok.sh"), "#!/bin/sh\necho 'all good'");

        let tool = make_exec_tool(tmp.path(), "ok.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;
        assert!(!output.is_error);
        assert!(
            !output.content.contains("Exit code:"),
            "exit 0 should not have exit code prefix, got: {}",
            output.content
        );
        assert!(output.content.contains("all good"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_nonzero_exit_stdout_and_stderr() {
        // When stdout and stderr have different content, both should appear
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("both.sh"),
            "#!/bin/sh\necho 'stdout line'\necho 'stderr line' >&2\nexit 1",
        );

        let tool = make_exec_tool(tmp.path(), "both.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;
        assert!(!output.is_error);
        assert!(output.content.contains("Exit code: 1"));
        assert!(
            output.content.contains("stdout line"),
            "should contain stdout, got: {}",
            output.content
        );
        assert!(
            output.content.contains("stderr line"),
            "should contain stderr, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_exec_handler_missing_command() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = make_exec_tool(tmp.path(), "nonexistent.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;
        assert!(output.is_error);
        assert!(output.content.contains("not found"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("slow.sh"),
            "#!/bin/sh\nsleep 60\necho done",
        );

        let tool = make_exec_tool(tmp.path(), "slow.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 2, None, None, None, None).await;
        assert!(
            output.is_error,
            "expected timeout error, got: {}",
            output.content
        );
        assert!(output.content.contains("timed out"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_reads_stdin() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("echo_input.sh"), "#!/bin/sh\ncat");

        let tool = make_exec_tool(tmp.path(), "echo_input.sh");
        let input = serde_json::json!({"query": "hello world"});
        let output = execute_skill_tool(&tool, input.clone(), 30, None, None, None, None).await;
        assert!(!output.is_error);
        // The output should contain the JSON input
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed, input);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_command_with_quotes() {
        let (_tmp, tool) = setup_shell_exec_handler();
        let input = serde_json::json!({"command": "echo \"hello world\""});
        let output = execute_skill_tool(&tool, input, 30, None, None, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("hello world"),
            "expected 'hello world' in output, got: {}",
            output.content
        );
    }

    /// Helper to create a temp dir with the real shell-exec handler script.
    fn setup_shell_exec_handler() -> (tempfile::TempDir, ResolvedSkillTool) {
        let tmp = tempfile::tempdir().unwrap();
        let handler_dir = tmp.path().join("handlers");
        fs::create_dir_all(&handler_dir).unwrap();
        write_script(
            &handler_dir.join("run.sh"),
            include_str!("../../templates/skills/shell-exec/handlers/run.sh"),
        );
        let tool = make_exec_tool(tmp.path(), "handlers/run.sh");
        (tmp, tool)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_css_hash_chars() {
        let (_tmp, tool) = setup_shell_exec_handler();
        // CSS selectors and hex colors contain # which must survive JSON → jq → eval
        let input = serde_json::json!({"command": "echo '#custom-relay { color: #a6e3a1; }'"});
        let output = execute_skill_tool(&tool, input, 30, None, None, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("#custom-relay"),
            "expected CSS selector in output, got: {}",
            output.content
        );
        assert!(
            output.content.contains("#a6e3a1"),
            "expected hex color in output, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_heredoc_multiline() {
        let (_tmp, tool) = setup_shell_exec_handler();
        // Multi-line command with heredoc: \n in JSON must become real newlines
        let input = serde_json::json!({
            "command": "cat << 'EOF'\n#selector { color: #fff; }\nEOF"
        });
        let output = execute_skill_tool(&tool, input, 30, None, None, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("#selector"),
            "expected CSS selector from heredoc, got: {}",
            output.content
        );
        assert!(
            output.content.contains("#fff"),
            "expected hex color from heredoc, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_sed_with_hash() {
        let (tmp, tool) = setup_shell_exec_handler();
        // Create a temp CSS file, then use sed to replace a selector with #
        let css_file = tmp.path().join("test.css");
        fs::write(&css_file, "#old-selector { color: red; }\n").unwrap();
        let cmd = format!(
            "sed 's/#old-selector/#new-selector/' '{}' && echo done",
            css_file.display()
        );
        let input = serde_json::json!({"command": cmd});
        let output = execute_skill_tool(&tool, input, 30, None, None, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("#new-selector"),
            "expected sed replacement with # in output, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_backslash_in_printf() {
        let (_tmp, tool) = setup_shell_exec_handler();
        // printf with \n format specifiers: backslashes must survive JSON → jq → eval → printf
        let input = serde_json::json!({"command": "printf 'line1\\nline2\\n'"});
        let output = execute_skill_tool(&tool, input, 30, None, None, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("line1"),
            "expected line1 in output, got: {}",
            output.content
        );
        assert!(
            output.content.contains("line2"),
            "expected line2 in output, got: {}",
            output.content
        );
    }

    #[test]
    fn test_truncate_output() {
        assert_eq!(truncate_output("short"), "short");

        let long = "x".repeat(MAX_OUTPUT_LEN + 100);
        let truncated = truncate_output(&long);
        assert!(truncated.len() < long.len());
        assert!(truncated.contains("truncated"));
    }

    #[tokio::test]
    async fn test_http_handler_unsupported_method() {
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "test_tool".to_string(),
                description: "Test".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Http {
                url: "http://localhost:9999".to_string(),
                method: "DELETE".to_string(),
            },
            skill_dir: PathBuf::from("/tmp"),
        };
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 5, None, None, None, None).await;
        assert!(output.is_error);
        assert!(output.content.contains("unsupported HTTP method"));
    }

    // -- Envelope protocol tests --

    #[test]
    fn test_try_parse_envelope_valid() {
        let json = r#"{"__mika_v1": {"text": "Screenshot taken.", "images": ["/tmp/shot.png"]}}"#;
        let env = try_parse_envelope(json).unwrap();
        assert_eq!(env.text, "Screenshot taken.");
        assert_eq!(env.images, vec!["/tmp/shot.png"]);
    }

    #[test]
    fn test_try_parse_envelope_no_images() {
        let json = r#"{"__mika_v1": {"text": "Done."}}"#;
        let env = try_parse_envelope(json).unwrap();
        assert_eq!(env.text, "Done.");
        assert!(env.images.is_empty());
    }

    #[test]
    fn test_try_parse_envelope_plain_text() {
        assert!(try_parse_envelope("hello world").is_none());
    }

    #[test]
    fn test_try_parse_envelope_pretty_printed() {
        // jq without -c produces pretty-printed JSON — must still parse
        let json = "{\n  \"__mika_v1\": {\n    \"text\": \"Image file: /tmp/shot.png (image/png)\",\n    \"images\": [\n      \"/tmp/shot.png\"\n    ]\n  }\n}";
        let env = try_parse_envelope(json).unwrap();
        assert_eq!(env.text, "Image file: /tmp/shot.png (image/png)");
        assert_eq!(env.images, vec!["/tmp/shot.png"]);
    }

    #[test]
    fn test_try_parse_envelope_other_json() {
        // JSON without sentinel key — treated as plain text
        assert!(try_parse_envelope(r#"{"result": "ok"}"#).is_none());
    }

    #[test]
    fn test_try_parse_envelope_invalid_json() {
        assert!(try_parse_envelope("{invalid").is_none());
    }

    // -- Magic byte detection tests --

    #[test]
    fn test_detect_image_type_jpeg() {
        assert_eq!(
            detect_image_type(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]),
            Some("image/jpeg")
        );
    }

    #[test]
    fn test_detect_image_type_png() {
        assert_eq!(
            detect_image_type(&[0x89, 0x50, 0x4E, 0x47, 0x0D]),
            Some("image/png")
        );
    }

    #[test]
    fn test_detect_image_type_gif() {
        assert_eq!(
            detect_image_type(&[0x47, 0x49, 0x46, 0x38, 0x39]),
            Some("image/gif")
        );
    }

    #[test]
    fn test_detect_image_type_webp() {
        let mut bytes = vec![0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00];
        bytes.extend_from_slice(&[0x57, 0x45, 0x42, 0x50]);
        assert_eq!(detect_image_type(&bytes), Some("image/webp"));
    }

    #[test]
    fn test_detect_image_type_unknown() {
        assert_eq!(detect_image_type(&[0x00, 0x01, 0x02, 0x03]), None);
    }

    #[test]
    fn test_detect_image_type_too_short() {
        assert_eq!(detect_image_type(&[0xFF, 0xD8]), None);
    }

    // -- Image file validation tests --

    #[tokio::test]
    async fn test_read_and_validate_image_nonexistent() {
        let err = read_and_validate_image("/tmp/nonexistent_abc123.png")
            .await
            .unwrap_err();
        assert!(err.contains("cannot resolve"));
    }

    #[tokio::test]
    async fn test_read_and_validate_image_not_image() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"this is plain text").unwrap();
        let err = read_and_validate_image(tmp.path().to_str().unwrap())
            .await
            .unwrap_err();
        assert!(err.contains("not a supported image type"));
    }

    #[tokio::test]
    async fn test_read_and_validate_image_valid_png() {
        let tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
        // Minimal valid PNG header + IHDR
        let png_bytes = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
            0x00, 0x00, 0x00, 0x0D, // IHDR length
            0x49, 0x48, 0x44, 0x52, // IHDR
        ];
        std::fs::write(tmp.path(), png_bytes).unwrap();
        let img = read_and_validate_image(tmp.path().to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(img.media_type, "image/png");
        assert!(!img.data.is_empty());
    }

    #[tokio::test]
    async fn test_process_envelope_images_respects_max() {
        // Create 7 temp PNG files — only 5 should be processed
        let dir = tempfile::tempdir().unwrap();
        let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let mut paths = Vec::new();
        for i in 0..7 {
            let p = dir.path().join(format!("img{i}.png"));
            std::fs::write(&p, png_header).unwrap();
            paths.push(p.to_str().unwrap().to_string());
        }
        let (images, errors) = process_envelope_images(&paths).await;
        assert_eq!(images.len(), 5);
        assert!(!errors.is_empty());
        assert!(errors.last().unwrap().contains("skipped"));
    }

    // -- Exec handler with envelope integration test --

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_with_image_envelope() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a fake PNG image
        let img_path = tmp.path().join("screenshot.png");
        let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        std::fs::write(&img_path, png_header).unwrap();

        // Script that outputs a Mika envelope
        let script = format!(
            "#!/bin/sh\nprintf '{{\"__mika_v1\":{{\"text\":\"Screenshot taken.\",\"images\":[\"{}\"]}}}}'\n",
            img_path.display()
        );
        let handler_dir = tmp.path().join("handlers");
        fs::create_dir_all(&handler_dir).unwrap();
        write_script(&handler_dir.join("screenshot.sh"), &script);

        let tool = make_exec_tool(tmp.path(), "handlers/screenshot.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("Screenshot taken."));
        assert_eq!(output.images.len(), 1);
        assert_eq!(output.images[0].media_type, "image/png");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_plain_text_backward_compat() {
        // Plain text output should still work as before (no envelope)
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("plain.sh"),
            "#!/bin/sh\necho 'just plain text'",
        );

        let tool = make_exec_tool(tmp.path(), "plain.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;
        assert!(!output.is_error);
        assert!(output.content.contains("just plain text"));
        assert!(output.images.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_strips_tmux_env() {
        let tmp = tempfile::tempdir().unwrap();
        // Script that prints the TMUX env var (empty if stripped)
        write_script(
            &tmp.path().join("check_env.sh"),
            "#!/bin/sh\nprintf 'TMUX=%s TMUX_PANE=%s' \"$TMUX\" \"$TMUX_PANE\"",
        );

        // Set TMUX in the current process environment
        // Safety: we're in a test with controlled env access
        unsafe {
            std::env::set_var("TMUX", "/tmp/tmux-1000/default,12345,0");
            std::env::set_var("TMUX_PANE", "%0");
        }

        let tool = make_exec_tool(tmp.path(), "check_env.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        // Both vars should be empty because env_remove strips them
        assert_eq!(output.content.trim(), "TMUX= TMUX_PANE=");

        // Clean up env
        unsafe {
            std::env::remove_var("TMUX");
            std::env::remove_var("TMUX_PANE");
        }
    }

    // -- Long-running exec tests --

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_long_running_missing_task_id() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("analyze.sh"), "#!/bin/sh\necho done");

        let tool = make_long_running_tool(tmp.path(), "analyze.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let ctx = LongRunningContext {
            db: async_db,
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
            originating_message: None,
        };

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"query": "test"}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(output.is_error);
        assert!(
            output.content.contains("invalid_uuid"),
            "expected UUID validation error, got: {}",
            output.content
        );
    }

    use crate::test_utils::test_helpers::create_test_task;

    fn make_long_running_tool(skill_dir: &std::path::Path, command: &str) -> ResolvedSkillTool {
        ResolvedSkillTool {
            definition: ToolDefinition {
                name: "long_test".to_string(),
                description: "Long-running test tool".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Exec {
                command: command.to_string(),
                long_running: true,
                estimated_duration_secs: Some(60),
            },
            skill_dir: skill_dir.to_path_buf(),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_long_running_creates_callback_task() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("analyze.sh"), "#!/bin/sh\necho done");

        let tool = make_long_running_tool(tmp.path(), "analyze.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_test_task(&async_db).await;
        let ctx = LongRunningContext {
            db: async_db.clone(),
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
            originating_message: None,
        };

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"query": "test", "task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("Task submitted"),
            "expected task submission message, got: {}",
            output.content
        );

        // Verify a callback task was created (2 tasks total: parent task + callback)
        let tasks = async_db
            .get_tasks_by_status(vec!["pending".to_string()])
            .await
            .unwrap();
        // Task is pending, callback task is also pending
        let callback_tasks: Vec<_> = tasks
            .iter()
            .filter(|t| t.trigger_type == "callback")
            .collect();
        assert_eq!(callback_tasks.len(), 1);
        assert_eq!(callback_tasks[0].action_type, "resume_agent");
        assert!(callback_tasks[0].label.starts_with("long_running:"));
        assert!(callback_tasks[0].timeout_at.is_some());
        assert_eq!(
            callback_tasks[0].parent_task_id.as_deref(),
            Some(wi_id.as_str()),
            "callback task should link to parent task via parent_task_id"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_long_running_false_blocks_normally() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("handler.sh"),
            "#!/bin/sh\necho 'sync result'",
        );

        // long_running: false — should execute synchronously even with LongRunningContext
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "sync_test".to_string(),
                description: "Sync test tool".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Exec {
                command: "handler.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: tmp.path().to_path_buf(),
        };

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let ctx = LongRunningContext {
            db: async_db,
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
            originating_message: None,
        };

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("sync result"),
            "expected sync output, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_long_running_failure_marks_task_failed() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("fail.sh"),
            "#!/bin/sh\necho 'error msg' >&2\nexit 1",
        );

        let tool = make_long_running_tool(tmp.path(), "fail.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_test_task(&async_db).await;
        let ctx = LongRunningContext {
            db: async_db.clone(),
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
            originating_message: None,
        };

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        // Should return success immediately (task submitted)
        assert!(!output.is_error, "unexpected error: {}", output.content);

        // Wait briefly for the background monitor to detect the failure
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let tasks = async_db
            .get_tasks_by_status(vec!["failed".to_string()])
            .await
            .unwrap();
        assert_eq!(
            tasks.len(),
            1,
            "expected 1 failed task, got {}",
            tasks.len()
        );
        let result_text = tasks[0].result.as_ref().unwrap();
        assert!(
            result_text.contains("Exit code: 1"),
            "expected 'Exit code: 1' in result, got: {result_text}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_receives_gh_token() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("check_token.sh"),
            "#!/bin/sh\necho \"GH_TOKEN=$GH_TOKEN\"",
        );

        let tool = make_exec_tool(tmp.path(), "check_token.sh");

        // With github_token provided — should appear in child env
        let output = execute_skill_tool(
            &tool,
            serde_json::json!({}),
            30,
            None,
            Some("ghp_test_token_123"),
            None,
            None,
        )
        .await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("GH_TOKEN=ghp_test_token_123"),
            "expected GH_TOKEN to be injected, got: {}",
            output.content
        );

        // Without github_token — GH_TOKEN should be absent (scrubbed)
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            !output.content.contains("GH_TOKEN=ghp_"),
            "expected GH_TOKEN to not contain a token when github_token is None, got: {}",
            output.content
        );
    }

    /// Regression test for #537: a long_running tool with no long_running_ctx
    /// must return an explicit error instead of silently falling through to
    /// the sync exec path (which lacks __mika_task_id injection).
    #[tokio::test]
    async fn test_long_running_tool_without_context_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "run_claude_pilot".to_string(),
                description: "Long-running test tool".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Exec {
                command: "./handlers/run.sh".to_string(),
                long_running: true,
                estimated_duration_secs: Some(300),
            },
            skill_dir: tmp.path().to_path_buf(),
        };

        // Pass None for long_running_ctx — simulates callback turn / silent mode / CLI test
        let output =
            execute_skill_tool(&tool, serde_json::json!({}), 30, None, None, None, None).await;

        assert!(output.is_error, "expected error, got: {}", output.content);
        assert!(
            output.content.contains("run_claude_pilot"),
            "error should name the tool: {}",
            output.content
        );
        assert!(
            output.content.contains("long_running"),
            "error should mention long_running: {}",
            output.content
        );
        assert!(
            output.content.contains("cannot run in the current context"),
            "error should explain the context restriction: {}",
            output.content
        );
    }

    // -- Dispatch-readiness guard tests (#525) --

    /// Helper: create a task and transition it to the given status.
    async fn create_task_with_status(db: &crate::async_db::AsyncDatabase, status: &str) -> String {
        let wi_id = create_test_task(db).await;
        if status != "pending" {
            // pending -> blocked or in_progress are valid transitions;
            // pending -> completed or cancelled are also valid.
            db.update_manual_task_status(&wi_id, status).await.unwrap();
        }
        wi_id
    }

    /// Helper: create a callback child task under a parent task.
    async fn create_callback_child(
        db: &crate::async_db::AsyncDatabase,
        parent_task_id: &str,
        status: &str,
    ) -> String {
        use crate::task_engine::types::{action_type, trigger_type};
        let task = crate::db::NewTask {
            agent_id: db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_task_id.to_string()),
            depth: 0,
            label: "long_running:run_claude_pilot".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let child_id = db.create_task(task).await.unwrap();
        if status != "pending" {
            // Transition the child to the requested status via direct DB update
            db.update_task_status(&child_id, status).await.unwrap();
        }
        child_id
    }

    fn make_lr_ctx(db: crate::async_db::AsyncDatabase) -> LongRunningContext {
        LongRunningContext {
            db,
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
            originating_message: None,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_blocked_task() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "blocked").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_not_dispatchable");
        assert_eq!(parsed["current_status"], "blocked");
        assert_eq!(parsed["task_id"], wi_id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_completed_task() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "completed").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        // Caught by validate_task() (first-pass) — not a JSON error
        assert!(output.is_error);
        assert!(
            output.content.contains("not an active task"),
            "expected validate_task rejection, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_cancelled_task() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "cancelled").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        // Caught by validate_task() (first-pass)
        assert!(output.is_error);
        assert!(
            output.content.contains("not an active task"),
            "expected validate_task rejection, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_nonexistent_task_id() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": "00000000-0000-0000-0000-000000000000"}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(output.is_error);
        assert!(
            output.content.contains("task_not_found"),
            "expected task_not_found error for valid-format-but-nonexistent UUID, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_active_callback_child() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        let child_id = create_callback_child(&async_db, &wi_id, "pending").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_active_dispatch");
        assert_eq!(parsed["active_child_id"], child_id);
        assert_eq!(parsed["task_id"], wi_id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_in_progress_callback_child() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        let _child_id = create_callback_child(&async_db, &wi_id, "in_progress").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_active_dispatch");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_allows_with_only_completed_callback_children() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        // Create completed and failed callback children — should not block
        create_callback_child(&async_db, &wi_id, "completed").await;
        create_callback_child(&async_db, &wi_id, "failed").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        // Should proceed to dispatch — guard must not reject.
        // If is_error, verify it's NOT a dispatch guard rejection.
        if output.is_error {
            assert!(
                !output.content.contains("task_not_dispatchable")
                    && !output.content.contains("task_active_dispatch"),
                "dispatch guard should not reject, got: {}",
                output.content
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_allows_cancelled_callback_children() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        // Cancelled callback child — should not block re-dispatch
        create_callback_child(&async_db, &wi_id, "cancelled").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        if output.is_error {
            assert!(
                !output.content.contains("task_not_dispatchable")
                    && !output.content.contains("task_active_dispatch"),
                "dispatch guard should not reject for cancelled child, got: {}",
                output.content
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_ignores_non_callback_children() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;

        // Create a non-callback child (e.g., resume_agent with manual trigger)
        let non_callback = crate::db::NewTask {
            agent_id: async_db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: Some(wi_id.clone()),
            depth: 0,
            label: "delegate:some-agent".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        async_db.create_task(non_callback).await.unwrap();

        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        // Should proceed — non-callback children don't block
        if output.is_error {
            assert!(
                !output.content.contains("task_not_dispatchable")
                    && !output.content.contains("task_active_dispatch"),
                "dispatch guard should not reject for non-callback children, got: {}",
                output.content
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_mixed_children_one_active_rejects() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        // One completed, one still pending — should block
        create_callback_child(&async_db, &wi_id, "completed").await;
        create_callback_child(&async_db, &wi_id, "pending").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_active_dispatch");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_auto_transitions_pending_to_in_progress() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_test_task(&async_db).await;

        // Verify starts as pending
        let task_before = async_db.get_task(&wi_id).await.unwrap().unwrap();
        assert_eq!(task_before.status, "pending");

        let ctx = make_lr_ctx(async_db.clone());

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(!output.is_error, "unexpected error: {}", output.content);

        // Verify task transitioned to in_progress
        let task_after = async_db.get_task(&wi_id).await.unwrap().unwrap();
        assert_eq!(
            task_after.status, "in_progress",
            "task should auto-transition to in_progress on dispatch"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_no_transition_for_already_in_progress() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        let ctx = make_lr_ctx(async_db.clone());

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(!output.is_error, "unexpected error: {}", output.content);

        // Should still be in_progress (no redundant transition)
        let task = async_db.get_task(&wi_id).await.unwrap().unwrap();
        assert_eq!(task.status, "in_progress");
    }

    // -- Integration test: PR #522 race scenario replay (#525) --

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pr522_replay_active_dispatch_with_pr_url() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        // Create in_progress task with PR URL in metadata
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        let metadata = serde_json::json!({
            "claude_pilot": {
                "pr_url": "https://github.com/senara-solutions/mika/pull/522",
                "branch": "feat/522/some-feature"
            }
        });
        async_db
            .update_task_metadata(&wi_id, &metadata.to_string())
            .await
            .unwrap();

        // Simulate active claude-pilot session (pending callback child)
        create_callback_child(&async_db, &wi_id, "pending").await;

        // Count tasks before attempted dispatch
        let tasks_before = async_db.get_child_tasks(&wi_id).await.unwrap().len();

        let ctx = make_lr_ctx(async_db.clone());

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        // Should be rejected
        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_active_dispatch");
        assert_eq!(
            parsed["pr_url"],
            "https://github.com/senara-solutions/mika/pull/522"
        );

        // Verify no new callback task was created
        let tasks_after = async_db.get_child_tasks(&wi_id).await.unwrap().len();
        assert_eq!(
            tasks_before, tasks_after,
            "no new callback task should be created when dispatch is rejected"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pr522_replay_no_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        create_callback_child(&async_db, &wi_id, "pending").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_active_dispatch");
        assert!(
            parsed["pr_url"].is_null(),
            "pr_url should be null when no metadata"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pr522_replay_retry_after_child_completes() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        let child_id = create_callback_child(&async_db, &wi_id, "pending").await;

        // First attempt: should be rejected
        let ctx = make_lr_ctx(async_db.clone());
        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;
        assert!(output.is_error);

        // Complete the child task
        async_db
            .update_task_status(&child_id, "completed")
            .await
            .unwrap();

        // Retry: should succeed now
        let ctx = make_lr_ctx(async_db.clone());
        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(
            !output.is_error,
            "retry after child completion should succeed, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_double_dispatch_pending_item() {
        // Regression: two sequential dispatches to a pending task.
        // First should succeed (and auto-transition to in_progress + create callback).
        // Second should be rejected by active-child check.
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_test_task(&async_db).await;

        // First dispatch: should succeed
        let ctx = make_lr_ctx(async_db.clone());
        let output1 = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;
        assert!(
            !output1.is_error,
            "first dispatch should succeed: {}",
            output1.content
        );

        // Verify first dispatch created exactly one callback child
        let children = async_db.get_child_tasks(&wi_id).await.unwrap();
        let callback_children: Vec<_> = children
            .iter()
            .filter(|c| c.trigger_type == "callback")
            .collect();
        assert_eq!(
            callback_children.len(),
            1,
            "first dispatch must create exactly one callback child"
        );

        // Second dispatch: should be rejected (active callback child from first dispatch)
        let ctx = make_lr_ctx(async_db.clone());
        let output2 = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;
        assert!(output2.is_error, "second dispatch should be rejected");
        let parsed: serde_json::Value = serde_json::from_str(&output2.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output2.content));
        assert_eq!(parsed["error"], "task_active_dispatch");

        // Verify task is in_progress (auto-transitioned by first dispatch)
        let task = async_db.get_task(&wi_id).await.unwrap().unwrap();
        assert_eq!(task.status, "in_progress");
    }

    // ---- Global dispatch guard tests (#583) ----

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_global_dispatch_guard_rejects_when_other_task_has_active_callback() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        // Create task A with an active callback child
        let wi_a = create_task_with_status(&async_db, "in_progress").await;
        let _callback_a = create_callback_child(&async_db, &wi_a, "pending").await;

        // Create task B — attempting to dispatch on this should be blocked
        let wi_b = create_task_with_status(&async_db, "in_progress").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_b}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "global_dispatch_active");
        assert_eq!(parsed["blocking_task_id"], wi_a);
        assert_eq!(parsed["task_id"], wi_b);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_global_dispatch_guard_allows_when_no_other_active_callbacks() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        // Task A has only completed callbacks
        let wi_a = create_task_with_status(&async_db, "in_progress").await;
        create_callback_child(&async_db, &wi_a, "completed").await;

        // Dispatch on task B should succeed
        let wi_b = create_task_with_status(&async_db, "in_progress").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_b}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        // Should NOT be a global_dispatch_active error
        if output.is_error {
            assert!(
                !output.content.contains("global_dispatch_active"),
                "global dispatch guard should not reject when other callbacks are completed, got: {}",
                output.content
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_global_dispatch_guard_allows_same_task_callback() {
        // The global guard should NOT block dispatch on the same task —
        // that's already handled by the per-task guard.
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi = create_task_with_status(&async_db, "in_progress").await;
        create_callback_child(&async_db, &wi, "pending").await;

        // Check the DB method directly — should return None since the only
        // active callback belongs to the excluded parent
        let result = async_db
            .has_active_callback_tasks_excluding(&wi, "implement")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "should not find active callbacks for the same task"
        );
    }

    // ---- Per-turn dispatch counter tests (#583) ----

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_turn_dispatch_counter_rejects_second_dispatch() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi = create_task_with_status(&async_db, "in_progress").await;
        let ctx = make_lr_ctx(async_db);

        // Simulate that a dispatch already happened this turn by setting the counter
        ctx.dispatch_count.store(1, Ordering::Relaxed);

        // Second dispatch should be rejected by the per-turn counter
        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;
        assert!(output.is_error, "second dispatch should be rejected");
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "dispatch_limit_exceeded");
        // Counter should still be 1 (not incremented on rejection)
        assert_eq!(ctx.dispatch_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_turn_dispatch_counter_resets_with_new_context() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        // First context with counter at 1
        let ctx1 = make_lr_ctx(async_db.clone());
        ctx1.dispatch_count.store(1, Ordering::Relaxed);
        assert_eq!(ctx1.dispatch_count.load(Ordering::Relaxed), 1);

        // New context should start at 0
        let ctx2 = make_lr_ctx(async_db);
        assert_eq!(ctx2.dispatch_count.load(Ordering::Relaxed), 0);
    }

    // ---- DB method tests for has_active_callback_tasks_excluding (#583) ----

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_has_active_callback_excluding_returns_none_when_empty() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let result = async_db
            .has_active_callback_tasks_excluding("nonexistent", "implement")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_has_active_callback_excluding_ignores_terminal_states() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi = create_task_with_status(&async_db, "in_progress").await;
        create_callback_child(&async_db, &wi, "completed").await;
        create_callback_child(&async_db, &wi, "failed").await;
        create_callback_child(&async_db, &wi, "cancelled").await;

        let result = async_db
            .has_active_callback_tasks_excluding("other-task", "implement")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "should not detect terminal-state callbacks as active"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_has_active_callback_excluding_finds_active_for_different_parent() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi = create_task_with_status(&async_db, "in_progress").await;
        let callback_id = create_callback_child(&async_db, &wi, "pending").await;

        let result = async_db
            .has_active_callback_tasks_excluding("different-parent", "implement")
            .await
            .unwrap();
        assert!(result.is_some());
        let (parent_id, found_callback_id, _label) = result.unwrap();
        assert_eq!(parent_id, wi);
        assert_eq!(found_callback_id, callback_id);
    }

    // ---- Per-class dispatch slot split tests (#1001) ----

    /// Helper: create a callback child task with a specific dispatch_class.
    async fn create_callback_child_with_class(
        db: &crate::async_db::AsyncDatabase,
        parent_id: &str,
        status: &str,
        dispatch_class: &str,
    ) -> String {
        use crate::db::NewTask;
        use crate::task_engine::types::{action_type, trigger_type};

        let task = NewTask {
            agent_id: db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.to_string()),
            depth: 0,
            label: format!("long_running:run_claude_pilot:{dispatch_class}"),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some(dispatch_class.to_string()),
        };
        let id = db.create_task(task).await.unwrap();
        if status != "pending" {
            db.update_manual_task_status(&id, status).await.unwrap();
        }
        id
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_class_slot_allows_different_class_concurrent() {
        // An active 'implement' callback should NOT block a 'groom' dispatch
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi1 = create_task_with_status(&async_db, "in_progress").await;
        create_callback_child_with_class(&async_db, &wi1, "pending", "implement").await;

        // Querying for 'groom' class should find no blocking dispatch
        let result = async_db
            .has_active_callback_tasks_excluding("other-task", "groom")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "groom dispatch should not be blocked by active implement dispatch"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_class_slot_blocks_same_class() {
        // An active 'implement' callback SHOULD block another 'implement' dispatch
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi1 = create_task_with_status(&async_db, "in_progress").await;
        let callback_id =
            create_callback_child_with_class(&async_db, &wi1, "pending", "implement").await;

        let result = async_db
            .has_active_callback_tasks_excluding("other-task", "implement")
            .await
            .unwrap();
        assert!(result.is_some(), "same-class dispatch should be blocked");
        let (parent_id, found_id, _label) = result.unwrap();
        assert_eq!(parent_id, wi1);
        assert_eq!(found_id, callback_id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_class_slot_groom_blocks_groom() {
        // An active 'groom' callback should block another 'groom' dispatch
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi1 = create_task_with_status(&async_db, "in_progress").await;
        create_callback_child_with_class(&async_db, &wi1, "pending", "groom").await;

        let result = async_db
            .has_active_callback_tasks_excluding("other-task", "groom")
            .await
            .unwrap();
        assert!(result.is_some(), "groom-vs-groom should be blocked");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pre_v34_null_dispatch_class_treated_as_implement() {
        // Pre-v34 tasks have dispatch_class IS NULL — they should be treated
        // as 'implement' via COALESCE in the SQL query.
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi1 = create_task_with_status(&async_db, "in_progress").await;
        // Create a callback child WITHOUT dispatch_class (simulating pre-v34)
        create_callback_child(&async_db, &wi1, "pending").await;

        // Should block 'implement' queries (NULL → 'implement' via COALESCE)
        let result = async_db
            .has_active_callback_tasks_excluding("other-task", "implement")
            .await
            .unwrap();
        assert!(
            result.is_some(),
            "NULL dispatch_class should be treated as 'implement'"
        );

        // Should NOT block 'groom' queries
        let result = async_db
            .has_active_callback_tasks_excluding("other-task", "groom")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "NULL dispatch_class should not block groom dispatches"
        );
    }

    /// Helper: create a deferred-wrapper callback child (mika#1163 regression coverage).
    /// Mirrors `register_deferred_callback`'s output shape — label suffix `:deferred`,
    /// trigger_type=`callback`, action_type=`resume_agent`, dispatch_class as supplied.
    async fn create_deferred_wrapper_child(
        db: &crate::async_db::AsyncDatabase,
        parent_id: &str,
        dispatch_class: Option<&str>,
    ) -> String {
        use crate::db::NewTask;
        use crate::task_engine::types::{action_type, trigger_type};

        let task = NewTask {
            agent_id: db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.to_string()),
            depth: 0,
            label: crate::agent::DEFERRED_DISPATCH_LABEL.to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("deferred_dispatch".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: dispatch_class.map(str::to_string),
        };
        db.create_task(task).await.unwrap()
    }

    /// mika#1163 — Per-class slot guard MUST NOT block on pending deferred wrappers.
    ///
    /// Reproduces the multi-wrapper deadlock observed 2026-05-17: when two parents
    /// each hold a pending `:deferred` wrapper, the per-class predicate (used by
    /// `validate_dispatch_readiness`) used to see the OTHER parent's wrapper as an
    /// active dispatch and register yet another wrapper. With the fix in place,
    /// neither wrapper blocks the other; only real (non-deferred) callbacks count
    /// as slot-occupying.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_class_slot_does_not_block_on_deferred_wrappers() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        // Two in-progress parents, each with a pending `:deferred` wrapper.
        let parent_a = create_task_with_status(&async_db, "in_progress").await;
        let parent_b = create_task_with_status(&async_db, "in_progress").await;
        create_deferred_wrapper_child(&async_db, &parent_a, Some("implement")).await;
        create_deferred_wrapper_child(&async_db, &parent_b, Some("implement")).await;

        // A's dispatch attempt: must NOT see B's wrapper as an active dispatch.
        let result = async_db
            .has_active_callback_tasks_excluding(&parent_a, "implement")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "Parent A's dispatch must not be blocked by Parent B's pending deferred wrapper \
             (mika#1163 deadlock — wrappers are pending markers, not active dispatches)"
        );

        // Symmetric: B's dispatch attempt must not see A's wrapper either.
        let result = async_db
            .has_active_callback_tasks_excluding(&parent_b, "implement")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "Parent B's dispatch must not be blocked by Parent A's pending deferred wrapper"
        );

        // Add Parent C with a REAL (non-deferred) pending callback. This IS an
        // active dispatch and MUST still be detected — exclusion is narrowly
        // scoped to `:deferred` rows.
        let parent_c = create_task_with_status(&async_db, "in_progress").await;
        let real_c =
            create_callback_child_with_class(&async_db, &parent_c, "pending", "implement").await;

        // Parent A's dispatch attempt now: should be blocked by C's real callback,
        // not by B's wrapper.
        let result = async_db
            .has_active_callback_tasks_excluding(&parent_a, "implement")
            .await
            .unwrap();
        let (blocking_parent, blocking_callback, _label) = result.expect(
            "real pending callback MUST still block — only :deferred wrappers are excluded",
        );
        assert_eq!(blocking_parent, parent_c, "real dispatch is the blocker");
        assert_eq!(blocking_callback, real_c);
    }

    /// mika#1163 — Pre-v34 NULL dispatch_class deferred wrapper must also be
    /// excluded from the slot check. COALESCE+label clauses both apply.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_class_slot_ignores_null_class_deferred_wrapper() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let parent_a = create_task_with_status(&async_db, "in_progress").await;
        // dispatch_class=None → COALESCE → 'implement' for the slot query
        create_deferred_wrapper_child(&async_db, &parent_a, None).await;

        let result = async_db
            .has_active_callback_tasks_excluding("other-task", "implement")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "Pre-v34 NULL-class deferred wrapper must be excluded — the :deferred \
             filter runs alongside the COALESCE class match"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_derive_dispatch_class_values() {
        assert_eq!(derive_dispatch_class(Some("dev-groom")), "groom");
        assert_eq!(derive_dispatch_class(Some("dev-pilot")), "implement");
        assert_eq!(derive_dispatch_class(Some("deploy_mika")), "implement");
        assert_eq!(derive_dispatch_class(None), "implement");
        assert_eq!(derive_dispatch_class(Some("unknown-skill")), "implement");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_task_dispatch_class() {
        // Verify that dispatch_class can be flipped on a task
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi = create_task_with_status(&async_db, "in_progress").await;

        // Initially no dispatch_class
        let task = async_db.get_task(&wi).await.unwrap().unwrap();
        assert!(task.dispatch_class.is_none());

        // Set to 'groom'
        let updated = async_db
            .update_task_dispatch_class(&wi, "groom")
            .await
            .unwrap();
        assert!(updated);
        let task = async_db.get_task(&wi).await.unwrap().unwrap();
        assert_eq!(task.dispatch_class.as_deref(), Some("groom"));

        // Flip to 'implement'
        let updated = async_db
            .update_task_dispatch_class(&wi, "implement")
            .await
            .unwrap();
        assert!(updated);
        let task = async_db.get_task(&wi).await.unwrap().unwrap();
        assert_eq!(task.dispatch_class.as_deref(), Some("implement"));
    }

    // -- Blocked-by guard tests (#713) --

    /// Helper: create a task with `reference_url` set and transition to a given status.
    async fn create_task_with_ref_url(
        db: &crate::async_db::AsyncDatabase,
        status: &str,
        reference_url: Option<&str>,
    ) -> String {
        use crate::db::NewTask;
        use crate::task_engine::types::{action_type, trigger_type};

        let task = NewTask {
            agent_id: db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "test task with ref".to_string(),
            trigger_type: trigger_type::MANUAL.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::NONE.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: reference_url.map(|u| u.to_string()),
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(task).await.unwrap();
        if status != "pending" {
            db.update_manual_task_status(&id, status).await.unwrap();
        }
        id
    }

    #[tokio::test]
    async fn test_blocked_by_guard_skips_when_no_reference_url() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;

        // No reference_url → blocked-by check skipped, dispatch proceeds
        let result =
            validate_dispatch_readiness(&async_db, &wi_id, Some("fake-token"), None, None).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[tokio::test]
    async fn test_blocked_by_guard_skips_when_reference_is_pr() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(
            &async_db,
            "in_progress",
            Some("https://github.com/senara-solutions/mika/pull/100"),
        )
        .await;

        // PR reference → blocked-by check skipped (only issues have blockedBy)
        let result =
            validate_dispatch_readiness(&async_db, &wi_id, Some("fake-token"), None, None).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[tokio::test]
    async fn test_blocked_by_guard_skips_when_no_github_token() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(
            &async_db,
            "in_progress",
            Some("https://github.com/senara-solutions/mika/issues/713"),
        )
        .await;

        // No token → blocked-by check skipped (fail-open), dispatch proceeds
        let result = validate_dispatch_readiness(&async_db, &wi_id, None, None, None).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    // ===================================================================
    // Unauthorized webhook dispatch guard tests (mika#933)
    // ===================================================================

    #[tokio::test]
    async fn test_dispatch_guard_rejects_unauthorized_webhook() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        // Create an in_progress task that would otherwise pass all checks
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;

        let result = validate_dispatch_readiness(
            &async_db,
            &wi_id,
            Some("fake-token"),
            None,
            Some("[GitHub] New comment on senara-solutions/mika#933 (title) by @samidarko"),
        )
        .await;

        assert!(result.is_err(), "expected Err, got: {result:?}");
        let err: serde_json::Value = serde_json::from_str(&result.unwrap_err()).unwrap();
        assert_eq!(err["error"], "unauthorized_webhook_dispatch");
    }

    #[tokio::test]
    async fn test_dispatch_guard_allows_ready_label_webhook() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;

        // Ready-label event should NOT be rejected by the webhook gate
        let result = validate_dispatch_readiness(
            &async_db,
            &wi_id,
            Some("fake-token"),
            None,
            Some("[GitHub] Issue labeled ready on senara-solutions/mika#933 — title"),
        )
        .await;

        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[tokio::test]
    async fn test_dispatch_guard_allows_no_originating_message() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;

        // None originating_message (callback continuation / silent trigger)
        let result =
            validate_dispatch_readiness(&async_db, &wi_id, Some("fake-token"), None, None).await;

        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    // ===================================================================
    // dispatch_task_has_open_pr guard tests (mika#920)
    // ===================================================================

    /// Helper: write a `claude_pilot.pr_url` field into the task's metadata.
    async fn set_task_pr_url(db: &crate::async_db::AsyncDatabase, task_id: &str, pr_url: &str) {
        let metadata = serde_json::json!({
            "claude_pilot": { "pr_url": pr_url }
        })
        .to_string();
        db.update_task_metadata(task_id, &metadata).await.unwrap();
    }

    /// Build a `run_claude_pilot` tool input with the given fields.
    fn pilot_input(skill: &str, prompt: &str, task_id: &str) -> serde_json::Value {
        serde_json::json!({
            "skill": skill,
            "prompt": prompt,
            "task_id": task_id,
        })
    }

    /// Scenario 1: Re-dispatch with open PR and no `iteration_context` → rejection.
    #[tokio::test]
    async fn test_open_pr_guard_rejects_re_dispatch_without_context() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;
        set_task_pr_url(
            &async_db,
            &wi_id,
            "https://github.com/senara-solutions/mika/pull/915",
        )
        .await;

        let input = pilot_input("dev-pilot", "mika#920", &wi_id);
        let result = validate_dispatch_readiness(&async_db, &wi_id, None, Some(&input), None).await;

        let err = result.expect_err("dispatch should be rejected");
        let parsed: serde_json::Value = serde_json::from_str(&err).unwrap();
        assert_eq!(parsed["error"], "dispatch_task_has_open_pr");
        assert_eq!(
            parsed["pr_url"],
            "https://github.com/senara-solutions/mika/pull/915"
        );
        assert_eq!(parsed["pr_number"], 915);
        assert_eq!(parsed["task_id"], wi_id);
        assert!(
            parsed["recovery"]
                .as_str()
                .unwrap()
                .contains("iteration_context")
        );
        assert!(parsed["reason"].as_str().unwrap().contains("open PR"));

        // Rejection JSON is also written to tasks.result for operator visibility (#1108).
        let task = async_db.get_task(&wi_id).await.unwrap().unwrap();
        let stored = task
            .result
            .expect("rejection should be written to tasks.result");
        assert!(stored.contains("dispatch_task_has_open_pr"));
    }

    /// Scenario 2: Re-dispatch with open PR AND `iteration_context` → bypass.
    #[tokio::test]
    async fn test_open_pr_guard_bypasses_with_iteration_context() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;
        set_task_pr_url(
            &async_db,
            &wi_id,
            "https://github.com/senara-solutions/mika/pull/915",
        )
        .await;

        let mut input = pilot_input("dev-pilot", "mika#920", &wi_id);
        input["iteration_context"] = serde_json::json!("Fix the failing AC on Unit 5");

        let result = validate_dispatch_readiness(&async_db, &wi_id, None, Some(&input), None).await;

        assert!(
            result.is_ok(),
            "iteration_context bypass should allow dispatch, got: {result:?}"
        );
    }

    /// Scenario 3: Fresh dispatch (no `pr_url` in metadata) → bypass.
    #[tokio::test]
    async fn test_open_pr_guard_bypasses_when_no_pr_url() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;
        // No metadata write — task has no claude_pilot.pr_url field.

        let input = pilot_input("dev-pilot", "mika#920", &wi_id);
        let result = validate_dispatch_readiness(&async_db, &wi_id, None, Some(&input), None).await;

        assert!(
            result.is_ok(),
            "fresh dispatch without pr_url should proceed, got: {result:?}"
        );
    }

    /// Scenario 4 (F2 regression): Ready-label webhook with open PR → bypass.
    #[tokio::test]
    async fn test_open_pr_guard_bypasses_ready_label_webhook() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;
        set_task_pr_url(
            &async_db,
            &wi_id,
            "https://github.com/senara-solutions/mika/pull/915",
        )
        .await;

        let input = pilot_input("dev-pilot", "mika#920", &wi_id);
        let result = validate_dispatch_readiness(
            &async_db,
            &wi_id,
            None,
            Some(&input),
            Some("[GitHub] Issue labeled ready on senara-solutions/mika#920 — title"),
        )
        .await;

        assert!(
            result.is_ok(),
            "ready-label webhook should bypass open-PR guard (operator positive consent), got: {result:?}"
        );
    }

    /// Scenario 5 (F3 regression): DeferredDispatch sentinel with open PR → bypass.
    #[tokio::test]
    async fn test_open_pr_guard_bypasses_deferred_dispatch_sentinel() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;
        set_task_pr_url(
            &async_db,
            &wi_id,
            "https://github.com/senara-solutions/mika/pull/915",
        )
        .await;

        let mut input = pilot_input("dev-pilot", "mika#920", &wi_id);
        input[INTERNAL_DEFERRED_DISPATCH_FIELD] = serde_json::json!(true);

        let result = validate_dispatch_readiness(&async_db, &wi_id, None, Some(&input), None).await;

        assert!(
            result.is_ok(),
            "deferred-dispatch sentinel should bypass open-PR guard (engine recovery), got: {result:?}"
        );
    }

    /// Bypass via skill: `dev-groom` is fresh grooming, not implementation re-run.
    #[tokio::test]
    async fn test_open_pr_guard_bypasses_dev_groom_skill() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;
        set_task_pr_url(
            &async_db,
            &wi_id,
            "https://github.com/senara-solutions/mika/pull/915",
        )
        .await;

        let input = pilot_input("dev-groom", "mika#920", &wi_id);
        let result = validate_dispatch_readiness(&async_db, &wi_id, None, Some(&input), None).await;

        assert!(
            result.is_ok(),
            "dev-groom dispatch should bypass open-PR guard, got: {result:?}"
        );
    }

    /// Sentinel-on-input survives the register_deferred_callback → replay round-trip.
    #[tokio::test]
    async fn test_register_deferred_callback_injects_sentinel() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;

        let input = pilot_input("dev-pilot", "mika#920", &wi_id);
        let registered = register_deferred_callback(&async_db, &wi_id, &input).await;
        assert!(registered, "deferred callback should register");

        // Walk the child tasks to find the deferred one and inspect its action_config.
        let children = async_db.get_child_tasks(&wi_id).await.unwrap();
        let deferred = children
            .iter()
            .find(|c| c.label == crate::agent::DEFERRED_DISPATCH_LABEL)
            .expect("deferred callback should be a child of the parent");

        let config: serde_json::Value = serde_json::from_str(&deferred.action_config).unwrap();
        let original_call = config
            .get("original_call")
            .expect("action_config should contain original_call");
        assert_eq!(
            original_call.get(INTERNAL_DEFERRED_DISPATCH_FIELD),
            Some(&serde_json::Value::Bool(true)),
            "register_deferred_callback must inject the __internal_deferred_dispatch sentinel"
        );
        // Original fields must be preserved alongside the sentinel.
        assert_eq!(
            original_call.get("skill"),
            Some(&serde_json::json!("dev-pilot"))
        );
        assert_eq!(
            original_call.get("prompt"),
            Some(&serde_json::json!("mika#920"))
        );
    }

    // ===================================================================
    // Idempotent-deferred intercept tests (mika#1205)
    // ===================================================================
    //
    // These tests exercise the intercept inserted in `execute_long_running`
    // between `validate_task` and `validate_dispatch_readiness`. The intercept
    // short-circuits with `status: "deferred", already_deferred: true` when a
    // per-agent pending deferred-callback child exists, so the LLM does not see
    // the `unauthorized_webhook_dispatch` guard (0) rejection that triggers
    // mika#716's hallucinated supervisor → blocked transition.

    /// Helper: construct a LongRunningContext with an explicit originating_message.
    fn make_lr_ctx_with_msg(
        db: crate::async_db::AsyncDatabase,
        originating_message: Option<String>,
    ) -> LongRunningContext {
        LongRunningContext {
            db,
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
            originating_message,
        }
    }

    /// AC1: When the LLM retries `run_claude_pilot` on a task that has a pending
    /// deferred-callback child for the same agent AND the originating_message is
    /// unauthorized, the intercept returns `ToolOutput::success` with
    /// `status: "deferred"` and `already_deferred: true`. No
    /// `unauthorized_webhook_dispatch` error reaches the LLM.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_execute_long_running_idempotent_ack_on_pending_deferred() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;

        // Seed a pending deferred-callback child for the same agent.
        let input = pilot_input("dev-pilot", "mika#1205", &wi_id);
        let registered = register_deferred_callback(&async_db, &wi_id, &input).await;
        assert!(registered, "deferred callback should register");

        // Unauthorized originating_message (Webhook Fallthrough domain) — would
        // normally trip guard (0). The intercept must fire BEFORE guard (0).
        let ctx = make_lr_ctx_with_msg(
            async_db,
            Some("[GitHub] New comment on issue#789".to_string()),
        );

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id, "skill": "dev-pilot", "prompt": "mika#1205"}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(
            !output.is_error,
            "intercept should return success, got error: {}",
            output.content
        );
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON success, got: {}", output.content));
        assert_eq!(parsed["status"], "deferred");
        assert_eq!(parsed["already_deferred"], true);
        assert!(
            !output.content.contains("unauthorized_webhook_dispatch"),
            "intercept must not surface guard (0) rejection: {}",
            output.content
        );
    }

    /// AC2: When no pending deferred-callback child exists, `execute_long_running`
    /// falls through to `validate_dispatch_readiness`. With an unauthorized
    /// originating_message, guard (0) rejects with `unauthorized_webhook_dispatch`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_execute_long_running_no_intercept_when_no_deferred_child() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;

        // No deferred-callback child seeded.
        let ctx = make_lr_ctx_with_msg(
            async_db,
            Some("[GitHub] New comment on issue#789".to_string()),
        );

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id, "skill": "dev-pilot", "prompt": "mika#1205"}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(output.is_error, "guard (0) should reject the dispatch");
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "unauthorized_webhook_dispatch");
    }

    /// AC3: When a pending deferred-callback child exists but belongs to a
    /// different `agent_id`, the intercept does not fire (per-agent isolation).
    /// Falls through to `validate_dispatch_readiness` which rejects.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_execute_long_running_intercept_scopes_per_agent() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;

        // Register the foreign agent so the FK constraint on tasks.agent_id holds.
        async_db
            .register_agent("other-agent", "Other Agent", "")
            .await
            .unwrap();

        // Seed a deferred-callback child but with a DIFFERENT agent_id.
        let foreign_child = crate::db::NewTask {
            agent_id: "other-agent".to_string(),
            team_run_id: None,
            parent_task_id: Some(wi_id.clone()),
            depth: 0,
            label: crate::agent::DEFERRED_DISPATCH_LABEL.to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: Some("deferred_dispatch".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        async_db.create_task(foreign_child).await.unwrap();

        let ctx = make_lr_ctx_with_msg(
            async_db,
            Some("[GitHub] New comment on issue#789".to_string()),
        );

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id, "skill": "dev-pilot", "prompt": "mika#1205"}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(
            output.is_error,
            "intercept must not match across agents; guard (0) should reject"
        );
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "unauthorized_webhook_dispatch");
    }

    /// AC4: When a deferred-callback child has completed (or failed), the
    /// intercept does not fire. Falls through to guard (0) which rejects.
    /// Proves fail-closed after DeferredDispatch resumes and the child completes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_execute_long_running_intercept_skips_completed_children() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;

        // Seed a deferred-callback child, then transition it to completed.
        let input = pilot_input("dev-pilot", "mika#1205", &wi_id);
        let registered = register_deferred_callback(&async_db, &wi_id, &input).await;
        assert!(registered);
        let children = async_db.get_child_tasks(&wi_id).await.unwrap();
        let deferred = children
            .iter()
            .find(|c| c.label == crate::agent::DEFERRED_DISPATCH_LABEL)
            .expect("deferred callback should exist");
        async_db
            .update_task_status(&deferred.id, "completed")
            .await
            .unwrap();

        let ctx = make_lr_ctx_with_msg(
            async_db,
            Some("[GitHub] New comment on issue#789".to_string()),
        );

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id, "skill": "dev-pilot", "prompt": "mika#1205"}),
            30,
            Some(&ctx),
            None,
            None,
            None,
        )
        .await;

        assert!(
            output.is_error,
            "completed deferred child must not authorize retry"
        );
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "unauthorized_webhook_dispatch");
    }

    // ===================================================================
    // Cycle detection and callback deferred dispatch tests (mika#1058)
    // ===================================================================

    #[test]
    fn test_parse_repo_issue_valid() {
        let (repo, issue) = parse_repo_issue(Some("mika#159"));
        assert_eq!(repo, Some("mika"));
        assert_eq!(issue, Some(159));
    }

    #[test]
    fn test_parse_repo_issue_with_prefix() {
        let (repo, issue) = parse_repo_issue(Some("Fix bug in mika-skills#42 please"));
        assert_eq!(repo, Some("mika-skills"));
        assert_eq!(issue, Some(42));
    }

    #[test]
    fn test_parse_repo_issue_none() {
        let (repo, issue) = parse_repo_issue(None);
        assert!(repo.is_none());
        assert!(issue.is_none());
    }

    #[test]
    fn test_parse_repo_issue_no_match() {
        let (repo, issue) = parse_repo_issue(Some("just some text without issue ref"));
        assert!(repo.is_none());
        assert!(issue.is_none());
    }

    #[test]
    fn test_parse_repo_issue_bare_hash() {
        // "#42" — no repo name
        let (repo, issue) = parse_repo_issue(Some("#42"));
        assert!(repo.is_none());
        assert!(issue.is_none());
    }

    #[test]
    fn test_extract_dispatch_tuple_from_action_config() {
        let task = crate::db::Task {
            id: "task-1".to_string(),
            agent_id: "mika-dev".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot:deferred".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: serde_json::json!({
                "trigger_kind": "deferred_dispatch",
                "original_call": {
                    "skill": "dev-groom",
                    "prompt": "mika#159",
                    "task_id": "parent-1"
                }
            })
            .to_string(),
            status: "pending".to_string(),
            process_id: None,
            input_context: None,
            result: None,
            created_by_session: None,
            created_trace_id: None,
            execution_trace_id: None,
            created_at: "2026-05-10T00:00:00Z".to_string(),
            updated_at: "2026-05-10T00:00:00Z".to_string(),
            fired_at: None,
            completed_at: None,
            reference_url: None,
            source: Some("deferred_dispatch".to_string()),
            metadata: None,
            r#type: "issue".to_string(),
            dispatch_class: Some("groom".to_string()),
        };

        let result = extract_dispatch_tuple(&task);
        assert!(result.is_some(), "expected Some, got None");
        let (repo, issue, skill) = result.unwrap();
        assert_eq!(repo, "mika");
        assert_eq!(issue, 159);
        assert_eq!(skill, "dev-groom");
    }

    #[tokio::test]
    async fn test_cycle_detection_rejects_same_tuple() {
        let db = crate::db::Database::open_in_memory().unwrap();
        db.create_session("test-session", "mika", "cli").unwrap();
        let async_db = crate::async_db::AsyncDatabase::new(db);

        // Create parent task with action_config containing (mika, 159, dev-groom)
        let parent_task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot:deferred".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: serde_json::json!({
                "trigger_kind": "deferred_dispatch",
                "original_call": {
                    "skill": "dev-groom",
                    "prompt": "mika#159",
                    "task_id": "grandparent-1"
                }
            })
            .to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("deferred_dispatch".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let parent_id = async_db.create_task(parent_task).await.unwrap();

        // Create child callback task
        let child_task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "callback".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let child_id = async_db.create_task(child_task).await.unwrap();

        // Propose same (mika, 159, dev-groom) — should be rejected
        let proposed = serde_json::json!({
            "skill": "dev-groom",
            "prompt": "mika#159",
            "task_id": &child_id
        });
        let result = check_lineage_cycle(&async_db, &child_id, &proposed).await;
        assert!(result.is_err(), "expected cycle detection to reject");
        assert!(
            result.unwrap_err().contains("Cycle detected"),
            "expected cycle message"
        );
    }

    #[tokio::test]
    async fn test_cycle_detection_allows_cross_skill() {
        let db = crate::db::Database::open_in_memory().unwrap();
        db.create_session("test-session", "mika", "cli").unwrap();
        let async_db = crate::async_db::AsyncDatabase::new(db);

        // Parent with (mika, 159, dev-groom)
        let parent_task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot:deferred".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: serde_json::json!({
                "trigger_kind": "deferred_dispatch",
                "original_call": {
                    "skill": "dev-groom",
                    "prompt": "mika#159",
                    "task_id": "grandparent-1"
                }
            })
            .to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("deferred_dispatch".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let parent_id = async_db.create_task(parent_task).await.unwrap();

        // Propose DIFFERENT skill (mika, 159, dev-pilot) — should be allowed
        let proposed = serde_json::json!({
            "skill": "dev-pilot",
            "prompt": "mika#159",
            "task_id": &parent_id
        });
        let result = check_lineage_cycle(&async_db, &parent_id, &proposed).await;
        assert!(result.is_ok(), "cross-skill chain should be allowed");
    }

    #[tokio::test]
    async fn test_gate_preserved_for_non_callback() {
        // Non-callback context: long_running tool with no long_running_ctx and
        // no callback_task_id should return the original error message.
        let tool = make_deferred_dispatch_tool();
        let input =
            serde_json::json!({"skill": "dev-pilot", "prompt": "mika#42", "task_id": "abc"});
        let output = execute_skill_tool(&tool, input, 30, None, None, None, None).await;
        assert!(output.is_error, "expected error for non-callback context");
        assert!(
            output.content.contains("cannot run in the current context"),
            "expected original gate error, got: {}",
            output.content
        );
    }

    // -- rearm_deferred_callback tests (mika#2045) --

    /// A `pending` self_dev issue parent plus a consumed deferred wrapper —
    /// the shape left behind when promotion fires and the turn never dispatches.
    async fn rearm_fixture() -> (crate::async_db::AsyncDatabase, String, String) {
        let db = crate::db::Database::open_in_memory().unwrap();
        db.create_session("test-session", "mika", "cli").unwrap();
        let async_db = crate::async_db::AsyncDatabase::new(db);

        let mut parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "ready-label: x/y#2045".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: Some("https://github.com/x/y/issues/2045".to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: Some("issue".to_string()),
            dispatch_class: Some("implement".to_string()),
        };
        parent.depth = 0;
        let parent_id = async_db.create_task(parent).await.unwrap();

        let action_config = serde_json::json!({
            "trigger_kind": "deferred_dispatch",
            "original_call": {
                "skill": "dev-pilot",
                "prompt": "mika#2045",
                "task_id": parent_id,
                INTERNAL_DEFERRED_DISPATCH_FIELD: true,
            }
        })
        .to_string();

        (async_db, parent_id, action_config)
    }

    async fn pending_wrappers_of(db: &crate::async_db::AsyncDatabase, parent_id: &str) -> usize {
        db.get_child_tasks(parent_id)
            .await
            .unwrap()
            .into_iter()
            .filter(|c| c.label == crate::agent::DEFERRED_DISPATCH_LABEL && c.status == "pending")
            .count()
    }

    #[tokio::test]
    async fn test_rearm_registers_replacement_wrapper() {
        let (db, parent_id, action_config) = rearm_fixture().await;

        let outcome = rearm_deferred_callback(
            &db,
            &parent_id,
            &action_config,
            "implement",
            "noop_completion",
        )
        .await;

        assert_eq!(
            outcome,
            RearmOutcome::Rearmed,
            "re-arm must succeed with budget available"
        );
        assert_eq!(pending_wrappers_of(&db, &parent_id).await, 1);
        assert_eq!(db.get_stuck_rearm_count(&parent_id).await.unwrap(), 1);
    }

    /// The replacement replays the same dispatch — including the sentinel that
    /// keeps the open-PR guard (mika#920) from livelocking the replay.
    #[tokio::test]
    async fn test_rearm_preserves_action_config_and_class() {
        let (db, parent_id, action_config) = rearm_fixture().await;

        assert_eq!(
            rearm_deferred_callback(
                &db,
                &parent_id,
                &action_config,
                "groom",
                "silent_turn_error"
            )
            .await,
            RearmOutcome::Rearmed
        );

        let wrapper = db
            .get_child_tasks(&parent_id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.label == crate::agent::DEFERRED_DISPATCH_LABEL)
            .expect("replacement wrapper");

        assert_eq!(wrapper.action_config, action_config);
        assert_eq!(wrapper.dispatch_class.as_deref(), Some("groom"));
        assert!(
            wrapper
                .action_config
                .contains(INTERNAL_DEFERRED_DISPATCH_FIELD)
        );
        assert_eq!(
            wrapper.reference_url, None,
            "wrappers must not take the index slot"
        );
    }

    /// Termination: the budget bounds the total repairs, so a parent whose turns
    /// never dispatch stops being re-armed and falls to the reaper.
    #[tokio::test]
    async fn test_rearm_refuses_once_budget_is_exhausted() {
        let (db, parent_id, action_config) = rearm_fixture().await;

        for _ in 0..MAX_STUCK_REARMS {
            assert_eq!(
                rearm_deferred_callback(&db, &parent_id, &action_config, "implement", "noop").await,
                RearmOutcome::Rearmed
            );
            // Consume the wrapper the way promotion does.
            for child in db.get_child_tasks(&parent_id).await.unwrap() {
                if child.label == crate::agent::DEFERRED_DISPATCH_LABEL && child.status == "pending"
                {
                    db.update_task_status(&child.id, "completed").await.unwrap();
                }
            }
        }

        assert_eq!(
            db.get_stuck_rearm_count(&parent_id).await.unwrap(),
            MAX_STUCK_REARMS
        );
        assert_eq!(
            rearm_deferred_callback(&db, &parent_id, &action_config, "implement", "noop").await,
            RearmOutcome::Unrepairable,
            "budget exhausted is terminal — the reaper may expire the task"
        );
        assert_eq!(pending_wrappers_of(&db, &parent_id).await, 0);
    }

    /// A full deferred-callback queue is a passing condition, not a verdict on
    /// this task. Reporting it as terminal would let the reaper destroy a task
    /// that still had repair budget left.
    #[tokio::test]
    async fn test_rearm_reports_flood_cap_as_transient_not_terminal() {
        let (db, parent_id, action_config) = rearm_fixture().await;

        for i in 0..MAX_PENDING_DEFERRED_CALLBACKS {
            let other = NewTask {
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: crate::agent::DEFERRED_DISPATCH_LABEL.to_string(),
                trigger_type: trigger_type::CALLBACK.to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: action_type::RESUME_AGENT.to_string(),
                action_config: format!("{{\"n\":{i}}}"),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: None,
                dispatch_class: Some("implement".to_string()),
            };
            db.create_task(other).await.unwrap();
        }

        assert_eq!(
            rearm_deferred_callback(&db, &parent_id, &action_config, "implement", "noop").await,
            RearmOutcome::NotNow,
            "a full queue clears on its own — the task must survive to be retried"
        );
        assert_eq!(
            db.get_stuck_rearm_count(&parent_id).await.unwrap(),
            0,
            "a refusal for capacity must not spend repair budget"
        );
    }

    /// Anti-vacuity: the turn DID dispatch. Re-arming here would double-dispatch.
    #[tokio::test]
    async fn test_rearm_declines_when_parent_already_dispatched() {
        let (db, parent_id, action_config) = rearm_fixture().await;

        let real_callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "long_running:run_claude_pilot".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        db.create_task(real_callback).await.unwrap();

        assert_eq!(
            rearm_deferred_callback(&db, &parent_id, &action_config, "implement", "noop").await,
            RearmOutcome::NotNow,
            "a live dispatch means nothing to repair — and nothing to expire either"
        );
        assert_eq!(pending_wrappers_of(&db, &parent_id).await, 0);
        assert_eq!(db.get_stuck_rearm_count(&parent_id).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_callback_deferred_dispatch_registered() {
        // Callback context with callback_task_id and db should register a
        // deferred dispatch instead of returning a hard error.
        let db = crate::db::Database::open_in_memory().unwrap();
        db.create_session("test-session", "mika", "cli").unwrap();
        let async_db = crate::async_db::AsyncDatabase::new(db);

        // Create a parent manual task
        let parent_task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "implement mika#42".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let parent_id = async_db.create_task(parent_task).await.unwrap();

        // Create callback task (child of parent)
        let callback_task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "callback".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let callback_id = async_db.create_task(callback_task).await.unwrap();

        let tool = make_deferred_dispatch_tool();
        let input = serde_json::json!({
            "skill": "dev-pilot",
            "prompt": "mika#42",
            "task_id": &parent_id
        });
        let output = execute_skill_tool(
            &tool,
            input,
            30,
            None,
            None,
            Some(&callback_id),
            Some(&async_db),
        )
        .await;

        // Should succeed with deferred status, not error
        assert!(
            !output.is_error,
            "expected deferred success, got error: {}",
            output.content
        );
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON, got: {}", output.content));
        assert_eq!(parsed["status"], "deferred");
        assert_eq!(parsed["deferred"], true);

        // Verify a deferred callback was created in the DB
        let count = async_db.count_pending_deferred_callbacks().await.unwrap();
        assert_eq!(count, 1, "expected one deferred callback to be registered");
    }

    /// Helper: create a long_running tool fixture for deferred dispatch tests (mika#1058).
    fn make_deferred_dispatch_tool() -> ResolvedSkillTool {
        ResolvedSkillTool {
            definition: ToolDefinition {
                name: "run_claude_pilot".to_string(),
                description: "Test long-running tool".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["skill", "prompt", "task_id"],
                    "properties": {
                        "skill": { "type": "string" },
                        "prompt": { "type": "string" },
                        "task_id": { "type": "string" }
                    }
                }),
            },
            handler: ToolHandler::Exec {
                command: "handler.sh".to_string(),
                long_running: true,
                estimated_duration_secs: Some(3600),
            },
            skill_dir: PathBuf::from("/tmp"),
        }
    }

    // --- check_grooming_markers tests (#919) ---

    /// Fully groomed issue body — all three signals present.
    #[test]
    fn test_grooming_markers_all_present() {
        let body = r#"
> - **Branch:** `fix/919/self-dev-agent-operator-cli-dispatch`
> - **Plan:** `mika/docs/plans/2026-05-13-001-fix-dispatch-grooming-marker-engine-guard-plan.md` (committed on branch @ `3f99625a`)
> - **Grooming history:** /ce:plan → mika-arch first-pass (ITERATE) → revisions → mika-arch second-pass (GROOMED)

## Symptom
Some issue body text here.
"#;
        let missing = check_grooming_markers(body);
        assert!(
            missing.is_empty(),
            "expected no missing signals, got: {missing:?}"
        );
    }

    /// Completely ungroomed — missing all three signals.
    #[test]
    fn test_grooming_markers_all_missing() {
        let body = "## Some Issue\n\nJust a plain issue body with no grooming markers.";
        let missing = check_grooming_markers(body);
        assert_eq!(
            missing,
            vec!["branch_callout", "plan_callout", "groomed_verdict"]
        );
    }

    /// Partially groomed — has Plan but missing Branch and verdict.
    #[test]
    fn test_grooming_markers_partial_missing() {
        let body = r#"
Some text here.

Plan: docs/plans/some-plan.md is referenced.

No branch callout and no second-pass line.
"#;
        let missing = check_grooming_markers(body);
        assert_eq!(missing, vec!["branch_callout", "groomed_verdict"]);
    }

    /// Has Branch and Plan but missing the architect verdict.
    #[test]
    fn test_grooming_markers_missing_verdict_only() {
        let body = r#"
> - **Branch:** `feat/something`
> - **Plan:** `mika/docs/plans/some-plan.md` (committed on branch @ `abc123`)

Plan: docs/plans/some-plan.md is also in the body.
"#;
        let missing = check_grooming_markers(body);
        assert_eq!(missing, vec!["groomed_verdict"]);
    }

    /// Has verdict text but not the exact canonical shape — must fail.
    #[test]
    fn test_grooming_markers_wrong_verdict_shape() {
        let body = r#"
> - **Branch:** `feat/something`
Plan: docs/plans/foo.md
Verdict: GROOMED
"#;
        // "Verdict: GROOMED" is NOT the canonical shape. The canonical shape
        // is "second-pass (GROOMED)" from the grooming history line.
        let missing = check_grooming_markers(body);
        assert_eq!(missing, vec!["groomed_verdict"]);
    }

    /// Empty body — all signals missing.
    #[test]
    fn test_grooming_markers_empty_body() {
        let missing = check_grooming_markers("");
        assert_eq!(
            missing,
            vec!["branch_callout", "plan_callout", "groomed_verdict"]
        );
    }

    // --- check_grooming_markers #1725 parameterized verdict widening ---

    /// mika#1723 dispatch failure shape: orchestrator-CC produced
    /// `second-pass (GROOMED, session fd4c1a14)` — the comma broke the strict
    /// substring match, gate rejected with `dispatch_no_grooming_marker`.
    #[test]
    fn test_grooming_markers_accepts_comma_parameter() {
        let body = r#"
> - **Branch:** `fix/1725/loop-substrate`
> - **Plan:** `docs/plans/2026-07-04-001-fix-plan.md`
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED, session fd4c1a14)
"#;
        let missing = check_grooming_markers(body);
        assert!(
            missing.is_empty(),
            "comma-parameterized GROOMED must pass, got: {missing:?}"
        );
    }

    /// Em-dash-annotated form (canonical orchestrator-CC session-id shape).
    #[test]
    fn test_grooming_markers_accepts_em_dash_annotation() {
        let body = r#"
> - **Branch:** `fix/1725/loop-substrate`
> - **Plan:** `docs/plans/2026-07-04-001-fix-plan.md`
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED — session-id: fd4c1a14)
"#;
        let missing = check_grooming_markers(body);
        assert!(
            missing.is_empty(),
            "em-dash-annotated GROOMED must pass, got: {missing:?}"
        );
    }

    /// Period-terminated form: `second-pass (GROOMED. Full ratification.)`.
    #[test]
    fn test_grooming_markers_accepts_period_terminator() {
        let body = r#"
> - **Branch:** `fix/1725/loop-substrate`
> - **Plan:** `docs/plans/2026-07-04-001-fix-plan.md`
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED. Full ratification.)
"#;
        let missing = check_grooming_markers(body);
        assert!(
            missing.is_empty(),
            "period-terminated GROOMED must pass, got: {missing:?}"
        );
    }

    /// False-positive guard: prose `"the ticket was GROOMED yesterday"` must
    /// NOT satisfy the check. The `second-pass (` prefix anchor blocks it.
    #[test]
    fn test_grooming_markers_rejects_prose_groomed_without_prefix() {
        let body = r#"
> - **Branch:** `feat/something`
> - **Plan:** `docs/plans/some-plan.md`

Discussion: the ticket was GROOMED yesterday but never actually reviewed.
GROOMED status pending in another ticket.
"#;
        let missing = check_grooming_markers(body);
        assert_eq!(
            missing,
            vec!["groomed_verdict"],
            "prose GROOMED without `second-pass (` prefix must not match"
        );
    }

    /// False-positive guard: `second-pass (GROOMEDLY)` — letter-continuation
    /// after GROOMED must be rejected by the character class discriminator.
    #[test]
    fn test_grooming_markers_rejects_letter_continuation() {
        let body = r#"
> - **Branch:** `feat/something`
> - **Plan:** `docs/plans/some-plan.md`
> - **Grooming history:** first-pass (READY) → second-pass (GROOMEDLY reviewed) — bogus
"#;
        let missing = check_grooming_markers(body);
        assert_eq!(
            missing,
            vec!["groomed_verdict"],
            "letter-continuation after GROOMED must not match"
        );
    }

    /// False-positive guard: `first-pass (GROOMED)` — the `second-pass (`
    /// prefix requirement blocks this; first-pass verdict is READY/ITERATE/ESCALATE.
    #[test]
    fn test_grooming_markers_rejects_first_pass_groomed() {
        let body = r#"
> - **Branch:** `feat/something`
> - **Plan:** `docs/plans/some-plan.md`
> - **Grooming history:** first-pass (GROOMED) — no second-pass, invalid
"#;
        let missing = check_grooming_markers(body);
        assert_eq!(
            missing,
            vec!["groomed_verdict"],
            "first-pass (GROOMED) without second-pass must not match"
        );
    }

    // --- check_grooming_markers recovery callout tests (#1123) ---

    /// Recovery callout written by dispatch-lib.sh post-flight (mika#1123)
    /// intentionally does NOT pass the gate — it surfaces drift without
    /// fabricating an architect verdict.
    #[test]
    fn test_check_grooming_markers_recovery_callout_does_not_pass() {
        let body = r#"> - **Branch:** `fix/794/agent-pr-merge`
> - **Plan:** `docs/plans/2026-05-15-001-fix-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** body callout recovered by post-flight (mika#1123) — architect verdict not verified, operator dispatch required

## Symptom
..."#;
        let missing = check_grooming_markers(body);
        // Branch and plan callouts pass, but groomed_verdict is correctly missing
        assert!(
            !missing.contains(&"branch_callout"),
            "Recovery callout should pass branch_callout check"
        );
        assert!(
            !missing.contains(&"plan_callout"),
            "Recovery callout should pass plan_callout check"
        );
        assert!(
            missing.contains(&"groomed_verdict"),
            "Recovery callout must NOT pass the groomed_verdict check — \
             it doesn't fabricate an architect verdict"
        );
    }

    /// Organic callout written by the LLM in dev-groom step 18 — all three
    /// signals present, should pass the gate completely.
    #[test]
    fn test_check_grooming_markers_organic_callout_passes() {
        let body = r#"> - **Branch:** `fix/794/agent-pr-merge`
> - **Plan:** `docs/plans/2026-05-15-001-fix-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** /ce:plan -> mika-arch first-pass (ITERATE) -> revisions -> mika-arch second-pass (GROOMED)

## Symptom
..."#;
        let missing = check_grooming_markers(body);
        assert!(
            missing.is_empty(),
            "Organic callout with all three signals should pass: {missing:?}"
        );
    }

    // --- check_grooming_markers single-pass verdict tests (mika#2012) ---

    /// The exact line `write_canonical_callout`'s `ready-single-pass` stage
    /// emits. Before mika#2012 this body had no recognized verdict, so the
    /// ticket stayed invisible to the gate and was re-dispatched as `dev-groom`
    /// forever.
    #[test]
    fn test_grooming_markers_accepts_single_pass_ready() {
        let body = r#"
> - **Branch:** `fix/2012/dispatch-le-loop-re-groome-des-tickets-d`
> - **Plan:** `docs/plans/2026-08-27-001-fix-2012-regroom-loop-verdict-gate-plan.md` (committed on branch @ `d2bd0ed2`)
> - **Grooming history:** first-pass (READY, single-pass GROOMED) — no second pass required — session-id: 66811de9
"#;
        let missing = check_grooming_markers(body);
        assert!(
            missing.is_empty(),
            "single-pass READY grooming exit must pass, got: {missing:?}"
        );
    }

    /// Load-bearing false-positive guard: a **bare** `first-pass (READY)` must
    /// still fail. It is the disposition the architect emits mid-grooming,
    /// before `write_canonical_callout` has committed the plan and stamped the
    /// callout — accepting it would dispatch tickets whose plan is not on the
    /// branch. Only the explicit `single-pass GROOMED` annotation, which only
    /// the writer emits, closes the gate.
    #[test]
    fn test_grooming_markers_rejects_bare_first_pass_ready() {
        let body = r#"
> - **Branch:** `feat/something`
> - **Plan:** `docs/plans/some-plan.md`
> - **Grooming history:** first-pass (READY) — awaiting second pass
"#;
        let missing = check_grooming_markers(body);
        assert_eq!(
            missing,
            vec!["groomed_verdict"],
            "bare first-pass (READY) must not satisfy the verdict signal"
        );
    }

    /// False-positive guard: prose mentioning the annotation without the
    /// `first-pass (` prefix anchor must not match.
    #[test]
    fn test_grooming_markers_rejects_prose_single_pass_groomed() {
        let body = r#"
> - **Branch:** `feat/something`
> - **Plan:** `docs/plans/some-plan.md`

Discussion: this one was a single-pass GROOMED case, unlike the others.
"#;
        let missing = check_grooming_markers(body);
        assert_eq!(
            missing,
            vec!["groomed_verdict"],
            "prose `single-pass GROOMED` without the first-pass anchor must not match"
        );
    }

    /// Non-regression: the spec-tolerated paraphrase (#1108) still passes after
    /// the mika#2012 widening.
    #[test]
    fn test_grooming_markers_paraphrased_still_passes() {
        let body = r#"
> - **Branch:** `feat/something`
> - **Plan:** `docs/plans/some-plan.md`
> - **Grooming history:** first-pass (ITERATE) → second-pass (READY, paraphrased GROOMED — plan sound)
"#;
        let missing = check_grooming_markers(body);
        assert!(
            missing.is_empty(),
            "paraphrased GROOMED must still pass after #2012, got: {missing:?}"
        );
    }

    /// Non-regression: the canonical two-pass verdict is unaffected by the
    /// added alternative.
    #[test]
    fn test_grooming_markers_two_pass_still_passes_after_2012() {
        let body = r#"
> - **Branch:** `feat/something`
> - **Plan:** `docs/plans/some-plan.md`
> - **Grooming history:** first-pass (ITERATE) → revisions → second-pass (GROOMED — session-id: abc123)
"#;
        let missing = check_grooming_markers(body);
        assert!(
            missing.is_empty(),
            "canonical two-pass GROOMED must still pass, got: {missing:?}"
        );
    }

    /// Bypass predicate: extract_skill_from_input returns correct skill.
    #[test]
    fn test_extract_skill_dev_pilot() {
        let input = serde_json::json!({"skill": "dev-pilot", "prompt": "mika#919"});
        assert_eq!(extract_skill_from_input(&input), Some("dev-pilot"));
    }

    /// Bypass predicate: extract_skill_from_input returns dev-groom.
    #[test]
    fn test_extract_skill_dev_groom() {
        let input = serde_json::json!({"skill": "dev-groom", "prompt": "mika#919"});
        assert_eq!(extract_skill_from_input(&input), Some("dev-groom"));
    }

    /// Bypass predicate: extract_skill_from_input returns None for missing skill.
    #[test]
    fn test_extract_skill_missing() {
        let input = serde_json::json!({"prompt": "mika#919"});
        assert_eq!(extract_skill_from_input(&input), None);
    }
}
