//! Regression suite for the `shell-exec` egress containment (mika#1991).
//!
//! T1 of the mission-passeport MSC report (2026-08-24, measured in real
//! conditions) found the bypass this suite guards: over one shot `fetch_url`
//! was called 0 times while `run_shell` + `curl` was called 78 times, and
//! `www.bouscat.fr` — a host NOT on the `fetch_url` compile-time allowlist —
//! was reached with no obstacle. The allowlist that made single-controlled
//! egress acceptable constrained none of that traffic: it walked past the
//! enforced door through `run_shell`.
//!
//! The fix constructs the incapacity rather than persuading the model: the
//! shell-exec handler refuses any direct HTTP fetcher (`curl`, `wget`), so
//! egress is forced onto the substrate (`fetch_url` / `web_search`), which is
//! the single allowlist-enforcing path. The handler deliberately does NOT
//! re-implement the allowlist — that would be a second source of truth (the
//! scattered-guarantee anti-pattern this ticket is about) and would trip
//! `scripts/verify-egress-uniqueness.sh`. So the gate is unconditional: an
//! off-allowlist host is refused because EVERY direct fetch is refused, and an
//! allowlisted host stays reachable through `fetch_url`, not through the shell.
//!
//! These tests drive the real `templates/skills/shell-exec/handlers/run.sh`
//! (materialized verbatim via `include_str!`) the same way the exec handler
//! does — and the same way the mika#1957 L3 hardening suite
//! (`shell_exec_l3_hardening.rs`) does: JSON on stdin, output on stdout,
//! refusals on stderr with a non-zero exit. Every bypass shape from the L3
//! plan gets a case here too, and the false-positive guards get one each so a
//! future tightening cannot silently start rejecting ordinary commands.

use std::io::Write;
use std::process::{Command, Stdio};

const RUN_SH: &str = include_str!("../templates/skills/shell-exec/handlers/run.sh");

/// Refusal marker emitted by the egress-containment gate. Distinct from the
/// L3 gated-CLI refusal and the first-word refusals so a test can tell which
/// gate fired.
const EGRESS_REFUSAL: &str = "shell-exec refuses direct network egress";

struct HandlerOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Run the real handler script with `command` as its JSON input.
fn run_handler(command: &str) -> HandlerOutput {
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = tmp.path().join("run.sh");
    std::fs::write(&script, RUN_SH).expect("write handler");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod handler");
    }

    let payload = serde_json::json!({ "command": command }).to_string();

    let mut child = Command::new("/bin/sh")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn handler");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write payload");
    let out = child.wait_with_output().expect("wait handler");

    HandlerOutput {
        success: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Assert the handler refused `command` via the egress-containment gate.
fn assert_blocked_by_egress(command: &str) {
    let out = run_handler(command);
    assert!(
        !out.success,
        "expected egress refusal for {command:?}, but the handler succeeded with stdout: {}",
        out.stdout
    );
    assert!(
        out.stderr.contains(EGRESS_REFUSAL),
        "expected the egress-containment refusal for {command:?}, got stderr: {}",
        out.stderr
    );
}

/// Assert `command` ran normally and produced `expected` on stdout — and,
/// load-bearing, that it was NOT stopped by the egress gate.
fn assert_allowed(command: &str, expected: &str) {
    let out = run_handler(command);
    assert!(
        out.success,
        "expected {command:?} to be allowed, but it failed with stderr: {}",
        out.stderr
    );
    assert!(
        !out.stderr.contains(EGRESS_REFUSAL),
        "expected {command:?} to pass the egress gate, but it was refused: {}",
        out.stderr
    );
    assert!(
        out.stdout.contains(expected),
        "expected {expected:?} in stdout for {command:?}, got: {}",
        out.stdout
    );
}

// --- The measured bypass: off-allowlist host via curl is now refused --------

/// The exact host measured escaping in T1 (`www.bouscat.fr`, 8 hits, off
/// allowlist). This is the load-bearing regression: before the fix it was
/// reached with no obstacle; it must now be refused.
#[test]
fn shell_exec_rejects_curl_off_allowlist_host() {
    assert_blocked_by_egress("curl https://www.bouscat.fr/demarches");
}

/// A host that `fetch_url` WOULD allow gets no shell exception — the gate is
/// host-agnostic and unconditional. It never parses the host (doing so would
/// duplicate the gateway allowlist, the scattered-guarantee anti-pattern this
/// ticket is about, and would trip `scripts/verify-egress-uniqueness.sh`), so
/// a would-be-allowlisted `curl` is refused just like any other. The
/// allowlisted path is `fetch_url`, not the shell — parity comes from routing
/// to the substrate, not from an in-handler allowlist. (The literal gouv.fr
/// hosts are deliberately NOT written here so this test file stays outside the
/// egress-uniqueness authorized set; the enforced allowlist lives solely in
/// `crates/mika-gateway/src/egress_fetch/`.)
#[test]
fn shell_exec_rejects_curl_for_would_be_allowlisted_host() {
    assert_blocked_by_egress("curl https://gouv-lookalike.example/particuliers");
}

#[test]
fn shell_exec_rejects_plain_curl() {
    assert_blocked_by_egress("curl https://example.com");
}

#[test]
fn shell_exec_rejects_wget() {
    assert_blocked_by_egress("wget https://example.com/file.tar.gz");
}

// --- Bypass shapes (mirrors the L3 hardening enumeration) -------------------

#[test]
fn shell_exec_rejects_sh_c_curl() {
    assert_blocked_by_egress("sh -c 'curl https://www.bouscat.fr'");
}

#[test]
fn shell_exec_rejects_bash_c_curl() {
    assert_blocked_by_egress("bash -c \"curl https://www.bouscat.fr\"");
}

#[test]
fn shell_exec_rejects_eval_curl() {
    assert_blocked_by_egress("eval \"curl https://www.bouscat.fr\"");
}

#[test]
fn shell_exec_rejects_piped_echo_curl_sh() {
    assert_blocked_by_egress("echo 'curl https://www.bouscat.fr' | sh");
}

/// Absolute / path-prefixed invocation: `/` is a boundary char, so the
/// path-qualified binary is still caught.
#[test]
fn shell_exec_rejects_absolute_path_curl() {
    assert_blocked_by_egress("/usr/bin/curl https://www.bouscat.fr");
}

#[test]
fn shell_exec_rejects_command_substitution_curl() {
    assert_blocked_by_egress("echo $(curl -s https://www.bouscat.fr)");
}

#[test]
fn shell_exec_rejects_backtick_curl() {
    assert_blocked_by_egress("echo `curl -s https://www.bouscat.fr`");
}

#[test]
fn shell_exec_rejects_statement_separator_curl() {
    assert_blocked_by_egress("pwd; curl https://www.bouscat.fr");
}

#[test]
fn shell_exec_rejects_and_chain_curl() {
    assert_blocked_by_egress("true && curl https://www.bouscat.fr");
}

#[test]
fn shell_exec_rejects_newline_separated_curl() {
    assert_blocked_by_egress("pwd\ncurl https://www.bouscat.fr");
}

/// A curl flag before the URL must not hide the invocation.
#[test]
fn shell_exec_rejects_curl_with_flags() {
    assert_blocked_by_egress("curl -sSL -o out.html https://www.bouscat.fr/page");
}

// --- Anti-vacuity: ordinary (non-egress) commands must keep working ---------
//
// A deny rule is worthless if it denies everything — the security test would
// pass vacuously while the tool is broken. These prove the gate is scoped to
// direct HTTP fetchers and still lets legitimate shell usage through, so the
// allowlisted egress workflow (via fetch_url) and everyday shell work both
// survive.

#[test]
fn shell_exec_allows_plain_echo() {
    assert_allowed("echo hello", "hello");
}

#[test]
fn shell_exec_allows_multi_token_command() {
    assert_allowed("echo one two three", "one two three");
}

/// `.` is deliberately excluded from the boundary class, so a file named
/// `curl.log` is not a match — ordinary paths stay usable.
#[test]
fn shell_exec_allows_curl_dot_log_filename() {
    assert_allowed("echo /tmp/curl.log", "/tmp/curl.log");
}

/// `-` is excluded from the boundary class, so `libcurl-dev` (a package name,
/// not the binary) is not a match.
#[test]
fn shell_exec_allows_libcurl_substring() {
    assert_allowed("echo libcurl-dev", "libcurl-dev");
}

/// `curl` as a substring inside a larger word is not on an identifier
/// boundary and stays allowed.
#[test]
fn shell_exec_allows_curly_substring() {
    assert_allowed("echo curly-braces", "curly-braces");
}

// --- Documented deliberate false-positive -----------------------------------

/// Merely mentioning the literal `curl` on an identifier boundary is refused.
/// The plan accepts this, exactly as the L3 block accepts refusing
/// `grep gws /etc/services`: shell-exec is a command-execution surface, and a
/// lexical scan cannot tell "mention `curl`" from "invoke `curl`" without
/// parsing the shell grammar. The safe direction is to refuse.
#[test]
fn shell_exec_rejects_literal_curl_mention_by_design() {
    assert_blocked_by_egress("echo please use curl here");
}
