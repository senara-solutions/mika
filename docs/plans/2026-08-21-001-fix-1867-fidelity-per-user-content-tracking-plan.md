---
ticket: mika#1867
type: fix
priority: p1-important
component: agent-core
class: fidelity
plan_date: 2026-08-21
plan_seq: 001
---

# Plan — mika#1867 — Per-user content-serve tracking (fidelity fix)

## Founding evidence (verbatim, Al founding incident)

> **22 juillet** : Al demande "un proverbe zen". Mika sert "Avant l'éveil, couper du bois, porter de l'eau. Après l'éveil, couper du bois, porter de l'eau."
>
> **28 juillet** : Al re-demande "un proverbe zen". Mika **ressert le même proverbe**.
>
> Al : "Duplicate 22 juillet, tu te répètes..."
>
> Mika : "Tu as raison, pardon. Celui-là, je te l'avais déjà servi. Je vais faire attention."

Source: Al (testeur Vietnam) via Telegram 2026-07-28. This is a P1 Couche Confiance (fidelity) regression on the mission-A axis. Al is a co-tester recognized as sharp founding evidence — his signal ratifies the class-of-drift being fixed here (parallel to mika#1815, mika#1814, mika#1813).

## Body-vs-code verification (RC-A/B/C/D)

| RC | Body claim | Code state | Verdict |
|----|-----------|------------|---------|
| RC-A | `async_db.rs::load_recent_messages(limit)` @ line 1113 + `db.rs::load_recent_messages(agent_id, limit)`; SQL filters `agent_id + role != 'summary' + channel_type != 'team'`, ORDER BY `created_at DESC`, no per-session/user filter | `async_db.rs:1428` and `db.rs:8117`; SQL at `db.rs:8205-8207` matches body's SQL byte-for-byte (`SELECT ... FROM messages m JOIN sessions s ON m.session_id = s.id WHERE m.agent_id = ?1 AND m.role != 'summary' AND s.channel_type != 'team' ORDER BY m.created_at DESC, m.id DESC LIMIT ?2`); no per-user filter present | **MATCHES** (body line numbers stale; behavior claim exact) |
| RC-B | Schema: `messages` has no user/chat/correspondent id; `sessions` has `channel_type + metadata`; `people` tracks canonical_name + notes only (no structured content-serves); `core_memory` is `(agent_id, key)` primary key, not `(agent, person)` | `db.rs:1262-1273` messages columns: `id, session_id, agent_id, role, content, metadata, trace_id, compacted_through_id, created_at, internal` — no user_id/chat_id/correspondent_id. `db.rs:1248-1257` sessions has `channel_type + metadata` as claimed. `db.rs:1303-1313` people has `canonical_name, relationship, notes, first_mentioned, last_mentioned, mention_count`, no content-serve columns. `db.rs:1294-1301` core_memory `PRIMARY KEY (agent_id, key)` | **MATCHES** |
| RC-C | Under-constrained content requests ("un proverbe zen") collapse toward the LLM's strong prior on canonical zen quotes | Empirical LLM-behavior hypothesis; not verifiable via `grep`. Body itself declares this **hors-scope** (§ "Sous-défauts hors-scope") and defers to a follow-up if RC-C remains visible after the structural fix | **UNKNOWN by design** — hypothesis, out of scope this ticket |
| RC-D | `prompt.rs` mentions "conversation history > search results" and "check conversation history and search_memory" as guides, but no content-serve dedup discipline | `prompt.rs:735` "conversation history > search results"; `prompt.rs:829` "Before asking a clarifying question, check conversation history and search_memory"; grep for `proverb`, `joke`, `content-request`, `content_request`, `search_content_signature`, `dedup` in `prompt.rs` → zero hits | **MATCHES** |

Schema-version footnote: the body sketches a v39-scoped migration; current live schema is **v44** (see mika/CLAUDE.md § Schema Version). The plan's migration will apply at the next available version (v45) — the body's DDL shape is unchanged; only the version number binds later.

## Design shape (from body, ratified)

The body's design is a **structural fix** targeting the class (RC-A + RC-B): introduce a per-(agent, person, category) content-serve ledger so "what Mika has served to person Y" becomes a fact, not an LLM guess. RC-D and prompt-layer nudges are the defense-in-depth surrounding it. RC-C stays outside this ticket by explicit body scope.

The plan adopts the body's design **as-is** (schema DDL, tool signatures, 8 AC boundaries) — the ticket-owner (orchestrator-CC 2026-07-28) already ran the divergence pass and produced a well-formed structural design. What follows is sequencing, file-path mapping, and integration-point resolution.

## Assumptions (surface for architect if any of these are wrong)

- **A1 — Category enum is stable at 7 values.** The body's `category CHECK` list — `proverb, quote, joke, poem, recommendation, story, fact` — is the initial taxonomy. Rare-category emergence (e.g., `riddle`) is a schema-migration follow-up, not a design escape hatch. Bind at v45.
- **A2 — `person_id` resolution uses `Database::get_person(agent_id, canonical_name)` (memory/mod.rs:112), with fallback to `upsert_person` at ledger-write time.** Ambiguous name (two `Al`s) resolves to first match by `id ASC`; disambiguation is a separate concern (there is no existing disambiguation ledger in mika, and building one is out of scope here).
- **A3 — Content-request classification stays skill-side + prompt-side, not a Rust regex.** The body notes "structural gate (pas prompt-only)" per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`. The **structural** gate here is the ledger itself + the retry-with-avoid loop (AC5); the classifier layer is the prompt/skill instruction telling the LLM to invoke `check_already_served` before generating on content-request classes. Rust-side content classification would require an LLM-shaped classifier we do not currently have — deferring adds fragility and is not what makes the fix structural.
- **A4 — Simple content hash sufficient for v1.** SHA-256 over `content_text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")` (whitespace + case normalization; punctuation preserved for a fair first cut). AC6's fuzzy embedding signature is a **deferred follow-up** if AC7's post-deploy verify demonstrates hash-only insufficient.
- **A5 — 90-day check window default.** `check_already_served` defaults to `served_at >= now() - INTERVAL 90 days`, overridable via parameter. Body doesn't fix the window; 90 days covers seasonal-recall patterns (Al's incident was 6 days) without unbounded history growth.
- **A6 — `channel_type != 'team'` boundary applies here too.** Ledger writes gated on `channel_type NOT IN ('team')` to keep team-internal dispatch chatter out of the user-facing fidelity ledger. Aligns with the existing `load_recent_messages` filter (`db.rs:8206`).

## Phases (implementation sequence)

### Phase 1 — Schema migration (AC1)

**Files.** `crates/mika-agent/src/db.rs` (v45 migration + `CREATE TABLE served_content`), `crates/mika-agent/src/db/kg_schema.rs` (pattern reference for shared-corpus migrations — not a target here; served_content is per-agent).

**Steps.**
1. Add `SCHEMA_VERSION = 45` const bump in `db.rs`.
2. Add v44→v45 migration branch in `apply_migrations()`. Body-provided DDL, applied verbatim (agent_id + person_id foreign keys, category CHECK enum, content_hash for exact-match dedup, optional content_signature for future fuzzy dedup, served_at ISO 8601 default, session_id soft link).
3. Add both body-specified indexes: `idx_served_content_person_cat(agent_id, person_id, category, served_at DESC)` for AC3 query patterns, `idx_served_content_hash(agent_id, person_id, content_hash)` for AC2 idempotency + AC5 exact-match retry.
4. Update mika-agent CLAUDE.md § Schema Version with the v44→v45 entry (backward-compat: read returns empty for rows pre-migration, consistent with "agent that has never served anything = no dedup, safe direction" per AC1).

**AC covered.** AC1 — Schéma `served_content` (migration DB).

### Phase 2 — DB write path + person resolution helper (foundation for AC2/AC3)

**Files.** `crates/mika-agent/src/memory/mod.rs` (add `record_served_content` and `list_served_content` `impl Database` methods, colocated with `upsert_person`, `list_people`), `crates/mika-agent/src/async_db.rs` (async wrappers), `crates/mika-agent/src/db.rs` (add `ServedContent` struct alongside `Person`, `Preference`, etc.).

**Steps.**
1. Add `ServedContent` struct in `db.rs` (mirror `Person` shape): `id, agent_id, person_id, category, content_text, content_hash, content_signature, served_at, session_id`.
2. Add `Database::record_served_content(agent_id, person_id, category, content_text, session_id) -> Result<RecordOutcome>` in `memory/mod.rs`. Computes `content_hash` via the A4 normalization. INSERT with `ON CONFLICT(agent_id, person_id, content_hash) DO NOTHING` returning `RecordOutcome::Inserted { id }` on rowcount==1, `RecordOutcome::Duplicate { existing_id, prior_served_at }` on rowcount==0 (query the pre-existing row).
3. Add `Database::list_served_content(agent_id, person_id, category, since: Option<DateTime>, limit: usize) -> Result<Vec<ServedContent>>` in `memory/mod.rs`. `since` defaults to A5's 90-day window when `None`.
4. Add `AsyncDatabase` wrappers in `async_db.rs` following the existing pattern (`with_db(move |db| db.record_served_content(...))`).
5. Ledger uniqueness is enforced at the SQL level by the `content_hash` unique constraint scoped to `(agent_id, person_id)`. Emit a `warn!` on `RecordOutcome::Duplicate` for observability (grepable `served_content.duplicate_write`).

**AC covered (partial).** AC2 write half (persistence idempotence), AC3 query half.

### Phase 3 — Tool exposure (AC2, AC3)

**Files.** `crates/mika-agent/src/tools/record_served_content.rs` (new), `crates/mika-agent/src/tools/check_already_served.rs` (new), `crates/mika-agent/src/tools/mod.rs` (register both), `crates/mika-agent/src/prompt.rs` (add both to tool-usage prompt block for content-request classes).

**Steps.**
1. Create `RecordServedContentTool` (mirror `StoreFactTool` structure in `store_fact.rs:8`): inputs `person: String, category: String, content: String, signature: Option<String>`. Resolves `person_id` via `Database::get_person(agent_id, person)`; if not found, `upsert_person` first (mirroring existing `store_fact(category="person")` flow, `store_fact.rs`).
2. Create `CheckAlreadyServedTool`: inputs `person: String, category: String, content_hash: Option<String>, days: Option<u32>`. Returns JSON `{ "served_count": N, "items": [{ "served_at": "...", "snippet": "...", "content_hash": "..." }, ...] }`. Snippet truncated at 200 chars.
3. Register both in `tools/mod.rs::default_tools()` (all agents; not skill-scoped — a fidelity gate is engine-level, not skill-level).
4. Add usage guidance to `prompt.rs` in the tool-usage section (near existing `store_fact` guidance, `prompt.rs:823`): "For content requests (proverb, quote, joke, poem, story, recommendation, fact): call `check_already_served(person, category)` BEFORE generating. If items are returned, generate content that does not match any prior `content_hash`. After delivering, call `record_served_content(person, category, content)` to persist the serve."

**AC covered.** AC2 (record tool), AC3 (check tool).

### Phase 4 — Content-request skill layer (AC4)

**Files.** `mika/skills/bundled/` (new skill: `content-request-fidelity`), `crates/mika-agent/src/prompt.rs` (register skill in `DEFAULT_AGENT_SKILL_ALLOWLIST`).

**Steps.**
1. Create bundled skill `content-request-fidelity/` (per `mika/skills/bundled/` conventions, discovered at build time via `build.rs`).
2. `skill.toml`: `[triggers] keywords = ["proverbe", "proverb", "citation", "quote", "joke", "blague", "poème", "poem", "histoire", "story", "recommandation", "recommendation", "fait", "fact"]` (bilingual FR/EN — Al is francophone).
3. `[constraints] required_tools = ["check_already_served"]` — the required-tools gate (see mika/crates/mika-agent/CLAUDE.md § Post-Conditions #3) forces the LLM to call the check tool before EndTurn on any turn where the skill is keyword-matched.
4. `system_prompt.md`: instruct the LLM to (a) call `check_already_served(person, category)`, (b) if items returned, generate something not matching any listed `content_hash`, (c) call `record_served_content` after final generation, (d) if 3 retries all collide (AC5), surface the AC5 fallback message.
5. Add `content-request-fidelity` to `DEFAULT_AGENT_SKILL_ALLOWLIST` (`mika-common/src/home.rs`) so personal/customer agents receive it by default. Also add to the well-known agent allowlists that face users — at minimum the default operator agent (per personal-agent allowlist, mika#1596).

**AC covered.** AC4 — Content-request classifier (skill or prompt-layer detection). Structural gate = the `required_tools` constraint (not prompt-only) per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`. The skill layer is the classifier; the required-tools gate is the structural enforcement.

### Phase 5 — Retry-with-avoid loop (AC5)

**Files.** `crates/mika-agent/src/tools/record_served_content.rs`, `crates/mika-agent/src/prompt.rs`.

**Steps.**
1. `RecordServedContentTool` returns `{ "status": "duplicate", "prior_served_at": "...", "retry_hint": "Content already served on <date>. Regenerate with a different item." }` on `RecordOutcome::Duplicate`. The LLM sees this on the tool_result and self-corrects via the standard agent loop.
2. Skill prompt (Phase 4 step 4) instructs: "If `record_served_content` returns status='duplicate', regenerate up to 3 total attempts. If all 3 collide, deliver the AC5 fallback: 'Je n'ai plus de proverbes zen frais pour toi aujourd'hui — veux-tu que je te propose une autre tradition ?' (adapted per category)."
3. Optional per-turn retry counter (tracked in ToolContext for observability, not enforcement) — the 3-cap is prompt-enforced; the structural gate is that duplicate writes cannot proceed.

**AC covered.** AC5 — Retry-with-avoid loop.

### Phase 6 — Fuzzy dedup (AC6, DEFERRED)

**Explicit deferral.** AC6 (embedding-based content_signature with cosine similarity > 0.90) ships only if AC8's post-deploy verify with Al (or synthetic test) shows hash-only dedup insufficient. The v45 schema column `content_signature TEXT` is present but NULL for v1; the fuzzy-match query would be added in a follow-up ticket when signal warrants. Filing a follow-up ticket is part of Phase 8.

**AC covered.** AC6 — reserved as follow-up.

### Phase 7 — Tests (AC7)

**Files.** `crates/mika-agent/src/memory/mod.rs` (inline `#[cfg(test)] mod tests`), `crates/mika-agent/src/db.rs` (inline tests for DDL + migration), `crates/mika-agent/src/tools/record_served_content.rs` (tool-level tests), `crates/mika-agent/tests/eval/` (integration scenario — mirror the eval-harness pattern used by `#741` grounding regressions).

**Steps.**
1. **Unit — hash normalization.** Assert `content_hash("Avant l'éveil, couper du bois.")` == `content_hash("avant l'éveil,  couper du BOIS.")` (case + whitespace insensitive; punctuation preserved).
2. **Unit — `check_already_served` window.** Seed 3 served rows at t-1d/t-30d/t-100d. Assert default (90d) returns 2; explicit `days=200` returns 3.
3. **Unit — record idempotency.** Two calls with identical (agent, person, hash) → second returns `RecordOutcome::Duplicate` with matching `prior_served_at`.
4. **Integration — 6-day-apart scenario (Al reproduction).** Two conversations 6 days apart via `EvalHarness` + `MockLlmProvider`. Turn 1: user asks "un proverbe zen", LLM emits X, `record_served_content` writes hash-X. Turn 2 (6 days later, mock time-shifted): user asks "un proverbe zen", `check_already_served` returns hash-X, LLM emits different content Y, `record_served_content` writes hash-Y. Assert (a) `check_already_served` was called before generation, (b) response text differs, (c) two rows in served_content with distinct hashes.
5. **Integration — retry-with-avoid.** Force MockLlmProvider to emit hash-X three times in a row (via sequence). Assert (a) 3 attempts, (b) final response is AC5 fallback text, (c) exactly one row persisted (the initial X from turn 1 setup; the 3 retries all hit duplicate and get rejected before write).

**AC covered.** AC7 — Tests (hash normalization, check_already_served window, integration reproduction).

### Phase 8 — Post-deploy verify + follow-ups (AC8)

**Steps.**
1. Post-deploy: reproduce Al's scenario via synthetic test on live Mika (Vincent, not Al — Al is founding evidence, not the verify surface). Synthetic: two "un proverbe zen" asks on the personal agent with 6-day time-shift of served_at (via a one-off `UPDATE served_content SET served_at = ...` in a scratch script). Expected: second response ≠ first response, `check_already_served` fires per audit_events / tool_calls.
2. File follow-up tickets IF signal warrants:
   - **AC6 fuzzy dedup** — file if hash-only insufficient (two lexically-different but semantically-identical proverbs slip through).
   - **RC-C content pool diversification** — file if the LLM's fallback (post retry-with-avoid) is itself repetitive (canonical proverbs pool exhausted; needs curated non-standard corpus).
   - **Cross-agent dedup** — file if any customer runs multiple agent_ids conversing with the same person.
   - **Confidence sourcing** — Al's separate concern about source citation (linked to mika#1815); confirm no scope-bundling here.

**AC covered.** AC8 — Post-deploy verify (via synthetic reproduction).

## Acceptance Criteria (verbatim from ticket body — preserved per feedback_interactive_mika_plan_needs_ac_section_no_rename)

- **AC1** — Schéma `served_content` (migration DB). Migration additive avec les colonnes + index ci-dessus. Backward compat : lecture retourne empty pour rows pré-migration.
- **AC2** — Tool `record_served_content(person, category, content, [signature])`. Nouveau tool exposé à Mika, appelé AVANT d'émettre la réponse. Persistence idempotente : (agent_id, person_id, content_hash) UNIQUE ; on log si duplicate détecté au write.
- **AC3** — Tool `check_already_served(person, category, [content_hash])`. Query : retourne les `served_at` timestamps + snippets pour tout content déjà servi à cette personne dans la catégorie. Utilisé PRE-génération par le prompt/skill orchestrator.
- **AC4** — Content-request classifier (skill or prompt-layer detection). Detect si la requête utilisateur est content-request. Si oui → obligatoire de check `check_already_served` avant génération. Structural gate (pas prompt-only per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`).
- **AC5** — Retry-with-avoid loop. Si le contenu généré matche déjà-servi → retry génération avec instruction explicite. Max 3 retries. Si 3× échec → surface fallback ("Je n'ai plus de proverbes zen frais pour toi aujourd'hui — veux-tu que je te propose une autre tradition ?").
- **AC6** — Fuzzy dedup (optional, follow-up si simple hash insuffisant). Embedding-based signature (content_signature). Match cosine similarity > 0.90 = déjà-servi. Deferred si simple hash suffit pour le vécu Al.
- **AC7** — Tests. Unit : hash normalization ; Unit : check_already_served retourne items dans la fenêtre ; Integration : simulate 2 conversations 6 jours apart, verify 2e génération diverge.
- **AC8** — Post-deploy verify (via Al ou synthetic test). Reproduire le scénario Al : "un proverbe zen" 2 fois avec 6 jours d'écart. Attendu : 2e réponse différente.

## Out of scope (from body, ratified — do NOT bundle)

- RC-C (LLM prior on canonical zen) — deferred until post-deploy signal warrants (Phase 8 § follow-ups).
- Cross-agent dedup — deferred.
- Confidence sourcing (mika#1815) — separate ticket.
- Fuzzy dedup (AC6) — deferred until post-deploy verify (Phase 6 explicit deferral).
- Retroactive backfill — v45 read-empty semantics are intentional (agent with no prior serves = no false-positive dedup).

## Class-fix compound value

The `served_content` structural layer isn't proverb-scoped — it addresses the whole family: joke, quote, story, recommendation, poem, fact. One layer, seven category endpoints (extensible to eight/nine via a v45+ migration when concrete new categories emerge). This is the compound-engineering shape called out in the body's "Class-fix" section, ratified.

## Grounding references

- Al founding incident (Telegram screenshot 2026-07-28) — Al is a co-tester whose signal ratifies mission-A class-of-drift.
- `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — why the fix MUST be structural (schema + required-tools gate), not prompt-only.
- `feedback_interactive_mika_plan_needs_ac_section_no_rename` — AC section preserved verbatim above.
- `feedback_implementation_scope_bundling` — deferred follow-ups filed as separate tickets (Phase 8).
- mika-agent/CLAUDE.md § Post-Conditions #3 (required-tools gate) — the structural enforcement mechanism used in Phase 4.
- mika-agent/CLAUDE.md § Schema Version — current v44 pin, why the migration binds at v45.
- mika/crates/mika-agent/src/memory/mod.rs (`upsert_person`, `get_person`) — the person resolution primitive Phase 3 builds on.
- mika/crates/mika-agent/src/tools/store_fact.rs — the tool structure mirrored in Phase 3.
- Related agent-honesty class (Al founding): mika#1815 (self-model confabulation), mika#1813 (sur-relance après stop), mika#1814 (Show HN violate hermetic).

## Open questions (surface for architect first-pass)

1. **Person resolution ambiguity.** Current `Database::get_person(agent_id, canonical_name)` matches by name only. If an agent has two people with `canonical_name = "Al"` (e.g., "Al Vietnam tester" and "Al Rousset"), which one gets the ledger row? Body doesn't address. Proposal: log a WARN on ambiguous match (multiple rows) and use first-by-id, filing a disambiguation follow-up. Alternative: reject the write and require caller to pass `person_id` directly. Architect — pick.
2. **Fallback text i18n.** AC5 fallback quotes French verbatim. Agent may serve English users. Proposal: fallback text is skill-level, per-locale variants in the skill prompt (French fallback in prompt.fr.md, English in root). Architect — confirm shape.
3. **Snippet length in `check_already_served` output.** Body doesn't fix. Proposal: 200 chars. Trade-off: longer = LLM has more context to avoid, but also more tokens in system context. Architect — sign off or overturn.
