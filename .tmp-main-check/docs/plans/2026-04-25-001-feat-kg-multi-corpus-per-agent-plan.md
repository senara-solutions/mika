---
title: "feat(kg): per-agent multi-corpus aggregation (array of docs roots)"
type: feat
status: active
date: 2026-04-25
issue: senara-solutions/mika#798
---

# feat(kg): per-agent multi-corpus aggregation (array of docs roots)

## Overview

Today every agent has at most one KG corpus. Schema v27 already keys the six shared-layer KG tables on `docs_root_hash` so multiple agents pointing at the same root share extraction (#786/#787). Per-agent KG identity has just landed via #778 — `identity.toml [kg]` carries `enabled: bool` + `docs_root: Option<PathBuf>` (singular), resolving to `KgAgentConfig::Enabled { docs_root, docs_root_hash }` cached on `AgentState`. What is *missing* is the ability for a single agent to span multiple corpora — needed for the planned `mika-arch` agent, which must reason across `docs/solutions/` from all six platform repos at once.

This plan adds:

- **Per-agent plural identity surface** — `identity.toml [kg].docs_roots: Option<Vec<PathBuf>>`, additive to and higher-priority than the existing singular `[kg].docs_root`. This is the primary mechanism for `mika-arch` to declare its six corpora.
- **Optional global plural surface** — `Settings.kg_docs_roots: Option<Vec<PathBuf>>` + `MIKA_KG_DOCS_ROOTS` colon-separated env, additive to and higher-priority than the existing singular global. Useful for ops-level defaults (e.g., a container image baking `MIKA_KG_DOCS_ROOTS` for every agent it hosts) without touching each agent's identity.toml.
- **`KgAgentConfig` shape change** — `Enabled { docs_root, docs_root_hash }` becomes `Enabled { corpora: Vec<CorpusConfig> }` where `CorpusConfig { docs_root, docs_root_hash }`. Single-root agents carry a one-entry `corpora` vec; behavior is byte-equivalent to today.
- **`agent_kg_corpora` table at schema v28** — maps `agent_id → {docs_root_hash, docs_root_path}` so the query path knows which corpora to fan out across without re-deriving from identity. Populated on every startup ingest.
- **Multi-corpus startup loops** — the three per-agent loops in `server/mod.rs` (lexical / extraction / resolution) iterate over `corpora` instead of destructuring a single root.
- **Multi-corpus subject resolution** — `SubjectEntityResolver::new` takes `Vec<String>` of hashes; pending-entity SQL uses `WHERE docs_root_hash IN (…)`.
- **Multi-corpus query** — `query_knowledge_graph` resolves the agent's hashes via `agent_kg_corpora` and fans out FTS5/vec/traversal across all hashes; merging reuses the RRF already in `Database::hybrid_search`.
- **Documented `mika-arch` config recipe** — paste-ready identity.toml shape; `mika-arch` provisioning itself ships in a follow-up.

The 11 existing single-root agents see no behavioral change: their `corpora` vec has length 1, their `agent_kg_corpora` row count is 1, and the query IN-list with one hash is byte-for-byte equivalent to today's singular path. Schema v28 is purely additive (one new table); no v27 row is rewritten.

## Problem Frame

`mika-arch` is the next agent on deck. It needs to reason across `mika/`, `mika-cloud/`, `mika-skills/`, `claude-pilot-py/`, `openclaw/`, and `lettabot/` simultaneously — six distinct repos, each with its own `docs/solutions/` tree. The two non-structural alternatives both fail:

- **Symlink aggregator** — pre-build a fake `docs/solutions/` tree symlinked from the six real trees. Brittle (stale symlinks, watch-loop confusion), and incompatible with the v27 hash key (`fs::canonicalize` resolves all docs back to their original repo paths, yielding six `docs_root_hash` values jumbled into one chunk row's `docs_root_path` column).
- **Team orchestrator** — make `mika-arch` a team-of-six, each member scoped to one repo's docs. Every cross-repo query becomes 6× LLM hops with synthesis on top. Adds latency, cost, and prompt-length floor; reverses the entire reason for KG (single graph, single query).

Multi-corpus support at the schema and config level is the structural answer. v27 laid the groundwork (`docs_root_hash` is the shared-corpus PK); #778 made KG config per-agent identity (`KgAgentConfig` enum, validated at agent init). This plan extends that "one corpus per agent" to "N corpora per agent" — naturally back-compat for the single-root case.

`docs_root_hash` per-host stability still applies; cross-host portability is explicitly not in scope and is documented on `kg::config::hash_docs_root`.

## Requirements Trace

- **R1.** `identity.toml [kg].docs_roots: Option<Vec<PathBuf>>` field added to `KgIdentityConfig`. Takes precedence over existing `[kg].docs_root` when both are set on the same agent. (Issue Proposed Solution + AC#1 reframed for per-agent identity post-#778.)
- **R2.** `Settings.kg_docs_roots: Option<Vec<PathBuf>>` and `MIKA_KG_DOCS_ROOTS` env var (colon-separated absolute paths), as the global fallback when an agent's identity has neither plural nor singular `docs_root`. (Issue AC#1, AC#2.)
- **R3.** `KgAgentConfig::Enabled` shape changes from `{ docs_root, docs_root_hash }` to `{ corpora: Vec<CorpusConfig> }`. Pre-1.0 breaking change per `CLAUDE.md` versioning rule. Single-corpus agents carry a one-entry vec.
- **R4.** Per-agent resolution chain returns `corpora: Vec<CorpusConfig>` with N≥1 entries when `enabled=true`. Order (first hit wins): identity-plural > identity-singular > global-env-plural > global-env-singular > global-config-plural > global-config-singular > CWD fallback (singular).
- **R5.** New `agent_kg_corpora` table at schema v28, populated on every startup lexical-ingest pass. (Issue AC#3.)
- **R6.** Lexical ingest runs once per `(agent, corpus)` pair on startup; emits one `lexical_ingest_complete` per pair with `docs_root_hash` set. (Issue AC#4.)
- **R7.** Subject extraction runs once per `(agent, corpus)` pair on startup, sharing a single per-agent `MIKA_KG_BATCH_BUDGET` across all corpora. Budget exhaustion mid-loop emits `kg_budget_exhausted` with `roots_remaining` field.
- **R8.** Subject entity resolution runs once per agent, scoped to all the agent's corpora via `WHERE docs_root_hash IN (…)`. Resolutions stay per-agent (`kg_subject_resolutions` PK unchanged).
- **R9.** `query_knowledge_graph` resolves the agent's `docs_root_hash` set from `agent_kg_corpora` and fans out FTS5/vec/traversal across all hashes via dynamic IN-lists. Cross-corpus merging reuses existing RRF. (Issue AC#5.)
- **R10.** Single-root identities continue to work without code or config changes — both `[kg].docs_root` and the global singular fallback remain the implicit defaults and produce a one-entry `corpora` vec. (Issue AC#6.)
- **R11.** Net-additive migration: no v27 row is rewritten. v28 creates `agent_kg_corpora` and backfills rows by joining `kg_subject_resolutions → kg_subject_entities` (existing per-agent → docs_root_hash linkage). Agents with no KG data populate on next ingest.
- **R12.** Documented `mika-arch` `[kg]` recipe in `crates/mika-agent/CLAUDE.md` and `docs/configuration.md`. The recipe describes the identity.toml shape; this plan does **not** auto-provision `mika-arch` — that ships in a follow-up. (Issue AC#7 → split.)

## Scope Boundaries

- **Non-goal:** changing the v27 `docs_root_hash` PK on shared-layer tables. v27's PK is the foundation. Schema v28 is purely additive (one new table; FK rewires only on `agent_kg_corpora` itself).
- **Non-goal:** cross-host hash stability. Same caveat as v27. `docs_root_hash` is per-host.
- **Non-goal:** removing or deprecating `[kg].docs_root` (singular) or the global singular surfaces. They stay as the back-compat default. Plural takes precedence within the same scope when both are set.
- **Non-goal:** per-corpus LLM budgets. Each agent gets one `MIKA_KG_BATCH_BUDGET` shared across corpora. A future ticket may split if `mika-arch`'s six-corpus case stresses the budget.
- **Non-goal:** Windows path-list parsing. Colon separator targets Linux/macOS only; documented on the field.
- **Non-goal:** runtime corpus mutation. `agent_kg_corpora` rows are written only by startup ingest. Removing a corpus from identity does not delete its rows from `agent_kg_corpora` or shared-corpus tables — orphan-pruning is a #779-class follow-up.
- **Non-goal:** `IngestionOrchestrator::reingest_and_reextract` multi-corpus handling. The orchestrator currently has no live callsite (verified by grep — see Context). When a future compound-hook callsite ships, that ticket picks the right corpus for the changed file by path-prefix matching.
- **Non-goal:** mixed plural+singular within one identity. If `[kg].docs_roots` is set, `[kg].docs_root` is ignored (with a warn-on-load to flag operator confusion). No union semantics.

### Deferred to Separate Tasks

- **`mika-arch` agent provisioning.** This plan documents the recipe; the `mika-arch` identity, soul, skill assignments, `MIKA_DEV_MODE` auto-provisioning, and any K8s-side env injection ship in a follow-up ticket on `mika` (and likely `mika-cloud`).
- **Compound-hook re-ingest path matching.** `IngestionOrchestrator::reingest_and_reextract` has zero live callsites today. When a future compound hook wires up, it must pick the right corpus for the changed doc.
- **Orphan-corpus pruning.** When an operator removes a corpus from identity, its `agent_kg_corpora` row stays and shared-corpus rows keyed on its hash stay. A `mika kg purge --orphan` CLI is a #779-class follow-up. **Stale-path corollary:** the orphan row's `docs_root_path` column also goes stale — if the operator later changes the filesystem so a different docs tree lives at the old path, the row points at the new tree with the old hash. Probably never bites in practice (paths and hashes drift together in normal ops), but worth naming so the cleanup ticket designs around it.
- **Per-corpus budget split.** Single shared budget across corpora in this plan.
- **`mika doctor` check for "agent has zero corpora rows".** Today an agent with `enabled=true` but unresolvable docs_roots would skip ingest with no aggregated breadcrumb beyond per-corpus warns. A future doctor check could surface intent-vs-state drift.

## Context & Research

### Relevant code and patterns

- **Predecessor #738 — singular global config + resolver:** `mika/docs/plans/2026-04-24-005-feat-kg-docs-root-config-plan.md`. Establishes `Settings.kg_docs_root: Option<PathBuf>`, `MIKA_KG_DOCS_ROOT`, `resolve_kg_docs_root(&Settings) -> (PathBuf, PathSource)`, and the `EnvVar`/`ConfigFile`/`CwdDefault` source variants. The plural global surface mirrors this exactly with an extra Vec layer.
- **Predecessor #786/#787 — v27 schema PK:** `mika/docs/plans/2026-04-24-006-feat-kg-schema-v27-docs-root-hash-plan.md` and `2026-04-24-007-feat-kg-data-migration-v27-coalesce-plan.md`. Establishes `hash_docs_root(&Path) -> String` (16-hex SHA-256 prefix of canonicalized path) and the shared-corpus contract: six tables keyed on `docs_root_hash`; resolutions stay per-agent. v28 does **not** change any of those PKs — adds one new table.
- **Predecessor #778 — per-agent identity-level KG config (just shipped):** `crates/mika-agent/src/kg/config.rs` lines 11-131. Established:
  - `KgAgentConfig::{Disabled, Enabled { docs_root: PathBuf, docs_root_hash: String }}` enum.
  - `KgConfigError` (`thiserror`-derived: `PathNotFound`, `NotADirectory`).
  - `resolve_per_agent_docs_root(&Identity, &Settings) -> Result<KgAgentConfig, KgConfigError>` — entry point at agent init time.
  - `validate_explicit_path(&Path) -> Result<KgAgentConfig, KgConfigError>` — internal helper that constructs the `Enabled` variant; this becomes the per-corpus builder in this plan.
  - `KgIdentityConfig { enabled: bool, docs_root: Option<PathBuf> }` in `crates/mika-agent/src/prompt.rs:55-74`, embedded in `Identity.kg` (line 88). Default: `enabled=true, docs_root=None`.
  - Hard-error policy: per-agent explicit path that doesn't exist → `KgConfigError`. CWD-default falls through to `Enabled` without validation (downstream warn-and-skip per #738).
  - `AgentState.kg_config: KgAgentConfig` cache (`crates/mika-agent/src/server/state.rs:52`); resolved at `init_agent` time (`server/mod.rs:362-457`).
- **Existing startup loops (`crates/mika-agent/src/server/mod.rs:760-1100`):** Three per-agent loops, each destructuring `KgAgentConfig::Enabled { ref docs_root, ref docs_root_hash }`:
  - Lexical ingest at `:770-848` — emits `lexical_ingest_disabled` info log on `Disabled`, runs `LexicalIngestor::ingest_all()` on `Enabled`. Includes the existing #778 R9 drift-WARN that calls `db.count_chunks_for_docs_root_hash(docs_root_hash)` to detect first-run corpora at lines 798-821. This naturally generalizes per-corpus.
  - Subject extraction at `:873-956` — destructures `Enabled { ref docs_root, docs_root_hash: _ }`, spawns one `tokio::spawn` per agent calling `SubjectExtractor::new(db, llm, docs_root, …).extract_pending(budget)`.
  - Entity resolution at `:1005-1100` — destructures `Enabled { docs_root: _, ref docs_root_hash }`, spawns one `tokio::spawn` per agent calling `SubjectEntityResolver::new(db, llm, docs_root_hash, …).resolve_pending(budget)`.
- **`Settings` field pattern (`crates/mika-common/src/config.rs:788-796`):** `kg_docs_root: Option<PathBuf>` with `#[serde(default)]`. `CONFIG_KEYS` registry entry at `:408`, `get_effective_value()` arm at `:537`, `clean_env()` test helper at `:1393-1405`, manual `Debug` impl at `:1409`, `test_defaults` at `:1283-1284`. New `kg_docs_roots: Option<Vec<PathBuf>>` follows this pattern verbatim.
- **`LexicalIngestor` (`crates/mika-agent/src/kg/lexical_ingestor.rs:84-110`):** Constructor `new(db, docs_root, trace_id)` precomputes `docs_root_hash` at construction. All write SQL keyed on `self.docs_root_hash` (`:273-329, :391-411, :428-431`). Agent-agnostic — only knows its hash. Multi-corpus is N constructor calls per agent with the same `db` clone, different `docs_root` arguments. v27's `INSERT OR IGNORE` semantics on `kg_extractions` and `(docs_root_hash, source_doc_path)` UNIQUE on `kg_chunks` make repeated runs cheap.
- **`SubjectExtractor::new` (`crates/mika-agent/src/kg/subject_extractor.rs:393`):** Same per-corpus pattern as ingestor. Multi-corpus = N extractor calls per agent. Shared per-agent budget threaded as a mutable counter in the outer loop.
- **`SubjectEntityResolver::new(db, llm, docs_root_hash, trace_id)` (`crates/mika-agent/src/kg/entity_resolver.rs:160`):** Currently takes a single hash. For multi-corpus, signature changes to `Vec<String>` of hashes; pending-entity SQL changes from `WHERE docs_root_hash = ?` to `WHERE docs_root_hash IN (?, ?, …)`. `kg_subject_resolutions` and `kg_resolutions_log` stay per-agent.
- **`query_knowledge_graph` query module (`crates/mika-agent/src/kg/query.rs`):** Path B (`:362-385`), Path C entry-via-chunks (`:516-555, :571-650, :1182-1227`), traversal (`:722-810`), context enrichment (`:915-1020`). Every `WHERE … = ?` predicate on `docs_root_hash` becomes `WHERE … IN (?, ?, …)`. Dynamic IN-list construction follows the pattern at `:932-940` for chunk ID enumeration. RRF is already implemented inside Path C's `Database::hybrid_search` — per-corpus chunks compete in a single ranked list naturally because all corpora share the same FTS5 index.
- **`query_knowledge_graph` tool (`crates/mika-agent/src/tools/query_knowledge_graph.rs:107-116`):** Input today has `agent_id: Option<String>` and `docs_root_hash: Option<String>`. Adds `docs_root_hashes: Option<Vec<String>>` with priority `docs_root_hashes` > resolved-from-`agent_id` (via new `Database::list_agent_corpora`) > `docs_root_hash` (deprecated singular).
- **`agents` table (`crates/mika-agent/src/db.rs:1053-1060`):** `id TEXT PRIMARY KEY` plus `home_dir`, `active`, `last_seen`. A real SQL table — `agent_kg_corpora.agent_id REFERENCES agents(id) ON DELETE CASCADE` is sound and gives free cleanup on agent deletion.
- **Migration pattern (`crates/mika-agent/src/db.rs:716-830`):** Chained `if (3..=N).contains(&version) { self.migrate_vN_to_vN+1()?; … }`. v27→v28 appends one arm. Migration body uses `BEGIN IMMEDIATE; … COMMIT;` per the v25→v26 idiom; idempotency via `table_exists`. Clean-slate `migrate_v1` (`:1288-1386`) adds the `CREATE TABLE agent_kg_corpora` DDL plus the `INSERT INTO schema_version (version) VALUES (28);` bump. `CURRENT_SCHEMA_VERSION = 27` at `db.rs:27` becomes `28`.
- **`schema_meta` table (added in v27):** Tracks migration completion markers. v28 does not need a marker — the migration is data-additive, not destructive. Partial completion just leaves an empty/partial `agent_kg_corpora` table that the next startup ingest re-populates. No safety guard required.
- **`Database::count_chunks_for_docs_root_hash` (existing, used in #778's drift-WARN at `server/mod.rs:801`):** Cheap helper; the multi-corpus drift-WARN reuses it per corpus.
- **Test fixture (`crates/mika-agent/tests/eval/kg_fixtures/mod.rs:25`):** `PINNED_SCHEMA_VERSION = 27` bumps to `28`. Add `seed_agent_corpus(agent_id, docs_root_hash, docs_root_path)` helper. Existing fixture builders that touch agents add `agent_kg_corpora` rows to keep `kg_self_knowledge` scenarios passing.
- **Compound-hook orchestrator (`crates/mika-agent/src/kg/ingestion_orchestrator.rs`):** `reingest_and_reextract` exists but has zero live callsites in `crates/` (verified). Constructor takes a single `docs_root`. This plan leaves the orchestrator's signature unchanged — captured under Deferred Tasks.
- **`.env.example` KG block (mika repo root, ~lines 41-52):** Lists `MIKA_KG_INGESTION_MODEL`, `MIKA_KG_EXTRACTION_MODEL`, `MIKA_KG_RESOLUTION_MODEL`, `MIKA_KG_BATCH_BUDGET`, `MIKA_KG_DOCS_ROOT`. New `MIKA_KG_DOCS_ROOTS` line goes adjacent.

### Institutional learnings

- **`docs/solutions/architecture-patterns/simplified-config-4-source-model.md`** — canonical cascade. With #778 in place, the per-agent identity sits *above* the four-source cascade for any KG-config-bearing field. The cascade still applies for global fallback when identity has no docs_root(s).
- **`docs/solutions/architecture-patterns/config-key-rename-across-layers.md`** — full-layer checklist: registry entry, Settings field, `get_effective_value()` arm, `.env.example`, CLAUDE.md, `docs/configuration.md`, `clean_env()`, test fixtures. Applies to the new global plural surface and to the new identity field.
- **`docs/solutions/architecture-patterns/config-key-registry-cli-management.md`** — `mika config get/set kg_docs_roots` works via the registry; CLI accepts the TOML representation: `mika config set kg_docs_roots '["a", "b"]'`.
- **`docs/solutions/database-issues/kg-schema-three-layer-sqlite-design.md`** — schema convergence test mandatory. v28 adds one table; convergence test compares fresh-install vs incremental.
- **`docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`** — migration immutability. Don't edit `migrate_v26_to_v27` or earlier; v28 work lives in `migrate_v27_to_v28`. Backfill at migration time avoids the empty-table window.
- **`docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md`** — per-corpus write contract is transactionally clean. Multi-corpus = N independent write targets; no contract change.
- **`docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`** — implicit state drifts. `agent_kg_corpora` makes corpus membership explicit at the schema level (one SQL row per agent-corpus link) rather than re-deriving from identity at every query.
- **`docs/solutions/best-practices/socratic-multi-ticket-milestone-planning-2026-04-21.md`** — when a sibling ticket surfaces a gap, amend the open ticket. Applied: if `mika-arch` provisioning surfaces a config-shape gap during its own grooming, this plan's CLAUDE.md recipe gets amended in-place rather than forking forward.
- **`mika/CLAUDE.md` versioning rule:** "Pre-1.0 breaking changes do not require backward compatibility" — applies to the `KgAgentConfig::Enabled` shape change. Document the migration step in the PR description.

### External references

- None warranted. Multi-tenant content fan-out + RRF merge is industry-standard and well-implemented locally. `Database::hybrid_search` already runs RRF on FTS5 + sqlite-vec; no external research adds value over the local pattern density.

## Key Technical Decisions

- **Per-agent identity is the primary multi-corpus surface; global plural is a secondary fallback.** With #778 in place, `mika-arch`'s six corpora belong in `~/.mika/agents/mika-arch/identity.toml [kg].docs_roots = […]`. The global `MIKA_KG_DOCS_ROOTS` exists for ops convenience (e.g., baking a default into a container image, or hosts where editing identity.toml is awkward), but the per-agent identity always wins. **Why:** identity is where per-agent KG behavior already lives post-#778; piling new global state on top would split mental models.

- **Resolution chain has six sources, first-hit-wins, no union.** Order: `identity.kg.docs_roots` (plural) > `identity.kg.docs_root` (singular) > `MIKA_KG_DOCS_ROOTS` env (plural) > `MIKA_KG_DOCS_ROOT` env (singular) > `settings.kg_docs_roots` (plural) > `settings.kg_docs_root` (singular) > CWD default. The env tier sits above config-file because env > config is the existing config-rs cascade rule. Within each tier, plural beats singular. **Why:** mirrors the layered cascade users already understand; the plural-beats-singular rule per tier is the only new mental model and it's intuitive (more specific wins).

- **Setting `[kg].docs_roots` and `[kg].docs_root` together is a misconfiguration; plural wins, singular ignored, warn-on-resolve.** Same for global plural+singular at *every* tier (env tier and config tier each emit the same `_singular_ignored` warn variant). **Why:** silently accepting both invites confusion when an operator forgets which is canonical. The warn gives an audit trail without rejecting the config. Symmetric across all three tiers — operator who set both env vars deserves the same observability as one who set both identity fields.

- **Plural sources use per-path validate-and-skip; singular sources keep #778's all-or-nothing.** This is the one architectural asymmetry the plan introduces, and it is deliberate. With singular, the unit of operator intent is one path — a typo bricks the agent loudly, which is what #778 chose and what is right for one path. With plural, the unit shifts: each path is an independent "I want this corpus." Treating the *set* as the unit means a one-character typo in `mika-arch`'s sixth path bricks the other five corpora the operator successfully configured — not what the operator asserted. So: for plural-identity, plural-env, and plural-config sources, each path is validated independently; existing-and-directory paths become `CorpusConfig` entries; missing or non-directory paths emit a `kg_corpus_skipped` warn (with `agent_id`, source tier, the bad path, and the count of paths that *did* resolve) and are dropped from the corpora vec. The agent goes `Disabled` only if zero paths resolve. Singular-identity / singular-env / singular-config keep the hard-error contract from #778. **Why:** matches the operator mental model when six paths are listed (clone five repos, miss one, get five corpora plus a clear "lettabot/ skipped" warn — rather than a single `KgConfigError` that hides which five would have worked). Asymmetry is documented in CLAUDE.md so operators reading either policy see the other named.

- **Empty list at any tier falls through; doesn't pin "explicit empty corpus".** `MIKA_KG_DOCS_ROOTS=""`, `MIKA_KG_DOCS_ROOTS=":::"`, `[kg].docs_roots = []`, and `settings.kg_docs_roots = []` all resolve as "no plural value at this tier" and the cascade continues. **Why:** silently treating these as "explicit empty corpora" would brick the agent (or skip KG without a clear log) and be confusable with "unset". Existing `kg_docs_root` empty-string warn (#738) covers the singular-empty case; the plural-empty case behaves identically to "unset".

- **`KgAgentConfig::Enabled` shape changes from `{ docs_root, docs_root_hash }` to `{ corpora: Vec<CorpusConfig> }` (with `CorpusConfig { docs_root, docs_root_hash }`).** Pre-1.0 breaking change per `mika/CLAUDE.md`'s versioning rule. Single-corpus agents have a one-entry vec. **Why:** introducing a third variant (`EnabledMulti`) doubles the destructure surface across `server/mod.rs` and is harder to grep over. A vec is naturally back-compat with a length-1 element. The breaking-change cost is bounded — exhaustive `match`es on the enum (currently three call sites in `server/mod.rs` plus tests in `kg/config.rs`) are mechanically updated in this plan.

- **`KgAgentConfig::Disabled` carries a `reason: DisabledReason` field.** Three variants: `OperatorOptOut` (identity `enabled=false`), `CwdDefaultMissing` (CWD fallback path doesn't exist), `AllPathsUnresolvable { source, attempted }` (plural source listed N paths, all skipped by validate-and-skip). **Why:** without the reason, an operator who lists six paths and gets zero corpora sees only the per-path skip warns plus a generic "agent KG disabled" log and has to mentally correlate them. The reason field makes the disabled state self-describing — `mika status` / `mika doctor` / startup log all surface "disabled because all 6 paths in `[kg].docs_roots` failed validation" as one line. One enum variant of cost; high diagnostic value.

- **`PathSource` enum extended with `EnvVarPlural` and `ConfigFilePlural`.** Plus a new `IdentityPathPlural` variant alongside the existing `IdentityPath` (if such a variant exists post-#778; see Open Questions). The exhaustive-match test continues to enforce hand-update on additions. **Why:** the source-of-origin distinction is load-bearing for any future operator diagnostic ("which tier did this corpus come from?") and for the warn-on-misconfig logic.

- **`agent_kg_corpora` is a real SQL table with `REFERENCES agents(id) ON DELETE CASCADE`.** Not a derived view, not in-memory only. **Why:** (a) the query path needs cheap, transactional access to "what corpora does agent X have?"; (b) FK CASCADE gives free cleanup when an agent row is deleted; (c) explicit SQL state is auditable at `mika kg status` time vs re-running config resolution.

- **Schema bump to v28; no `schema_meta` marker required.** Migration adds one table and backfills via `kg_subject_resolutions → kg_subject_entities`. Backfill is idempotent (`INSERT OR IGNORE`). **Why:** v28 is data-additive — partial migration is safe (next startup ingest re-populates). The v27 marker pattern guarded against destructive rebuild and is overkill here.

- **`SubjectEntityResolver::new` signature changes from `docs_root_hash: String` to `docs_root_hashes: Vec<String>`.** **Why:** subject entities for a multi-corpus agent live across N corpora; running N resolver instances per agent would multiply `kg_resolutions_log` writes and re-run Stage-1 exact-match passes redundantly. A single resolver scoped to the agent's full corpus set is cheaper.

- **`SubjectExtractor` constructor signature unchanged.** It still takes one `docs_root` per call. The startup loop just calls it N times per agent, sharing a per-agent budget counter. **Why:** the extractor's pending-doc tracking is per-corpus by design (`kg_extractions` keyed on `(docs_root_hash, source_doc_path)`) — running it N times is the correct semantics.

- **Per-corpus extraction shares one per-agent budget, drained left-to-right in `corpora` array order.** When the budget runs out mid-loop, emit `kg_budget_exhausted` with `roots_remaining` naming the unfinished corpora. **Why:** budget today is "per-startup-per-agent"; splitting per-corpus changes operator mental model. If `mika-arch`'s case stresses the budget, a follow-up adds the split. **Operator implication:** array order matters under budget pressure — the first corpus in `[kg].docs_roots` always gets fully extracted; the last gets leftovers. Documented in CLAUDE.md as: "if extraction exhausts budget, the trailing entries in `docs_roots` are skipped this restart and resume next startup. Place highest-priority corpora first." Round-robin would be more fair but adds complexity; documentation is the cheap fix.

- **`query_knowledge_graph` tool input gains `docs_root_hashes: Option<Vec<String>>`; existing `docs_root_hash` (singular) deprecated with concrete removal commitment.** `#[deprecated(since = "0.6.0", note = "use docs_root_hashes; removed in 0.7.0")]`. Lookup priority: explicit `docs_root_hashes` > resolved-from-`agent_id` (via `agent_kg_corpora`) > singular `docs_root_hash` > none. **Why:** the common case is "tool has agent_id; look up corpora from `agent_kg_corpora`". Pre-1.0, perpetual deprecation has zero benefit — name the removal version up front so the cleanup is unambiguous.

- **RRF reuses `Database::hybrid_search`'s existing FTS5+vec ranking.** Multi-corpus IN-list filter applies before ranking, so cross-corpus chunks compete in a single ranked list. **Why:** writing a second RRF risks divergence; the existing one has been hardened against edge cases.

- **`mika-arch` provisioning is out of scope.** This plan documents the recipe but does not auto-provision the agent. **Why:** auto-provisioning needs identity, soul, skill assignments, and likely K8s env injection — separate ticket scope.

## Open Questions

### Resolved during planning

- **Q: Should `[kg].docs_roots` and the global `kg_docs_roots` accept TOML arrays or colon-separated strings in the config files?** → TOML arrays. Colon-separated is for env vars only (shell-friendly). config-rs deserializes `Option<Vec<PathBuf>>` from TOML arrays natively.
- **Q: Should `agent_kg_corpora` rows be deleted when a corpus is removed from identity?** → No, not in this plan. Orphan rows are harmless to query correctness (stale hashes return empty fan-out branches) and a future `mika kg purge --orphan` handles cleanup.
- **Q: Should `query_knowledge_graph` keep accepting the singular `docs_root_hash` field?** → Yes, deprecated but functional, with a concrete removal commitment: remove in `mika 0.7.0`. Current workspace is `0.5.0`; this plan ships in `0.6.0` with the `#[deprecated]` annotation; removal lands in the next minor. `#[deprecated(since = "0.6.0", note = "use docs_root_hashes; removed in 0.7.0")]` makes the contract explicit. Pre-1.0 the cost is low, so don't let "one release" drift into perpetual deprecation.
- **Q: Should the migration backfill `agent_kg_corpora` for agents that have identity-set roots but no KG data yet?** → No — backfill only reads from existing KG data. The next startup ingest writes rows for any agents whose data was missing.
- **Q: Backfill SELECT misses agents in the "extracted but not resolved" state — they have `kg_chunks` and `kg_subject_entities` rows but no `kg_subject_resolutions` rows yet. Result: their first query post-migration returns empty corpora. Should backfill UNION in extraction-only state?** → Partial yes. `kg_subject_entities` is keyed by `docs_root_hash` (no `agent_id`) post-v27, so direct attribution is impossible. But `kg_chunks` was originally written with a session/trace from one agent's startup ingest — that agent's identity is the source of truth for "who configured this corpus." Backfill cannot recover that mapping from v27 data alone. Accept the gap and rely on the **startup-ordering invariant** to close it: `Database::open()` runs migration → `init_agent` constructs `kg_config` → startup lexical/extraction/resolution loops run (Unit 4 writes `agent_kg_corpora` rows in the lexical loop) → server begins accepting requests. Agents in extracted-but-not-resolved state get their `agent_kg_corpora` row written before any query can hit them. Unit 4 verification step explicitly asserts this ordering: no message-handling code path executes before the lexical loop completes for all agents.
- **Q: Does `mika-arch` need a separate K8s/Helm change in `mika-cloud`?** → Likely yes (env injection or identity ConfigMap), but that's part of the deferred `mika-arch` provisioning ticket.
- **Q: Should the resolver collapse duplicate paths (e.g., `[kg].docs_roots = ["/a", "/a"]`)?** → Yes, with a two-tier severity split. Compare path strings before vs after canonicalization. Literal duplicate path strings (`["/a", "/a"]` or normalized identicals) → `info` log `kg_docs_roots_duplicate_literal` with `agent_id` and the duplicated path string. Distinct path strings that canonicalize to the same hash (symlinks, bind mounts, `/projects/repo-a/docs` and `/projects/repo-b/docs` both pointing at the same tree) → `warn` log `kg_docs_roots_duplicate_canonical` with **all collision members named individually** (`source_paths: [String]`), the **canonicalized target** (`canonical_path: String`), and the resulting `docs_root_hash`. Operator reading the warn must be able to pick which symlink to investigate without rerunning anything; the hash alone is insufficient. The warn deserves a louder signal than a copy-paste typo because it almost always means the operator wrote something they didn't mean.
- **Q: How does the budget count when an agent has multiple corpora?** → Single per-agent counter, drained across corpora in resolution order. The first corpus that exhausts it stops the loop; next restart resumes via existing pending-doc tracking.
- **Q: Should the `agent_kg_corpora` row carry a creation source (env / config / identity-plural / etc.)?** → No — the row already carries `docs_root_path` for human inspection. Source-of-origin is a transient resolution-time concern; persisting it would diverge over time as operators move config between layers.
- **Q: Should `IdentityPathPlural` be a new `PathSource` variant?** → Yes. The existing post-#778 `PathSource` enum is the resolver's public source-of-origin enum. Adding the plural variants there keeps the contract unified. If #778 did not introduce an `IdentityPath` variant (and instead handles identity outside of `PathSource`), then this plan adds only `EnvVarPlural` and `ConfigFilePlural` to `PathSource` and the identity-source distinction lives outside the enum. Verify at implementation time.
- **Q: Should `[kg].docs_roots` and `[kg].docs_root` be allowed simultaneously?** → No. Plural wins, singular ignored with a `kg_docs_roots_singular_ignored` warn at *resolver* time (not identity-load time). Same warn variant fires symmetrically when env-plural+env-singular are both set, or config-plural+config-singular. Carries `agent_id`, source tier (`identity` / `env` / `config`), and the ignored singular path. Resolver placement (in `kg/config.rs::resolve_per_agent_docs_root`) means it fires once per agent at init time rather than every time identity is deserialized — same observability, no log spam from status / lint / config-validation paths.

### Deferred to implementation

- Exact wording for the `MIKA_KG_DOCS_ROOTS` `ConfigKeyInfo` description — mirror `MIKA_KG_DOCS_ROOT`'s description with "comma" replaced by "colon" and "single absolute path" replaced by "colon-separated absolute paths".
- Whether `Database::list_agent_corpora(agent_id)` returns `Vec<(String, String)>` (hash + path) or `Vec<String>` (hash only). Query tool only needs hashes; status diagnostics may want both. Easier to add the second column at use-site.
- Whether per-corpus drift-WARN at `server/mod.rs:798-821` (post-#778) emits N times per agent or aggregates into one log line. Probably N times for searchability; defer to implementer judgment.
- Whether `KgIdentityConfig` exposes `docs_roots()` accessor returning effective `Vec<PathBuf>` (canonical post-fallback) or keeps the raw `docs_root` and `docs_roots` fields and the resolver does the fold. Probably the latter to keep `KgIdentityConfig` a deserialization shape.
- Two layers of "absent" representation: `KnowledgeGraphQuery.docs_root_hashes: Vec<String>` (default empty = "no filter" sentinel for in-process callers) and `query_knowledge_graph` tool input `Option<Vec<String>>` (preserves "absent" at the JSON-input boundary). Both intentional — document the why in code comments so a future editor doesn't try to unify them and break test fixtures that rely on the empty-vec sentinel.

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

### Resolution flow at agent init

```
identity.toml [kg] section + Settings (config-rs cascade) + process env
    │
    └── resolve_per_agent_docs_root(&Identity, &Settings) -> Result<KgAgentConfig, KgConfigError>
            │
            if !identity.kg.enabled:
                -> KgAgentConfig::Disabled
            else:
                paths := first_hit_in_priority_order:
                    1. identity.kg.docs_roots  (plural)         IdentityPathPlural
                    2. [identity.kg.docs_root]  (singular)      IdentityPath  (post-#778)
                    3. split(':', MIKA_KG_DOCS_ROOTS)           EnvVarPlural
                    4. [MIKA_KG_DOCS_ROOT]                      EnvVar
                    5. settings.kg_docs_roots (plural)          ConfigFilePlural
                    6. [settings.kg_docs_root] (singular)       ConfigFile
                    7. [<CWD>/docs/solutions]                   CwdDefault
                paths := dedupe_by_hash(paths)
                corpora := paths.map(p => validate_path(p, source))
                    -> on explicit-source PathNotFound/NotADirectory: Err(KgConfigError)
                    -> on CWD-source missing: skip silently (warn-and-skip per #738)
                if corpora.is_empty(): -> KgAgentConfig::Disabled
                else: -> KgAgentConfig::Enabled { corpora }
```

The `validate_path` function returns either `Some(CorpusConfig)` (Enabled-eligible) or `Err` for explicit-source misconfig or `None` for CWD-source missing.

### Startup ingest flow

```
For each agent:
    match agent_state.kg_config:
        Disabled -> log + continue
        Enabled { corpora } ->
            For each CorpusConfig { docs_root, docs_root_hash } in corpora:
                if !docs_root.exists():
                    log "docs_root not found — skipping" + continue
                INSERT OR IGNORE INTO agent_kg_corpora (agent_id, docs_root_hash, docs_root_path)
                drift_warn := db.count_chunks_for_docs_root_hash(docs_root_hash) == 0
                LexicalIngestor::new(db, docs_root, …).ingest_all()
                    -> writes shared kg_chunks rows keyed on docs_root_hash

After lexical:
    For each agent:
        match agent_state.kg_config:
            Disabled -> log + continue
            Enabled { corpora } ->
                budget := MIKA_KG_BATCH_BUDGET
                For each corpus in corpora:
                    stats := SubjectExtractor::new(db, llm, corpus.docs_root, …).extract_pending(budget)
                    budget -= stats.llm_calls
                    if budget <= 0:
                        log "kg_budget_exhausted scope=extraction roots_remaining=N"
                        break

After extraction:
    For each agent:
        match agent_state.kg_config:
            Disabled -> log + continue
            Enabled { corpora } ->
                hashes := corpora.map(c => c.docs_root_hash)
                SubjectEntityResolver::new(db, llm, hashes, …).resolve_pending(budget)
                    -> internally: WHERE docs_root_hash IN (?, ?, …)
```

### Query fan-out

```
query_knowledge_graph(agent_id, …)
    │
    ├── if input.docs_root_hashes set       -> use those
    ├── elif input.agent_id set             -> SELECT FROM agent_kg_corpora WHERE agent_id=?
    ├── elif input.docs_root_hash set       -> [singleton] (deprecated, back-compat)
    └── else                                -> [empty]; no corpus filter (test/debug)
        │
    hashes: Vec<String>
        │
    Path B   WHERE docs_root_hash IN (?, …)            -> subject entity matches across corpora
    Path C   hybrid_search(...) -> kg_chunk_subjects
                WHERE docs_root_hash IN (?, …)         -> RRF merge across all corpora (existing)
    Traversal WHERE r.docs_root_hash IN (?, …)         -> edges spanning agent's corpora
    Context  WHERE cs.docs_root_hash IN (?, …)         -> chunk-prose enrichment

dedupe by (layer, entity_id) keeping highest confidence (existing logic at query.rs:298)
```

IN-list construction follows the `query.rs:932-940` pattern — bind one parameter per hash, build the SQL string with `?,?,…` of the right length.

### `agent_kg_corpora` table

```sql
CREATE TABLE agent_kg_corpora (
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    docs_root_hash TEXT NOT NULL,
    docs_root_path TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (agent_id, docs_root_hash)
);
CREATE INDEX idx_agent_kg_corpora_hash ON agent_kg_corpora(docs_root_hash);
```

The hash-side index supports the inverse "which agents share this corpus" query — useful for the existing `kg_shared_docs_root` advisory and future operator diagnostics.

## Implementation Units

### Unit 1: Add `kg_docs_roots` to `Settings` and `CONFIG_KEYS` registry

- [ ] **Unit 1**

**Goal:** Land the global plural field, registry entry, and `get_effective_value()` arm. Env-var parsing happens in the resolver (Unit 3).

**Requirements:** R2.

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-common/src/config.rs`
  - Add `pub kg_docs_roots: Option<Vec<PathBuf>>` with `#[serde(default)]` to `Settings`, adjacent to `kg_docs_root` (~line 796).
  - Add `ConfigKeyInfo` entry: `key="kg_docs_roots"`, `env_var=Some("MIKA_KG_DOCS_ROOTS")`, `secret=false`, `backend=ConfigBackend::File`. Description mirrors `MIKA_KG_DOCS_ROOT` with "colon-separated list" wording; mentions Linux/macOS-only and that this is the global fallback (per-agent identity wins).
  - Add `get_effective_value()` arm for `"kg_docs_roots"` returning the field value (serialize Vec to colon-joined display).
  - Add `MIKA_KG_DOCS_ROOTS` to `clean_env()` test helper.
  - Add the field to the manual `Debug` impl at ~line 1409.
  - Add `kg_docs_roots: None` to `Settings::test_defaults()`.

**Approach:**
- Mirror `kg_docs_root` field style verbatim, swapping type and description.
- `Option<Vec<PathBuf>>` so `None` distinguishes "not set" from `Some(vec![])`. The resolver in Unit 3 collapses both to "no plural value at this tier" and continues the cascade.

**Patterns to follow:**
- `crates/mika-common/src/config.rs:788-796` — `kg_docs_root` field.
- `crates/mika-common/src/config.rs:408-410` — `kg_docs_root` registry entry.
- `crates/mika-common/src/config.rs:537-538` — `get_effective_value` arm.
- `docs/solutions/architecture-patterns/config-key-rename-across-layers.md` — full-layer checklist.

**Test scenarios:**
- Happy path (TOML array): `Settings::from_str("kg_docs_roots = [\"/a\", \"/b\"]")` → `kg_docs_roots == Some(vec![PathBuf::from("/a"), PathBuf::from("/b")])`.
- Happy path (empty config): `Settings::from_str("")` → `kg_docs_roots == None`.
- Edge case (`kg_docs_roots = []`): `Some(vec![])`. The resolver in Unit 3 treats this as "no value" and falls through.
- Integration: `get_effective_value("kg_docs_roots")` returns expected source/value.
- Integration: `clean_env()` actually unsets `MIKA_KG_DOCS_ROOTS`.
- Coverage: existing `get_effective_value` coverage test (CI-gated) includes the new key.

**Verification:**
- `cargo test -p mika-common config::tests` passes.
- `mika config get kg_docs_roots` returns `[unset]` on a clean install (manual smoke).

### Unit 2: Schema v28 — `agent_kg_corpora` table + backfill

- [ ] **Unit 2**

**Goal:** Bump `CURRENT_SCHEMA_VERSION` to 28; add `migrate_v27_to_v28()`; update clean-slate DDL; bump test fixture pin.

**Requirements:** R5, R11.

**Dependencies:** None (parallelizable with Unit 1).

**Files:**
- Modify: `crates/mika-agent/src/db.rs`
  - Bump `CURRENT_SCHEMA_VERSION` from 27 to 28 (line 27).
  - Add `migrate_v27_to_v28()` following the v25→v26 idiom: `BEGIN IMMEDIATE; CREATE TABLE agent_kg_corpora (…); CREATE INDEX …; INSERT OR IGNORE INTO agent_kg_corpora SELECT DISTINCT r.agent_id, e.docs_root_hash, e.docs_root FROM kg_subject_resolutions r JOIN kg_subject_entities e ON e.id = r.subject_entity_id; COMMIT;`. Idempotency via `table_exists("agent_kg_corpora")` guard.
  - Append v27→v28 arm to migration dispatch chain (~end of `:830`).
  - Update `migrate_v1` clean-slate DDL block (~`:1288-1386`) to include the `CREATE TABLE` and `CREATE INDEX`.
  - Update the `INSERT INTO schema_version` seed in `migrate_v1` from 27 to 28.
  - Add `Database::register_agent_corpus(agent_id, docs_root_hash, docs_root_path)` (INSERT OR IGNORE).
  - Add `Database::list_agent_corpora(agent_id) -> Vec<(String, String)>` (hash + path).
- Modify: `crates/mika-agent/tests/eval/kg_fixtures/mod.rs`
  - Bump `PINNED_SCHEMA_VERSION` from 27 to 28.
  - Add `seed_agent_corpus(db, agent_id, docs_root_hash, docs_root_path)` helper.
  - Update existing fixture builders that touch `agents` to populate `agent_kg_corpora` rows so `kg_self_knowledge` scenarios continue to query correctly.
- Test: `crates/mika-agent/src/db.rs` inline `#[cfg(test)] mod tests` for migration cases.
- Test: `crates/mika-agent/tests/db_schema_convergence.rs` — fresh-install vs incremental.

**Approach:**
- Migration body is a single `execute_batch` with `BEGIN IMMEDIATE; … COMMIT;`. Backfill SELECT joins through `kg_subject_resolutions → kg_subject_entities` because both have rows for any agent with KG data.
- Backfill is `INSERT OR IGNORE` — re-running migration on an already-migrated DB is a no-op.

**Patterns to follow:**
- `crates/mika-agent/src/db.rs:2959-2979` — v25→v26 migration template.
- `crates/mika-agent/src/db.rs:716-830` — migration dispatch chain.
- `crates/mika-agent/src/db.rs:1288-1386` — clean-slate DDL block.
- `crates/mika-agent/tests/db_schema_convergence.rs` — convergence test idiom.

**Execution note:** Test-first for the migration. Write the convergence test and a backfill-correctness test before touching `db.rs`.

**Test scenarios:**
- Happy path (fresh install): clean-slate `migrate_v1` produces a DB with `agent_kg_corpora` present and `schema_version = 28`.
- Happy path (upgrade from v27): seed v27 with two agents (`alice`, `bob`) each with subject entities under `HASH_A`. Run `migrate_v27_to_v28`. `agent_kg_corpora` has 2 rows.
- Happy path (no KG data): seed v27 with agents but no `kg_subject_resolutions`. Migration runs; `agent_kg_corpora` empty; schema_version=28. Next ingest populates.
- Edge case (idempotent re-run): two migration runs; row count unchanged.
- Edge case (FK cascade): insert corpus row for `charlie`; `DELETE FROM agents WHERE id='charlie'`; corpus row gone.
- Convergence: fresh-install at v28 ≡ incremental v1→v28 (PRAGMA table_info equivalence).
- Integration: `kg_self_knowledge` scenarios pass after fixture pin bump and `seed_agent_corpus` integration.

**Verification:**
- `cargo test -p mika-agent db::tests::migrate_v27_to_v28` passes.
- `cargo test -p mika-agent --test db_schema_convergence` passes.
- `cargo test -p mika-agent --test eval kg_self_knowledge` passes.

### Unit 3: Multi-corpus per-agent identity + resolver

- [ ] **Unit 3**

**Goal:** Add `[kg].docs_roots: Option<Vec<PathBuf>>` to `KgIdentityConfig`. Change `KgAgentConfig::Enabled` from `{ docs_root, docs_root_hash }` to `{ corpora: Vec<CorpusConfig> }`. Rewrite `resolve_per_agent_docs_root` to walk the six-source priority chain and produce a corpora vec.

**Requirements:** R1, R3, R4, R10.

**Dependencies:** Unit 1.

**Files:**
- Modify: `crates/mika-agent/src/prompt.rs`
  - Add `pub docs_roots: Option<Vec<PathBuf>>` to `KgIdentityConfig` with `#[serde(default)]`.
  - Update `Default` impl (line ~69) and `KgIdentityConfig` deserialization tests at line ~2046.
  - **No warn here.** The dual-set warn lives in the resolver (see below) so it fires once per agent at init rather than every identity load.
  - Resolver-call contract: `resolve_per_agent_docs_root` is **init-only**, called from exactly one site (`server::init_agent` ~line 362) per process lifetime per agent. Document on the function with `# Call-site contract` doc-section. Future config-reload / hot-identity-reload features must either reuse the cached `kg_config` from `AgentState` or explicitly re-warn — they MUST NOT silently re-call the resolver, because the dual-set warn has no dedup state and would emit per call. If hot-reload is added, it ships with a deduplication strategy (e.g., warn only on transitions) as part of the same ticket.
- Modify: `crates/mika-agent/src/kg/config.rs`
  - Replace the `Enabled { docs_root, docs_root_hash }` variant with `Enabled { corpora: Vec<CorpusConfig> }`.
  - Add `pub struct CorpusConfig { pub docs_root: PathBuf, pub docs_root_hash: String }` (also `Debug, Clone`).
  - Extend `PathSource` with `EnvVarPlural`, `ConfigFilePlural`, and (if needed for identity tier) `IdentityPathPlural` plus `IdentityPath` (verify whether #778 already added an identity variant; if not, this plan introduces both).
  - Rewrite `resolve_per_agent_docs_root` to walk the six-source priority chain, dedupe by `hash_docs_root`, validate per-source per-path (explicit-source paths must exist; CWD-source missing skipped silently), and assemble the corpora vec.
  - Promote `validate_explicit_path` to return `Result<CorpusConfig, KgConfigError>` (drop the `KgAgentConfig::Enabled` wrapping; the caller wraps when assembling the vec).
  - Update `path_source_exhaustive` test to match all variants.
  - Update inline doc comments for `KgAgentConfig`, `PathSource`, and `resolve_per_agent_docs_root` to describe multi-corpus.
- Test: extend the existing `resolve_per_agent_docs_root` test suite at `kg/config.rs:402+` with multi-source scenarios.

**Approach:**
- Resolution body checks each tier in priority order; first tier yielding ≥1 non-empty path wins.
- **Dual-set warn (symmetric across tiers):** before walking, check identity-plural+identity-singular, env-plural+env-singular, and config-plural+config-singular. For each pair where both are set, emit a `kg_docs_roots_singular_ignored` warn carrying `agent_id`, source tier (`identity` / `env` / `config`), and the ignored singular path. Three tiers can each contribute one warn per init.
- **Dedupe-then-validate** (validate runs against the deduped set, so `["/a", "/missing", "/a"]` validates `/missing` once not twice). Two-tier dedup severity per Key Technical Decisions: literal-string duplicates → `info kg_docs_roots_duplicate_literal`; canonical-collision (distinct strings, same canonicalized hash) → `warn kg_docs_roots_duplicate_canonical` with both source paths.
- **Per-path validation policy diverges by source cardinality:**
  - **Singular sources** (`identity.kg.docs_root`, `MIKA_KG_DOCS_ROOT`, `settings.kg_docs_root`): keep #778's all-or-nothing. Path missing or non-directory → `Err(KgConfigError)`. Agent fails to init.
  - **Plural sources** (`identity.kg.docs_roots`, `MIKA_KG_DOCS_ROOTS`, `settings.kg_docs_roots`): per-path validate-and-skip. For each path: if exists+is-dir, push `CorpusConfig`; else emit `kg_corpus_skipped` warn with `agent_id`, source tier, the bad path, and a running `resolved_count` of paths that did succeed; drop that path. If `corpora.is_empty()` after the loop, log `kg_all_corpora_skipped` warn (loud — every plural path failed) and return `Disabled`.
  - **CWD source:** unchanged from #738 — no validation; `LexicalIngestor` handles missing-path warn-and-skip downstream.
- Empty list at any tier (`Some(vec![])`, `MIKA_KG_DOCS_ROOTS=""`, `MIKA_KG_DOCS_ROOTS=":::"`) falls through to the next source.
- If the cascade produces zero corpora (e.g., CWD doesn't exist and no other source set), return `KgAgentConfig::Disabled { reason: CwdDefaultMissing }` with a `cwd_default_missing` info log — semantically same as today's per-agent `Disabled` path, now with structured reason.

**Technical design** (directional, not implementation spec):

```rust
pub struct CorpusConfig {
    pub docs_root: PathBuf,
    pub docs_root_hash: String,
}

pub enum KgAgentConfig {
    Disabled { reason: DisabledReason },
    Enabled { corpora: Vec<CorpusConfig> },
}

pub enum DisabledReason {
    /// `identity.kg.enabled = false` — operator-explicit opt-out.
    OperatorOptOut,
    /// CWD-default fell through and `<CWD>/docs/solutions` does not exist.
    /// Pre-#798 default for hosts without a configured docs_root.
    CwdDefaultMissing,
    /// Plural source listed N paths; every one failed validation
    /// (skip-and-continue policy left zero survivors).
    AllPathsUnresolvable { source: PathSource, attempted: usize },
}

pub fn resolve_per_agent_docs_root(identity, settings) -> Result<KgAgentConfig, KgConfigError> {
    if !identity.kg.enabled {
        return Ok(Disabled { reason: OperatorOptOut });
    }

    if let Some(roots) = &identity.kg.docs_roots {
        if !roots.is_empty() { return build_corpora(roots, IdentityPathPlural); }
    }
    if let Some(p) = &identity.kg.docs_root {
        return build_corpora(&[p.clone()], IdentityPath);
    }
    if let Ok(s) = env::var("MIKA_KG_DOCS_ROOTS") {
        let paths: Vec<_> = s.split(':').filter(|p| !p.is_empty()).map(PathBuf::from).collect();
        if !paths.is_empty() { return build_corpora(&paths, EnvVarPlural); }
    }
    if let Ok(p) = env::var("MIKA_KG_DOCS_ROOT") {
        return build_corpora(&[PathBuf::from(p)], EnvVar);
    }
    if let Some(roots) = &settings.kg_docs_roots {
        if !roots.is_empty() { return build_corpora(roots, ConfigFilePlural); }
    }
    if let Some(p) = &settings.kg_docs_root {
        return build_corpora(&[p.clone()], ConfigFile);
    }
    let cwd = env::current_dir().unwrap_or_default().join("docs").join("solutions");
    build_corpora(&[cwd], CwdDefault)
}

fn build_corpora(paths, source) -> Result<KgAgentConfig, KgConfigError> {
    let deduped = dedupe_by_hash_with_severity(paths);
    //   ^ emits info kg_docs_roots_duplicate_literal vs warn kg_docs_roots_duplicate_canonical
    let is_plural = matches!(source,
        IdentityPathPlural | EnvVarPlural | ConfigFilePlural);
    let mut corpora = Vec::with_capacity(deduped.len());
    for p in deduped {
        match source {
            CwdDefault => {
                // Per #738: CWD missing is warn-and-skip downstream.
                let hash = hash_docs_root(&p);
                corpora.push(CorpusConfig { docs_root: p, docs_root_hash: hash });
            }
            _ if is_plural => {
                match validate_explicit_path(&p) {
                    Ok(c) => corpora.push(c),
                    Err(e) => warn!(event = "kg_corpus_skipped",
                                    agent_id, source = ?source,
                                    bad_path = %p.display(),
                                    resolved_count = corpora.len(),
                                    error = %e,
                                    "plural-source path skipped; agent will run with remaining corpora"),
                }
            }
            _ /* singular */ => corpora.push(validate_explicit_path(&p)?),
        }
    }
    if corpora.is_empty() {
        if is_plural {
            warn!(event = "kg_all_corpora_skipped", agent_id, source = ?source,
                  attempted = deduped.len(),
                  "every plural-source path failed validation; agent disabled");
            return Ok(Disabled { reason: AllPathsUnresolvable {
                source, attempted: deduped.len() } });
        }
        Ok(Disabled { reason: CwdDefaultMissing })
    } else { Ok(Enabled { corpora }) }
}
```

**Patterns to follow:**
- `crates/mika-agent/src/kg/config.rs:62-131` — existing `resolve_per_agent_docs_root` and `validate_explicit_path` (#778).
- `crates/mika-agent/src/prompt.rs:55-74` — `KgIdentityConfig` shape.

**Test scenarios:**
- Happy path (identity-plural wins everything): `identity.kg.docs_roots = Some(vec!["/a", "/b"])` plus all other tiers also set → returns `Enabled { corpora: [(/a, hash(/a)), (/b, hash(/b))] }` from `IdentityPathPlural`.
- Happy path (identity-singular over global): `identity.kg.docs_root = Some("/c")`, env+settings also set → corpora=[(/c, …)] from `IdentityPath`.
- Happy path (env-plural over env-singular): identity unset, `MIKA_KG_DOCS_ROOTS=/d:/e`, `MIKA_KG_DOCS_ROOT=/f` → corpora=[(/d), (/e)] from `EnvVarPlural`.
- Happy path (env-singular over config): identity unset, `MIKA_KG_DOCS_ROOT=/f`, `settings.kg_docs_roots = Some(vec!["/g"])` → corpora=[(/f)] from `EnvVar`.
- Happy path (config-plural): identity unset, env unset, `settings.kg_docs_roots = Some(vec!["/g", "/h"])` → corpora=[(/g), (/h)] from `ConfigFilePlural`.
- Happy path (config-singular): only `settings.kg_docs_root = Some("/i")` → corpora=[(/i)] from `ConfigFile`.
- Happy path (CWD fallback): all unset, CWD = `/tmp/r`, `/tmp/r/docs/solutions` exists → corpora=[(/tmp/r/docs/solutions, hash, CwdDefault)].
- Edge (CWD missing): all unset, CWD = `/tmp/r`, no `docs/solutions` subdir → `Disabled { reason: CwdDefaultMissing }` with `cwd_default_missing` info log.
- Edge (identity plural empty): `docs_roots = Some(vec![])` falls through to singular.
- Edge (identity dual-set warn): `docs_roots = Some(vec!["/a"])` AND `docs_root = Some("/b")` set; resolver uses plural, emits `kg_docs_roots_singular_ignored` warn at resolver entry with `source="identity"`, `ignored_path="/b"`.
- Edge (env dual-set warn): `MIKA_KG_DOCS_ROOTS=/a` AND `MIKA_KG_DOCS_ROOT=/b` both set → same warn variant with `source="env"`.
- Edge (config dual-set warn): both `kg_docs_roots` and `kg_docs_root` in config → same warn with `source="config"`.
- Edge (env plural separator-only): `MIKA_KG_DOCS_ROOTS=":::"` falls through.
- Edge (dedup literal): `docs_roots = ["/a", "/a"]` returns 1 corpus, emits `kg_docs_roots_duplicate_literal` info log.
- Edge (dedup canonical): `docs_roots = ["/projects/a/docs", "/projects/b/docs"]` where both are symlinks to the same target → returns 1 corpus, emits `kg_docs_roots_duplicate_canonical` warn with both source paths and the canonicalized result.
- Singular hard-error (identity): `docs_roots` unset, `docs_root = Some("/missing")` → `Err(PathNotFound)` (preserves #778 policy).
- Singular hard-error (env): `MIKA_KG_DOCS_ROOT=/missing` → `Err(PathNotFound)`.
- Plural skip-and-continue (identity): `docs_roots = ["/a", "/missing"]` (where `/a` exists) → `Enabled { corpora: [CorpusConfig(/a, …)] }` with one `kg_corpus_skipped` warn for `/missing` and `resolved_count=1`. Agent ingests successfully against `/a`.
- Plural skip-and-continue (env): `MIKA_KG_DOCS_ROOTS=/a:/missing` → same — `/a` becomes a corpus, `/missing` warns.
- Plural all-skipped: `docs_roots = ["/missing-1", "/missing-2"]` → `Disabled { reason: AllPathsUnresolvable { source: IdentityPathPlural, attempted: 2 } }` with two `kg_corpus_skipped` warns plus one `kg_all_corpora_skipped` warn.
- Plural validate-after-dedup: `docs_roots = ["/a", "/missing", "/a"]` → dedup collapses literals; `/missing` gets exactly one `kg_corpus_skipped` warn (not two).
- Asymmetry preserved: `docs_root = "/missing"` (singular) hard-errors; `docs_roots = ["/missing"]` (plural with one element) skips and goes `Disabled { reason: AllPathsUnresolvable }`. Documented behavior.
- Disabled reason — operator opt-out: `identity.kg.enabled = false` → `Disabled { reason: OperatorOptOut }`. Distinct from any path-based disable.
- Disabled: `identity.kg.enabled = false` → `Disabled` regardless of any path config.
- Contract (signature binding): `let _: fn(&Identity, &Settings) -> Result<KgAgentConfig, KgConfigError> = resolve_per_agent_docs_root;`.
- Contract (exhaustiveness): updated `path_source_exhaustive` test matches all `PathSource` variants.

**Verification:**
- `cargo test -p mika-agent kg::config::tests` passes.
- `cargo test -p mika-agent prompt::tests` passes (KgIdentityConfig deserialization).

### Unit 4: Wire multi-corpus into the three startup loops

- [ ] **Unit 4**

**Goal:** Update lexical, extraction, and resolution loops in `server/mod.rs` to iterate `corpora: Vec<CorpusConfig>` instead of destructuring `{ docs_root, docs_root_hash }`. Populate `agent_kg_corpora` per (agent, corpus) before each lexical pass. Keep the existing #738 missing-path warn and the #778 R9 drift-WARN per corpus.

**Requirements:** R5, R6, R7, R8, R10, R11.

**Dependencies:** Units 2, 3.

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs`
  - Replace the `let KgAgentConfig::Enabled { ref docs_root, ref docs_root_hash } = …` destructure at `:771-784`, `:874-886`, and `:1006-1018` with `let KgAgentConfig::Enabled { ref corpora } = &agent_state.kg_config else { /* log reason */ continue; };`. The else-branch logs `event = "lexical_ingest_disabled"` (or `extraction_disabled` / `resolution_disabled`) with a structured `reason` field carrying the `DisabledReason` discriminant — so `OperatorOptOut`, `CwdDefaultMissing`, and `AllPathsUnresolvable` are individually grep-able without parsing free-text reason strings.
  - Inner per-corpus loop in each block.
  - Lexical block: per-corpus drift-WARN (existing logic at `:798-821` runs once per corpus); per-corpus `agent_kg_corpora` insert via `db.register_agent_corpus(agent_id, docs_root_hash, docs_root_path)`; per-corpus `LexicalIngestor::ingest_all`. Each `lexical_ingest_complete` log carries the corpus's hash.
  - Extraction block: shared per-agent `budget` counter drained left-to-right across the inner loop; emit `kg_budget_exhausted` with `roots_remaining` when exhausted; break inner loop.
  - Resolution block: gather all `docs_root_hash` from `corpora` into `Vec<String>` and pass to `SubjectEntityResolver::new(db, llm, hashes, …)` (signature change in Unit 5).
- Test: new file `crates/mika-agent/tests/kg_multi_corpus_startup.rs`.

**Approach:**
- Outer agent loop unchanged. Inner per-corpus loop replaces the previously-singleton ingest call.
- Existing logs gain `corpus_index = i, corpus_count = n` fields for searchability. Per-corpus event emission keeps log volume linear in corpora — fine for typical operator workloads.
- The empty-path guard (CLAUDE.md "If set to an empty string, lexical ingestion skips with a distinct warn") is preserved at the resolver boundary; no per-corpus empty-path branch needed in `server/mod.rs`.
- `agent_kg_corpora` row insertion is non-transactional with the lexical write — if ingest fails, the row stays (semantically correct: the agent IS configured for that corpus, even if the ingest had a problem).

**Execution note:** Test-first. The integration test seeds two agents (one single-corpus, one multi-corpus) and asserts on `agent_kg_corpora` rows + `kg_chunks` distribution before writing the loop change.

**Patterns to follow:**
- `crates/mika-agent/src/server/mod.rs:770-848` — current lexical loop with #738+#778 logs preserved.
- `crates/mika-agent/src/server/mod.rs:873-956` — current extraction loop.
- `crates/mika-agent/src/server/mod.rs:1005-1100` — current resolution loop.

**Test scenarios:**
- Happy path (one agent, one corpus): identity has `[kg].docs_root = "/X"`, no plural. After startup: `agent_kg_corpora` has one row; `lexical_ingest_complete` emitted once. Identical to pre-plan single-root behavior.
- Happy path (one agent, two corpora): identity has `[kg].docs_roots = ["/X", "/Y"]`. After startup: 2 corpus rows; 2 `lexical_ingest_complete` events; `kg_chunks` has rows under both `hash(/X)` and `hash(/Y)`.
- Happy path (two agents share one corpus): both have `[kg].docs_root = "/X"`. After startup: 2 corpus rows (one per agent, both with `hash(/X)`); `kg_chunks` rows under `hash(/X)` shared (idempotent on second run).
- Happy path (mixed): `alice` has `["/X", "/Y"]`, `bob` has `"/X"`. 3 corpus rows. `kg_chunks` for `hash(/X)` shared.
- Drift-WARN (multi-corpus): one corpus has prior data, another is fresh → drift-WARN emits once for the fresh corpus, not for the populated one.
- Error path (one corpus missing on disk in CWD-default identity): falls through resolver; ingest skipped with `docs/solutions not found` info log; no `agent_kg_corpora` row written.
- Plural skip-and-continue path: `[kg].docs_roots = ["/X", "/missing"]`, `/X` exists. Agent's `kg_config` reaches `Enabled { corpora: [/X] }`. Ingest runs on `/X`; `agent_kg_corpora` has one row; `kg_corpus_skipped` warn for `/missing` is observable in startup logs.
- Singular hard-error path: `[kg].docs_root = "/missing"` (no plural set) → `KgConfigError::PathNotFound` at agent init; agent's `kg_config` never reaches `Enabled`; ingest disabled with reason `kg_config_error`. (Preserves #778 contract.)
- Budget (extraction): two corpora, budget=10, first consumes all 10 → second corpus skipped; `kg_budget_exhausted` event fires with `roots_remaining=1`.
- Idempotent restart: re-run startup against an already-populated DB — no duplicate `agent_kg_corpora` rows; no extra `kg_chunks` (v27 idempotency preserved).
- Single-corpus back-compat: pre-plan agents with `[kg].docs_root` set produce one corpus row each; behavior identical to today.

**Verification:**
- `cargo test -p mika-agent --test kg_multi_corpus_startup` passes.
- **Ordering invariant test (startup-sequence-internal, not external):** the assertion is *not* "send a request after startup and check `agent_kg_corpora`" — that's an external invariant that can pass while a query-serving codepath internal to startup (background task, init callback, post-domain-rebuild query) still races. The assertion is **structural**: at the `server/mod.rs` level, prove no codepath capable of reading `agent_kg_corpora` runs before `register_agent_corpus` has committed for every `Enabled` agent's full corpora vec. Concretely: (a) instrument the lexical loop to record a `lexical_loop_complete` timestamp per agent in a test-mode Atomic; (b) instrument every callsite that could read `agent_kg_corpora` (the query tool, the resolver, any background task spawned during startup) to record its first-read timestamp; (c) the test asserts `min(first_read_timestamps) > max(lexical_loop_complete_timestamps)` for every `Enabled` agent. Catches the regression class — a future refactor that moves the lexical loop after a background task spawn fails the test even if the external request-after-startup test passes. The test lives at `crates/mika-agent/tests/kg_multi_corpus_startup.rs::ordering_invariant`.
- Manual smoke (post-merge): set `[kg].docs_roots` on a test agent; restart server; `SELECT * FROM agent_kg_corpora` returns expected rows; `lexical_ingest_complete` events match.

### Unit 5: Multi-corpus subject resolution

- [ ] **Unit 5**

**Goal:** `SubjectEntityResolver::new` takes `Vec<String>` of hashes; pending-entity SQL uses `WHERE docs_root_hash IN (?, …)`. `kg_resolutions_log` semantics stay per-agent.

**Requirements:** R8, R10.

**Dependencies:** Unit 4.

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs`
  - Constructor: `SubjectEntityResolver::new(db, llm, docs_root_hashes: Vec<String>, trace_id)`.
  - Internal pending-entity SQL: `WHERE docs_root_hash IN (?, ?, …)` — dynamic IN-list following `kg/query.rs:932-940`.
  - All callsites updated (only `server/mod.rs:1031-1032` from Unit 4).
- Modify: `crates/mika-agent/src/server/mod.rs` (already touched in Unit 4) — pass the hash vec.
- Test: extend `crates/mika-agent/tests/kg_multi_corpus_startup.rs` with resolution assertions.

**Approach:**
- Pending-entity query joins from `kg_subject_entities` (filtered by IN-list) to `kg_resolutions_log` (per-agent). Existing per-agent UNIQUE constraint on `kg_resolutions_log` unchanged.
- Empty hash list (defensive, should not happen post-Unit-4): resolver short-circuits with zero work.
- Existing #757 budget-guard semantics preserved.

**Patterns to follow:**
- `crates/mika-agent/src/kg/query.rs:932-940` — IN-list construction.
- `crates/mika-agent/src/kg/entity_resolver.rs` existing pending query.

**Test scenarios:**
- Happy path (two corpora): `alice` has `[H1, H2]`, both with subject entities matching domain entries. Resolution writes per-agent `kg_subject_resolutions` rows for entities from both corpora.
- Edge (entity in both corpora — same `entity_key`, different IDs): both subject IDs get separate resolution rows pointing to the same domain entity. Per-agent UNIQUE on `(agent_id, subject_entity_id, domain_entity_id)` — no conflict.
- Back-compat (single corpus): single-element hash list produces identical behavior to today.
- Edge (empty corpora list): resolver short-circuits, zero log entries.
- Budget: extracted from a multi-corpus agent, the resolver respects per-agent budget — Stage-1 exact matches free, Stage-2 LLM calls drain.

**Verification:**
- `cargo test -p mika-agent --test kg_multi_corpus_startup -- resolution` passes.
- `cargo test -p mika-agent kg::entity_resolver::tests` passes.

### Unit 6: Multi-corpus query path

- [ ] **Unit 6**

**Goal:** Change every `WHERE docs_root_hash = ?` predicate in `query.rs` to IN-list. Update `query_knowledge_graph` tool input to accept `docs_root_hashes`. Resolve from `agent_id` via `agent_kg_corpora`.

**Requirements:** R9.

**Dependencies:** Units 2, 4.

**Files:**
- Modify: `crates/mika-agent/src/kg/query.rs`
  - Change `KnowledgeGraphQuery.docs_root_hash: Option<String>` to `KnowledgeGraphQuery.docs_root_hashes: Vec<String>` (default empty); add `From` impl from old singular shape for back-compat in tests.
  - Update Path B (~`:362-385`), Path C (~`:516-650, :1182-1227`), traversal (~`:722-810`), context enrichment (~`:915-1020`) to use IN-lists.
  - Helper `fn build_in_list_placeholders(n: usize) -> String` emitting `"?,?,?"`.
- Modify: `crates/mika-agent/src/tools/query_knowledge_graph.rs`
  - Input schema gains `docs_root_hashes: Option<Vec<String>>`.
  - Resolution priority: `input.docs_root_hashes` > `db.list_agent_corpora(agent_id)` (via Unit 2's helper) > `input.docs_root_hash` (deprecated singular, kept) > empty.
  - Mark `docs_root_hash` deprecated: `#[deprecated(since = "0.6.0", note = "use docs_root_hashes; removed in 0.7.0")]`.
- Test: extend tool tests with multi-corpus scenarios.

**Approach:**
- IN-list construction follows `query.rs:932-940`.
- RRF: Path C already uses RRF inside `Database::hybrid_search`. IN-list filter applies before ranking, so cross-corpus chunks compete in a single ranked list — no second-level RRF.
- Cross-corpus dedup: existing `(layer, entity_id)` dedup at `:298` handles cross-corpus correctly because subject-entity IDs differ across corpora.
- Empty hash list: skip the corpus filter entirely (global query, used by tests).

**Patterns to follow:**
- `crates/mika-agent/src/kg/query.rs:932-940` — IN-list binding.
- `Database::hybrid_search` — existing RRF.

**Test scenarios:**
- Happy path (two-corpus agent, Path C touches both): question routed via Path C returns ranked entries from both; top-K mixes both; RRF ordering correct.
- Happy path (two-corpus traversal): start entity in H1 with edges into H2 entities → traversal returns edges from both.
- Back-compat (one-corpus): IN-list with one hash behaves identically to single equality.
- Edge (empty hash list — global query): no corpus filter; raw test fixtures still work.
- Edge (deprecated singular field): tool still accepts `docs_root_hash`; resolution priority places it last.
- Edge (explicit `docs_root_hashes` set, agent_id also set): explicit hashes win over `agent_kg_corpora` lookup.
- Cross-corpus dedup: same `entity_key` exists in H1 and H2 as separate subject entities → both returned (correct: separate facts from separate corpora).
- Imbalanced corpora: H1 has ~5000 chunks, H2 has ~50. Query a term that matches both. Assert top-K includes ≥1 entry from H2. RRF on heterogeneous corpora can crowd out the smaller corpus when one is materially larger; this scenario locks in the expected behavior. If H2 is never represented, raise it to a known limitation in the plan and consider a per-corpus rank-floor follow-up.

**Verification:**
- `cargo test -p mika-agent kg::query::tests` passes.
- `cargo test -p mika-agent tools::query_knowledge_graph::tests` passes.
- Manual smoke: query against an agent with two corpora returns chunks from both via the dashboard's KG query panel.

### Unit 7: Documentation — `.env.example`, CLAUDE.md, `docs/configuration.md`, `mika-arch` recipe

- [ ] **Unit 7**

**Goal:** Every operator-facing surface that lists config keys mentions `MIKA_KG_DOCS_ROOTS` / `kg_docs_roots` and `[kg].docs_roots`. Document the `mika-arch` identity recipe.

**Requirements:** R12.

**Dependencies:** Units 1, 3 (key shapes final).

**Files:**
- Modify: `.env.example` — add `MIKA_KG_DOCS_ROOTS=` line in the KG block (~lines 41-52, adjacent to `MIKA_KG_DOCS_ROOT`). Comment includes colon-separator note and "global fallback; per-agent identity wins" note.
- Modify: `crates/mika-agent/CLAUDE.md` — extend the `## Knowledge Graph — Docs Root Configuration` section with a sub-section on multi-corpus + `mika-arch` recipe. Recipe shows: identity.toml `[kg]` section, `docs_roots = ["/path1", "/path2", "/path3", "/path4", "/path5", "/path6"]` listing all six platform repos' `docs/solutions/`, and a one-liner on per-agent override precedence over global.
- The same section MUST include three explicit policy callouts:
  1. **Singular vs plural validation asymmetry.** A dedicated table row: "singular `[kg].docs_root` — typo bricks the agent (hard-error). Plural `[kg].docs_roots` — typo skips that one corpus (warn-and-continue)." Cross-reference both directions so operators reading either policy see the other.
  2. **Array order matters under budget pressure.** "If extraction exhausts `MIKA_KG_BATCH_BUDGET`, trailing entries in `docs_roots` are skipped this restart and resume next startup. Place highest-priority corpora first." For `mika-arch` specifically, the recipe block must include a one-liner: "List the repo this agent reasons about most often first (e.g., `mika/docs/solutions` typically belongs at index 0); list reference-only repos like `openclaw/` and `lettabot/` last." Abstract guidance ("budget drains in array order") is insufficient — operators need the concrete corollary.
  3. **Resolution chain priority.** A six-tier table making the precedence explicit (identity-plural > identity-singular > env-plural > env-singular > config-plural > config-singular > CWD).
- Modify: `crates/mika-agent/CLAUDE.md` — extend the schema-version section (`Recent migrations:`) to document v28 with one line on `agent_kg_corpora`.
- Modify: `docs/configuration.md` — add `kg_docs_roots` row to the config-key table near existing `kg_docs_root`, and `MIKA_KG_DOCS_ROOTS` row to the env-var table.
- Modify: `CLAUDE.md` (mika repo root) — add `MIKA_KG_DOCS_ROOTS` to the `Optional (Knowledge Graph LLM):` block immediately after `MIKA_KG_DOCS_ROOT`. Description: "Optional colon-separated list of docs-root paths for multi-corpus agents (e.g., `mika-arch` reasoning across multiple repos). Global fallback; per-agent `[kg].docs_roots` in identity.toml takes precedence. Linux/macOS only."

**Approach:**
- Mirror description style of `MIKA_KG_DOCS_ROOT` from #738 / `[kg].docs_root` from #778.
- The `mika-arch` recipe block documents the *intended* config; the agent itself ships in a future ticket.
- Call out per-agent identity precedence: `~/.mika/agents/mika-arch/identity.toml [kg].docs_roots` is canonical, not the global Settings field.

**Patterns to follow:**
- `crates/mika-agent/CLAUDE.md` `## Knowledge Graph — Docs Root Configuration` (existing, post-#778).
- `.env.example` KG block.
- `docs/configuration.md` table formatting at existing `kg_*` rows.

**Test expectation:** none — pure documentation. CI markdown-lint flags formatting issues.

**Verification:**
- Grep `MIKA_KG_DOCS_ROOTS` across repo; every expected surface has an entry (.env.example, two CLAUDE.mds, docs/configuration.md).
- Grep `[kg].docs_roots` in `crates/mika-agent/CLAUDE.md` finds the recipe.
- The `mika-arch` recipe is paste-ready for the future provisioning ticket.

## System-Wide Impact

- **Interaction graph:** Startup ingest path loops per-`(agent, corpus)` instead of per-agent. Hot path (per-message agent loop) unaffected. Query path reads from `agent_kg_corpora` once per query (cheap indexed lookup).
- **Error propagation:** Per-corpus failures stay isolated for plural sources — a missing `/Y` does not block ingest of `/X` when paths come from a plural source (identity-plural, env-plural, config-plural) or CWD-default. Singular sources (identity-singular, env-singular, config-singular) keep #778's hard-error contract — one bad path → agent fails to init. Asymmetry is intentional and documented. Existing `tracing::warn!` paths preserve message shapes; new variants tagged with `roots_remaining` for budget exhaustion, `corpus_index/corpus_count` for searchability, `resolved_count` for plural-skip diagnostics.
- **State lifecycle risks:** `agent_kg_corpora` rows survive across restarts (intended). Removing a corpus from identity does not delete rows — orphan-pruning is deferred. No transactional risk: each `(agent, corpus)` ingest pass commits independently.
- **API surface parity:**
  - `kg::config::resolve_kg_docs_root` (#738 singular, global) — unchanged.
  - `kg::config::resolve_per_agent_docs_root` (#778) — return shape changes via `KgAgentConfig::Enabled` variant rewrite (pre-1.0 breaking change).
  - `kg::config::KgAgentConfig::Enabled` — `{ docs_root, docs_root_hash }` becomes `{ corpora: Vec<CorpusConfig> }`. Three call sites in `server/mod.rs` updated mechanically; tests in `kg/config.rs:402+` updated.
  - `kg::config::PathSource` — gains plural variants (and possibly identity variants if not present post-#778). Exhaustive-match test enforces hand-update.
  - `kg::entity_resolver::SubjectEntityResolver::new` — signature change `String` → `Vec<String>` for hashes.
  - `kg::query::KnowledgeGraphQuery` — `docs_root_hash: Option<String>` deprecated, `docs_root_hashes: Vec<String>` added.
  - `tools::query_knowledge_graph` input — adds `docs_root_hashes`, keeps `docs_root_hash` deprecated.
  - `Database::register_agent_corpus`, `Database::list_agent_corpora` — new public surfaces.
- **Integration coverage:** `kg_multi_corpus_startup.rs` integration test exercises the full startup loop end-to-end with single+multi corpus agents. Schema convergence test catches migration drift.
- **Unchanged invariants:**
  - v27 `docs_root_hash` PK on shared-corpus tables — untouched.
  - `kg_subject_resolutions` and `kg_resolutions_log` per-agent semantics — untouched.
  - `IngestionOrchestrator::reingest_and_reextract` signature and compound-hook contract — untouched (no live callsite today).
  - `LexicalIngestor`, `SubjectExtractor` constructors — unchanged signatures (each instance still takes one root).
  - `kg::config::resolve_kg_docs_root` (singular global, #738) — unchanged.
  - Schema-version surface (`mika status` output, `mika doctor` output) — gets one digit bump from 27 to 28.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Forgetting a layer in registry / `.env.example` / CLAUDE.md / configuration.md chain produces silent-failure surfaces. | Unit 1 + Unit 7 split makes the registry vs documentation boundary explicit. `get_effective_value` coverage test fails CI on missing arm. Unit 7 verification step grep-checks all surfaces. |
| `KgAgentConfig::Enabled` shape change breaks an undiscovered downstream caller. | Pre-1.0 versioning rule allows the breaking change, but PR description must list every updated call site. Compile errors will surface any missed sites — `cargo build` is the canonical check. Three known sites in `server/mod.rs` plus tests in `kg/config.rs`; grep for `KgAgentConfig::Enabled` before merge. |
| Backfill misses agents that have data but no `kg_subject_resolutions` rows (extraction-disabled agents). | Fallback is the next startup ingest cycle, which writes the row before ingesting. Backfill is a "make first query correct" optimization, not a correctness gate. Documented under Open Questions. |
| Query fan-out across many corpora (e.g., `mika-arch` with 6) materially slows queries. | RRF runs in-memory after SQL phase; SQL phase uses indexed IN-list. 6 hashes is well within SQLite's IN-list performance envelope. v27 hash dedup means IN-list is bounded by *distinct* corpora, not agent×corpus pairs. |
| Per-agent identity override doesn't apply because operators set `MIKA_KG_DOCS_ROOTS` globally and expect it to apply only to one agent. | Documented explicitly that per-agent identity is the canonical place for per-agent overrides; global is fallback. CLAUDE.md and `.env.example` call this out. |
| `agent_kg_corpora` rows accumulate as identity changes; never pruned. | Documented as deferred to `mika kg purge`. Orphan rows are query-correct (stale hashes return empty fan-out branches). |
| Schema convergence drift between fresh-install DDL and upgrade migration. | Convergence test (existing pattern) fails CI on divergence. |
| Operator sets `[kg].docs_roots = []` (empty list) and expects multi-corpus, gets the next-tier fallback silently. | Cascade falls through to `[kg].docs_root` → global → CWD. Resolver doesn't write a "you set empty list" warn (could be noisy if an operator legitimately wants the fallback), but the resolved-source field is observable in startup logs. A future `mika doctor` check could flag intent-vs-state drift. |
| `docs_root_hashes` as `Vec<String>` instead of `Option<Vec<String>>` in the query type means callers can't distinguish "no filter" from "empty corpora set". | Empty Vec is the explicit "no filter" sentinel; tests rely on this. The tool layer has `Option<Vec<String>>` to preserve the absent case at the JSON-input boundary. |
| Per-corpus extraction failure under shared budget leaves later corpora un-extracted. | `kg_budget_exhausted` log includes `roots_remaining` so operators identify pending corpora. Next restart picks up via existing #757 idempotency. |
| `mika-arch` setup hits one missing repo (e.g., `lettabot/` not yet cloned) and operator expects all six corpora to fail loudly. Instead the plural-skip policy ingests the five available and warns for the sixth. | Documented in CLAUDE.md: "plural sources skip missing paths and continue; singular sources hard-error. To force all-or-nothing semantics, use `[kg].docs_root` (singular) or set `enabled=false`." Each `kg_corpus_skipped` warn names the bad path *and* `resolved_count` so the operator immediately sees both what's missing and what worked. |
| Operator misreads the asymmetric policy (singular all-or-nothing vs plural skip) and assumes plural is also fail-loud. | CLAUDE.md `## Knowledge Graph — Docs Root Configuration` section names the asymmetry in a dedicated table row: "singular: typo bricks; plural: typo skips one corpus." Both policies cross-reference each other. |
| Identity dual-set (`docs_roots` AND `docs_root`) silently drops the singular. | Resolver-time warn (`kg_docs_roots_singular_ignored` with agent id, source tier, ignored path) makes the drop observable. Symmetric across identity / env / config tiers. |
| `mika-arch` provisioning lands in a follow-up ticket and may surface a config-shape gap. | Documented in Deferred Tasks. Per `socratic-multi-ticket-milestone-planning`, if grooming surfaces a gap, this plan's CLAUDE.md recipe gets amended in-place. |

## Documentation / Operational Notes

- **Single-corpus operators:** no action required. Existing `[kg].docs_root` and global `MIKA_KG_DOCS_ROOT` / `kg_docs_root` keep working.
- **Multi-corpus per-agent (the `mika-arch` case):** edit `~/.mika/agents/<name>/identity.toml`:

  ```toml
  [kg]
  enabled = true
  docs_roots = [
      "/abs/path/to/mika/docs/solutions",
      "/abs/path/to/mika-cloud/docs/solutions",
      "/abs/path/to/mika-skills/docs/solutions",
      "/abs/path/to/claude-pilot-py/docs/solutions",
      "/abs/path/to/openclaw/docs/solutions",
      "/abs/path/to/lettabot/docs/solutions",
  ]
  ```

  Restart the service. Expect six `lexical_ingest_complete` events for that agent and six `agent_kg_corpora` rows.
- **Multi-corpus global default (every agent gets these N corpora unless overridden):** set `MIKA_KG_DOCS_ROOTS=/path1:/path2:/path3` in `~/.mika/.env` or the service env. Per-agent identity overrides this at agent init.
- **Container deploys:** no action required if running single-corpus. For multi-corpus, set `MIKA_KG_DOCS_ROOTS` in the container env. Mika-cloud Helm `values.yaml` can pass it through without chart-code changes.
- **Post-deploy verification:** `SELECT agent_id, COUNT(*) FROM agent_kg_corpora GROUP BY agent_id;` shows each agent's corpus count. After `mika-arch` ships, its row count should be 6.
- **Schema upgrade:** v27 → v28 is automatic on `Database::open()`. Backfill is bounded (one row per agent-corpus pair derived from existing data); typically <100 rows total.

## Sources & References

- **Origin issue:** [senara-solutions/mika#798](https://github.com/senara-solutions/mika/issues/798)
- **Predecessor plans:**
  - `mika/docs/plans/2026-04-24-005-feat-kg-docs-root-config-plan.md` — singular global config + resolver (#738)
  - `mika/docs/plans/2026-04-24-006-feat-kg-schema-v27-docs-root-hash-plan.md` — v27 schema PK (#786)
  - `mika/docs/plans/2026-04-24-007-feat-kg-data-migration-v27-coalesce-plan.md` — v27 data migration (#787)
  - Per-agent KG identity (#778) — landed in code; no separate plan doc found, but the shipped surface is documented in `crates/mika-agent/CLAUDE.md` `## Knowledge Graph — Docs Root Configuration`.
- **Related issues:**
  - #738 (closed) — singular global config surface
  - #778 (just landed) — per-agent KG identity, single corpus
  - #786 / #787 (closed) — v27 shared-corpus PK, schema enables this without re-key
  - #688 (closed) — KG query tool, this plan adds query-side fan-out
- **Architecture patterns:**
  - `mika/docs/solutions/architecture-patterns/simplified-config-4-source-model.md`
  - `mika/docs/solutions/architecture-patterns/config-key-rename-across-layers.md`
  - `mika/docs/solutions/architecture-patterns/config-key-registry-cli-management.md`
  - `mika/docs/solutions/database-issues/kg-schema-three-layer-sqlite-design.md`
  - `mika/docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md`
  - `mika/docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`
  - `mika/docs/solutions/best-practices/socratic-multi-ticket-milestone-planning-2026-04-21.md`
  - `mika/docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`
- **Anchor files:**
  - `crates/mika-common/src/config.rs:408` (registry), `:537` (`get_effective_value` arm), `:788-796` (`kg_docs_root` field), `:1283-1284` (`test_defaults`), `:1393-1405` (`clean_env`), `:1409` (Debug impl)
  - `crates/mika-agent/src/prompt.rs:55-74` (`KgIdentityConfig`), `:88` (`Identity.kg`), `:2046+` (deserialization tests)
  - `crates/mika-agent/src/kg/config.rs:11-131` (post-#778 resolver, `KgAgentConfig`, `KgConfigError`, `validate_explicit_path`), `:135-216` (`PathSource` and #738 helpers), `:402+` (per-agent resolver tests)
  - `crates/mika-agent/src/server/mod.rs:362-457` (`init_agent` resolver call), `:770-848` (lexical loop), `:873-956` (extraction loop), `:1005-1100` (resolution loop)
  - `crates/mika-agent/src/server/state.rs:52` (`AgentState.kg_config`)
  - `crates/mika-agent/src/db.rs:27` (schema version), `:716-830` (migration dispatch), `:1053-1060` (`agents` table), `:1288-1386` (clean-slate DDL)
  - `crates/mika-agent/src/kg/lexical_ingestor.rs:84-110, :273-329, :391-411` (write paths)
  - `crates/mika-agent/src/kg/subject_extractor.rs:393-` (constructor)
  - `crates/mika-agent/src/kg/entity_resolver.rs:160-, :886, :932` (constructor + write paths)
  - `crates/mika-agent/src/kg/query.rs:30-34, :172-, :298, :362-385, :516-650, :722-810, :915-1020, :932-940, :1182-1227` (query paths)
  - `crates/mika-agent/src/tools/query_knowledge_graph.rs:59-116` (tool input + execution)
  - `crates/mika-agent/tests/eval/kg_fixtures/mod.rs:25` (`PINNED_SCHEMA_VERSION`)
  - `.env.example:41-52` (KG env-var block)
  - `docs/configuration.md` (config-key + env-var tables)
  - `crates/mika-agent/CLAUDE.md` `## Knowledge Graph — Docs Root Configuration`
  - `CLAUDE.md` (mika repo root) `Optional (Knowledge Graph LLM):` block
