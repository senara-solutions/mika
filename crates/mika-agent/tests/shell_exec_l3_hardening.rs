//! Regression suite for the `shell-exec` L3 hardening (mika#1957).
//!
//! F3 of the mika#1798 adversarial review: the `shell-exec` skill's block-list
//! only inspected the command's first token, so any shape that reached a gated
//! CLI through a subshell, a path prefix, or a statement separator bypassed it —
//! and, because the call never entered the `run_gws`/`run_gh` builtin handler,
//! it bypassed all four non-transit doctrine layers with it.
//!
//! These tests drive the real `templates/skills/shell-exec/handlers/run.sh`
//! (materialized verbatim via `include_str!`) the same way the exec handler
//! does: JSON on stdin, output on stdout, refusals on stderr with a non-zero
//! exit. Every bypass shape enumerated in the plan gets a case, and the
//! false-positive guards get one each so a future tightening of the scan cannot
//! silently start rejecting ordinary commands.
//!
//! Plan: `docs/plans/2026-08-23-003-fix-1957-shell-exec-l3-hardening-plan.md`

use std::io::Write;
use std::process::{Command, Stdio};

const RUN_SH: &str = include_str!("../templates/skills/shell-exec/handlers/run.sh");

/// Refusal marker emitted by the L3 hardening scan. Distinct from the older
/// first-word refusals so a test can tell which gate fired.
const L3_REFUSAL: &str = "shell-exec refuses commands that route to skill-gated CLIs";

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

/// Assert the handler refused `command` via the L3 hardening scan.
fn assert_blocked_by_l3(command: &str) {
    let out = run_handler(command);
    assert!(
        !out.success,
        "expected refusal for {command:?}, but the handler succeeded with stdout: {}",
        out.stdout
    );
    assert!(
        out.stderr.contains(L3_REFUSAL),
        "expected the L3 hardening refusal for {command:?}, got stderr: {}",
        out.stderr
    );
}

/// Assert the handler refused `command` via the pre-existing first-word gate,
/// identified by its own distinct message. Matching on the specific string
/// matters: the L3 scan would refuse these commands too, so a test that only
/// asserted "some error" would keep passing if the first-word block were
/// deleted — and would stop being the regression guard it claims to be.
fn assert_blocked_by_first_word(command: &str, skill: &str) {
    let out = run_handler(command);
    assert!(
        !out.success,
        "expected refusal for {command:?}, but the handler succeeded with stdout: {}",
        out.stdout
    );
    let expected = format!("Use the dedicated {skill} skill instead of run_shell");
    assert!(
        out.stderr.contains(&expected),
        "expected the first-word refusal ({expected:?}) for {command:?}, got stderr: {}",
        out.stderr
    );
}

/// Assert `command` ran normally and produced `expected` on stdout.
fn assert_allowed(command: &str, expected: &str) {
    let out = run_handler(command);
    assert!(
        out.success,
        "expected {command:?} to be allowed, but it failed with stderr: {}",
        out.stderr
    );
    assert!(
        out.stdout.contains(expected),
        "expected {expected:?} in stdout for {command:?}, got: {}",
        out.stdout
    );
}

// --- Bypass shape 1: `sh -c "..."` subshell -----------------------------------

#[test]
fn shell_exec_rejects_sh_c_gws() {
    assert_blocked_by_l3("sh -c 'gws gmail messages list'");
}

#[test]
fn shell_exec_rejects_sh_c_gh() {
    assert_blocked_by_l3("sh -c 'gh pr merge 42'");
}

// --- Bypass shape 2: `bash -c "..."` / `eval "..."` ---------------------------

#[test]
fn shell_exec_rejects_bash_c_gws() {
    assert_blocked_by_l3("bash -c \"gws gmail messages list\"");
}

#[test]
fn shell_exec_rejects_eval_gws() {
    assert_blocked_by_l3("eval \"gws gmail messages list\"");
}

// --- Bypass shape 3: pipe into a shell ----------------------------------------

#[test]
fn shell_exec_rejects_piped_echo_sh() {
    assert_blocked_by_l3("echo 'gws gmail messages list' | sh");
}

// --- Bypass shape 4: absolute / path-prefixed invocation ----------------------

#[test]
fn shell_exec_rejects_absolute_path_gws() {
    assert_blocked_by_l3("/usr/bin/gws gmail messages list");
}

// --- Bypass shape 5: command substitution -------------------------------------

#[test]
fn shell_exec_rejects_command_substitution() {
    assert_blocked_by_l3("pwd; $(gws gmail messages list)");
}

#[test]
fn shell_exec_rejects_backtick_substitution() {
    assert_blocked_by_l3("echo `gws gmail messages list`");
}

// --- Bypass shape 6: statement separators -------------------------------------

#[test]
fn shell_exec_rejects_statement_separator() {
    assert_blocked_by_l3("pwd; gws gmail messages list");
}

#[test]
fn shell_exec_rejects_and_chain() {
    assert_blocked_by_l3("true && gws gmail messages list");
}

#[test]
fn shell_exec_rejects_newline_separated_statement() {
    assert_blocked_by_l3("pwd\ngws gmail messages list");
}

// --- Regression: the pre-existing first-word gate still refuses ---------------

#[test]
fn shell_exec_first_word_gws_still_blocked() {
    assert_blocked_by_first_word("gws gmail messages list", "run_gws");
}

#[test]
fn shell_exec_first_word_gh_still_blocked() {
    assert_blocked_by_first_word("gh pr list", "run_gh");
}

// --- False-positive guards: ordinary commands must keep working ---------------

#[test]
fn shell_exec_allows_plain_echo() {
    assert_allowed("echo hello", "hello");
}

/// `highlight` contains `gh`, but not on an identifier boundary.
#[test]
fn shell_exec_allows_substring_gh_inside_a_word() {
    assert_allowed("echo highlight", "highlight");
}

/// A dot precedes `gh` in `.github`, and `.` is deliberately excluded from the
/// boundary class so ordinary repository paths stay usable.
#[test]
fn shell_exec_allows_dot_github_paths() {
    assert_allowed("echo .github/workflows/ci.yml", ".github/workflows/ci.yml");
}

/// A hyphen follows `gh` in `gh-pages`, likewise excluded from the boundary.
#[test]
fn shell_exec_allows_gh_pages_ref() {
    assert_allowed("echo origin/gh-pages", "origin/gh-pages");
}

/// `gws.log` is a filename, not the `gws` binary — `.` is not a boundary.
#[test]
fn shell_exec_allows_gws_prefixed_filename() {
    assert_allowed("echo /tmp/gws.log", "/tmp/gws.log");
}

#[test]
fn shell_exec_allows_multi_token_command() {
    assert_allowed("echo one two three", "one two three");
}

// --- Documented deliberate false-positive ------------------------------------

/// Grepping for the literal string `gws` is refused. The plan accepts this:
/// `shell-exec` is a command-execution surface, and a lexical scan cannot tell
/// "mention `gws`" from "invoke `gws`" without parsing the shell grammar.
#[test]
fn shell_exec_rejects_literal_gws_mention_by_design() {
    assert_blocked_by_l3("grep gws /etc/services");
}
