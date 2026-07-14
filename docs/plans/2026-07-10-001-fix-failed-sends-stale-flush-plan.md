---
ticket: mika#1751
branch: fix/1751/agent-failed-sends-flush-delivers-stale
type: fix
scope: crates/mika-agent
grooming: /mika-groom-ticket
---

# Plan — mika#1751 stale `failed_sends` flush delivers unmarked duplicates

## Problem

Live observation (family-customer-1, 2026-07-09 15:36): a user's "Hello" received **two introductions** one minute apart. Sequence:

- t=12:00 — user sent "Hello". Mika composed a reply. Gateway `/send` failed (due to a since-fixed `customer_id` bug). The reply parked in `failed_sends`.
- t=15:36 — user re-sent "Hello". Mika composed a fresh reply AND the inbound handler simultaneously spawned `flush_failed_sends`, which delivered the 3.5h-old parked reply.
- Reader-side experience: two introductions to one greeting.

The flush mechanism is behaving as coded. The **policy** is wrong for conversational channels: (a) no age gate, (b) no marker on late deliveries, (c) no coordination between the fresh-turn response and the same-turn flush.

## Root cause

`flush_failed_sends` at `crates/mika-agent/src/server/handlers.rs:1096-1138`:

- Reads oldest-first (LIMIT 5), no age filter (`crates/mika-agent/src/db.rs:8520-8542`).
- Sends verbatim through `GatewayMessageSender`.
- Fires on every inbound message via a detached `tokio::spawn` (handlers.rs:361-369), racing the fresh-turn response spawn. No coordination.

`failed_sends` schema (crates/mika-agent/src/db.rs:1393-1400) has `created_at` (ISO-8601 UTC TEXT). Age is measurable without a migration.

The only INSERT site is `crates/mika-agent/src/messaging.rs:219` — reply-retry after a second gateway `/send` failure. **All rows currently in `failed_sends` are conversational replies.** Scheduled `send_message` tasks (task_engine/dispatcher.rs:129-164) share the sender but do not currently insert (they log-only on failure). This means we can apply a conversational-channel policy in v1 without a schema column for message class.

## Scope

### In scope for v1 (this PR)

**AC1 — Staleness threshold drops rows.** In `flush_failed_sends`, for each row read from `get_pending_failed_sends`, compare `now - created_at` to a threshold. If age exceeds `FAILED_SEND_STALE_THRESHOLD` (5 minutes), delete the row without sending and emit a `warn!` log with `agent_id`, `id`, `age_secs`, `retry_count`, and the first 80 chars of the parked text (to make the drop auditable, matching the existing sender-log style).

**AC2 — Delivered rows are marked.** Rows that pass the threshold gate are prefixed with `⏳ from earlier — ` before the send call. If the send succeeds, the row is deleted (unchanged). If the send fails, `increment_failed_send_retry` (unchanged). The prefix constant lives in `handlers.rs` co-located with `flush_failed_sends`.

**AC3 — Unit tests cover both paths.** In `crates/mika-agent/src/db.rs`'s `#[cfg(test)]` block:
- A test that inserts a row, ages `created_at` past the threshold via direct UPDATE, calls the flush helper, and asserts the row is deleted and no HTTP request was issued.
- A test that inserts a fresh row, calls the flush helper against a mock gateway, and asserts the sent text carries the prefix. If mocking `GatewayMessageSender` is prohibitive, factor the prefix-vs-drop decision into a pure helper `classify_failed_send(created_at, now) -> FlushAction` and unit-test the helper directly. See §Implementation guardrails below for the concrete branch policy.

**AC4 — Threshold and prefix are named constants.** Not runtime-configurable in v1 (per samidarko-claude's dispatch note: "propose ~5min in the PR"). Declared as `const FAILED_SEND_STALE_THRESHOLD: chrono::Duration = chrono::Duration::minutes(5)` (or `i64` seconds if `Duration` const-fn hasn't stabilized in the pinned toolchain — check at implementation time) and `const FAILED_SEND_STALE_PREFIX: &str = "⏳ from earlier — "`.

### Out of scope for v1 (deferred)

- **Same-turn dedup.** The ticket's third bullet ("consider consuming a flush triggered by the same turn that generates a fresh reply") is architectural: it requires either (a) making `flush_failed_sends` sequential-before the agent loop and injecting parked rows into LLM context (reverts #124's off-critical-path optimization — latency regression), (b) cross-spawn cancellation/coordination (race-prone, hard to test), or (c) the agent loop absorbs pending rows into its own context before composing a response (touches the reasoning loop; larger blast radius). All three are real architectural changes, not a ~120-line additive PR. AC1's threshold already resolves the observed "twice-hello" incident (a 3.5h-old row will drop). **Follow-up ticket to file alongside this plan** (per mika-arch first-pass §Uncertainty 1 recommendation): a coordination ticket that acknowledges the uncoordinated parallel-spawn at `handlers.rs:361-369` is a design smell introduced intentionally by #124 for performance, and any dedup solution must preserve that intent.
- **Message-class discrimination.** No `message_class` column added. Adding one is unnecessary until a scheduled/notification send starts using `failed_sends` AND we want a longer TTL for that class. File follow-up when the second class of caller appears.
- **Retry-count cap.** `increment_failed_send_retry` still increments unbounded. Independent bug, separate ticket: file **mika#new — cap `failed_sends.retry_count` and drop after N retries** if not already tracked.
- **Config knob for threshold.** The 5-minute constant is fine for v1. If tuning is needed post-merge, promote to a `SettingsSection`.

## Implementation guardrails

### File and function targets

| Change | File | Location |
|---|---|---|
| Add `const FAILED_SEND_STALE_THRESHOLD` | `crates/mika-agent/src/server/handlers.rs` | Above `flush_failed_sends` |
| Add `const FAILED_SEND_STALE_PREFIX` | `crates/mika-agent/src/server/handlers.rs` | Above `flush_failed_sends` |
| Introduce `classify_failed_send(created_at: &str, now: DateTime<Utc>) -> FlushAction` | `crates/mika-agent/src/server/handlers.rs` | Above `flush_failed_sends` |
| Modify `flush_failed_sends` to consult `classify_failed_send` and act | `crates/mika-agent/src/server/handlers.rs:1096-1138` | Replace the inner `for send in sends` block |
| Add `warn!` log for drops | Same function | Alongside the delete call |
| Add unit tests for `classify_failed_send` | `crates/mika-agent/src/server/handlers.rs` (test mod) or new sibling test file | `#[cfg(test)] mod tests` |
| Update runtime-structure docs | `crates/mika-agent/docs/runtime-structure.md:171` | One-sentence note on staleness gate + prefix |
| Update architecture docs | `crates/mika-agent/docs/architecture.md:840,982` | Reference the policy change |

### `FlushAction` enum (pure decision)

```rust
enum FlushAction {
    Drop,                              // age > threshold, delete without sending
    Deliver { prefix: &'static str },  // send with prefix; `prefix` names which marker to use
}
```

Two prefix values are used at call sites:

- `FAILED_SEND_STALE_PREFIX` — `"⏳ from earlier — "` — the normal case (row was within threshold but had been parked).
- `FAILED_SEND_UNPARSEABLE_PREFIX` — `"⚠️ UNPARSEABLE TIMESTAMP — "` — the parse-failure fallback (see below).

**Prefix policy note.** V1 prefixes ALL rows that reach the deliver arm. Rationale: the row parked at all, so ordering is broken by definition — reader benefits from the marker. Simpler than a second sub-threshold (e.g., "prefix only if age > 30s"). If review prefers a sub-threshold, add a second constant `FAILED_SEND_MARK_THRESHOLD = Duration::seconds(30)` and gate the prefix on `age > MARK_THRESHOLD`. Prefer simplicity until we see a case where an in-threshold, delivered-quickly row's prefix confuses the reader.

### Timestamp parsing (fail-open policy)

`created_at` is stored as `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')` (`crates/mika-agent/src/db.rs:1400`). Parse with `chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%dT%H:%M:%SZ")` and `.and_utc()`. On parse failure:

- Return `Deliver { prefix: FAILED_SEND_UNPARSEABLE_PREFIX }`.
- Emit `error!(agent_id, id, created_at=<raw string>, "failed_sends row has unparseable created_at; delivering fail-open with warning prefix")` alongside the delivery. Include the raw `created_at` value so a future migration debug does not require querying a since-deleted row.

**Rationale (per mika-arch first-pass §Uncertainty 3):** dropping on parse-fail is fail-closed and silent-data-loss unless the `error!` is actively paged. This code path is <1% frequency; alerts are unlikely. Fail-open with a screaming prefix keeps the message visible to the reader and preserves the audit trail. If telemetry ever alerts on this `error!` reliably, tighten to `Drop` in a v2.

### Retry-count discipline

The `Drop` arm calls `delete_failed_send` directly — it does NOT call `increment_failed_send_retry`. A dropped row was not retried; incrementing would be incorrect accounting. This is intentional; document with an inline comment above the `Drop` arm so a future reader does not add `increment` before `classify` and silently break the retry semantics.

### Log discipline

- Drop: `warn!(agent_id, id, age_secs, retry_count, created_at=<raw>, text_prefix=<first 80 chars>, "dropping stale failed_send")`. Include `created_at` explicitly so the audit trail is sufficient to reconstruct the drop without re-querying the (since-deleted) row.
- Deliver: existing `info!` at the sender is unchanged; add a `debug!` at the flush site indicating the prefix was applied so the operator can grep flush behavior in the log.
- Unparseable-timestamp deliver: `error!(agent_id, id, created_at=<raw>, "failed_sends row has unparseable created_at; delivering fail-open with warning prefix")`.

### Backwards compatibility

- Schema: no change (no migration).
- API: no change.
- Log format: additive `warn!`/`debug!` entries.
- Callers: none affected.

## Acceptance criteria

**AC1.** A row in `failed_sends` with `created_at` older than `FAILED_SEND_STALE_THRESHOLD` (5 minutes) is deleted by `flush_failed_sends` **without any gateway send call**. A `warn!` log line records the drop with `agent_id`, `id`, `age_secs`, `retry_count`.

**AC2.** A row in `failed_sends` with `created_at` within the threshold is delivered by `flush_failed_sends` with the sent text prefixed by `⏳ from earlier — `.

**AC3.** Unit tests cover the three classifications:
- `classify_failed_send` returns `Drop` for a timestamp older than the threshold.
- `classify_failed_send` returns `Deliver { prefix: FAILED_SEND_STALE_PREFIX }` for a timestamp within the threshold.
- `classify_failed_send` returns `Deliver { prefix: FAILED_SEND_UNPARSEABLE_PREFIX }` for an unparseable timestamp string (fail-open policy, per mika-arch first-pass §Uncertainty 3).

**AC4.** The threshold (`FAILED_SEND_STALE_THRESHOLD`) and prefix (`FAILED_SEND_STALE_PREFIX`) are named constants in `crates/mika-agent/src/server/handlers.rs`, discoverable via grep.

**AC5.** `cargo build --release`, `cargo test -p mika-agent`, and `cargo clippy --workspace --all-targets -- -D warnings` all pass.

**AC6.** `crates/mika-agent/docs/runtime-structure.md` and `crates/mika-agent/docs/architecture.md` note the staleness policy (one sentence each is sufficient).

## Verification steps (post-implementation)

1. `cargo test -p mika-agent classify_failed_send` — unit tests green.
2. `cargo test -p mika-agent failed_send` — existing `test_save_and_get_failed_send` still green.
3. `cargo clippy --workspace --all-targets -- -D warnings` — clean.
4. Manual: with a running local agent, insert a synthetic row via SQL (`INSERT INTO failed_sends (agent_id, text, created_at) VALUES ('...', 'stale row test', datetime('now', '-10 minutes'));`), send an inbound message to trigger the flush, confirm no send hit the gateway, confirm the row is gone from the table, confirm the `warn!` line is present in logs. Then repeat with a `datetime('now', '-30 seconds')` row and confirm the delivered text starts with `⏳ from earlier — `. This step is documentation-only (part of the PR body / handoff), not a CI gate.

## Rollout

- Merge to `main` → next `make deploy` picks it up (no cluster ops).
- Family-customer-1 tenant will see the fix as soon as its container is redeployed.
- Watch: for the first 24h post-deploy, grep for `dropping stale failed_send` and `⏳ from earlier —` in agent logs to confirm the paths fire. File a follow-up ticket if the drop rate is surprisingly high (suggests upstream `/send` is failing at a bad rate — a separate substrate concern).

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| A conversational reply is genuinely useful even at 6 minutes old (edge case). | Threshold is a named constant. Adjust on review; promote to config if the 5-min bound proves too aggressive. |
| A future scheduled-`send_message` failure paths into `failed_sends` and gets dropped too eagerly. | Not currently a code path (dispatcher.rs logs only). When it becomes one, add a `message_class` column and gate the drop on `class == 'conversational'`. Follow-up ticket. |
| Timestamp parse fails and the fallback drops legitimate rows. | The `strftime` format is fixed at insert time (`crates/mika-agent/src/db.rs:1400`), so parse failure indicates DB corruption. Error-logging + drop is preferable to error-logging + re-send forever. |
| The prefix `⏳ from earlier — ` is not rendered by every downstream channel (e.g. some SMS gateways strip Unicode). | Ships with a Telegram-first bias. If SMS/other becomes a problem, replace the emoji with an ASCII fallback (e.g. `"[from earlier] "`). Not v1's concern. |
| Removing 3.5h-old parked replies means a customer's original question goes fully unanswered on a slow retry. | Correct behavior. The parked reply is a snapshot of a stale intent; the customer already re-engaged, and the fresh turn is the current answer. Answering both is the bug. |

## Files changed (expected)

- `crates/mika-agent/src/server/handlers.rs` — constants, `classify_failed_send` helper + tests, flush loop update.
- `crates/mika-agent/docs/runtime-structure.md` — one-sentence note.
- `crates/mika-agent/docs/architecture.md` — one-sentence note.

Estimated diff: ~120 net lines added (helper + tests + docs).

## Grooming history

- 2026-07-10 — `/ce:plan` draft
- 2026-07-10 — `mika-arch` first-pass review (session `af4ef961-706e-493d-952f-609bbcc1703a`): **Disposition: READY**. Three refinements applied to plan before commit:
  1. Unparseable-timestamp policy flipped from `Drop` to `Deliver` with `⚠️ UNPARSEABLE TIMESTAMP — ` prefix (fail-open).
  2. Drop log includes `created_at` explicitly for audit-trail sufficiency.
  3. Documented "no retry-count increment on drop" invariant.
