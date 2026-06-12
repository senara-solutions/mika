---
ticket: mika#593
branch: feat/593/tui-startup-hidden-internal-count
status: active
date: 2026-06-12
origin: https://github.com/senara-solutions/mika/issues/593
execution: code
---

# Plan: surface startup-hidden internal-message count in [N hidden] badge (mika#593)

## Problem frame

In inbox mode (default), the TUI footer shows `[N hidden]` to flag internal messages filtered from view. Today, that counter tracks internals arriving **during** the session — messages filtered out by the startup `load_recent_messages_filtered(20, true)` query are invisible to the user.

For an agent driving a claude-pilot dev loop, most recent messages are tagged `internal=1` (mika#570). At TUI launch, the 20-row startup load can return very few visible messages, and the user has no indication that filtering happened — looks like messages silently disappeared.

UX polish, safe to defer. The fix: seed `hidden_internal_count` at startup with the delta between total filtered internals and what was loaded.

## Approach choice (per first-pass groom)

**Option 2 wins** — extend `load_recent_messages_filtered` to return `(Vec<SessionMessage>, usize)` where the `usize` is the count of internal messages discarded in the same window. Rationale (per architect first-pass):
- One round trip vs two queries.
- DRY: filtering predicate stays co-located with counting; future filter changes don't need cross-query sync.
- The count is a byproduct of the filtering logic the function already performs.

Option 1 (separate count query) is rejected as duplicative.

## Call-site enumeration (verified at plan time)

Confirmed via `grep -rn load_recent_messages_filtered crates/`:

**Production call sites (3):**
- `crates/mika-cli/src/commands/chat.rs:570` — startup load (will seed hidden_internal_count)
- `crates/mika-cli/src/tui/commands/handlers.rs:1002` — `/inbox` toggle path
- `crates/mika-cli/src/tui/commands/handlers.rs:1244` — second `/inbox` toggle / message-reload path

**API surface (2):**
- `crates/mika-agent/src/db.rs:7140` — sync canonical (return type changes here)
- `crates/mika-agent/src/async_db.rs:860` — async wrapper (return type changes here)

**Internal callers / tests (4):**
- `crates/mika-agent/src/db.rs:7135` — `load_recent_messages` wrapper (calls with `exclude_internal=false`; will destructure to drop the count)
- `crates/mika-agent/src/db.rs:14700-14714` — 2 existing tests (`test_load_recent_messages_filtered_excludes_internal`) — update destructuring

No external (cross-repo) callers. Both `db.rs` and `async_db.rs` are internal to `mika-agent`.

## Sibling-interaction note (mika#773)

mika#773 (`--inbox` flag, in flight on branch `feat/773/mika-chat-inbox-flag`) touches the same TUI initialization path. Composition:
- If `--inbox` is set, TUI launches with `app.inbox_mode = false` (audit mode, no filtering at startup).
- In that case, `load_recent_messages_filtered(20, false)` is called at startup → no internals filtered → `hidden_internal_count` seeded to 0. Correct UX.
- If `--inbox` is unset (default), `app.inbox_mode = true` → filtering happens → `hidden_internal_count` seeded with discarded count. Correct UX.

The two features compose without coupling. Whichever PR merges first, the second rebases trivially (no overlapping line edits in `App::new`).

## Scope boundaries

- Extend `load_recent_messages_filtered` return type to `(Vec<SessionMessage>, usize)`.
- Update 3 production call sites + 1 internal caller + 2 tests.
- Seed `App::new` (and `App::new_team`)'s `hidden_internal_count` from the startup load.
- **Out of scope:** post-startup re-seeding on `/inbox` toggle (current behavior keeps the counter scoped to "messages hidden since this toggle"; revisiting is a follow-up); rendering changes to the badge itself; filtering predicate changes.

## Implementation Units

### U1 — Extend DB return type

**Goal:** `load_recent_messages_filtered` returns `(Vec<SessionMessage>, usize)` — the `usize` is the count of internal rows filtered out within the requested window.

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (around line 7140 — `pub fn load_recent_messages_filtered`)
- Modify: `crates/mika-agent/src/async_db.rs` (around line 860 — async wrapper)

**Approach:** The sync canonical at `db.rs:7140` currently filters by SQL. Two implementation shapes work:

1. **SQL: two-aggregate query** — `SELECT messages..., (SELECT COUNT(*) FROM messages WHERE agent_id = ?1 AND internal = 1 AND ... <same window>) AS hidden_count`. Keeps the work in a single round trip but the COUNT subquery is over the same window predicate.
2. **Application-level counting** — change the SELECT to NOT apply `exclude_internal` at the SQL layer, return all matching rows, then partition in Rust: visible rows into `Vec<SessionMessage>`, count discarded internals as `hidden`. Single round trip, no subquery, but loads slightly more data.

**Recommended: Option 2 (application-level counting).** It's simpler, keeps the SQL surface stable, and the row delta is small (20 rows for the startup load). Filter `exclude_internal && row.internal == 1` rows into a counter; everything else into the returned Vec.

When `exclude_internal == false`, the count is always 0 (nothing was hidden by this call). When `exclude_internal == true`, the count is the number of `internal == 1` rows discarded from the same windowed selection.

The async wrapper at `async_db.rs:860` just forwards the new return type.

**Test scenarios:**
- **Happy path filtered:** session with 10 internal + 10 visible messages; `load_recent_messages_filtered(agent, 20, true)` returns `(10 visible, 10)`.
- **Happy path unfiltered:** same session; `load_recent_messages_filtered(agent, 20, false)` returns `(20 messages, 0)`.
- **Empty:** no messages; returns `(vec![], 0)` for both modes.
- **Limit smaller than total visible:** 30 visible + 5 internal, limit 20 — returns `(20 visible, 5)`. Limit applies to the visible set; internals within the windowed selection ALL count toward `hidden`.

**Important: window-vs-limit semantics.** The current function selects up to `limit` matching rows. With Option 2, the SQL selection must return enough rows to cover both `limit` visible AND any filtered internals within the same chronological window. Two acceptable shapes:
1. Bound the SQL by `limit` only, count internals within that bound (cheapest, but may undercount if the limit cap is hit before all internals are seen).
2. Use a larger SQL window (e.g., `LIMIT limit * 2` or no internal LIMIT, with application-side truncation) to guarantee accurate counting up to the visible window's chronological edge.

Recommendation: choose the simpler shape (1) and document the semantic in the doc comment ("count is best-effort; reflects internals discarded from the limit-bound window"). Operator UX value comes from "is the count > 0", not from precision.

**Verification:** all 2 existing tests pass after destructure update; 2 new tests above pass; `cargo test -p mika-agent db::tests` clean.

### U2 — Update internal `db.rs` caller and tests

**Goal:** Internal wrapper and existing tests destructure the new return type.

**Files:**
- Modify: `crates/mika-agent/src/db.rs:7135` (`load_recent_messages` wrapper — drop the count: `let (msgs, _) = ...; msgs`)
- Modify: `crates/mika-agent/src/db.rs:14700-14714` (two existing tests — destructure or use `.0`)

**Approach:** Pure mechanical destructure updates. No behavior change.

**Test scenarios:** existing tests pass after destructure.

### U3 — Thread the count through `chat.rs` startup to `App::new`

**Goal:** Seed `hidden_internal_count` at App construction with the startup-load hidden count.

**Files:**
- Modify: `crates/mika-cli/src/commands/chat.rs` (around line 570 — startup load + App construction site at ~497-508 per issue body)
- Modify: `crates/mika-cli/src/tui/app.rs` (`App::new` at ~647 and `App::new_team` at ~723 — add `initial_hidden_count: usize` parameter)

**Approach:**

1. Change the startup load destructure to capture the count:
   ```rust
   let (messages, hidden_at_startup) = db
       .load_recent_messages_filtered(agent_id.clone(), 20, /* exclude_internal */ true)
       .await?;
   ```
2. Pass `hidden_at_startup` to `App::new` as a new parameter (last position to minimize churn).
3. In `App::new`, replace `hidden_internal_count: 0` with `hidden_internal_count: initial_hidden_count`.

**Constraint:** if mika#773 (`--inbox` flag) lands first, the startup load will use `app.inbox_mode = !args.inbox`. When inbox mode is false (audit), the second param of `load_recent_messages_filtered` is false → no filtering → count is 0 (current load_recent_messages_filtered call shape `_filtered(20, app.inbox_mode)` already passes the right value). No code change needed for the composition.

**`App::new_team`:** team mode doesn't load chat history at startup the same way — team runs use their own message-event stream. Likely safe to pass 0 for `initial_hidden_count`. Plan implementer verifies and documents.

**Test scenarios:**
- **Happy path:** startup-filtered session shows correct `[N hidden]` badge value > 0 at first render.
- **Default behavior preserved:** no internal messages in startup load → `[0 hidden]` badge does NOT render (existing ui.rs:1056 gate).
- **Unfiltered startup:** if startup runs with `exclude_internal=false` (e.g., #773 audit-mode launch), `initial_hidden_count == 0`, badge does not render.

**Verification:** smoke test post-build: launch `mika chat --agent mika-dev` (an agent with heavy internal traffic), observe `[N hidden]` shows a non-zero count immediately.

### U4 — Update handler call sites

**Goal:** `/inbox` toggle paths destructure the new return type.

**Files:**
- Modify: `crates/mika-cli/src/tui/commands/handlers.rs:1002` — destructure the tuple
- Modify: `crates/mika-cli/src/tui/commands/handlers.rs:1244` — same

**Approach:** Decide whether `/inbox` toggle re-seeds `hidden_internal_count` from the new load count, or keeps existing semantics (counter resets to 0 on toggle, accumulates only during the active inbox-mode session). Current behavior is the latter — see `handlers.rs:1017` which conditionally resets.

Recommendation: preserve current `/inbox` toggle semantics. Destructure-and-discard the count at both toggle sites:
```rust
let (messages, _hidden_count) = db.load_recent_messages_filtered(...).await?;
```

Re-seeding on toggle is a UX design decision separate from this ticket's scope (startup seeding). Leave as a follow-up if anyone asks.

**Test scenarios:** existing `/inbox` toggle behavior unchanged — manual smoke test confirms.

**Verification:** `cargo test -p mika-cli tui` passes; smoke test confirms toggle behavior unchanged.

### U5 — Docs update

**Goal:** `crates/mika-cli/CLAUDE.md` § TUI Features / Footer badges documents that `[N hidden]` now reflects startup-filtered + during-session counts.

**Files:**
- Modify: `crates/mika-cli/CLAUDE.md`

**Approach:** Update the footer badge bullet point. One-line addition: "Seeded at startup with the count of internals filtered from the initial message load (mika#593)."

**Verification:** manual read.

## Dependencies / sequencing

- U1 → U2 (U2 updates callers and tests for the U1 return type change)
- U1 → U3 → U4 (U3 and U4 are independent destructure updates after U1's signature change)
- U5 (docs) ships in the same PR; last

## Patterns to follow (cross-cutting)

- `crates/mika-agent/src/db.rs:7140` — existing fn signature and filter behavior
- `crates/mika-agent/src/async_db.rs:860` — wrapper pattern
- `crates/mika-cli/src/tui/ui.rs:1056` — existing badge render gate (`if app.inbox_mode && app.hidden_internal_count > 0`) — no change needed; ticket just makes `hidden_internal_count` honest at startup

## Verification (top-level)

- `cargo test -p mika-agent` passes (DB tests + existing filter tests)
- `cargo test -p mika-cli` passes
- `cargo clippy --workspace` clean
- `cargo fmt --all -- --check` clean
- Manual smoke test: heavy-internal-traffic agent (mika-dev) shows non-zero `[N hidden]` immediately at launch in default inbox mode

## Risk / known unknowns

- **Window-vs-limit semantics** — see U1's important note. The simpler shape may undercount if internal rows cluster densely; UX value still preserved (count > 0 is the signal). If precision becomes needed, follow-up ticket can switch to the expanded-window shape.
- **`App::new_team` count parameter** — team mode's message-history loading shape isn't fully audited in this plan. Implementer must verify whether team mode hits the same DB function at construction time; if it doesn't, the team-mode parameter is just a passthrough of 0.

## Out-of-scope (explicit)

- Re-seeding `hidden_internal_count` on `/inbox` toggle (separate UX call).
- Badge render gate or label change at `ui.rs:1056`.
- Filtering predicate changes (`messages.internal = 1` is the source of truth).
- Per-agent persistent state for hidden counts (overengineering for a UX badge).
