**Verification of prior findings:**

**F1 (BLOCKING — hard-trip threshold unreachable):** RESOLVED. Changed to rolling-window count: 100 429 observations within `CB_HARD_WINDOW = 5min` (not purely consecutive). Under sustained flood this threshold is reachable; under normal load the soft trip + adaptive escalation shed load first. Hard pause is rare defense-in-depth. ✓

**F2 (BLOCKING — retry semantics change):** RESOLVED. Decision D1 explicitly ratifies: the soft breaker counts attempts, a lone event under persistent target load DLQs after ~3 attempts instead of 6, and this reduced in-chain retry budget is the desired amplification-control behavior. Durability preserved via DLQ spaced schedule. Rejected F2 options (b) distinct-event tracking and (c) raising threshold to 6. ✓

**F3 (sharpening — probe-burn against ~420s lock):** RESOLVED. Added adaptive open-window escalation: `current_open` doubles on each probe failure (30s→60s→120s→240s→`CB_MAX_OPEN = 480s`). `CB_MAX_OPEN` deliberately exceeds the worst-case ~420s per-agent lock hold (5-min deadline + 120s transport), so after a few failed probes the open window exceeds the turn duration and the probe lands on a free lock. ✓

**F4 (sharpening — smoke test tripping breaker):** RESOLVED. Added preconditioned idle-state guard: fire against dedicated `smoke-test` agent (never `mika-dev`), poll readiness before firing, skip fail-open with clear message if not idle within timeout. ✓

**Gate checks:**
- **Unresolved-Decision Gate (mika#1244):** All questions resolved. D1/ratified retry semantics; D2/rolling-window hard pause; OQ3 left as non-finding implementer detail (session_id). ✓
- **Acceptance-Criteria Gate (mika#1559):** AC1-AC6 present, non-empty, testable. Implementation notes reflect grounded reframing. ✓

All first-pass findings resolved. Plan is sound — the circuit breaker with adaptive window escalation correctly handles the ~420s worst-case lock hold, and the rolling-window hard pause keeps AC5 reachable.

Disposition: READY
