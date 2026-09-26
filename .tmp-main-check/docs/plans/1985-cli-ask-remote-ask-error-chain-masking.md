---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: mika#1985
branch: fix/1985/cli-ask-remote-ask-error-masking-hides
---

# fix(cli,ask): remote_ask error chain masking hides root cause of "failed to reach mika-spirit"

## Problem Frame

`mika ask --agent <name> "<msg>"` returns a **single** wrapper error — `failed to reach mika-spirit over A2A at <url> (is it running?)` — for every failure class the CLI encounters: 401 auth, connection refused, DNS failure, HTTP 500 from a server-side LLM provider error, invalid A2A state transition. The underlying `A2aError` message is **present in the anyhow chain** but never displayed on stdout because the CLI's outer error printer emits only the top context via `{err}` (not `{err:#}`).

The founding incident (2026-08-24, Prime OAuth substrate loss) cost ~2h of paired diag on a symptom whose root cause — `LLM provider error: OAuth token resolution failed. Run mika setup --mode oauth` — was invisible to the CLI operator and only surfaced via server-log grep. This is tier-2 substrate: it doesn't break the loop, but it silently taxes debugging time on every substrate incident. Every future incident re-pays the 2h cost until fixed.

## Requirements

- **R1** — `mika ask` failure output must surface the underlying `A2aError` message text (HTTP status, reqwest transport reason, JSON-RPC error, invalid state transition) rather than only the generic wrapper.
- **R2** — Regression coverage must assert the visible error message contains the underlying reason string, so the mask-through cannot silently regress.
- **R3** — The fix must be localized to the `commands/ask.rs` call site (Option A per issue) — no changes to `A2aError` variant semantics, no global anyhow display switch, no retry logic.

## Acceptance criteria

- [ ] **AC1** — `mika ask` failure output includes the underlying `A2aError` message (not just the generic wrapper)
- [ ] **AC2** — unit test covers the `A2aError::ClientError` mask-through
- [ ] **AC3** — manual verification post-fix — trigger a known server-side failure (e.g., stop mika-spirit, run `mika ask`) and verify the CLI output surfaces the underlying error variant

## Non-goals

- Not changing `anyhow` error type semantics globally (Option B — global `{err:#}` printer swap — filed separately if wanted).
- Not adding retry logic.
- Not touching `A2aError` variants themselves.
- Not touching the `--remote` dispatch path (`dispatch_remote` in `remote_ask.rs`) — that path already surfaces `A2aError` variant-specific prefixes and is a different call site.

## Key Technical Decisions

### KTD1 — Option A: `map_err` with alternate-format collapse at the CLI call site

Change the local wrapper at `crates/mika-cli/src/commands/ask.rs:318-322` from:

```rust
.with_context(|| {
    format!("failed to reach mika-spirit over A2A at {spirit_endpoint} (is it running?)")
})?
```

to:

```rust
.map_err(|e| anyhow::anyhow!("mika ask to {spirit_endpoint} failed: {e:#}"))?
```

**Why Option A over Option B:**
- **Localized blast radius:** only the specific masking site changes. The rest of the CLI's error surface stays byte-identical, so no other command's error output shape shifts.
- **`{e:#}` (alternate `Display`) flattens the anyhow chain in-line** — the visible message becomes `mika ask to http://localhost:8081/a2a/mika-prime failed: connection error: HTTP 500 Internal Server Error <preview>`, which contains both the endpoint context (still useful for "wrong URL" diagnosis) and the underlying reason (the load-bearing new content).
- **Preserves the actionable framing** — the caller-facing prefix still names "mika ask" and the endpoint, so operators reading a support log immediately see the operation. Only the trailing wrapper "(is it running?)" is dropped — that hint was actively misleading in the 2026-08-24 incident (spirit *was* running; OAuth had died).

**Trade-off accepted:** the layered anyhow chain structure (each layer as a separate `err.source()` link) is collapsed into a single flat message. `anyhow::Chain` walkers downstream (there are none in this codebase for CLI errors) would see one layer instead of two. Acceptable because the CLI's terminal consumer is human-readable stdout via `eprintln!("Error: {err}")`.

### KTD2 — Test the shape, not the exact string

The unit test asserts:
1. The visible error message contains the underlying reason token (e.g., "connection error"), proving the mask-through fixed.
2. The visible error message contains the endpoint URL, proving the actionable context survived.

Do NOT hard-code the exact "mika ask to <url> failed:" prefix — a future prose tweak (e.g., adding a hint about `--verbose`) should not require a test update. The mask-through invariant is the load-bearing property.

Test placement: inline `#[cfg(test)] mod tests` at the bottom of `crates/mika-cli/src/commands/ask.rs` (mika convention: tests colocated with source). The test constructs an `anyhow::Error` shaped like what `remote_ask::send_message_to_agent()` returns on `A2aError::ClientError` (i.e., `anyhow!("connection error: {}", inner)`), applies the same `.map_err(|e| anyhow!("mika ask to {} failed: {e:#}", url))` transform in isolation, and asserts the resulting `format!("{}", err)` output contains both `connection error` and the URL.

## Implementation Units

### U1. Apply `map_err` fix at the ask.rs call site

- **Goal:** Replace `.with_context(...)` at `crates/mika-cli/src/commands/ask.rs:318-322` with `.map_err(|e| anyhow::anyhow!("mika ask to {spirit_endpoint} failed: {e:#}"))` so the underlying `A2aError` message surfaces in the visible error.
- **Requirements:** R1, R3, AC1
- **Dependencies:** none
- **Files:**
  - `crates/mika-cli/src/commands/ask.rs` (modify lines 318-322)
- **Approach:** One-line semantic swap — `with_context` (adds new layer, only top layer visible via `{err}`) → `map_err` (replaces error with a single-layer message that has already flattened the chain via `{e:#}`). No import changes needed; `anyhow::anyhow!` is already in scope via the existing `use anyhow::...` at the top of the file (verify via `grep -n "use anyhow" crates/mika-cli/src/commands/ask.rs`).
- **Verification:** `cargo build -p mika-cli` succeeds. `cargo clippy -p mika-cli --all-targets -- -D warnings` clean.
- **Test scenarios:** covered by U2's regression test — this unit is the source change under test.

### U2. Add regression test for mask-through invariant

- **Goal:** Add a unit test asserting that a `connection error:`-shaped underlying error surfaces through the `map_err` wrapper on the visible `Display` output.
- **Requirements:** R2, AC2
- **Dependencies:** U1
- **Files:**
  - `crates/mika-cli/src/commands/ask.rs` (add to existing `#[cfg(test)] mod tests` block, or create one if missing)
- **Approach:** The test does NOT need to spin up a real A2A server. It constructs an `anyhow::Error` matching the shape `send_message_to_agent()` returns on `A2aError::ClientError` (`anyhow::anyhow!("connection error: HTTP 500 fake body")`), applies the same `.map_err` transform in isolation against a fixed endpoint URL, then asserts on the resulting `format!("{}", wrapped)` output:
  - Contains the substring `connection error` (proves mask-through).
  - Contains the substring `http://test.local/a2a/test-agent` (proves endpoint context survived).
- **Patterns to follow:** existing `#[cfg(test)] mod tests` blocks in `crates/mika-cli/src/commands/` (search `grep -rn "#\[cfg(test)\]" crates/mika-cli/src/commands/`). Use standard `#[test]` + `assert!(...contains(...))` shape; no external test-utility crate needed.
- **Test scenarios:**
  - **Happy path (mask-through):** underlying `connection error: HTTP 500 ...` shows in wrapped visible output.
  - **Endpoint context preserved:** the endpoint URL appears in the wrapped output.
- **Verification:** `cargo test -p mika-cli` includes the new test and it passes. Regression proof: reverting U1 alone (leaving U2 in place) should make the test **fail** — the test must actually be sensitive to the fix (see § Verification Contract).

## Verification Contract

- **V1** — `cargo build -p mika-cli` succeeds.
- **V2** — `cargo test -p mika-cli` passes, including the new test from U2.
- **V3** — `cargo clippy -p mika-cli --all-targets -- -D warnings` clean.
- **V4** — `cargo fmt --check` clean.
- **V5** — **Fix-in-diff sanity check** (per memory `feedback_verify_pipeline_passes_without_the_fix`): if U1's source change is reverted (git checkout of the pre-fix `ask.rs`) while U2's test remains, `cargo test -p mika-cli` **must fail** on the new test. This proves the test is actually load-bearing on the fix and not tautological.
- **V6** — Manual verification (AC3): with mika-spirit running against a broken provider (or stopped entirely), `mika ask --agent mika-prime "test"` visibly surfaces the underlying reason (connection refused, HTTP 500 body preview, OAuth token error, etc.) in stderr — not just the generic "failed to reach mika-spirit... is it running?" wrapper.

## Definition of Done

- [ ] Code compiles clean (`cargo build -p mika-cli`).
- [ ] Clippy clean (`cargo clippy -p mika-cli --all-targets -- -D warnings`).
- [ ] Formatted (`cargo fmt --check`).
- [ ] All acceptance criteria checked (AC1, AC2, AC3).
- [ ] Regression test passes (`cargo test -p mika-cli`).
- [ ] Fix-in-diff sanity check passes (V5).
- [ ] PR body includes verbatim `git diff --stat main..HEAD` output.
- [ ] PR body includes `Closes #1985`.
- [ ] PR body includes short HEAD SHA fingerprint.

## Scope Boundaries

**In scope:**
- The single call site at `crates/mika-cli/src/commands/ask.rs:318-322`.
- One regression test covering the mask-through invariant.

**Out of scope (deferred to follow-up work):**
- **Option B (global `{err:#}` printer swap in `main.rs`)** — separate ticket if the operator wants every CLI error path to auto-flatten chains. The 2026-08-24 incident evidence only justifies the localized fix.
- **`--verbose` flag adding full anyhow chain walk** — different UX pattern, separate scope.
- **Sibling `set_env_var` append-vs-replace hygiene** — already mentioned in issue as separately-filed OAuth doublon root cause; not this ticket.

## Sources & Research

- Issue: `senara-solutions/mika#1985`
- Founding incident: `/var/log/mika/server.log` grep on 2026-08-24 09-11:00 UTC (documented in issue body)
- Call site: `crates/mika-cli/src/commands/ask.rs:313-322` (endpoint construction + wrapped send)
- Underlying error origin: `crates/mika-cli/src/remote_ask.rs:127-141` (`send_message_to_agent` return type + `A2aError` variant mapping)
- `anyhow` alternate `Display` semantics: `{err:#}` prints top + all `source()` chain concatenated by `:`, whereas `{err}` prints only top layer
