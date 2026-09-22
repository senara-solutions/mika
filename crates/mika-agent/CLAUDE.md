# mika-agent — Agent Engine

Agent container: SQLite DB, agent loop, tools, prompt assembly, A2A server endpoints, HTTP server binary (mika-spirit). This is the core crate where most development happens.

## Agent Loop

Max 20 tool steps (all modes: conversation, callback, reminder, team), a per-agent turn envelope **defaulting** to 5 minutes (settable since mika#2189 — see § Timeout Budgets), 30s default per-tool timeout (overridable via `Tool::timeout_secs()` for builtins, and by the owning skill's `timeout_secs` for skill tools — see § Per-Tool Skill Budget). `LoopMode::Silent { max_steps }` carries per-trigger step limits via `SilentTrigger::max_steps()`. Step-awareness nudge injected at step `max_steps - 2` for all modes to encourage wrapping up. Silent mode nudge text is tailored for `send_message` notification.

**Deadline enforcement (#848, #939, mika#2189):** the turn budget is enforced via an `Instant`-based deadline checked at five points: (1) top of each `run_loop` step iteration, (2) end of prelude work in each inner function before entering `run_loop`, (3) before `attempt_continuation_turn` entry — skip when `now + CONTINUATION_TIMEOUT_SECS > deadline`, (4) inside `attempt_continuation_turn` itself, where the inner timeout is clamped to `min(60s, deadline - now)`, (5) inside the LLM transport retry loop — `send_message_with_deadline()` aborts the retry chain when the remaining budget falls under a threshold **derived from the effective per-call plafond** (`0.75 × plafond + 0.25 × plafond`, which is the historical 90 + 30 = 120s at the default plafond), preventing doomed retries from consuming the deadline (#939). **The outer agent deadline still never drops a future mid-flight** — that is the mika#848 contract and it is unchanged: the `llm_calls` row is always persisted (success or transport-timeout), and worst-case turn duration remains `envelope + plafond`, i.e. `300s + 120s = 420s` at the shipped defaults. What changed with **mika#2342** is that the provider's `reqwest` timeout is no longer the **sole** cancellation mechanism: `run_loop`'s call is wrapped in a `tokio::time::timeout` sized on `LlmProvider::worst_case_failure_secs() + LLM_WATCHDOG_MARGIN_SECS` (60 s) — see § LLM-call watchdog below. Sized on the *worst case* rather than on the remaining deadline, that net does not contradict the mika#848 reasoning: on a rail that bounds its own calls it can only fire once the rail's mechanism has already failed, i.e. on a call that is already lost. Note the Anthropic rail (`crates/mika-common/src/claude.rs`) still hard-codes its own `120s` literal instead of reading the plafond — a real inconsistency mika#2189 names and leaves to its own ticket; mika#2342 makes it a *declared* value (`AnthropicProvider::worst_case_failure_secs`) so the net is not sized on a budget that rail does not honour. `LoopResult` is a three-variant enum (`Done`/`MaxStepsExceeded`/`DeadlineExceeded`) without `#[non_exhaustive]` — the compiler's match-exhaustiveness check enforces that all three outer handlers (conversation, silent, team) handle every variant. CI lint guard `scripts/check-loop-select.sh` rejects `tokio::select!` inside `run_loop`'s body (would shadow the iteration-top deadline check).

On max-steps exceeded: continuation turn (tools disabled, deadline-clamped timeout, ceiling 60s) forces a text summary via shared `attempt_continuation_turn()` helper (used by Conversation, Team, and Silent modes); the helper persists an `llm_calls` row in all outcomes (success/error/timeout) so the continuation is never the silent-drop variant of the in-flight-cancel bug at smaller scale. If continuation fails or is skipped (deadline too close), structured fallback shows last 5 tool names with status. Silent mode continuation sends the summary via `message_sender` if available, prefixed with "[Background task exceeded tool step limit]".

**Test-only entry points (`run_*_with_deadline`):** publicly visible by naming convention, used only by `EvalHarness` to inject short deadlines for the deadline-during-LLM-call eval scenarios. Production callers should always use `run_agent`/`run_silent_agent`/`run_team_agent`. `AgentParams` carries no deadline knob.

### Timeout Budgets (mika#2189)

**The asymmetry this closed.** Two numbers govern a turn: the **per-call
plafond** (what `reqwest` is given) and the **per-agent envelope** (the turn
deadline). Since mika#1660 an operator could raise the plafond via
`MIKA_LLM_HTTP_TIMEOUT_SECS`; the envelope was a bare `300` constant in
`planning/policy.rs` with no env var and no per-agent setting. **One could raise
the ceiling and not the room meant to contain it** — and raising the plafond
alone lets a single call eat the whole envelope of a pass that averages three.

The founding measurement (7 days ending 2026-09-05, `llm_calls`): **209
failures** under one message — `failed to read response body: … operation timed
out` — whose latency distribution has no tail, only two values: **240 s** (171
occurrences) and **120 s** (37). Not provider variance; a client guillotine
crossed once or twice. Three agents, two models, one provider — so a model
fallback for mika-arch would have moved 7 % of the problem.

**`LlmTimeoutBudget` (`mika-common::llm::budget`)** now holds the pair, and the
three retry thresholds are **derived** from the plafond rather than written
beside it: `typical_call = 0.75 ×`, `retry_buffer = 0.25 ×`,
`transport_retry_min_remaining = 0.50 ×`. Those fractions reproduce the old
literals (90 / 30 / 60) **exactly** at the 120 s default, so the migration is a
no-op at the shipped geometry and follows any later setting — where literals
would have silently described a geometry that stopped existing.

**Containment invariant (D2).** `LlmTimeoutBudget::validate` refuses
`plafond >= envelope` at **provider construction**, the same lifecycle point
where mika#1660 already panics on a too-small plafond. Consequence, written down
rather than discovered: **a `mika` that boots is not proof that its budgets are
valid; the first call is.**

**Bounded failure cost (AC3-b).** Raising a plafond cannot slow a call that
already succeeded. The regression it *does* create is that a **failing** call
gets more expensive, so `max_attempts = floor(envelope / plafond)` (clamped to
the provider's own `MAX_RETRIES + 1`) makes `attempts × plafond ≤ envelope` true
by construction. At the default geometry that is **2** — exactly what the
measurement shows. Not a pure no-op, and saying otherwise would be more
comfortable and less true: before this, a third attempt could start at exactly
`remaining == threshold` and carry a failure to 360 s, past the 300 s envelope.

**Reading the envelope.** `planning::policy::agent_total_timeout_secs(llm)` takes
it off the **provider**, which already holds the plafond — taking both from one
object is what stops them drifting apart again. The retired constants
(`AGENT_TOTAL_TIMEOUT_SECS` and its team sibling, which was a second literal
`300` whose doc comment merely *promised* it matched) are gated by
`policy::tests::no_bare_agent_timeout_constant_remains`, a source scan over
`src/`. **One documented exception:** `DELEGATE_TASK_TOOL_TIMEOUT_SECS` stays
static because `Tool::timeout_secs` takes `&self` and cannot reach a provider —
an agent with a raised envelope delegating through that tool would be cut at the
default. Not reachable today (mika-arch has no `delegate_task`), so it ships as a
stated bound rather than a `Tool` trait change bundled into a timeout ticket.

**Per-agent settings** (`~/.mika/agents/<name>/config.toml`, next to
`llm_provider` / `openrouter_model`): `llm_http_timeout_secs` and
`agent_total_timeout_secs`. The fleet stays at 120/300; **mika-arch is retuned to
240/900** (`MIKA_ARCH_CONFIG`), derived from its measured p99 = 191 s, max =
233 s and 3.1 calls per pass. Deliberately **no per-family default** (Q3): a new
slow agent discovering the problem through a failure is an accepted, *visible*
risk — the invariant's message names both values and the file to fix.

**Post-deploy check (V3).** Re-run the failure-latency count per agent after at
least 24 h and 100 mika-arch calls. The criterion is that 120 s / 240 s vanish
from the latency column of errors. **If a fresh signature appears at 240 s /
480 s, halt** — the plafond moved and so did the distribution, which says the
model of the failure is wrong, not that the value is too low. Capture the
baseline before deploying: `prune_old_llm_calls` purges old rows.

**Observability + boot guard (mika#2293).** mika#2189 made the pair settable and
left it unobservable, and the two failures that follow from that are closed here.

*The pair is now said out loud.* `llm_budget_resolved` (INFO, **ungated** by
`MIKA_STORE_LLM_CALLS`) is emitted at `server::init_agent` and per team run in
`teams::engine` — the two callers that already hold the agent's name, rather than
widening `create_provider_with_budget`, a free function that knows neither agent
nor home. It carries `agent_id`, both values, `max_attempts`,
`worst_case_failure_secs`, and the **provenance** of each half. That last field is
the point: `agent_config` at 240 means the setting is in force and the cause of a
cut is elsewhere; `process_env` means a fleet-wide variable is shadowing the
per-agent file; `default` means the file was never read or never carried the key.
Three remedies, and none of them is deducible from a plafond someone raised — which
is why mika-arch's `240/900`, shipped 2026-09-06, could fail in silence until the
2026-09-11 measurement. Deduplicated on the resolved pair, so a repetition is silent
and a **change** is re-emitted. **Deliberate blind spot, written at the emission
site:** the per-skill `[llm]` override path (`agent_loop`'s `make_provider_for`,
one provider per override) emits nothing — the question is an agent's *nominal*
budget, not what a skill overrides for one turn. Reader lives in
`mika_common::llm::budget_provenance` (see `mika-common/CLAUDE.md` for why the
cascade is rebuilt rather than recorded, and why its inverted order is pinned).

*A half-configured pair now fails at boot.* `server::budget_guard::assert_llm_budgets_valid`
runs in `run_server` after `provision_well_known_agents` (which writes the
`config.toml` carrying the pair) and before any agent is initialized, over the same
`servable_agent_names` population as `tier_guard`. It refuses startup naming the
agent, both values, their provenance and the key to fix. **The failure it closes:**
`MIKA_LLM_HTTP_TIMEOUT_SECS=300` on the service without raising the envelope gives
mika-dev and mika-qa `cap = 300 >= envelope = 300` — no LLM call ever again — while
**mika-arch survives** on its own `config.toml`'s 900. Two agents silent, a third
answering: the shape that looks least like a configuration mistake and gets blamed
on the provider most readily, which is the misreading mika#2293 exists to correct.
Pre-existing violations get **no grace period and that costs nothing** — such an
agent could not make an LLM call anyway, so the guard moves an existing failure from
the first call to startup and makes it legible. It corrects no value and invents
none. It also **precedes** mika#1660's panic rather than replacing it: the `None`
path of the plafond goes through `llm::http_timeout_secs()`, which aborts on an
unparseable or below-floor value without naming the agent or the cascade door, so
the guard reads the raw values through the non-panicking `BudgetProvenance` the
observability half already writes. The panic stays for unguarded paths — a `mika`
CLI reaching it still panics exactly as before.

### LLM-call watchdog (mika#2342)

**The failure.** A mika-arch turn entered `llm_call started` and returned
**nothing for 27+ minutes**: no `llm_call completed`, no `turn_usage`, no error.
The task stayed `in_progress`, the A2A client walked away, the groom delivered
`first-pass _arch_ask failed`. Twice, on the same brief, with
`MIKA_LLM_HTTP_TIMEOUT_SECS=420` and `MIKA_AGENT_TOTAL_TIMEOUT_SECS=600` in the
service environment — a call of that length *should not exist*.

**The structural defect, which is readable without reproducing the incident.**
`run_loop`'s call was made bare, and the envelope is a test at the **top of the
iteration**, not a wrapper. A call that does not return the future never reaches
that test. So the provider's `reqwest` timeout was the **sole** mechanism able to
end an in-flight call, and when it failed — for a reason still unnamed — nothing
in the process could bound it and nothing recorded that anything had happened.
The asymmetry made it hard to read as a choice: `attempt_continuation_turn`, in
the same file, has wrapped *its* call in a `tokio::time::timeout` all along and
persists a row on all three arms.

**The net.** `tokio::time::timeout(llm.worst_case_failure_secs() +
LLM_WATCHDOG_MARGIN_SECS, …)`. On timeout: a `llm_call_watchdog_fired` WARN, an
`audit_events` row (`tool_name = 'llm_call_watchdog'`), and a synthetic
`LlmError::Transport` routed into the **pre-existing** `Err` branch, so the
`llm_calls` row is persisted with `status = "error"`, the real latency and
`request_bytes` without duplicating the persistence path.

**Why the worst case and not the remaining deadline.** Cutting on `remaining`
would re-open mika#848: a call dropped in flight loses its result *and* its row,
including when it was about to succeed. Sized on the worst case, the net fires
only after the rail's own mechanism has already failed — on a call that is
already lost, so cutting it costs nothing and its firing is first-order
information. **That property is conditional and the code says so:** it holds
*because* the three production rails bound their own calls, not because the
`LlmProvider` trait requires it. On a rail without a transport timeout (the mock
today, a new rail tomorrow) the net is the *first* mechanism, not the second.

**The worst case is declared by the rail, never derived from the budget.**
`LlmProvider::worst_case_failure_secs()` defaults to
`timeout_budget().worst_case_failure_secs(DEFAULT_ATTEMPTS_HARD_CAP)`, and
`AnthropicProvider` overrides it: that rail hands `reqwest` a `120s` literal
instead of the plafond, so at `MIN_HTTP_TIMEOUT_SECS` a derived value would read
40 s against a transport that can take 480 — a guaranteed false positive. At the
current geometries: 420/600 → `max_attempts = 1`, net at **480 s**; 240/900 →
`max_attempts = 3`, net at **780 s**. Both far under the 27 minutes observed and
above any legitimate chain.

**Per-attempt instrumentation is the discriminator, not decoration.** The net
alone says "something exceeded the worst case" without saying what.
`llm_call_attempt` (before each `send_once`, on the three rails, carrying
`attempt`, `max_attempts`, `request_bytes`) separates three readings of the same
silence: N attempts of roughly a plafond each (the chain runs, the cause is
upstream); one attempt outliving the plafond (reqwest did not bound it); no
attempt at all after `started` (the block is *before* `send_once` — body
serialization, connection acquisition). `request_bytes` rides on the **start** of
the attempt because that is the only way to know the size of a request that never
comes back: `llm_calls.request_bytes` is written after the call returns.

**Structural guard.** `agent_loop::tests::mika2342_every_llm_call_is_wrapped_in_a_timeout`
— a source scan, because removing the net breaks no assertion: the loop keeps
working and every existing test stays green, only the call goes unbounded and
silent again. Inventory closed at two call sites, **no allowlist** (an allowlist
born empty is a place to put the next violation); a third site is halt-and-surface.

**Operator surfaces.** `llm_call_watchdog_fired` in `$MIKA_SPIRIT_LOG_FILE`
should stay **empty** — any hit is a call reqwest failed to bound, i.e. the root
cause of mika#2342 made visible, and it feeds the follow-up ticket.
`SELECT COUNT(*) FROM audit_events WHERE tool_name = 'llm_call_watchdog';`.
`grep llm_call_attempt … | jq 'select(.attempt > 0)'` gives the real retry volume,
until now uncountable. **Halt condition:** repeated `llm_call_watchdog_fired` on
one agent with no intervening `llm_call_attempt` points at a block *before*
`send_once` — a different defect; do not raise the net.

Tool call summaries (name, truncated input/output, success, non_zero_exit) persisted in `messages.metadata` JSON column for cross-turn introspection (capped at `TOOL_METADATA_MAX = 4000` chars — tail entries dropped when exceeded, #744). The `tool_calls` DB table is the authoritative source; the dashboard's inline `ToolCallsTable` fetches from `GET /api/v1/traces/:trace_id/tool-calls` with metadata as fallback for pre-v15 messages. `MessageResponse` exposes `trace_id: Option<String>` to enable this lookup. `non_zero_exit` is set by heuristic detection of `Exit code:` / `Killed by signal:` prefixes from exec handlers; history builder tags these with `[NON-ZERO]` (distinct from `[FAILED]`). History builder appends `<context type="tool_history">` blocks to assistant messages.

Compaction includes tool names in summarization. Multi-modal tool results: `ToolOutput` carries optional `images: Vec<ImageData>` (base64-encoded), converted to multi-block `tool_result` content arrays for the Claude API. Prior-turn images are stripped before each API call to prevent unbounded memory growth.

**Per-turn tool_use dedup guard (#582):** `process_tool_calls()` deduplicates identical `(tool_name, arguments)` pairs emitted inside a single LLM response. The underlying tool runs once, the `tool_calls` DB row is saved once, one `ToolCallSummary` is emitted, and duplicate tool_use ids receive a `tool_result` built from the cached `ToolOutput` so the conversation/API history stays paired. Images on the cached result are stripped before reuse so the duplicate does not re-consume the shared `image_bytes_budget` (the LLM already received the images on the first duplicate's `tool_result`). Defends against provider-side duplication (observed with non-Anthropic providers). Logs `warn!` with `trace_id`, `tool`, `step`, and `cached_was_error` when it fires.

### Post-Conditions (EndTurn Chain)

Twelve sequential post-conditions on assistant text responses, plus one early-accept:

1. **Text-based tool call detection:** `detect_text_based_tool_call()` catches XML-style patterns (`<function=...>`) that slip through `extract_xml_tool_calls()` in mika-common, re-prompts the LLM once.
2. **Prose-style tool call detection (#569):** `detect_prose_style_tool_call()` catches function-call-style prose patterns (`tool_name({"key": "val"})`) where the identifier matches a registered tool (builtins + skills + MCP). Gated against the tool set to avoid false positives on code examples. Single retry.
3. **Required-tools gate:** When keyword-matched skills declare `[constraints] required_tools`, the engine tracks tool calls across all steps; if required tools haven't been called, the response is rejected once. `filter_available_required_tools()` pre-filters against builtins + skill tools + MCP. Only `Keyword`-matched skills contribute constraints (#463). **Self-contained response instruction (#890):** The correction message includes a persistence-awareness clause instructing the LLM to restate the full content on its corrected response — not reference prior turns — because only the final `EndTurn` is persisted to `messages`. Defense-in-depth: mika-arch skill prompts (`mika-arch-groom-ticket`, `mika-arch-second-review`) reinforce the same contract in their `### Constraints` section. **Terminal failure bypass (#516):** `has_terminal_required_tool_failure()` checks `all_tool_summaries` for required tools that failed with known terminal errors (GitHub self-approval, HTTP 4xx, permission errors). When detected, the gate allows EndTurn without retry — the agent attempted the tool and hit an unrecoverable wall. `is_terminal_tool_error()` classifies output via `RETRYABLE_ERROR_PATTERNS` (checked first, takes priority) and `TERMINAL_ERROR_PATTERNS`. Unknown errors default to retryable (conservative).
3b. **PR review early-accept (#695, #821):** `has_successful_pr_review()` checks if `all_tool_summaries` contains a successful `run_gh` call with `"pr"` and `"review"` in the input. When true, guards #3 (required-tools, #821), #4–#6b, #7–#9 are all skipped — but NOT guards 6c (asserted_unavailability) or 6d (assert-grounded) (#1178). Those two detect claim-without-evidence, orthogonal to the PR-review completion semantics. — the qa-review workflow's primary action completed and forced continuation would risk duplicate submissions. Defense-in-depth (two layers): (1) Session-scoped `pr_reviews_posted` map on `AppState` (`DashMap<String, HashSet<String>>`, #821) prevents duplicate reviews across turns within the same session — the primary defense, keyed by `(session_id, repo|pr_identifier)`. Entries evicted at the 5 dispatch/delegate session-teardown sites (`delegate_task.rs` + 4 `dispatcher.rs` callsites). Conversation-mode sessions that post a review are not covered — a known slow, bounded leak (tracked as coherence-debt DEBT-E in `mika-platform/docs/coherence-debt.md`). (2) Per-turn `ToolContext.pr_review_posted` AtomicBool (#695) rejects duplicates within a single turn. Both guards reject `pr review` calls with structured `duplicate_pr_review` error. `make_pr_dedup_key()` derives the session-scope key from `gh pr review` arguments.
4. **Completion-claim guard (#483):** `detect_completion_claim()` detects completion-claim keywords (`merged`, `deployed`, `complete`/`completed`, `shipped`) in assistant text. If detected AND `update_task_status` is in the tool registry AND it was not called AND active tasks exist, the response is rejected once. Skips for delegates and team agents.
4b. **Milestone-close-claim guard (#797, #1207):** `detect_milestone_close_claim_without_patch()` detects when assistant text contains a first-person claim that a GitHub milestone was closed (regex `\b(I|we|i've|we've)\s+(closed|closed out|completed)\b.{0,40}\bmilestone\b`) without a satisfying `run_gh` call in the turn. A qualifying call has `input_summary` containing `"api"`, `"PATCH"`, `state=closed`, and a milestones API path. Discrimination granularity (#1207): when the claim contains a parseable milestone number, suppress only if that specific number appears in a PATCH URL within the turn; otherwise fall back to presence/absence. This is a deliberate divergence from #483's presence/absence pattern, justified by mika-arch's multi-milestone review surface — a single turn may legitimately PATCH one milestone while writing prose about another. Single retry tracked via standalone `milestone_close_claim_retry_done` flag. Composes with #4: when both regexes match (e.g., "I completed milestone#14"), #4 fires first; #4b fires on a subsequent EndTurn after #4 is satisfied. After the retry, second-violation in the same `run_loop` emits a `warn!` and accepts the EndTurn (single-retry budget exhausted).
5. **Fabricated action-claim guard (#308):** `detect_fabricated_action_claim()` detects when the agent claims to have performed an action with a GitHub resource URL but made zero tool calls in the turn. Single retry.
5b. **Dev-groom fabrication guard (#1133):** Detects `Verdict: GROOMED` or `Verdict: ESCALATE` in conversation-mode assistant text without a successful `run_claude_pilot_groom` call in the turn. dev-groom is a dispatcher — verdicts arrive via callback from claude-pilot, not from the dispatcher LLM's turn. Gated: only fires when `run_claude_pilot_groom` is in `enabled_tool_names` (the dev-groom skill is loaded for the agent — producer skills like mika-arch-second-review that legitimately emit Verdict lines don't have this tool and bypass the guard entirely). Conversation mode only (`mode.is_conversation()`); callback turns (Silent mode) legitimately carry Verdict lines from the inner session and bypass this guard. Single retry with corrective re-prompt. Companion manifest fix: `dev-groom/skill.toml` was changed from `[output] required_suffix_lines` (producer shape) to `[constraints] required_tools = ["run_claude_pilot_groom"]` (dispatcher shape). Static parity test `test_dispatcher_skills_dont_declare_required_suffix_lines` guards against regression.
5c. **Distribution Doctrine public-promo guard (mika#1814):** Detects assistant text that proposes, drafts, or plans a prohibited public-launch surface — Show HN, Product Hunt, Reddit launch thread, Twitter promo thread, growth-hack — in violation of Mika's invitation-only distribution invariant. Two-layer regex: Layer A (subject) matches one of the prohibited-surface keywords; Layer B (verb) matches a first-person/second-person proposal or drafting verb (bilingual FR + EN — the founding incident, Al B 2026-07-20, was in French; Vincent operates on operator-tier in English). Both layers must fire so an educational answer ("Mika does not do Show HN — she grows by invitation") does NOT trigger the guard. Applies uniformly across modes (conversation, silent, team) and both tiers (family/operator) — a heartbeat that spontaneously drafts a Show HN would be exactly as bad as a conversation-mode one. Single retry via `intent_guard_retries` with label `DOCTRINE_PUBLIC_PROMO_LABEL`. Not skipped by `skip_remaining_guards` (#1178) — a successful PR review does not grant license to draft a public launch. Structural half of the mika#1814 fix (the prompt-template `## Distribution Doctrine` section is the intent half, code-managed in `prompt::write_distribution_doctrine_section`; see `feedback_prompt_enforcement_fragile`). Position: after 5b (dev-groom fabrication), before 6 (intent-precondition registry) — same fabrication-class family. Related: mika#1798 (umbrella non-transit doctrine bake), mika#1783 (substrate leak "Salut Vincent").
5d. **False local-hosting claim guard (mika#2290):** Refuses assistant text that asserts **this instance** runs locally — or that the user's data never leaves their machine — while the resolved `Deployment` is not `Local`. Founding incident: a cloud tenant answered « tout tourne en local, tes données ne quittent pas ta machine » to a campaign guest on 2026-09-11. Position 5d, immediately after 5c, whose form, `intent_guard_retries` budget and `guard.*` telemetry it reuses; same fabrication family, one step more self-referential.

**What it closes, and what it does not need.** The prompt half (the `## Runtime` hosting line and rule 5 of `## Self-Identity Discipline`) is the *intent* half; per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` it does not hold alone. **This guard closes the p1 without the companion `mika-cloud` ticket**: a cloud tenant today emits no `MIKA_DEPLOYMENT`, resolves `Unknown`, is therefore not `Local`, and the measured claim is refused from this deploy onwards. The `cloud` signal improves the *answer*; it was never needed for the *refusal*.

**Two layers, and the discrimination is of scope, not of vocabulary — that is the hard part.** The remedy the ticket prescribes contains the word "local" itself (« la MÊME stack open-source (MIT) est self-hostable en local si tu veux »), so a guard that fired on the word would forbid the true sentence it exists to make Mika say. **Layer A** is a locality predicate (`en local`, `localement`, `ta machine`, `locally`, `on your machine`, …). **Layer B** is a present-indicative assertion *carrying its own grammatical subject* (`tout tourne`, `je tourne`, `tes données restent`, `I run`, `your data stays`) — modal, conditional, interrogative and self-hosting forms are simply not in the list, which is what makes the remedy sentence structurally unmatched rather than specially excused. Both must fire, in that order, within 40 non-terminator characters (so "same sentence" is enforced by the character class rather than by a second pass).

**Layer B has two polarities and they obey opposite gap rules**, because half the measured claim was written in the negative (« tes données ne quittent **pas** ta machine »). A `pas` between assertion and locality *cancels* a positive claim ("Je tourne sur un serveur, pas sur ton téléphone" — which is the family Cloud line this ticket puts in the prompt) and *completes* a negated one. One list with one rule would have to get one of those two wrong. Contrast conjunctions (`mais`, `but`, `however`) break the predication for both polarities; a sentence ending in `?` and a narrow set of self-hosting markers suppress it. The conditional suppressors are deliberately **narrow** — the exact fragments of the prescribed remedy, not a general modal list, because `peux`/`can`/`could` would make "je peux te dire que tout tourne en local" a one-phrase bypass.

**Predicate signature takes the deployment** (`detect_false_local_hosting_claim(text, deployment)`) rather than leaving it to a caller-side `if`, so "a declared local install may say it runs locally" is a property of the pure function and carries its own test. Single retry via `intent_guard_retries`; **not** skipped by `skip_remaining_guards` (#1178) — a successful PR review grants no licence to make a false privacy claim, the same literal reason as 5c. Applies uniformly across modes; a heartbeat that asserts local hosting is exactly as false, and the compacted history hands it to the next conversational turn. **Second violation in the same turn:** the budget is spent, the EndTurn is accepted, and a distinct `guard.false_local_hosting_claim_uncorrected` WARN is emitted — the gesture 4b already makes for the milestone-close guard, so the residual population stays countable instead of merging with healthy turns. Regression scenarios: `tests/eval/doctrine_regressions/false_local_hosting_claim_caught.rs` (the measured turn, the prescribed remedy as negative control, the declared-local exemption, the retry-exhaustion boundary).

5e. **Unactioned frequency-promise guard (mika#2358):** Refuses assistant text that promises to change how often the agent sends its own **unprompted** messages — or to suspend them — while the turn called no `set_config` on `proactive_daily_budget` / `proactive_pause_until`. Position 5e, immediately after 5d, whose shape, `intent_guard_retries` budget and `guard.*` telemetry it reuses. Same family, one step further along: 5c and 5d refuse a false statement **about the world**, this one refuses a **commitment about the future** the turn did nothing to bring about.

**Founding incident, 2026-09-17, Al's cloud tenant.** Asked why he had received three technical digests when he had asked for one, Mika answered « Je vais corriger ça concrètement : plus aucun message de veille technique aujourd'hui. Et demain, un seul — pas deux, pas trois. » and called nothing. **She had nothing to call**: the only gesture reachable from a conversation was cancelling the `heartbeat` row, which expresses "none" and never "one", and which `revert_config_cancel_recurring_task` undoes at the next restart (mika#2271). mika#2358's U1 gives the promise an actor; this guard is what makes the turn reach for it. The prompt half is deliberately absent — the framing of the heartbeat turn is untouched, per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`.

**Three terms, and the actor is checked FIRST.** (C) no `set_config` on either key among `all_tool_summaries`; (A) a frequency / suspension subject about unprompted messages; (B) a performative assertion. Checking (C) first is free, and it makes *"having called the tool is enough"* a property of the pure function rather than a branch of the loop — the same reasoning that put `deployment` in 5d's signature. A and B must share a sentence within `CLAIM_GAP_MAX` characters, in **either order**: French pronominalises the object it has just named, so « la fréquence de mes veilles, je **la** réduis » is the normal way to write the subject-first form, and the assertion alternation carries an optional clitic group for exactly that.

**Layer B carries its own grammatical subject** (borrowed from 5d, same reason): the vocabulary of the promise overlaps the vocabulary of the true sentence we want Mika to be able to say. « je ne peux pas régler ça moi-même » also speaks of settings and frequency and is structurally unmatched rather than specially excused — and admissions of incapacity additionally suppress at sentence scope. **That honest exit is written into the correction text as its second branch**, because a correction that only pushed towards the tool would make the model call it to be rid of the guard; the text says so in as many words ("do not call the tool merely to satisfy this message"). Layer A's `veille` requires its qualifier (`veille technique`): the bare noun is the ordinary French word for "the day before", and a guard that fired on « je vais corriger ça la veille de ton départ » would be refusing a sentence about a calendar.

**What term (C) accepts, and what it costs.** An **attempted** `set_config` satisfies it, success or failure — the `callback_terminal_action` convention. Named consequence: a promise resting on a call the tool rejected passes this guard. That belongs to the assert-grounded family (mika#1331), and widening the predicate to cover it would also re-prompt every agent that tried honestly and reported the failure.

Single retry via `intent_guard_retries`; **not** skipped by `skip_remaining_guards` (#1178) — a posted PR review changes no setting, the same literal reason as 5c/5d/6f. Applies uniformly across modes (a promise made inside a heartbeat turn is exactly as empty, and compaction carries it into the next one). Second violation in the same turn: budget spent, EndTurn accepted, `guard.unactioned_frequency_promise_uncorrected` WARN emitted — **expected regime zero**, and without it that residue would be indistinguishable from a healthy turn. Predicate units in `evidence::guards::tests::mika2358_*` (including the control that separates "reads the calls" from "reads a word": the same text passes once `set_config` is in the summaries); production-path coverage in `tests/eval/test_unactioned_frequency_promise_2358.rs` (measured turn, honest-admission negative control, factual-description negative control, retry-budget boundary). Config keys, the four-producer perimeter, operator greps and the post-deploy probe: root `CLAUDE.md` § *Une consigne de fréquence a un site d'inscription*.

5f. **Response language-drift guard (mika#2247 AC2):** Refuses assistant text measured in a language other than the one **this tenant declared** through the `language` key of `customer_config`. Position 5f, immediately after 5e, whose shape, `intent_guard_retries` budget and `guard.*` telemetry it reuses.

**Founding measurement, 2026-09-06, MikaSenara captures.** One thread flipped EN↔FR: « So — who are you… », « All good », then « Bonjour ! Je suis Mika… ». The tenant's persona (`FAMILY_SOUL`) already prescribed French **twice, once in bold**, and the drift happened anyway — which is precisely why a third sentence was refused as the remedy (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, mika#2120: nine recurrences under prompt enforcement against zero when the fact is posed by code). What was missing was a **declared** axis and a structural half, not another instruction.

**The discriminant is closed-list function words, and the reason is fail-open cheapness.** `measure_response_language` counts hits from two closed lists (`le la les de des un une et est dans que pour` / `the a an of and is in that for to`) and issues a verdict only when **three** independent thresholds clear: ≥12 word tokens, ≥3 hits for the winner, and a ≥2-hit lead. No new dependency, and function words are the part of a text a model does *not* vary — a proper noun, a code identifier or an emoji contributes to neither score. **Anything else is `Undetermined` and the guard does not fire**: « Bonjour 🌸 », « OK », « All good » must stay undecidable, because on a general-public tenant a false positive costs a needlessly re-prompted honest turn and a reply the person waits longer for. That is the expensive error here, not the missed drift.

`expected` is a parameter of the pure function rather than a caller-side `if`, for mika#2290's reason: "an undeclared tenant is never guarded" then carries its own test instead of living in one branch of the agent loop. **The third state is the common one** — every engineering agent and every un-configured tenant declares nothing, resolves `None`, and is byte-identical to the pre-mika#2247 behaviour.

Single retry via `intent_guard_retries`; **not** skipped by `skip_remaining_guards` (#1178) — a posted PR review does not license answering in the wrong language, the same literal reason as 5c/5d/5e. Applies uniformly across modes: a heartbeat that opens in the wrong language is exactly as wrong, and it is the turn that *starts* the exchange. Second violation in the same turn: budget spent, EndTurn accepted, `guard.response_language_drift_uncorrected` WARN emitted — **expected regime zero**. **What this guard does NOT do, written at the site rather than discovered:** a one-shot re-prompt does not *guarantee* AC2; it bounds it and makes the residue countable. If the post-deploy measurement shows a non-negligible residue, the remedy is an engine net (mika#2368 shape), **not a second re-prompt — and that is a ticket, not a setting**. Predicate units in `evidence::guards::tests::mika2247_*`; production-path coverage in `tests/eval/doctrine_regressions/tenant_register_held.rs` (measured drift, undeclared-tenant negative control, nominal turn). Config key, its four measured refusals of an env var, operator greps and the post-deploy probe with its three halts: root `CLAUDE.md` § *Le tenant grand-public tient son registre*.

5g. **Time-of-day greeting guard (mika#2247 AC3):** Refuses a greeting that names a part of the day the tenant is not in. Position 5g, immediately after 5f.

**It is a net, not the mechanism, and the two are tuned in opposite directions.** The measured symptom — a « belle journée » sent in the evening — had no instruction defect behind it: the prompt posed UTC and left the model **two untooled inferences** (convert to the zone, then derive a moment of day). The fix is therefore the *posed fact*: `write_time_section` now computes and renders `Local time (Asia/Singapore): 2026-09-06 20:14 / Sunday evening`, parsing both an IANA name and a fixed offset (the second because rows predating `validate_config_value` still carry `+08:00`, and reading only the first would make AC3 silently inert on exactly those tenants). This guard sits behind that fact, so its lexicon is a **closed, narrow list** where 5f's is a measurement. Two entries are deliberately permissive and say so: `bonjour` admits the afternoon (French uses it until the evening) and `bonne nuit` admits the evening (going to bed at nine is not an error).

**Fail-open on an unknown hour**, and `local_part_of_day` is taken by the pure function for 5f's reason. It is threaded from `prompt::resolve_local_part_of_day`, computed once per turn from the **same instant and the same timezone** the `## Current Time` section renders — a second parse would be free to disagree about what "evening" means, and the guard would then refuse the very greeting the prompt asked for. When no usable timezone is declared there is no local hour, so there is nothing to contradict; the prompt has already forbidden a time-stamped greeting on that path and states the ignorance rather than leaving the void that produced the symptom.

Single retry, not skipped by `skip_remaining_guards`, `guard.time_of_day_greeting_mismatch_uncorrected` for the residue — same contract as 5f above.

6. **Intent-precondition registry (#702):** Registry-driven guard that generalizes the webhook zero-tools guard (#696). `INTENT_GUARDS` is a const array of `IntentPrecondition` entries, each with a trigger function, satisfaction check, and correction message. Retry tracking uses `HashSet<&'static str>` keyed by label (one retry per entry). Current entries: (a) `webhook_ready_label_dispatch` (#846, #907, #1089) — if user message matches the `[GitHub] Issue labeled ready on` marker, requires `run_claude_pilot` attempt (dispatch via dev-pilot, or auto-groom via dev-groom). Post-#1089: the `send_message` grooming-rejection path was removed — all legitimate paths call `run_claude_pilot`; (b) `webhook_no_unauthorized_dispatch` (#910) — if user message starts with `[GitHub]` but does NOT match the ready-label marker, rejects when `run_claude_pilot` was successfully called (only successful calls — failed attempts are already blocked by the dispatch-readiness guard in `executor.rs`). Engine-level fix for recurring unauthorized dispatch from comment events (#798, #838, #910) where prompt-level source-check rules drifted under load. Post-#933: this post-hoc EndTurn guard is **defense-in-depth** — the primary prevention is the pre-hoc tool-boundary gate in `validate_dispatch_readiness()` check (0) which rejects `run_claude_pilot` before the subprocess spawns. Post-#1102: the trigger predicate now delegates to `is_unauthorized_webhook_dispatch()` from `crate::webhook_dispatch` — the same positive-allowlist predicate used by the tool-boundary guard. PR review and check-suite events (qa/ci skill territory) no longer trip the guard. Shared predicates live in `crate::webhook_dispatch` module; (c) `webhook_zero_tools` — if user message starts with `[GitHub]` and zero successful tool calls, rejects once (unchanged #696 behavior); (d) `resume_reconcile` — if user message contains resume/continue verb + milestone/project reference and no successful `check_task` or `list_tasks` call was made, rejects once; (e) `callback_terminal_action` (#870) — if user message starts with `[callback:` (Silent mode callback trigger), requires BOTH `update_task_status` AND `send_message` before EndTurn. AND-shape: both tools must be attempted (success or failure). Also has an inline mirror guard in the empty-text exit path for Silent mode, where the INTENT_GUARDS registry is not evaluated (the registry only fires in the non-empty text branch). `CALLBACK_TERMINAL_ACTION_LABEL` and `CALLBACK_TERMINAL_ACTION_CORRECTION` shared consts keep both sites in sync. **Two carve-outs, both read by both sites because they share the `callback_trigger_active` predicate:** `[callback:deferred-dispatch]` (mika#1011, own contract) and `[callback: long_running:build_mika]` (mika#2355 — a build callback owns no self_dev parent to mark terminal; the #870 audit's "only one callback flow exists" was false, `build_mika` was a second one, and imposing this contract on it let three mika-qa turns answer "Build succeeded" via `send_message` with no PR verdict on 2026-09-17). `build_callback_trigger_context` is the framing half of the same carve-out: it prescribes the self_dev terminal contract to every callback except a build callback, which is told its own (a posted `run_gh pr review`). The discriminant — label, message marker, satisfaction predicate, correction — lives in `crate::qa_build_callback` and nowhere else.
6a. **QA build-callback verdict guard (mika#2355):** Inline guard (not in `INTENT_GUARDS`), label `qa_build_callback_verdict`. Trigger is **conjunctive** — the user message starts with `[callback: long_running:build_mika]` AND `qa-review` is among the turn's injected skills (`loaded_skill_names`, a `run_loop` parameter threaded from all three call sites; in silent mode that is `callback_safe_skills()`, exactly where mika#2355 B1 made `qa-review-build-callback` reachable). Satisfied by a **successful** `run_gh` call whose input carries `"pr"` and `"review"` (`qa_build_callback::pr_review_posted_in_turn`, the same predicate as early-accept 3b). Single retry via `intent_guard_retries`, correction `QA_VERDICT_REQUIRED_CORRECTION` naming `run_gh pr review` and the `VERDICT:` line. Inline because the registry's `fn(&str) -> bool` sees the message alone, and the label does not distinguish mika-qa from mika-dev — mika-dev carries `build-mika` in its allowlist and launches builds that owe nobody a verdict; armed on the label alone this guard would trade the QA loop-breaker for a dev one (AC4b). Has an empty-text mirror in the Silent exit path like 6(e)/6b, because a bare EndTurn is the shape a turn with nothing to say takes. Coupled with the 6(e) carve-out above: the one removes the wrong contract from the build flow, the other supplies the right one. Production-path coverage: `tests/eval/test_qa_build_callback_verdict_2355.rs` drives `run_silent_agent` through both exit sites. What this guard does **not** do: post anything itself — a second EndTurn without a review is accepted (single-retry contract). What it now does on that path is **say so**: the turn raises `SilentTurnOutcome.qa_verdict_unmet`, and the engine-side `hold[review]` net reads it in the dispatcher (mika#2368 — see § *Verdict Net*, "The second reason"). The guard's one-shot budget is unchanged; the net succeeds it rather than extending it.
6b. **Callback milestone advance guard (#991):** Inline guard (not in `INTENT_GUARDS` const array) that enforces queue advancement on milestone/project-context callback turns. Triggers on `[callback:` + `[milestone-parent: <id>]` markers in the user message (the marker is injected by `run_silent_agent` after a DB lookup of the parent task type). Satisfied by EITHER Path A: `run_claude_pilot` call (advance to next child), OR Path B: `update_task_status` targeting the parent task ID with status `blocked`/`completed` (halt or finish). Inline because the satisfied predicate needs the parent_task_id from the user message. Composes with `callback_terminal_action` (entry e) — a milestone-context callback must satisfy BOTH guards. Also has an empty-text exit mirror guard. Companion `SilentTrigger::PostCallbackAdvance` fires a second advance turn if the first callback turn did not advance; auto-blocks the milestone if the second turn also fails. **Webhook companion guard (#1218, paired with #991):** Sibling inline guard for `pull_request.closed(merged:true)` webhook turns. Triggers on the `[milestone-parent: <id>]` marker prepended by `server::milestone_context_handler` when the PR-closed event correlates to a task with a `milestone`/`project` parent. Satisfaction has three valid paths: Path A (`run_claude_pilot` or `run_claude_pilot_groom` — advance), Path B (`update_task_status` on parent with `blocked`/`completed` — halt), Path C (`deploy_mika` + `send_message` — deploy-hook ack per self-dev-webhook-qa step 5.5.b). Mutually exclusive triggers with #991: the callback prefix `[callback:` and the webhook prefix `[GitHub] PR closed:` cannot both appear on a single user message. The marker parser `extract_milestone_parent_id` and the constant `MILESTONE_PARENT_MARKER` are shared with #991.
6c. **Asserted-unavailability guard (#862, #894):** Inline guard (not in `INTENT_GUARDS` const array) that detects when assistant text claims a tool is unavailable ("X is not callable", "X not callable", "I don't have access to X", "X is skill-scoped", "X skill-scoped", "cannot call X", "X is structurally not callable") while X is in the agent's turn-start enabled-tool set and no call to X was attempted in the turn. Five regex patterns with named `(?P<tool>...)` capture groups, normalized to lowercase for case-insensitive registry lookup. P2 and P4 use optional copula `(?:is )?` to catch elided-copula forms; P2 and P3 use optional adverb `(?:\w+ly )?` to catch adverb-interposed forms (e.g., "structurally", "currently"). Two-layer false-positive filter: snake-case capture constraint + enabled-set lookup. Inline rather than in the registry because it checks *assistant* text (not user input) and needs the `enabled_tool_names` snapshot + dynamic `format!` correction message. Uses `intent_guard_retries` with label `"asserted_unavailability"` for single-retry semantics. Not skipped by `skip_remaining_guards` (#1178) — a successful PR review does not grant license to fabricate tool unavailability claims. `enabled_tool_names: HashSet<String>` is a turn-start snapshot of the LLM tool array (after identity denylist + skill overrides + MCP), threaded to `run_loop` from all three call sites (conversation, silent, team). Structural counterpart to Rule 2 of `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`.
6d. **Assert-grounded guard (#1331):** Inline guard that detects affirmative state claims about referenced resources (issue/PR/task #N) without a grounding tool call (`run_gh`, `check_task`, `gh_read`) in the turn. Four regex patterns detect claim shapes (first-person verification claims, passive state assertions, handler/callback completion claims). Two-layer false-positive filter: narrow claim-verb + resource-type noun pairs, plus resource-ref extraction requirement (no ref → no fire). Satisfaction predicate checks `all_tool_summaries` for any attempt (success or failure) to a grounding tool with matching resource reference. Single retry via `intent_guard_retries` with label `"assert_grounded"`. Mirror of `asserted_unavailability` (negative → affirmative claims). Not skipped by `skip_remaining_guards` (#1178) — a successful PR review does not ground affirmative claims about unrelated resources.
6f. **Unacknowledged send-failure guard (mika#2136):** Refuses an EndTurn that closes over a `send_message` the engine watched fail and nothing repaired. Founding incident, 2026-09-01 on Al's Telegram: a 12 000-character document refused for length, answered with « Le voici en entier 👆 »; then a split whose part 1/4 died at the transport, with the agent moving on to « Partie 2/4 » without a word. **The model had been told, both times, with the exact figures in the `tool_result`** — which is what makes this the hardest of the three same-day tickets and what rules out a prompt line as the remedy (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`: nine recurrences under prompt enforcement against zero when the fact is posed by code).

**The information already existed; what was missing was a type and a reader.** `dispatch.rs` has always pushed `{name: "send_message", success: false}` into `all_tool_summaries`, and eleven guards read that vector at close — but `grep -c "send_message" crates/mika-agent/src/evidence/guards.rs` returned **0**. So this ticket does not create a fact, it qualifies one and reads it.

**The channel is typed, never the error prose.** `ToolOutput.delivery: Option<Box<DeliveryVerdict>>` (`tools/mod.rs`), built exclusively by `ToolOutput::delivery` and modelled on `substrate_diagnostic` (mika#1783): posted by `send_message` alone, **never serialized to the LLM**, `None` everywhere else. Boxed because `ToolOutput` is the `Err` variant of a dozen `builtin_handlers` validators and an inline verdict pushes it past `result_large_err`. Reading the English error message instead would have made a sentence meant for the model into a wire format, and the three `is_error` exits do not describe the same damage anyway — a length refusal is repaired by *splitting*, a transport death by *resending the same text*.

**The verdict carries the text, and that is a decision rather than a portability detail.** It is `cleaned` (post-`strip_internal_tags`) on all six exits, because `send_message` is the sole holder of the register that was actually measured and sent. `dispatch.rs` holds no usable text for a *failed* send: its only capture site is gated on `tool_succeeded`, reads the raw `arguments["text"]`, and truncates to 200 chars. A record rebuilt from it would be wrong in three directions at once — and the error leans the dangerous way: a retry re-emitting the same content with one internal tag more or less yields two different raw values for one `cleaned`, the repair goes unrecognized, and **a line asserting a loss goes out to a user whose message did arrive**.

**The predicate is entirely structural — zero lexicon, two stages.** `evidence::guards::undelivered_sends(&[DeliveryRecord]) -> Option<UndeliveredSends>`, a pure function on the turn's full sequence: a `Failed` with no *later* record of the same text `Delivered`; a `RefusedTooLong` with no later `Delivered` at all. `Delivered` is never a term on its own — only a repair — which is what makes the happy path free **by construction** rather than by precaution. `NoChannel` / `NoSender` are in the enum and outside the predicate: both return success by design (#650, regression-guarded by mika#1090) because they are permanent session conditions, so firing on them would reopen the retry loop that ticket closed; enumerating them makes that population countable the day it gets its own ticket.

**Satisfaction is an act, never an admission read from the text — the least intuitive point.** 5c and 5d do use bilingual lexicons, but to detect a *violation*, where failing to recognize costs a false negative. Here a lexicon would recognize a *satisfaction*: failing to recognize re-prompts every agent that told the truth in unexpected words, and recognizing wrongly waves through exactly the measured case. So there is no honesty detector. Accepted cost, paid on the failure path only: one extra LLM turn per unrepaired failure **including when the agent had already explained itself well** — `test_tool_calling::test_send_message_gateway_failure_surfaces_error` is its first concrete instance and says so.

**Three mirrors, and the third is the one that matters.** Single retry via `intent_guard_retries` (`UNACKNOWLEDGED_SEND_FAILURE_LABEL`), **not** skipped by `skip_remaining_guards` (#1178) — a posted PR review makes no message arrive to anyone, the same literal reason as 5c/5d/6c/6d. Sites: (a) the non-empty-text path; (b) the silent-mode empty-text exit, which is the shape a turn whose send failed typically takes; (c) **the send-message boundary (#771) exit** — a *fourth* `run_loop` return that fires on `stop_reason == ToolUse` and traverses no post-condition at all. Without (c) the measured sequence would still close in silence in the exact mode it happened in: part 1/4 dies, part 2/4 lands, the boundary arms on that success, and the turn ends with no guard consulted. On that site the model gets its turn to *say* what failed but cannot repair by resending (the boundary stays armed) — #771's decision about how many sends a turn carries, deliberately untouched.

**On an exhausted budget the engine writes the fact itself**, diverging from 5d, which settles for an `..._uncorrected` WARN. The divergence is paid for by the damage: for 5d an exhausted budget costs one more false sentence, visible in the log; here it costs **a document that never arrived and a recipient who believes it did**. `AgentOutput.undelivered_sends` (sibling of `deadline_exceeded`, stamped on all three exits including `MaxStepsExceeded` / `DeadlineExceeded`) reaches `server::handlers`, which appends a minimal factual line **before** `sender_arc.send` — the same neighbourhood and the same reason as mika#2276's net: in conversation mode the closing text *is* the channel, and it is the one point where the engine can land a fact without going through the model. On a mute turn the line **replaces** `EMPTY_RESPONSE_FALLBACK` rather than joining it. Two registers by exhaustive `match` on `PersonaProfile` with no `_ =>` (model: mika#2290) — `FAMILY_SOUL` forbids mentioning infrastructure, so the operator wording cannot be served to a family tenant, and serving nothing would leave the void that produced the claim. **The wording is past-tense and does not exclude a later arrival, deliberately:** a `SendOutcome::Failed` is saved to `failed_sends` for a later flush, so "was not received" stays true when written where "will not arrive" would be false — and that nuance is what stops a successful flush from reading as a false positive (see halt 4).

**Guarantee, and its three named false negatives.** The engine knows *that* a send left, never *that the refused content* did. (1) **Partial coverage** — splitting into four, sending two, saying "that's everything" passes. (2) **Extinction by an unrelated send** — after a refusal, one delivered "sorry, too long, here's a summary" puts the refusal stage out while the document still never left; pinned by `faux_negatif_epingle_refus_eteint_par_un_envoi_sans_rapport`, which **reddens and names the hole** the day someone closes it rather than letting anyone believe it was covered. A length floor was declined: the threshold would be arbitrary and its error leans toward asserting a loss that did not happen. (3) **Boundary suppression (#771)** — a suppressed send never reaches `execute`, posts no verdict, and is invisible; that is #771 deciding, not a delivery failure. The transport stage has none of the first two holes: its repair is a text's equality with itself.

**What the prompt keeps, and why it cannot be structural (AC5).** One sentence extending the Grounding rule, declared as the *intent* half and never as the guarantee: the engine can state a fact and refuse a close, but **telling the user what happened, in their language and their persona's register, is not something it can write** — mika#2290 established that one fact needs two formulations and that deriving the register from a technical field is a product choice in disguise.

**Operator surfaces.** `guard.unacknowledged_send_failure` (WARN, #953 family — `stage`, `failed_index`, `failed_count`, `guard_correlation_id`, joinable to `guard.correction_accepted`): **expected regime non-zero but low**; each line is a dead send the agent was about to keep quiet, and a sustained flow is not treated by widening the guard — it says the gateway or Telegram is refusing, and *that* is what to treat. `guard.unacknowledged_send_failure_uncorrected` (WARN + an `audit_events` row `tool_name = 'unacknowledged_send_failure'`): **expected near zero**; sustained on one agent means the model is not using the re-prompt, so the half to interrogate is the guard's, not the annex's. `send_failure_annexed` (INFO) + `SELECT count(*) FROM audit_events WHERE tool_name = 'undelivered_send_annexed';` — **SOLE WRITER** is `handlers.rs`, so its *absence* under a user complaint is itself information: it says the send did not fail where you think, and points at `NoChannel` or the gateway.

**Post-deploy probe, and its two halts.** Over 7 days, read the stage distribution: if `RefusedTooLong` dominates, mika#2134's length guard is being crossed daily and it is **its** threshold to re-interrogate, not this guard. *Halt (silence):* a user reports a document announced and never received while all three greps are **empty** → do not widen the predicate; the send succeeded from the engine's point of view and the loss is downstream (gateway, Telegram, mika#2126) or in the `NoChannel` population. *Halt (false positive):* the line appears on a turn where everything arrived → disarm the annex and repair the predicate, a false line in the user channel being damage of the same order as the one repaired — **but check the `failed_sends` flush first**: a `Failed` fragment later delivered by the flush arrives *after* the line, and that is not a false positive (the line was true when written). The discriminant is a flush entry later than the turn on the same content; only in its absence is the false positive real.

7. **Persistence evaluation guard (#648):** `detect_informational_input()` checks user input for informational signals (FYI, diagnostic, correction, status update) and `detect_persistable_output()` checks assistant text for verdict-shaped patterns (root cause, confirmed, validated, lesson learned). If no persistence write tool (`store_fact`, `update_fact`, `update_core_memory`) was called and either detection matches, nudges the model once to consider calling `store_fact`. Conversation mode only. Nudge, not rejection — the model can decline. `PERSISTENCE_WRITE_TOOLS` constant defines the write-tool set.
8. **Required-suffix-line guard (#864):** Manifest-driven guard for skill-declared output contracts. Skills opt in via `[output] required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]` in `skill.toml`. `collect_required_suffix_lines()` unions lines from `Keyword` and `AlwaysOn` matched skills (not `Dependency`). The guard scans the assistant's last 3 non-empty lines (after `trim()`) for an exact match to any entry; missing match rejects EndTurn once with a corrective re-prompt naming the accept-set. Standalone retry flag `required_suffix_line_retry_done`. Position: after persistence-eval — other guards' rejections take precedence. Silent mode passes empty `required_suffix_lines` (no enforcement). Currently opted-in: `mika-arch-second-review` (GROOMED/ESCALATE) and `mika-arch-groom-ticket` (READY/ITERATE/ESCALATE). Structural counterpart for the verdict-ghosting failure mode in mika#788.
9. **Required-finding-list guard (#901):** Manifest-driven guard for skill-declared F-list emission contracts. Skills opt in via `[output] required_finding_list_prefixes = ["F1:", "F2:", ..., "F10:"]` in `skill.toml`. `collect_required_finding_list_prefixes()` unions prefixes from `Keyword` and `AlwaysOn` matched skills (not `Dependency`). The guard fires only on terminal dispositions (ITERATE/ESCALATE/Verdict: ESCALATE) — `is_terminal_disposition()` checks the last 3 non-empty lines against both the skill's `required_suffix_lines` and a `TERMINAL_DISPOSITIONS` constant. Scan range: message start up to (exclusive of) the first line matching any `required_suffix_lines` entry. At least one line in the scan range must start with a declared prefix (via `starts_with`, no regex). Missing match rejects EndTurn once. Standalone retry flag `required_finding_list_retry_done`. Position: immediately after #864 suffix-line guard. Silent mode passes empty `required_finding_list_prefixes` (no enforcement). Currently opted-in: `mika-arch-groom-ticket` and `mika-arch-second-review`. Structural counterpart for the conditional-disclosure-evasion failure class (N=8 incidents, mika#901).

### Deterministic Context Injection

Skills with `[context.*]` sections have their data pre-fetched by the engine before the LLM turn. `resolve_contexts()` dispatches to engine-owned handlers by `context_type`, deduplicates across skills, and returns `ContextBlock`s. `apply_context_replacements()` performs single-pass `{{key}}` template substitution on skill prompts (injection-safe — replaced content is never re-scanned). If a `required = true` context fails, the declaring skill is excluded from the turn; if `required = false`, a sentinel message replaces the placeholder. Known types: `gh_pr_diff`. Module: `skills/context.rs`.

## Three-Layer Memory Model

- **Layer 1:** Core memory (always in system prompt, agent-editable via `update_core_memory` tool, 2500 token limit, 5 blocks: user_summary, self_model, current_priorities, key_people, workflows)
- **Layer 2:** Structured facts (People, Commitments, Preferences, Events — plaintext). Managed via `store_fact`, `update_fact`, `search_memory` tools.
- **Layer 3:** Hybrid search (FTS5 full-text + sqlite-vec cosine similarity via Reciprocal Rank Fusion). Optional OpenAI embeddings (text-embedding-3-small, 512 dims). Graceful degradation: hybrid -> FTS5-only -> LIKE fallback. Indexed on store_fact/update_fact, backfilled on startup.

**Per-agent override:** mika-arch sets `[context.summary] inject = false`, removing the *conversation summary* layer from its system prompt entirely (mika#1009 leak protection). New agents that disable summary injection should be listed here.

**Stop-signal convention (mika#1813):** the `stop_topic_*` preference key prefix (`prompt::STOP_TOPIC_PREFIX`) captures user requests to stop being re-nagged on a subject. When the user says "arrête" / "stop bringing this up" on subject X, the agent persists a `store_fact(category='preference', key='stop_topic_<slug>', value=…)` in the same turn. `AgentContext::load_agent_context` (`agent_loop/mod.rs`) fetches these via `search_preferences(STOP_TOPIC_PREFIX)` on every turn, filters the result through `prompt::filter_stop_topic_preferences` (strict `category` prefix — `search_preferences` is a substring LIKE over both `category` and `value`, so an unfiltered load would surface false positives and mute unrelated axes), and threads the filtered set into `PromptContext` (conversation) and `SilentPromptContext` (silent). `build_system_prompt` and `build_silent_prompt` render them as a `<stopped-topics>` block with an explicit "do NOT re-initiate on these" instruction. The state layer is the structural gate — the block re-appears on every future turn even if the model forgets, so the pattern degrades gracefully. Direct user questions about a stopped topic remain answerable (stop = don't re-initiate; question = respond normally). DB read is fail-open with a `stop_topic_load_failed` WARN log so a persistent DB failure is greppable instead of silently disabling suppression. **Compact-provider path — wired since mika#1925.** All three builders now render the contract; `build_compact_system_prompt` (`ProviderKind::MikaModel`) carries an abbreviated form of it. Three things a future editor needs, because each undoes itself if guessed:

- **The `persist` rule is unconditional there, and that is the fix.** The ticket asked only for a block gated on `stopped_topics` being non-empty; on that path the state was unreachable. The compact builder renders no `## Instructions`, so nothing told a MikaModel agent to write a `stop_topic_*` preference, and `build_silent_prompt` carries `consult` only — so the table stayed empty and a conditional block would have rendered never. Putting `persist` back under the condition restores a no-op with a green suite over it; `mika1925_compact_prompt_renders_persist_even_with_no_stopped_topics` is what reddens if someone does.
- **It was never a byte arbitration.** The compact sections totalled ≈ 437 bytes against the 5120 asserted ceiling before this change, and 934 after (1335 with three stopped topics) — still ~74 % unused. What bites is the section-count assertion and, behind it, the OOD completion mode the compact prompt exists to avoid, which was made of *markdown headings*. Form, not mass. The `persist` rule therefore carries no heading; the conditional `## Stopped Topics` section is the fifth, and raising that count again means naming the section, its ticket and what it guarantees, as the comment in `test_build_compact_system_prompt_size_bound` does.
- **AC4 is a blocking precondition, not a deferral.** mika#1925's calibration pass on MikaModel (mika#1190 discipline) is **not satisfied** and was not simulated — a mock validates prompt shape, never model obedience. It is not executable in this repo for three independent reasons: no MikaModel endpoint exists (`default_base_url` is an Ollama-shaped `http://localhost:11434` nothing serves); `calibration::providers::create_real_provider` returns `None` for any provider lacking a `MIKA_<PREFIX>_API_KEY` and exempts only `ProviderKind::Ollama`, so MikaModel cannot be instantiated without a key it has no use for; and none of the four role suites exercises a conversational agent or the stop-signal contract. **Before MikaModel serves a real tenant, run that pass** — those three dependencies are its cost, and they are a follow-up ticket's body.

**Open scope question, posed and deliberately not settled here.** Three sibling carve-outs on the same builder cite mika#1925 as their follow-up — `## Distribution Doctrine` (mika#1814), the hosting line (mika#2290), `## Mika Doctrine` (mika#2292) — which reads it as an umbrella, while its acceptance criteria name only `stopped_topics`, mika#1813's AC2 and mika#1813's pinning test. Both readings cannot be true. The three were left closed and their five pinning tests green and untouched: each carries its own cost analysis, and mika#2292's is written as an **accepted** cost, not a postponement, so closing them together would assume one answer fits four. Recommendation for the operator: three sibling tickets, one per carve-out, each carrying its own trade-off, and mika#1925 closed on its literal scope.

**Team-agent inheritance (mika#1926):** team-child prompts thread `stopped_topics` from the CHILD agent's own DB, not the orchestrator's — the decision on whether the operator's stop-signals should propagate down through team delegation is open under mika#1926.

## Context Injection Configuration

`[context]` section in `identity.toml` controls prompt-assembly behavior for context blocks. Each context block has its own nested subsection.

### `[context.summary].inject` (bool, default: `true`)

When `true`, the conversational summary (from compaction) is loaded from the DB and injected into the system prompt as `<context type="summary" trust="data">`. When `false`, the summary is **load-prevented** — `db.load_conversation_summary()` is not called, the summary is not deserialized, and is not available to any downstream code path in the turn. This is strictly stronger than injection-prevention and is the correct shape for context-leakage protection.

Use case: agents where the conversational summary is a known context-channel leak source (mika#1009). mika-arch is provisioned with `[context.summary] inject = false` by default.

```toml
[context.summary]
inject = false
```

### `[context.summary].max_tokens` (usize, optional, default: `None`)

Mode-conditional token budget for summary injection (Axis 3 — mika#1021). The field name is mode-agnostic; the gate condition (`SilentTrigger.is_some()`) lives in code. Orthogonal with `inject`: when `inject = false` (Axis 4), Axis 4 wins — `load_gated_summary()` short-circuits before evaluating `max_tokens`.

When set and the in-code gate fires (currently: silent-mode turns — callback, webhook, heartbeat, etc.):

- `Some(0)` → **load-omit sentinel**: summary omitted entirely on silent-mode turns. NOT interpreted as "zero-token cap."
- `Some(n)` for n > 0 → summary truncated to approximately `n × CHARS_PER_TOKEN_ESTIMATE` (= 4) characters before injection. A truncation marker `[… summary truncated to fit silent-mode budget …]` is appended so the model knows content was elided.
- `None` → no cap (default; current behavior).

Non-silent turns (conversation mode, CLI) are never affected by `max_tokens` regardless of its value.

Token approximation uses `CHARS_PER_TOKEN_ESTIMATE = 4` (heuristic, conservative for English). Truncation cuts at a word boundary via `truncate_to_token_budget()` in `prompt.rs`.

```toml
# Cap summary to ~1000 tokens on silent-mode turns, keep full on interactive turns
[context.summary]
inject = true
max_tokens = 1000

# Omit summary entirely on silent-mode turns
[context.summary]
inject = true
max_tokens = 0
```

## Session Configuration

`[session]` section in `identity.toml` makes an agent **single-session-by-nature** (mika#1401). `SessionIdentityConfig` in `prompt.rs` deserializes it.

### `[session].singleton` (bool, default: `false`)

When `true`, every conversational invocation (`mika ask`, `mika chat`, HTTP `/send`) reuses one canonical session instead of minting a fresh `Uuid::new_v4()` per ask. The single-session invariant is enforced by the engine, not by remembering to pass `--session-id`. Opt-in — absent or `false` preserves the default (random UUID per ask). An explicit `--session-id` always overrides the canonical session.

### `[session].canonical_id` (string, optional, default: derived)

The literal session ID to use. When absent and `singleton = true`, the engine derives `canonical-{agent_id}` — a prefix-typed sibling of `system-{agent_id}`, structurally exempt from `prune_old_sessions` (the `canonical-` prefix is not in its LIKE clause). `canonical_id` exists for mika-prime's operator-chosen zero-UUID (`00000000-0000-0000-0000-000000000000`), which is likewise pruning-exempt (no matching prefix).

```toml
[session]
singleton = true
canonical_id = "00000000-0000-0000-0000-000000000000"  # optional
```

**Mechanism:** `resolve_canonical_session_id(&Identity, agent_id) -> Option<String>` returns `None` for non-singleton agents (callers mint per-ask UUIDs), else the resolved ID. The `/send` handler caches it on `AgentState.canonical_session_id` at `init_agent` time; CLI paths resolve it per-invocation. Session creation goes through the idempotent `get_or_create_canonical_session` (`INSERT OR IGNORE`, mirroring `get_or_create_system_session`). Session teardown goes through `end_session_unless_canonical(id, canonical_id)`, which no-ops when `id == canonical_id` so the canonical session's `ended_at` stays NULL forever. In the TUI, `/clear` for a singleton agent clears only the display (gated by `App.is_singleton_session`) — the session ID and worker context are preserved.

**Compaction interaction:** compaction already keys on `agent_id`, not `session_id`, so a singleton agent is unaffected — the single canonical session simply accumulates all history. The silent/callback/heartbeat paths use their own derived namespaces (`callback-*`, `heartbeat-*`) and are untouched.

**Concurrency:** the singleton merges all channels into one thread. For a single-surface, zero-skill oracle (mika-prime's profile) this is the designed intent, not a defect. High-concurrency multi-surface agents should not opt in.

### Session contract of the architect passes (mika#2305)

**The contract already in force, on both halves.** *New by default:* `mika ask`
without `--session-id` falls back to the canonical session only when the agent is
singleton, and `resolve_canonical_session_id` returns `None` for a non-singleton
one. mika-arch declares **no** `[session]` block — `grep -c singleton
crates/mika-agent/src/well_known_agents.rs` returns `0` — so it mints a fresh
UUID per ask. *Continued on explicit request:* `_arch_ask`
(`skills/bundled/_shared/dispatch-lib.sh`) takes an **optional** `session_id` in
`$3` and passes `--session-id` only when it is non-empty.

**The three usages in `_iterate_groom_loop`, all deliberate:**

| call | `session_id`? | why |
|---|---|---|
| 1st pass `mika-arch-groom-ticket` | **no** | fresh session |
| UNPARSED retry | **yes** | the architect must see **its own prior turn** — the retry does not re-ask for the review, it asks the architect to *complete* its answer with the missing `Disposition:` line. Without the session the request is unintelligible: the carry-over is the mechanism's **condition of correctness**, not its contamination |
| 2nd pass `mika-arch-second-review` (both branches) | **yes** | continuing the session so findings stay in conversation memory, per that skill's session-continuity contract |

The session id is even written into the body-callout published on the ticket
(`_write_canonical_callout`).

**The leak mika#2305 suspected was real, and on the other axis.** It did not live
in `--session-id`; it lived in `HistoryScope::Agent`, the **default** of the
conversation window, where `rebuild_context(None, …, 20)` draws the agent's last
20 messages **across all sessions** — for an architect, twenty whole plans and
reviews. Closed by mika#2295 (the setting) + mika#2330 (putting it in force).
What mika#2305 adds is the means to *see* it: `context_window_assembled` now
carries `history_scope` next to `distinct_sessions`, because that count alone has
two causes of opposite sign (scope fell back to `agent`, or one session iterated
legitimately). Reading table and post-deploy probe: root `CLAUDE.md`
§ *Portage de contexte entre passes architecte*.

**Not bounded by `scope`, and deliberately left intact:** the agent-scoped memory
(`update_core_memory`, `store_fact`, `update_fact`, `search_memory`), which
crosses every session by design and is pinned as constitutive of being an agent
(`test_mika_arch_disabled_tools_excludes_agent_self_state`). Naming it is what
closes the ticket's question; pretending it does not exist would not.

**Guards:** `test-dispatch-lib.sh` pins the table above. If those assertions
redden, someone "fixed" the intra-invocation carry-over believing they were
closing mika#2305 — restore, and read the retry row.

## Tools

Each tool validates inputs. Control fields capped at `MAX_INPUT_LEN = 10_000` chars; payload fields capped at `MAX_PAYLOAD_BYTES = 200 * 1024` bytes (200 KB).

`ToolContext` contains `{ db, session_id, trace_id, home_dir, global_home_dir, core_memory_edit_count, is_onboarding, message_sender, embedding_client, brave_api_key, github_token, skills_dirty, is_reflection, is_task_context, is_callback_turn, provider_name, model_name, active_skill_paths, pr_review_posted, pr_reviews_posted, callback_task_id }`. `callback_task_id: Option<&str>` is `Some` for `SilentTrigger::Callback` and `SilentTrigger::DeferredDispatch` turns (carries the task ID for deferred dispatch registration and cycle detection, mika#1058), `None` otherwise. `global_home_dir: Option<&Path>` is `Some` for conversation mode, `None` for silent/team/delegate modes (blocks cross-agent file access). `active_skill_paths: &[SkillPathInfo]` lists skill prompt files already injected into the system prompt; populated in conversation mode, empty (`&[]`) in silent/team/investigate modes. `pr_reviews_posted: Option<&Arc<DashMap<String, HashSet<String>>>>` is `Some` in server mode (from `AppState`), `None` in CLI/test/silent/team modes — falls back to per-turn `pr_review_posted` AtomicBool (#821).

Tool trait uses `#[async_trait]` (Send futures). Per-tool timeout override via `timeout_secs()` default method (returns `None` -> uses 30s default). Shared `validate_and_resolve_path(path, base_dir, create_parents: bool)` helper in `tools/mod.rs` for path security (tilde expansion to base_dir, `~username` rejection, empty check, length limit, absolute rejection, traversal inspection, symlink check, canonicalize containment). Three-layer UUID validation chain in `tools/mod.rs` (#531, #596): (1) `validate_uuid(field_name, value)` — format-only via `Uuid::parse_str()`, returns `Result<Uuid, ToolOutput>` with structured JSON `{"error": "invalid_uuid", ...}`; (2) `validate_task_exists(db, field_name, value)` — format + DB existence + agent-scope, returns `Result<Task, ToolOutput>` with `{"error": "task_not_found", ...}` or `{"error": "db_error", ...}` (fail-closed); (3) `validate_task(db, task_id)` — calls `validate_task_exists` then layers trigger_type=manual + active status checks, returns `Option<String>`. Most task-accepting tools use layer 2 (`get_task`, `cancel_task`, `complete_task`, `update_task_status`, `check_task`); `delegate_task` and long-running dispatch use layer 3. `create_task` (`parent_task_id`) uses layer 1 only + `get_task_depth`.

**Cross-agent file access:** `read_agent_file`, `write_agent_file`, and `list_agent_files` accept an optional `agent` parameter for orchestrator-only cross-agent file access. `resolve_agent_home(agent_param, ctx)` helper validates permissions.

**Core memory path guard (#645):** `read_agent_file` rejects paths targeting core_memory sections with a domain-specific error before reaching `validate_and_resolve_path`. `is_core_memory_path(path)` matches `core_memory/` and `core-memory/` prefixes, bare section names (with or without `.md`), tilde/dot-prefixed variants, and exact directory names. Uses `core_memory_section_names()` from `db.rs` as single source of truth. The system prompt's core_memory preamble and tool-usage section also warn against reading core_memory via file tools (defense-in-depth).

**Context-redundancy guards (#647):** Pre-tool checks that detect when read tools request data already in the agent's context. Three guards extend the #645 pattern: (1) `read_agent_file` rejects paths matching active skill prompt files (`is_active_skill_prompt()` checks `ToolContext.active_skill_paths`); (2) `search_memory` hard-redirects `category="core_memory"` since core memory is always in the system prompt; (3) `search_memory` appends a soft hint when `category="all"` and the query matches a `core_memory_section_names()` entry. All guards use `ToolOutput::error()` for definitive redirects and hints prepended to success results for soft nudges. Path normalization shared via `normalize_path_prefix()` helper. Guard ordering: core_memory path guard → skill prompt guard → normal execution.

### Post-Action Hooks (mika#772)

`src/tools/post_action_hooks.rs` — Registry-driven side-effect callbacks that fire after specific tool calls succeed. Architecturally distinct from the EndTurn post-condition guard chain (§ Post-Conditions): hooks run AFTER a tool call's side effects commit, BEFORE the tool result returns to the LLM. Failure is warn-and-continue — hooks never affect the tool's own result or the agent loop.

**Registered hooks:**

| Hook | Tool | Fires when | Action |
|------|------|-----------|--------|
| Task completion auto-fact | `update_task_status` | `status="completed"` on non-milestone, non-project tasks | Emits `store_fact(category="event")` via `db.add_event()` with structured description (repo#issue, PR URL, turns, cost, task ID) |

**Wiring:** `run_post_action_hooks()` is called from `tool_execution/dispatch.rs` after `execute_tool()` returns, before tool-call recording and summary building. Uses `AsyncDatabase` directly (same write path as the `store_fact` tool), NOT re-dispatch through `ToolRegistry` (avoids recursive dispatch + audit_events double-counting).

**Milestone/project exclusion:** Iteration 1 excludes `type="milestone"` and `type="project"` tasks — existing prompt directives handle those. **Graceful degradation:** Missing `claude_pilot` metadata fields (hand-completed tasks) use `—` placeholders.

### Management Tools

14 tools for multi-agent/team workflows (`create_agent`, `list_agents`, `create_team`, `delete_team`, `update_team`, `add_team_member`, `remove_team_member`, `delegate_task`, `list_teams`, `run_team`, `get_team_status`, `get_team_history`, `create_task`, `update_task_status`). `create_agent`, `list_agents`, `create_team` always registered; others added when `agents.len() > 1 || !teams.is_empty()`. Orchestrator guards: only default agent or team-listed orchestrators can delegate/run teams; self-delegation blocked. **Task guard:** `delegate_task` and long-running skills require `task_id` referencing an active manual task. Per-tool timeouts: `run_team` (300s), `delegate_task` (120s).

**Delegate session persistence:** `delegate_task` creates a `delegate-{uuid}` session with parent linkage, persists task and response as messages. `AgentParams` has `global_home_dir` distinct from per-agent `home_dir`. **Team conversation continuity:** injects previous run context into orchestrator's system prompt. **Coverage check (#286):** `decompose()` re-prompts once if the orchestrator silently omits team members; falls through with `warn!` log (`team_coverage_gap`) on second miss. `TeamRun.coverage_retry_fired` bool persists via checkpoint JSON.

**Local-member reach: name-addressed, not `a2a_call` (mika#1653).** `delegate_task` is the **conversation-mode** local-delegation tool (name-addressed). In a **team run** there is no per-member reach tool: the orchestrator reaches members by listing them in the decompose-JSON task-assignment array (`[{"agent","task","output_file"}]`); the engine spawns each assigned member's session by name (decompose→spawn→resume) and members exchange results via workspace files. `a2a_call` is **suppressed from the team-mode tool array** (`teams::engine::build_team_tool_registry` removes the `TEAM_SUPPRESSED_TOOLS` entries from the team's private `ToolRegistry`; regression-gated by `test_team_registry_suppresses_a2a_call`). `a2a_call` is **remote-only** — external, cross-container agents addressed by URL; it has no local fast-path and 503s on local siblings (no gateway route exists for `~/.mika/agents/<name>`). Its description and SSRF-rejection error redirect local targets to `delegate_task` (ADR-009). Composes with mika#1652 (containment): this prevents the 503-loop, the reaper catches residual stuck `team_runs`.

**A run's terminal disposition is one compile-checked decision (mika#1940).**
`RunStatus::disposition(&self) -> RunDisposition<'_>` (`teams/types.rs`) is **the**
exhaustive match over the enum, with no `_` arm; `is_terminal_failure()` and the
failure reason are *derived* from it, never written beside it, which makes "it is
a failure ⟺ it has a reason" true by construction rather than true by test. The
constant `NO_DELEGATION_REASON` gives the field-less `FailedNoDelegation` variant
a single write site for the **operator** register — `engine.rs`'s
`team_runs.failure_reason` column and the CLI's `stderr`; `notification.rs` keeps
its own, deliberately different, **user** register (mika#2290/#2292's rule).

**The split it closed is between forms, not between authors.** `FailedNoDelegation`
(mika#1676) and `FailedTransport` (mika#1671) were added after the CLI was
written. The three exhaustive matches in this crate — `Display`, the DB-column
match in `engine.rs`, `notification.rs` — followed **both**; the three
hand-written patterns in `mika-cli` (`matches!`, two `if let`) followed **neither**.
`matches!` and `if let` are exactly the two forms that keep compiling when a
variant appears, i.e. the class mika#2023 M2 had to name in writing about
`tier == AgentTier::Family`. The remedy was not six more arms but taking the right
to enumerate away from the call sites: `mika-cli` now classifies through
`commands/team_outcome.rs`, which names no variant, and
`mika1940_no_hand_written_run_status_predicate_in_the_cli` (a `ProductionScanner`
source scan) refuses a new one — **with an allowlist shipped empty**, since there
is nothing to exempt, so no slot in which to drop the next lapse (mika#2323).
Verified by hand at delivery: a seventh variant fails to compile at exactly four
sites — `disposition()`, `Display`, `engine.rs`'s column match, `notification.rs`
— and **zero** in `mika-cli`. Exit-code and `stdout` contract: `crates/mika-cli/CLAUDE.md`
§ `mika ask`.

### Task Tracking

4 tools: **Write** (orchestrator-only): `create_task`, `update_task_status`. **Read** (all agents): `list_tasks`, `check_task` (with optional GitHub PR/issue status enrichment). Tasks reuse the `tasks` table with `trigger_type='manual'` + `action_type='none'`.

**`list_tasks` output enrichment:** Response includes a `Summary:` line with status-count breakdown (e.g., `"50 items total — 2 blocked, 48 completed"`) computed in-memory from the result set. Fully unfiltered calls also include a `Note:` with filter guidance discouraging redundant re-filtering. Filtered calls (by status or source) get a scoped summary but no guidance note. Per-task lines include `dispatch_class` when not the default `"implement"` (shows `class:groom` for groom-class tasks, #1172 W2). `get_task` always shows `Dispatch class:` in output (#1172 W2). See #572.

**Status transition state machine:** `pending` -> any; `in_progress` -> blocked/completed/cancelled; `blocked` -> in_progress/completed/cancelled; `cancelled` -> in_progress (cancel-and-retry path, mika#856). `completed` is terminal — status cannot transition, but metadata can still be written (#617). Cancelled tasks can be reverted to in_progress, reusing the original row instead of creating a new task; while cancelled, metadata writes (e.g. via `cancelled_reason`) continue to work.

**Phantom retry guard (#579):** `update_task_status` rejects retry-semantic metadata writes (any top-level key containing "retry", case-insensitive) when the task has an active callback child task (`trigger_type="callback"` in `pending` or `in_progress` status). Returns structured JSON error `retry_metadata_rejected_active_dispatch`. Fail-open on `get_child_tasks` DB error — the dispatch readiness guard (#525) is the primary defense against re-dispatch. Non-retry metadata writes are unaffected.

**Idempotent creation:** Deduplicates on `reference_url` (DB partial unique index `idx_tasks_manual_active_ref_url`) and on label (case-insensitive pre-check). Five loop-prevention guards. Max 25 agent-created items per session (configurable via `max_agent_tasks_per_session`, user_request exempt).

**Note on renamed scheduled task tools:** The former scheduled-task tools `create_task` and `list_tasks` have been renamed to `create_scheduled_task` (in `create_scheduled_task.rs`) and `list_scheduled_tasks` (in `list_scheduled_tasks.rs`) to avoid name collisions with the task tracking tools above. `list_scheduled_tasks` supports an optional `label_contains` string parameter for substring filtering (e.g., `label_contains: ":deferred"` to find pending deferred dispatch wrappers, #1172 W1).

### PR Merge Gate

`pr_merge_with_gate` builtin tool — structural backstop against merging PRs with failing required CI checks. Registered in `default_tools()` (all agents, including delegates). Returns a tagged-union `MergeGateResult` via `#[serde(tag = "action")]` — the LLM branches on the `action` field. Five variants: `merged`, `auto_merge_enabled`, `blocked` (with `BlockReason` sub-enum and backward-compat `failing_checks`), `already_merged`, `gate_errored` (with `GateErrorKind` sub-enum: `gh_cli_failure`, `credential_scope`, `network_error`, `parse_error`, `unknown`). Preflight `gh pr view` detects CONFLICTING/CLOSED/DRAFT before attempting merge (#794). **Credential-scope diagnostic (mika#1616):** when a `gh` call fails with a 403 / "Resource not accessible by integration" / forbidden response (GitHub App not installed on the target repo, or PAT missing write scope), `classify_credential_scope_error()` returns `GateErrorKind::CredentialScope { repo }` with an actionable detail naming the repo + remediation, instead of an opaque `gh_cli_failure`. Wired into all four `gh` failure sites (preflight, checks, auto-merge, immediate-merge). Mirrors the `classify_gh_error()` heuristic in `builtin_handlers.rs`. This stops the LLM from paraphrasing an opaque exit code into a fabricated cause (the reported symptom on mika-cloud PRs #135/#136). Decision matrix: CONFLICTING/DIRTY -> blocked[merge_conflict]; fail/cancel -> blocked[required_check_failed]; pending -> auto-merge; all pass -> immediate merge; already merged -> no-op; infra failure -> gate_errored. 60s timeout. Requires `ctx.github_token`. See #490, #794.

**Supervisor pr_url write on `auto_merge_enabled` (mika#1211):** On the `auto_merge_enabled` branch, the tool writes `$.claude_pilot.pr_url = "https://github.com/<owner>/<repo>/pull/<n>"` to the supervisor task's metadata (resolved via `ToolContext.callback_task_id → parent`, gated by `trigger_type='manual' && source='self_dev'`). This neutralises the orphan reaper's `pr_url IS NULL` predicate (#871) and arms the parent-completer (mika#1162), so the supervisor stays `in_progress` until the dispatch callback ages past `REAPER_GRACE_SECONDS` and is then promoted to `completed`. Mirrors `dispatcher::try_extract_callback_metadata` (#376): two-level shallow merge via `task_metadata::merge_metadata`, fire-and-forget on error. Conversation-mode invocations (no `callback_task_id`) skip the write silently. See `docs/solutions/best-practices/pr-merge-with-gate-supervisor-metadata-2026-05-20.md`.

### Issue Dependency Resolution

`resolve_issue_order` builtin tool — resolves dependency-aware execution order for a set of GitHub issues using `blockedBy` GraphQL edges. Input: `{ repo: "owner/repo", issues: [1, 2, 3] }`. For each issue, queries GitHub GraphQL API (via shared `github_graphql` module) for blocked-by relationships, builds a DAG, runs Kahn's algorithm with issue-number-ascending tiebreaker, and returns `{ sorted, edges, external_blockers, cycle }`. Cycle detection: if the DAG has a cycle, `sorted` is cleared and `cycle` lists the cycle members. External blockers (issues outside the input list) are tracked separately and do not affect the sort order. Fail-open: returns input order with a warning when no GitHub token is configured. 60s timeout. Used by the self-dev milestone workflow (M2b step) to order issues before dispatch. See #714.

**Shared `github_graphql` module:** `fetch_open_blockers()` and `extract_open_blocker_numbers()` extracted from `skills/executor.rs` into `crate::github_graphql` for reuse by both the blocked-by dispatch guard (#713) and `resolve_issue_order`.

### GitHub Read-Only Handler

`gh_read` builtin handler (#811, #817) — read-only GitHub CLI operations for the mika-arch architect agent. Input: `{"op": "<operation>", "target": "<number>", "repo": "owner/repo"}`. Five allowed ops: `issue_view` (→ `gh issue view --json`), `pr_view` (→ `gh pr view --json`), `pr_diff` (→ `gh pr diff`), `issue_list` (→ `gh issue list --json`, optional target as milestone number or label filter), `file_view` (→ `gh api /repos/{owner}/{repo}/contents/{path}?ref={ref}`, base64 decode, returns `{content, ref, path, size_bytes}`). Structured error variants: `NotFound`, `AuthFailed`, `RateLimited`, `NetworkError`, `MalformedRequest`, `FileTooLarge` (#817). `FileTooLarge` fires when GitHub returns empty content + non-zero size for files > `FILE_VIEW_MAX_BYTES` (1 MiB, matching GitHub's contents API cap). Files > 100 MiB hit GitHub's 403 response, classified as `AuthFailed` via existing `classify_gh_error()` — this is a pre-existing GitHub boundary, not a regression. `file_view` input validation: `path` charset-restricted to `[A-Za-z0-9._/-]` (prevents URL-decoding attacks), no `..`, no leading `/` or `-`; `ref` optional (defaults to `main`), no leading `-`, max 256 chars. Error classification via `classify_gh_error()` on `spawn_and_collect` output content prefix (`"Exit code:"` detection for non-zero exits). Audit log line `gh_read_invocation` with `agent_id`, `op`, `resource` (`<ref>:<path>` for file_view, target number for other ops), `repo`, `latency_ms`, `status`, `blob_sha` (file_view only — resolved file content sha from GitHub response, cost-free) fields. Auth reuses `ToolContext.github_token` (shared GitHub App installation). Declared in skill `tools.json` with `"handler": {"type": "builtin", "function": "gh_read"}`. See #811, #817.

### `run_gh` — GitHub CLI Handler

General-purpose `gh` CLI handler. Four-tier validation: (1) global subcommand allowlist (`GH_ALLOWED_SUBCOMMANDS`: pr, issue, run, workflow, release, repo, search, label, api), (2) skill-scoped scope gate (`validate_qa_review_gh_scope` for qa-review, mika#1196), (3) `gh api` per-method gating (`validate_gh_api_scope`, mika#1167). **`gh api` per-method gating (#1167, evolved from #805 + #1153):** `gh api` is in the global subcommand allowlist but further restricted via a method+path allow matrix (`GH_API_ALLOW_MATRIX`). Each entry defines an HTTP method + API path regex + rule name. The matrix is deny-by-default: any combination not matching at least one entry is rejected by `validate_gh_api_scope()`. Initial entries: 4 GET (branch, branches-list, commit, milestone) + 1 PATCH (milestone-update). Matrix compiled once via `LazyLock`. Audit event `gh_api_invocation` includes `allowed_by_rule` for structured anomaly detection. Adding a new endpoint requires adding a `GhApiAllowEntry` to the matrix — no other code changes needed.

**Destructive-action grounding gate (mika#1646, tier 4).** `validate_destructive_action_grounding` refuses `gh pr close` / `gh issue close` that cannot show it rests on the target's current state, and refuses a *repeat* close that does not acknowledge the prior one. Predicates live in `evidence::guards` next to their siblings assert-grounded (mika#1331) and equivalence-claim (mika#1645); the enforcement is here, in the pre-subprocess chain, and that placement is the whole point.

**Why not an EndTurn guard.** Its two siblings read assistant *text* and re-prompt — the right shape when the defect is a sentence. mika#1646's defect is the call. `gh pr close 1644` leaves at step 3 of the tool loop, so by the time the EndTurn arm runs the PR is closed and a re-prompt can only annotate an accomplished fact. Hence the pre-subprocess placement, alongside mika#1196 / mika#1682 / mika#1167 (and consistent with the Layer 4 note in `tool_execution::dispatch`: builtins like `run_gh` have no `data_grade`, so their gate lives in the handler).

**Layer A (AC1) — grounded, and said so.** The turn must contain a read of the target (`gh pr view --json files`, `gh pr diff`, `gh issue view`, or a qa/arch read path — resolved from `tool_calls` by `trace_id`), *and* the close comment must cite what the read showed. Both halves are required: the founding incident's two close comments each paraphrased an upstream verdict and cited nothing checkable, so a read-without-citation is indistinguishable from no read at all. The satisfaction predicate excludes the close call itself, or `gh pr close 1644` would ground `gh pr close 1644`.

**Layer B (AC2) — a second execution knows it is a second execution.** If the same close ran against the same target within `MIKA_DEV_REPEAT_ACTION_WINDOW_SECS` (default 1800), the comment must acknowledge the prior one. Detection queries the persisted `tool_calls` table **scoped to the agent** — not the turn, not the session. This is load-bearing, not incidental: the founding incident's second close (11:15:49Z) came from a deferred webhook replay of the 11:12:11Z event, a context sharing no in-memory state with the first close at 11:08:54Z. A session-scoped or turn-scoped check finds nothing there and waves it through. Verifying state before acting is also not enough on its own — between the read and the write, someone can reopen. Only the record makes the repeat knowable.

**Which way it fails.** Detection is fail-open: an argv not recognized as a close is none of the gate's business, so `gh` at large is untouched and the surface stays bounded to `pr close` / `issue close`. Everything after recognition is fail-closed: once the call *is* a recognized destructive action, any inability to prove it founded — missing grounding, unreadable history, `MIKA_STORE_TOOL_CALLS` disabled, DB error — is a refusal. That inverts `assert_grounded`'s deliberately lean-narrow fail-open policy, and the inversion is deliberate. A ticket left open in error is visible and gets corrected; a ticket closed in error drops out of the count and nobody goes looking for it. Refusal bodies are structured JSON (`error`, `doctrine`, `target`, `reason`, `remedy`) and name the unblocking gesture.

**Audit (AC3).** Every decision — refusal *and* authorization — writes an `audit_events` row with `tool_name = "destructive_action_grounding"` and `target_key = "<pr|issue>:close:<n>"`. No migration was needed: `audit_events` has no `event_type` column, `tool_name` is free-form TEXT, and this follows `phantom_aged_out` (mika#1712) / `wip_rescue` (mika#1852). Query: `SELECT * FROM audit_events WHERE tool_name = 'destructive_action_grounding'`. Operator grep signal: `destructive_action_blocked` in `$MIKA_SPIRIT_LOG_FILE` — any hit is a close the engine stopped; sustained hits from one agent mean an upstream verdict source is producing ungrounded close recommendations (the mika#1645 class).

**Regression coverage.** Predicate units in `evidence::guards::tests`, repeat-detection SQL units in `db::tests` (cross-session, window edge, `#164` vs `#1644`, pr-vs-issue, cross-agent), and the AC4/AC5 calibration scenario `destructive_action_thread_reground` (mika-dev role suite) which replays the PR #1644 timeline and fails the model both for re-closing *and* for declining without engaging the contradiction.

**Verdict↔flag coherence gate (mika#2237, tier 5).** `validate_pr_review_flag_coherence` refuses a `gh pr review` whose flag contradicts the `VERDICT:` line in its own body. Same family as mika#1646 above, same reason for being pre-subprocess: the defect is the call, not a sentence — by the time an EndTurn arm ran, the review would be on GitHub. Placed immediately after `validate_review_depth_present`; depth is a condition of the body, the flag a condition of the act.

**The failure, 2026-09-08.** mika#2218 restored `--approve` by giving the reviewer a machine identity distinct from the author. On the first review after that deploy — mika#2236, body `VERDICT: pass ✅` — mika-qa posted `--comment` and **never tried** `--approve`: argv `["pr","review","2236","--comment",…]`, zero attempt. The skill mapped `pass → --approve`; what overrode it was the agent's own memory of the 137 pre-fix self-approval refusals. Re-measured since: the same agent posted **three `APPROVED` reviews later that day**, so the defensive memory was an *intermittent* arbitration, not a stable blocker — which is the measurement that rules out a memory-side remedy (a tag or a dated invalidation assumes persistent state to correct) in favour of a per-turn one.

**The mapping has one reader, derived from `Verdict`.** `required_review_flag` maps `Pass → --approve`, `Block(_) | Hold(_) → --comment`, `Missing → None`. Never a table transcribed from `qa-review/system_prompt.md`: the truth is the enum `verdict_handler` already consumes, which refuses to merge a `pass` whose `state != approved` — the same contract read at the other end. **Not a style preference but the condition of correctness**: a hand-rolled `contains("VERDICT: pass")` would fail open on `VERDICT: pass ✅`, i.e. on the literal body of the founding incident, and be indistinguishable from a guard that works. Going through `parse_verdict` inherits mika#1828's emphasis tolerance and mika#2239's trailing-decoration fallback **and their bounds** — `VERDICT: pass — but see findings` stays `Missing` (mika#1821), and leading decoration stays out of scope (mika#2239 D-D) and is deliberately **not** worked around here, which would create the second reader. Both bounds are pinned as decisions in `evidence::guards::tests::mika2237`.

**A third bound, on the guard's own side: long flag forms only.** `PR_REVIEW_FLAGS` carries `--approve` / `--comment` / `--request-changes` and not `-a` / `-c` / `-r`, so a short-form call falls open. That matches the body reader, which has read `--body` and not `-b` since mika#275, so the two halves of the recognition engage on the same population — the long-form shape qa-review's own table prescribes (`system_prompt.md:607-611`). **Widening the list alone would be worse than the gap**: with no canonicalization a correct `pr review N -a` on a `pass` body reads as posted `-a` ≠ required `--approve` and is refused, i.e. a legitimate review made impossible to post — the one outcome R8 forbids. Closing it means a canonicalizing map applied to the posted flag *and* the body reader, never a longer list here. Pinned by `short_flag_forms_are_out_of_the_population_and_that_is_pinned`.

**Gated on the BODY, never on the active skill.** Its neighbour `validate_review_depth_present` gates on `!ctx.required_tool_arg_suffixes.is_empty()`, a proxy for "qa-review is loaded". This one gates on its own subject, so a human or ad-hoc review with no `VERDICT:` line is never blocked, and the guard does not vanish in silence the day qa-review reorganizes its manifest.

**Both directions.** The measured defect is a degradation (`pass` → `--comment`), which is conservative; its inverse — `block[…]` posted as `--approve` — would merge a blocked PR. `--request-changes` is the mapping of no verdict and is refused on any classified one.

**The escape hatch is the fix, not its softening.** A guard that always refused `--comment` on `pass` would turn a real constraint into an inability to post the review at all — the turn loops and dies. So `--comment` on `pass` is allowed **iff** a `--approve` on the same PR was attempted in the same `trace_id` and failed (`!row.success`, which covers GitHub refusing, `gh` failing to spawn, and an upstream gate refusing — the three A2 populations). That is also the ticket's second ask made structural: a refusal means "was going to degrade without trying" (stale memory), the hatch means "tried and the door was shut" (a real constraint). Those two used to be separable only by reading argv by hand.

**Fail-OPEN on the hatch, the inverse of mika#1646.** The term the history carries is *the absence of an attempt*, and a term that cannot be read is never a satisfied term (mika#2277). Refusing on an unreadable or empty history would produce the loop `--approve` fails → `--comment` refused → `--approve` fails… Named cost: `MIKA_STORE_TOOL_CALLS=false` makes the history empty, so the guard is **inert** on the `pass → comment` direction (the `block → approve` direction consults no history and is unaffected). Same inertia shape as `MIKA_LOG_PILOT_TRANSCRIPTS` for the mika#2249 reaper, made visible the same way: the abstention grep.

**The refusal leaves an exit that is not a lie.** Naming only "post with `--approve`" would push a model held by its memory to rewrite its *verdict* (`pass → hold[review]`) instead of its flag — the same defect under another name, and the guard cannot adjudicate which verdict is right. The body names both correct ways out and ends on the sentence that **is** the ticket's fix (c) made operational: *"a failure recorded in your memory is not evidence about THIS pull request."* That workaround stays open, named, and is watched by halt 3 below and asserted by the `memory_vs_skill_no_verdict_degradation` calibration scenario.

**A defect found while wiring the tests, and load-bearing for the hatch.** The mika#821 session dedup registered its key on any `gh` invocation that merely spawned — and `spawn_and_collect` returns `ToolOutput::success` even on a non-zero exit. So a `--approve` **refused by GitHub** consumed the right to post, and the fallback `--comment` was rejected as `duplicate_pr_review`: the hatch was decorative. The ledger now registers only reviews that landed (`!is_error && !has_non_zero_exit_prefix`), the same predicate `tool_execution::dispatch` computes for every tool, so the dedup ledger and `tool_calls.success` agree on what a successful review is. Registering a review nobody posted was wrong independently of this ticket.

**Audit + operator greps.** Every non-nominal decision writes an `audit_events` row with `tool_name = "pr_review_flag_guard"`, `target_key = "pr_review:{repo}#{n}"` and `after_value` in `{refused, degraded_after_attempt, abstained}`; audit writes are warn-and-continue so losing the row never changes the verdict. Grep signals: `pr_review_flag_refused` (WARN — **expected non-empty**: each line is a degradation the memory still pushed and the engine stopped, i.e. the measure of the remanence nothing gave before; a plateau rather than a decay says the memory is being re-written, and *that* is when the tagging follow-up opens, with a count rather than an intuition), `pr_review_flag_degraded_after_attempt` (INFO), `pr_review_flag_guard_abstained` (WARN — expected zero; check `MIKA_STORE_TOOL_CALLS` before touching the predicate).

**Downstream half.** `verdict_handler`'s `Verdict::Pass` arm used to return `Passthrough` in silence when `state != "approved"` — no WARN, no audit row, no counter, which is where mika#2236 sat and what cost the defect eleven days of invisibility. It now emits `verdict_pass_without_approval` (WARN + audit row), **SOLE WRITER** pinned by a source scan with an allowlist shipped empty. Behaviour unchanged — the merge-safety gate still stands; what is added is attribution. Complementary population of mika#2239's `verdict_approved_but_unclassified`, and the two must stay countable apart. **Expected regime: zero**; any hit is a review that got past the pre-subprocess guard, and the halt is to establish *which path* posted (another agent, `run_gh_subprocess`, a binary predating the fix — class mika#2340) before widening anything.

**Regression coverage.** Predicate units in `evidence::guards::tests::mika2237` (both directions, the nine inherited body shapes with three negative controls, the hatch open and shut, URL ↔ bare-number matching); production path in `tests/eval/test_pr_review_flag_coherence_2237.rs` (`MockLlmProvider`, no network — plus three negative controls, without which "the guard decides" would be indistinguishable from "the guard blocks", and an audit-row assertion separating "accepted by the hatch" from "accepted by abstention"); U2 in `tests/eval/test_verdict_handler.rs` with its approved-pass negative control; and the two mika-qa calibration scenarios `memory_vs_skill_precedence` / `memory_vs_skill_no_verdict_degradation` (real-provider swap gate, mika#1190 — a text proxy, stated as such). Compound entry: `docs/solutions/best-practices/une-memoire-apprise-dun-echec-survit-au-fix-de-cet-echec-2026-09-19.md`.

### Per-Tool Skill Budget (mika#2276 M1)

**A skill tool runs under the budget of the skill that DEFINES it**, resolved once
per turn by `agent_loop::build_skill_tool_timeouts` into a `tool name → secs` map
threaded to `tool_execution::dispatch`. Sibling of `build_skill_tool_map` and
`build_skill_data_grades`: same `matched` slice, same order, same last-write-wins
collision rule — a tool dispatched to one skill's handler while cut at another
skill's budget is exactly the defect this closed. `max_skill_timeout` keeps its
maximum semantics and is now only the fallback for anything absent from the map.

**What it replaced.** That maximum used to be applied *uniformly to every tool
call*, so an outer skill loaded for an unrelated reason raised everyone's ceiling.
`shell-exec` declares 30 s and owns `run_shell`; `build-mika` declares 300 s and is
a declared dependency of `qa-review` — so a QA review turn ran `run_shell` under
300 s. Measured on PR #2275, trace `921f11f0`: two `cargo test --release` calls of
**237,9 s** and **231,1 s**, 469 s of a ~506 s envelope, then
`agent deadline exceeded` at `steps_completed=5` with no verdict written. And
`qa-review/system_prompt.md:194` said literally *"against `run_shell`'s 30s
budget"* — the doctrine reasoned on a floor the engine never held
(`feedback_prompt_enforcement_fragile`).

**Detail worth knowing before tuning any of these numbers.** Every skill with a
large declared budget (`dev-pilot`/`dev-groom`/`address-pr-comments`/
`resolve-pr-conflicts` at 600, `build-mika` at 300, `deploy-mika` at 120) exposes
**only `long_running` tools**, which return from `execute_skill_tool` *before* the
timeout is applied (detached spawn + callback). Those budgets therefore never
protected their own tools; their only observable effect was raising other skills'
ceiling. A value whose sole effect is on somebody else is a value nobody re-reads
— which is why `qa-review/skill.toml` now declares `timeout_secs = 30` explicitly
even though 30 is the manifest default (mika#2276 AC5).

**Fan-out:** `run_loop` has three callers (conversation `mod.rs` ~3382, silent
~4291, team ~4956) and all three build and thread the map. Compound entry:
`docs/solutions/best-practices/un-budget-declare-par-un-manifeste-doit-etre-celui-applique-2026-09-10.md`.

### Verdict Net (mika#2276 M2, generalised by mika#2368)

`server::deadline_verdict` — when a turn **owed a verdict on a PR and did not post
one**, the engine itself posts `VERDICT: hold[review]` on that PR.

**Two reasons since mika#2368, and the question the module answers changed with
them.** It is no longer *"was the turn cut off?"* but *"did this turn owe a verdict
and fail to post one?"* — `VerdictReason::CutOffByDeadline` (mika#2276: the turn
never reached its conclusion) and `VerdictReason::CallbackConcludedWithoutVerdict`
(mika#2368: a QA build callback **concluded**, cleanly, posting nothing). The
reason decides the **body**, the **log line** and the **event name**; it decides
nothing else — target resolution, the anti-double-post registry, the 422
classification and the never-return-an-error discipline are shared.

The entry guard that used to read `overrun == None → NotApplicable("turn_completed")`
is gone, and its removal is the shape of the generalisation rather than a
relaxation: "the turn concluded" now describes *exactly* the second reason's
population, so it could not stay a refusal. It moved down into
`deadline_verdict_target`, the webhook call-site's entry decision, where it still
costs nothing on the nominal path (no token resolution, no log, no parse).

**The signal.** `AgentOutput.deadline_exceeded: Option<DeadlineOverrun>` (carrying
`steps_completed`) is stamped in `persist_deadline_fallback` — the one function all
three deadline gates (prelude, continuation-skip, `LoopResult::DeadlineExceeded`)
return through, so the flag cannot be set on two and forgotten on the third.
Architect Q3 chose a field over propagating `LoopResult`: `handlers.rs` already
consumes this struct, and widening the agent-core/orchestrator interface was too
much surface for a p1.

**Why `text` could not answer.** On deadline the loop persists a canned assistant
message — *"I'm sorry, that took too long"* — and returns it as `text` like any
other response, which `run_agent_for_message` then sends on the reply channel.
**That is the exact mechanism of the mika#2276 symptom:** Telegram notified on each
of four turns, PR silent, because the only path that posts a verdict is
`run_gh pr review` and the LLM never reached it. "The turn finished" and "the turn
concluded" were two different facts nothing in the code separated.

**`hold[review]`, not a new verdict (Q1).** The line already exists in
`qa-review/skill.toml` and `verdict_handler` already understands it (notify
operator, leave the task `in_progress`). A `block[timeout]` would have needed a new
branch in `verdict_handler.rs` — hence the CODEOWNERS gate — for a meaning
`hold[review]` already carries.

**Anti-double-post (AC3), two independent layers.** (1) The existing session-scoped
`pr_reviews_posted` registry, read for both key shapes `run_gh` can write
(`{repo}|{n}` and `__default__|{n}` when the call carried no `--repo`); the net also
registers its own post so a second pass in the same session cannot fire. (2) A POST
answering **422** is read as **idempotent success, never a verdict failure**
(architect Q4) — the in-memory registry does not survive a restart, and 422 is the
net for when it was lost. Classification is narrow on purpose: a bare `422` would be
too wide, and a genuine 403/404 must stay a failure or the outage goes invisible
again.

**Boundaries.** The POST is injected (`poster` closure) so the contract AC2 asks for
— *a verdict IS posted* — is assertable without touching GitHub; production wires
`run_gh_subprocess` with a PAT-first token (`Settings::resolve_github_token`, ADR-008
— posting a review is an operation whose author GitHub reads). Both call-sites use
that same canonical resolver, and deliberately **not** `resolve_periodic_scan_token`,
whose own doc-comment excludes in as many words the paths that require the machine
identity (PR review / merge). The net never returns an error: a net that fails the
webhook would replace a silence with an outage. It does not replace the
conversational fallback, which still goes out on the reply channel.

#### The second reason: a QA build callback that concluded mute (mika#2368)

**What it closes, written in the test mika#2355 itself left behind.**
`the_verdict_guard_fires_once_and_does_not_loop` says it verbatim: after the single
re-prompt, a second EndTurn with no review is **accepted** and `run_gh` was never
called — *"nothing was posted — the net's job"*. That is not a defect of the
`qa_build_callback_verdict` guard, it is its contract: every `intent_guard_retries`
guard has a one-shot budget, deliberately, because a guard that re-prompts for ever
turns a silence into a loop. Budget spent, what remains is an injunction the model
ignored twice and a mute PR — the exact shape
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` predicts.

**The signal travels up; it is not re-derived.** `run_loop` takes an out-param
(`qa_verdict_unmet: Option<&AtomicBool>`) posed on **both** EndTurn exit paths —
non-empty text and the empty-text mirror — under
`qa_build_callback::verdict_unmet_after_retry`, the exact complement of the guard
(same conjunction, budget term inverted). `run_silent_agent` returns it as
`SilentTurnOutcome`; `task_engine::dispatcher::post_callback_verdict_net` reads it
after a callback turn returns `Ok`, **before** the session's registry entry is
evicted. Not a `LoopResult` variant: that enum's exhaustiveness is a contract
forcing three external handlers to treat every *termination mode*, and "concluded
without posting" is not one — a turn can be `Done` **and** mute.

**Two exit sites, not one, and the second is the likelier.** The empty-text mirror's
own comment says why: *"a bare EndTurn is exactly the shape a turn that has nothing
to say takes, and it is the one the registry never sees."* A signal posed only on
the non-empty path would leave the net blind on the more probable half of its
population — and nothing would say so, because a blind net is silent, exactly like a
net with nothing to do. Hence one positive control **per site**
(`tests/eval/test_qa_callback_verdict_net_2368.rs`, T10a/T10b), verified red when
either site alone is unwired.

**The PR target is said, never derived.** `skills::executor::execute_long_running`
resolves it at spawn from `LongRunningContext.originating_message` through
`deadline_verdict::parse_pr_target` — the single reader of that grammar — and stamps
it on the callback row under `QA_REVIEW_PR_TARGET_KEY`, the same trajectory as
`metadata.dispatch_worktree_file` (mika#2249) and `metadata.pilot_transcript_expected`
(mika#2040). What is condemned is the **late** derivation: the one that would happen
at net time, when the failure is no longer recoverable and logs nowhere. Here the
resolution happens at spawn, its failure is logged on the spot
(`qa_review_pr_target_unresolved`), and **the net parses nothing** — it reads a
stamp. Fail-safe: no stamp, unreadable metadata, missing key, unparsable target →
zero POST and a line naming the abstention (`no_metadata`, `metadata_unreadable`,
`no_target_stamp`, `target_unreadable`). Four separate negative controls, because a
conjunction of fail-safe terms is not proven by neutralising all of them at once —
the mika#2277 lesson.

**The registry now reaches the silent path (AC7).** `agent_loop` posed
`pr_reviews_posted: None` with the comment *"Silent mode: no session-scoped dedup
needed"* while `skills::builtin_handlers` carried
`debug_assert!(ctx.pr_reviews_posted.is_some(), "…must be threaded for production pr
review calls")`. A QA callback that posts its review **is** a production pr review
call: the two statements had contradicted each other since the build callback became
a flow that posts reviews. AC7 corrects an inconsistency the source already
declared. The callback's session is fresh, so the registry carries exactly what that
turn posted.

**`hold[review]` and nothing else — a safety constraint, not a registry choice.**
`verdict_handler` routes `pass` to a **merge**. A net posting `pass` because the
build went green would merge a PR **no diff was ever reviewed on** — strictly worse
than the silence it replaces. Asserted on `DEADLINE_VERDICT_LINE` *and* by feeding
the produced body to `server::verdict::parse_verdict`: the constant alone does not
prove what the state machine will read.

**Kill-switch:** `MIKA_QA_CALLBACK_VERDICT_NET` (default armed; `0` disarms with no
redeploy). Not caution on principle — mika#2355's probe 2c prescribes *"disarm the
net before any diagnosis"* if a PR is ever merged unreviewed, and that prescription
is only executable if the lever exists. An unrecognised value is **said** and leaves
it armed: a disarm by typo on a safety net would be the silent failure this whole
ticket closes.

**Operator grep signals — two names, and that is what saves the probes.**
`qa_deadline_verdict` (reason `CutOffByDeadline`) and `qa_callback_verdict` (reason
`CallbackConcludedWithoutVerdict`), each **SOLE WRITER** of its own name in the log
and in `audit_events`, pinned by a source scan
(`mika2368_each_event_name_has_exactly_one_writer_in_production`) — a behavioural
test cannot see that class, since a second writer would make no decision wrong, only
the two populations inseparable. mika#2355's negative-control probe is literally
`grep qa_deadline_verdict … | jq 'select(.outcome == "posted")'` — *the mika#2276 net
must not start firing*; a shared name would merge two populations and make a `posted`
of the new reason read as a regression of the old. Same principle as
`phantom_aged_out` / `phantom_sweep_spared` (mika#2156) and `auto_pull_no_token` /
`wip_rescue_no_token` (mika#2205). Both carry an `outcome` field in
`{posted, already_reviewed, already_posted_upstream, post_failed, no_token, no_registry}`,
plus, for the callback reason, `disarmed` and the four abstention motives above.
Steady state for `qa_deadline_verdict` after M1 is zero lines. For
`qa_callback_verdict`, **near zero is the contract**: a net carrying nominal traffic
has replaced a silence with a systematic `hold[review]`, which means mika#2355's
B1/B2/guard did not take — and *that* is where to look, not in the net's tuning.
*This is a net, not a path*; if it carries the nominal traffic it has also erased the
signal that would show it.

### Structural Verdict Handler

`server::verdict_handler` — intercepts `pull_request_review.submitted` webhook events **before** the LLM turn in `handle_message`. Parses `VERDICT:` line from the review body (authoritative regardless of GH `review.state`). Full dispatch table: `pass` (state=approved only) → merge via `run_gh_checks` + `run_gh_merge`; `block[ac]` → dispatch claude-pilot with AC-fix prompt, bounded retry counter (max 3), escalate on limit; `block[ci]` → dispatch claude-pilot with CI-fix prompt, bounded retry counter (max 3), escalate on limit; `block[security]`/`block[pipeline]` → mark task blocked, notify operator, NO auto-dispatch; `hold[review]` → notify operator, leave task in_progress; missing/unparseable → safe-default hold[review] semantics + `verdict_classification_failed` structured log event. AC extraction from qa-review's `[❌] unsatisfied:` lines with 2000-char fallback. Pre-digests for all verdict classes avoid completion-claim guard trigger words. Shared helpers: `find_task_for_verdict` (task lookup + in_progress gate), `has_active_callback_child` (in-flight guard), `send_notification`, `truncate_body` (UTF-8 safe). Parser in `server::verdict` depends on gateway's `format_event_text()` output format. 60s timeout on subprocess calls. See #524, #889.

### Structural CI Success Handler

`server::ci_success_handler` — intercepts `check_suite.completed(success)` webhook events **before** the LLM turn. Companion to `verdict_handler`: re-evaluates merge eligibility for PRs that have a pending `VERDICT: pass` but were blocked on CI at approval time. Queries GitHub API for open PR, QA pass review, stale-SHA gate (`review.commit_id == pr.head.sha`), and CI aggregation via `run_gh_checks` + `classify_checks`. Reuses `VerdictAction` return type and `pr_merge_with_gate` helpers. Order-independent with `verdict_handler` — each handler self-selects on event type. 60s timeout on subprocess calls. See #571.

**It evaluates; it does not merge (mika#2248).** On every gate cleared it emits a `MergeReadySignal` and returns; the merge belongs to `server::merge_ready_handler` (next section). The reason is the routing table, not style: `check_suite.completed(success)` routes to `mika-dev` **and** fans out to `mika-qa` (`secondary_targets`, mika#1711), both run this handler, and a `gh pr merge` issued here runs under the token of whichever agent won the race. Measured 2026-09-08 on mika#2244 — `mergedBy = mika-platform-qa`, the reviewer closing its own approval, with no handoff to the dispatcher that owns the cycle. The absence of a `run_gh_merge` callsite in this file is asserted by `tests/eval/test_ci_success_handler.rs`, and `tests/eval/test_merge_identity_2248.rs` asserts the gates still precede the signal. **Operator grep signal:** `ci_success_merge_ready` (INFO + `audit_events` row keyed `pr:{repo}#{n}@{head_sha}`). Note the prompt layer was already right here and could not help: `qa-review-webhook-success` tells mika-qa "never … merge tools", and mika-qa's allowlist carries neither `self-dev-webhook-ci` nor `self-dev-webhook-qa` — this handler runs before the LLM turn and is unaffected by either.

**Burst dedup (mika#1869):** a single push fans out to up to 8 workflows, each firing its own `check_suite.completed(success)` webhook → N redundant walks of the full handler path that saturated mika-qa's mailbox (41 `rate_limit_trip`/h baseline). Two `head_sha`-keyed layers, placed right after `find_open_pr` resolves `head_sha`, collapse the burst to one merge evaluation: (1) an in-memory precise gate (`server::check_suite_dedup::try_dedup_check_suite`, bounded 1000-entry DashMap, 10-min TTL, 60s window) and (2) an audit-durable gate (`ci_success_handler_processed` marker rows queried via `count_recent_audit_events_for_target`, keyed `pr:{repo}#{n}@{head_sha}`) that survives a process restart the in-memory map cannot. Both key on `(repo, branch, head_sha)`, so genuine distinct pushes (new `head_sha`) are never conflated; the audit read fails open. **Operator grep signals:** `ci_success_dedup.skip` (in-memory gate fired) and `ci_success_handler.dedup_skip` (audit gate fired) in `$MIKA_SPIRIT_LOG_FILE`.

### Merge Actor (mika#2248)

`server::merge_ready_handler` — consumes the `MergeReadySignal` emitted by `ci_success_handler` and performs the merge. Wired in `handlers.rs` **immediately after** that handler: the signal travels in `req.text` (the evaluator's pre-digest), so a handler inserted between the two would rewrite the text and carry the signal away — the ordering is asserted in `tests/eval/test_merge_identity_2248.rs`.

Four conditions, all necessary, in order: (1) a re-parseable signal (`mika_common::forge_identity::parse_merge_ready_signal`); (2) the running agent is not the reviewer named **in the signal** (`would_merge_as_reviewer` — AC1 enforced at runtime, not only in a routing table); (3) the running agent is the dispatcher (`merge_disposition`, an **allowlist** — an unknown agent holds, it does not merge); (4) the perimeter re-verified here, fail-closed and bounded at 60s, so a signal arriving by any other path still cannot close a DECISION-CORE PR. The merge then runs under the running agent's own token, which is what makes `mergedBy` deterministic.

The policy itself lives in `mika_common::forge_identity` — shared with `mika-gateway`, whose `QA_REVIEWER_LOGIN` is the same constant, because the login that decides who may *review* is the login that must not *merge*.

**Why the signal is not a `merge-ready` label.** Two measured reasons. Permission: a label write under the resolved PAT fails (`Resource not accessible by personal access token (addLabelsToLabelable)`, 29 refusals measured 2026-09-07, mika#2228), so the signal would need a second token and therefore a second failure mode on the critical path. Authority: a label is human-writable, which would turn an interface gesture into a merge authorization able to bypass the DECISION-CORE gate.

**Operator grep signals** (`$MIKA_SPIRIT_LOG_FILE`, each with an `audit_events` row of the same name): `merge_ready_hold_reviewer_is_not_merge_actor` (the AC1 refusal — the shape mika#2244 measured), `merge_ready_hold_not_dispatcher` (any other agent holding), `merge_ready_human_gate_required` (perimeter hold at the actor), `merge_ready_merge_initiated` (the merge fired; the `ci_success_merge` audit row moved here from the evaluator). The tool-side companion refusal is `pr_merge_with_gate_reviewer_refused` — `pr_merge_with_gate` blocks the reviewer at step 0, before input validation and before any `gh` call, and returns `blocked` / `reason.reason = "reviewer_cannot_merge"` (the eighth `BlockReason`, carried in the three `self-dev*` prompts' taxonomy).

### QA-Review Reconciler (mika#2334)

`qa_review_reconcile` — the fourth recurring scan, next to `auto_pull_groomed`
and `wip_rescue` in `task_engine/dispatcher.rs`, carried by mika-dev, cron
`0 */15 * * * *`. It asks `mika-platform-qa` for a review on the loop's open PRs
that nothing came to review. **Outside the LLM, outside the pilot session,
triggered by time rather than by an event** — which is the whole point.

**Two measurements moved the diagnosis, and they are worth keeping.** The
founding ticket read two reviewerless PRs (2026-09-15) as a trailing
`gh pr edit --add-reviewer` the pilot died before reaching. (M1) **That step did
not exist anywhere in the repo** — the exhaustive search returns one hit and it
is a flag-arity table; `dispatch-lib.sh`'s eight `gh pr edit` carry only
`--add-label` / `--title` / `--body`. (M2) **The review never depended on the
pilot**: `mika-gateway/src/github.rs` routes `pull_request.opened` to mika-qa
with no draft filter, so creating the PR starts the cascade. The real defect is
that `opened` is a **single, non-replayable event** — droppable head-of-line by
the mika#1870 queue at saturation, then DLQ and `dead` behind the gateway
circuit breaker, then lost to a second empty turn that `webhook_zero_tools` only
opposes once — and **no path re-read an open PR without a review**. `auto_pull`
works on issues, `wip_rescue` only on `wip-rescue`-labelled drafts,
`curator_review` on skills; the mika#1711 `check_suite.completed(success)`
fan-out is the one existing catch-up and it requires `draft: false` **and** a
green CI.

**Why the letter of the fix was declined.** An *unconditional* reviewer posted at
PR creation double-reviews **every** PR of the loop: `opened` starts a session,
the gesture emits `review_requested` on exactly `REVIEWER_FORGE_LOGIN` — so
mika#1655's `is_suppressed_review_request` lets it through — and the second
session cannot see the first (`gh pr view` is outside `QA_REVIEW_GH_ALLOWED`, and
the first review is not posted yet anyway). The duplicate would be nominal, not
exceptional — the class #886 closed once already. Hence: the gesture must be
**conditional on the absence of a review**, and so it cannot live in
`dispatch-lib.sh`, whose tail delivers its callback and dies in seconds and
cannot observe an absence only measurable after a delay. The anchor there is real
(`_post_flight_recovery`, `PR_URL` resolved, `_stamp_pr_origin` already called)
and is declined for that reason alone.

**Shape.** `select_prs_needing_review(&[PrSnapshot], now, &ReconcileConfig)` is a
pure function carrying the entire decision; the `gh` execution is a thin caller.
Six conjunctive terms, each fail-safe — unreadable information takes a PR **out**
of the population, never into it (deleted author, unparseable `createdAt`, a
future timestamp clamped to age 0). **The pilot's termination is not a parameter
of that function**, which is the structural form of "independent of the pilot's
survival". Deserialization is part of the guard: `review_requests` and `reviews`
deliberately carry **no** `#[serde(default)]`, because a defaulted absence would
read as "no request, no review" and *admit* a PR on missing information.

**Sole writer** of the `qa_review_reconciled` audit `tool_name`, which is what
makes `SELECT … WHERE tool_name = 'qa_review_reconciled'` the exact list of PRs
the loop had to catch up on — and therefore the measure of the nominal path's
health. This is a **net, not a path**: if it carries nominal traffic, the upstream
loss is the subject and the net is now hiding the signal that would show it.

Token via `Settings::resolve_github_token` (PAT-first, App fallback — posting a
reviewer is not an operation whose author GitHub reads in the ADR-008 sense), and
the mika#2205 structural guard covers this fourth scan. `PeriodicScan` gained a
variant, so `mika2334_every_scan_variant_is_covered` makes the next one fail to
compile until the guards enumerate it. Config, kill-switch and the operator
grep signals: root `CLAUDE.md` § *Optional (QA-review reconciliation — mika#2334)*.

#### The ledger decides (mika#2347)

**What mika#2334 left open, and it is readable in the code alone.** Its only
idempotence terms were *no request* and *no review* for `REVIEWER_FORGE_LOGIN` —
two **GitHub states that exist only once the review has concluded**. A PR whose
review turn dies without posting anything therefore fell back into the population
on the next tick, identical to itself: with the `0 */15 * * * *` cron and the
then-default cap of 3, up to **96 re-requests per day per PR** across the seven
days of `MAX_AGE`. The `qa_review_reconciled` audit row was written and **never
read back** — the ledger existed and decided nothing.

**The key carries the head SHA.** `PrSnapshot` gained `head_ref_oid` (requested in
`--json`, **no `#[serde(default)]`**, empty value treated as unreadable and takes
the PR *out* of the population), and the audit key became
`pr:{repo}#{n}@{sha}` — the exact form `ci_success_handler` already uses for its
durable dedup (mika#1869). The operator query is unchanged and the `pr:{repo}#{n}`
prefix is preserved. **A new SHA reopens the budget by itself**: the key changes,
the count restarts — the "per (PR, SHA)" idempotence obtained from the shape of
the key rather than from one more column.

**Two bounds, read through `count_recent_audit_events_for_target` — no new DB
method, no migration.** `review_ledger_verdict` answers `Postable` / `Cooldown` /
`Abandoned{attempts}` / `Unreadable`: a pose inside `COOLDOWN_SECS` skips the PR
for that tick; `MAX_ATTEMPTS` poses inside the `MAX_AGE` window abandon that SHA
for good. **The budget is what separates this fix from a slowdown** — a cooldown
alone replays for ever, just less often. Direct precedent, cited in the code:
`MIKA_AUTO_PULL_MAX_REDRIVES` (mika#2020), born of mika#1901 receiving the `ready`
label sixteen times in nineteen hours. Legacy rows (`pr:{repo}#{n}`, no SHA) are
consulted by a **second exact query**, never a `LIKE` — which would match `#234`
against `#2343`; that query is dated in the code and can go once every pre-fix row
is older than `MAX_AGE`.

**Fail-closed, the inverse of `ci_success_handler`.** An unreadable
`audit_events` refuses the pose (`qa_review_reconcile_ledger_unreadable`, WARN,
**must stay empty**). Same choice as `wip_rescue` (mika#2199) and for the reason
written there: a false negative makes a PR wait, which is what it did before
mika#2334 anyway; a false positive replays a review, i.e. produces the very defect
this closes.

**The truncation moved, and that is a contract change.** `select_prs_needing_review`
no longer truncates to `max_per_tick`; the caller truncates **after** the cooldown
filter. Left upstream, `max_per_tick` PRs in cooldown would consume the whole tick
and a fourth, legitimately recoverable one would never be seen — a cap on *skips*
instead of a cap on *writes*. The tick budget is debited on **attempted poses**
only; the inverse rule already written (a *failed* pose does debit) is unchanged.
`MAX_PER_TICK` default drops **3 → 1**: 4 requests/hour against a measured
capacity of 6 reviews/hour (600 s envelope, serialized execution), the one setting
here whose effect is arithmetically demonstrable.

**Serialization (AC4) is an engine invariant, pinned rather than implemented.**
The scan triggers no turn: it posts a reviewer, and the resulting
`review_requested` falls into the mika#1870 bounded queue, drained by **one worker
per agent** taking `agent_lock`. Writing a concurrency lock inside
`qa_review_reconcile` would be a placebo — the scan holds no turn.
`test_qa_review_reconcile_2347.rs` pins it two ways: behaviourally (two
`review_requested` never coalesce and drain in order) and structurally (one
`spawn_webhook_drain_worker` definition, exactly two spawn sites, and
`run_agent_for_message` *receiving* the guard rather than taking it). A behavioural
test alone cannot see that class: a second consumer would make no decision wrong,
it would lift the invariant in silence.

**What this fix does NOT prove.** The ticket's hourly evidence is not compatible
with the reconciler as the sole source of the churn — two `hold[review]` eleven
minutes apart cannot come from a fifteen-minute scan, and the first `hold` **is a
posted review**, which takes the PR out of the population on the next tick.
`pull_request.synchronize`, the `check_suite` fan-out and a queue replay remain in
play and are out of scope. This reduces the number of reviews *requested*; it makes
no turn faster.

**What kept it inert for a day, and where that is fixed: mika#2337.** The scan
shipped complete — the registration literal and the match arm are two literals of
the same commit — and never ran. Its first fire failed on
`unknown run_skill trigger: qa_review_reconcile`, which only a binary *older* than
that commit can emit, and the death did the rest. See § *Unknown-Trigger Veto Lift*
below; the root cause (merged code ≠ running code) is not in this repo and is
tracked as follow-up.

### Hot STOP for `auto_pull` (mika#2329)

`crates/mika-agent/src/auto_pull_stop.rs` — **sole reader** of the sentinel file
`~/.mika/state/auto-pull-stop`. Its existence stops the feeder at the next tick
(≤ 10 min, `AUTO_PULL_CRON`); removing it resumes it. The content is never read.

**Why a file and not the env var the ticket proposed first.** `mika-spirit` calls
`load_dotenv` **once** at startup and nothing watches the file afterwards; a live
Linux process's environment is not mutable from outside, so **editing `~/.mika/.env`
changes nothing about what `std::env::var` returns**, even called every tick.
Re-reading the variable at tick time would have moved the defect one notch and made
it *harder* to see, since the code would then look like it re-reads. And the house
deliberately freezes its `MIKA_*` variables (four are documented "not
hot-swappable"): making one of them hot would create an invisible exception between
two variables nothing distinguishes — notably `MIKA_DEV_WIP_RESCUE`, the twin form
that does not re-read.

**The row is never touched, and that is the design.** The short-circuit sits at the
**head** of `dispatch_auto_pull_groomed`, before `resolve_periodic_scan_token` (two
token resolutions, one potentially a GitHub App exchange over the network) and
before the two `gh` fetches — same placement reasoning as mika#2279's gate 2c. The
recurring row keeps ticking; only its dispatch returns early. **Reversibility is
therefore not machinery but the absence of machinery**: no contact with the
mika#1742 anti-zombie guard, the mika#2271 config-cancel exemption, or
`RECURRING_ZOMBIE_GRACE_HOURS`. The boot-time knob, by contrast, *cancels* the row —
which is precisely what cost mika#2271 and its still-present repair machinery.

**The WARN is what actually closes the incident.** On the non-short-circuited path
(the predicate's population — if the process had booted with the knob, the row would
be cancelled and no tick would run at all), `stale_env_knob` reads the `.env`
**files** — per-agent then global, the mika#2218 order — and emits
`auto_pull_stop_stale_env_knob` when one carries `MIKA_DEV_AUTO_PULL=0`. Without it
the defect stays open in its most dangerous form: **a silently inoperative STOP reads
exactly like a STOP that works.** Inverse mirror of mika#2205.

**SOLE WRITER** of `tool_name = 'auto_pull_stop'`, one row per **transition**
(`armed` / `lifted`) — never per tick (mika#2131 doctrine). The INFO line, by
contrast, fires on every short-circuited tick: for a switch, liveness *is* the
information (Signal P precedent, mika#2156). Transition state is an `AtomicBool` on
the dispatcher and is **lost on restart, deliberately** — a fresh process
re-photographs what it finds.

**Fail-open, named:** `Path::exists()` returns `false` on any access error. Fail-closed
is not cleanly implementable (`exists()` cannot separate "absent" from "unreadable",
and `symlink_metadata()` would make the nominal "no `state/` dir" case a permanent
STOP). What makes that acceptable is the **visibility**, not the reasoning: the
operator sees the effect in ≤ 10 min. If the reader ever becomes fallible in a way
the operator cannot observe (DB, network), redo the trade-off rather than transport it.

**Scope: `auto_pull`, then `worktree_reap` (mika#2420).** `is_stopped`/`stop_file_path`
are parameterized by scan name, and mika#2329 shipped that parameterization while
explicitly refusing to use it twice, for want of a measured need: *"stopping QA review
is not the same decision as stopping the feeder."* **A destructive operation is
precisely that need** — during an incident one wants to stop what *deletes* without
restarting mika-spirit, which is the thing one least wants to do with dispatches in
flight. `wip_rescue` and `qa_review_reconcile` still have **no** switch: stopping them
destroys nothing, and shipping gestures nobody asked for stays YAGNI.
Structural guard `mika2329_le_chemin_du_fichier_sentinelle_a_un_seul_lecteur` refuses
a second occurrence of **either** scan's path literal under `src/` — extended with the
second scan in the same commit, because a scan born outside the guard is a scan on
which the `grooming_marker` lesson replays. A copy would make no decision wrong the day
it is written, which is exactly why no behavioural test can see it. Note the guard is
deliberately literal: naming the sentinel path in a *doc comment* is a hit too, so both
call sites reference the constant and the operator-facing path lives in `CLAUDE.md`.
Operator surfaces, the exact gesture, and the post-deploy probe: root `CLAUDE.md`
§ *Optional (STOP global à chaud — mika#2329)* and § *Optional (terminal-worktree
reaper — mika#2420)*.

### Terminal-Worktree Reaper (mika#2420)

`worktree_reaper` — the **fifth** recurring scan, beside `auto_pull`, `wip_rescue` and
`qa_review_reconcile` in `task_engine/dispatcher.rs`, carried by mika-dev, cron
`0 */10 * * * *`. It removes the dispatch worktrees under `.claude/worktrees/` whose PR
is terminal — and their `target/` with them, which is the 25-to-44-Go consumer the
founding ticket measures. Outside the LLM, outside the pilot session, triggered by time.

**It is the only scan of the family whose action is destructive and irreversible**, and
three things follow from that, in this order in the body of `dispatch_worktree_reap`:
the STOP short-circuit is **first**, before any token resolution; the disposition is
gated separately from the detection (`MIKA_WORKTREE_REAP_DISPOSITION=observe`); and
every term of the predicate is fail-safe towards *keep*.

**Three measurements moved the diagnosis, and they are the first deliverable.** The
ticket reads prior art **#1694** as "closed but ineffective". (M1) It is a **dormeur**,
a live line of `docs/dormeurs.md` whose wake condition is now met — mika#2420 is its
*wake*, and U6 removes the line per the register's contract. (M2) Its logic **never
reached `main`**: commit `097cc66c` carries a real implementation saved as `wip()` by
the mika#1282 recovery and never promoted. (M3) Its own doc admits the defect in
writing — *"layers A/B clean up whatever slips through"* — where A and B are **manual**
and C is a `pull_request.closed` webhook handler, i.e. a **single non-replayable event**
losable in the four places mika#2334 already had to name. The gesture whose three-hour
half-life the ticket measures *is* layer B.

**Shape.** `screen_worktrees` (T1–T6) and `apply_work_states` (T7) are pure functions
carrying the whole decision; `select_worktrees_to_reap` composes them for tests while
production calls them in sequence so the two `git` subprocesses of T7 are only paid for
the survivors of T1–T6. The registry is read from `git worktree list --porcelain` —
**the worktree path is declared, never derived** (mika-platform#58) — and `prunable`
entries are dropped (they belong to `git worktree prune`, their directory is already
gone). One `gh pr list --state all` per repo per tick; the `owner/repo` is derived from
`git remote get-url origin` rather than declared in a second list to keep in sync.

**The cap is a cap on writes**, applied by the caller after the filter (mika#2347's
lesson transposed): applied upstream it would cap *skips*, and refused candidates would
consume the tick in place of treatable ones. Pinned by
`mika2420_un_refus_ne_consomme_pas_la_place_dun_traitable`.

**Token** via `resolve_periodic_scan_token` (PAT-first, App fallback), covered by the
mika#2205 structural guard, which was extended to this fifth scan in the same commit.
The scan writes **nothing** on the forge (`gh pr list` is its only call), so there is no
author for GitHub to read in the ADR-008 sense and no
`resolve_periodic_scan_label_token`. `PeriodicScan` gained a variant, so
`mika2334_every_scan_variant_is_covered` fails to compile until the guards enumerate it.

**SOLE WRITER** of both audit `tool_name`s — `worktree_reaped` (`armed`, an effective
removal) and `worktree_reap_would_dispose` (`observe`, the population that *would* be
removed — mika#2469) — pinned by the same two-needle source scan. That is what makes
`SELECT … WHERE tool_name = 'worktree_reaped'` the exact list of worktrees the loop
removed, i.e. the ticket's guard-rail 3: since mika#2469 the name is reserved to the
removal, and `outcome_for(disposition)` is the one site the log event and the audit
`tool_name` both read, so the two surfaces cannot diverge. Refusals are written under
`worktree_reap_skipped`, keyed `worktree:<path>@<motif>` and **deduplicated on 24 h**
(mika#2131): a dirty worktree of a merged PR would otherwise write a row every ten
minutes, and a motif *change* rewrites because it is a state change.

**The negative control is what makes the positive one mean anything, and it had to
assert the motif.** A vacuity-only assertion is **insensitive for T3 and T4**, both
absorbed downstream by T5 (no known PR ⇒ no `closedAt` ⇒ refused; an open PR carries
`closedAt: null` ⇒ refused). Verified by mutation, not by reasoning: removing T3 shifts
the motif from `pr_unknown` to `pr_closed_at_unreadable` while leaving a vacuity-only
test green. All seven terms were mutated one at a time and each was observed red.

**Config, the eleven motifs, the four halts and the named residual risk** (an absent
`refs/remotes/origin/<branch>` reads `Clean`, not `Unreadable`): root `CLAUDE.md`
§ *Optional (terminal-worktree reaper — mika#2420)*.

### Unknown-Trigger Veto Lift (mika#2337)

**The failure this closes is a veto, not a missing wire.** A `run_skill` recurrence
whose trigger the running binary cannot route falls into `dispatch_run_skill`'s
catch-all and dies `failed`. There is **no spam**: re-enqueue lives only in
`fire_task`'s `Ok` arm, so a recurrence that fails is never rescheduled — the line
dies once and goes quiet, which is why it took a day to notice. What the death
leaves behind is the mika#1742 refuse-to-zombie veto, armed for
`RECURRING_ZOMBIE_GRACE_HOURS` (24 h), whose only exit
(`revert_config_cancel_recurring_task`) targets `status = 'cancelled'` and never
lifts a veto born of a `failed`. **Every restart inside that window — including one
carrying the correct binary — refuses to re-register.** The natural remedy is
exactly what the state neutralises.

**Why this class alone is exempt.** mika#1742 exists because a recurrence that
kills the system must not re-arm on every restart. Here the premise does not hold:
the binary that re-registers a trigger name is, by construction, the binary that
carries its arm — so the death predicts nothing about the next one, and the veto
only prolongs the outage it was meant to contain. mika#1742 is **not** disarmed in
general: any other cause of death still arms it
(`db::tests::mika2337_any_other_death_still_arms_the_veto`).

**Mechanism, three writes.** (1) `DispatchError::UnknownTrigger { trigger }` — the
class comes from the **variant**, never a substring on the rendered message; the
message text itself is unchanged, so operator greps and log history keep working.
(2) `fire_task` stamps `RECURRING_UNKNOWN_TRIGGER_PATH` on the row **before**
`update_task_failed` (never an unmarked corpse, not even between two writes), emits
the named `recurring_unknown_trigger` WARN instead of the generic
`task dispatch failed` — indistinguishable from a network failure — and writes an
`audit_events` row. The terminal state stays `failed`: what is corrected is its
*consequence* on re-registration. (3) `create_recurring_task_if_absent` skips a
marked dead sibling, and **spends** the lift in the same statement that honours it
(marker removed, `RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH` written). A
**second** unknown-trigger death on the same label inside the window meets a fully
armed veto — the lift buys one restart, not immunity. The consumed marker rides on
the row it was spent on, so it ages out of the grace window with it and the
exemption re-arms rather than being lost for good.

**Not retroactive, and that is a stated bound.** A row that died *before* the
stamping code existed carries no marker, so it stays a `dead_sibling`. mika#2337
makes every **future** death repairable by restart; it does not retroactively
unblock the one that caused it. Reading a
`mika#1742: refusing to re-register` on `qa_review_reconcile` at the first
post-deploy startup is that pre-existing row, **not** a regression — wait out the
window, or clear the row by hand.

**Guards.** `tests/eval/test_recurring_trigger_wiring_2337.rs` — a class guard
asserting every **registered** trigger has an arm (population = the calls to
`task_engine::ensure_recurring_task`, anchored on the *caller* rather than on the
`{"trigger":"X"}` literal, which has three false members in the tree), plus a
firing probe driving the recurrence through `TaskEngine::tick` with no network.
`db::tests::mika2337_*` carry the veto behaviour. **Honest scope:** these close the
**intra-binary** divergence; none of them would have caught the 2026-09-16 incident,
which is a skew between merged and running code.

**Operator surfaces.** `SELECT * FROM audit_events WHERE tool_name =
'recurring_unknown_trigger'` — **must stay empty** in nominal operation; any row
names a registered trigger a running binary does not know, i.e. a version skew.
Grep `recurring_unknown_trigger` in `$MIKA_SPIRIT_LOG_FILE`. If
`unknown run_skill trigger` reappears after a deploy, **do not touch the
dispatcher** — establish the version of the running binary instead.

**Registry inspector (mika#2360).** `GET /api/v1/recurring-tasks` (dashboard or
internal token; `?agent_id=`, `?page=`, `?per_page=`) lists every
`trigger_type = 'recurring'` row — all statuses, sorted `label COLLATE NOCASE,
created_at` so a label's duplicates sit together — as a **closed projection**
(`db::RecurringRegistryRow`: `label, agent_id, trigger_type, action_type,
cron_expr, next_fire_at, status, created_at, updated_at, zombie_veto_active`).
No `action_config` / `result` / `input_context` / `metadata`: a `send_message`
recurrence carries the user's reminder text in `action_config`, which is why this
is not `GET /api/v1/tasks?trigger_type=recurring`. `zombie_veto_active` is the
mika#1742 guard evaluated per row in SQL (lift-spent term correlated on
`(agent_id, label COLLATE NOCASE)`, mika#2337); `db::tests::mika2360_zombie_veto_flag_*`
pin it to `create_recurring_task_if_absent`. Reached from outside the cluster
through the gateway's `GET /admin/tenants/{customer_id}/recurring-tasks`
(admin read token — see `crates/mika-gateway/CLAUDE.md`).

### Structural CI Failure Handler

`server::ci_failure_handler` — intercepts `check_suite.completed(failure|timed_out)` webhook events **before** the LLM turn. Failure-side companion to `ci_success_handler`. Matches CI failures to open PRs and existing work items, fetches failing-job context (up to 3 jobs, 100 lines each), and constructs a pre-digest instructing the LLM to dispatch `run_claude_pilot` for an autonomous fix. Circuit breaker: `ci_fix_count >= 2` in task metadata triggers escalation instead of dispatch — the handler increments `ci_fix_count` deterministically (not reliant on LLM). Checks both task-level callback children and global dispatch guard, including results in the pre-digest. Reuses `VerdictAction`, `find_open_pr`, `run_gh_checks`/`classify_checks`, and `has_active_callback_child` from sibling modules. Also fixes `CHECK_SUITE_RE` regex in `webhook_queue.rs` to match actual gateway format (was `Check suite (failure)`, corrected to `Check suite failure`). Order-independent with other handlers. 30s timeout per subprocess call. See #594.

### Webhook Deferral Queue

`server::webhook_queue` — in-memory queue that holds inbound GitHub webhooks when the target task has an in-flight `run_claude_pilot` callback (#528). Prevents race conditions where a webhook (e.g. `pull_request_review.submitted`) arrives before the callback persists metadata (`pr_url`). Correlation: PR URL via `parse_pr_review_event()`, branch via check_suite regex, fallback to sole-inflight-callback heuristic. 60s per-webhook timeout with forced replay. Drain triggers: callback completion in `handle_task_complete` (Ok path only), or timeout expiry via `drain_expired()`. Emits `webhook_deferred` and `webhook_replayed` audit events. Queue is in-memory only (lost on restart; GitHub supports redelivery). See #528.

### Bounded Webhook Queue (v2)

`server::webhook_queue_v2` — per-agent bounded queue with backpressure + coalescing (mika#1870). **A different mechanism from the mika#528 deferral queue above** (which sequences webhooks against in-flight callbacks). This is the general-purpose ingestion queue that replaces `POST /message`'s legacy `try_lock_owned()` → 429-reject pattern. **It does NOT serve `/a2a/{agent}`, which has its own waiting form — see § Bounded A2A Wait Line below and do not unify the two without reading why.** **Uniform-queue model:** every inbound `POST /message` request enqueues; a single per-agent drain worker (`handlers::spawn_webhook_drain_worker`) is the sole consumer that acquires `agent_lock` and runs `run_agent_for_message`. The mika#528 deferral check runs **before** the enqueue and is unchanged. The accepted-case HTTP response is byte-identical to the legacy path (`status: "accepted"`) — the queue is invisible to the gateway; only the busy case changes (429 → queued).

**Classification + coalescing (AC2):** `classify_event(text)` maps gateway-formatted text to an exhaustive `WebhookEventKind` (`CheckSuite`/`PullRequestSync`/`Push`/`PrReview`/`IssueLabeled`/`ReadyLabel`/`Other`); `coalescing_key(&kind)` is an **exhaustive match with no wildcard arm** — a new variant fails to compile until a coalescing decision is made. Coalescing keys: `check_suite:{repo}:{branch}`, `pr_sync:{repo}:{pr}`, `push:{repo}:{branch}`, `labeled:{repo}:{issue}:{label}`; `PrReview`/`ReadyLabel`/`Other` return `None` (never coalesce — user input / dispatch trigger / order-preserving). `PR synchronize` → `PullRequestSync`; other PR actions (opened/closed/review_requested) → `Other`. Reuses `verdict::parse_pr_review_event` + sibling regexes. `Push` is reserved (the current gateway does not emit a branch-bearing push text; `classify_event` never produces it today).

**Enqueue algorithm (HYBRID: coalesce → block → drop-oldest):** (1) if a queued sibling shares the coalescing key → remove it, push the new to the back (newest-wins), return `Coalesced{replaced_event_id}`; (2) else if `depth < max_depth` → push, `Enqueued{depth}`; (3) else block up to `block_timeout` (default 100ms) on a `Notify` for a drain slot, retry once; (4) still full → drop the oldest (`pop_front`), push new, `Dropped` (dead-letter surface). `dequeue()` is cancel-safe (nothing popped until an item is present), awaits an item `Notify` when empty. Two `Notify` instances separate item-available (empty→wake) from slot-available (full→wake) to avoid mixed-semantics lost wakeups.

**`check_suite` SHA caveat (correctness-critical):** mika#1869 keys on `(repo, branch, head_sha)`; the gateway text carries repo+branch but **NOT** `head_sha`. v1 mitigates the same-branch-double-push missed-event hazard via **time-bounded coalescing** — only entries *still in the queue* coalesce, and the downstream `ci_success_handler` re-aggregates all required checks on every invocation, so a coalesced-away duplicate skips only a redundant walk, never a state transition. Threading `head_sha` end-to-end is a deferred cross-repo follow-up. Single FIFO priority class in v1; priority classes deferred.

**Drain worker (AC4):** one `tokio::spawn` per agent (boot loop in `run_server`; lazy-resolved agents get one in `resolve_agent`, mika#1399), `select!`-ing over a parent `CancellationToken` (`AppState.webhook_queue_shutdown`, cancelled at shutdown alongside `kg_shutdown_token`), `dequeue()`, and a 5s gauge heartbeat. Processing runs in a child task so a panic never crashes the loop (captured as `JoinError`). Blocking `lock_owned().await` serialises turns — modeled exactly on `replay_deferred_webhooks`.

**Audit events (AC5, throttled ≤1/sec/action/agent via `AppState.webhook_queue_audit_last`):** `webhook_queue_enqueued`, `webhook_queue_coalesced`, `webhook_queue_drop_oldest`, `webhook_queue_dequeued`, `webhook_queue_processing_error` (all `target_key = agent:{name}`). Best-effort 5s structured-log gauge `webhook_queue.gauge` (`depth`/`enqueued_total`/`coalesced_total`/`dropped_total`), emitted only when the queue is non-idle. **Config (AC6):** `MIKA_WEBHOOK_QUEUE_MAX_DEPTH` (64), `MIKA_WEBHOOK_QUEUE_BLOCK_TIMEOUT_MS` (100), `MIKA_WEBHOOK_QUEUE_ENABLED` (true; `false` = kill-switch AC9 → legacy 429-reject path verbatim, no redeploy). **Operator grep signals:** `webhook_queue_coalesced` (real coalescing volume), `webhook_queue_drop_oldest` (overflow/dead-letter — should be near-zero), `webhook_queue.gauge` (depth trend), and `rate_limit_trip` near-zero once enabled (baseline was 41/h on mika-qa). See mika#1870.

### Bounded A2A Wait Line (mika#2163)

`server::a2a_wait_queue` — the two `/a2a/{agent}` lock gates (`a2a.rs`
`handle_message_send`, `handle_message_stream`) **wait in a bounded line** for
`agent_lock` instead of refusing on collision. Before mika#2163 both did
`try_lock_owned()` and answered `-32603 "Agent is busy"` — a refusal, on a code
whose defined meaning is "the server failed", for what is ordinary contention.
`/a2a` is the path of every `mika ask` (`mika-cli/src/commands/ask.rs`) and
therefore of every pilot `canUseTool` callback, so the refusal was load-bearing:
the founding measurement is a grooming architect pass refused **five times in a
row** at 20 s intervals before the sixth attempt landed.

**Why this is not `webhook_queue_v2`, and must not be merged into it.** mika#2163
AC1 asked to reuse the mika#1870 mechanism; reading the code shows it does not
transpose. `WebhookQueue`'s producer has no return channel (`enqueue` answers
`202`, a drain worker runs the turn later) while `message/send` is synchronous and
its caller holds the connection open for the completed `Task`; its coalescing is
inert here (two `mika ask` calls never merge); and its saturation policy is
`drop_oldest`, which cannot apply to a request someone is waiting on. Generalising
it would mean a per-entry `oneshot`, a second saturation policy and a second drain
worker — putting `POST /message`, the autonomous loop's own dispatch path, into
this fix's blast radius. **Decision R-A, ruled by mika-prime 2026-09-05: take the
mika#1870 *form*, not its code.** The duplication of form between the two modules
is deliberate and load-bearing: the control contract differs (`/message` is
fire-and-forget, `/a2a` is synchronous). A future unification that skips this
paragraph reintroduces exactly what was rejected.

**Mechanism.** No second queue structure is introduced. The wait is the one
`tokio::sync::Mutex` already provides — documented FIFO-fair (`tokio-1.53.1`
`sync/mutex.rs:112-116`), so the bound bounds a line and not a scramble — and a
per-agent `Semaphore` (`AgentState.a2a_wait_slots`) only makes the depth of that
line explicit and refusable. **The permit is released at lock acquisition, never
at turn end**: holding it for the turn would make one wait and one execution count
against the same number, and the configured depth would stop meaning anything.

**The two gates differ, deliberately.** `message/send` waits **in the handler**
(it is synchronous; there is nothing to return early with). `message/stream` takes
its place in the handler **before** the `spawn` — take it inside and the spawn is
unbounded and the backpressure decorative — but waits **inside** the spawned task,
so the SSE stream opens immediately rather than staying silent until the lock
frees. Saturation is still answerable as a JSON-RPC error on both, because it is
detected before the stream opens.

**Caller abandonment (AC7) — the risk this fix creates rather than removes.** A
spawned task is not cancelled when the client disconnects, so a streaming caller
that hangs up mid-wait would otherwise still acquire the lock and run a turn
nobody reads, holding its place in front of everyone behind it. The spawned task
therefore races `wait_for_agent_lock` against `tx.closed()` (`broadcast.rs:919`,
completes when the last SSE receiver drops) with `biased`. The abandonment lands
**before** acquisition: the place in the mutex FIFO is given up, the permit
returns, the agent is never started, and the task row goes `canceled`. Detection
is bounded by the SSE `KeepAlive` interval, which is a transport property — the
in-crate test pins the mechanism and the accounting, and says explicitly that it
does not measure the over-the-wire delay.

**Error surface.** `AGENT_BUSY = -32000` (`mika-a2a/src/jsonrpc.rs`), **not**
`-32099`: the A2A spec claims the whole `-32001`..`-32099` band for itself
(`a2aproject/A2A@main` `docs/specification.md` §9.5; `-32001`..`-32009` assigned
today), so every code from `-32001` down is a number in someone else's namespace.
`-32000` is the one code of the JSON-RPC implementation-defined server-error band
A2A does not claim. `error.data` carries
`{reason: "queue_full"|"wait_timeout", retry_after_ms, queue_depth}` —
`retry_after_ms` is the **configured** bound, not a prediction of the running
turn's length, which the server does not know.

**Config (AC3, three tiers, same shape as mika#1870):**
`MIKA_A2A_QUEUE_MAX_DEPTH` (8; `0` → default + WARN),
`MIKA_A2A_QUEUE_WAIT_TIMEOUT_MS` (30000; `0` honoured as "do not wait"),
`MIKA_A2A_QUEUE_ENABLED` (true; `false` = kill-switch → `try_lock_owned()` →
`-32603 "Agent is busy"` **verbatim, error code included** — a rollback that
changed the code would not be a rollback). **The 30 s default is sized against the
tightest real caller budget, not the most generous one:** `A2aClient::DEFAULT_TIMEOUT`
is 300 s, but the claude-pilot `canUseTool` relay — the path this ticket was filed
about — is **120 s** (`.claude/claude-pilot.json`) and covers the wait *and* the turn.
A 120 s wait, reasoned from the 300 s figure alone, would spend a pilot's whole budget
waiting and let the relay kill `mika ask` at the exact instant the wait expired, so the
caller would never read the refusal it was owed. Pinned by the assertion in
`config::tests::a2a_queue_defaults`.

**Two costs this creates, named so they are not rediscovered.** (1) `returnImmediately`
is **exempt** from the line and must stay so: that branch returns a `submitted` Task
without running the agent loop, so it needs neither the lock nor a place — harmless as
a `try_lock`, not harmless as a wait. (2) A cross-agent call cycle (A's turn shells out
`mika ask --agent B` while B's shells out `mika ask --agent A`) now stalls for the wait
budget instead of failing in microseconds. Inherent to waiting rather than refusing; the
budget and the kill-switch bound the damage. If such cycles become a pattern rather than
an accident, the answer is a cycle detector, not a longer wait.

**Operator grep signals** (`$MIKA_SPIRIT_LOG_FILE`, throttled ≤1/sec/action/agent
via `AppState.a2a_queue_audit_last`): `a2a_queue_wait` (a wait that actually
happened, carries `wait_ms`), `a2a_queue_reject` (carries `reason` and the code),
`a2a_queue_abandoned` (a streaming caller hung up before acquiring — no orphan
turn ran). Audit-events SQL surface: `SELECT * FROM audit_events WHERE tool_name
IN ('a2a_queue_wait', 'a2a_queue_reject')`. Post-deploy expectation: `mika ask`
retry loops written by hand in caller shells stop firing.

**Relation to mika#2160.** Raising `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT` above
1 was gated on this ticket: two pilots whose permission escalations overlap would
have seen one refused mid-session. This is an *operational* prerequisite of N>1,
not a delivery one — mika#2160 ships its default of 1 without it.

### No Completed Turn Is Lost In Silence (mika#2270)

**The failure.** Eleven consecutive `mika ask` calls came back empty on
2026-09-09 — `.content` absent, **exit 0** — while the engine had produced the
full answer and written it to the log (trace `d5887aa7`, 43 108 input tokens,
28 s, `stop_reason=EndTurn`). mika#2266's architect verdict had to be harvested by
hand out of a 19 GB log file for the grooming to finish. The shape is what makes
it a loop-breaker rather than a slowdown: well-formed JSON, code zero, a key
simply missing — **nothing distinguished "the agent had nothing to say" from "the
answer was lost"**. A probe that lies without failing.

**M1 — `message/send` held the answer and threw it away; `message/stream` never
did.** `run_a2a_agent` returns `Result<Option<String>, String>`: the turn's text,
in memory. The stream port serves it directly. The send port filtered it out with
`Ok(_)` and then **rebuilt a Task from the database** via `a2a_build_task`. Same
loop, two ports, one of which kept the reply in hand. That is the literal shape of
"LLM cost paid and discarded".

**M2 — the renderer's "three tiers" were never three sources**, and its own doc
comment claimed defence in depth for months. `a2a_insert_artifact` has no
production caller, so tier 1 is dead on the spirit path; and `a2a_build_task`
derives `status.message` from the last agent-role message of `history`, which is
`a2a_get_messages`' result — so tiers 2 and 3 fail **together**, on one query.
Chasing the cause in the renderer, which is what the ticket's main lead proposed,
could not have converged: the renderer was the victim.

**What ships.** (a) `message/send` keeps the loop's text and guarantees that a
terminal Task it serves carries at least one non-empty slice —
`ensure_send_task_carries_text`, whose decision half `decide_content_net` is a
pure function with three outcomes (`Nominal` / `MuteTurn` / `Rescued`). (b) The
renderer returns `Result` instead of `String`, so an unreadable Task **fails
naming what was inspected** (artifact and history counts with their roles,
`status.message` presence, plus `task_id` and `context_id` — the two handles that
find the turn in `$MIKA_SPIRIT_LOG_FILE`). The signature change, not a parallel
function, is what makes the compiler force both call sites.

**The two halves share one predicate, and that is load-bearing.**
`mika_a2a::render::render_task_text` is the single definition of "does this Task
carry text", used by the server's net and by the CLI's renderer. A server-side
approximation could call a Task fine while the client found nothing readable —
the exact gap the net exists to close. It also means the fourth line of the
decision table is not a detail: a turn that produced **no** text is served with
the same literal `message/stream` serves (`COMPLETED_WITHOUT_TEXT`), which
**removes the whole "completed but empty" class from this port** and is what lets
the CLI treat every empty Task as a loss with no false positive.

**The net does not hide the defect.** A silent repair would turn the loop green
and make the fault permanently invisible — i.e. build the next occurrence. So the
rescue path emits `a2a_send_task_content_lost` carrying only the facts that settle
the cause: `task_id`, `context_id`, `session_id`, the `trace_id` handed to the
loop (equal to `task_id` today, reported rather than assumed), and the two counts
from `Database::a2a_message_census` — rows for this session, rows for this trace —
which separate "nothing was persisted" from "persisted under another trace id"
without an operator opening the database.

**SOLE WRITER:** `a2a_send_task_content_lost`, in the log and in `audit_events`.
Its **absence** under a symptom is therefore information: it says the loss is not
here. Pinned by `server::a2a::tests::the_loss_signal_has_exactly_one_writer_in_this_module`,
a lexical guard — a second writer would make no decision wrong, only
unattributable, which no behavioural test can see.

**What this does NOT explain: why `a2a_get_messages` returns empty.** M3 reduces
it to one predicate on one column written by one path (the agent loop's
`trace_id`), but deciding between "nothing persisted" and "persisted under another
trace id" needs the production database. Follow-up ticket **conditioned on the
first occurrence** of the WARN, which carries exactly the two counts that
depart it. Opening it before that line exists would be instructing without a
measurement.

**Operator surfaces.** Grep `a2a_send_task_content_lost` in
`$MIKA_SPIRIT_LOG_FILE` — **empty in nominal operation**; continuous firing means
the net is masking a persistence failure that deserves its own fix, so treat the
cause, not the threshold. SQL:
`SELECT COUNT(*) FROM audit_events WHERE tool_name = 'a2a_send_task_content_lost';`
Two adjacent names, deliberately distinct: `a2a_send_task_census_unreadable`
(ERROR — the counts could not be read, the loss is still reported) and
`a2a_send_task_content_lost_audit_failed` (the WARN landed, the audit row did
not).

**Halt conditions.** The symptom returns and the WARN stays silent → the loss is
not where this fix places it: do **not** widen the net or add a tier to the
renderer, reopen the investigation on the client or transport side with the POST
trace. A nominal reply whose bytes change → the net is biting where it must not;
halt before deploying, since a return-channel fix that alters healthy answers is
worse than the silence it replaces.

### Introspection Tools

5 read-only tools: `query_timeline`, `get_session_messages`, `list_audit_events`, `search_tool_history` (30-day retention, 500-char field truncation, 10KB output cap), `query_knowledge_graph`. Non-orchestrator agents scoped to their own agent_id/sessions.

## Milestone Manager (Phase 1)

`src/milestone_manager/` — milestone-scope operational coordinator (`mika-manager` entity, distinct from `mika-prime`). Ratified 2026-08-21 by Vincent + Prime (5 verdicts, brief at `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`). **LECTURE seule** — zero dispatch, zero ticket mutation, zero PR merge; the only outbound side effect is a report `POST` to a well-known delivery endpoint (Prime→sami→Vincent per D8 subsystem-2 pattern) or an offline sink write when the URL is unset.

**Three composers + one loop.** `Reader` (`reader.rs`) wraps `gh` CLI (`api`/`issue list`/`pr list`) mirroring the `auto_pull::gh_list_open_issues` subprocess shape and composes `MilestoneState` (sub-issues + progress counts + recent activity). `Assessor` (`assessor.rs`) applies four rules (stale-blocker, silent-progress, silence-in-JOURS, priority ranking) and classifies `Severity` (Healthy/Attention/Blocked). `Reporter` (`reporter.rs`) formats the § 2d Markdown report. `run_manager_cycle` in `cadence.rs` orchestrates read→assess→deliver with hybrid cadence: event-driven trigger on `state_digest` change + 6h plancher heartbeat (« l'absence d'event EST l'event »).

**Wrapper-only INV-2.** No new GitHub API client dep; `Reader` uses `tokio::process::Command::new("gh")` verbatim from `auto_pull.rs`. Delivery uses the existing workspace `reqwest` — bearer-token POST to `MIKA_MANAGER_DELIVERY_URL` (normal) or `MIKA_MANAGER_ESCALATION_URL` (Severity::Blocked → Vincent-direct route). Both URLs unset → offline sink at `MIKA_MANAGER_OFFLINE_SINK_DIR` so nothing is lost during bring-up. **That variable name is the one the code reads, and this line used to carry a different one** (`MIKA_MANAGER_SINK_DIR`, which exists nowhere) while the line six paragraphs down spelled it correctly — the class mika#1971 named in as many words: *a deployer who followed it put the key in the one place it could not work*. The occurrence is fixed; the class is closed by `mika2267_every_manager_env_const_is_declared_in_env_example`, which refuses an `ENV_*` const the code reads and `.env.example` does not declare.

**Structural LECTURE-seule enforcement.** `no_dispatch_test.rs` greps the module tree for forbidden write-authority tokens (`run_claude_pilot`, `pr_merge_with_gate`, `gh api "PATCH"`, `gh issue edit`, …) and fails the test binary if any executable code contains them. Prompt-level "no dispatch" rules are fragile per `feedback_prompt_enforcement_fragile` — this test is the structural gate. Comments are stripped before scanning so doc prose can describe what's forbidden.

**CLI surface.** `mika milestone {read,assess,report} <owner/repo>#<number>` — thin adapter in `crates/mika-cli/src/commands/milestone.rs`. Report subcommand emits Markdown to stdout; `read`/`assess` default to pretty JSON (also YAML via `--format yaml`).

### The offline sink has a reader (mika#2267)

**The defect, and the rectification the code imposed on the ticket.** mika#2267 read the symptom as a broken cm delivery channel (*"sink offline vs endpoint, à déterminer"*) and offered two branches: repair the delivery, or document the fallback sink. Reading the code moves the diagnosis one notch, and that displacement is the first deliverable. **The channel was not a pierced pipe: the fallback sink had no reader at all.** Exhaustive search over the tree returned `offline_sink` / `OFFLINE_SINK` at exactly three sites — the write (`cadence.rs`), the config (`spawn.rs`), the prose (this file). Zero CLI command, zero tool, zero HTTP route, zero runbook, zero consumer. From the point of view of the human who owes the Reader fidelity verdict, **"the channel is broken" and "the channel is a well" produce exactly the same bytes: none.** Same class as mika#2205 (*a silently inert scan reads exactly like a scan that found nothing to do*) and mika#2131's mute exclusions.

**Scope constraint, stated rather than discovered.** `control-monitor` is **not in this workspace** (it holds `claude-pilot/` and `mika/` only), so no line of this repo can make a cm endpoint exist or make it live. This work takes the second branch the ticket explicitly authorises — and makes it better than documentation by shipping a **reader**. Prose saying "reports are in `~/.mika/manager/sink/`" leaves the verdict hanging on an `ls` someone has to remember; a subcommand makes it executable and, above all, **names the path it consulted**, which is the only way the CLI-and-daemon-disagree case becomes readable in one line instead of an investigation. Repairing the cm delivery stays a **follow-up ticket**, and its precondition is the measurement this work produces: until one knows whether a URL was ever posted, "repair the channel" is repairing blind.

**`mika milestone reports`** — `--target <owner/repo>#<n>` restricts to one milestone, `--latest` prints the most recent report's Markdown, `--limit` (default 20) caps the listing, `--format text|json|yaml`. **The load-bearing property is that an absent directory is not an empty one:** `SinkListing` has three variants (`DirAbsent` / `Empty` / `Entries`), never a bare empty list, because the two say opposite things — *the cadence has never written HERE* against *the cadence wrote here and the sink was emptied* — and collapsing them would reproduce, at the reader's own surface, the silence it exists to lift. The output **always names the consulted path and its provenance**, on the nominal path too.

**One resolver, and the guard that keeps it one.** The path used to be composed inline in `manager_config_from_env`. A CLI reader recomposing it would be free to diverge from the writer — **and a reader that looks somewhere the writer does not is precisely the defect being closed, reproduced one layer up**. `milestone_manager::sink_dir::resolve_offline_sink_dir() -> (PathBuf, SinkDirSource)` is the sole site; writer and reader both go through it. `mika2267_sink_dir_resolution_has_a_single_reader` is a source scan over `crates/mika-agent/src` **and** `crates/mika-cli/src` refusing two needles — the env-var **literal** (a second reader relying on its own copy of the name) and the `.join("sink")` recomposition (a second reader refabricating the default path). **Allowlist shipped empty:** when it fires, remove the second site, do not allowlist it. No behavioural test can see that class — a second resolver makes no decision wrong the day it is written; it diverges later, in silence, with every assertion still green.

**`manager_delivery_resolved` (INFO, once per cadence start)** — the line that settles the ticket's own question without reading the source. Fields: `milestone`, `route_normal` / `route_escalation` (`"http"` | `"offline_sink"` — what a non-`Blocked` and a `Blocked` severity will respectively take), `delivery_url_set`, `escalation_url_set`, `delivery_token_present`, `offline_sink_dir` (the **resolved** path), `sink_dir_source` (`"env"` | `"default"`). A distinct event rather than more fields on `manager_cadence_start`: it is a *configuration* event answering "where do my reports go?", not "the cadence started", and it must grep alone — exact precedent and identical reasoning, `llm_budget_resolved` (mika#2293), *un réglage qu'on ne peut pas observer n'est pas un réglage, c'est un espoir*. `delivery_token_present` is a **boolean** — never the value, never a prefix, never a length (doctrine `api_key_present`, asserted by a negative test that sweeps *every* field, because a leak arrives through the field nobody thought to look at). **Its absence while the cadence runs means the deployed binary predates the fix (class mika#2340)** — establish the deployment before concluding anything about the channel; never read the silence as "all is well".

**`route` is a wire format, and the fallback stopped being mute.** Three constants, one site (`ROUTE_HTTP` / `ROUTE_OFFLINE_SINK` / `ROUTE_OFFLINE_SINK_FALLBACK`), pinned by test. The post-HTTP-failure fallback was **the only one of the three write paths emitting nothing**: it wrote the sink, set `delivered = true`, and left no line saying a report had been written or where. It now emits `manager_cycle_delivered` under `offline_sink_fallback`, **deliberately distinct** from `offline_sink`: the two populations say opposite things — *no URL was posted* (nominal bring-up, nothing is broken) against *a URL was posted and failed* (a real fault) — and merging them would erase the exact distinction the ticket asks to draw. Two crossed tests hold it: asserting only that the failure emits *a* line would be satisfied by reusing `offline_sink`.

**What this does NOT change.** `CycleOutcome::delivered` keeps its meaning: `true` on all three paths. It is an output field read by tests and telemetry, and hardening it would change a public contract for a gain `manager_delivery_resolved` and the fallback route already deliver. What was missing was never a stricter boolean — **it was knowing where.** A future ticket that genuinely wants to tell "read" from "written" must first define what "read" means; that is not a boolean question. The sink **format** is unchanged too (`write_offline_sink` still writes `report_markdown` alone, not the full `DeliveryBody`), with the named consequence that **the reader cannot filter by severity** — it lives in the report body, not in a machine header. Writing the full JSON would be richer but would change the format of files already on disk in production, and nothing in the ticket asks for that filter; date ordering covers the measured need, which is reading the latest report. A named limit, not an oversight.

**Operator probes, and their halts.** (1) `grep manager_delivery_resolved "$MIKA_SPIRIT_LOG_FILE" | jq '{route_normal, route_escalation, delivery_url_set, offline_sink_dir, sink_dir_source}'` — `route_normal: "offline_sink"` means **no URL was ever posted**: the reports have been on disk from the start, the cm channel never existed, and the remedy is a configuration gesture plus the cm follow-up ticket, **not a code fix**. `route_normal: "http"` sends you to probe 2. **Halt — no line at all while the cadence runs:** class mika#2340, establish the deployment first. (2) `grep manager_cycle_delivered "$MIKA_SPIRIT_LOG_FILE" | jq -r .route | sort | uniq -c` — `http` dominant means delivery works and the symptom is **downstream** (the cm consumer does not render the reports): **halt, do not touch this repo**. `offline_sink_fallback` dominant means the endpoint refuses, and `manager_cycle_delivery_failed` carries the error; the cm follow-up then opens **with a count rather than an intuition**. (3) `mika milestone reports --latest` must return a non-empty Phase 1 report — **that is the criterion unblocking the Reader fidelity verdict**. **Halt — empty output while probe 1 says `offline_sink`:** the CLI and the daemon do not resolve the same path (different `HOME`, service under another user); compare the printed path against probe 1's `offline_sink_dir`, which is exactly why the reader names it. **Expected regimes:** `manager_delivery_resolved` one line per start, always present; `offline_sink_fallback` **zero in a healthy regime** — any occurrence is a configured endpoint refusing.

**Out of scope, named.** The sink has **no rotation** and grows unbounded; the reader makes the volume visible, which is the precondition for deciding on rotation — a follow-up ticket if the probe shows a problematic volume. And `emit_auth_alarm` (`spawn.rs`) still writes **nothing** to the sink when `escalation_url` is unset: it returns early and only the `error!` survives. A real hole, found on the way and deliberately **distinct** — a lost report is not a lost alarm, and fixing it requires deciding what an alarm in a well even means. **Follow-up ticket.**

**Env vars.** All optional with three-tier fallback: `MIKA_MANAGER_TARGET_MILESTONE` (Phase 1 single-target — loop disabled when unset), `MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS` (default 21600 = 6h), `MIKA_MANAGER_POLL_INTERVAL_SECS` (default 300 = 5min; clamped to `min(poll, heartbeat)`), `MIKA_MANAGER_SILENCE_THRESHOLD_DAYS` (default 3), `MIKA_MANAGER_DELIVERY_URL` / `MIKA_MANAGER_DELIVERY_TOKEN`, `MIKA_MANAGER_ESCALATION_URL`, `MIKA_MANAGER_HEALTH_URL` (optional cm executor liveness endpoint), `MIKA_MANAGER_CHECKPOINT_DIR` (default `$HOME/.mika/manager/checkpoints`), `MIKA_MANAGER_OFFLINE_SINK_DIR` (default `$HOME/.mika/manager/sink`). **All ten are declared in `.env.example` since mika#2267** — before it, not one `MIKA_MANAGER_*` variable was declared anywhere in the repo (`.env.example`, `docker-compose.yml`, `packaging/`), so no review had ever been in a position to notice their absence or their misspelling. `mika2267_every_manager_env_const_is_declared_in_env_example` keeps it that way.

**Cadence spawn.** `spawn.rs` (`manager_config_from_env` + `spawn_manager_cycle_task`, the latter taking an `Arc<dyn TokenResolver>` since mika#2013) wires the cycle as a background tokio task at `server::run_server` startup. Env-gated on `MIKA_MANAGER_TARGET_MILESTONE`: unset → INFO `manager_cadence_disabled` and no spawn; set-and-valid → INFO `manager_cadence_start` + spawn; set-and-malformed → ERROR `manager_cadence_config_invalid` and no spawn (startup continues). The loop polls at `cfg.poll_interval` and calls `run_manager_cycle`; the cycle body itself decides whether to deliver (state-change OR heartbeat-elapsed) and is a no-op otherwise. Cycle failures log at WARN and do not stop the loop — except a sustained authentication failure, which escalates per the token-renewal note below (mika#2013). Graceful shutdown responds to a dedicated `manager_shutdown_token` (sibling to `kg_shutdown_token` and `webhook_queue_shutdown`) — cancelled at the same `.with_graceful_shutdown` site. Structurally mirrors `kg::resolver_tick::spawn_resolver_tick_task` (same `tokio::select!` shape, same fail-open discipline, same `info!/warn!` lifecycle events). Injection-verified per `todos/mika-manager-cadence-wiring-injection-verification.md`.

**Token renewal + auth alarm (mika#2013).** The cycle token is re-resolved **before every cycle** through a `TokenResolver` (`SettingsTokenResolver` in production — PAT first per ADR-008, GitHub App installation token as fallback), never frozen at spawn. The founding bug: `ManagerConfig.github_token` was resolved once in `manager_config_from_env` and forwarded verbatim to `gh` forever, so an App installation token (~1h TTL) left the manager cycling 401 until the process restarted — 16 `manager_cycle_error auth_class=401` in one night with nothing louder than a WARN. The renewal machinery already lived in `GitHubApp::installation_token()` (memory cache + 5-min expiry buffer); the defect was only that it was asked once. A change in the resolved value emits `manager_token_refreshed` (INFO, presence booleans only — never token material). Paired with it, `AuthFailureTracker` counts the **duration** of an unbroken `AuthClass::Unauthorized` run (a duration, not a cycle count: `poll_interval` is operator-configurable, so N cycles has no stable temporal meaning). Past `AUTH_PERSISTENT_FAILURE_THRESHOLD` (30 min) it emits `manager_auth_persistent_failure` (ERROR) and escalates to `MIKA_MANAGER_ESCALATION_URL`, re-announcing at most hourly (`AUTH_ALARM_REEMIT_INTERVAL`). The escalation carries a dedicated `AuthAlarmBody`, **not** a `DeliveryBody` — the alarm fires because the milestone could not be read, so an `Assessment` there would be fabricated state on the wire. Any successful cycle clears the window; non-401 failures neither advance nor clear it (a network blip is not proof of recovery, nor of auth failure). A failed re-resolution KEEPS the previous token rather than overwriting it with `None` — `reader.rs` only sets `GH_TOKEN` when `Some`, so overwriting would silently drop the cycle onto the host's ambient credentials. The refresh is bounded by `TOKEN_REFRESH_TIMEOUT` (15s) because it sits outside the `select!` on the cancellation token and `GitHubApp` holds its cache write-lock across an un-timed HTTP call. `AuthClass::Forbidden` is excluded from the alarm — a known gap tracked as mika#2063, left open because AC3 of the ratified mika#2013 plan asserts 403 does not fire.

**Post-mika#1968 hardening (mika#1975).** Three changes, none of which moves a
delivery route or an alarm threshold.

*The boot probe is bounded.* `verify_gh_auth` wraps its single `gh` call in
`GH_AUTH_PROBE_TIMEOUT` (15s, a named constant, deliberately not
env-configurable — the same YAGNI rule `AUTH_PERSISTENT_FAILURE_THRESHOLD`
states). **What an unbounded probe blocked was never startup**, contrary to the
ticket: `spawn_manager_cycle_task` returns its handle immediately and
`run_server` never awaits it, while the probe runs *inside* the spawned task.
What it blocked is the **cadence loop before its first tick** —
`tokio::time::interval` is built after the probe — so a hung `gh` meant zero
cycles for ever, and it survived graceful shutdown too (`ProcessGhRunner::run`
has no timeout, and the `select!` on `cancel` is downstream of the probe, so
nothing drops the future `kill_on_drop` would need). The silence looked
healthy: `manager_cadence_start` and `manager_delivery_resolved` are both
emitted *before* the probe. New `AuthClass::Timeout` (`auth_class=timeout`),
posed on the `Err(Elapsed)` arm and **nowhere else** — `classify_cycle_error`
carries no `Timeout` pattern, and `"timed out"` stays under `Network` because a
transport timeout is an observation about the network while this variant is a
statement about a budget we enforced. Excluded from `is_auth_failure` for
`Network`'s reason: a slow host is not a closed door.

*The classifier stops reading its own arguments.* `ProcessGhRunner` now fails
with a typed `GhCommandError { args, stderr }` (house precedent: `DeliveryError`
+ `downcast_ref`, and the mika#2179 rule that error classes come from the
variant). `classify_cycle_error` / `classify_milestone_probe_error` take
`&anyhow::Error` and classify the **`stderr` alone**, falling open to the
rendered string when the downcast misses. The vector this closes is structural
rather than measured: the rendered error carried
`gh api /repos/{owner}/{repo}/milestones/{N}`, so with
`MIKA_MANAGER_TARGET_MILESTONE=owner/repo#403` *every* failure of that call —
a 500, an unrecognised DNS wording, a parse error — classified `Forbidden`,
reached `is_auth_failure` and fired the 30-minute alarm with the wrong class and
the wrong hint. The three numeric tests additionally go through
`has_http_status`, which requires a status marker (`http`/`status`/`code`)
within 24 characters upstream and non-digit boundaries. **No non-numeric pattern
was removed** — `unauthorized`, `bad credentials`, `gh auth login`,
`authentication token not found`: the last two are mika#2013's *missing-token*
shapes, which carry no HTTP status at all, and dropping one would make a
token-less manager classify `Other`, i.e. reopen the blindness mika#2013 closed.
That invariant (*the tightening removes false positives only*) is pinned by a
frozen corpus, `mika1975_the_classification_of_every_measured_auth_shape_is_unchanged`.
`stderr_head` deliberately keeps the **whole** rendered string, command line
included: the tightening is on what is *classified*, never on what is *logged*.

*The spawn guard cannot poison.* `MANAGER_SPAWN_GUARD` is an `AtomicBool` with
`swap(true, SeqCst)` instead of a `Mutex<bool>` + `.lock().unwrap()`. The ticket
asked for `PoisonError` recovery; the critical section was a `bool` read, a
`warn!` and a `bool` write, so **the only realistic panic site in it was the
logging macro** — a recovery path that logs is unreliable exactly when it is
needed. No poisoning test was added: asserting "it does not poison" on a type
with no poisoning is empty.

**Out of scope, named.** AC4 (proactive App-token refresh) is mika#2013's whole
subject and was removed from this ticket by an operator comment dated
2026-08-29; mika#2013 has since shipped it (`refresh_cycle_token`). The
cycle-body `gh` calls in `Reader::read` remain unbounded — same class, larger
blast radius (the loop freezes mid-cycle, in steady state), different remedy
(a per-call budget must compose with `poll_interval` and with
`MissedTickBehavior::Burst`), and a single global timeout in
`ProcessGhRunner::run` is refused because one constant cannot fit both
`gh api /milestones/N` and `gh pr list --limit 100 --search`, nor the CLI paths
where an operator is waiting. That is its own ticket.

**Phase 2 gates NOT wired.** Dispatch authority stays gated behind the three portes (forge-gate loop-résistance + contention exec + INTERNAL_TOKEN alignment) documented in the brief § 3. This module contains no dispatch class, no `run_claude_pilot` invocation site, no scope-approval callsite. Promotion to Phase 2 requires updating both `no_dispatch_test.rs` FORBIDDEN_TOKENS and the module docstring atomically.

## Skills System

Git-based and local skill distribution via `mika skills install/uninstall/update`. Sources: git URLs, GitHub shorthand, local paths, `file://` URIs. Optional `--link` flag creates absolute symlinks. Four-tier origin: `[built-in]`, `[marketplace]`, `[marketplace/linked]`, `[custom]`. Tracks in `marketplace.lock` (TOML).

**Bundled skill sources (dual path, #598):** `seed_bundled_skills()` seeds two concatenated sources on startup — the hardcoded `BUNDLED_SKILLS` list (10 community-category skills: tmux, shell-exec, web-search, file-reader, self-knowledge, git-ops, google-workspace, github, mcp, browser-control — `include_str!`-embedded from `templates/skills/`) and the directory-sourced `ENTRIES` table generated at build time by `build.rs` walking `<workspace>/skills/bundled/*/`. `all_bundled_skills()` merges both with case-insensitive ENTRIES-wins-on-name-collision semantics. Production `skills/bundled/` contains 22 engine-coupled skills: self-dev, self-dev-callback (#1106), self-dev-iterate, self-dev-webhook-qa, self-dev-webhook-ci, self-dev-webhook-ready-label (#1106), qa-review, qa-review-build-callback, skill-review, dev-pilot, build-mika, deploy-mika, permission-policy, agents-teams, address-pr-comments, resolve-pr-conflicts, self-check, dev-groom (#845), dev-handsoff (#967), mika-arch-groom-ticket (#811), mika-arch-groom-milestone (#879), mika-arch-second-review (#811). Engine-coupled = correctness depends on staying in lockstep with Rust engine code (tool schemas, callback contracts, prompt-discipline rules encoded as Rust guards). `is_bundled_skill()` consults both sources. Trust-critical classification (`TRUST_CRITICAL_SKILLS`) stays hardcoded regardless of source. Shared build-time discovery helper lives at `build_support/bundled_skills_discover.rs` and is consumed by both `build.rs` and integration tests via `#[path = ...]` mod attributes. Directories prefixed with `_` (e.g., `_shared/`) are excluded from discovery — convention-reserved for shared support libraries. `_shared/dispatch-lib.sh` (#893, #932) provides the shared claude-pilot dispatch plumbing. Each dispatch skill owns its own tool: dev-pilot registers `run_claude_pilot` (`skill: ["dev-pilot"]`, entry `/mika`), dev-groom registers `run_claude_pilot_groom` (`skill: ["dev-groom"]`, entry `/mika-groom-ticket`). Both handlers source the shared lib; the lib derives the entry command from `$SKILL` via a `case` switch. Restored from prompt-only design in mika#1173 after 5 LLM-dependency regressions.

**Dependency resolution:** BFS with cycle detection (max depth 10), same-source sibling resolution. `--link` propagates to deps from same source. `find_orphaned_deps()` + `--remove-deps` for cleanup.

**Per-provider and per-model variants:** Two-level directory hierarchy: `{provider}/` and `{provider}/{model}/`. `resolve_prompt(provider, model)` returns `ResolvedPrompt` with four-step fallback: hand-authored model variant -> generated model variant -> generated canonical variant -> root `system_prompt.md`.

**Per-skill LLM override:** DB-only via `skill_overrides` table (schema v20). `[llm]` section no longer supported in `skill.toml` (#504). `resolve_skill_llm_override()` constructs per-skill `LlmProvider`. **AlwaysOn + DB-override carve-out (mika#1011):** `AlwaysOn` skills with DB-sourced LLM overrides (`LlmOverride.from_db_override = true`, set by `apply_overrides()`) also qualify for override resolution — this ensures operator intent via `mika skills llm set` fires on autonomous-loop webhook turns where no keyword match exists. The #463 protection against `skill.toml [llm]` hijacks is preserved (developer-time overrides have `from_db_override = false`).

**Identity-driven skill allowlist (#811, #815):** All four well-known agents declare `[skills].allowlist` in `identity.toml` to own their skill set via identity rather than `skill_overrides` DB rows. mika-arch was the first (#811); mika-dev, mika-qa, and mika-relay followed in #815 (D2 cross-cutting). New bundled skills are denied by default unless explicitly added to an agent's allowlist — see the root `CLAUDE.md` § "Adding a New Bundled Skill" for the checklist. `SkillsIdentityConfig` in `prompt.rs` deserializes the `[skills]` block with `allowlist: Option<Vec<String>>`. `SkillRegistry::apply_identity_allowlist()` runs as Phase -1 before `apply_overrides()` — evicts all skills NOT in the allowlist (case-insensitive), following the same `retain()` + `DisabledSkill` pattern as Phase 0. Warns on allowlisted names not found in the registry. Empty or absent allowlist = no-op (all skills active). Wired into all skill-loading paths: server init, hot-reload (handlers.rs, a2a.rs), team engine, delegate_task, list_skills tool, and CLI (chat.rs, ask.rs, skills.rs). A one-time data migration (`migrate_well_known_to_identity_allowlist`, guarded by `schema_meta` marker `well_known_d2_migration_v1`) deletes stale `skill_overrides` denylist rows for the three migrated agents on first startup after deploy. **Default personal/customer agent (#1596):** the same "absent allowlist = no-op = all skills active" rule made fresh agents default-permissive — a personal `mika` provisioned via `bootstrap_fresh_install` (the family-rollout path when `add-customer.sh` runs without `--identity-dir`) loaded the engineering skills (`dev-pilot`, `dev-groom`, `mika-arch-*`, `qa-review*`, `self-dev*`) and leaked their internal `Disposition:`/`Verdict:` contract into user-facing replies. The default identity now ships a narrow operator-assistant allowlist — see `DEFAULT_AGENT_SKILL_ALLOWLIST` + `DEFAULT_IDENTITY` in `crates/mika-common/src/home.rs` (the personal-agent counterpart to the well-known-agent allowlists here; keep the two in sync). An explicit `[skills]` block in a provisioned `identity.toml` still overrides it.

**Boot-time tier guard (mika#1962):** `server::tier_guard::assert_family_tier_env_consistency(home_dir, tier)` refuses server startup when an agent is family-provisioned on disk while the process tier does not itself write the family templates. **Both gates ask that through `AgentTier::expects_family_provisioning()` — an exhaustive match — never `tier == AgentTier::Family` (mika#2023 M2).** The equality comparison they used to carry was a mine the compiler could not see: `AgentTier::Champion` is provisioned with the family templates on both axes, so introducing that variant under the old predicate would have made the boot guard `bail!` for the entire champion population, and the lazy-path gate below decline every champion at its first `/send`, with no compile error anywhere to announce either. **Unchanged blind spot, and it is the one that matters for launch:** the guard detects *family* provisioning only. A champion tenant bootstrapped before mika-cloud#209 (2026-08-28) carries the **operator** allowlist on disk, the guard does not fire, and the agent starts silently with `shell-exec`/`tmux`/`git-ops`/`github`. Nothing in mika#2023 retrofits that population — it is a re-provisioning gesture, and verifying it is a blocking gate before any champion account is opened. It is called from `run_server` immediately after `home::migrate_to_multi_agent` (which is what creates `agents/<name>/` — scanning earlier would read a pre-migration layout and find nothing) and before any agent is initialized. `run_server` has one production caller and zero test callers, so placing the guard inside it means a future binary entry point inherits it rather than having to remember to call it.

**Coverage set.** The guard enumerates via `servable_agent_names`, the **union** of `agent::list_agents` (which requires `config.toml`) and any `agents/*/` directory carrying an `identity.toml`. The two predicates disagree by design — `AppState::resolve_agent` gates only on `identity.toml` and `Settings::load_for_agent` adds the per-agent `config.toml` with `.required(false)` — so an agent home with identity but no config is fully servable while being invisible to `list_agents`. The guard must see what the server will serve.

**Post-boot agents.** The boot guard scans once. `resolve_agent`'s lazy-construction slow path (mika#1399) builds an `AgentState` at *request* time for agents that appeared since, and re-reads the tier from the server's env — so it calls the per-agent form, `tier_guard::check_agent_tier_consistency`, before constructing, and declines the agent (`error!` + `None`) on mismatch rather than aborting the process. The reachable drift here is a **two-process** disagreement, not a mid-runtime env mutation: `mika agents create <name>` run from a shell exporting `MIKA_AGENT_TIER=family` writes family templates to disk (`home::bootstrap` reads that shell's env), and the server — started without the var — would otherwise serve that family persona under operator semantics on first `/send`.

Detection is two independent OR-combined axes, both living in `mika-common::home` next to the constants they read: `soul_has_family_marker` (the `FAMILY_SOUL_MARKER` sentinel, a **tail** marker — see `mika-common/CLAUDE.md` for why it must not be the first line) and `identity_allowlist_matches_family` (set comparison against `FAMILY_AGENT_SKILL_ALLOWLIST`). Two axes because neither suffices alone — the marker is not retroactive, so **axis 2 is the load-bearing detector for every family agent provisioned before mika#1962**; and the allowlist can be legitimately hand-edited, which axis 1 then covers. Both would have to be broken to hide family provisioning. The `tier` is a parameter, not read from the env inside the guard, so the guard is a pure function of (disk state, tier) and its tests never mutate process-global env state.

Companion cache: `AgentState.tier` (`server/state.rs`) holds `AgentTier::from_env()` read once at `init_agent`, and all four production `ToolContext` construction sites read it via their params structs (`AgentParams`/`SilentAgentParams`/`TeamAgentParams` each carry a `tier` field) rather than calling `from_env()` per turn. `TaskDispatcher.tier` and `TeamEngine.tier` thread the same cached value into silent and team turns. **Structural gate:** `grep -rn 'AgentTier::from_env()' crates/mika-agent/src/` must return no production hit outside the tier-resolution surface: `server/mod.rs` (`init_agent` populating the cache, plus the boot-guard callsite), `server/state.rs` (the lazy-resolve guard callsite), and `server/tier_guard.rs` (message coherence). A hit in `agent_loop/`, `teams/`, `task_engine/`, or `server/investigate.rs` means a per-turn consumer stopped reading the cache. See `todos/1962-injection-verification.md` § I3 for why this gate is structural rather than a unit test.

**Identity-driven tool denylist (#811):** Mirror of the skill allowlist for built-in tools. Well-known agents declare `[tools].disabled` in `identity.toml` listing tools that must not appear in their LLM tool array. `ToolsIdentityConfig` in `prompt.rs` parses the section. `agent::apply_agent_tool_visibility()` is the named filter hook — applied inside `inject_skills_and_resolve_tools` at the LLM-tool-array assembly site, before the tool defs are converted to `LlmToolDefinition`. The model never sees disabled tools, cannot call them, cannot be prompt-injected into trying. The shared `Arc<ToolRegistry>` is unchanged — filtering is per-agent at the presentation layer. mika-arch denies platform-mutational tools (skill mutations, config writes, file writes, task mutations, PR merge, cross-agent invocation, agent/team mutations) while keeping `send_message` and memory writes (`update_core_memory`, `store_fact`, `update_fact`) allowed — both are agent-scoped self-state, constitutive of being an agent, not platform side-effects (see `docs/architecture/review-guide.md` § Orthogonality). Future migration: extend to support `[tools].allowlist` for the symmetric well-known-agent shape.

**Fail-closed identity for well-known agents (#811):** `prompt::load_identity()` distinguishes well-known from user-defined agents on parse failure. For well-known agents (detected by matching the home_dir's last component against `find_well_known_agent`), a malformed `identity.toml` returns a fail-closed `Identity` with a sentinel allowlist (`["__fail_closed_no_skills__"]`) that matches no real skill, plus the full `MIKA_ARCH_DISABLED_TOOLS` denylist. The agent is effectively neutered until the operator fixes the file. For user-defined agents, parse failure logs `error!` and falls back to `Identity::default()` (current behavior — no security contract to preserve). The discrimination happens inside `parse_identity_or_fail_closed()`, so callers don't need to know the agent's well-known status.

**Absence is not permission (mika#2027):** the same `load_identity()` used to answer a *missing* `identity.toml` with `Identity::default()`, whose `skills.allowlist` is `None` — and `apply_identity_allowlist` treats `None` as a no-op. So **deleting an agent's identity file made it MORE permissive**, not less: every bundled skill active, `shell-exec` / `git-ops` / `github` / `tmux` included. mika#1596 closed this class one step earlier (a provisioned agent with no allowlist); the file-entirely-absent case stayed open, and was reachable by a tier remediation that looks correct.

**Deleting it is permanent, which is why the widening was not self-healing.** Three gates each decline to rewrite the file: `bootstrap_fresh_install` is guarded by `home::is_initialized`, true for any tenant whose `data/mika.db` exists; `home::bootstrap` would rewrite it via `write_default_if_missing` but nothing calls it for an already-provisioned agent (the CLI's `ensure_initialized_for_agent` tests for `config.toml`, not `identity.toml`); and for a well-known agent, startup takes the existing-agent branch into `reconcile_well_known_identity`, which reads the on-disk file first and returns early when the read fails. Measured on a live tenant 2026-08-28: delete, full restart, still absent — ~2 min of exposure.

A read failure now returns the **same fail-closed sentinel as the malformed path**, and does so **universally** — well-known and user-defined agents alike (F1). An agent that needs skills must carry an explicit `identity.toml`, even a minimal one; that regression is the posture, not a side effect. **Sentinel rather than refusing to boot (F2, a deliberate divergence from the architect's first-pass suggestion):** mika-spirit serves every agent from one process, so a hard refusal would take the healthy agents down with the broken one, and it would contradict mika#1962's `tier_guard`, which explicitly tolerates a missing persona file.

**Two things "fail-closed" does NOT mean here, both worth knowing before relying on it.** (a) The denylist is `MIKA_ARCH_DISABLED_TOOLS`, which deliberately keeps `send_message` and the agent-scoped memory writes (`update_core_memory`, `store_fact`, `update_fact`) — #811 counts those as constitutive of being an agent, not platform side effects. So a neutered agent can still write its own core memory, which is re-injected into every later system prompt. Whether the fail-closed path wants a stricter denylist than mika-arch's steady-state one is a genuine open question and a separate change: the two share one constant, so tightening it here would also tighten mika-arch. (b) The sentinel is not confined to memory — `startup::seed_bundled_skills_if_needed` hands it to `materialize_agent_skill_links`, whose pass 2 removes the symlink of every de-allowlisted bundled skill, so a fail-closed start empties `<agent_home>/skills/` of its library symlinks. Recoverable (re-materialized on the first valid start) and scoped (marketplace and `--copy-managed` dirs untouched), but a real disk effect — including on a *transient* `identity_toml_unreadable`.

**One deliberate asymmetry remains, and it is narrower than it looks:** a *user-defined* agent fails closed on an absent file but still falls back to `Identity::default()` on a *malformed* one (the #811 behaviour above, unchanged). mika#2027 scopes itself to the absent case; widening the malformed path is a separate decision with a wider blast radius. The consequence to know when reading logs: a user-defined agent with a broken `identity.toml` can look healthy while running fully permissive — `identity_toml_malformed` is the only signal.

**Operator grep signals** (`$MIKA_SPIRIT_LOG_FILE`), three names because three remediations: `identity_toml_absent` (`ErrorKind::NotFound` — re-provision from the tier template), `identity_toml_unreadable` (present but permissions/I-O — fix readability, do NOT overwrite), `identity_toml_malformed` (fix the TOML). Each carries `agent` and `path`. Steady state is zero lines; any hit is an agent running with no skills (or, in the asymmetric case above, with all of them). Runbook: `docs/operator/agent-identity-reprovision.md`. Regression gates: `prompt::tests::mika2027_*` (the sentinel is produced, on both the sync and async loaders) and `skills::tests::mika2027_fail_closed_sentinel_evicts_every_skill` (it bites) — the pair is what makes "absent → zero skill" a fact rather than a convention. The tooled re-provision path is mika#2230. **Consequence for test authors:** a temp agent home must now write an `identity.toml` — `EvalHarness::build` and the `list_skills` / `toggle_skill` fixtures do, with a `name`/`emoji`-only file that parses to exactly `Identity::default()`. A fixture without one is not "the default agent", it is a neutered one.

**Skill enabled state:** DB-backed via `skill_overrides.enabled` column (schema v24, #629). Tri-state: `NULL` = default (enabled), `0` = disabled, `1` = explicitly enabled. `apply_overrides()` evicts disabled skills from `SkillRegistry.entries` into `disabled: Vec<DisabledSkill>` before applying `always_on`/LLM overrides. `enabled=false` always wins over `always_on=true` and over identity allowlist (DB override at Phase 0 runs after identity allowlist at Phase -1). `toggle_skill` agent tool and CLI `mika skills enable/disable` write to DB. Legacy `.disabled` marker files are migrated to DB rows on startup via `migrate_disabled_markers()` (one-shot, idempotent, fail-open). No match-time filter — disabled skills are evicted before matching (#630).

**Per-turn skill restriction over A2A (mika#2363).** `message/send` and
`message/stream` read an optional request-metadata key,
`mika.only_skills` (`mika_a2a::params::ONLY_SKILLS_KEY`), holding an array of
skill names. When present and non-empty, `run_a2a_agent` restricts **this turn's**
registry to those names via `SkillRegistry::apply_only_skills`. `mika ask
--only-skill <name>` (repeatable) writes it; `_arch_ask` in `dispatch-lib.sh`
passes the pass it is running.

**The measurement.** mika-arch declares three `always_on` skills —
`mika-arch-groom-ticket` (16 269 B), `mika-arch-second-review` (14 290 B),
`mika-arch-groom-milestone` (9 239 B) — and a turn runs exactly **one** pass. So
**23 529 B, 59 % of the skill portion of every architect system prompt, described
two tasks the turn was not doing**. Declaring the pass removes 23.5–30.6 KB
depending on which one it is. That saving is unconditional and costs no
correctness: nothing is summarised, truncated or elided — the reviewed plan, the
history window (mika#2295/#2330) and the user message are untouched.

**Strictly subtractive, and that is the whole safety argument.**
`apply_only_skills` computes the complement of the named set and delegates to
`apply_transient_disable`; it **never** calls `apply_transient_always_on`. A
skill named here that would not otherwise have been active is not resurrected, so
the field can only ever narrow a turn. That matters because `/a2a/{agent}` is
reachable by any authenticated caller: the additive half of the channel would let
one force a skill into an agent's prompt, and it buys nothing here — the skill
`_arch_ask` needs is already `always_on`. The additive half of mika#1727's missing
config channel therefore stays deferred, with that reason.

**Per-turn, never on the cache (R4).** The restriction is `&mut`, and the registry
lives behind an `Arc` on `AgentState` that concurrent turns share. `run_a2a_agent`
clones it, restricts the clone, and never writes it back — an empty request does
not even clone. Removing that clone would break no assertion: the declaring turn
would still be correct and the *next* turn would silently inherit its restriction.
Hence a lexical guard,
`server::a2a::tests::mika2363_the_restriction_is_applied_to_a_clone_and_never_written_back`.

**Fail-soft on the wire.** Key absent, `null`, not an array, an empty array — all
mean "no restriction", so a caller that declares nothing gets the pre-mika#2363
turn byte for byte. Non-string entries inside a valid array are dropped
individually rather than discarding the declaration, which would silently restore
the triple injection. A name the registry does not carry keeps nothing: the turn
ends with zero skills, which is loud (`active_skill_count = 0` on
`system_prompt_assembled`, no output-contract guard armed) and deliberately not
softened into a no-op.

**What it tightens, and how to disarm it.** `collect_required_suffix_lines` unions
the lines of every `AlwaysOn`-matched skill, so an *unrestricted* architect turn
accepted five lines — a first pass could satisfy the suffix-line guard with
`Verdict: GROOMED`, a contract `_parse_disposition` does not read and reports as
`UNPARSED`. A declaring turn accepts only its own pass's lines. This is a
tightening: a model that leaned on the tolerance now costs one corrective
re-prompt instead of passing. Watch `UNPARSED` counts after deploy; to disarm,
delete `--only-skill "$skill"` from `_arch_ask` — shell only, no binary redeploy.
`escalate_unattested_disposition` (mika#2037) is unaffected: it escalates within
the withdrawn line's own family, and each arch skill declares its own family's
`ESCALATE` (pinned by
`agent_loop::tests::mika2363_escalation_still_has_a_declared_target_under_restriction`).

**The trap, pinned.** Do **not** flip the three arch `always_on` flags to `false`
now that the pass is declared. The channel cannot activate anything, and keyword
routing cannot stand in — `mika-arch-groom-ticket` triggers on `plan review` and
`mika-arch-second-review` on `second pass`, both of which occur in the body of a
plan, so the content under review would select the skill that reviews it (T2b).
With the flags off, an architect turn carries no prompt at all: no
`required_suffix_lines`, no `required_finding_list_prefixes`, no review-anchor
contract — and nothing fails loudly, `_parse_disposition` just returns `UNPARSED`
on every groom. `tests/only_skills_arch_pass_2363.rs` refuses the flip with that
reasoning in its failure message.

**Observability, no new instrumentation.** `a2a_only_skills_applied` (INFO, fields
`agent`, `task_id`, `requested`, `kept`, `evicted_count`, `unknown_count`) and
`only_skills_unknown_name` (WARN). The reduction itself is attested by the two
surfaces that already existed: `system_prompt_assembled.active_skill_count` /
`.per_skill_bytes` (mika#1217) and `turn_usage.system_prompt_bytes` (mika#1889).
**Both must move together** — one moving alone means the measurement is wrong, not
the system.

### Per-turn model override over A2A (mika#2304)

Sister key of `mika.only_skills`, same channel, **opposite failure policy** — and
the asymmetry is the design decision, not an oversight.

`message/send` and `message/stream` read `mika.model_override`
(`mika_a2a::params::MODEL_OVERRIDE_KEY`), the raw string the operator typed.
`resolve_caller_model_override` resolves it against **this** agent's
`llm_provider` (alias → conditional prefix strip → API-key check, all in
`mika_common::llm::model_override`, the single site `mika-cli` also calls) and
builds a provider for the turn. `run_a2a_agent` passes it as `AgentParams.llm`
and sets `caller_model_override: true`, which makes `resolve_skill_llm_override`
stand down — an operator running a provider pre-flight must not have their model
replaced by a skill's `[llm]` section.

**Reading is fail-soft; applying is fail-CLOSED.** Key absent, `null`,
non-string, blank → no override, and the turn is the pre-mika#2304 turn byte for
byte. But a **declared** override this agent cannot serve refuses the request,
before the task row exists and before the agent lock is taken, as a JSON-RPC
`INVALID_PARAMS` naming the model and the provider. It is never degraded to "no
override": a skill restriction silently dropped makes a turn *wider*, which is
visible and falsifies no measurement; a model silently dropped makes the
measurement **wrong while producing a plausible answer** — the founding defect.

**The key check precedes the construction, and that order is load-bearing.**
`create_provider_with_budget` routes the ten OpenAI-compatible variants —
OpenRouter among them, the rail the founding measurement ran on — to
`OpenAiCompatibleProvider::new`, which returns no `Result` and never consults
`api_key`. A missing key therefore *succeeds* at construction, so the refusal has
to be **posed**, never hoped for. Pinned by
`server::a2a::tests::mika2304_the_key_is_checked_before_the_provider_is_built`. What
no layer can check before the call is whether the provider serves that model id:
an unknown id fails on the provider's own 400/404, already fail-closed, with
nothing to write.

**The attestation is taken where the turn was served, not where it was handed
in.** `AgentOutput.effective_model` is filled in `run_agent_inner` from
`effective_llm` — *after* the per-skill recompute — and `handle_message_send`
stamps it on `Task.metadata` under `mika.effective_model` at the mika#2270
intervention point. Reading it in `a2a.rs` off `agent_state.llm` would compile and
be wrong on exactly the per-skill population: a field asserting a model that did
not run, wearing the authority of a server attestation. Written on **every** turn,
override or not, so the client can read absence as "this server did not say" and
refuse to print a local value.

**Scope, stated rather than assumed:** synchronous `message/send` — the path both
doors of `mika ask` take. `message/stream` refuses an unserviceable override
identically (same key, same policy) but stamps nothing: it serves events, not a
rebuilt `Task`. `returnImmediately` runs no turn, so there is nothing to attest.
Both fall in the honest "not attested" population.

**Operator grep signals** (`$MIKA_SPIRIT_LOG_FILE`): `a2a_model_override_applied`
(INFO — a turn ran under a caller's model; expected rare, correlated with
pre-flight campaigns; a sustained flow means an automated caller is imposing one
and deserves identifying) and `a2a_model_override_refused` (WARN — **expected
regime: zero lines**; each one names a model or a missing API key, i.e. an
operator typo or an unconfigured agent, not a defect of the channel).

**Transient enable/disable overrides (#682):** Two methods handle per-invocation skill overrides, called after `apply_overrides()` (disable first, enable second — matches Phase 0/1 pattern): (1) `SkillRegistry::apply_transient_disable(skill_names)` evicts named skills from the registry entirely for a single invocation. Returns `TransientDisableResult` with `not_found` list. (2) `SkillRegistry::apply_transient_always_on(skill_names)` sets `always_on = true` on named skills. Returns `TransientOverrideResult` with separate `disabled` and `not_found` lists. Cannot resurrect disabled (evicted) or skipped skills. Neither is persisted.

**Who calls them today, and the CLI does not (mika#1727, mika#1883).** This paragraph used to read "Used by `mika ask --disable-skill`" / "`--enable-skill`", and that has been false since `mika ask` became a thin client — those two flags mutate the **CLI process's** registry, which is not where the turn runs. Their arg-level conflict check survives (a skill named in both is still a hard error before any registry op) and mika#1883 makes the inertia audible on stderr, but neither flag reaches these methods. The live caller of `apply_transient_disable` is `apply_only_skills` (mika#2363), server-side, by complement: `--only-skill` is the one selection flag that travels (`mika.only_skills`), and it is **strictly subtractive**, which is why it delegates here and never to `apply_transient_always_on`. That second method stays reachable from the DB-override path only; the additive half of the config channel is deliberately not exposed on `/a2a/{agent}` — it would let any authenticated caller force one of the agent's skills to `always_on`.

**Oversized prompt handling (#630):** Skills with prompts exceeding their size limit are hard-skipped at scan time (pushed to `ScanResult.skipped`) regardless of `always_on` status. This prevents zombie skills with tools but no prompt context. Tool-only skills (no `system_prompt.md`) are unaffected — they load with an empty prompt via `SnippetLoadResult::Empty`.

**Startup logging:** `SkillRegistry::log_summary()` emits a three-state `DEBUG` line (`loaded=N disabled=N skipped=N`) plus per-skip `WARN` lines. Call after both `apply_overrides()` and `apply_load_safety_check()` for accurate counts.

**Validation:** `validate_skill()` checks name-in-keywords rejection (#510), markdown validation (#511), required_tools references, context types, and `{{key}}` placeholders. **Load-time crash-protection (#530, #1335):** `SkillRegistry::apply_load_safety_check()` runs `validate_skill()` on every loaded skill after `apply_overrides()`. This is NOT the validation gate — CI and `mika skills validate` own change-time validation. This method is a runtime safety net that prevents malformed manifests from crashing the agent. Decision matrix: missing handler/broken tools.json → skip skill entirely; deprecated `[llm]` section/name-in-keywords/invalid markdown → load with warning. Results stored in `validated_warnings` for TUI/CLI display. `is_skip_worthy_failure()` classifies Fail diagnostics.

**Required tools enforcement:** Optional `[constraints]` section with `required_tools`. `collect_required_tools()` computes union across keyword-matched skills only. One retry on EndTurn violation.

**Required suffix-line enforcement (#864):** Optional `[output]` section with `required_suffix_lines`. `collect_required_suffix_lines()` computes union across keyword-matched AND always-on skills (not dependency). One retry on EndTurn violation. `validate_skill()` warns on explicitly-empty lists and rejects empty/whitespace entries.

**Required finding-list enforcement (#901):** Optional `[output]` section with `required_finding_list_prefixes`. `collect_required_finding_list_prefixes()` computes union across keyword-matched AND always-on skills (not dependency). Enforced only on terminal dispositions (ITERATE/ESCALATE) — non-terminal (READY/GROOMED) are exempt. Scan range: message start up to the suffix-line landmark. One retry on EndTurn violation. `validate_skill()` warns on explicitly-empty lists and rejects empty/whitespace entries. Same Warn-not-Fail pattern as suffix-line validation.

**Match-reason conditioning (#463):** `match_skills()` returns `MatchedSkill` wrappers with `MatchReason` (`Keyword`, `AlwaysOn`, `Dependency`). `always_on` skills do not enforce constraints unless the user's message also triggered a keyword.

**Review-target exclusion (#513):** `review_filter::apply_review_filter()` runs after `match_message()` and before `resolve_contexts()` in both conversation and team mode. When `skill-review` is keyword-matched, any other keyword-matched skill whose name appears in the user message (case-insensitive) is excluded from the matched set. This prevents the reviewed skill's prompt from contaminating the review context. `AlwaysOn` and `Dependency` skills are never excluded. Silent mode is unaffected (no keyword matching).

## Exec Handlers

**Image protocol (`__mika_v1`):** Scripts return images via JSON envelope `{"__mika_v1": {"text": "...", "images": ["/path/to/img"]}}`. Executor validates files (5MB limit, magic-byte check for JPEG/PNG/GIF/WebP), base64-encodes, max 5 images per result.

**Long-running:** `long_running: true` + `estimated_duration_secs` in `skill.toml`. Conversation mode and `DeferredDispatch` silent mode (#1058). Creates callback task, injects `__mika_task_id` and `__mika_agent` env vars, spawns detached process. PID recorded for orphan cleanup. **Callback deferred dispatch (#1058):** When a callback or DeferredDispatch turn calls a long-running tool and `long_running_ctx` is `None`, the executor gate intercepts the call via `callback_task_id` on `ToolContext`. Instead of a hard error, it runs `check_lineage_cycle()` (lineage walk on `(repo, issue_number, skill)` tuple, max 4 hops, fail-open on extraction failure) and, if no cycle is detected, calls `register_deferred_callback()` to enqueue the dispatch. Returns `{"status": "deferred", "deferred": true}` so the LLM knows not to retry. The deferred callback fires as a `DeferredDispatch` silent turn which HAS `LongRunningContext` injected. Cycle detection rejects same-tuple re-dispatch (e.g., `groom-#159 → retry-groom-#159`) but allows cross-skill chains (e.g., `groom-#159 → pilot-#159`). **Dispatch-readiness guard (#525):** before spawning, `validate_dispatch_readiness()` enforces seven checks: (0) unauthorized webhook dispatch (#933) — if `originating_message` is present and matches the Webhook Fallthrough domain (`[GitHub]` prefix excluding ready-label, PR, and check-suite events), rejects with `unauthorized_webhook_dispatch` before any DB access. Pure string-prefix check, cheapest guard. Predicate shared via `crate::webhook_dispatch::is_unauthorized_webhook_dispatch()`. (1) task status must be `pending` or `in_progress` (rejects `blocked`/`completed`/`cancelled` with structured JSON error `task_not_dispatchable`), (2) no active callback child task may exist (rejects with `task_active_dispatch`), (3) no other task of the same dispatch class may have an active callback child — per-class slot guard (rejects with `global_dispatch_active`, scoped to `agent_id` + `dispatch_class`) (#583, #1001). `dispatch_class` is `'implement'` (dev-pilot, deploy_mika) or `'groom'` (dev-groom); pre-v34 NULL rows are treated as `'implement'` via SQL `COALESCE`. One implement + one groom dispatch may run concurrently per agent. The rejection JSON includes `blocker_kind` (`"real_callback"` or `"deferred_wrapper"`) and `blocking_label` for agent-native diagnostics (#1172 W3), (4) per-turn dispatch counter must be zero — only one long-running dispatch per agent turn (rejects with `dispatch_limit_exceeded`) (#583), (5) grooming-marker check (#919, #1108) — if the task's `reference_url` points to a GitHub issue AND the dispatch skill is `dev-pilot` AND `task.type == "issue"`, fetches the issue body via REST API and checks for three canonical grooming callouts: `> - **Branch:**`, `docs/plans/`, and a `second-pass` marker (canonical `(GROOMED)` or spec-tolerated `(READY, paraphrased GROOMED ...)`). Rejects with `dispatch_no_grooming_marker` (listing `missing_signals`) if any are absent. Bypass predicates: non-`dev-pilot` skill, non-issue task type, non-GitHub-issue reference_url, or `MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1` env var (WARN-logged). Fail-open when no `github_token` configured; fail-closed on API errors. Coupled pair with `skills/bundled/self-dev/system_prompt.md:253` (defense-in-depth prompt-level check), (6) GitHub `blockedBy` check — if the task's `reference_url` points to a GitHub issue, queries the GraphQL API for open blockers and rejects with `dispatch_blocked_by` if any are still open (#713). Fail-open when no `github_token` configured (check skipped with warning); fail-closed on API errors. Uses GraphQL variables (not string interpolation) for injection safety. `extract_open_blocker_numbers()` parses the response. `LongRunningContext` carries `dispatch_count: AtomicU32` initialized to 0 per turn; incremented after task creation and path validation, right before subprocess spawn. `LongRunningContext` also carries `originating_message: Option<String>` (#933) — populated from the latest user-role message in conversation mode, `None` for silent triggers. Fail-closed on DB errors. Auto-transitions `pending` tasks to `in_progress` on successful dispatch. Stricter than the shared `validate_task()` which also allows `blocked` for `delegate_task`. **Dispatch-rejection observability (#1108):** All 7 rejection sites write the structured JSON error to `tasks.result` via `record_dispatch_rejection()` (fire-and-forget, warn on DB failure). This surfaces rejection reasons to operator-visible surfaces (`mika tasks list`, dashboard task detail) without requiring DB-level inspection. The `write_task_dispatch_rejection()` DB method is agent-unscoped (keyed by `task_id` + `trigger_type = 'manual'`) because the earliest rejection site (unauthorized webhook) fires before the task is fetched.

**Cancel discriminator protocol (#749):** When `cancel_task_and_kill` terminates a long-running subprocess, it pre-writes a reason file at `/tmp/mika-cancel-reason-{pid}` with `STATUS=CANCELLED_BY_OPERATOR` before sending SIGTERM. The shell-side TERM trap in `dispatch-lib.sh` writes `STATUS=CANCELLED_BY_SIGNAL` only if no reason file exists (belt-and-suspenders for signal-initiated cancels). The EXIT trap reads the reason file and prefixes the callback envelope so the consumer (`self-dev-callback`) can distinguish cancel from crash. Two discriminators: `CANCELLED_BY_OPERATOR` (cancel_task initiated) and `CANCELLED_BY_SIGNAL` (signal-initiated, no pre-write). Absence of the prefix = existing `HANDLER CRASH` / success paths fire unchanged (backward compatible).

## MCP (Model Context Protocol) Client

Connects to external MCP servers at startup via `McpManager`. Configured in `{agent_home}/mcp.json`. Supports stdio and Streamable HTTP transports. Tools namespaced as `mcp__{server}__{tool}`. Dispatch chain: builtins -> skills -> MCP -> unknown error. MCP tools excluded from silent/heartbeat mode. Child processes use `env_clear()` + allowlist.

## Evaluation — Golden Dataset (#339)

`tests/eval/golden/` — 25 curated end-to-end quality scenarios across 4 capability classes: Memory (8), Tool Selection (8), Conversation Quality (5), Skill-Specific (4). Each scenario has hard assertions (regression-gating) and optional soft tags (`quality:*` namespace, observability-only). Sibling tickets own their namespaces: #740 `self-knowledge:*`, #741 `grounding:*`.

**Three-tier execution model (D6):**
1. **Unit** — `MockLlmProvider`, runs on every CI push: `cargo test -p mika-agent --test eval golden`
2. **Integration** — real providers via `MIKA_EVAL_REAL_PROVIDERS` + `--ignored`
3. **Calibration** — integration + `MIKA_EVAL_CALIBRATE=1` artifact capture for weekly drift detection (#742)

**Scenario registration (D7):** Each scenario calls `register()` on `GoldenRegistry`. `HashMap::insert` uniqueness guard panics on duplicate names at test binary load time — protects against copy-paste-without-rename.

**Scoring (D4):** `GoldenOutcome` carries `hard_assertions: Vec<HardAssertion>` (pass/fail) + `soft_tags: Vec<SoftTag>` (LLM-judged quality signals). Judge model pinned to `claude-sonnet-4-6` with `MIKA_EVAL_JUDGE_MODEL` override. Tag-based judging — no free-form 0-10 scores. Judge-deprecation-as-reset protocol: when pinned model is EOL'd, baseline resets via explicit PR.

**Ticket-namespaced vocabulary (D4):** #339 owns `quality:concise`, `quality:verbose`, `quality:uncertain`, `quality:actionable`, `quality:off-topic`. Each sibling ticket defines its own namespace.

See `tests/eval/golden/README.md` for author-facing guidance (fixture patterns, assertion style, how to add scenarios).

## Evaluation — KG Provider Comparison (#762)

`tests/eval/kg_provider_eval/` — reproducible harness comparing LLM providers for the two KG call types (entity extraction and entity resolution). Uses direct LLM calls with the *production* KG prompts (not the full agent loop) so results reflect prompt-level provider behavior.

**Gating:** `#[ignore]` + `MIKA_EVAL_KG_PROVIDERS` env var (separate from `MIKA_EVAL_REAL_PROVIDERS` to avoid accidentally running during the basic provider matrix). Format: comma-separated `provider/model` strings (e.g. `anthropic/claude-haiku-4-5-20251001,openrouter/deepseek/deepseek-v3`) or `default` for the four-provider minimum set (Anthropic Haiku + Sonnet, OpenRouter DeepSeek + Kimi). Each referenced provider must have its API key set.

**Fixtures:** 15 sample docs (`docs/solutions/kg/eval-fixtures-2026-04-24/extraction_sample_docs.toml`) and 30 hand-labeled resolution ground-truth cases (`resolution_ground_truth.toml`). Scoring: extraction uses entity-set F1 + triple-set F1 against annotated expectations; resolution uses exact-match accuracy against the labeled correct candidate.

**Outputs:** per-run report with per-provider quality/cost/latency tables. Decision matrix lives at `docs/solutions/kg/kg-provider-evaluation-2026-04-24.md` (populated by running the eval with API keys). Compound pattern doc at `docs/solutions/best-practices/kg-provider-eval-harness-reproducible-comparison-2026-04-24.md`.

**Run:** `MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval`

## Evaluation — Model Calibration (#1190)

`src/calibration/` — Pre-swap calibration gate for agent model changes. Role-scoped scenario suites with structural assertions (no LLM-as-judge). Framework lives in the library crate; the `calibrate` binary (`src/bin/calibrate.rs`) provides the operator-facing CLI.

**Three role suites (v1):**
- **mika-dev** (5 scenarios): refusal_regression (#1168), contract_dev_groom (#1166), golden_path_dispatch, required_tools_gate, plan_callout_recognition
- **mika-arch** (5 scenarios): groom_ticket_basic, groom_milestone, citation_discipline, disposition_keyword_discipline, required_finding_list
- **mika-qa** (5 scenarios, #1632): verdict_format_precision, per_ac_enumeration, absence_claim_grounding, wip_rescue_skip, no_fabricated_fix

**Fixtures:** Markdown inputs at `tests/eval/calibration_fixtures/<role>/<scenario>.md` + YAML manifests at `tests/eval/calibration_fixtures/<role>/manifest.yaml`.

**Run:** `make calibrate-mika-dev MODEL=anthropic/claude-sonnet-4-6` or `make calibrate-mika-arch MODEL=anthropic/claude-opus-4-6`. The binary accepts `--baseline <path>` for pass-rate comparison and `--output <path>` for artifact location.

**Module structure:**
- `calibration/artifact.rs` — CalibrationArtifact JSON schema, diff tool (promoted from `tests/eval/calibration.rs`)
- `calibration/scenario.rs` — ScenarioOutcome, Scenario types (promoted from `tests/eval/scenarios.rs`)
- `calibration/providers.rs` — Provider construction helpers (promoted from `tests/eval/providers.rs`)
- `calibration/role.rs` — RoleScenario, RoleScoreReport, RoleManifest, YAML manifest schema
- `calibration/failure.rs` — FailureClass enum (Refusal, Fabrication, EmptyResponse, Timeout, TransportError, ContractViolation, Other)
- `calibration/roles/mika_dev.rs` — mika-dev scenario implementations
- `calibration/roles/mika_arch.rs` — mika-arch scenario implementations
- `calibration/roles/mika_qa.rs` — mika-qa scenario implementations (#1632)

**Baselines:** `docs/eval/calibration/baselines/`. Every model-swap PR must include the calibration report.

**Pre-swap discipline:** See root `CLAUDE.md` — no model swap merged without a passing calibration run.

## Evaluation — Grounding Regressions (#741, #862, #863, #864, #890, #894, #901, #1059)

`tests/eval/grounding_regressions/` — 39 fabrication-detection scenarios. Scenarios 1–5 from the KG milestone #14 retrospective (#741), scenarios 6–7 from the gate-evasion compound doc (#862), scenario 8 (3 tests) from the elided-copula regex extension (#894), scenarios 9–11 from the quoted-resource pre-fetch guard (#863), scenarios 12–16 from the required-suffix-line verdict-ghosting guard (#864), scenarios 17–19 from the required-tools-gate transport-contract fix (#890), scenarios 20–21 from the qa-review per-AC enumeration fix (#1059, mika-skills#159), scenarios 22–29 from the required-finding-list conditional-disclosure-evasion guard (#901), scenarios 30–33 from the dev-groom fabrication guard (#1133), scenarios 34–37 from the assert-grounded affirmative-claim guard (#1331). Each tests a concrete fabrication class with hard assertions (forbidden-word, required-tool, contains-in-order, contains, per-element-enumeration, absence-grounding). No LLM-judge gating — each class has objectively checkable signals.

**Scenarios:** GraphQL field fabrication (#720), auto-merge vs merged (#727), core memory priority drift (#732), fabricated shell errors (feedback doc), KG result ignored (#740 D4), asserted unavailability caught (#862 — guard fires on fabricated unavailability claim), asserted unavailability genuine (#862 — guard does NOT fire on genuinely disabled tool), asserted unavailability elided-copula/elided-skill-scoped/adverb-interposed (#894 — extended regex catches elided-copula and adverb-interposed shapes), quoted resource pre-fetch caught/no-op/mixed (#863 — pre-fetch guard augments required_tools from brief content), required suffix line caught/pre-fix/position-3/position-4/unconstrained (#864 — verdict-ghosting guard fires on missing suffix line), required-tools retry thin-final-turn regression/post-fix/correction-message (#890 — transport-contract guard: final turn must be self-contained after required-tools retry), qa-review per-element enumeration/absence-claim grounding (#1059 — per-AC enumeration rule and absence-claim evidence rule from mika-skills#159), required finding list caught-on-iterate/no-op-on-ready/no-op-when-unset/position-inclusive/position-exclusive/position-at-message-start/caught-on-verdict-escalate/no-op-on-verdict-groomed (#901 — finding-list guard fires on thin F-list emission with terminal disposition), dev-groom fabricated verdict caught/caught-escalate/dispatched-no-verdict/status-response-no-verdict (#1133 — dev-groom fabrication guard fires on Verdict claim without dispatch), assert-grounded PR state caught/PR state satisfied/false-positive guard/task state caught (#1331 — assert-grounded guard fires on affirmative state claims without grounding tool call).

**Assertion helpers:** `tests/eval/grounding_assertions/mod.rs` — `assert_response_forbids`, `assert_any_tool_called_from`, `assert_response_contains_in_order`, `assert_response_contains`, `assert_tool_called_before_response`, `assert_response_contains_question`, `assert_response_contains_per_element_enumeration`, `assert_absence_claim_grounded`.

**Frozen regression fixtures:** Each scenario has a `fixtures/{scenario}_pre_fix.json` file with the pre-fix response that demonstrates the failure class. Regression-reproduction tests prove assertions catch the failure.

**Tag vocabulary (`grounding:*`):** `fabricated-ref-suppressed`, `completion-claim-suppressed`, `source-cited-correctly`, `verification-before-claim`, `uncertainty-admitted`, `training-data-hallucination` (failure), `transport-contract-thin-final-turn` (failure), `transport-contract-self-contained`. Scope boundary with #740 `self-knowledge:*`: self-knowledge = query-invocation code paths; grounding = response-to-evidence paths.

See `tests/eval/grounding_regressions/README.md` for the full vocabulary, capability matrix, and how to add scenarios.

## Evaluation — Doctrine Regressions (mika#1814)

`tests/eval/doctrine_regressions/` — public-promo suppression scenarios for the Distribution Doctrine EndTurn guard (position 5c) and prompt-shape contract for the `## Distribution Doctrine` section rendered by `prompt::build_system_prompt` / `build_silent_prompt`. Sibling of `grounding_regressions/` scoped to doctrine-shape (not evidence-fabrication).

**Scenarios (D2):** `doctrine_public_promo_show_hn_caught` (FR founding-incident shape — Al B 2026-07-20; guard catches, agent redirects to invitation chain), `doctrine_public_promo_product_hunt_caught` (EN bilingual coverage — same guard fires on Product Hunt / growth-hack / Reddit-launch surfaces), `doctrine_public_promo_educational_answer_no_op` (false-positive avoidance — Layer A hit + Layer B miss must NOT fire), `doctrine_prompt_section_rendered` (headless-safe AC1/AC9/AC11 assertions — heading present in operator-tier + family-tier, absent in compact-provider carve-out, bearing memory cited by name).

**Tag vocabulary (`doctrine:*`):** `doctrine:public-promo-proposed` (pre-fix failure), `doctrine:public-promo-suppressed` (post-fix success — guard caught and corrected turn stripped drafting language), `doctrine:invitation-only-honored` (post-fix success — corrected turn carries the invitation-chain redirect).

**Scope boundary:** `grounding:*` (mika#741) = evidence fabrication (unattempted tools, hallucinated citations); `self-knowledge:*` (mika#740) = query-invocation code paths; `doctrine:*` (mika#1814) = content-shape violations of a load-bearing product doctrine.

**Assertion helpers:** Reuses `tests/eval/grounding_assertions/mod.rs` (`assert_response_forbids`, `assert_response_contains`) — shared across grounding + doctrine regression suites.

### Calibration CI Workflow (#742)

`.github/workflows/eval-calibration.yml` — Weekly scheduled workflow that keeps the committed baseline (`tests/fixtures/eval-baseline.json`) trustworthy by detecting provider tolerance drift.

**Schedule:** Mondays 08:00 UTC (cron) + `workflow_dispatch` for on-demand runs.

**How it works:**
1. Runs full calibration matrix (`MIKA_EVAL_REAL_PROVIDERS=all`, `MIKA_EVAL_CALIBRATE=1`)
2. If no baseline exists → `eval-diff bootstrap` creates the initial file, opens PR with label `calibration-bootstrap`
3. If baseline exists → `eval-diff diff` compares artifacts:
   - Exit 0 (no drift): logs success, no PR
   - Exit 1 (drift detected): updates baseline, opens PR with labels `eval-calibration` + `drift-detected`
   - Exit 2 (error): workflow fails, no PR

**Provider secrets:** `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `GROQ_API_KEY` in repo Settings → Secrets. Missing keys are soft-skipped (logged as warnings, remaining providers still run).

**`eval-diff` CLI** (`src/bin/eval-diff.rs`):
- `eval-diff bootstrap <artifact> [<baseline>]` — Copy artifact as committed baseline (default: `tests/fixtures/eval-baseline.json`)
- `eval-diff diff <baseline> <new>` — Semantic diff; exit 0 = no drift, 1 = drift, 2 = error
- `eval-diff format-pr-body <baseline> <new>` — Generate Markdown title + body for drift PR

**Manual trigger:** Go to Actions → "Eval Calibration" → "Run workflow" → select branch (usually `main`). Useful before model upgrades, after prompt changes, or to verify baseline freshness.

**Expected PR cadence:** ~0-2 drift PRs per month under normal conditions. A burst of drift PRs suggests a provider changed models or a prompt change shifted outcomes — investigate before blindly merging.

**Interpreting a drift PR:** The PR body contains a fenced JSON block listing each changed scenario with provider, model, previous/new outcome, and classification. Review the changes to confirm they reflect real provider behavior shifts, not workflow bugs.

**Rolling back a merged baseline:** `git revert <merge-sha>` on the baseline-update PR. The next weekly run will re-detect the drift and open a fresh PR.

**Cost:** ~$2.57/run (four providers × full scenario matrix). Annual: ~$150 (52 weekly + ~10 manual runs). Bounded by 30-minute workflow timeout.

## Knowledge Graph — Domain Graph Builder

`src/kg/domain_builder.rs` — Deterministic startup-time builder that populates `kg_entities` and `kg_relationships` from five authoritative sources: `SkillRegistry`, `ToolRegistry`, `McpManager`, agent configs, and concept seeds (hardcoded). Runs once per server boot in `run_server()` after all agents are initialized. No LLM calls — pure code projection.

**Entity types:** Skill, Tool, Agent, ProblemType (5 seeds: `ci_failure`, `merge_conflict`, `duplicate_pr`, `stale_uuid`, `fabrication`), Concept (20 seeds: 7 `concept:cross-repo:*` + 13 `concept:infra:*`). **Relationship types:** `DEPENDS_ON` (Skill→Skill from `skill.toml` dependencies), `PROVIDES` (Skill→Tool from skill tools).

**Sole-writer contract:** This module is the sole writer of `skill:*`, `tool:*`, `agent:*`, `problem_type:*`, and `concept:*` entity_keys. No other code path writes these namespaces.

**Idempotency:** Entity UPSERT via `INSERT ... ON CONFLICT(entity_key) DO UPDATE` preserves rowids (protects `kg_subject_resolutions.domain_entity_id` FK references). Relationships are DELETE-all-then-INSERT per rebuild. Stale entities are pruned with a type-scoped DELETE that only touches `KG_DOMAIN_ENTITY_TYPES`.

**Failure policy:** Rebuild failures log `warn!` — the server continues to boot. KG queries return stale or empty results until the next successful rebuild. **Staleness contract:** Domain graph reflects registry state as of the last server boot.

**Resolution invalidation on type expansion (#960):** When `rebuild()` adds N≥1 entities of type T, all `kg_resolutions_log` rows with `outcome='no_match'` for subjects where `kg_subject_entities.type = T` are deleted in the same transaction. The next resolver tick re-attempts those subjects against the expanded domain graph. Per-type invalidation counts surface via the `domain_rebuild_invalidated_resolutions` log event. `matched_*` outcomes are NOT invalidated (re-ranking against a better-matched entity is a separate ranking concern). Sub-namespace over-invalidation explicitly accepted: adding one `concept:infra:*` entity invalidates `no_match` rows for all `concept`-typed subjects; cost bounded by `MIKA_KG_BATCH_BUDGET`. First restart after deploy may see elevated `pending_before` on the next resolver tick.

**Observability:** Single `trace_id` per rebuild invocation, INFO-level structured logs (`domain_rebuild_start`, `domain_rebuild_entities`, `domain_rebuild_edges`, `domain_rebuild_invalidated_resolutions`, `domain_rebuild_complete`). No `audit_events` rows (per conventions C3.1).

**Cross-cutting conventions:** See `docs/architecture/kg-implementation-conventions.md` (C1–C3) and `docs/architecture/kg-id-convention.md` for the `<type>:<name>` entity key format.

## Knowledge Graph — v27 Migration Recovery

If a database restart between #786 and #787 deployments leaves the DB stuck at v27 with empty tables and the `v27_coalesce_complete` marker absent, see `docs/solutions/database-issues/kg-v27-stuck-migration-recovery-2026-04-24.md` for the operator recovery procedure.

## Knowledge Graph — Docs Root Configuration

`src/kg/config.rs` — Resolution chain for the docs root path the lexical ingestor reads (#738, #778). Two levels of resolution:

**Global fallback** (#738): `resolve_kg_docs_root(&Settings) -> (PathBuf, PathSource)`. Resolution order (first hit wins): `MIKA_KG_DOCS_ROOT` env var > `kg_docs_root` config.toml field > `<CWD>/docs/solutions` (container-native default).

**Per-agent resolution** (#778): `resolve_per_agent_docs_root(&Identity, &Settings) -> Result<KgAgentConfig, KgConfigError>`. Each agent's `identity.toml` gains a `[kg]` section:

```toml
[kg]
enabled = true                    # default: true
docs_root = "/absolute/path"      # optional; falls back to global chain above
```

**Behavior matrix:**

| `enabled` | `docs_root` set | Behavior |
|-----------|-----------------|----------|
| `true` (default) | set | Validate path exists as directory; hard-error if not; else use it with computed `docs_root_hash`. |
| `true` | unset | Fall back to global resolver. Hard-error on explicit env/config source if missing; warn-and-skip on CWD default. |
| `false` | any | Skip KG entirely (no `LexicalIngestor`, no `SubjectExtractor`, no `SubjectEntityResolver`). Existing shared-corpus rows preserved. |

**Types:** `KgAgentConfig` enum (`Disabled` / `Enabled { docs_root, docs_root_hash }`), `KgConfigError` via `thiserror`. Resolved at `init_agent` time, cached on `AgentState.kg_config`. Per-agent failure isolation: a single agent's KG misconfiguration skips that agent; others start normally.

**Hard-error policy:** Explicit paths (per-agent or global config/env) that don't exist fail loud. CWD-based default uses warn-and-skip per #738's policy. `enabled=false` does NOT delete existing rows — cleanup via #779's CLI.

**Well-known agent KG topology (#800):** mika-arch is the sole KG consumer among well-known agents. mika-dev and mika-qa are provisioned with `[kg].enabled = false` — they have zero `query_knowledge_graph` usage (retrieval goes through `search_memory` over `memory_facts`). This eliminates the shared-corpus extractor race on the mika-docs corpus where multiple agents redundantly called the LLM for the same doc. Re-enable per-agent with one `identity.toml` edit (`enabled = true`) + restart if a dev/qa flow needs KG-backed retrieval.

For OpenRC hosts where the service starts with CWD ≠ repo root, set `MIKA_KG_DOCS_ROOT=/path/to/mika-repo/docs/solutions` in the service config, or use the existing `--chdir` init-script workaround.

### Multi-Corpus Support (#798)

Agents that need to reason across multiple repositories can specify an array of docs roots. The `mika-arch` agent is the canonical example — it indexes all six platform repos' `docs/solutions/` directories.

**Per-agent identity.toml recipe (mika-arch):**

```toml
name = "mika-arch"
emoji = "A"

[kg]
enabled = true
docs_roots = [
  "/data/workspace/mika-platform/mika/docs/solutions",
  "/data/workspace/mika-platform/mika-cloud/docs/solutions",
  "/data/workspace/mika-platform/mika-skills/docs/solutions",
  "/data/workspace/mika-platform/claude-pilot-py/docs/solutions",
  "/data/workspace/mika-platform/openclaw/docs/solutions",
  "/data/workspace/mika-platform/lettabot/docs/solutions",
]
```

**Precedence:** Per-agent `[kg].docs_roots` overrides the global `MIKA_KG_DOCS_ROOTS` env var, which overrides the singular `MIKA_KG_DOCS_ROOT` / `kg_docs_root` chain. If both `docs_root` (singular) and `docs_roots` (plural) are set in the same `[kg]` section, the plural form wins.

**Policy callouts:**

**a. Singular vs plural validation asymmetry:**

| Field | Missing path | Empty value |
|-------|-------------|-------------|
| `docs_root` (singular) | Hard-error if explicit; warn-and-skip if CWD default | Skips ingestion with distinct warn |
| `docs_roots` (plural) | Each path validated independently; missing paths logged as WARN and skipped; agent starts if at least one path is valid | Empty array treated as "not set" (falls back to singular chain) |

**b. Array order no longer starves secondary corpora (#962).** Budget is distributed fairly across corpora using two-pass allocation (`kg::budget::allocate_fair_budget`) for both extraction and resolution. Each corpus with pending work receives a proportional share. Array order only affects tiebreaking when the budget cannot be evenly divided — not a starvation vector.

**c. Resolution chain priority (seven tiers, first match wins):**

| Priority | Source | Field |
|----------|--------|-------|
| 1 | Per-agent identity.toml | `[kg].docs_roots` (plural) |
| 2 | Per-agent identity.toml | `[kg].docs_root` (singular) |
| 3 | Environment variable | `MIKA_KG_DOCS_ROOTS` (colon-separated) |
| 4 | Environment variable | `MIKA_KG_DOCS_ROOT` |
| 5 | Config file | `kg_docs_roots` in config.toml (plural) |
| 6 | Config file | `kg_docs_root` in config.toml |
| 7 | CWD default | `<CWD>/docs/solutions` |

## Knowledge Graph — Subject Extractor

`src/kg/subject_extractor.rs` — Per-agent LLM-based extraction of named entities and fact triples from previously-ingested documents (#690). Uses constrained NER with approved entity/relationship types and structural validation (not just prompt-based).

**Approved entity types:** `skill`, `tool`, `agent`, `problem_type`, `solution_path`, `failure_mode`, `pattern`, `concept`. **Approved relationship types:** `SOLVED_BY`, `USES`, `CALLS`, `INDICATES`, `PREVENTS`, `CAUSED_BY`, `MENTIONS` — each with from/to type constraints enforced in code.

**Sole-writer contract:** This module is the sole writer of `kg_subject_entities`, `kg_subject_relationships`, `kg_chunk_subjects`, `kg_chunk_subject_relationships`, and `kg_extractions` rows.

**Execution contexts (D2):** (1) Startup: background `tokio::spawn` per agent after lexical ingestion, non-blocking. (2) Compound hook: synchronous inline after doc write via `IngestionOrchestrator`, failure non-fatal (C2.3 log-and-skip). (3) Periodic tick (#1052): runs as Phase 1 of `resolver_tick` every 30 minutes.

**Graceful shutdown (mika#802):** The periodic tick loop accepts a `CancellationToken` (parent created in `server/mod.rs`, child tokens per agent). On SIGTERM, the parent token cancels all per-agent child tokens. The loop checks cancellation between iterations via `tokio::select!` and before starting each batch. In-flight LLM calls complete (bounded by reqwest timeout, typically <2s); remaining pending work is abandoned (not written). Half-extracted docs stay pending for the next startup. This prevents the OLD-binary-writes-under-stale-config drift documented in mika#802's incident.

**Extraction flow (D1, D4):** Read full doc from disk → insert `[CHUNK N]` markers at chunk boundaries → LLM call → parse/validate JSON output → UPSERT entities and relationships with provenance in single transaction.

**Re-extraction (D5):** Three-phase capture → reingest → reconcile. `IngestionOrchestrator` (`src/kg/ingestion_orchestrator.rs`) coordinates #689 and #690 — neither module calls the other directly. Scoped orphan sweep deletes entities/relationships that lost all provenance after doc change.

**Pending-doc detection (D7 + #757 hash check):** `kg_extractions` tracking table with `UNIQUE(agent_id, source_doc_path)` and a `source_doc_hash TEXT` column (nullable, added in v26). A doc is pending when it either has no `kg_extractions` row OR `kg_extractions.source_doc_hash != kg_chunks.source_doc_hash` — direct equality, no aggregation, because the lexical ingestor writes one identical per-doc hash across every chunk row. See `src/db/kg_schema.rs` for the full idempotency contract.

**Budget guard (#757):** `extract_pending(budget: u32)` caps per-batch LLM calls. `budget == 0` short-circuits with zero calls. On overflow, emits `kg_budget_exhausted` WARN with `scope="extraction"` and leaves remaining docs pending. Stats carry `aborted_budget: bool` + `llm_calls: u32`. Default budget: `MIKA_KG_BATCH_BUDGET` (500).

**LLM policy (C2):** Model from `MIKA_KG_EXTRACTION_MODEL` → `MIKA_KG_INGESTION_MODEL` fallback. Retry taxonomy per C2.2 (transport: 3 attempts with backoff; semantic: one retry with prompt reinforcement; config: no retry). Log-and-skip per C2.3. `llm_calls` rows per C2.4. Audit events per C3.3.

**Parse tolerance (#876):** `parse_extraction_json` tolerates reasoning prose before/after the JSON object — a common failure mode with haiku-class models (sibling of #768). Three-layer parsing: (1) strip markdown code fences, (2) direct `serde_json::from_str`, (3) `extract_first_json_object()` brace-matching fallback that locates the first balanced `{…}` in surrounding text with string-literal/escape-aware depth tracking. Schema validation stays strict — only surrounding-prose tolerance is added. When the slow path (layer 3) succeeds, emits `extraction_parse_slow_path` WARN for operator visibility. The extraction prompt also includes a JSON-only output instruction as defense-in-depth.

**Roster-grounding observability (#1158):** Three structured log events for domain-roster injection diagnostics: (1) `subject_roster_mismatch` WARN — LLM emitted a non-roster entity without `discovered=true`; entity is dropped and an error line added to the semantic-retry prompt. Fields: `agent_id`, `doc`, `entity_type`, `entity_name`, `chunk_indices`. (2) `extraction_roster_unbuilt` WARN — `kg_entities` table is empty at extraction time (domain builder has not yet populated); extraction batch skipped. Fields: `trace_id`, `agent_id`. (3) `extraction_roster_failed` ERROR — `kg_entities` has rows but zero of the roster types (skill, tool, agent, problem_type); domain builder likely failed; extraction batch skipped. Fields: `trace_id`, `agent_id`.

## Knowledge Graph — Entity Resolver

`src/kg/entity_resolver.rs` — Per-agent entity resolution that bridges subject graph entities to domain graph nodes (#691). Two-stage pipeline: exact match (case-insensitive) then LLM disambiguation for unresolved or ambiguous cases.

**Sole-writer contract:** This module is the sole writer of `kg_subject_resolutions` (subject → domain edges with confidence scores) and `kg_resolutions_log` (resolution tracking with outcome enum). No other code path writes these tables.

**Two-stage pipeline (D1):** Stage 1: case-insensitive exact match against `kg_entities.entity_key`. If match found and extraction confidence > 0.9, resolve immediately (confidence = extraction_confidence). Stage 2: LLM disambiguation with candidate list (max 50) and source chunk prose context. Combined confidence = min(extraction_confidence, llm_confidence). Discovered types (solution_path, failure_mode, pattern) skip resolution entirely — no domain counterpart exists.

**Execution contexts (D5):** (1) Startup: background `tokio::spawn` per agent after extraction tasks, non-blocking. (2) Compound hook: `IngestionOrchestrator` spawns async resolution after extraction commits. (3) Periodic tick (#906): `kg::resolver_tick::spawn_resolver_tick_task()` runs `resolve_pending(budget)` every 30 minutes per KG-enabled agent, decoupling drain rate from restart cadence. First fire skipped (startup spawn covers it). Fail-open (log-and-skip per C2.3). Structured log events: `kg_resolver_tick.start`, `kg_resolver_tick.complete`, `kg_resolver_tick.error`.

**Graceful shutdown (mika#802):** The periodic tick loop accepts a `CancellationToken` (parent created in `server/mod.rs`, child tokens per agent). On SIGTERM, the parent token cancels all per-agent child tokens. The loop checks cancellation between iterations via `tokio::select!` and before starting each batch. In-flight LLM calls complete (bounded by reqwest timeout, typically <2s); remaining pending work is abandoned (not written). Half-resolved subjects stay `pending` for the next startup. This prevents the OLD-binary-writes-under-stale-config drift documented in mika#802's incident (279-424 rows/agent written under stale `docs_root_hash` during a 3-5s deploy window).

**Pending-entity detection (D4):** `kg_resolutions_log` tracking table with `UNIQUE(agent_id, subject_entity_id)`. Pending query: subject entities with well-known types that have no log row, or whose `source_extraction_trace_id` differs from the latest `kg_chunk_subjects` extraction.

**Per-corpus fairness (#927):** `get_pending_entities(budget)` distributes the selection budget across all agent corpora via two-pass reallocation and round-robin interleaving. First pass assigns each corpus `min(pending_count, budget/N)`; second pass redistributes unused slots proportionally to hungry corpora. Per-corpus fetch limit uses 2× oversupply with a floor of 50. Results are interleaved `[A₀,B₀,C₀,A₁,B₁,...]` so no single large corpus starves smaller ones under the Stage-2 budget cap. Single-corpus agents take a fast path with no allocation overhead. `ResolutionStats.per_corpus_attempted: HashMap<String, u32>` tracks per-corpus attempt counts; emitted as JSON in the `kg_resolver_tick.complete` log event via `per_corpus_attempted` field.

**Budget guard (#757):** `resolve_pending(budget: u32)` caps per-batch **Stage-2** LLM disambiguation calls. Stage-1 exact matches cost no LLM calls and are NOT debited against the budget — even `budget=0` lets exact matches resolve (selection uses an effective minimum of 50 entities). On overflow, emits `kg_budget_exhausted` WARN with `scope="resolution"` and the remaining entities stay pending (no `kg_resolutions_log` row written). Stats carry `aborted_budget: bool` + `llm_calls: u32`. Default budget: `MIKA_KG_BATCH_BUDGET` (500).

**LLM policy (C2):** Model from `MIKA_KG_RESOLUTION_MODEL` → `MIKA_KG_INGESTION_MODEL` fallback. Mid-tier model recommended. Same C2.2 retry taxonomy as extraction. `no_match` is a first-class LLM response (not an error). `llm_calls` rows per C2.4. Per-batch audit events per C3.3.

**Failure policy:** Resolution failures are log-and-skip per C2.3. Failed entities stay pending for next startup's `resolve_pending`. No resolution model configured → exact-match-only mode with `outcome = 'skipped_no_llm'`.

## Knowledge Graph — Query Tool

`src/kg/query.rs` + `src/tools/query_knowledge_graph.rs` — Read-only graph traversal tool that lets agents discover capabilities, find solution paths, and reason about their environment (#688). Registered in `default_tools()`.

**Query modes:** Exactly one of `question` (free-text → entry path resolution) or `traversal.start` (known `entity_key` → direct traversal). `agent_id` enables agent-scoped context enrichment. `include_context` returns chunk prose text. `result_limit` caps output (max 20 entities, 30 edges, 10 chunks).

**Hybrid entry paths (D1):** Three parallel strategies find starting entities from a free-text question:
- **Path A** — Direct domain entity match: case-insensitive LIKE on `kg_entities.name`/`entity_key`. Confidence: 1.0 (exact) / 0.8 (LIKE).
- **Path B** — Subject entity match (agent-scoped): same against `kg_subject_entities`. Confidence scaled by extraction confidence.
- **Path C** — Semantic search via chunks: `hybrid_search(source_type="kg_chunk")` → `kg_chunk_subjects` → subject entities → resolve to domain via `kg_subject_resolutions` (substitutes domain entity when resolved, keeps subject when unresolved).
Results merged, deduped by `(layer, entity_id)` keeping highest confidence, top-K (5) as traversal starting points.

**Graph traversal (D2):** Recursive CTE over `kg_relationships` (domain) and `kg_subject_relationships` (subject). Delimiter-bounded cycle detection (`INSTR(',' || path || ',', ...)`). Default depth 2, cap 4. Default edge types: `SOLVED_BY`, `PROVIDES`, `DEPENDS_ON`, `USES`, `CALLS` (overridable via `follow`).

**Ranking (D3):** Lexicographic sort on `(hop ASC, cumulative_confidence DESC)`. Distance always dominates.

**Agent context (D5):** Annotate, don't filter. Skill entities enriched with `agent_context.enabled` (tri-state from `skill_overrides.enabled` via `COALESCE`). Non-skill entities have no context.

**Status values (D4):** `ok` (results found), `starting_entity_missing` (all entry paths failed), `traversal_empty` (entity found, no edges). Status enables #692 self-knowledge skill fallback logic.

## Knowledge Graph — Eval Fixture Seeding (#740)

`tests/eval/kg_fixtures/mod.rs` — crate-shared helpers for seeding a known KG state into a test `AsyncDatabase`. Used by `#740` (self-knowledge scenarios), `#741` (grounding scenarios), and `#787` (v27 migration invariant tests). Schema pin lives in the module (currently v27) with an actionable assertion message on drift. v26 fixture helpers (`V26_KG_DDL`, `V27_KG_DDL`, `DriftProfile`, `open_v26_for_coalesce()`, `build_v26_synthetic_db()`) support migration testing with realistic extraction drift simulation.

**Spec-struct pattern.** Each seed helper takes a `*Spec` struct and returns the inserted row ID: `seed_domain_entity`, `seed_domain_relationship`, `seed_subject_entity`, `seed_chunk`, `seed_chunk_subject`, `seed_resolution`, `disable_skill`. Query helpers (`get_resolution_log`, `get_resolution`) read rows back for assertions.

**FTS/vec parity.** `seed_chunk` writes to `kg_chunks` and calls `Database::index_content(agent_id, KG_CHUNK_SOURCE_TYPE, Some(chunk_id), text)` so `search_content` and `fts_search` stay in parity — Path C semantic retrieval depends on FTS5 being populated. A raw insert into `search_content` silently breaks Path C.

**Where scenarios live.** `tests/eval/kg_self_knowledge/` holds the seven #740 scenarios (one file per scenario, named `{class}_{shape}_{descriptor}.rs`). New scenarios: add the file, register it as `pub mod <name>;` in `kg_self_knowledge/mod.rs`, import `kg_fixtures::*`, call `assert_schema_version(&db).await` in setup, and update the capability-×-status matrix in the README. See `tests/eval/kg_self_knowledge/README.md` for the fixture table, tag vocabulary, and baseline scores.

## Silent Mode Agent Loop

Background tasks (heartbeat, reminders) where text output is NOT delivered. Agent must use `send_message` tool explicitly. Separate `run_silent_agent` function with `SilentPromptContext`.

**Trigger-aware skill selection:** `Heartbeat`, `Reflection`, `Reminder`, and `SkillRun` modes use `safe_always_on_skills()` which filters out exec/http-handler skills and does NOT resolve dependencies for security (autonomous triggers must not execute arbitrary commands). `Callback` mode uses `callback_safe_skills()` which preserves exec/http handlers AND resolves transitive skill dependencies via BFS (same algorithm as `match_skills()` in `matcher.rs`) — callback turns continue a tool call the agent already authorized in conversation mode, so retry/continuation workflows must have access to the same tool set (#567, #578). Loop-prevention guards in the long-running dispatch path prevent callbacks from spawning new unrelated long-running tasks.

**Task health awareness (heartbeat and callback):** `get_task_health_summary(agent_id)` detects 8 anomaly types and injects `<task-health>` block. Gated to `Heartbeat`, `Callback`, and `Reminder` triggers. Anomaly types: `stuck_callback` (completed but not delivered >10min), `failed_recurring`, `long_running` (in_progress >1h), `stale_blocked` (blocked >24h), `stale_pending` (pending >24h with no callback child — detects tasks created but never dispatched) (#583), `github_linked` (active items with GitHub PR URL), `dispatch_failures` (3+ recent `run_claude_pilot` failures in 2h window — wedged iteration loop detection, #980), `dispatch_stale` (in_progress task with no dispatch attempt in >1h — aging defense for wedges that stop retrying, #980).

## MessageSender Trait

`#[async_trait]` with `Send + Sync` bounds for `Arc<dyn MessageSender>`. Returns `Result<SendOutcome>` where `SendOutcome` is `Delivered` (gateway 2xx), `Failed { reason }` (non-2xx after retry, saved to `failed_sends`), or `NoChannel` (`chat_id == 0` sentinel — no reply channel available, e.g. GitHub webhook sessions). `Err` is reserved for infrastructure failures (chat_id resolution, DB errors). Text-only outbound. CLI prints to stdout. Server uses `GatewayMessageSender` (one retry after 2s, error classification: connection/timeout/HTTP status with body snippet). Team engine agents intentionally have `message_sender: None`. `NoopSender` (pub in `messaging.rs`) silently returns `Ok(Delivered)` — used to suppress user-facing notifications in team-child callback turns (#287) where the consolidated team-run notification handles delivery.

**`NoChannel` sentinel (#650, #1090):** `GatewayMessageSender::send()` detects `chat_id == 0` after `resolve_chat_id()` and returns `Ok(NoChannel)` before the HTTP POST — no retry, no `failed_sends` entry. `chat_id == 0` is the documented sentinel for sessions without a Telegram reply channel (GitHub webhooks, non-Telegram channels). The agent should use channel-appropriate tools (e.g., `run_gh`) instead of `send_message`. Two `error!`-level log lines are emitted on every `NoChannel` event for operator observability (#1090): Site A (`messaging.rs`) carries `agent_name`, `request_id`, and `message_text` (truncated to 500 chars via `truncate_for_log`); Site B (`send_message.rs`) carries `trace_id` and `session_id`. Both use the event name `send_message_nochannel` for grep-friendly filtering.

**Callsite handling policy:** The `send_message` tool surfaces `Failed` as `ToolOutput::error` so the LLM knows delivery failed; `NoChannel` returns `ToolOutput::success` with redirect guidance (prevents LLM retry loops). The task-engine dispatcher absorbs `Failed` and `NoChannel` with a warning (fire-and-forget for scheduled sends). Server handlers and notification paths (verdict, CI success) log warnings on `Failed` and `NoChannel` but continue. The `failed_sends` flush path increments retry count on `Failed`; deletes entries on `NoChannel` (permanent condition).

## Conversation Compaction & Rewind

**Compaction:** Threshold-based (50 messages). Keeps 20 most recent, summarizes older via Claude API. Summary injected into system prompt. `replace_with_summary` uses RAII `rusqlite::Transaction` (DEFERRED) — auto-rollback on error prevents stuck transactions that pin the WAL snapshot (#636).

**Summarizer output contract (#1024):** The compaction summarizer produces *factual state assertions*, not conversational summaries. Output bullets use one of four prefixes: `Fact:` (objective state), `Decision:` (choices and disposition), `Outcome:` (results and state transitions), `Open:` (unresolved questions). The prompt explicitly forbids first-person language, conversational verbs (discussed/agreed/decided), and process narration. This shape is per mika#1009 finding (Axis 2 — content reform): the summary block is consumed by the next session as system-prompt context, and conversational shape there causes the LLM to misread it as prior turns it participated in. The prompt is a single `const &str` at `compaction.rs:14`; tests at `compaction.rs` (`summarization_prompt_enforces_factual_shape`) assert prompt invariants.

**Rewind:** `rewind.rs` — two-phase flow: `preview_rewind()` then `execute_rewind()` with automatic reversal of memory/fact mutations via audit log. TUI: `/undo` (1 exchange), `/rewind [N | to <message_id>]`. Server: `POST /api/v1/rewind/{resolve,preview,execute}`.

## Unified Task Engine

`src/task_engine/` — single SQLite-backed scheduler. Min-heap + dedup set; 1-second tick loop; periodic DB scan (60 ticks). `TaskDispatcher` matches on `action_type`. `ensure_recurring_task()` idempotently registers heartbeat and reflection at startup. **Orphan recurring task sweep (mika#1436):** At server startup, after the filesystem walk populates `state.agents`, the engine cancels any active recurring tasks (`trigger_type='recurring'`, status in `pending|recurring_active|in_progress`) whose `agent_id` is not in the on-disk set. Cancellation preserves the audit trail (vs deletion). Companion to #1399's lazy-insert: lazy-resolved post-boot agents add their recurring tasks via `ensure_recurring_task` on demand.

**Callback/resume lifecycle:** agent creates callback task -> external process completes it -> server dispatches silent agent run with `SilentTrigger::Callback`. Loop prevention: callback turns cannot **directly** spawn new long-running tasks; the executor's `long_running_ctx == None` rejection is intercepted and re-routed through `DeferredDispatch` with `(repo, issue_number, skill)` lineage cycle detection (mika#1058). Direct spawn from callback context still hits the gate; deferred re-dispatch is the safe path.

**SilentTrigger variants:** `Heartbeat`, `Reflection`, `Callback`, `SkillRun`, `Reminder`, `PostCallbackAdvance` (#991), `DeferredDispatch` (mika#1011). Each produces correct system-prompt framing. `PostCallbackAdvance` is an engine-side structural backstop — fired by the dispatcher after a milestone/project-context callback turn completes without advancing the queue. `DeferredDispatch` is an engine-side auto-recovery for `global_dispatch_active` rejections — when `run_claude_pilot` is rejected because another dispatch is active, the engine registers a `pending` callback task with label `long_running:run_claude_pilot:deferred`. When the blocking dispatch completes, the deferred callback is promoted (status → `completed`) and dispatched on the next engine tick as a `DeferredDispatch` turn whose only required action is `run_claude_pilot` (enforced by the `deferred_dispatch_action` INTENT_GUARD). **Promotion paths (mika#1070):** (1) Inline — `dispatch_next_deferred_callback()` (`pub(crate)`) fires after `mark_task_delivered` on a non-deferred callback. mika#1124 re-added the inline anti-cascade guard at `dispatcher.rs:495`: when a `:deferred` wrapper itself completes (e.g., the silent turn no-ops), inline chain-promotion is SKIPPED — relying on the periodic backstop instead. This prevents the no-op-cascade failure mode where N wrappers drain the queue inline without ever dispatching. Real (non-deferred) callback completions still chain-promote immediately. (2) Periodic backstop — `promote_pending_deferred_if_idle()` runs every `DB_SCAN_INTERVAL_TICKS` (60 ticks), iterates over the `dispatch_class` values (`DISPATCH_CLASSES = &["implement", "groom"]` — pinned to `derive_dispatch_class` at `skills/executor.rs` via a shape test in `engine.rs`), checks `has_any_active_callback_for_class(class)` (excludes deferred wrappers via `label NOT LIKE '%:deferred'`, scopes by `COALESCE(dispatch_class, 'implement') = ?`), and promotes one wrapper per idle class per tick via `dispatch_next_deferred_callback_for_class(class)` (mika#1175). Per-class iteration prevents cross-class throughput halving when wrappers from multiple classes are co-pending. Placed BEFORE `dispatch_undelivered_callbacks` for same-tick dispatch. Fail-closed per-class on DB errors — one class's check error does not stall the other. The agent-wide siblings `has_any_active_callback()` and `dispatch_next_deferred_callback()` are kept for the inline-promotion path in `handle_task_complete` (out-of-scope for #1175, see plan § Open question 1). **Both slot predicates must exclude `:deferred` wrappers (mika#1163).** `has_any_active_callback`/`has_any_active_callback_for_class` (engine backstop) AND `has_active_callback_tasks_excluding` (per-class gate inside `validate_dispatch_readiness`) all apply `label NOT LIKE '%:deferred'`. The earlier asymmetric version caused a multi-wrapper deadlock: when two parents each held a pending wrapper, every dispatch attempt from one wrapper saw the OTHER wrapper as slot-occupied and registered yet another wrapper, with no real dispatch ever spawning. **AgentBusy recovery (mika#1070):** when `dispatch_resume_agent` returns `AgentBusy` in `handle_task_complete`, the callback keeps `completed` status (not reset to `pending`) with `next_fire_at` set to now+30s for retry delay. `dispatch_undelivered_callbacks` has a `next_fire_at` guard that skips tasks whose retry delay has not expired. γ composition: the LLM's `send_message` notification and the engine's deferred callback are independent; `validate_dispatch_readiness()` arbitrates any race. Per-agent cap of 10 pending deferred callbacks prevents flood. `cancel_task()` cascades to callback children (both immediate and deferred). **No-op wrapper detection (mika#1172 R9):** When a `:deferred` wrapper completes, the dispatcher checks `has_non_deferred_active_callback_child(parent_task_id)`. If no active child exists, emits `deferred_dispatch_noop_completion` WARN + audit event — the silent turn failed to spawn a real dispatch (mika#1124 regression signal). **Dispatch lifecycle audit events (mika#1172 W4):** Three events written to `audit_events`: `deferred_dispatch_promoted` (on each inline or periodic backstop promotion, with promoted task ID), `deferred_dispatch_registered` (on deferred callback registration at `global_dispatch_active` rejection), `deferred_dispatch_noop_completion` (on no-op wrapper detection). Promote methods (`promote_next_deferred_callback`, `promote_next_deferred_callback_for_class`) return `Option<String>` (promoted task ID) instead of `bool` to enable meaningful audit event resource_id. **(3) Force-promote (mika#1453)** — `promote_deferred_callback` agent tool (fail-closed: rejects when slot busy, no override) and `mika tasks promote-deferred <class>` CLI verb (with `--override` for cancel-then-promote). Both call `force_promote_deferred_for_class()` which shares the `has_any_active_callback_for_class()` predicate (mika#1163 parity). `find_active_callback_for_class()` identifies the blocker for the CLI override path. Three audit event types: `deferred_dispatch_force_promote_succeeded`, `deferred_dispatch_force_promote_rejected_slot_busy`, `deferred_dispatch_force_promote_override`. **What the stuck-pending reaper makes of a promoted wrapper (mika#2181).** Promotion writes `status = 'completed'`; the silent turn that consumes the wrapper only reaches `delivered` when it *returns*, minutes later under a slow model. For the reaper's predicate (`find_orphaned_pending_issue_tasks`) a deferred wrapper therefore counts as **live** when it is `pending`, **or** `completed` with a `completed_at` newer than `now - MIKA_PROMOTED_WRAPPER_LIVENESS_SECS` (default 2700s, `PROMOTED_WRAPPER_LIVENESS_DEFAULT_SECS`). The bound is load-bearing, not a rounding: on the silent-turn error path the wrapper is re-armed but never marked `delivered`, so it stays `completed` forever, and an unbounded predicate would turn that corpse into a permanent shield against repair. `delivered`, `failed` and `cancelled` are never live. `has_live_deferred_wrapper_child` (renamed from `has_pending_deferred_wrapper_child`) answers the same question with the same predicate — the two must not diverge. The `mika tasks stuck` probe takes the same two windows as the reaper, so probe and engine report on one population.

#### The direct measure that succeeds the proxy window (mika#2184)

**What mika#2181 could not reach, measured.** Its window is a proxy on the
*promotion* instant, and the residue is in the code's own doc-comment: over 30
days, **139 of 799** delivered wrappers (17 %) delivered past 2700 s, **81** of
their parents were expired `stuck_pending_no_deferred_wrapper`, and **8** of
those were expired **2820–4996 s** after promotion — out of reach of *any* value
of the constant compatible with a useful reaper. Widening it is the wrong reflex,
and the ticket's argument for that is the one worth keeping: a wrapper delayed by
a restart is a **healthy** wrapper, so a bigger number buys coverage by blinding
the reaper for longer. *A window on a proxy is a dated debt*
(`docs/solutions/best-practices/une-fenetre-bornee-sur-un-proxy-est-une-dette-datee-2026-09-05.md`).

**The chain is joined by equality, and it already existed.** The turn consuming a
promoted wrapper is a `SilentTrigger::DeferredDispatch`, and
`dispatch_resume_agent` opens its session with
`create_session_with_parent(…, task_id = Some(&task.id))` where `task.id` is **the
wrapper**. So `parent → wrappers (label = DEFERRED_DISPATCH_LABEL) → sessions
(sessions.task_id = wrapper.id) → llm_calls / tool_calls`. Stricter than mika#1652,
which has to fall back on `session_id LIKE 'team-' || r.id || '%'`. The `task_id`
column has existed since v19: no migration, no column.

**Three causes of delay, and they do NOT share a measure — this is the ticket's
own premise, corrected.** The ticket writes that the silent turn "produces those
same rows". True *when the turn runs*. A wrapper `completed` and not `delivered`
past 2700 s has three causes: **(A)** the turn runs and is slow — the AC1 case,
and the only one an activity measure covers; **(B)** `AgentBusy` — the agent lock
is held elsewhere and `dispatch_resume_agent` returns `Err` **before**
`create_session_with_parent`, so there is no session and nothing to measure;
**(C)** a service restart — nothing was running, and neither was the reaper.
This work closes **A** by measurement and **C** by an admission (see
`NotYetObservable` below); **B is not covered, and the refusal is reasoned** —
the only available signal would be "the agent has activity somewhere else", which
is true almost permanently on mika-dev and would disarm the reaper under cover of
precision. Follow-up named.

**Filtered in the application, never as a third `NOT EXISTS`.** The reflex would
be one more clause in `find_orphaned_pending_issue_tasks`. Refused, and the
refusal is written in the doc mika#2181 itself shipped: *"the shelter lives in a
SQL `NOT EXISTS`: a sheltered parent never becomes a candidate and never traverses
the application. No log, no audit, no counter […] indistinguishable from a healthy
regime."* That is the defect mika#2181 had to repair after the fact with
`find_parents_sheltered_by_promoted_wrapper`; doing it a second time in the same
function would re-dig the hole just filled. The SQL yields the candidates (proxy
window included, unchanged) and the direct measure filters afterwards, where it
can log.

**The DB returns an AGE, not a boolean.** `find_deferred_wrapper_activity_age_secs`
— `None` when no row exists, and `None` is never `0` (mika#2331). Three reasons in
order of weight: the threshold leaves the SQL and becomes a parameter of a pure
function, testable at its boundaries without a database; the log line can then
*name* the age, which mika#2277 paid dearly for not doing (two false positives read
"nominal" on first inspection because one age was reported on a disposition
crossing three); and a negative age (clock skew) clamps to `0` — "very recent",
never "very old", fail-safe towards sparing.

**Four states, and the disposition follows BOUNDEDNESS, not certainty.**
`classify_wrapper_activity(last_activity_age_secs, window_secs, engine_uptime_secs,
telemetry_armed)` is a pure function reading no global state; the disposition is an
exhaustive `match` with **no `_ =>` arm** (the `hosting_ground_truth_line` pattern,
mika#2290).

| state | disposition | why |
|---|---|---|
| `Active` | **spare** | the turn is demonstrably working |
| `NotYetObservable` | **spare** | the ignorance is **bounded**: it extinguishes itself as soon as uptime exceeds the window. Covers cause C |
| `Silent` | reap | today's behaviour, bit for bit |
| `NotRecorded` | reap + WARN | the ignorance is **permanent**: sparing here would restore the corpse-shield mika#2181 had to bound |

*What cannot extinguish itself cannot spare.* `NotYetObservable` and `NotRecorded`
are two variants rather than one `Unobservable { reason }` precisely because their
disposition differs — **a reason that decides is not a reason, it is a state**.
`telemetry_armed = store_llm_calls || store_tool_calls` is a setting that is
**read**, never inferred from an absence of rows: telling "telemetry is off" from
"the agent did nothing" is impossible by observation, which is the confusion
mika#2277 condemns.

**`MIKA_STUCK_PENDING_ACTIVITY_WINDOW_SECS`**, default **600 s** = 2× the default
turn envelope (`AGENT_TOTAL_TIMEOUT` 300 s, mika#2189), so it covers a whole turn
*and* the interval to the next call with a factor of 2 of margin. Deliberately
twice mika#1652's 300 s for team runs: the expensive error here is killing a live
turn. House three-tier parse, plus a 30-day clamp — **not** the anti-`strftime`
mechanism of `PROMOTED_WRAPPER_LIVENESS_MAX_SECS` (this threshold never enters the
SQL), but the coherence of the knob: an absurd setting would spare every parent for
ever, which is what `NotRecorded` already refuses on the other axis.

**Two event names, and that rectifies AC4's letter while holding its intent.** AC4
asks that `stuck_pending_sheltered_by_promoted_wrapper` "distinguish the two spare
causes"; the literal reading is a `cause` field on that event. **Refused:** its name
*carries* its cause, so routing an unrelated spare through it would make the name
false and split in two the population mika#2181's probe counts to measure whether
its debt is retiring. The house has an established way to keep two populations
countable apart and has used it three times — `phantom_aged_out` /
`phantom_sweep_spared` (mika#2156), `qa_deadline_verdict` / `qa_callback_verdict`
(mika#2368), `auto_pull_no_token` / `wip_rescue_no_token` (mika#2205). So
`stuck_pending_sheltered_by_promoted_wrapper` is **untouched** and
`stuck_pending_sheltered_by_activity` is its sibling, each SOLE WRITER of its own
name, pinned by `mika2184_the_two_spare_causes_have_one_writer_each`.

**One reader, held by a source scan.** `mika2184_wrapper_activity_has_a_single_reader`
refuses a second production site joining `tasks → sessions → llm_calls/tool_calls`,
**allowlist shipped empty** — when it fires, remove the second site, do not exempt
it. That is the `grooming_marker` lesson (mika#2158), which this very file has
already paid a second time with `has_pending_deferred_wrapper_child`. The needle is
a conjunction of three terms, each added by a measured false positive: it must not
accuse `find_stuck_team_runs` (whose join is a `LIKE` on `session_id`) nor Signal A
of `get_task_health_summary` (which walks the same three tables in the opposite
direction). **Deliberately not unified with mika#1652:** disjoint populations,
different joins — an abstraction drawn over two points whose joins differ is the
wrong abstraction. What is shared is the *pattern*, not the code.

**What a fresh process means for tests.** A newly constructed `TaskEngine` has zero
uptime, so `classify_wrapper_activity` answers `NotYetObservable` and the reaper
spares **everything** — correctly. Every reaper test that expects an action
therefore declares that precondition through the `observing_engine` helper, rather
than inheriting it from `Instant::now()` happening to be old enough.

**The phantom-sweep sibling (comment 1 of mika#2184) is a SEPARATE fix, and the
reason is structural.** A NULL-PID `action_type='none'` tracking row queued behind
a busy slot produces **no** `llm_calls` and **no** `tool_calls`: it has not started.
Measuring activity would spare it exactly zero times. Its discriminant is already
named, in writing, in `dispatch_liveness`'s own doc-comment — *"a tracking row still
waiting for a dispatch slot has no PID-carrying child at all — only a deferred
wrapper — so this guard cannot see it"* — and the remedy there is to consult the
deferred wrapper (`has_live_deferred_wrapper_child`, a predicate that already exists
and has nothing to do with activity). Different population, different discriminant,
different blast radius. **Precondition before opening it:** establish that the class
still recurs — the measured defect dates from 2026-09-07 and mika#2156 has since
raised the grace to 14400 s.

Operator surfaces, the five probes and their halts: root `CLAUDE.md`
§ *Optional (mesure directe de vivacité du tour différé — mika#2184)*.

#### A represented parent has nothing to repair (mika#2413)

**The failure, measured 2026-09-19.** Three tickets were un-parked at once
(#2237, #1952, #2025) and the three grooms serialised on the single `groom`
slot. The third one's parent (`e7c4e9ad`) produced four `expired` wrappers in
twenty minutes and went **`failed` at 19:19:45Z**. The loop self-healed later —
`stuck_ready_reconcile` re-dispatched and #2025 groomed — but at the cost of a
whole cycle and a transient `failed` visible on every operator surface.

**There is no TTL, and nothing expired by the clock.** Both wrapper creation
sites (`register_deferred_callback`, `rearm_deferred_callback`) build their
`NewTask` with `timeout_at: None`, and `expired` is mika#2169's *terminal
vocabulary* (`mark_deferred_wrapper_noop`: the turn happened and dispatched
nothing), not a deadline. The bound actually crossed was `MAX_STUCK_REARMS = 2`
— a **repair budget**, and the parent died in
`rearm_consumed_deferred_wrapper`'s `Unrepairable` arm, in the same second as
the third consumption. That simultaneity is the signature; no delay produces it.

**The self-sustaining loop, which is the real finding.** Contention explains only
the *first* re-arm; what condemned the parent is that the re-arm manufactured the
condition making the next turn sterile. Three individually-correct mechanisms
compose: (1) the re-arm posts a `pending` wrapper so the parent stays represented
(mika#2045), on top of the one `register_deferred_callback` already posted on the
`global_dispatch_active` refusal; (2) `execute_long_running`'s mika#1205
`already_deferred` intercept short-circuits on any `pending` wrapper **before**
`validate_dispatch_readiness`, so the next turn tests nothing — including when the
slot has since freed; (3) R9 (`has_non_deferred_active_callback_child`, whose SQL
carries `label NOT LIKE '%:deferred'`) cannot see that fresh wrapper, so the turn
that correctly re-queued is counted a no-op and re-armed again. Two confusions
compose: **R9 confuses "did nothing" with "re-queued correctly"**, and **the budget
confuses "will never dispatch" with "is waiting its turn"**.

**The fix is a predicate on the parent's state, never a list of causes.** A cause
list is an implicit wire format that drifts — a fifth cause would inherit the
wrong default — and the cause is not the right question anyway. The question the
re-arm exists to ask is *"does anything still represent this parent in the
queue?"*, and it already had an exact answer: `has_live_deferred_wrapper_child`
(mika#2181). So `rearm_deferred_callback` returns the new
`RearmOutcome::AlreadyRepresented` — **no wrapper created, no counter touched** —
when the parent carries a live wrapper. That single predicate closes both
confusions: no surplus wrapper, so the mika#1205 intercept stops short-circuiting
every turn, and a turn that finds the slot free dispatches for real.

**Two details that look incidental and are not.** (a) The **consumed wrapper is
excluded by id** (`find_live_deferred_wrapper_child(..., exclude_task_id)`): on the
`silent_turn_error` path it is still `completed` with a fresh `completed_at`, so
without the exclusion it would count itself as live and mika#2045's repair would
be dead on that path for the whole liveness window. (b) The guard is **fail-safe
towards repairing** — the opposite of the `has_non_deferred_active_callback_child`
guard immediately above it, which fails closed. An unreadable read must not be
read as "the parent is represented": that would leave a parent with nothing in the
queue and nothing to put anything back, never repaired and never expired. A wrong
"I repair" costs one point of a budget of two; a wrong "it is represented" costs
the parent.

**The guard lives in the function, and the enum makes the reapers say so.** Four
call sites reach the re-arm chain (`rearm_consumed_deferred_wrapper` ×2, the
stuck-pending reaper, the L3b stale-blocked sweep); putting the guard at the
callers would fork it four ways — the class `grooming_marker` (mika#2158) had to
close once here. The new variant is a fourth `RearmOutcome` rather than a third
use of `NotNow` for two reasons: only this path owes the consumed wrapper an
honest terminal record (a `NotNow` from `has_non_deferred_active_callback_child`
means the turn genuinely dispatched, and writing `expired` there would deny a real
dispatch), and both reapers treat any non-`Rearmed`/non-`NotNow` outcome as grounds
to **expire the parent** — so the stuck-pending reaper's two `if`s became a `match`,
and a variant added and forgotten there now fails to compile instead of destroying
tasks.

**U2 — the consumed wrapper still gets its record.** `AlreadyRepresented` writes
`mark_deferred_wrapper_noop` with a reason naming *parent déjà représenté*. Not
tidiness: on the `silent_turn_error` path the wrapper would otherwise stay
`completed`, which is exactly the status `count_promoted_undelivered_wrappers`
(L2b) reads as "promoted, never taken" — so the starvation indicator would fire
on a healthy regime, and an indicator that fires in the nominal regime is an
indicator that gets muted.

**A counter the success never clears ends up bounding something else.**
`increment_stuck_rearm_count` was the only writer, so the counter was monotone for
the row's whole life — and a groom parent **becomes** the implementation parent
(mika#1614 task reuse, `update_task_dispatch_class` flips `groom` → `implement` on
the same row). Two contentions suffered while grooming therefore condemned the
implementation before it began. `reset_stuck_rearm_count` clears it in
`execute_long_running`, **after** `spawn_long_running_exec`. **The discriminant is
"having reached a real dispatch", never "having started one"** — that is what
separates it from the reset mika#2158 had to remove, which fired on
`in_flight_self_dev` (the very action the counter counted) and made the counter
unreachable, 31 re-drives reading 1. Moving this call earlier is the regression,
and `mika2413_a_refused_dispatch_does_not_reset_the_repair_budget` is what
reddens if anyone does.

**What did NOT change, and the PR says so in answer to the 20/09 operator note:**
no dispatch policy. The `groom` cap stays 1 (`max_concurrent_for_class`, `_ => 1`),
the arch-seat serialisation is untouched, no TTL is introduced, no retry is added
(AC4 — this work *removes* undue budget spends and one surplus wrapper),
`MAX_STUCK_REARMS` stays 2, `MAX_PENDING_DEFERRED_CALLBACKS` stays 10, and no
environment variable is created.

**Operator surfaces.** `deferred_rearm_skipped_parent_represented` (INFO — fields
`parent_task_id`, `task_id` (the consumed wrapper), `live_wrapper_id`, `cause`,
`dispatch_class`). **Expected regime: non-empty under multi-un-parking, silent
outside contention** — it is the direct measure that the guard bites, and without
it a suppressed re-arm would read exactly like a re-arm that never had a reason to
happen (class mika#2205). Deliberately INFO-only, no `audit_events` row: the
population is one or two lines per parent per contention episode, and a durable row
per tick would be the churn mika#2131 bounds. `stuck_rearm_count_reset` (INFO) — a
budget cleared without a trace is the thing nobody can audit afterwards. The number
that says whether the fix took is the distribution of exhaustion causes:

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'deferred_dispatch_unrepairable_parent_failed' GROUP BY 1;
```

**Post-deploy probe, and its three halts.** At the next real un-parking of N≥3,
`deferred_rearm_skipped_parent_represented` must be non-empty and
`deferred_dispatch_unrepairable_parent_failed` empty for the `groom` class.
*Halt 1* — the second is non-empty with `cause = "noop_completion"` while the
first is empty: the guard is not biting. **Do not raise `MAX_STUCK_REARMS`**;
check the liveness window first. *Halt 2* — the first is non-empty and the parent
dies anyway: a fifth path is spending the budget. Read the `cause` and establish
the site before touching the predicate. *Halt 3* —
`deferred_dispatch_promotion_starved` (L2b) starts firing on a healthy regime: the
terminal record of U2 is not landing; post the record rather than mute the
indicator.

**Dispatcher-source arbitration (mika#1948, Porte 2).** Three dispatchers share the per-class exec slots: the **autonomous loop** (`mika_dev` — ready-label, wip-rescue mika#1852, auto-feeder mika#1863, auto-pull mika#1824), the **milestone manager** (`mika_manager`, gated behind Phase 2 promotion), and the **operator** (`operator` — interactive `/mika`, `/mika-spawn`, `mika tasks promote-deferred`). `tasks.dispatcher_source` (v51, nullable, CHECK-pinned to those three values) records which one initiated a task. NULL means a pre-v51 row and reads as `mika_dev` via `COALESCE` — the autonomous loop was the only dispatcher before the column existed. Write it with `set_task_dispatcher_source()`, a deliberate separate write rather than a `NewTask` field: `NewTask` has ~175 construction sites and all but a handful are the loop, whose correct value is exactly the NULL default.

**This is NOT the `dispatch:*` seat label (mika#2084).** The seat (`webhook_dispatch::CURRENT_DISPATCH_SEAT`, values `loop|ssc|mpc`) says which *engine* owns a TICKET and is carried on the GitHub issue; the seat gate in `validate_dispatch_readiness` refuses a ticket belonging to another engine. `dispatcher_source` says which *role inside one engine* asked for the work, and arbitrates the exec slot among them. Neither subsumes the other; they are deliberately not merged into one column.

**The seat vocabulary is written twice and guarded (mika#2092).** `KNOWN_DISPATCH_SEATS` (`webhook_dispatch.rs`) and the `dispatch:*` entries of `.github/labels.yml` are one list in two files, and they must move in the same commit. An undeclared seat is not merely undocumented: label-sync runs with `delete-other-labels: true`, so it is a label GitHub deletes from the repo and from every issue carrying it, without an `unlabeled` event — after which `classify_dispatch_seat` reads `NoSeatLabel` everywhere and the gate refuses nothing, silently. That happened on 2026-08-30 at 09:12:51Z, an hour before mika#2084 shipped, and it is why `dispatch:loop` — the label making `SeatVerdict::OwnedByCurrentSeat` reachable at all — did not exist until mika#2092. `scripts/check-dispatch-seats-declared.sh` now compares the two lists **both ways** (an orphan label resolves to `Unresolvable` and refuses the ticket, which is the mirror failure) and fails CI via the `dispatch-seats-lint` job; `scripts/test-check-dispatch-seats-declared.sh` pins its negative behaviour. `dispatch:zorglub`, the unknown-seat test fixture below, must stay undeclared.

**The exec slot is CLAIMED, not checked.** `has_active_callback_tasks_excluding` is a bare SELECT: on `None` the dispatch path proceeds, and the callback row that makes the slot observably held is written much later by the caller. In between, `validate_dispatch_readiness` performs several GitHub round-trips (issue body, open-PR, grooming markers — 10s timeout each), and four production callers enter it (`ready_label_handler`, the tool boundary, `task_engine::dispatcher`, `verdict_handler`). Two dispatchers could therefore both read "free" and both proceed — the 2026-08-30 shape, where a second writer landed on a branch SSC already had a PR open on. A slot two claimants can simultaneously believe they hold is not arbitration, it is a convention. `dispatch_slot_leases` (PRIMARY KEY `(agent_id, dispatch_class)`) makes the claim a fact: `try_acquire_dispatch_slot()` runs one `INSERT … ON CONFLICT … WHERE expired OR same-holder` inside an IMMEDIATE transaction, so the second claimant's INSERT collides rather than races. The claim is the **LAST** gate in `validate_dispatch_readiness`, on purpose — no fallible step follows it, which is why no error path needs to release a lease. Refusal is `dispatch_slot_contended`, registers a deferred wrapper like any other rejection, and writes an audit event naming the holder. The lease TTL (`DISPATCH_SLOT_LEASE_TTL_SECS`, 120s, override `MIKA_DISPATCH_SLOT_LEASE_TTL_SECS`) is what keeps fail-closed from becoming loop-breaking: it must exceed the window it guards (validation done → callback row exists, i.e. a process spawn) and stay far below a real dispatch's duration, so a dispatcher that dies mid-claim stalls its class for one TTL rather than forever.

**Operator priority.** The three dispatchers are not peers: `operator > mika_manager > mika_dev`. `promote_pending_deferred_if_idle()` consults `has_pending_operator_task_for_class()` and stands down when the operator has `pending` work in that class, so an automatic wrapper promotion cannot take the slot out from under work the operator already drafted (they cannot see the queue and would simply find their dispatch refused). This check is fail-OPEN, unlike the slot-occupancy check above it — a busy slot is information we must have, whereas priority is a preference, and fail-closing here would strand deferred wrappers and cost the loop its recovery path. `force_promote_deferred_for_class` (mika#1453) is unaffected: it is the operator's own escape hatch, not the automatic path.

**Cycle detection on the source axis.** `check_lineage_cycle` keeps its exact-tuple rule `(repo, issue, skill)` unchanged and first, then adds a narrow second rule: a `mika_manager` dispatch is refused when an ancestor in its lineage is also `mika_manager` on the SAME `(repo, issue)`, regardless of skill. That is the deadlock shape Porte 2 exists to prevent — a manager queueing behind work it started itself — and the exact-tuple rule misses it because the skill differs (`dev-pilot → callback → dev-groom`). It fires only when BOTH sides are explicitly `mika_manager`, so NULL on either side is a no-op and `mika_dev`/`operator` lineages are untouched; widening it to "any matching source" would refuse the ordinary `dev-pilot → dev-groom` chain the loop runs constantly.

**Contention reporting.** The `global_dispatch_active` rejection carries `blocking_dispatcher_source`, and `Assessment.contention_events` (`#[serde(default)]`, rendered by `Reporter` as a `### Dispatch contention` section only when non-empty) gives the manager a place to report deferrals. A pre-v51 NULL passes through as JSON `null` and renders as `unknown (pre-v51 row)` — never defaulted to `mika_dev`, so a reader can tell "we don't know" from "we observed". Phase 1 never populates this: `no_dispatch_test.rs` FORBIDDEN_TOKENS blocks any file under `milestone_manager/**` from writing `dispatcher_source = 'mika_manager'` (both SQL and Rust literal forms). Phase 2 moves that write to a dispatch-wire file outside the subtree and updates the token list in the same commit.

**Engine-level callback metadata extraction (#376):** `try_extract_callback_metadata()` parses structured fields from callback results and persists to parent task. `extract_callback_fields()` extracts `session_id`, `turns`, `cost_usd`, `duration_ms`, and `pr_url` from claude-pilot output into the `claude_pilot` metadata object. The `pr_url` field (#871 R4) is parsed from `^PR:\s+<url>` lines emitted by `skills/bundled/_shared/dispatch-lib.sh` (shared handler for dev-pilot and dev-groom, #893); the reaper (below) keys off its presence/absence.

**Callback process liveness watchdog (#959):** `check_callback_process_liveness()` runs every 60-tick cycle and detects when a long-running subprocess (e.g., `run_claude_pilot`) has crashed without delivering its callback result. Detection: queries `in_progress` callback tasks with `process_id IS NOT NULL`, checks PID liveness via `kill(pid, 0)` + `/proc/<pid>/stat` field 22 (process start time) comparison to guard against PID reuse. On first detection of a dead process, records `first_dead_at` in task metadata; after `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS` (default 120s) elapses, re-checks task status (race guard) then marks the task `failed` with `error_reason = "subprocess_exited_without_delivery"`. Process start time is stored in callback task metadata at spawn time by `spawn_long_running_exec()`. The watchdog detects death in ~60s (one tick) + 120s grace = ~3 minutes total, vs the previous 6-hour `timeout_at` fallback. Platform: Linux only (`/proc` filesystem). The existing `timeout_at` mechanism serves as a panic-fallback for edge cases where PID tracking fails.

**Pilot silent-stall reaper (mika#2249, predicate corrected in mika#2277):** `reap_silently_stalled_pilots()` runs every 60-tick cycle **immediately after** `check_callback_process_liveness`, and the two are exact mirrors: the watchdog owns the dispatch whose process is **dead**, this one owns the dispatch whose process is **alive** and has gone silent on **every** observable surface. Nothing owned that second population before — every other reaper fires on task state — which is why `fb355061` sat `in_progress` for 2 h 18 with a live PID, an empty `result` and no terminal marker. The two select disjoint sets by construction, so the ordering costs nothing.

**Why it is in the engine and not in claude-pilot.** The founding measurement is that both mika#2246 pilots carried a *working* internal watchdog (`toolWaitCeiling=1800s modelWaitCeiling=900s` in all three logs — fields that only exist post-cpp#145) and neither fired. A watchdog starved inside the pilot's own event loop cannot fire on a pilot-side timer either, whatever that timer measures; any detector housed in claude-pilot inherits the failure it exists to see. Hardening that watchdog is `senara-solutions/claude-pilot#168`, a companion, not a substitute.

**Predicate (AC4), a conjunction whose every term has its own negative control:** (1) the population from `get_live_dispatch_callback_tasks_with_pid` (`trigger_type='callback'`, `status IN ('pending','in_progress')`, `process_id NOT NULL`) — **`pending` is where a live dispatch actually sits** (mika#2272; the callback child is created without a status, so `create_task` writes `pending`, and #525 transitions the *parent*). Scanning `in_progress` alone selected an **empty** set: 897 rows have carried a `process_id` in production and not one was ever `in_progress`, which is why mika#2261 shipped and never fired. `in_progress` is kept in the SELECT because nothing pins the state machine to this shape. The #959 watchdog deliberately keeps the narrower query — widening *that* population changes who marks a task `failed` when a process dies, which races the spawn monitor; (2) the process is **alive** via `is_same_process_alive` — a PID with no stored `process_start_time` is **declined**, because the pair that identifies a process *instance* is incomplete and a recycled PID would read as alive; (3) the **transcript** declared on the row (`PILOT_TRANSCRIPT_EXPECTED_KEY`) has not been appended to for longer than the window; (4) the **claude-pilot session log**, derived as `<pilot_log_dir>/<task-id>.log`, has not been written for longer than the window; (5) a worktree was declared (`DISPATCH_WORKTREE_FILE_KEY` → a file `dispatch-lib.sh` wrote), the declaration is readable and non-empty, the path exists, yields an mtime, and that mtime is older than `MIKA_PILOT_STALL_REAP_AGE_SECONDS`; (6) the task is **still** on one of those two live statuses on re-read, so an in-flight callback — which moves the row to `completed`/`delivered` — wins the race cleanly.

**Why three surfaces and not one (mika#2277) — the worktree-only predicate was measurably false.** On 2026-09-10 at 07:16:10Z, the first armed deploy of mika#2272 killed the only two pilots in flight, both alive and productive: their claude-pilot logs and transcripts had been written 3 min 40 s and 3 min 31 s earlier, while their worktrees had not moved since `checkout` (2758 s and 2744 s, past the window). A pilot in its reading / exploration / planning phase touches no worktree file while the SDK keeps yielding turns. **The negative control that justified the window is refuted with it**: the 24-minute inter-write gap of the healthy run `c3f9a2f9` was never the upper bound of a healthy worktree gap — the bound is the length of a reading phase, which has no measured ceiling, so *no* window is safe for a worktree-only predicate. The **transcript** does separate: `~/.mika/data/pilot-transcripts/<task-id>.jsonl` (mika#1705) timestamps every LLM turn, and on the two false positives the largest inter-turn gap was **387 s and 376 s** — a factor of 7 under the window — while being the *definition* of the class D1 targets (mika#1901: the SDK stream goes quiet) rather than a proxy for it.

**The fail-safe rule, one sentence, three terms:** *a signal that cannot be read is never a satisfied term.* Terms 3, 4 and 5 all obey it. `worktree_activity::seconds_since_last_write` and its mika#2277 sibling `seconds_since_file_write` both return `None` for an absent path, an unreadable one, no mtime, or an mtime in the future (clock skew) — plus, for the file probe, anything that is not a regular file — and every `None`, like a missing metadata key, is *not a candidate*. **That rule has a cost and AC4 keeps it from being paid in silence**: disabling `MIKA_LOG_PILOT_TRANSCRIPTS` would otherwise disarm the reaper fleet-wide with nothing said. The first time a dispatch drops out for an *unavailable* signal, the reaper emits `pilot_stall_signal_unavailable` (WARN, field `missing_surfaces`) and stamps `metadata.pilot_stall_signal_unavailable_reported` so it says it **once per dispatch** — the same motif as mika#2040's `PILOT_TRANSCRIPT_REPORTED_KEY`. A dispatch dropped because a surface is **active** is nominal and stays silent. Disposition of that warning is emit-and-continue: a safety mechanism that halted because it could not measure would be a worse defect than the one it reports.

**Evaluation order is cost, not semantics.** The two single-file `stat`s run before the bounded worktree walk and an active one short-circuits it, so a live pilot no longer pays for a directory walk on every scan. The conjunction is unchanged — a surface found active ends the question either way.

**The pilot-log path is the one derived path, and that is safe here specifically.** `dispatch-lib.sh` sets `LOG_ID="$TASK_ID"` and launches `claude-pilot --log-dir "$_PILOT_LOG_DIR" --task-id "$LOG_ID"`, so the file is `<pilot_log_dir>/<callback-task-id>.log` — verified empirically against the two mika#2277 false positives. The two halves read **different** environment variables (`PILOT_LOG_DIR` in the shell, `MIKA_PILOT_LOG_DIR` in the engine, because `scrub_mika_env_vars` strips every `MIKA_*` from the dispatch child), so their **defaults** are the only place they agree without an operator — both `/var/log/claude-pilot`, pinned by `config::tests::pilot_log_dir_defaults_to_the_dispatch_lib_path`. A disagreement makes the file absent, which is `Unavailable`, which takes the dispatch out of the population: a wrong derivation can only buy inertia, never a false positive. That is why deriving is acceptable here and not for the worktree.

**The worktree path is declared, never derived.** `dispatch-lib.sh` is the sole holder (it calls `scripts/derive-worktree-path`); re-deriving it in Rust is the duplication mika-platform#58 closed, and `db.rs` states in prose why `worktree_claims` is keyed `(repo, issue_number)` with no path column. `skills/executor.rs::inject_dispatch_worktree_env` injects `MIKA_DISPATCH_WORKTREE_FILE` after the env sandbox (like `GH_TOKEN`) and stamps the path on the task once the spawn succeeded — the same trajectory and the same fire-and-forget discipline as mika#2040's transcript stamp, for the same reason: an unstamped dispatch must be invisible to the detector, never a blocked dispatch. The channel does not cross bubblewrap — bwrap wraps only the claude-pilot invocation, and the `git worktree add` runs outside it.

**Detection is unconditional; the disposition is armed (mika#2272).** `MIKA_PILOT_STALL_REAP_ENABLED` defaults to `true` since mika#2272: the reaper signals the process and transitions the task. mika#2249 landed it `false` behind a flip condition — three reviewed `pilot_silent_stall` rows with no false positive — which turned out to be **unsatisfiable rather than unmet**: it counted rows written by a detector whose population was empty, so the count could only ever stay at zero. Two live pilots went ~50 min silent on 2026-09-09 and produced no row at all.

The asymmetry mika#2249 named is real and unchanged — a false negative costs a dispatch slot, a false positive destroys hours of decision-core work — and it is now paid for by the conjunction itself: the fail-safe keeps any dispatch without a readable worktree, transcript **and** pilot log out of the population entirely, and `MIKA_PILOT_STALL_REAP_ENABLED=0` still restores observation-only without a rebuild. What mika#2272 added and #2249 never had is a **positive control on a real process** — `tests/eval/test_reaper_reaps_live_pending_pilot_2272.rs`, an authentically live silent pilot seeded through the production write path, found by the scan and killed by the disposition. **What mika#2277 added is the missing negative half**: `tests/eval/test_reaper_liveness_all_surfaces_2277.rs` carries the positive control plus **four** single-term negatives — N1 the exact incident shape (transcript and log fresh, worktree past the window), N2 transcript-fresh-only, N3 log-fresh-only, N4 signal-unavailable. Four and not one, because *an AND of three terms cannot be proven by neutralising all three at once*: a single test that freshens everything would go green on a predicate reading only one surface. The fixtures of the two earlier reaper files were updated to silence all three surfaces for the same reason — a worktree-only fixture no longer describes a stalled pilot, it describes a false positive.

The 2700 s window is unchanged and is now applied to each surface. It is very wide for the transcript (factor 7 on the measured max inter-turn gap); tightening it per surface would catch a real stall sooner, but the inter-turn distribution is measured on two runs only, so mika#2277 keeps one window and leaves the split to its own ticket.

**Open risk, carried deliberately (mika#2277):** the corrected predicate may have **no population at all**. `audit_events` holds exactly two `pilot_silent_stall` rows to date — the two false positives; no true positive has ever been reaped. The three founding cases of mika#2249 cannot be re-measured on the new signals (their transcripts were never ingested) and their pilot logs were **active** at T−5 min and T−3 min of a manual disposition taken "after CHECK mtime-worktree" — i.e. diagnosed with the predicate this ticket refutes. So the "SDK stall" class is less established than n=3 suggested. The verdict is to be taken on audit rows, not intuition; the AC4 warning is what makes the inertia legible while that accumulates.

The disposition runs **before** the audit write so `after_value` reports what happened rather than what was intended: a kill that failed and a kill never attempted must not produce the same row. `before_value` carries the status **actually read off the row**, never a constant — hard-coding `in_progress` there would make every audit row assert the very thing mika#2272 disproved.

**`failed`, not `cancelled`, behind a discriminator pre-written before the signal.** `process_kill::pre_write_cancel_reason(pid, CANCEL_REASON_PILOT_SILENT_STALL)` puts `STATUS=REAPED_PILOT_SILENT_STALL` in `/tmp/mika-cancel-reason-<pid>` first; `dispatch-lib`'s TERM trap only writes when that file is absent, so the trace names the real cause instead of `CANCELLED_BY_SIGNAL` — which `self-dev-callback` reads as an operator cancel and answers with *do NOT retry*, killing the pilot **and** the retry. The name is deliberately outside the `CANCELLED_BY_*` family and falls through that parser's branches. Re-dispatch stays with `stuck_ready_reconcile` (`auto_pull.rs`, re-drive budget from mika#2020); a retry branch in a skill prompt would be prompt-enforcement on loop substrate.

The audit row's `reasoning` carries **all three** idle ages since mika#2277 (`worktree_idle_secs`, `transcript_idle_secs`, `pilot_log_idle_secs`), not the worktree alone. That row is the only surface anyone reads this mechanism through, so it has to let the decision be replayed — and `worktree_idle_secs=2758` reported by itself is exactly what made the 2026-09-10 false positives look nominal on first inspection.

**SOLE WRITER:** `pilot_silent_stall` — this method is the only site writing that audit `tool_name` and the only site writing `error_reason = "pilot_silent_stall"` on `tasks.result`. Both strings are the operator's discriminator between silent-stall reaps, dead-PID subprocess deaths (`subprocess_exited_without_delivery`), delivered-without-PR orphans (`task_engine_reaper`) and childless parents (`task_engine_childless_reaper`); reusing either collapses two populations that need separate counts.

**Orphaned parent reaper (#871):** `reap_orphaned_parent_tasks()` runs every 60-tick cycle (same cadence as other periodic scans) and detects parent self_dev tasks left `in_progress` after their callback subtask delivers without producing a PR. Detection query (`find_orphaned_parent_tasks`): parent `status='in_progress'`, `source='self_dev'`, `trigger_type='manual'`; child `trigger_type='callback'`, `action_type='resume_agent'`, `status='delivered'`; child `updated_at` older than `REAPER_GRACE_SECONDS` (600s); parent metadata has no `$.claude_pilot.pr_url`; `NOT EXISTS` active sibling guard (defers when #870's retry loop launched a new callback child). On match: transitions parent to `failed` via guarded `update_task_failed` (terminal-state check prevents TOCTOU race), emits `audit_events` row with `tool_name='task_engine_reaper'`. Pre-existing leaks (age > 24h) get a distinct log line for post-deploy backfill visibility.

**Parent auto-completer (mika#1162):** success-side coupled pair of the reaper. Covers the gap where callback delivers with `pr_url` (success indicator) but the silent agent turn fails to call `update_task_status` (timeout, max-steps continuation that drops the call, transport error). Two layers: (1) inline `try_complete_parent_on_callback_success` in `dispatcher.rs` runs alongside `try_extract_callback_metadata` (#376) and `try_promote_parent_on_retry_success` (#958) on every callback delivery — fires immediately so the dispatch slot frees fast. (2) periodic `complete_parent_tasks_on_callback_success` in `engine.rs` runs every 60-tick cycle as a backstop for crash-recovery cases (server died between callback delivery and the inline call) and pre-deploy wedges. Both paths use the same `find_completable_parent_tasks_on_pr_url` DB query — identical join shape and guards to the reaper but inverted on the `pr_url` predicate (`IS NOT NULL` instead of `IS NULL`), so the two queries never select the same row. Both paths transition `in_progress → completed` via `update_task_completed` (which guards `status IN ('pending', 'in_progress')` — concurrent operator cancels or agent updates lose the race cleanly) and emit `audit_events` row with `tool_name='task_engine_parent_completer'` and a reason string starting with `parent_completed_from_callback`. Inline-path reason carries the literal `parent_completed_from_callback (pr_url: <url>)`; periodic-path reason carries `parent_completed_from_callback_backstop (pr_url: <url>)` for source distinction. SOLE WRITER: `task_engine_parent_completer` audit events come from these two sites only — coupled pair with `reap_orphaned_parent_tasks` (failure-path SOLE WRITER of `task_engine_reaper`). Together the two backstops cover every (callback outcome × parent state) combination.

**Dispatch-refusal resolver (mika#2158):** the refusal-side third sibling of the two above. `try_resolve_parent_on_dispatch_refusal` runs inline on every callback delivery and transitions the parent `in_progress → completed` when the callback body is a `dispatch-lib` auto-skip (`{"status":"auto_skipped","reason":"…"}` — canonically `already_groomed`, mika#2012 exit semantics via `_deliver_callback` + `exit 0`).

**Why `completed`, not `failed`:** the refusal is a *correct outcome* — the ticket genuinely is groomed. Marking it failed would say something went wrong and would pollute the count of real breakages that `task_engine_reaper` exists to hold.

**Why it matters beyond a tidy row.** Before this, the refusal reached the engine and produced nothing: the tracking row `ready_label_handler` pre-created stayed `in_progress` for ~60 min until the mika#1712 phantom sweep failed it. Measured 2026-09-03: 31 such rows on mika#1772, 8 on #2127, 7 on #2108, every one `phantom_aged_out`. For those ~60 min the ticket read `in_flight`, which the auto-pull Phase 2 reconciler treated as *progress* and used to zero the re-drive budget — so the guard meant to bound the loop was erased by the loop, every tick. The mika#2158 companion change in `auto_pull::classify_stuck_ready` (`in_flight` now skips **without** resetting) is the other half; neither closes the livelock alone.

**Scope:** all `auto_skipped` reasons, not just `already_groomed`. The phantom mechanism is identical for `issue_closed` (mika#988) and for any future refusal; the reason is carried verbatim into `tasks.result` and the audit row, so the populations stay separable. Filtering on the one measured reason would leave the same defect open under another name.

**SOLE WRITER:** `dispatch_refusal_resolver` audit rows come from this site only. That is load-bearing for attribution (mika#2158 M6a): a `phantom_aged_out` row used to be compatible with two incompatible stories — the pilot never ran, or `dispatch-lib` refused and the refusal never landed on the engine. Now the second leaves a named trace, so its **absence** under a phantom is itself evidence for the first.

**Childless-parent reaper (mika#1687):** `reap_childless_stuck_parent_tasks()` runs every 60-tick cycle after the auto-completer (so delivered-child success/failure cases resolve first). The deterministic backstop for **silent pilot death** — a parent that reaches `in_progress` but never records a callback child falls through all three sibling mechanisms above: the orphan reaper and auto-completer both INNER-JOIN a delivered callback child, and the watchdog keys off the callback child's PID. Detection query (`find_childless_stuck_parent_tasks`): parent `status='in_progress'`, `source='self_dev'`, `trigger_type='manual'`, `type='issue'`, `updated_at` older than `MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS` (default 1800s / 30 min — far larger than the orphan reaper's 600s because a legitimately-dispatching parent is childless only for the sub-second window between its `pending → in_progress` transition and the callback-child row commit), and `NOT EXISTS (SELECT 1 FROM tasks child WHERE child.parent_task_id = parent.id)` — the **exact complement** of the sibling reapers' INNER JOIN, so the three selection sets are disjoint by construction. On match: transitions parent to `failed` via guarded `update_task_failed` (terminal-state check prevents TOCTOU race), emits an `audit_events` row plus an INFO `task_engine_childless_reaper.reaped` log (with `age_minutes`); reuses `get_reaper_child_snapshot` for a `.evaluated` log confirming `children_count == 0` at decision time. Its job is **fail-with-telemetry, not re-drive** — freeing the dispatch slot + emitting a greppable signal; re-driving the still-open ticket is mika#1824's job at the auto-pull/label layer. Scoped to `type='issue'` in v1 — milestone/project childless-stuck detection is a deferred follow-up (they carry their own advancement backstops #991/#1218). **SOLE WRITER:** distinct `error_reason = "stuck_in_progress_no_callback_child"` (on `tasks.result`) and distinct `tool_name = 'task_engine_childless_reaper'` (on `audit_events`) — neither string is written anywhere else; reusing either breaks the operator/monitor discriminator that counts silent-pilot deaths separately from delivered-without-PR orphans (`task_engine_reaper`) and dead-PID subprocess deaths (`subprocess_exited_without_delivery`).

**Supersede-on-new-dispatch (mika#1934 AC2, mika#2263, mika#2335):** `tracking_cleanup::supersede_prior_tracking_rows` cancels the stale tracking rows a fresh dispatch for a `reference_url` replaces, and — since mika#2263 — disposes of the pilot still running under them. Fail-open end to end: superseding is a courtesy cleanup, never a precondition for dispatch.

**A dispatch is TWO rows, and forgetting it cost mika#2263 its entire effect.** The **parent** tracking row is `manual` / `action_type='none'`, carries the issue URL, and **never** carries a `process_id`; the **callback child** is `callback` / `resume_agent`, carries the pgid, and **never** carries a `reference_url`. mika#2263's kill resolved its population with a query keyed on both at once (`process_id IS NOT NULL AND reference_url IN (…)`) — a conjunction **empty on the topology production writes**, so it could never kill a dispatch pilot, in that incident or any other. Measured 2026-09-15: the log of the surviving pilot `590a06c0` contains zero occurrences of `SIGTERM`, `CANCELLED_BY`, `superseded` or `Killed`, and two pilots wrote to the `feat-2334` worktree for **28 min 54 s**. The general rule worth keeping: *a resolver written a second time is a resolver that can disagree with the first* — the traversal that works already existed and was tested (`find_dispatch_children_with_pid`, mika#2156, whose own doc says "the missing link is read here, not added"). The dead query was **deleted**, not left in place; one that can return nothing is read as a guarantee by whoever finds it next.

**Order and population.** The candidate parents are computed **once**, before the kill, and reused for the row cancellation — the two halves used to run independent lookups that could diverge. The processes die **before** the row bookkeeping, deliberately: the fresh dispatch is about to claim the same worktree. Two filters bound the population, and both matter now that the kill lands: a child whose status is terminal has no pilot left to kill (its pgid is stale), and a child with **no readable `process_start_time` is not signalled** — `kill_process_gracefully` would fall back to a bare `/proc/<pid>` check, which cannot tell the pilot from a recycled PID, and mis-signalling a process *group* is unbounded damage. That child survives its supersession; the cost is named rather than hidden, counted under mika#2156's `unusable_child_count` vocabulary. Inertia, never a blind kill.

**`fired_at` on the parent (mika#2335 F2a).** `Database::mark_parent_dispatched` is the SOLE WRITER of the parent-side dispatch transition, and the **three** production dispatch paths go through it: `skills::executor::execute_long_running` (#525, the original), `server::ready_label_handler` and `server::verdict_handler` — the last two exist because the first was copied, and say so in their own comments. mika#2263 stamped `fired_at` in `set_task_process_id`, which is correct and is the **child's** writer; the parent — the row `mika tasks`, the dashboard and the health probes actually read — went through `update_manual_task_status`, which writes `status`, `updated_at` and `completed_at` and nothing else. So a live dispatch's parent read "never fired" for the pilot's whole life, and on 2026-09-15 an operator read exactly that and cancelled a pilot 38 minutes into its work. A **named writer** rather than a `CASE` added to `update_manual_task_status`, because that method also serves `rewind.rs`, which restores a prior status and must stamp nothing. An existing `fired_at` is never overwritten — the reapers measure a dispatch's age from it. **Blast radius, censused:** the only predicate reader of `fired_at` is the `long_running` probe in `get_task_health_summary`, which filters `trigger_type != 'manual'` and therefore never sees a parent; every other reader is a display surface.

**Guard:** `db::tests::mika2335_no_production_dispatch_transitions_a_parent_without_stamping` — a source scan refusing a fourth `update_manual_task_status(…, "in_progress")` in production code, with an **empty allowlist** (the three sites migrated; none is exempt). A behavioural test cannot catch this class: the regression makes no covered decision wrong, it leaves a *new* path mute while every existing assertion stays green. Its scope is lexical on the `"in_progress"` literal — `rewind.rs` and `tools/update_task_status.rs` pass the status by variable and escape it structurally, which is correct (neither dispatches), but a future dispatch path building its status in a variable would pass underneath. Disposition for a fourth site is **halt-and-surface**, not an allowlist entry: whether it dispatches a parent is a question the guard cannot answer for you.

**Operator surface (mika#2335 F2b/F2c).** `mika tasks list|get` now report **measured** pilot liveness (`is_same_process_alive` + the claude-pilot log mtime) instead of inferring "executing" from the presence of a PID column, and on a **parent** row they traverse to the dispatch child — before this, the row an operator consults showed nothing at all. `mika tasks cancel` warns, names the PID and the log age, and asks for confirmation when a pilot is alive (`--yes` / `-y` to skip; refused outright in a non-TTY without it). It warns and asks rather than refusing: killing a live pilot is sometimes exactly the intent. This is the tool-side engraving of the operator lesson the incident produced — *never cancel without checking the PID and the log mtime.*

**Live-pilot predicate (mika#2279):** `crates/mika-agent/src/live_pilot.rs` — **sole reader** of "is a pilot alive for this ticket?". Two surfaces ask it, the ready-label handler's gate 2c and the `auto_pull` Phase 2 filter 4b, and one module answers.

**The topology is the whole difficulty, and it is the same two lines mika#2335 had to name.** The URL lives on the parent, the pgid on the child, nothing carries both — so every predicate reading a single row is blind to half the state. `has_active_self_dev_task_for_issue` conjoins `reference_url LIKE …` with `status IN ('pending','in_progress')` on one row, and that conjunction goes **false** the instant a supersession cancels the parent while its pilot keeps working. `find_dispatch_children_for_issue_url` traverses child → parent on `parent_task_id` with **no predicate on the parent's status**: that absence *is* the fix, a cancelled parent being exactly the state where the question needs asking.

**The loop it closes, measured on #2276 (2026-09-10).** A second `labeled ready` landed 31 s after the dispatch. Supersede cancelled the parent — and since mika#2335 that also **kills** the running pilot — then the readiness check failed and a duplicate deferred dispatch was registered; `auto_pull` then read the cancelled parent as "nothing in flight" and re-drove the label. One full turn every ~20 min, every GitHub event by `mika-platform-bot`, no human in it. The defect is self-sustaining, and each turn spends one point of the mika#2020 re-drive budget — three turns abandon a healthy ticket to `operator-review`.

**Where the guard belongs.** Not in `supersede_prior_tracking_rows`: disposing of the pilot a **new** dispatch replaces is the supersession's explicit contract, and guarding it there would undo mika#2335. A replayed `labeled` event is not a new dispatch, so the guard is on the trigger. Gate 2c sits between the repo allowlist (2b) and the token (3) — before any `gh issue view`, since the predicate reads the issue's URL and never its body, and before 6b/7, which is the property the three neighbouring gates already state: zero task created, zero process signalled, zero dispatch deferred. It refuses in `Handled`, never `Passthrough`, for the reason written at each of its neighbours.

**Fail-safe, and the asymmetry that decides it.** `Unreadable` is not `Alive`: an unreadable `process_start_time`, a `process_id` outside `u32`, a DB error — none can *prove* a pilot is alive, so none blocks, and the pre-fix behaviour is resumed. That population is the one mika#2335 counts under `unusable_child_count` and is the only shape in which #2279 can still occur; it is named rather than hidden. A false `Alive` freezes **one** ticket and the freeze is bounded (the PID watchdog #959, the silent-stall reaper #2249/#2277 and the phantom sweep #1712 all make the row terminal, after which the predicate goes false on its own); a false `None` replays the incident in a loop. `LivePilotVerdict::is_alive()` exists so a caller cannot write `!matches!(v, None)` and invert that.

**Cost, bounded by ordering rather than by a cache.** Phase 2 resolves the fact **only when `in_flight` is false**, and `classify_stuck_ready` keeps its `live_pilot` branch *after* the `in_flight` one — pinned by `mika2279_the_nominal_in_flight_case_is_still_named_in_flight`, because a branch that drifted above would both charge the nominal case a `/proc` probe and merge two populations that must stay countable apart. The verdict is `Skip`, never `SkipAndResetBudget`: waiting for a pilot is right, calling the wait a success is what made the mika#2020 guard unreachable (mika#2158).

**Filter B is not redundant with gate A.** "Terminal parent + live pilot" has other legitimate producers, the clearest of which is written in the code itself: when the supersession's kill does **not** land (EPERM, survival past SIGKILL), `tracking_cleanup` deliberately leaves the row and its pgid intact so a reaper can still reach the process. Add `mika tasks cancel --yes` and the grooming branch of `tools/create_task.rs`. **Out of scope, deliberately:** the LLM path into the supersede (`tools/create_task.rs`), whose legitimacy is not settled by this predicate.

**Operator surfaces.** `ready_label_pilot_in_flight` (INFO + an `audit_events` row of the same name, keyed on the issue reference — no `task_id` exists at that point by construction, which is the property the gate is defined by), and the `live_pilot_orphaned_parent` exclusion filter on the `auto_pull` side. Queries, expected regimes and the 48 h post-deploy probe: root `CLAUDE.md` § *Un `labeled ready` répété sur un pilote vif est un NO-OP*.

**QA-review reconciler (mika#2334):** `qa_review_reconcile::reconcile_qa_review_requests()` — the fourth recurring scan, beside `auto_pull` and `wip_rescue`, carried by mika-dev on cron `0 */15 * * * *`. It asks `mika-platform-qa` for a review on the loop's open PRs that nothing came to review.

**The event it backs up is unique and nothing replays it.** `pull_request.opened` is what starts the QA cascade (routed by the gateway, no draft filter), so the pilot's death after the push cannot by itself prevent the review — the founding ticket's causal premise does not hold, and the trailing `gh pr edit --add-reviewer` it wanted moved **did not exist anywhere in the repo**. What does hold is that `opened` is losable in four places (drop-oldest in the mika#1870 bounded queue → 429 → the gateway circuit breaker → a DLQ row that only a manual replay pulls back; plus the empty LLM turn, where `webhook_zero_tools` is opposed once) and that **no path re-read an open PR without a review**. The one pre-existing recovery, the mika#1711 `check_suite.completed(success)` fan-out, requires `draft: false` **and** a green CI, so a PR whose CI is red or never ran was never caught.

**The decision is a pure function, and what is NOT an input to it is the point.** `select_prs_needing_review(prs, now, cfg)` takes no signal about how the pilot terminated — that absence *is* the structural form of "independent of pilot survival" (AC1), and it is why the fix covers the three upstream losses too, none of which are pilot deaths. Six conjunctive terms, each **fail-safe** (unreadable information takes a PR *out* of the population, never into it): open, author = `DISPATCHER_FORGE_LOGIN`, `isDraft == false`, no request **and** no review from `REVIEWER_FORGE_LOGIN`, age inside `]MIN_AGE, MAX_AGE[`. The review term is not redundant with the request term: GitHub withdraws the request once the review lands, so without it every reviewed PR would be re-requested for ever.

**Why not a step before the fragile ones, as the ticket's comment asked.** An *unconditional* gesture at PR creation doubles the review on **every** PR of the loop, not just the stranded ones: `opened` has already started a session, and the `review_requested` the gesture emits targets exactly `REVIEWER_FORGE_LOGIN`, so `is_suppressed_review_request` (mika#1655) does not filter it and a second session starts — one that cannot observe the first, since `gh pr view` is outside qa-review's tooled perimeter and the first review is not posted yet anyway. The gesture must therefore be **conditional on the absence of a review**, which the shell tail cannot be: `dispatch-lib.sh` delivers its callback and dies in seconds, and an absence is only measurable after a delay. A prompt step is ruled out by `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`.

**No new literal.** `REVIEWER_FORGE_LOGIN` and `DISPATCHER_FORGE_LOGIN` both come from `mika_common::forge_identity`, which `mika-gateway` imports for the same filter — writing the login here would be the constant divergence a shell implementation would have guaranteed. **Token:** `Settings::resolve_github_token` (PAT-first, App fallback) via `resolve_periodic_scan_token`, covered by the mika#2205 structural guard; no label is written, so there is no `resolve_periodic_scan_label_token` here.

**SOLE WRITER:** `tool_name = 'qa_review_reconciled'`. That is what makes `SELECT … WHERE tool_name = 'qa_review_reconciled'` the exact list of PRs the loop had to catch up on — i.e. the measure of the nominal path's health. **Operator grep signals:** `qa_review_reconciled`, `qa_review_reconcile_tick` (aggregate, emitted **only when the tick acts**), `qa_review_reconcile_no_token`, `qa_review_request_failed` (should stay empty — `requested_reviewers` is the same family as the label write that already fails under PAT, mika#2228), `qa_review_reconcile_error`. Config and post-deploy probes: root `CLAUDE.md` § *réconciliation des demandes de revue*. **This is a net, not a path:** sustained volume means `opened` is being lost systematically and the net is now hiding the signal that would show it.

**Team task tree:** parent `invoke_orchestrator` task + child `resume_agent` tasks per delegation. Suspend/resume on pending grandchild callbacks. **Team-run user notification (#287):** fired once at terminal status from two symmetric callsites (`run_team` tool for sync completion, `dispatch_invoke_orchestrator` for async resume), both routing through `teams::notification::build_run_completion_message`. Per-child `resume_agent` callbacks have their user-facing `send_message` suppressed via `NoopSender`; the silent turn still runs (updates memory, records `llm_calls`) — only the user channel is gated. Deliverable text is UTF-8-safe truncated at 4000 chars (below Telegram's 4096 limit).

## HTTP Server (mika-spirit)

Axum-based with two auth layers: mutation endpoints require `MIKA_INTERNAL_TOKEN` only; read-only dashboard API accepts either `MIKA_DASHBOARD_TOKEN` or `MIKA_INTERNAL_TOKEN` (superuser).

**Mutation endpoints:** `/message` (202 async, 10MB limit), `/tasks/{id}/complete` (200 sync, 100KB cap), `/tasks/{id}/cancel` (200 sync), `/api/v1/rewind/*`, `/a2a/*`.

**Dashboard API:** `/api/v1/*` — timeline, agents, sessions, messages, traces, investigate, tasks (+ detail/children/descendants/sessions), team-runs (+ summary), llm-calls (+ detail), tool-calls (+ detail), dev-runs (+ detail), skills (+ property filters), github proxy endpoints. CORS scoped to `MIKA_CORS_ORIGIN`.

**Skills listing (#606):** `GET /api/v1/skills` lists loaded skills with optional property filters: `?source=bundle|marketplace` and `?always_on=true|false`. AND semantics. Invalid param values silently ignored (no filter applied for that property). Returns `{ "skills": [...] }` with per-skill `name`, `description`, `origin`, `enabled`, `always_on`, `tools` fields.

**Lazy agent resolution (#1399):** `AppState.agents` uses `DashMap` for concurrent mutable access. `resolve_agent()` is async — on cache miss (agent created after server startup), it checks for `identity.toml` on disk + DB row, lazy-constructs the `AgentState` via the same `init_agent` factory used at startup, and inserts into the map. Subsequent calls hit the fast path. Emits `agent_resolved_lazily` INFO event on successful lazy insert. Agents whose dir is deleted while running remain in the map (orphan removal is mika#1436).

**Time-range filtering (#659):** All list endpoints (timeline, sessions, llm-calls, tool-calls, team-runs, tasks, dev-runs) accept `from`/`to` ISO 8601 string query params for server-side filtering against the surface's primary timestamp column (`created_at` or `started_at`). String comparison is correct because ISO 8601 ordering matches chronological ordering. Frontend emits via `<TimeRangeFilter />` from `@samidarko/ui`.

**Request logging:** `tower_http::trace::TraceLayer` middleware. `inject_request_meta` middleware copies method+path for top-level JSON fields. `/health` logged at DEBUG. Agent lock via `tokio::sync::Mutex<()>` with non-blocking `try_lock` (429 if busy).

**Failed sends flush:** Before each message processing, flushes up to 5 pending failed outbound sends from DB.

**Wedge diagnostics (mika#1722):** When mika-spirit wedges (mika#1719 was n=3 in a single week), the canonical capture tool is `scripts/mika-spirit-wedge-capture.sh <pid>`. It writes three timestamp-prefixed files into `/tmp/spirit-wedge-<ts>/`: `<ts>-stacks.txt` (gdb `thread apply all bt 30` + `info registers`), `<ts>-maps.txt` (`/proc/<pid>/maps`, which IS the PIE-base artifact — the first line's start address is the load address of the main binary, never computed from stack traces), and `<ts>-status.txt` (`/proc/<pid>/status`). Captures run concurrently as background jobs so the maps snapshot is close-in-time to gdb's attach. Missing gdb is a warn-and-continue path — maps + status alone are enough to compute PIE base and thread count. Root-Claude uses this script for any post-wedge snapshot; do not run bare `gdb -p <pid>` and lose the maps snapshot.

**WAL checkpoint (#636):** `server::checkpoint::spawn_dashboard_checkpoint_task()` runs `PRAGMA wal_checkpoint(PASSIVE)` on the dashboard DB connection every 60 seconds. Defense-in-depth against stale WAL snapshots: if a transaction leak pins the connection's read snapshot, the periodic checkpoint forces the connection to advance. Structured log events: `checkpoint.start`, `checkpoint.complete` (with `busy_pages`, `log_pages`, `checkpointed_pages`), `checkpoint.error`, `checkpoint.stopped`. Hard-coded 60s interval (future tunable: `MIKA_DASHBOARD_CHECKPOINT_INTERVAL_SECS`). If WAL grows despite PASSIVE checkpoints, escalate to RESTART mode per the operating-envelope trigger documented in `checkpoint.rs`.

## Observability

"Always instrument, optionally export" pattern. Two orthogonal correlation axes: `trace_id` (per-request/per-turn, 32-char hex) + `session_id`/`agent_id` (system-level). `unified_timeline` VIEW enables cross-subsystem queries.

**Span filtering:** Per-layer `filter::Targets` on the OTel layer exports only `target: "mika::otel"` spans (LLM calls, agent turns, server requests).

**LLM observability:** `llm_call` spans with `gen_ai.*` semantic convention attributes, feature-gated behind `#[cfg(feature = "telemetry")]`. When `log_llm_bodies=true`, request/response bodies are attached as `gen_ai.prompt`/`gen_ai.completion` span attributes for Langfuse Generation input/output (#671). Response bodies use `serialize_response_text()` from `mika-common::llm` (same format as `llm_calls.response_text`). Team engine emits `TeamEvent` variants for live dashboard updates.

**`turn_usage` says how big the brief was, including when the turn fails (mika#2331 AC1).** `TurnUsageFields` carries `request_bytes` and `system_prompt_bytes`, threaded through all three `build_turn_usage_fields` / `emit_turn_usage` pairs (the `Ok` arm, the `Err` arm, and `save_continuation_llm_call`). The `Err` arm is the point: `usage` is `None` there, so every token count is 0, and the line used to say nothing about the size of the brief that timed out — `input_tokens = 0` was all an operator measuring a 420 s hang could read. Both values are computed **before** the call (`request.payload_bytes()`, `system_prompt_len`) and therefore survive it; both were already written to `llm_calls` (v53 / v38) and were simply invisible to the log stream, so this moves an existing measurement from a gated surface to the ungated one. `Option`, and `null` is never `0`: no request is empty, so a zero would be a readable lie about a measurement that did not happen. Both are RAW dimensions, so Prime hard condition #1 (no `phase`/`is_planning`/`role`) still holds. Companion event `llm_call_attempt` lives in `mika-common` (see its CLAUDE.md); the procedure that reads the two together is in the workspace `CLAUDE.md` § *Lire un hang LLM « 420 s sans octet »*.

**Session lifecycle:** Silent dispatcher variants call `end_session()` after completion. CLI commands call `end_session()` on all exit paths. `startup_recovery()` prunes old sessions via `prune_old_sessions()`.

### Guard Fabrication Telemetry (#953)

Structured telemetry for the five fabrication-class EndTurn guards. Each guard firing emits a `target: "mika::otel"` event (exported to Langfuse via OTLP when telemetry is enabled, always in log file). Two paired event types:

**Detection events** — emitted when a guard fires (step N):

| Event name | Guard position | Fires when |
|---|---|---|
| `guard.callback_state_claim` | 4c | Callback turn claims downstream state without `run_gh`/`check_task`/`gh_read` |
| `guard.fabricated_action_claim` | 5 | Claims action with GitHub URL but zero tool calls |
| `guard.dev_groom_fabrication` | 5b | Claims Verdict without `run_claude_pilot_groom` call |
| `guard.doctrine_public_promo` | 5c | Proposes / drafts a prohibited public-launch surface (Show HN, Product Hunt, Reddit launch, Twitter promo, growth-hack) in violation of Mika's invitation-only distribution doctrine (mika#1814) |
| `guard.false_local_hosting_claim` | 5d | Asserts this instance runs locally (or that the user's data never leaves their machine) while the resolved `Deployment` is not `Local` (mika#2290). Extra fields: `deployment`, `persona`, `matched_subject`, `matched_assertion` |
| `guard.response_language_drift` | 5f | Answers in a language other than the one this tenant declared via `customer_config.language` (mika#2247). Extra fields: `expected_language`, `detected_language`, `detected_hits`, `expected_hits`. Expected regime: **low but non-zero** — each line is a turn caught |
| `guard.time_of_day_greeting_mismatch` | 5g | Greets with a formula naming a part of the day the tenant is not in, while the local hour is known (mika#2247). Extra fields: `greeting`, `implied_part`, `actual_part` |
| `guard.asserted_unavailability` | 6c | Claims tool unavailable when it's in the enabled set |
| `guard.assert_grounded` | 6d | Affirms resource state without grounding tool call |

Fields: `trace_id`, `agent_id`, `session_id`, `step`, `guard_correlation_id` (`{trace_id}:{step}:{guard_label}`), `label` (mode), plus guard-specific detail fields.

**Correction event** — emitted when the next EndTurn passes all guards (step N+1):

| Event name | Fires when | Key fields |
|---|---|---|
| `guard.correction_accepted` | EndTurn accepted after a guard previously fired | `guard_correlation_id` (matches detection), `original_guard`, `original_step`, `corrected_content` (truncated to 2000 chars) |

**Residue event** — emitted when a single-retry budget is spent and the violation goes out anyway:

| Event name | Fires when | Key fields |
|---|---|---|
| `guard.false_local_hosting_claim_uncorrected` | Guard 5d already fired this turn and the re-prompted response still asserts local hosting (mika#2290) | `deployment`, `matched_subject`, `matched_assertion`, `step`, `label` |
| `guard.response_language_drift_uncorrected` | Guard 5f already fired and the re-prompted response is still in the wrong language (mika#2247) | `expected_language`, `detected_language`, `step`, `label` |
| `guard.time_of_day_greeting_mismatch_uncorrected` | Guard 5g already fired and the re-prompted response still names the wrong part of the day (mika#2247) | `greeting`, `implied_part`, `actual_part`, `step`, `label` |

**Expected regime: zero lines.** It exists because that population is otherwise indistinguishable from a healthy turn — the blind spot the Fire-Disposition gate (mika#1574) is there to close. It is **not** a second correction: the family grants one re-prompt, and a guard that diverged from its neighbours on that point would become the one everybody re-reads to find out why.

**Architect self-report** — complementary success signal (not fabrication detection):

When a verdict-producer agent (mika-arch) self-reports inability to anchor a citation (the desired behavior per #952 skill prompts), an `audit_events` row is written with `tool_name='fabrication_guard'`, `target_key='arch_anchoring_self_report'`. Queryable via SQL: `SELECT * FROM audit_events WHERE target_key = 'arch_anchoring_self_report'`.

**Query patterns:**
- Detection events: `grep 'guard\.' server.log | jq 'select(.event | startswith("guard.") and . != "guard.correction_accepted")'`
- Correction events: `grep guard.correction_accepted server.log | jq '{guard_correlation_id, corrected_content}'`
- Cross-event join: match `guard_correlation_id` between detection and correction events
- Self-report (SQL): `SELECT * FROM audit_events WHERE target_key = 'arch_anchoring_self_report'`

### Log Sinks

Two distinct sinks carry per-agent runtime log events (team runs have a third — see the table). Both emit the same structured JSON with an `agent_id` field on every entry — the difference is the file target and the process that writes to it.

| Sink | Initializer | Process | Path | Rotation |
|------|-------------|---------|------|----------|
| **Server log** | `mika_common::logging::init()` (`crates/mika-common/src/logging.rs:240`) | `mika-spirit` (long-running daemon) | `MIKA_SPIRIT_LOG_FILE` (e.g. `/var/log/mika/server.log`) | None — single file via `tracing_appender::rolling::never` |
| **Per-agent CLI log** | `mika_common::logging::init_pretty()` (`crates/mika-common/src/logging.rs:356`) | `mika-cli`, **for what still runs in-process**: `mika chat` (TUI) and its silent-mode background turns (heartbeat, reflection, reminder). **Not** `mika ask` agent turns since mika#1727 — see the carve-out below. (`mika team run` writes to the team sink, `~/.mika/teams/<team>/logs/`, via `init_team_logging` — `crates/mika-cli/src/main.rs:433`.) | `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` (daily) | Daily via `tracing_appender::rolling::daily` |

**`mika ask` carve-out (mika#1727, documented by mika#2069).** `mika ask` no longer runs the agent loop in-process. `crates/mika-cli/src/commands/ask.rs` is a thin client: it ships the prompt to the local mika-spirit daemon over A2A `message/send` at `{spirit_url}/a2a/{agent}` and renders the returned Task, with no in-process fallback. Spirit owns the execution session, so **every agent turn of a `mika ask` — `turn_usage`, `llm_call`, tool events — is emitted by the spirit process and lands in `MIKA_SPIRIT_LOG_FILE`**, not in the per-agent sink. The per-agent file is still initialized as that process's log sink (`crates/mika-cli/src/main.rs:233`), so the invocation's *client-side* events land there — the file is populated, just not with what a turn-accounting reader is after. Measured 2026-08-30 (mika#2069): 21 084 `turn_usage` in `/var/log/mika/server.log` against 346 across every per-agent log combined, and the 346 were all `heartbeat-…` / `reflection-…` / `reminder-…` / `mika chat` sessions — not one execution turn.

**Decision tree for operators:** If you want to read the running mika-spirit's runtime events (skill execution, task engine, callback lifecycle, autonomous-loop wedges) — **or any agent turn, including a `mika ask` one** — read `MIKA_SPIRIT_LOG_FILE` filtered by `agent_id`. Read `~/.mika/agents/<name>/logs/mika.log.<date>` for a `mika chat` session's turns, for silent-mode background turns (heartbeat, reflection, reminder), or for a `mika ask` invocation's client-side events. Both contain the same `agent_id` field on every entry, so cross-filtering by agent works in either sink.

**Single-sink rationale:** mika-spirit uses one file with `agent_id`-filtered queries instead of per-agent file appenders because: (a) per-agent appenders would double the disk-write rate per event, (b) they create a sync gap risk if the per-agent appender worker can't keep up, and (c) they duplicate data already correctly addressable via the JSON `agent_id` field. The per-agent CLI sink is correct for its purpose (discrete CLI invocations) and is not a substitute for the server log.

**Every event is one line — but only since mika#2195, and the cut-off matters when you read old data.** The JSON branch of `logging::init` used to compose *two* JSON layers, one on stdout and one on the file, while the OpenRC unit redirects stdout into that same file (`supervise-daemon mika-spirit --stdout /var/log/mika/server.log --stderr /var/log/mika/server.log`). Every event therefore landed in `server.log` **twice**, uniformly — not a `turn_usage` quirk but a property of the layer composition: `mika-spirit listening` (the server binds its port once) appeared twice per restart, `domain_rebuild_complete` four times per boot. **Any count taken over log lines written before the mika#2195 deploy is doubled** — that is the origin of RT-005's factor-2 (−7,43 → −3,71 deduplicated) and of mika#2179's "38 failures" that were 19. It never affected *values* inside an event, only how many times the event appeared, so deduplicating is the whole correction. The fix withholds the stdout layer whenever a log file is configured (`json_stdout_layer_enabled`, `crates/mika-common/src/logging.rs:226`); the price, stated: with `MIKA_SPIRIT_LOG_FILE` set, `docker logs` and `supervise-daemon --stdout` carry no application lines — read the file. Post-deploy probe: `grep -c 'mika-spirit listening' $MIKA_SPIRIT_LOG_FILE` over one restart window must be **1**, not 2. A 2 means the fix did not take — halt, do not redeploy blindly.

**Common mistake — and note the failure shape:** Audit tooling that reads `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` for server-mode events (or for agent turns of any kind since mika#1727) does **not** get an error, and does **not** get an empty file. It gets a sparse, plausible, wrong dataset: the heartbeat / reflection / reminder / `mika chat` turns that legitimately live there, which look enough like the real thing to be analysed by mistake. That is what mika#2069 caught in the RT-005 measurement channel. Use `jq 'select(.agent_id == "<name>")' < $MIKA_SPIRIT_LOG_FILE` for server-mode queries and for any turn accounting. `mika logs --agent <name>` prints both resolved paths.

## Audit Log

`audit_events` table tracks all memory mutations per session. All writes include `trace_id`.

### `tool_name = 'auth_boundary'` — cross-boundary authentication failures (mika#1949, Porte 3)

The mika-manager write path crosses four boundaries, each guarded by a different
env-var-backed token. Before mika#1949 a failure at any of them was
indistinguishable from a network error — mika#2013 is the measured precedent:
a frozen installation token cycled `auth_class=401` sixteen times in one night
without naming itself. These rows are the fix: one per observed authentication
failure, naming the token and the boundary.

**Row shape** (written only by `Database::record_auth_boundary`, `evidence/audit.rs`):

| column | value |
|---|---|
| `tool_name` | `auth_boundary` (`evidence::audit::AUTH_BOUNDARY_TOOL_NAME`) |
| `session_id` | `auth-boundary` (`AUTH_BOUNDARY_SESSION_ID`) — a boundary failure belongs to no conversation |
| `target_key` | `<from>_to_<to>` |
| `after_value` | the failure kind — `missing` \| `empty` \| `invalid` \| `rejected` \| `unreachable` |
| `reasoning` | JSON `{"token_name","kind","from","to"}` |
| `before_value` | always `NULL` — there is no prior state to name |

**The four boundary pairs `target_key` can take:**

| `target_key` | token | site |
|---|---|---|
| `gateway_to_spirit` | `MIKA_INTERNAL_TOKEN` | `server/auth.rs` refusal arms |
| `cm_to_spirit` | `INTERNAL_TOKEN` | control-monitor `A2aAdapter` (mirrored shape, cm's own ledger) |
| `cm_to_content_plane` | `CM_FULL_ACCESS_TOKEN` | cm `scope::check` |
| `manager_to_delivery` | `MIKA_MANAGER_DELIVERY_TOKEN` | `milestone_manager` report + alarm delivery |

**Two properties, both load-bearing:**

- **A token NAME, never a token VALUE.** `AuthBoundaryError` (in
  `mika-common::auth_boundary`) has no field that can hold a secret, and both
  it and this writer carry a negative-control test built with a secret-shaped
  *name* to pin that.
- **Fire-and-forget.** Call sites go through
  `crate::auth_boundary_ledger::AuthBoundaryLedger`, whose `record` returns
  `()`. An audit failure cannot change an authentication verdict, because the
  signature offers no way to propagate one. A failed authentication still drops
  the request exactly as before; only the drop became visible.

**Operator query:**

```sql
SELECT target_key, after_value, count(*)
FROM audit_events
WHERE tool_name = 'auth_boundary'
GROUP BY 1, 2;
```

Rotation procedure for all four tokens (and the deliberate 401/403 divergence
between mika and cm): `mika-platform/docs/operator/token-rotation-procedure.md`.

## Timestamps

All SQLite timestamp columns use ISO 8601 TEXT format (`%Y-%m-%dT%H:%M:%SZ`). The `crate::timestamp` module provides centralized helpers: `now()`, `format()`, `parse()`, `now_plus()`, `now_minus()`. Fixed-width UTC format ensures correct lexicographic ordering.

## DB Module Layout (mika#2321)

`crate::db` is a file module (`db.rs`) with a sibling directory (`db/`) — the
Rust 2018 layout, so **no `db/mod.rs` exists and none is needed**. Where a piece
of the DB layer lives is decided by one rule, not by taste:

| path | holds | why there |
|---|---|---|
| `db.rs` | types, `open`/`open_in_memory`, the ~30 thematic `impl Database` sections, free functions | the region that *churns* — methods get rewritten, replaced, deleted |
| `db/migrations.rs` | `migrate` + every `migrate_vN_to_vM`, and their tests | **append-only**: a migration is never deleted (v1…v54, for ever) |
| `db/tests/` | the `#[cfg(test)]` module, split by theme | **append-only**: a test is rarely deleted |
| `db/kg_schema.rs`, `db/operational.rs` | their own `impl Database` blocks | pre-existing, same idiom |

**The two split-out regions are the two whose size is monotone.** That is the
whole criterion (plan mika#2321 E1): `db.rs` had crossed the 1 MB cap
`scripts/check-secrets.sh` enforces, and was exempted by name in
`LARGE_FILE_ALLOWLIST` — an exemption that was not static but **unbounded**, and
under which the file took ~64 KB in three days without a signal. The allowlist is
now empty and the CI gate bounds the growth on its own.

**Callers did not move, and cannot need to.** A method stays `Database::foo`
whatever file carries its `impl`, so `async_db.rs` and the ~30 modules doing
`use crate::db::…` are untouched. `crate::db::tests::*` — a real inter-module
contract, imported by `skills::executor` — survives byte for byte.

**Two visibility rules govern any future move**, and they are what kept this
refactor at *one* signature change in total (`migrate` → `pub(super)`):

1. A child module sees its parent's private items; two siblings do not. So
   `db::tests` and `db::migrations`, both children of `db`, still reach every
   private item of `db` — but not each other's.
2. Therefore **a test module travels with the code it tests**. The migration
   tests live in `db/migrations.rs`, not in `db/tests/`, because they call ~31
   private `migrate_vN_to_vM`; putting them with the other tests would have cost
   ~31 widened signatures. The same rule pulled `migration_v38_to_v39_idempotent`
   out of `db/operational.rs`.

**A guard premise this invalidated, repo-wide.** Several structural guards
isolate "production" by truncating each file at its first `#[cfg(test)]` literal.
An extracted test module carries no such literal — the attribute stays on the
parent's `mod tests;` declaration — so those guards scan the whole test file as
production. The premise was already false for `db/tests/harnais_porte.rs`
(mika#2310) and `perimeter/tests.rs`, and merely benign. Three guards now consult
[`crate::source_scan::is_test_source_path`] (segment `/tests/` **or** filename
`tests.rs`, the predicate `perimeter/rules.rs` already tests) — `db` (mika#2335
F2a), `agent_loop` (mika#2305) and `auto_pull` (mika#2361). **If a fourth reddens
after moving test code: repair the classification, never widen the needle and
never add a file allowlist** — each of those guards refuses that in as many
words, and each carries a negative control proving it still catches a real
production site. Remaining `src/**/*.rs` scanners use other exclusion mechanisms
and have not been audited against this premise (follow-up).

## Schema Version

**Current: v53.** Tables: sessions, messages (with `internal` flag for agent-to-agent visibility), team_workspace, audit_events, skill_overrides (with `enabled` column for DB-backed disable state), tasks (with manual/callback/a2a trigger types and a `type` column distinguishing `issue`/`milestone`/`project`), a2a_task_map, a2a_artifacts, a2a_push_notification_configs, llm_calls (with `response_text` and `reasoning` columns for LLM output persistence, and the two size columns `system_prompt_bytes` / `request_bytes`), tool_calls, team_runs (with `delegation_count` / `solo_absorption` / `failure_context` columns and `status` CHECK expanded to include `failed_no_delegation` — mika#1676 — and `failed_transport` — mika#1671), schema_meta (migration state tracking), kg_entities, kg_relationships, operational_items (#1262 — canonical operational-item ledger with 7 kind variants, 6 status variants, source-based dedup), permission_decisions (#1733 — provenance ledger for operator permission decisions with `classifier_verdict`/`operator_decision`/`override_used`/`decision_authority`/`tenant_id`/`agent_id` columns), pilot_transcripts (#1705 — claude-pilot LLM-call transcripts ingested from JSONL files), served_content (#1867 — per-(agent, person, category) content-serve ledger for proverb/quote/joke/poem/recommendation/story/fact dedup). **Shared-corpus KG tables (keyed by `docs_root_hash`):** kg_chunks, kg_subject_entities (with `discovered` and `discovery_reason` columns for roster-grounding #1158), kg_subject_relationships, kg_chunk_subjects, kg_chunk_subject_relationships, kg_extractions (first-writer-wins via INSERT OR IGNORE). **Per-agent KG tables:** kg_subject_resolutions, kg_resolutions_log (outcome CHECK includes `skipped_discovered_subject`), agent_kg_corpora (agent_id to docs_root_hash mapping for multi-corpus fan-out). `unified_timeline` VIEW for cross-subsystem queries. Session-based message storage with FK. System sessions (`system-{agent_id}`) for compaction.

Recent migrations:
- v18->v19: `sessions.task_id` column for reverse session->task lookups. `get_sessions_for_task_tree()`.
- v19->v20: `skill_overrides.llm_provider` and `skill_overrides.llm_model` for per-skill LLM override.
- v20->v21: `llm_calls.prompt_variant` for skill prompt variant recording.
- v21->v22: `messages.internal` column (`INTEGER NOT NULL DEFAULT 0`) for agent-to-agent message visibility. TUI inbox mode filters internal messages at the DB level. Set by `delegate_task` tool and by `mika ask --task-id` relay sessions (without `--task-complete`). `AgentParams.internal` threads the flag through `run_loop` to all message save paths.
- v22->v23: `tasks.type` column (`TEXT NOT NULL DEFAULT 'issue' CHECK (type IN ('issue', 'milestone', 'project'))`). Foundational for milestone/project dispatch (mika#595): `create_task` accepts an optional `type` parameter; `list_tasks` and `check_task` surface it. mika core stays a dumb task store — orchestration logic lives in self-dev (mika-skills#149). `NewTask.r#type: Option<String>` defaults to `'issue'` via SQL DEFAULT when `None`. Constants: `TASK_TYPE_ISSUE`/`TASK_TYPE_MILESTONE`/`TASK_TYPE_PROJECT`/`VALID_TASK_TYPES` in `db.rs`.
- v23->v24: `skill_overrides.enabled` column (`INTEGER`, nullable tri-state). `NULL` = default (enabled), `0` = disabled, `1` = explicitly enabled. Replaces `.disabled` marker files (#629). `SkillOverride.enabled: Option<bool>`. `set_skill_enabled()` with default-equals-delete (row deleted when all columns are NULL). `apply_overrides()` evicts disabled skills from `SkillRegistry.entries` into `disabled: Vec<DisabledSkill>`. One-shot `migrate_disabled_markers()` converts legacy `.disabled` marker files to DB rows at startup (fail-open on marker removal).
- v24->v25: Knowledge graph tables. Domain layer: `kg_entities`, `kg_relationships`. Lexical layer: `kg_chunks` + `search_content` integration. Subject layer: `kg_subject_entities`, `kg_subject_relationships`, provenance tables (`kg_chunk_subjects`, `kg_chunk_subject_relationships`), `kg_extractions` tracking. Resolution layer (#691): `kg_subject_resolutions` (subject → domain edges with confidence, UNIQUE on agent_id+subject_entity_id+domain_entity_id), `kg_resolutions_log` (resolution tracking with outcome CHECK constraint, UNIQUE on agent_id+subject_entity_id).
- v25->v26: `kg_extractions.source_doc_hash TEXT` (nullable) for #757 extraction idempotency. Pending-doc query now compares the stored hash against `kg_chunks.source_doc_hash` directly; pre-v26 rows get NULL and re-extract once under the budget before populating. Additive-nullable; no backfill needed. See `src/db/kg_schema.rs` → **Idempotency key** for the full contract.
- v27->v28: `agent_kg_corpora` table (#798) — maps `agent_id` to `docs_root_hash` for multi-corpus query fan-out. Populated by startup lexical ingest. Enables agents with `[kg].docs_roots` (plural) to query across multiple corpora.
- v28->v29: Backfill migration (#908) — scrubs secret-shaped values from existing `tool_calls.input` and `tool_calls.output` rows using `secret_scrubber::scrub_secrets()`. Data-only, no DDL. `save_tool_call()` now applies `scrub_secrets()` to both `input` and `output` before INSERT. `ToolCallSummary` metadata (`input_summary`, `output_summary`) also scrubbed before serialization to `messages.metadata`.
- v29->v30: Expand `kg_resolutions_log.outcome` CHECK constraint to include `'matched_llm_db_fallback'` (#874). Table rebuild mirroring v26→v27 shape. Enables DB-fallback acceptance path for LLM matches outside the in-prompt candidate window.
- v30->v31: `llm_calls.response_text TEXT` and `llm_calls.reasoning TEXT` columns (#653). Stores serialized LLM response content (text blocks joined with newlines, tool-call summaries as `[Tool Call: name(args)]`, stripped of internal tags, capped at 50K chars) and extended thinking text. Additive ALTER TABLE with per-column `column_exists` guards for crash-recovery safety. `save_llm_call()` gains two new params. `get_llm_call_by_id()` uses `row_to_llm_call_detail` to read the new columns; list queries use `row_to_llm_call` which returns `None` for performance. New `get_tool_calls_by_llm_call_id()` query. New `GET /api/v1/llm-calls/{id}/tool-calls` endpoint.
- v26->v27: **Shared-corpus primary key** (#786 + #787). Six shared-layer KG tables (`kg_chunks`, `kg_subject_entities`, `kg_subject_relationships`, `kg_chunk_subjects`, `kg_chunk_subject_relationships`, `kg_extractions`) change primary-key scope from `agent_id` to `docs_root_hash` — a 16-hex-char SHA-256 prefix of `fs::canonicalize(docs_root)` computed by `kg::config::hash_docs_root`. Agents with the same `docs_root` now share a single corpus; extraction cost drops from N× to 1×. Per-agent tables (`kg_subject_resolutions`, `kg_resolutions_log`) FK-rewired but row-count preserved. `schema_meta` table added for migration state tracking. Two-phase migration: (1) DDL renames v26 tables to `*_v26_backup`, creates empty v27 tables (#786); (2) coalesce reads from backups, deduplicates via majority-vote (normalized `entity_key`, agent-count + mean-confidence + `MIN(id)` tiebreak), rewires FKs via temp lookup tables, drops backups, writes `v27_coalesce_complete` marker (#787). `v27_coalesce_sql()` is public for integration test access. `docs_root` resolved from `MIKA_KG_DOCS_ROOT` env var or CWD fallback at migration time. Startup guard refuses `Database::open()` when `schema_version == 27` and marker is absent. Recovery runbook: `docs/solutions/database-issues/kg-v27-stuck-migration-recovery-2026-04-24.md`.

- v33->v34: `tasks.dispatch_class TEXT` column (#1001) — nullable, CHECK constraint (`'implement'`/`'groom'`). Per-class dispatch slot split: the global single-session-at-a-time guard becomes per-class, allowing one implement + one groom dispatch concurrently per agent. Pre-v34 NULL rows treated as `'implement'` via `COALESCE` in the guard query. Partial index `idx_tasks_dispatch_class` on `(agent_id, dispatch_class, status)`. `update_task_dispatch_class()` method for mika#996 task-reuse pattern (flip class on groom→implement transition).
- v34->v35: Expand `kg_resolutions_log.outcome` CHECK constraint to include `'no_candidate_of_type'` (#1154). Table rebuild (RENAME → CREATE → INSERT SELECT → DROP) same pattern as v29→v30. Enables the resolver to distinguish phantom subjects (no domain counterpart) from genuine disambiguation failures.
- v35->v36: `kg_subject_entities.discovered INTEGER NOT NULL DEFAULT 0` and `kg_subject_entities.discovery_reason TEXT` columns (#1158). Additive ALTER TABLE. Marks entities flagged by the extractor as not in the canonical roster but clearly referenced in the document. Discovered entities skip resolution entirely.
- v36->v37: Expand `kg_resolutions_log.outcome` CHECK constraint to include `'skipped_discovered_subject'` (#1158). Table rebuild. Enables the resolver to short-circuit discovered subjects without attempting Stage 1/2 resolution.
- v37->v38: `llm_calls.system_prompt_bytes INTEGER` column (mika#1217). Per-call assembled-system-prompt byte count for context-budget observability. Additive nullable ALTER TABLE with `column_exists` guard, mirroring v30→v31 shape. Pre-v38 rows stay NULL; new rows populated by `save_llm_call()` from the helper `emit_system_prompt_assembled` in `agent.rs`. Companion INFO log event `system_prompt_assembled` emitted per turn at all three entry points (conversation, silent, team).
- v38->v39: `operational_items` table (mika#1262). Canonical operational-item ledger for the What's Next engine. Seven `kind` variants (`goal`/`task`/`commitment`/`decision`/`blocker`/`evidence`/`next_action`), six `status` variants (`now`/`waiting`/`delegated`/`scheduled`/`at_risk`/`done`), four `owner_type` variants (`user`/`mika`/`person`/`agent`). Source-based dedup via `UNIQUE(agent_id, source_table, source_id)` partial index. `evidence_refs` stored as JSON TEXT with Rust-type-as-only-writer contract (foundation Decision F). Terminal status transitions (`Done`) require `EvidenceRef` via `complete_operational_item()` (foundation Decision G); `NonTerminalStatus` enum provides compile-time guard against passing `Done` to `update_operational_item_status()`. Writes always-on; HTTP read API (`GET /api/v1/operational-items`) gated behind `MIKA_OPERATIONAL_PARTNER=1`. Rust types and write paths in `crates/mika-agent/src/operational/`. DB methods in `crates/mika-agent/src/db/operational.rs`.
- v43->v44: `permission_decisions` table (mika#1733 AC4). Additive `CREATE TABLE` + two indexes (`request_id`, `created_at DESC`); no rebuild of existing tables. Records every operator permission decision routed through `PermissionsChannel::resolve_decision`: `classifier_verdict` (approved/denied/held), `operator_decision` (approve/deny, nullable), `override_used` (0/1), `decision_authority` (strict/override), `tenant_id`, `agent_id`, `created_at`. `override_used = 1` iff `classifier_verdict = 'denied' AND operator_decision = 'approve' AND decision_authority = 'override'` — derived by the resolver, never wire-carried. `AsyncDatabase::insert_permission_decision` writes on a `tokio::spawn` so the classifier oneshot is never blocked by DB latency (AC4 ordering).
- v44->v45: `pilot_transcripts` table (mika#1705). Additive `CREATE TABLE` + two indexes (`task_id`, `created_at DESC`). Captures LLM-call transcripts emitted by claude-pilot subprocesses (the implementation-reasoning corpus, ~90% of the trajectory that in-process `llm_calls` never sees). Rows ingested by the engine tick from `~/.mika/data/pilot-transcripts/<task-id>.jsonl` files. `insert_pilot_transcripts_batch` is atomic per source file — either every row commits or none does.
- v45->v46: `served_content` table (mika#1867). Additive `CREATE TABLE` + two indexes (`(agent_id, person_id, category, served_at DESC)`, `(agent_id, person_id, content_hash)`). Per-(agent, person, category) content-serve ledger for `proverb`/`quote`/`joke`/`poem`/`recommendation`/`story`/`fact` classes. Prevents Mika from re-serving the same proverb/joke/etc. to the same person across weeks or months. Founding incident: Al (Vietnam tester) 2026-07-28 — same zen proverb served twice, 6 days apart, because history-fetch is global-recency (not per-user). Idempotency at the SQL layer: `UNIQUE(agent_id, person_id, content_hash)` + `ON CONFLICT DO NOTHING` in `Database::record_served_content`. `content_signature TEXT` column reserved (NULL in v1) for v2 fuzzy dedup (embedding cosine similarity per AC6). No `audit_events` writes on record — high-volume low-signal; `warn!(event = "served_content.duplicate_write", ...)` on duplicate detection is the sole observability surface (AC10). Backward compat: reads return empty for rows pre-migration (agent that has never served anything = no dedup, safe direction per AC1). Two engine-level tools (`record_served_content`, `check_already_served`) exposed to every agent — a fidelity gate is engine, not skill. Bundled skill `content-request-fidelity` is the classifier layer (bilingual FR/EN keyword triggers + `[constraints] required_tools = ["check_already_served"]`). v1 scope: 1:1 conversations only; team-channel dedup deferred.
- v46->v47: Table-rebuild on `team_runs` (mika#1676). Expands the `status` CHECK constraint to include `'failed_no_delegation'` — the terminal state Unit A's delegation gate transitions to when the orchestrator returns a `Conversational` decompose for an actionable goal (after one reinforced retry). Adds three columns for Unit B observability: `delegation_count INTEGER NOT NULL DEFAULT 0` (incremented per spawned member session in `execute_tasks()`), `solo_absorption INTEGER NOT NULL DEFAULT 0` (flag set by `finalize_and_shutdown()` when the run completed with zero delegations), `failure_context TEXT` (nullable JSON `{"phase": "first_decompose" | "revision_after_critic"}` carrying the phase the gate fired in). Follows the v11→v12 CREATE-INSERT-DROP-RENAME sequence — NOT the v34→v35 RENAME-first sequence — because `tasks.team_run_id REFERENCES team_runs(id)`: a RENAME-first sequence would let SQLite silently retarget the child FK to the backup table and then orphan it after the DROP (every subsequent INSERT INTO tasks would fail "no such table: team_runs_v46_backup").
- v47->v48: **Behavioral marker only — no DDL** (mika#1712). Anchors the phantom NULL-PID sweep semantics added in `TaskEngine::sweep_null_pid_phantoms` (AC3 watchdog tick) and the equivalent startup step in `TaskEngine::startup_recovery` (AC5). Reserved for future DDL if write-time enforcement (mika#1934 cause-racine) ever lands.
- v48->v49: Expand the `team_runs.status` CHECK constraint to include `'failed_transport'` (mika#1671 D3). Composes independently with mika#1676's v46→v47 `failed_no_delegation` addition — the two terminal states cover different failure classes (all-transport-failed short-circuit vs zero-delegation gate). Table rebuild because SQLite cannot alter a CHECK in place. Uses the **build-new-then-swap** shape (CREATE `team_runs_new` → INSERT SELECT → DROP `team_runs` → RENAME `team_runs_new` → `team_runs`), NOT the rename-to-backup shape used by KG-table rebuilds (v34→v35): `team_runs` is FK-referenced by `tasks.team_run_id`, and renaming the referenced table first makes SQLite (default `legacy_alter_table = OFF`) rewrite `tasks`'s FK target to the backup name. `foreign_keys` toggled OFF around the rebuild. The rebuilt table carries forward the v46→v47 columns (`delegation_count`, `solo_absorption`, `failure_context`) and both extended CHECK values. Backs the new `RunStatus::FailedTransport(String)` terminal variant — the all-delegations-transport-failed short-circuit that fails a team run fast instead of blocking to the full timeout.

- v49->v50: Additive re-drive accounting on `auto_pull_stats` (mika#2020) — three columns: `redrive_count INTEGER NOT NULL DEFAULT 0`, `last_redrive_at TEXT`, `redrive_abandoned_at TEXT`. Three `ALTER TABLE ADD COLUMN` statements, no table rebuild (nothing FK-references `auto_pull_stats`). Backs the per-ticket re-drive budget that bounds the Phase 2 stuck-ready reconciler. The columns exist because `failure_count` cannot serve this purpose: it means "the `gh` call failed" and `reset_auto_pull_failure` runs on **every** successful rescue and promotion — precisely the event the budget must count. Merging the two would rebuild the mika#1901 defect (16 `ready` re-applications in 19 h, each one zeroing the only counter that existed). `redrive_abandoned_at` is what separates "the budget just ran out, the label is not posted yet" from "the budget ran out earlier and the operator has since removed the hold label" — the latter being the re-entry gesture. **mika#2361 added no column and no migration**; it gave that same column a second reader (`get_auto_pull_redrive_abandoned_at`, the *instant* rather than the boolean) to bound a per-abandonment comment, and it corrected the vocabulary above: the gesture is removing the **hold label** — `operator-review`, `blocked` or `operator-gated` — not `operator-review` specifically, and not re-applying `ready`, which lifts nothing. See root `CLAUDE.md` § *auto-pull stuck-ready reconciler* for the `operator_review_or_blocked` / `abandoned_operator_held` pair and its operator surfaces.
- v50->v51: `tasks.dispatcher_source` + the `dispatch_slot_leases` table (mika#1948, Porte 2). The column is nullable and CHECK-pinned to `mika_dev`/`mika_manager`/`operator`; the table makes an assigned exec slot an observable fact, one row per `(agent_id, dispatch_class)`. (Entry backfilled in mika#2160 — the list skipped it.)
- v51->v52: `dispatch_slot_leases` rebuilt with `slot_index INTEGER NOT NULL DEFAULT 0` joining the PRIMARY KEY (mika#2160). Table rebuild — SQLite cannot extend a PRIMARY KEY in place; existing rows land at `slot_index = 0`, so a database migrated mid-dispatch keeps its live lease and its holder. **Why it had to move:** `PRIMARY KEY (agent_id, dispatch_class)` was itself a hard cap of one, independent of any predicate — a class could not hold two live leases whatever the TTL or the guard decided. Without this, `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT=2` would be accepted, logged, and have no effect. At the default cap of 1 exactly one index is ever written and the behaviour is the pre-v52 behaviour, bit for bit.
- v52->v53: `llm_calls.request_bytes INTEGER` (mika#2189). Additive nullable `ALTER TABLE` with a `column_exists` guard, the same shape as v37→v38's `system_prompt_bytes` and for the same family of reason. **What it closes:** mika#2189's AC1 asks for the expiry distribution "by brief size", but the **error** path writes `input_tokens = 0` and `output_tokens = 0` as literals — a failed call carried no size at all, so the request that timed out was unrecoverable after the fact. Written on **both** paths of `run_loop` and on all three arms of the continuation turn (success, provider error, deadline-clamp timeout); writing it on the success path only would reopen the hole on exactly the side that matters. An estimated `input_tokens` was rejected (Q2): the axis exists to *correlate* size with expiry, and a correlation built on an estimate cannot settle anything. The value comes from `LlmRequest::payload_bytes()`, which sums caller-supplied bytes (system prompt, message text, tool-call arguments, tool-result content, base64 image data, tool definitions) and deliberately **excludes** the provider-specific JSON envelope so the number stays comparable across rails — a lower bound on wire size, monotonic rather than exact. KG extraction and resolution write `None`: a batch NER call runs under no agent envelope, so the axis has nothing to correlate there. Non-retroactive — pre-v53 rows read NULL, which is why the column has no `DEFAULT 0` that would be indistinguishable from a genuinely empty request.

Full migration history: see `docs/runtime-structure.md`.
