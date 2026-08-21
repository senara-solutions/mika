---
module: mika-agent
tags: [fidelity, memory, ledger, agent-behavior, structural-gate, founding-incident]
problem_type: agent-behavior
category: best-practices
ticket: mika#1867
plan: docs/plans/2026-08-21-001-fix-1867-fidelity-per-user-content-tracking-plan.md
compound_date: 2026-08-21
---

# Per-user content-serve ledger — the structural cure for repeated content

## The founding incident

On **22 July 2026**, Al (mika tester, Vietnam) asked Mika for "un proverbe zen" via Telegram. Mika served: *"Avant l'éveil, couper du bois, porter de l'eau. Après l'éveil, couper du bois, porter de l'eau."*

On **28 July 2026** — 6 days later — Al asked again. Mika served the **same proverb**.

Al: *"Duplicate 22 juillet, tu te répètes..."*
Mika: *"Tu as raison, pardon. Celui-là, je te l'avais déjà servi. Je vais faire attention."*

The Couche Confiance (fidelity) breaks precisely there: **Mika RECOGNIZED the collision when corrected, but did not VERIFY proactively before serving**. Data-driven diagnosis revealed four root causes; this doc is about the class-fix that closes RC-A + RC-B structurally.

## The four root causes

- **RC-A — history-fetch is global-recency, not per-user.** `Database::load_recent_messages(agent_id, limit)` at `crates/mika-agent/src/db.rs::8117` filters by `agent_id + role != 'summary' + channel_type != 'team'` with `ORDER BY created_at DESC LIMIT ?`. Default limit 30-50. On an active agent, 30 messages = a few hours of traffic, not 6 days. **Between 22 and 28 July, the first serve fell out of the in-context history window.** The LLM had no memory of having served the proverb.
- **RC-B — no per-user content ledger.** `messages` has no `user_id`/`chat_id`/`correspondent_id`. `people` tracks mentions but no structured content-serves. `core_memory` is `(agent_id, key)` primary key, not `(agent, person)`. Nothing tracks "Mika served X to person Y."
- **RC-C — under-constrained requests collapse to LLM priors.** "Un proverbe zen" is maximally under-specified. LLMs have a very strong prior on canonical zen quotes ("Avant l'éveil..." is *the* most-cited zen proverb in Western contexts). Same request → same output at high probability. **Out of scope this fix** — deferred until post-deploy signal warrants.
- **RC-D — no prompt-level dedup discipline.** `prompt.rs` guides "check conversation history and search_memory" but nothing specific to content classes. Fragile per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — prompt-only fails at loop substrate.

## The structural fix

**Design principle: served content = structured fact, not LLM guess.**

Introduce a per-`(agent, person, category)` content-serve ledger + two engine-level tools + a bundled skill with `required_tools` gate.

### Schema (v45 → v46 migration, `crates/mika-agent/src/db.rs::migrate_v45_to_v46`)

```sql
CREATE TABLE served_content (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    category TEXT NOT NULL CHECK (category IN (
        'proverb','quote','joke','poem','recommendation','story','fact'
    )),
    content_text TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    content_signature TEXT,  -- reserved for v2 fuzzy dedup (AC6)
    served_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    UNIQUE(agent_id, person_id, content_hash)
);
CREATE INDEX idx_served_content_person_cat
    ON served_content(agent_id, person_id, category, served_at DESC);
```

The `UNIQUE` implicit index covers the exact-match dedup lookup; the per-cat index covers `list_served_content` window queries.

### Tools (engine-level, all agents)

- **`record_served_content(person_id, category, content, [signature])`** at `crates/mika-agent/src/tools/record_served_content.rs`. Inserts via `INSERT ... ON CONFLICT DO NOTHING`. Returns `{"status": "recorded", "id": N}` on Inserted, `{"status": "duplicate", "existing_id": N, "prior_served_at": "...", "retry_hint": "..."}` on Duplicate.
- **`check_already_served(person_id, category, [days], [content_hash])`** at `crates/mika-agent/src/tools/check_already_served.rs`. Returns last 3 serves with 200-char snippets. Optional `content_hash` filter pushed into SQL (avoids LIMIT-3-then-filter false-negatives).

Both tools require `person_id: i64` — architect F2 arbitration rejected fuzzy-name matching (silent 50% mis-attribution risk in multi-name scenarios).

### Bundled skill (`skills/bundled/content-request-fidelity/`)

Bilingual FR/EN keyword triggers. `[constraints] required_tools = ["check_already_served", "record_served_content"]` — the mika-agent EndTurn Post-Condition #3 (required-tools gate) forces both tools to be called on keyword-matched turns before EndTurn.

## The load-bearing shapes

Three architectural properties make this class-fix durable:

### 1. Structural gate, not prompt guidance

`required_tools = [check, record]` is the structural enforcement of both halves. Prompt-only "please call this tool" fails at loop substrate (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`). Putting **both** the read AND the write in the gate is what makes the ledger complete — a check-only gate would let the LLM verify absence, generate content, then EndTurn without recording (the founding incident then repeats on the next turn).

### 2. Hash normalization that resists LLM variance

An LLM regenerating "Avant l'éveil" on retry will not produce byte-identical output — NFC vs NFD accents (`é` U+00E9 vs `e\u{0301}`), typographic vs ASCII quotes (`'` U+2019 vs `'` U+0027), trailing punctuation variance (`.` vs `…` vs no punct) all produce distinct SHA-256s despite semantic identity. `compute_content_hash` at `crates/mika-agent/src/memory/mod.rs` applies:

```
NFKC normalize → strip zero-width chars (U+200B..U+200D, U+FEFF, U+2060)
  → fold typographic quotes to ASCII → trim trailing punctuation
  → to_lowercase → collapse whitespace → SHA-256
```

The founding proverb `"Avant l'éveil"` (U+2019 curly apostrophe) hashes identically to `"avant l'eveil"` (ASCII apostrophe, decomposed accent, uppercased) — the LLM cannot escape the gate by trivial variation.

### 3. Class-fix, not proverb-fix

The 7 categories (`proverb`, `quote`, `joke`, `poem`, `recommendation`, `story`, `fact`) cover the whole content-request class. One ledger, one skill, seven endpoints. Extending to an eighth category is a schema migration + one keyword group. This is the compound-engineering shape called out in the ticket body's "Class-fix" section.

## Design decisions worth citing

- **`person_id: i64` required, no fuzzy-name fallback (architect F2).** The old proposal ("WARN + first-by-id + follow-up") silently mis-attributed 50% of the time in multi-`Al` scenarios. Reject-and-force-caller-pass is the KISS path.
- **AC9 retention unbounded v1, documented growth ceiling.** Personal agent worst-case ~2-4K rows/year ≈ 200-400 KB/year — under the noise floor of the existing DB. Follow-up trigger: >10K rows/month/agent on a customer surface.
- **AC10 no `audit_events` for `record_served_content`, WARN log only.** High-volume low-signal writes would flood audit_events. Grepable event `served_content.duplicate_write` in server.log is the sole observability surface. Verified structurally by `test_ac10_duplicate_write_emits_warn_and_no_audit_event`.
- **1:1 conversations only in v1 (A6).** Team-channel content dedup deferred — team-run chatter would flood the user-facing fidelity ledger, and the operator-facing surface Al surfaced is 1:1.
- **Fuzzy dedup (AC6) deferred.** `content_signature TEXT NULL` column reserved; v1 ships hash-only. Follow-up fires if AC8 post-deploy verify shows hash-only insufficient.

## The pipeline learning

The `/ce:review` multi-agent pass on the first-cut implementation found **7 P1 blockers** — none of which any single lens would have caught alone:

1. **Tool descriptions referenced non-existent tools** (`list_people`, `upsert_person`) — the corresponding DB methods exist as private helpers but are not registered as LLM tools. Caught by `api-contract` + `agent-native` + `correctness` (triple-corroborated via `BUILTIN_TOOL_NAMES` grep). Would have caused first-attempt tool_use failures.
2. **Bare keyword `fait`** in triggers would hijack every French conversation containing "c'est fait", "en fait", "qu'est-ce que tu as fait". Caught by `agent-native` + `api-contract`. Al is francophone — the fix's founding user would be the first casualty. mika#1650 class collision.
3. **`record_served_content` not in `required_tools`** — only `check_already_served` was gated. LLM could call check, generate content, EndTurn without recording. Founding incident repeats. Caught by `adversarial`.
4. **Hash normalization only case+whitespace** — NFC/NFD, curly quotes, trailing punct all escape. Caught by `adversarial`. Al's `l'éveil` proverb hits this exact escape.
5. **RC-A per-user filter test missing** — every DB test used a single `person_id`. Removing `AND person_id = ?2` from `list_served_content` would leave all tests green. **The founding-incident invariant was unprotected.** Caught by `testing`.
6. **Bilingual matcher test hardcoded keywords** — didn't load from the shipped `skill.toml`. Silent drift possible. Caught by `testing`.
7. **`check_already_served` LIMIT-then-filter false negative** — `content_hash` filter applied client-side after `LIMIT 3` returned false "not served" for hashes older than the top-3. Caught by `adversarial` + `correctness` + `agent-native`.

**The generalizable pattern:** a fix that works at the DB layer can still be broken at the LLM surface (tool descriptions, prompt guidance, keyword triggers, gate composition). Ledger-class fixes need review of both the persistence layer AND the LLM-facing surface AND the required-tools gate composition. A single lens cannot cover this — the multi-agent review earned its cost here.

## Verify signals

- **Post-deploy Signal (AC8):** Reproduce Al scenario via synthetic time-shift on Vincent's agent. `mika ask "un proverbe zen"` twice with 6 days between (`UPDATE served_content SET served_at = ...` scratch script). Expected: 2nd response ≠ 1st, `check_already_served` fires per `tool_calls` table.
- **Steady-state operator grep:** `grep served_content.duplicate_write $MIKA_SPIRIT_LOG_FILE | jq .` should show duplicate-write events only when a real regeneration collides on hash. Sustained bursts (>5/agent/day) indicate LLM discipline drift → tighten the retry-with-avoid prompt.

## Follow-ups (deferred with documented triggers)

- **AC6 fuzzy dedup** — file if AC8 post-deploy shows two lexically-different but semantically-identical proverbs slip through.
- **RC-C content pool diversification** — file if the LLM's post-retry fallback ("plus de proverbes zen frais...") is itself repetitive (canonical proverbs pool exhausted; needs curated non-standard corpus).
- **Cross-agent dedup** — file if multi-agent-per-tenant surfaces show N agents re-serving to the same person.
- **Team-channel dedup** — file if team-run repeat serves become a signal.
- **Retention policy** — file if any customer surface shows >10K rows/month/agent.

## References

- Al founding incident: 2026-07-28 Telegram screenshot (au dossier samidarko).
- Plan: `docs/plans/2026-08-21-001-fix-1867-fidelity-per-user-content-tracking-plan.md`.
- Related agent-honesty class (Al founding tickets): mika#1815 (self-model confabulation), mika#1814 (Show HN violate hermetic), mika#1813 (sur-relance après stop).
- Load-bearing memory: `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — why structural, not prompt-only.
- Load-bearing memory: `feedback_interactive_mika_plan_needs_ac_section_no_rename` — AC section preserved verbatim.
- Post-Conditions #3 (required-tools gate) — `crates/mika-agent/CLAUDE.md` § Post-Conditions.
- Schema migration pattern: `migrate_v44_to_v45` at `crates/mika-agent/src/db.rs:4495`.
