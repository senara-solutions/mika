---
ticket: mika#773
branch: feat/773/mika-chat-inbox-flag
status: active
date: 2026-06-12
origin: https://github.com/senara-solutions/mika/issues/773
execution: code
---

# Plan: `mika chat --inbox` flag (mika#773)

## Problem frame

`mika chat` always launches in default inbox mode (internal messages filtered from view). Toggling to audit mode (showing internals) requires typing `/inbox` after launch — friction for agents like `mika-relay` where the internal traffic IS the primary view.

## Naming clarification (code vs user-facing terminology)

There is a terminology mismatch between issue body and code:

- **Code:** `app.inbox_mode: bool` — `true` filters internal messages from view (default), `false` shows all messages. The `/inbox` slash command at `crates/mika-cli/src/tui/commands/handlers.rs:996` flips this boolean.
- **User-facing (issue body):** Vincent calls the audit-mode-with-internals view "inbox mode" because the user is watching the agent's internal-message inbox. The `--inbox` flag is meant to "enable inbox mode on TUI launch" — equivalent to typing `/inbox` after the default startup.

**Resolution:** the flag's semantic is "equivalent to typing `/inbox` once at launch" — it flips `inbox_mode` from the default `true` to `false`, so internal messages are visible. The naming inversion is preserved (don't rename the code field — out of scope) but documented inline at the construction site.

## Scope boundaries

- Add `--inbox` boolean flag to `ChatArgs` (`crates/mika-cli/src/cli.rs:149`).
- Thread the flag into `App::new` and `App::new_team` so initial `inbox_mode` is set to the flag's inverse.
- In-session `/inbox` toggle continues to work unchanged.
- **Out of scope:** rename `inbox_mode` → `audit_mode` (consistency churn, separate ticket if wanted), per-agent persistence (`identity.toml [tui]` config), session-restore semantics.

## Implementation Units

### U1 — CLI flag definition

**Goal:** Add `--inbox` to `mika chat` argument parsing.

**Files:**
- Modify: `crates/mika-cli/src/cli.rs` (`ChatArgs`, ~line 149)

**Approach:** Add a `#[arg(long)]` boolean field. Help text disambiguates: "Launch directly in audit mode (show internal/internal-to-internal messages). Equivalent to typing `/inbox` after launch. Default off."

```rust
#[derive(clap::Args)]
pub struct ChatArgs {
    // existing fields...

    /// Launch directly in audit mode (show internal messages, equivalent to typing `/inbox` after launch).
    /// Default off — chat opens in inbox mode (internal messages filtered).
    #[arg(long)]
    pub inbox: bool,
}
```

**Test scenarios:**
- **Happy path:** `mika chat --inbox` parses cleanly with `inbox: true`.
- **Default:** `mika chat` (no flag) parses with `inbox: false`.
- **Composition:** `mika chat --agent mika-relay --inbox` parses with both fields set; `mika chat --team my-team --inbox` parses with both set.

**Verification:** `cargo test -p mika-cli cli::tests` (or new test in the cli module) passes; `cargo build -p mika-cli` clean.

### U2 — Thread the flag into `App::new` and `App::new_team`

**Goal:** Initial `inbox_mode` honors the flag.

**Files:**
- Modify: `crates/mika-cli/src/tui/app.rs` (`App::new` at ~647 and `App::new_team` at ~723) — add `start_in_audit_mode: bool` parameter
- Modify: `crates/mika-cli/src/commands/chat.rs` — pass `args.inbox` through to both `App::new` callsites

**Approach:**

1. Add `start_in_audit_mode: bool` as the LAST parameter of `App::new` and `App::new_team` to minimize signature churn in callers that don't care.
2. At the `inbox_mode: true,` initializer (line 712 and line 801), replace with `inbox_mode: !start_in_audit_mode,` — when the flag is set, audit mode is on, so `inbox_mode` is false.
3. Update both call sites in `crates/mika-cli/src/commands/chat.rs` to pass `args.inbox` (with a `// audit mode launch (#773)` brief comment at the call site).

**Constraint:** `inbox_mode: true` is the only literal in `App::new` and `App::new_team` for this field today. The default behavior (flag absent → `inbox: false` → `inbox_mode: !false = true`) preserves current UX exactly.

**Test scenarios:**
- **Happy path:** `App::new(..., start_in_audit_mode: true)` produces an `App` with `inbox_mode == false`; the `[N hidden]` badge does NOT render at startup.
- **Default behavior preserved:** `App::new(..., start_in_audit_mode: false)` produces `inbox_mode == true`; existing behavior unchanged.
- **In-session toggle still works:** after construction with `start_in_audit_mode: true`, calling the `/inbox` handler at `handlers.rs:996` flips `inbox_mode` back to `true` (normal toggle behavior).

**Verification:** `cargo test -p mika-cli tui::app::tests` (or extend existing app construction tests) passes; the in-session `/inbox` toggle integration is exercised by the existing `handlers.rs` flow (no new test needed if construction-time test covers the initial state).

**Patterns to follow:**
- `crates/mika-cli/src/tui/app.rs:647-720` — existing `App::new` parameter ordering style (constructor params, not builder)
- `crates/mika-cli/src/commands/chat.rs:566` — existing pattern of reading `app.inbox_mode` for `load_recent_messages_filtered`

### U3 — Startup message-load respects the flag

**Goal:** The initial message-load query uses the launch-mode setting.

**Files:**
- Verify (likely zero changes): `crates/mika-cli/src/commands/chat.rs:566-570` — the existing call already reads `app.inbox_mode` AFTER `App::new` runs, so once U2 lands, this site naturally uses the flag-set value.

**Approach:** No code change expected. The call site `db.load_recent_messages_filtered(20, app.inbox_mode)` already uses the post-construction value. If profiling shows the initial load is called BEFORE the App is fully constructed (unlikely — the chat command builds the App and then runs its main loop), the load-call passes the App's `inbox_mode` value at that point.

**Test scenarios:**
- **Manual smoke test (post-build):** `mika chat --agent mika-relay --inbox` shows internal messages immediately at launch; `mika chat --agent mika-relay` (no flag) hides them (current behavior).

**Verification:** existing tests pass; smoke test confirms the user-visible behavior.

### U4 — Docs update

**Goal:** `crates/mika-cli/CLAUDE.md` § TUI Features documents the flag.

**Files:**
- Modify: `crates/mika-cli/CLAUDE.md` (TUI Features section, the `/inbox` and inbox-mode bullet)

**Approach:** Add a one-line note in the inbox-mode bullet:

> `/inbox` toggles between inbox mode (filtered) and audit mode (all messages visible). Reloads message history from DB on toggle. `--inbox` flag on `mika chat` launches directly in audit mode (equivalent to typing `/inbox` after launch).

**Verification:** doc-sync CI gate passes (this file lives outside the synced `docs/` tree, so no extra script needed); manual read confirms accuracy.

## Dependencies / sequencing

- U1 → U2 (U2 reads the flag U1 defines).
- U3 is a no-op verification step; can be done at the same time as U2.
- U4 (docs) ships in the same PR; can be authored last.

## Patterns to follow (cross-cutting)

- `crates/mika-cli/src/cli.rs` (existing `ChatArgs`) — clap `#[arg(long)]` style with help text on the doc comment
- `crates/mika-cli/src/tui/app.rs:617` — `pub inbox_mode: bool` field convention
- Existing `--model` flag plumbing through `ChatArgs` → chat command → TUI session — same shape as this work

## Verification (top-level)

- `cargo test -p mika-cli` passes
- `cargo clippy -p mika-cli` clean
- `cargo fmt --all -- --check` clean
- Manual smoke test: `mika chat --inbox` opens with internals visible (no `/inbox` toggle needed)

## Risk / known unknowns

- **Terminology drift.** The code field `inbox_mode` and the user-facing `--inbox` flag have inverse semantics. This is documented in the plan and at the construction-site comment; future readers will not be surprised. A separate ticket could rename `inbox_mode` → `filter_internals` or `audit_mode` for consistency — explicitly out of scope here.
- **Constructor signature churn.** `App::new` already has `#[allow(clippy::too_many_arguments)]`. Adding one more parameter does not breach a structural limit but does push the function further from "should be a builder." If a builder refactor is pursued separately (mika#X), this flag fits naturally. For now, additive param is the lowest-friction path.

## Out-of-scope (explicit)

- Persistent per-agent `inbox_on_launch` config in `identity.toml` (alternatives-considered in the issue body; deferred).
- Remembering last-toggle state across launches (alternatives-considered in the issue body; deferred).
- Renaming `inbox_mode` field for terminology consistency (separate concern).
