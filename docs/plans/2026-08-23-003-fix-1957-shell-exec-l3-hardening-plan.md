# Plan — fix(shell-exec): CLI bypass hardening — L3 gate escape via shell-exec

**Status:** READY (architect-validated — R1 applied 2026-08-27)
**Date:** 2026-08-23
**Ticket:** mika#1957
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** Non-transit data-grade doctrine follow-up (F3 from mika#1798 adversarial review)
**Cross-refs:** mika#1798 (parent doctrine bake), PR#1956 (MERGED 2026-08-23), mika#1641 (orchestrator seat — see C1), mika#1778 (family tier), `feedback_prompt_enforcement_fragile`

## Corrections post-grooming (2026-08-27)

Found at implementation time, routed to mika-arch on session `bbbff7b5-9b49-4653-b5b7-0436289bf6ba`.
Architect disposition: **READY with R1** — Tier 1 / AC1 struck, Tier 2+3+4 ship.

### C1 — Tier 1 / AC1 REMOVED (architect-ratified overturn)

The Tier 1 rationale was false against the code. Evidence:

- `crates/mika-common/src/home.rs:13-16` — `AgentTier::Default` is documented as the
  **"Operator/platform-owner persona … full skill allowlist including `github`/`git-ops`/`shell-exec`"**.
  `DEFAULT_AGENT_SKILL_ALLOWLIST` is therefore the *operator/orchestrator* tier, not a personal tier.
- `crates/mika-common/src/home.rs:461-468` — `FAMILY_AGENT_SKILL_ALLOWLIST` (mika#1778) already excludes
  `shell-exec`; `home.rs:1096-1112` is an existing test asserting that exclusion. The population mika#1798
  was designed to protect is **already protected on `main`** — AC1 would have added nothing there.
- `crates/mika-common/src/home.rs:377-382` — the shipped orchestrator seat (mika#1641) names
  `shell-exec` as load-bearing: *"`git-ops`, `shell-exec`, `tmux`, and `file-reader` above already cover
  the rest of the orchestrator tool surface."* Verified present at the grooming commit `cf027697`, so the
  plan simply missed it. Executing AC1 would have regressed mika#1641.
- The plan's "preserve for mika-dev" escape does not apply: `mika-dev`/`mika-qa`/`mika-arch` are
  well-known agents with their own allowlists (`crates/mika-agent/src/well_known_agents.rs:154,232`),
  untouched by `DEFAULT_AGENT_SKILL_ALLOWLIST`. Orchestrator **Mika herself** runs on `DEFAULT_IDENTITY`
  (`home.rs:268`).

**Consequence:** Tier 1 is dropped. AC1 and the `test_default_agent_skill_allowlist_excludes_shell_exec`
guard are struck from the Definition of Done. AC4 downgrades to verification-only (no code change).
Injection-verification inversion 1 is dropped.

**Residual risk deliberately left open:** `AgentTier::Default` covers two populations under one template —
the orchestrator seat (needs shell reach) and a per-customer container agent provisioned *without*
`MIKA_AGENT_TIER=family` (arguably should not). Splitting them is a new architectural surface (tier enum +
bootstrap + provisioning contract + mika-cloud chart) and is **out of scope here**, per architect. Tier 2
closes F3 for *both* populations regardless of allowlist, which is why R1 is safe.

### C2 — Tier 2 regex corrected (mechanism fix, AC2 unchanged)

The regex written in § Tier 2 below does **not** satisfy AC2. Measured against the six bypass shapes plus
seven false-positive guards, the plan's literal regex fails 4 of 11 block cases:

| Command | Plan regex | Corrected regex |
|---|---|---|
| `sh -c 'gws gmail messages list'` | **PASS (bypass!)** | BLOCK |
| `bash -c "gws gmail messages list"` | **PASS (bypass!)** | BLOCK |
| `echo 'gws gmail' \| sh` | **PASS (bypass!)** | BLOCK |
| `eval "gws gmail messages list"` | **PASS (bypass!)** | BLOCK |
| `gws gmail messages list` | BLOCK | BLOCK |
| `/usr/bin/gws gmail messages list` | BLOCK | BLOCK |
| `pwd; gws gmail messages list` | BLOCK | BLOCK |
| `pwd; $(gws gmail messages list)` | BLOCK | BLOCK |

**Cause:** the plan's leading boundary class `[[:space:]|;&`$(]` omits the quote characters `'` and `"`.
In every `sh -c '...'` shape the character immediately preceding `gws` is a quote, so the alternation never
fires — precisely bypass shape 1, the shape the ticket exists to close.

**Corrected regex (implemented):**

```
(^|[^A-Za-z0-9_.-])(gws|gh)([^A-Za-z0-9_.-]|$)
```

Boundary = "any character that cannot be part of a command identifier", which subsumes whitespace, quotes,
`;`, `|`, `&`, `` ` ``, `$`, `(`, and `/`. The separate `/gws[[:space:]]` alternation becomes unnecessary
(`/` is a non-identifier char). Excluding `.` and `-` from the boundary preserves the false-positive guards:
`.github/workflows/ci.yml`, `git push origin gh-pages`, `/tmp/gws.log`, and `echo highlight` all still pass.
Measured 18/18 on the case matrix.

### C3 — § Dependency on PR#1956 resolved

PR#1956 (mika#1798) **MERGED 2026-08-23**. `crates/mika-agent/docs/non-transit-data-grade.md` exists on
`main`. The Path A split (Tier 1-3 now, Tier 4 as follow-up) is obsolete — **Tier 4 ships in this PR.**

### C4 — Line-number drift

Plan cites `home.rs:367` / `:402`; actual is `:372` (Rust const) / `:410` (TOML mirror). Moot for the code
change now that Tier 1 is dropped, but the citations above are corrected.
`run.sh:27-32` still holds (`FIRST_WORD` block at lines 28-32).

### C5 — Tier 4 gains a doc correction

`crates/mika-agent/docs/non-transit-data-grade.md:243-245` currently asserts *"Personal-tier agents ship
with `shell-exec` in `DEFAULT_AGENT_SKILL_ALLOWLIST`"* — the same tier conflation as C1. Tier 4 corrects
that sentence in addition to moving F3 from § Known bypass classes to § Applied hardening.


## Why

PR#1956 (mika#1798) ships a four-layer structural bake of the non-transit data-grade doctrine (L1 prompt, L2 registry ban, L3 gws command validation, L4 execute-time guard). Adversarial review surfaced F3: **`shell-exec` bypasses L3 because it has its own security model** — `run_shell 'sh -c "gws gmail messages list"'` reaches the Google Workspace CLI via a subshell whose FIRST_WORD (`sh`) never trips the L3 gws validator.

Verified against current `main` state:
- `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh:27-32` — the current block-list is FIRST-WORD-only: `case "$FIRST_WORD" in gws) reject ;; gh) reject ;; esac`.
- shell-exec IS in `DEFAULT_AGENT_SKILL_ALLOWLIST` (`crates/mika-common/src/home.rs:372`) — the surface is live for every agent on `AgentTier::Default`. **Corrected by C1:** that tier is the *operator/orchestrator* persona (mika#1641), not a personal tier; the family tier (mika#1778) already excludes shell-exec.
- Bypass shape 1: `sh -c "gws gmail ..."` — first_word=`sh`, gates skip, subprocess spawns gws.
- Bypass shape 2: `bash -c "gws gmail ..."`, `zsh -c ...`, `eval "gws gmail ..."`.
- Bypass shape 3: pipe/redirect — `echo 'gws gmail ...' | sh`, `printf ... | bash`.
- Bypass shape 4: path-prefixed — `/usr/bin/gws gmail ...` — first_word=`/usr/bin/gws` (not `gws`).
- Bypass shape 5: command substitution — `` `gws gmail ...` `` or `$(gws gmail ...)`.
- Bypass shape 6: statement separators — `pwd; gws gmail ...`, `true && gws gmail ...`, newlines.

Prompt-only enforcement of "don't call gws through shell" is empirically fragile (per `feedback_prompt_enforcement_fragile` — the doctrine-bake ticket itself demonstrates n≥3 substrate hits requiring structural gates). This ticket adds the structural analog for the shell-exec surface.

## What

**Post-C1 shape:** Tier 1 is struck. The shipped work is Tier 2 (the universal structural gate inside `shell-exec` itself), Tier 3 (regression tests), and Tier 4 (doctrine doc). Tier 2 fires regardless of which allowlist granted `shell-exec`, so it closes F3 for every tier — which is why dropping Tier 1 costs no coverage.

### ~~Tier 1 — Remove `shell-exec` from `DEFAULT_AGENT_SKILL_ALLOWLIST`~~ — STRUCK (see § Corrections C1)

> **Not implemented.** Architect-ratified removal on 2026-08-27: `DEFAULT_AGENT_SKILL_ALLOWLIST` is the operator/orchestrator tier (mika#1641 depends on `shell-exec` being there), and the family tier it was meant to protect already excludes `shell-exec` (mika#1778, test at `home.rs:1096-1112`). The original text is preserved below for the record.

<details>
<summary>Original Tier 1 text (not implemented)</summary>

**File:** `crates/mika-common/src/home.rs:367` (and mirrored in the Python-format identity template just below — line 402).

**Change:** Delete the `"shell-exec"` entry from both `DEFAULT_AGENT_SKILL_ALLOWLIST` and its Python-format sibling. Also delete from `MIKA_DEV_IDENTITY`, `MIKA_ARCH`-computed identity, and other well-known-agent identity templates that don't need shell execution — verify per-agent via `grep -n '"shell-exec"' crates/mika-common/src/home.rs`.

**Preserve for:** operator-tier agents that genuinely need shell reach (e.g., `mika-dev` which dispatches claude-pilot subprocesses via shell). Explicit per-agent inclusion, not global default.

**Rationale (review-guide.md § YAGNI + Orthogonality):** personal-tier agents (the family-tier being that mika#1798 was designed to protect) never need arbitrary shell execution. The default allowlist was permissive by inheritance from mika-dev's identity template. Narrowing the default aligns tier-of-agent with tier-of-tool.

**Backward-compat:** existing installed agents' `identity.toml` files are untouched — the template only affects newly-provisioned or `MIKA_DISABLE_AGENT_PROVISIONING=false` reprovisioned agents. Operators who intentionally want shell-exec on their personal-tier agent add it to their agent's `[skills].allowlist` — one-line explicit consent, not a silent default.

</details>

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
if printf '%s\n' "$COMMAND" | grep -Eq '(^|[^A-Za-z0-9_.-])(gws|gh)([^A-Za-z0-9_.-]|$)'; then
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

## Dependency on PR#1956 — RESOLVED

PR#1956 (mika#1798) **MERGED 2026-08-23**. `crates/mika-agent/docs/non-transit-data-grade.md` is on `main`,
and this branch is based on a `main` that contains it. The Path A / Path B split is obsolete: **Tier 4 ships
in this PR**, alongside the C5 correction to the doc's tier characterisation.

## Acceptance criteria

Ticket body has no explicit `## Acceptance criteria` section; the "Scope" line ("Harden `shell-exec` L3
surface OR remove from default agent registry OR gate via classifier") enumerates three approaches. The
groomed plan combined the first two; per architect resolution R1 (§ Corrections C1) only the **harden**
approach ships. Derived per `mika-arch-groom-ticket` § Acceptance-Criteria Gate:

- [ ] **AC1 — STRUCK (C1).** ~~`shell-exec` removed from `DEFAULT_AGENT_SKILL_ALLOWLIST`.~~ Not implemented;
      architect-ratified 2026-08-27. Satisfied vacuously — recorded here so the numbering stays stable
      against the grooming comment on mika#1957.
- [ ] **AC2** — `handlers/run.sh` command-string scan catches all six bypass shapes from § What Tier 2
      (`sh -c`, `bash -c`/`eval`, pipe-into-shell, absolute path, statement separator, command substitution),
      using the C2-corrected regex.
- [ ] **AC3** — regression suite in `crates/mika-agent/tests/shell_exec_l3_hardening.rs` covers all six
      shapes + first-word-`gws` regression + first-word-`gh` regression + false-positive-guarded happy paths
      (`ls -la`, `.github/...`, `gh-pages`, `echo highlight` all succeed).
- [ ] **AC4 — verification-only (C1)** — well-known agents (`mika-dev`, `mika-qa`, `mika-arch`) and the
      Default tier keep `shell-exec`; no code change. Verified by `grep -n shell-exec
      crates/mika-common/src/home.rs crates/mika-agent/src/well_known_agents.rs` and by the existing
      `home.rs` sync tests still passing.
- [ ] **AC5** — `crates/mika-agent/docs/non-transit-data-grade.md` moves F3 from § Known bypass classes to
      § Applied hardening, and the "personal-tier agents ship with `shell-exec`" sentence is corrected to
      name the operator/orchestrator tier (C5). `docs/` and the crate-local mirror stay in sync per the
      repo's `docs-sync` CI job.

## Definition of Done

- [ ] `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh` includes the L3 hardening regex block per § Tier 2 (C2-corrected regex).
- [ ] `crates/mika-agent/tests/shell_exec_l3_hardening.rs` — tests covering all six bypass shapes + first-word regressions + false-positive happy paths.
- [ ] `crates/mika-agent/docs/non-transit-data-grade.md` + `docs/` mirror: F3 moved to § Applied hardening, tier characterisation corrected (C5).
- [ ] `crates/mika-common/src/home.rs` UNCHANGED (C1) — `DEFAULT_AGENT_SKILL_ALLOWLIST` still contains `"shell-exec"`.
- [ ] `cargo test --workspace` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all --check` clean.
- [ ] `make verify-bundled-skills` clean.
- [ ] PR body notes: (a) Tier 1 struck per architect R1 with the mika#1641 reason (§ Corrections C1), (b) deliberately-accepted false-positives (`grep gws foo.txt` rejected), (c) documented coverage gaps (base64, renamed binary, curl direct-API), (d) residual two-population risk left open as a separate surface.

## Injection verification (per `feedback_verify_pipeline_passes_without_the_fix`)

Two inversions (inversion 1 dropped with Tier 1, per C1):

1. **Tier 2 catches `sh -c`** — temporarily remove the L3 hardening regex block from `run.sh`; verify `shell_exec_rejects_sh_c_gws` fails (bypass succeeds); restore.
2. **Tier 2 boundary class is load-bearing** — temporarily revert the regex to the plan's original `[[:space:]|;&\`$(]` leading class; verify `shell_exec_rejects_sh_c_gws`, `shell_exec_rejects_bash_c_gws`, `shell_exec_rejects_piped_echo_sh`, and `shell_exec_rejects_eval_gws` all fail; restore. This is the C2 finding, re-run as a guard.

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
