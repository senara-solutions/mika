# Plan: Event-Driven Filtered Monitoring for Orchestrator Agents (mika#1403)

## Goal

Enable Mika Prime (bearing-keeper / orchestrator) to receive **push-based, filtered notifications** of work-state changes — replacing manual monitoring and heartbeat cadence with event-driven wakeups that respect the single-session (#1401) and no-clock-noise design constraints.

## Problem Statement

Today, `route_event()` in `crates/mika-gateway/src/github.rs` maps each GitHub webhook event to exactly ONE target agent (1:1 routing). Mika Prime cannot observe work-state transitions because:

1. Events routed to mika-dev/mika-qa are consumed — no fan-out to observers.
2. Mika Prime's only cadence mechanism is the heartbeat (hourly cron), which is the noise being removed.
3. No filter layer exists to distinguish bearing-moving transitions from noise.

## Design: Three Deltas on Existing Rails

### Delta 1 — Observer Subscriptions (Gateway Fan-Out)

**Current:** `route_event()` returns `Option<&'static str>` — one agent name or None.
**Target:** After the primary agent receives its event, qualifying events are ALSO forwarded to registered observer agents. The primary routing path is unchanged.

#### Approach: `observe_event()` companion function

Add a new function alongside `route_event()`:

```rust
/// Returns observer agents that should receive a filtered copy of this event.
/// Observers receive the event as `internal=1` messages — they do NOT steal
/// the event from the primary route.
pub fn observe_event(
    event_type: &str,
    action: Option<&str>,
    check_conclusion: Option<&str>,
    merged: Option<bool>,
) -> Vec<ObserverNotification> {
    // ... filter logic
}

pub struct ObserverNotification {
    pub agent: &'static str,
    pub event_class: &'static str,  // e.g., "backlog_changed", "dispatch_gate", "loop_health"
}
```

**Why a separate function (not modifying `route_event`):**
- `route_event()` is the load-bearing 1:1 router with 15+ tests. Changing its return type to a list would touch every callsite and test.
- Observer semantics are fundamentally different: they don't replace the primary route, they supplement it. Keeping them separate makes the intent clear and avoids coupling observer filter changes with primary routing changes.
- The observer function takes an additional `merged` parameter (from `pull_request.closed` events) that the primary router doesn't need.

**Integration point in `handle_github_webhook`:**

After the primary `tokio::spawn` dispatch (line ~840), add a second block:

```rust
// 12b. Observer fan-out — filtered copy to observer agents
let observers = observe_event(event_type, event.action.as_deref(), check_conclusion, merged);
for obs in observers {
    let obs_state = state.clone();
    let obs_text = format_observer_text(&obs, &text);  // Prefix with event class
    let obs_request_id = format!("{request_id}-obs-{}", obs.agent);
    let obs_semaphore = state.webhook_semaphore.clone();
    // Observers share the same semaphore — they're lightweight and should
    // respect the same backpressure as primary events.
    let obs_permit = match obs_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => continue,  // Shed observer load before primary — observers are best-effort
    };
    let obs_agent = obs.agent.to_string();
    tokio::spawn(async move {
        deliver_with_retry(
            &obs_state,
            &obs_agent,
            &obs_text,
            &obs_request_id,
            repo_name.as_deref(),
            &event_type_owned,
            obs_permit,
            &obs_semaphore,
        )
        .await;
    });
}
```

**Observer delivery is best-effort:** If the semaphore is full, observer events are shed (the primary agent already got the event). Observer delivery failures do NOT go to the DLQ — they're informational, not operational.

### Delta 2 — Event Filter (The No-Noise Guarantee)

The filter is the product decision. The proposed filter set from the ticket:

#### WAKE events (bearing-moving transitions):

| Event | Route | Event Class | Rationale |
|-------|-------|-------------|-----------|
| `pull_request.closed` (merged=true) | Primary: mika-dev | `backlog_changed` | Backlog count dropped — "what's next?" |
| `issues.closed` (via issue_comment or label) | Not currently routed | `backlog_changed` | Issue closed — backlog count dropped |
| `check_suite.completed(success)` + open PR exists | Primary: mika-dev | `dispatch_gate` | Dispatched PR reached CI green — sequencing gate |
| `check_suite.completed(failure\|timed_out)` | Primary: mika-dev | `loop_health` | CI failure — loop-health signal |
| `pull_request.opened` | Primary: mika-qa | `dispatch_gate` | New PR opened — sequencing awareness |

#### IGNORE events (noise):

- `issues.assigned` — internal routing, not bearing-moving
- `issues.labeled` — label churn, handled by mika-dev
- `issue_comment.created` — comments, not state transitions
- `pull_request.synchronize` — pushes, not bearing-moving
- `pull_request.review_requested` — internal routing
- `pull_request_review.submitted` — review nits, handled by mika-dev

**Implementation in `observe_event()`:**

```rust
pub fn observe_event(
    event_type: &str,
    action: Option<&str>,
    check_conclusion: Option<&str>,
    merged: Option<bool>,
) -> Vec<ObserverNotification> {
    let mut observers = Vec::new();

    match (event_type, action) {
        // Backlog changed: PR merged
        ("pull_request", Some("closed")) if merged == Some(true) => {
            observers.push(ObserverNotification {
                agent: "mika-prime",
                event_class: "backlog_changed",
            });
        }
        // Dispatch gate: CI passed on a PR
        ("check_suite", Some("completed")) if check_conclusion == Some("success") => {
            observers.push(ObserverNotification {
                agent: "mika-prime",
                event_class: "dispatch_gate",
            });
        }
        // Loop health: CI failed
        ("check_suite", Some("completed"))
            if matches!(check_conclusion, Some("failure" | "timed_out")) =>
        {
            observers.push(ObserverNotification {
                agent: "mika-prime",
                event_class: "loop_health",
            });
        }
        // Dispatch gate: new PR opened
        ("pull_request", Some("opened")) => {
            observers.push(ObserverNotification {
                agent: "mika-prime",
                event_class: "dispatch_gate",
            });
        }
        _ => {}
    }

    observers
}
```

**Note on `issues.closed`:** The current `route_event()` does not route `issues.closed` at all (returns `None`). Adding it as an observer-only event requires extracting the action from the event before `route_event()` returns `None`. The `handle_github_webhook` handler already has access to `event.action` before calling `route_event()`, so the observer call can be placed before the early-return on `None` routing.

**Revised integration point:** The `observe_event()` call must happen BEFORE the `route_event() == None` early return, so observer-only events (like `issues.closed`) are captured even when the primary route drops them:

```rust
// 8b. Observer fan-out (before primary routing, so observer-only events are captured)
let merged = event.pull_request.as_ref().and_then(|pr| pr.merged);
let observers = observe_event(event_type, event.action.as_deref(), check_conclusion, merged);
// ... store observers for later dispatch

// 9. Route to agent (existing primary routing)
let target_agent = match route_event(...) { ... };

// 12. Primary async dispatch (existing)
// 12b. Observer dispatch (from stored observers)
```

### Delta 3 — Agent-Side: Target Session 0000, Internal=1, Trigger Turn

**How observer events arrive at the agent:**

The gateway forwards observer events to mika-prime via the same `/message` endpoint used for primary events. The message payload carries:

```json
{
    "text": "[Observer: backlog_changed] PR merged: senara-solutions/mika#1234 ...",
    "channel": "github",
    "request_id": "<delivery_id>-obs-mika-prime",
    "agent": "mika-prime"
}
```

**Session targeting:** The ticket specifies events should target session `00000000-0000-0000-0000-000000000000` (mika#1401's single-session). This requires a mechanism to route observer events to a specific session instead of creating a new UUID per message.

**Approach:** Add a `session_id` field to `MessageRequest` (optional, defaults to new UUID when absent). Observer dispatch sets `session_id: "00000000-0000-0000-0000-000000000000"`. The `/message` handler uses this session if provided, falling back to `Uuid::new_v4()`.

```rust
// In server/types.rs, extend MessageRequest:
pub struct MessageRequest {
    pub text: String,
    pub chat_id: Option<i64>,
    pub channel: String,
    pub request_id: String,
    pub agent: String,
    pub images: Option<Vec<ImagePayload>>,
    pub session_id: Option<String>,  // NEW: observer events target a specific session
}
```

**Internal flag:** Observer messages are written with `internal=true` — they're system-to-agent messages hidden from the user inbox. The `internal` flag on `AgentParams` needs to be set based on whether this is an observer event.

**Approach:** Add an `internal` field to `MessageRequest` (optional, defaults to false). Observer dispatch sets `internal: true`. The handler threads this through to `AgentParams.internal`.

```rust
// In MessageRequest:
pub internal: Option<bool>,  // NEW: observer events are internal=1
```

**Turn trigger:** Observer events trigger a normal agent turn — the agent processes the event through its skills and decides whether to act (re-order, dispatch) or stay silent (heading holds). This replaces the heartbeat cadence for mika-prime.

### Dependency: mika#1401 (Single-Session Structural)

The plan assumes session `00000000-0000-0000-0000-000000000000` exists as a durable, derived target. If mika#1401 has not shipped when this ticket is implemented:

- The `session_id` field on `MessageRequest` still works — the handler will create the session if it doesn't exist (via `create_session`).
- The single-session invariant (all events land in one session for continuity) is maintained by the gateway always sending the same `session_id`.

### Heartbeat Interaction

The ticket says observer events REPLACE heartbeat cadence for mika-prime. Implementation:

- mika-prime's `identity.toml` should have `[heartbeat] enabled = false` (already supported via `heartbeat_enabled_for_agent`).
- The observer subscription is the new wakeup mechanism — event-driven, not clock-driven.
- If event volume is zero for an extended period (no GitHub activity), mika-prime has no wakeup. This is intentional — no bearing shift means no action needed. If a fallback cadence is desired later, it can be added as a low-frequency recurring task (e.g., daily check-in) without reintroducing the hourly noise.

## File Changes

### Gateway (`crates/mika-gateway/`)

| File | Change | Lines (est.) |
|------|--------|-------------|
| `src/github.rs` | Add `observe_event()`, `ObserverNotification`, `format_observer_text()` | ~60 |
| `src/github.rs` | Integrate observer dispatch into `handle_github_webhook` | ~40 |
| `src/github.rs` | Add tests for `observe_event()` (mirror `test_route_event_*` pattern) | ~80 |
| `CLAUDE.md` | Document observer routing | ~15 |

### Agent (`crates/mika-agent/`)

| File | Change | Lines (est.) |
|------|--------|-------------|
| `src/server/types.rs` | Add `session_id: Option<String>` and `internal: Option<bool>` to `MessageRequest` | ~4 |
| `src/server/handlers.rs` | Use `req.session_id` if provided; thread `req.internal` to `AgentParams` | ~15 |

### Total: ~215 lines changed

## Acceptance Criteria

1. **Observer routing works:** `observe_event()` returns `mika-prime` for merged PRs, CI success, CI failure, and PR opened events. Returns empty for all other events.
2. **Primary routing unchanged:** `route_event()` is not modified. All existing tests pass.
3. **Fan-out is non-blocking:** Observer dispatch does not delay the primary agent's event delivery or the 200 response to GitHub.
4. **Best-effort delivery:** Observer events that fail semaphore acquisition are shed silently (no DLQ entry, WARN log only).
5. **Session targeting:** Observer events to mika-prime target session `00000000-0000-0000-0000-000000000000`.
6. **Internal flag:** Observer messages are stored with `internal=1`.
7. **Heartbeat disabled:** mika-prime's identity has `[heartbeat] enabled = false`.
8. **`issues.closed` observer-only:** The `issues.closed` event (not currently routed to any primary agent) generates an observer notification for mika-prime.
9. **Tests:** Unit tests for `observe_event()` covering all WAKE and IGNORE events. Integration test for observer dispatch path.
10. **Structured logging:** Observer dispatch emits a structured log event (`observer_event_dispatched`) with `event_type`, `action`, `event_class`, `observer_agent`, `delivery_id`.

## Open Questions (for operator confirmation)

1. **Filter refinement:** The WAKE/IGNORE sets above are the first cut from the ticket. The operator should confirm or refine before implementation. Key tension: too many events reintroduce noise; too few miss bearing shifts.
2. **`issues.closed` routing:** Currently not routed at all. Should it also be added to the primary `route_event()` → mika-dev path, or remain observer-only?
3. **Repeated CI failures:** Should consecutive `check_suite.completed(failure)` events for the same PR be deduplicated at the observer level, or should mika-prime handle dedup in her session context?
4. **Observer agent extensibility:** Is `mika-prime` the only observer for now, or should the system support multiple observer agents from day one? The `Vec<ObserverNotification>` return type supports multiple observers, but the filter logic currently hardcodes `mika-prime`.

## Risks

1. **mika#1401 dependency:** If single-session is not yet shipped, session targeting still works mechanically but mika-prime may not have the continuity context expected.
2. **Event volume:** On active development days, merged PRs + CI events could generate 10-20 observer events. This is well within the semaphore capacity (30 permits shared) but worth monitoring.
3. **Agent lock contention:** mika-prime's per-agent mutex (`try_lock_owned`) means overlapping observer events return 429. The retry schedule handles this, but rapid-fire events (e.g., 3 CI failures in 30 seconds) may queue up. This is acceptable — the agent processes them sequentially, and the last event carries the most recent state.
