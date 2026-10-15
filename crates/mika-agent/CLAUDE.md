# mika-agent — Agent Engine

Agent container: SQLite DB, agent loop, tools, prompt assembly, A2A server endpoints, HTTP server binary (mika-spirit). This is the core crate where most development happens.

## Agent Loop

Reference moved to [`docs/agent-internals/agent-loop.md`](../../docs/agent-internals/agent-loop.md) (91 KB). Read it before changing this subsystem, and document new material there, not here.

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

### `[context.history]` is a role FLOOR, narrowed per tenant (mika#2425)

`scope` (`agent` | `session`) and `max_tokens` bound the conversation window
(mika#2295). Since mika#2425 the identity's values are a **floor**: two
`customer_config` keys — `context_history_scope`, `context_history_max_tokens` —
may make the window narrower for one agent and **never wider**. `session` always
wins; the effective ceiling is the `min`, with `None` as infinity.

`agent_loop::context_history` owns the cascade and is its **only** site. Three
guards hold that, and each covers a class the others cannot see:

- `mika2425_identity_context_history_has_a_single_reader` — a source scan,
  allowlist shipped **empty**. A second production site reading
  `identity.context.history` would apply the floor **without** the narrowing,
  silently, with every behavioural assertion still green (the `grooming_marker`
  class, mika#2158). Its good-faith control is
  `mika2425_the_reader_scan_reddens_on_a_second_reader`; note the predicate
  anchors on non-comment lines, because prose *about* the field is not a read and
  the decision site carries three paragraphs of it (the mika#2050 Signal S shape).
- `mika2305_the_scope_has_a_single_decisional_reader` — **amended, not excepted**:
  the count goes `2 → 3` and the path constraint relaxes from `agent_loop/mod.rs`
  to `agent_loop/`. The resolver is not a second answer to *"which rows may this
  window draw from?"*; it **produces** the answer `scoped_session_id` consumes.
  The resolver's `match` keeps its three arms on one line each, deliberately: a
  rustfmt reflow splits it into two sites and the guard reads that as two
  competing answers.
- `tests/eval/test_context_history_per_tenant_2425.rs` — the behavioural half.
  Neither source scan can see a decision site that reads the *right* field and
  simply never consults the database, so only a real turn with a real row closes
  it. Four cases: the narrowing reaches the window, the negative control without
  the row, a refused widening (said as well as refused), and the neutral
  cancelling a narrowing on the next turn with no restart.

Both keys are **absent from `SETTABLE_CONFIG_KEYS`** — that constant is the
`set_config` tool surface, so the omission is the whole refusal, and it is
asserted (`mika2425_the_db_half_is_absent_from_the_tool_surface`). `0` is refused
for the ceiling with a floor of 500: `Some(0)` is the omission sentinel that
empties the history, a role decision carried by the identity.

Operator surfaces (`context_history_resolved`, `context_history_widening_refused`),
the reading table, the five probes and the **R3 halt** — on a non-singleton agent
a "session" *is* a message, so `session` does not trim the window, it empties it
— live in the root `CLAUDE.md` § *La fenêtre de conversation est réglable par
tenant*.

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

Reference moved to [`docs/agent-internals/tools.md`](../../docs/agent-internals/tools.md) (160 KB). Read it before changing this subsystem, and document new material there, not here.

## Milestone Manager (Phase 1)

Reference moved to [`docs/agent-internals/milestone-manager-phase-1.md`](../../docs/agent-internals/milestone-manager-phase-1.md) (27 KB). Read it before changing this subsystem, and document new material there, not here.

## Skills System

Reference moved to [`docs/agent-internals/skills-system.md`](../../docs/agent-internals/skills-system.md) (35 KB). Read it before changing this subsystem, and document new material there, not here.

## Exec Handlers

Reference moved to [`docs/agent-internals/exec-handlers.md`](../../docs/agent-internals/exec-handlers.md) (61 KB). Read it before changing this subsystem, and document new material there, not here.

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

`.github/workflows/eval-calibration.yml` — On-demand workflow (weekly cron removed, see **Schedule** below) that keeps the committed baseline (`tests/fixtures/eval-baseline.json`) trustworthy by detecting provider tolerance drift.

**Schedule: `workflow_dispatch` only — the cron was removed by mika#2683 (D2).** It had failed 17/17 runs from 2026-06-22 to 2026-10-12 (`--ignored` and `--include-ignored` passed together, masked by `|| true`), so it never produced an artifact, and each Monday its red `check_suite` on `main` cost mika-dev an LLM turn. Fixing the flag alone would have turned "fails before any call" into "calls four real providers every Monday" (~$2.57/run, ~$150/yr) — a recurring spend, hence an operator decision. The flag **is** fixed; rearming the cron is three lines of YAML, documented in the workflow's own header, and gated on that decision being recorded on mika#2683. Until then, provider-tolerance drift detection is on demand.

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

**Cost:** ~$2.57/run (four providers × full scenario matrix). Annual **if the cron is rearmed**: ~$150 (52 weekly + ~10 manual runs) — that figure is the budget decision mika#2683 defers to the operator. With `workflow_dispatch` only, the spend is ~$2.57 per deliberate run. Bounded by 30-minute workflow timeout.

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

**`[kg]` is operator-owned by DECISION, so the code's declaration never reached an already-provisioned agent (mika#1833).** mika#1833 asked when mika-dev's and mika-qa's identities had been *overwritten* to carry `enabled = true` against the topology above. **Nothing overwrote them — it was never written.** The code does declare `enabled = false` for mika-dev, mika-qa and mika-test, and `enabled = true` for mika-arch (`well_known_agents.rs`); but `CODE_OWNED_IDENTITY_SECTIONS` does **not** contain `kg`, and its doc-comment says so in as many words (*"Sections NOT listed here are preserved verbatim from the on-disk file (operator-owned: `name`, `emoji`, `[reflection]`, `[kg]`)"*), while `write_default_if_missing` never rewrites an existing `identity.toml`. The declaration is therefore **inert for any agent already provisioned**: mika-dev and mika-qa were provisioned before #800 made `enabled = false` the declared value, and no path has retro-applied the decision since. Exactly the class mika#2327 → mika#2330 had to name for `[context.history]` — made code-owned, merged, and inert in production for the same reason, one section away.

Two corollaries an operator needs. **A hand-written `enabled = false` survives every restart**, precisely because the section is operator-owned — good for safety (the 2026-07-26 palliative still holds today), bad for visibility (it will never lift by itself, and nothing reports it; `kg_budget_resolved`'s `kg_enabled` field is what makes it readable). And **the real re-activation perimeter is narrower than the ticket suggests**: by topology #800 mika-dev and mika-qa must *stay* disabled, `mika` is already active (`DEFAULT_IDENTITY` carries its `[kg]` block commented out, so `enabled` takes its `true` default), so the only agent where the palliative contradicts the declared topology is **mika-arch**. The sequenced gesture and its preconditions live in the root `CLAUDE.md` § *Ré-activer le KG sur mika-arch* — **no line of mika#1833 performs it.**

Adding `kg.enabled` to `CODE_OWNED_IDENTITY_SECTIONS` would make the declaration bite, and is deliberately **not** done: it would overwrite that palliative at the next startup **and re-enable mika-arch** (where the code declares `true`) without anyone having asked — a safety call dressed as consistency. Precondition for opening it: the root `CLAUDE.md` probes green after a *manual* re-activation of mika-arch.

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

**Budget guard (#757, zero-budget contract refined by mika#1833):** `extract_pending(budget: u32)` caps per-batch LLM calls. Above zero, behaviour is unchanged: on overflow it emits `kg_budget_exhausted` WARN with `scope="extraction"` and leaves remaining docs pending. Stats carry `aborted_budget: bool` + `llm_calls: u32`. Default budget: `MIKA_KG_BATCH_BUDGET` (500).

`budget == 0` short-circuits with zero calls **before `get_pending_docs`** — so the pending-detection query is no longer paid for nothing — and leaves `aborted_budget` **false**: "exhausted" describes a budget consumed on the way, not a phase the operator disarmed, and the `kg_budget_exhausted` WARN the old path emitted here said the opposite of what was happening. One INFO line, `kg_phase_disabled_by_zero_budget`, is the positive control for that silence. The predicate is read through `kg::budget::phase_is_disabled`; the Entity Resolver's budget guard below carries the full contract, what it removed, and the lever that replaces it.

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

**Budget guard (#757, zero-budget contract rewritten by mika#1833):** `resolve_pending(budget: u32)` caps per-batch **Stage-2** LLM disambiguation calls. Above zero, behaviour is unchanged: Stage-1 exact matches cost no LLM calls and are NOT debited against the budget, so a tight budget still lets them through; on overflow it emits `kg_budget_exhausted` WARN with `scope="resolution"` and the remaining entities stay pending (no `kg_resolutions_log` row written). Stats carry `aborted_budget: bool` + `llm_calls: u32`. Default budget: `MIKA_KG_BATCH_BUDGET` (500).

**`budget == 0` is a declared no-op (mika#1833).** `resolve_pending(0)` and `resolve_doc_entities(_, _, 0)` return `ResolutionStats::default()` **before** `get_pending_entities` — so before the correlated subquery on `kg_chunk_subjects` that carried essentially all of the measured 26-48 s per tick — and leave `aborted_budget` **false**. The predicate has a **single reader**, `kg::budget::phase_is_disabled`, whose doc-comment carries the reasoning; a source scan (`mika1833_the_zero_budget_predicate_has_a_single_reader`, allowlist shipped empty, pinned empty by a sibling test) refuses a second site under `src/kg/`. When it fires, route the site through the reader — do not add a line (doctrine mika#2201).

**What that removed, and why — because this entry asserted the opposite.** Until mika#1833 it read *"even `budget=0` lets exact matches resolve (selection uses an effective minimum of 50 entities)"*, a behaviour added deliberately by the #757 review (finding P1). It is what made the founding livelock possible, and two measurements condemn it. *(a)* It was **already not** delivered to the modal population: the Stage-1 threshold is `> 0.9` **strict**, so an exact match at the modal extraction confidence `0.9` escalates to Stage-2, where a zero budget discards it as `SkippedBudget` — which `continue`s **without calling `apply_result`**, hence writes no `kg_resolutions_log` row, hence the same `ORDER BY e.id ASC LIMIT 50` head of queue returns on the next tick. Measured 2026-07-26 on every well-known agent, every tick: `pending: 1288-1493`, `resolved_in_tick: 0`, `duration_ms: 26000-48000`, `aborted_budget: true`, `llm_calls: 0`. A deterministic livelock, and **the domain graph did not need to be empty to produce it** — which is where the ticket's own hypothesis went wrong. *(b)* The intention "exact matches only, no LLM" already had **its own lever, and that one terminates**: configure no resolution model, so `self.llm == None` — matches above the threshold resolve, everything else drains as `skipped_no_llm` (see **Failure policy** below), and the queue empties. Two spellings for one intention, one of which did not terminate: mika#1833 removed that one.

**Cost of the change, named rather than silent.** An operator who had set `MIKA_KG_BATCH_BUDGET=0` to obtain exact-match-only mode loses that behaviour — `0` now disarms the phase whole. The remedy is the lever above: leave `MIKA_KG_RESOLUTION_MODEL` and `MIKA_KG_INGESTION_MODEL` unset.

**Positive control, because a disarmed phase must not be mute.** Both tick phases still emit their completion line under a zero budget, carrying `skipped_reason: "zero_budget"` and their pending field **absent** — `pending_before` on `kg_resolver_tick.complete`, `total_pending` on `kg_extraction_tick.complete` (two distinct field names, each the one that phase already used; grepping for the other returns nothing). Absent means `null` in JSON, **never `0`**: the count did not happen, it did not return zero (motif mika#2331). Without that line, "the tick runs and does nothing" and "the tick does not run" would return identical bytes (class mika#2205). The budget actually in force is said once per agent per startup by `kg_budget_resolved` (`budget`, `budget_source` ∈ `{config, default}`, `kg_enabled`, `resolution_armed`, `extraction_armed`, `resolution_model_configured`), on **every** branch including the healthy one — *a setting you cannot observe is not a setting, it is a hope* (mika#2293). `MIKA_KG_BATCH_BUDGET=0` had been in force on the host for two months with no line saying so, and that silence is what sent the diagnosis to the domain graph instead of to the configuration. Its **absence** means the binary being served predates the fix (class mika#2340), never "the budget is fine".

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

Reference moved to [`docs/agent-internals/unified-task-engine.md`](../../docs/agent-internals/unified-task-engine.md) (97 KB). Read it before changing this subsystem, and document new material there, not here.

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

Reference moved to [`docs/agent-internals/observability.md`](../../docs/agent-internals/observability.md) (20 KB). Read it before changing this subsystem, and document new material there, not here.

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
| `db/migrations.rs` | `migrate` + every `migrate_vN_to_vM`, and their tests | **append-only**: a migration is never deleted (v1…v56, for ever) |
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

Reference moved to [`docs/agent-internals/schema-version.md`](../../docs/agent-internals/schema-version.md) (20 KB). Read it before changing this subsystem, and document new material there, not here.
