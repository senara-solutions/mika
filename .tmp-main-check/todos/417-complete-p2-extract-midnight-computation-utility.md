---
status: complete
priority: p2
issue_id: "417"
tags: [code-review, quality, duplication, reflection]
dependencies: []
---

# Extract Duplicated Midnight Computation Into Utility Function

## Problem Statement

The local-midnight-to-UTC computation appears in 4 places (1 pre-existing, 3 new):
1. `db.rs` — `count_heartbeat_sends_today` (pre-existing)
2. `db.rs` — `last_reflection_run_today` (new)
3. `scheduler.rs` — `check_and_fire_reflection` (new)
4. `agent.rs` — `run_silent_inner` (new)

All follow the identical 8-13 line pattern. This is a DRY violation with risk of drift.

## Findings

- **Code simplicity**: "~32 lines removed (8 per site), eliminates a subtle bug risk where one site might diverge"
- **Pattern recognition**: "4 duplications, 1 pre-existing"
- **Architecture**: Confirmed same recommendation

## Proposed Solutions

### Option A: Extract to utility function in db.rs (Recommended)
```rust
pub fn today_midnight_utc(timezone: &str) -> DateTime<Utc> {
    let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    let now_local = Utc::now().with_timezone(&tz);
    // ... (the standard pattern)
}
```
- **Effort**: Small (~35 net LOC saved)
- **Risk**: Low

## Technical Details

- **Affected files**: `crates/mika-agent/src/db.rs`, `scheduler.rs`, `agent.rs`

## Acceptance Criteria

- [ ] Single utility function for midnight computation
- [ ] All 4 call sites use the utility
- [ ] Tests for the utility (valid TZ, invalid TZ fallback)
