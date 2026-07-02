F1: (sharpening) Hard-trip threshold (100→60s) may be unreachable given soft-trip short-circuit.
   Concern: The soft-trip opens the circuit at 3 consecutive 429s for 30s, short-circuiting all new deliveries to DLQ without HTTP attempts. During the 30s open window, no new 429s are generated for that target. The half-open probe (1 delivery) generates at most 1 additional 429 every 30s. To reach 100 consecutive 429s would require ~50 open/close cycles (25+ minutes of sustained overload), but the system would likely recover or the DLQ backoff would space events further apart. The hard-trip threshold adds complexity that may never trigger.
   Change required: Either (a) remove the hard-trip threshold and rely solely on soft-trip + DLQ backoff, or (b) change the "consecutive 429s" counter to a "429s per rolling window" (e.g., 100 per 5 minutes) that counts across open/close cycles, or (c) justify the hard-trip as a defense-in-depth with explicit acknowledgment it may be rare.
   Citation: review-guide.md § YAGNI / "Complexity budget requires reachable activation paths"

F2: (BLOCKING) Single-event retry interaction changes semantics significantly.
   Concern: A single event with its 6-attempt retry chain produces up to 6 consecutive 429s. With the shared per-target breaker threshold at 3, the event's own retries will trip the breaker mid-chain. The current plan acknowledges this ("a lone event may now DLQ after 3, not 6") but doesn't explicitly ratify this behavioral change as acceptable. This is a functional change to the retry contract — events may now get fewer retry attempts before DLQ when the target is under stress.
   Change required: Explicitly ratify in the plan that (a) the reduced retry count under stress is desired behavior (amplification control outweighs per-event persistence), OR (b) change the breaker to count distinct events rather than attempts (more complex — requires event ID tracking), OR (c) increase the soft threshold to 6+ to preserve the existing per-event retry budget.
   Citation: review-guide.md § Behavioral Contracts / "Changes to retry semantics require explicit operator ratification"

F3: (sharpening) Half-open probe during 30s window likely fails against 420s lock hold.
   Concern: The per-agent lock is held for up to ~420s (5-min deadline + 120s transport). The half-open probe fires after only 30s (soft) or 60s (hard). If the agent is still processing the turn that caused the original 429s (highly likely), the probe will 429 and immediately re-open the circuit. This creates a rapid probe-fail cycle that doesn't actually test recovery — it just burns probe deliveries.
   Change required: Either (a) extend the open duration to exceed the worst-case lock hold (420s+), or (b) use a smarter probe that checks agent liveness via a lightweight health endpoint rather than a real delivery attempt, or (c) accept the probe-fail cycle as "acceptable noise" with explicit acknowledgment that the breaker is primarily a backpressure valve, not a recovery detector.
   Citation: review-guide.md § Timing / "Circuit-breaker half-open probes must test against realistic recovery windows"

F4: (sharpening) `scripts/smoke-webhook-flood.sh` may trigger the very breaker it's testing.
   Concern: AC6 requires firing ~10 mock webhooks and asserting zero 429s. But if the agent is still processing a prior turn (e.g., from a previous deployment's queued DLQ events), the first few webhooks will 429, trip the soft breaker, and subsequent webhooks will short-circuit to DLQ without HTTP attempts — the test sees "no 429s" but also no successful deliveries, or sees the breaker as a false positive.
   Change required: Add a pre-condition check in the smoke test: query the agent's `/health` or wait for `agent_lock` to be free before firing the flood. Alternatively, fire against a dedicated `smoke-test` agent that is guaranteed idle. Document this precondition explicitly.
   Citation: review-guide.md § Test Reliability / "Tests that assert on success must ensure pre-conditioned idle state"

Disposition: ITERATE
