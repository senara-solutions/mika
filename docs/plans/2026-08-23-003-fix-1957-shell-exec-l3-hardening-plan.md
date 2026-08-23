# Plan — fix(shell-exec): CLI bypass hardening — L3 gate escape via shell-exec

**Status:** DRAFT
**Date:** 2026-08-23
**Ticket:** mika#1957
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** Non-transit data-grade doctrine follow-up (F3 from mika#1798 adversarial review)
**Cross-refs:** mika#1798 (parent doctrine bake), PR#1956 (open — sibling ticket land site), `feedback_prompt_enforcement_fragile`

## Why

PR#1956 (mika#1798) ships a four-layer structural bake of the non-transit data-grade doctrine (L1 prompt, L2 registry ban, L3 gws command validation, L4 execute-time guard). Adversarial review surfaced F3: **`shell-exec` bypasses L3 because it has its own security model** — `run_shell 'sh -c "gws gmail messages list"'` reaches the Google Workspace CLI via a subshell whose FIRST_WORD (`sh`) never trips the L3 gws validator.

Verified against current `main` state:
- `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh:27-32` — the current block-list is FIRST-WORD-only: `case "$FIRST_WORD" in gws) reject ;; gh) reject ;; esac`.
- shell-exec IS in `DEFAULT_AGENT_SKILL_ALLOWLIST` (`crates/mika-common/src/home.rs:367`) — personal-tier agents ship with the surface live.
- Bypass shape 1: `sh -c "gws gmail ..."` — first_word=`sh`, gates skip, subprocess spawns gws.
- Bypass shape 2: `bash -c "gws gmail ..."`, `zsh -c ...`, `eval "gws gmail ..."`.
- Bypass shape 3: pipe/redirect — `echo 'gws gmail ...' | sh`, `printf ... | bash`.
- Bypass shape 4: path-prefixed — `/usr/bin/gws gmail ...` — first_word=`/usr/bin/gws` (not `gws`).
- Bypass shape 5: command substitution — `` `gws gmail ...` `` or `$(gws gmail ...)`.
- Bypass shape 6: statement separators — `pwd; gws gmail ...`, `true && gws gmail ...`, newlines.

Prompt-only enforcement of "don't call gws through shell" is empirically fragile (per `feedback_prompt_enforcement_fragile` — the doctrine-bake ticket itself demonstrates n≥3 substrate hits requiring structural gates). This ticket adds the structural analog for the shell-exec surface.

## What

Two-tier defense-in-depth. Tier 1 is the primary structural gate (removes the surface entirely for the tier most exposed). Tier 2 is the shell-exec-internal hardening for cases where operators explicitly opt in.

### Tier 1 — Remove `shell-exec` from `DEFAULT_AGENT_SKILL_ALLOWLIST`

**File:** `crates/mika-common/src/home.rs:367` (and mirrored in the Python-format identity template just below — line 402).

**Change:** Delete the `"shell-exec"` entry from both `DEFAULT_AGENT_SKILL_ALLOWLIST` and its Python-format sibling. Also delete from `MIKA_DEV_IDENTITY`, `MIKA_ARCH`-computed identity, and other well-known-agent identity templates that don't need shell execution — verify per-agent via `grep -n '"shell-exec"' crates/mika-common/src/home.rs`.

**Preserve for:** operator-tier agents that genuinely need shell reach (e.g., `mika-dev` which dispatches claude-pilot subprocesses via shell). Explicit per-agent inclusion, not global default.

**Rationale (review-guide.md § YAGNI + Orthogonality):** personal-tier agents (the family-tier being that mika#1798 was designed to protect) never need arbitrary shell execution. The default allowlist was permissive by inheritance from mika-dev's identity template. Narrowing the default aligns tier-of-agent with tier-of-tool.

**Backward-compat:** existing installed agents' `identity.toml` files are untouched — the template only affects newly-provisioned or `MIKA_DISABLE_AGENT_PROVISIONING=false` reprovisioned agents. Operators who intentionally want shell-exec on their personal-tier agent add it to their agent's `[skills].allowlist` — one-line explicit consent, not a silent default.

### Tier 2 — Harden `run.sh` FIRST_WORD block-list to a semantic command-tokenizer

**File:** `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh:27-32`.

**Change:** Replace the FIRST_WORD-only case with a **command-string scan for banned tokens**. The scan detects the bypass shapes enumerated above by searching for `gws`, `gmail`, `drive.google` in the command string with word-boundary regex, considering both raw shell tokens and quoted substrings.

**Concrete implementation (POSIX sh, no bash-isms):**

```sh
# --- shell-exec L3 hardening (mika#1957) ---
# Reject commands that route (via any shell shape) to skill-gated CLIs.
# The intent is orthogonal to the FIRST_WORD block below — this catches
# `sh -c "gws ..."`, `eval "gws ..."`, `/usr/bin/gws ...`, and semicolon /
# pipe / substitution chains. Fail-closed: on ambiguity, refuse.
if printf '%s' "$COMMAND" | grep -Eq '(^|[[:space:]|;&`$(])(gws|gh)([[:space:]]|$)|/gws[[:space:]]|/gh[[:space:]]'; then
    echo "Error: shell-exec refuses commands that route to skill-gated CLIs (gws, gh). Use the dedicated run_gws or run_gh skill instead." >&2
    exit 1
fi
# --- end shell-exec L3 hardening ---
```

**Coverage matrix:**
- `gws gmail ...` — first-word match (existing behavior preserved).
- `sh -c "gws gmail ..."` — regex catches `[[:space:]]gws[[:space:]]` inside the quoted substring (grep operates on the raw command string as a byte sequence — quotes don't hide the substring from a lexical scan).
- `; gws ...` / `| gws ...` / `` ` gws ` `` / `$( gws ...)` — leading-boundary alternation `[[:space:]|;&`$(]` catches statement/substitution separators.
- `/usr/bin/gws ...` — final alternation `/gws[[:space:]]` catches path-prefixed absolute invocations.
- Newlines in `$COMMAND` — grep's `-E` operates line-by-line by default; if the command contains a newline where the bypass sits on line 2, the scan still fires on line 2 because each line is scanned. Multi-statement newline delimiter is handled by the `^` alternation in the regex.

**Deliberate false-positives:**
- `echo "hello gws"` (an operator wanting to print the literal string `gws`) — rejected. Acceptable: shell-exec is for command execution, not for pretty-printing skill names. Operators wanting to echo the literal can use `printf` with a variable or `\g\w\s` escapes.
- `#gws is my favorite skill` (comment string) — rejected if the `#` is not at the start of the command (a leading `#` in `$COMMAND` would be treated as command-name by `eval` anyway, so this is a genuine sub-case).

**Deliberate coverage gaps (documented, not fixed):**
- `env PATH=/tmp:$PATH gws-alias gmail ...` where `gws-alias` is a shell alias or renamed binary — not covered. The scan looks for the literal tokens `gws` / `gh`; renaming defeats it. Mitigation: L2 registry-ban + L4 execute-time guard from PR#1956 still fire on the actual gws tool call. This is a defense-in-depth layer, not a sole gate.
- Base64-encoded or otherwise obfuscated command payloads (`echo Z3dzIGdtYWls... | base64 -d | sh`) — not covered. Same mitigation: L4 is the last-mile guard at the tool-execute call site.
- `curl -X POST https://gmail.googleapis.com/...` — direct API call bypassing the gws CLI. Not covered by any L1-L4 layer; requires the doctrine's L4 to be extended to raw-HTTP surfaces (an MCP `data_grade` follow-up per mika#1959). Explicitly out of scope for this ticket.

### Tier 3 — Regression tests

**File:** `crates/mika-agent/tests/shell_exec_l3_hardening.rs` (new).

Six tests, one per bypass shape from § What Tier 2:

```rust
#[test]
fn shell_exec_rejects_sh_c_gws() {
    // Run run.sh via std::process::Command with COMMAND="sh -c 'gws gmail messages list'"
    let output = run_handler(r#"sh -c 'gws gmail messages list'"#);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("shell-exec refuses"));
}

#[test]
fn shell_exec_rejects_bash_c_gws() { /* ... */ }

#[test]
fn shell_exec_rejects_piped_echo_sh() {
    // COMMAND="echo 'gws gmail' | sh"
}

#[test]
fn shell_exec_rejects_absolute_path_gws() {
    // COMMAND="/usr/bin/gws gmail messages list"
}

#[test]
fn shell_exec_rejects_statement_separator() {
    // COMMAND="pwd; gws gmail messages list"
}

#[test]
fn shell_exec_rejects_command_substitution() {
    // COMMAND="pwd; $(gws gmail messages list)"
}

#[test]
fn shell_exec_allows_unrelated_commands() {
    // COMMAND="ls -la" — must succeed (false-positive protection)
    // COMMAND="grep gws foo.txt" — REJECTED per plan's deliberate false-positive doc
}

#[test]
fn shell_exec_first_word_gws_still_blocked() {
    // Regression: existing block still fires
    // COMMAND="gws gmail messages list"
}
```

**Test harness:** each test runs the actual `handlers/run.sh` shell script via `std::process::Command`, feeds `$COMMAND` via stdin JSON, asserts stderr / exit code.

**Also add** one identity-template test: `test_default_agent_skill_allowlist_excludes_shell_exec` in `crates/mika-common/src/home.rs` `#[cfg(test)] mod tests` — asserts `shell-exec` NOT in `DEFAULT_AGENT_SKILL_ALLOWLIST`. Regression guard against re-adding.

### Tier 4 — Doctrine doc update

**File:** `crates/mika-agent/docs/non-transit-data-grade.md` (created by PR#1956).

**Change:** Move F3 out of the "Known bypass classes" § into the "Applied hardening" § with a citation to this ticket. Requires this ticket to land AFTER PR#1956 (see § Dependency).

## Dependency on PR#1956

PR#1956 (mika#1798) is currently OPEN. This ticket's Tier 4 doc update requires the doctrine doc file to exist. Two paths:

**Path A (recommended):** ship this ticket AFTER PR#1956 merges. The `> **Companion PR:**` callout is NOT applicable here because this ticket's *implementation* (shell-exec run.sh + allowlist edit + tests) doesn't depend on PR#1956's code; only the *doc update* does. Split the delivery: land Tier 1-3 first as a merge-clean-against-main patch; Tier 4 as a follow-up commit once PR#1956 merges.

**Path B (parallel):** ship Tier 4 as part of PR#1956's own doc changes (Vincent's call — since PR#1956 is under Vincent's disposition, adding a paragraph is trivial).

Plan commits to Path A. If Vincent prefers B, the plan's Tier 4 becomes a suggestion for PR#1956, not a change for this ticket.

## Acceptance Criteria (derived from ticket body — mapped to changes)

Ticket body has no explicit `## AC` section; the "Scope" line ("Harden `shell-exec` L3 surface OR remove from default agent registry OR gate via classifier") enumerates three approaches. This plan **combines the first two** (harden + remove-from-default) as coordinated defense-in-depth. Deriving explicit AC per `mika-arch-groom-ticket` § Acceptance-Criteria Gate:

- **AC1:** `shell-exec` removed from `DEFAULT_AGENT_SKILL_ALLOWLIST` (`crates/mika-common/src/home.rs:367`) and mirror Python template block. Verified by `test_default_agent_skill_allowlist_excludes_shell_exec`.
- **AC2:** `handlers/run.sh` command-string scan catches all six bypass shapes from § What Tier 2 (sh -c, bash -c, pipe/echo, absolute path, statement separator, command substitution).
- **AC3:** Regression test suite in `crates/mika-agent/tests/shell_exec_l3_hardening.rs` covers all six shapes + first-word preservation + false-positive-guarded happy-path (`ls -la` succeeds).
- **AC4:** Well-known agents that need shell-exec (mika-dev, mika-arch) explicitly retain it in their identity template's `[skills].allowlist` — verified per-agent by `grep -n shell-exec crates/mika-common/src/home.rs`.
- **AC5:** Doctrine doc (once PR#1956 lands) reflects F3 as applied hardening rather than known bypass. Follow-up commit after PR#1956 merge.

## Definition of Done

- [ ] `crates/mika-common/src/home.rs`: `DEFAULT_AGENT_SKILL_ALLOWLIST` no longer contains `"shell-exec"` (both Rust const + Python template block).
- [ ] `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh` includes the L3 hardening regex block per § Tier 2.
- [ ] `crates/mika-agent/tests/shell_exec_l3_hardening.rs` — 8 tests covering all six bypass shapes + first-word regression + happy-path.
- [ ] `crates/mika-common/src/home.rs` `#[cfg(test)] mod tests`: `test_default_agent_skill_allowlist_excludes_shell_exec` added.
- [ ] Well-known agents that need shell-exec (mika-dev, mika-arch, any others per grep) explicitly retain it in `[skills].allowlist`.
- [ ] `cargo test --workspace` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all --check` clean.
- [ ] `make verify-bundled-skills` clean.
- [ ] PR body notes: (a) coordination note with PR#1956 (§ Dependency), (b) deliberately-preserved false-positives (`grep gws foo.txt` rejected), (c) documented coverage gaps (base64, curl direct-API).

## Injection verification (per `feedback_verify_pipeline_passes_without_the_fix`)

Three inversions:

1. **Tier 1 fires** — temporarily re-add `"shell-exec"` to `DEFAULT_AGENT_SKILL_ALLOWLIST`; verify `test_default_agent_skill_allowlist_excludes_shell_exec` fails; restore.
2. **Tier 2 catches sh -c** — temporarily remove the L3 hardening regex block from `run.sh`; verify `shell_exec_rejects_sh_c_gws` fails (bypass succeeds); restore.
3. **Tier 2 catches path-prefix** — temporarily remove the `/gws[[:space:]]` alternation from the regex; verify `shell_exec_rejects_absolute_path_gws` fails; restore.

Document in `todos/1957-injection-verification.md`.

## Out of scope

- **MCP L4 gap** (F5 from mika#1798, tracked as mika#1959) — separate ticket for the manifest `data_grade` field.
- **Direct raw-HTTP API surfaces** (`curl -X POST https://gmail.googleapis.com/...`) — no L1-L4 layer covers this; separate hardening class if a real bypass emerges.
- **Base64/obfuscated payload detection** — arms-race territory; not a productive gate at the shell-exec layer.
- **Alias / renamed-binary bypass** (`gws-alias` symlink) — same class, mitigated by L2+L4 from PR#1956.
- **Rulebook-level ban on shell-exec across the platform** — this ticket narrows the *default* allowlist; operator-tier explicit inclusion remains policy-allowed. A stricter "never ship shell-exec" is a separate operator-decision surface.

## Risks and mitigations

- **Regex false-positives break legitimate operator flows** — e.g., `grep gws-config /etc/services` gets rejected. Mitigation: the plan explicitly acknowledges this as deliberate — shell-exec is a narrow tool; if operators need to grep for `gws` literally, they escape to a non-shell-exec channel (file-reader, tmux). The tradeoff favors safety over convenience for a low-frequency false-positive.
- **Dependency on PR#1956 for doc update** — mitigated by Path A: ship Tier 1-3 as merge-clean-against-main; defer Tier 4 as a follow-up commit. This ticket does NOT block on PR#1956 for Tier 1-3.
- **Removing shell-exec from default breaks personal-tier agents that rely on it** — operators who provisioned a personal-tier agent expecting shell reach must add it explicitly. Documented in `docs/solutions/best-practices/shell-exec-default-removal-2026-08-23.md` (compound doc from this PR): one-line `[skills].allowlist` addition in their agent's `identity.toml`.

## Related solutions

- `crates/mika-agent/docs/non-transit-data-grade.md` (once PR#1956 lands) — § Known bypass classes lists F3 pre-hardening; this ticket moves it to § Applied hardening.
- `feedback_prompt_enforcement_fragile` — the founding memory that shell-exec-prompt-only rejection is empirically insufficient.
- `feedback_structural_gate_audit_grep_all_callsites` — this plan's Tier 1 requires grep across `home.rs` identity templates to catch all mirror sites.

## Compounding potential

After merge:

- **Shell-command-string scanning as a defense pattern** (~40 lines): the regex shape (boundary alternation + path-prefix alternation) is reusable for any future `run.sh`-style handler that needs to block routing to gated CLIs. Compound doc naming this pattern lets future skill authors copy the shape without re-deriving.
- **Default-allowlist narrowing as a structural gate**: the pattern of moving a permissive tool from `DEFAULT_AGENT_SKILL_ALLOWLIST` to explicit-per-agent inclusion is a general lever — file a compound note if another tool (browser-control, desktop) faces the same class of overreach.
