---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
origin: github-issue senara-solutions/mika#1813
created: 2026-08-19
---

# fix(agent-behavior): Respect user stop-signals — Mika stops re-nagging after "arrête" (#1813)

## Summary

Al (family testeur, 2026-07-20) reported that Mika keeps re-raising a subject (web config) even after he tells her « arrête ». The user has to repeat "stop" several times before she disengages. This plan closes that gap through a hybrid **state + prompt** fix: a `stop_topic_*` preference prefix (state) plus prompt discipline that instructs the agent to persist a stop-signal on refusal (conversation prompt) and to consult the stopped-topics list before initiating any proactive topic (silent prompt). The state layer is the structural gate — even if the model forgets, the `<stopped-topics>` block re-appears on every future turn.

---

## Problem Frame

**What breaks:** After a user tells Mika to stop bringing up subject X, subsequent heartbeat/reminder/callback silent turns re-inject the same subject into the LLM's context via the `<commitments>` block (`db.list_commitments("pending")`, loaded at every silent turn from `agent_loop/mod.rs:3456`). The heartbeat trigger explicitly encourages proactive send_message ("If there is something timely and worthwhile to share, use send_message" — `agent_loop/mod.rs:3517`). Nothing filters commitments the user has explicitly refused, and nothing captures a session-scoped or durable "don't re-raise this" signal.

**Why the current design allows it:**
- The commitments table has no "user refused to be re-raised" bit.
- The `Preference` structure exists and is loaded in silent mode for `task_policy_*` (`agent_loop/mod.rs:3634`), but no convention exists for stop-topics.
- The system prompt (`prompt.rs:637-643`) has a "Confirmation before action" rule but it targets informational questions only, not stop-refusals.
- No prompt rule tells the agent to `store_fact(category='preference', ...)` when receiving a stop.

**Not a confabulation.** Distinct from mika#1784 (self-model bug). This is a defect of user-signal respect, not fabrication.

**Bearing (from ticket):** Mika = présence tenue, pas insistance. Respect du "stop" est structurel : quand l'user dit non, on ne re-tente pas sur le même axe.

---

## Requirements

- **R1.** When the user says stop on subject X (words like "stop", "arrête", "don't bring this up", "no more", "assez"), the agent MUST acknowledge concisely AND persist the stop-signal in the same turn via `store_fact(category='preference', ...)` with a `stop_topic_<slug>` key.
- **R2.** On any subsequent silent-mode turn (heartbeat/reminder/callback), the agent MUST see the active stop-topics list injected as a dedicated `<stopped-topics>` block and MUST NOT re-initiate any proactive send_message on a matching topic.
- **R3.** On any conversation turn, the agent MUST see the active stop-topics list (defense-in-depth for the case where the same session continues to nag).
- **R4.** A stop on subject X MUST NOT suppress proactivity on any other subject Y (no leak).
- **R5.** A direct user question about a stopped subject X MUST still be answered (stop = do not re-initiate; direct query = respond normally).

---

## Product Contract preservation

No prior brainstorm/Product Contract exists — this plan is `product_contract_source: ce-plan-bootstrap`. The ticket body is the authoritative product input; its acceptance criteria are transcribed verbatim below.

---

## Key Technical Decisions

### KTD1. Hybrid approach: state layer + prompt layer, not prompt-only

**Choice:** Combine a durable state signal (`stop_topic_*` preference prefix, injected as `<stopped-topics>` block) with prompt discipline (conversation prompt tells the agent to persist the stop; silent prompt tells the agent to consult the list before initiating).

**Rationale:** `feedback_prompt_enforcement_fragile` — prompt-only rules erode under context weight. The state layer is the structural gate: even if the model forgets, the `<stopped-topics>` block re-appears on every future turn. Preferences (per `crates/mika-agent/CLAUDE.md` § Three-Layer Memory Model) are Layer 2 (structured facts, plaintext, DB-backed) — durable across sessions, session-continuity-preserving.

**Alternatives rejected:**
- Pure prompt-layer: fragile under load; no re-inforcement on subsequent turns.
- Pure state-layer via a new `commitments.suppressed` column: requires DDL migration, ontology drift into `commitments` (semantically wrong — the user isn't cancelling the commitment, they're refusing re-mention), and doesn't cover proactive suggestions that were never a "commitment" in the first place. Preferences generalise better.

### KTD2. `stop_topic_` key prefix, not a new category

**Choice:** Reuse the existing `preferences` table and the existing "prefix convention" pattern established by `task_policy_*` (`agent_loop/mod.rs:3634`). Keys are `stop_topic_<short-slug>`; values carry human-readable description + timestamp.

**Rationale:** Zero DDL change, zero new tool. The `store_fact(category='preference', ...)` tool already exists and works. The `search_preferences("stop_topic_")` query pattern mirrors `search_preferences("task_policy_")` and requires only the same wiring at load sites. Preserves orthogonality (per `docs/architecture/review-guide.md`).

### KTD3. Inject stop-topics into BOTH conversation and silent prompts

**Choice:** Load stop-topics in `load_agent_context` and inject into both `build_system_prompt` (conversation) and `build_silent_prompt` (silent).

**Rationale:** The ticket surface is heartbeat/callback re-nagging, but the same failure mode can occur mid-conversation (Al's original report was a chat thread — he said "stop", she came back on it in the same conversation). Both surfaces need the block. Injection cost is bounded (typically 0-5 short lines).

### KTD4. Dedicated `<stopped-topics>` block, not merged into `<stored-preferences>`

**Choice:** Add a new `<stopped-topics>` block placed before `<commitments>` in the silent prompt, and a matching section in the conversation prompt. Do NOT overload the existing `<stored-preferences>` block.

**Rationale:** Semantic distinctness aids the model. Stop-topics are a **suppression** directive; task_policy preferences are an **autonomous-action** directive. Rendering them in a distinct block with distinct framing ("DO NOT re-initiate on these topics") is clearer than an unlabeled union. Also keeps the two searches decoupled (they use different query patterns and can evolve independently).

### KTD5. Test discipline: MockLlmProvider + failing-first regression fixture

**Choice:** Add unit tests for the prompt rendering (deterministic, no LLM). Add at least one agent-loop eval test using `MockLlmProvider` from `mika-common::llm::mock` that demonstrates the pre-fix pattern (assert the mock's canned response would trigger a re-nag on next silent turn) versus the post-fix pattern (assistant is now seeing `<stopped-topics>` and won't re-raise).

**Rationale:** `feedback_verify_pipeline_passes_without_the_fix` — regression-fixture tests prove the assertions catch the failure class. `feedback_never_call_real_broadcast_in_tmux_test` — no real notifications. MockLlmProvider is the canonical harness for deterministic scenarios.

---

## Acceptance criteria

Transcribed verbatim from ticket #1813:

- [ ] AC1. Test scenario : user dit "stop [sujet]" → Mika acknowledge → aucun re-mention du sujet dans les N tours suivants (sauf si l'user le rouvre)
- [ ] AC2. Distinction préservée : stop = ne pas ré-initier ; question de l'user sur le sujet = OK répondre
- [ ] AC3. Al re-teste : ré-envoi un « arrête » → Mika ne relance plus
- [ ] AC4. Aucun leak dans les autres axes (Mika reste proactive sur les autres sujets non stopped)

---

## Definition of Done

- All unit tests pass (`cargo test -p mika-agent --lib prompt`)
- All eval tests pass (`cargo test -p mika-agent`)
- Clippy clean (`cargo clippy --workspace --all-targets -- -D warnings`)
- Format clean (`cargo fmt --check`)
- At least one regression-fixture test demonstrates: same MockLlmProvider script → post-fix, `<stopped-topics>` block is present in the assembled prompt; pre-fix, block is absent.
- PR body includes `Closes #1813` and WHY-first framing.
- No hooks skipped, no `--no-verify`.

---

## Implementation Units

### U1. Load stop-topics in `AgentContext` (state wiring)

**Goal:** Extend `AgentContext` to carry `stopped_topics: Vec<Preference>`. Populate in `load_agent_context` via `db.search_preferences("stop_topic_")`.

**Requirements:** R2, R3.

**Dependencies:** none.

**Files:**
- `crates/mika-agent/src/agent_loop/mod.rs` — add `stopped_topics` field to `AgentContext`; call `db.search_preferences("stop_topic_")` in `load_agent_context`.

**Approach:**
- Add `stopped_topics: Vec<crate::db::Preference>` to the `AgentContext` struct at line ~205.
- In `load_agent_context` at line ~212, add `let stopped_topics = db.search_preferences("stop_topic_").await.unwrap_or_default();` — fail-open (if the query errors, we don't want to block the turn).
- Include in the return struct.

**Patterns to follow:** the existing pattern in `run_silent_inner` (line 3634) where `stored_preferences` uses `.unwrap_or_default()`.

**Test scenarios:** covered by U4 unit tests (verifying the field is loaded and threaded to prompt builders); no unit test for `load_agent_context` itself (async DB helper — covered by higher-level eval tests).

**Verification:** compiles; downstream consumers can read `ctx.stopped_topics`.

### U2. Inject `<stopped-topics>` block into silent-mode prompt

**Goal:** Add a `stopped_topics` field to `SilentPromptContext`. Emit a `<stopped-topics>` block in `build_silent_prompt` when non-empty, placed BEFORE the existing `<commitments>` block. Add a rule-line to the "Silent Mode" instructions telling the agent not to initiate on any stopped topic.

**Requirements:** R2.

**Dependencies:** U1.

**Files:**
- `crates/mika-agent/src/prompt.rs` — add `stopped_topics: &'a [Preference]` field to `SilentPromptContext` (line ~922); emit block in `build_silent_prompt` (line ~972); add prompt rule.
- `crates/mika-agent/src/agent_loop/mod.rs` — thread `ctx.stopped_topics` into the `SilentPromptContext` at line ~3642.

**Approach:**
- Field: `pub stopped_topics: &'a [Preference]`.
- Block: placed after core-memory / preferences area, before `<commitments>`. Uses the same `sanitize_label` helper for injection safety. When empty, block is omitted entirely (avoid empty XML noise).
- Rule text: "**Respect stop signals.** Before initiating any proactive `send_message`, check `<stopped-topics>`. If the topic you would raise matches an entry (subject substring or slug), DO NOT re-raise. Direct user questions about a stopped topic are still fine to answer."
- Update all `SilentPromptContext { ... }` literal constructions in tests to pass `stopped_topics: &[]`.

**Patterns to follow:** the existing `<stored-preferences>` block at prompt.rs:1109-1117.

**Test scenarios:**
- `test_silent_prompt_stopped_topics_block_present_when_nonempty`: build with one stop_topic → block appears in prompt; contains category + value; contains the "Respect stop signals" rule.
- `test_silent_prompt_stopped_topics_block_absent_when_empty`: build with `&[]` → block does not appear.
- `test_silent_prompt_stopped_topics_sanitized`: build with a category/value containing `<script>` and newlines → sanitized in output.
- Covers AC1 (state-layer visibility on silent turns), AC3 (repeat "arrête" still lands in the block).

**Verification:** `cargo test -p mika-agent --lib prompt::tests::test_silent_prompt_stopped_topics` passes.

### U3. Inject `<stopped-topics>` block into conversation-mode prompt

**Goal:** Add `stopped_topics` field to `PromptContext`. Emit block in `build_system_prompt`. Add a rule-line to the "Instructions" telling the agent to persist a stop-signal when the user asks to stop a topic, AND to respect an existing `<stopped-topics>` list.

**Requirements:** R1, R3.

**Dependencies:** U1.

**Files:**
- `crates/mika-agent/src/prompt.rs` — add `stopped_topics: &'a [Preference]` field to `PromptContext`; emit block in `build_system_prompt`; add two prompt rules (persist on stop; respect existing block).
- `crates/mika-agent/src/agent_loop/mod.rs` — thread `ctx.stopped_topics` into `PromptContext` at line ~2745.

**Approach:**
- Field on `PromptContext`.
- Block: placed after `## Core Memory` and before `## Instructions`, with framing "## Stopped Topics — user has asked you not to re-raise these".
- Rules added to the `## Instructions` block:
  - "**Respect stop signals (persist):** When the user asks you to stop bringing up a topic (words like 'stop', 'arrête', 'don't bring this up', 'no more', 'assez', 'laisse tomber', 'oublie ça'), acknowledge concisely AND call `store_fact(category='preference', key='stop_topic_<short-slug>', value='<one-line: what the user asked to stop, with today's date>')` in the SAME turn. The `<short-slug>` is a lowercase kebab-case identifier for the subject (e.g., `web-config`, `budget-review`). Do NOT re-initiate on this topic in later turns unless the user re-opens it themselves. A direct user question about the topic is not a re-opening — you may still answer it."
  - "**Respect stop signals (consult):** If a `<stopped-topics>` block is present above, do NOT proactively re-raise any listed subject. Answer direct questions normally."
- Update all `PromptContext { ... }` literal constructions in tests to pass `stopped_topics: &[]`.

**Patterns to follow:** the "Confirmation before action" rule at prompt.rs:637-643.

**Test scenarios:**
- `test_conversation_prompt_stopped_topics_block_present_when_nonempty`: block appears; contains the two rule texts.
- `test_conversation_prompt_stopped_topics_block_absent_when_empty`: no block emitted.
- `test_conversation_prompt_persist_stop_rule_present`: the "persist" rule with `store_fact(category='preference'` text is present in the assembled prompt.
- Covers AC1, AC3 (both flavours: persist on refusal, respect on later turns).

**Verification:** `cargo test -p mika-agent --lib prompt::tests::test_conversation_prompt_stopped_topics` passes.

### U4. Existing-test callsite updates + smoke regression for `run_silent_inner` wiring

**Goal:** Update every existing `PromptContext { ... }` / `SilentPromptContext { ... }` construction site (unit tests and callers) with the new `stopped_topics` field. Add a unit test that constructs a `SilentPromptContext` with a non-empty `stopped_topics: &[Preference]` and asserts the assembled prompt contains BOTH the stop-topic subject slug AND the "Respect stop signals" rule text — this is the load-bearing regression fixture: it fails on `main` because the field does not exist and the rule text is not emitted.

**Requirements:** R2.

**Dependencies:** U1, U2, U3.

**Files:**
- `crates/mika-agent/src/prompt.rs` — new unit test.

**Approach:**
- Enumerate all existing `SilentPromptContext { ... }` and `PromptContext { ... }` literal constructions (grep result: prompt.rs:1415, 1442, 1477, 1614, 1640, 1950, 1974, 1998, 2165, 2192, 2262, 2294, 2324, plus PromptContext callsites) and add `stopped_topics: &[]`.
- New test constructs a Preference with `category = "stop_topic_web_config"` value `"user asked to stop 2026-08-19"`, builds the silent prompt, asserts subject slug present + rule text present.

**Test scenarios:**
- `test_silent_prompt_regression_stop_topic_visible_and_rule_present`: hard-asserts BOTH signals — the exact regression fixture. Documented in-test as: "This test fails on main (stopped_topics field does not exist on SilentPromptContext), demonstrating the fix is load-bearing per feedback_verify_pipeline_passes_without_the_fix."

**Verification:** the added test runs green post-fix.

### U5. Documentation touch-up in `crates/mika-agent/CLAUDE.md`

**Goal:** Add a brief paragraph to `crates/mika-agent/CLAUDE.md` § "Three-Layer Memory Model" explaining the `stop_topic_*` preference convention and pointing to the two prompt sites that consume it.

**Requirements:** R2, R3 (docs discipline).

**Dependencies:** U1, U2, U3.

**Files:**
- `crates/mika-agent/CLAUDE.md` — one paragraph in the relevant section.

**Approach:** Add the paragraph. Keep it short (3-5 lines). No section rename.

**Test scenarios:** N/A (docs change). Test expectation: none — pure documentation.

**Verification:** file compiles as markdown; no CI docs-sync trigger (docs/ changes on `crates/mika-agent/CLAUDE.md` don't drive the `docs-sync` job).

---

## Scope Boundaries

**In scope:** the five implementation units above. State + prompt for stop-topics. Unit tests. Doc touch-up.

**Out of scope (deferred to follow-up):**
- Automatic expiry of `stop_topic_*` preferences (e.g., after N days). Ticket does not require it; can be added later if operator experience shows it's needed.
- A dashboard surface to view/edit stop-topics. Not asked for.
- Migrating the `<commitments>` block to filter by stop_topics via join. The prompt-level respect rule is sufficient; DB-level filtering can be added later if the prompt rule proves fragile.
- Adding an `update_fact` step to also cancel any matching pending commitment. Semantically distinct (user may not want the commitment cancelled — they may just not want it *re-raised*); can be added later.

---

## Verification Contract

- `cargo build --workspace` — green
- `cargo test -p mika-agent --lib prompt` — includes the new unit tests, all green
- `cargo test -p mika-agent` — full crate test suite green
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --check` — clean

---

## Sources & Research

- Ticket: `senara-solutions/mika#1813` (Al testeur report, 2026-07-20)
- Prompt assembly: `crates/mika-agent/src/prompt.rs` (build_system_prompt, build_silent_prompt, PromptContext, SilentPromptContext)
- Silent turn wiring: `crates/mika-agent/src/agent_loop/mod.rs:3450-3660` (run_silent_inner, load_agent_context)
- Preference wiring precedent: `crates/mika-agent/src/agent_loop/mod.rs:3634` (task_policy_ preference load) + `crates/mika-agent/src/prompt.rs:1109-1117` (stored-preferences render)
- DB: `crates/mika-agent/src/db.rs:8604` (search_preferences)
- Guardrails: `feedback_prompt_enforcement_fragile`, `feedback_verify_pipeline_passes_without_the_fix`, `feedback_never_call_real_broadcast_in_tmux_test`, `feedback_never_skip_ce_review`
- Memory-model doc: `crates/mika-agent/CLAUDE.md` § "Three-Layer Memory Model"
