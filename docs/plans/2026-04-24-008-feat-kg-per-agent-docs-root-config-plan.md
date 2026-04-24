---
title: "feat(kg): per-agent docs_root config — stops context pollution from wrong corpus"
type: feat
status: active
date: 2026-04-24
issue: senara-solutions/mika#778
branch: feat/778/per-agent-docs-root-config-stops-context
milestone: senara-solutions/mika#17
depends_on_plans:
  - docs/plans/2026-04-24-005-feat-kg-docs-root-config-plan.md
  - docs/plans/2026-04-24-006-feat-kg-schema-v27-docs-root-hash-plan.md
  - docs/plans/2026-04-24-007-feat-kg-data-migration-v27-coalesce-plan.md
---

# feat(kg): per-agent docs_root config — stops context pollution from wrong corpus

## Overview

Eleven mika agents share one hardcoded `docs_root` today. Three of them — `odds-engine-ceo`, `odds-engine-cto`, `odds-engine-quant` — are a separate team on Polymarket trading strategies. They currently ingest the mika platform's engineering docs and build their KG from that corpus. An odds-engine agent answering a trading question draws confidently from platform-engineering entities its extraction happened to produce. The KG has entries, the model has context, the reasoning looks structured — the answer is contaminated. Coherence borrowed from the wrong corpus is anti-correctness masquerading as correctness.

This plan adds a `[kg]` section to per-agent `identity.toml` with `enabled` (default `true`) and `docs_root` (optional `PathBuf`). At agent startup, each agent resolves its docs_root via per-agent → global → CWD-default chain; the resolved path flows into the v27 schema's `docs_root_hash` so agents pointing at the same corpus share extraction, and agents pointing at different corpora get clean isolation. Schema (#786), data migration (#787), and global fallback resolver (#738) are committed upstream; this plan is the config read + startup integration that makes them usable per-agent.

## Problem Frame

The KG system was designed before agent-team boundaries existed. The hardcoded `docs/solutions/` path worked for mika's platform-engineering team but doesn't work the moment an agent belongs to a different domain. The current failure mode is **quality-primary, not cost-primary**: an odds-engine agent's answer can look coherent and structured while drawing its context from the wrong knowledge domain. This is harder to detect than "the answer is missing data" — the presence of irrelevant-but-coherent context masks the absence of relevant context.

Cost reduction happens as a side effect of the shared-corpus model (agents with matching `docs_root` share extraction via #786's `docs_root_hash` keying). But cost is the payoff, not the motivation. The motivation is cleanliness of per-agent reasoning.

This is not about fuzzy entity cleanup, cross-language normalization, or smart corpus discovery. It's about operators being able to say "this agent reads from this tree; that agent reads from that tree" and have the system honor it deterministically.

## Requirements Trace

- **R1.** `Identity` struct (loaded from per-agent `identity.toml`) gains a `kg: KgIdentityConfig` field with `enabled: bool` (default `true`) and `docs_root: Option<PathBuf>`.
- **R2.** Backward compatibility: existing `identity.toml` files without `[kg]` continue to work. Default is `enabled=true, docs_root=None` (fallback to #738's chain).
- **R3.** New `pub fn resolve_per_agent_docs_root` (in `crates/mika-agent/src/kg/config.rs` alongside #738's `resolve_kg_docs_root` and #786's `hash_docs_root`) implements the behavior matrix:
  - `enabled=true`, `identity.kg.docs_root = Some(path)` → validate path exists as directory; hard-error if not; else return the path + hash.
  - `enabled=true`, unset → call #738's global resolver. If the global resolver returns an explicit path (env or `settings.kg_docs_root`) that doesn't exist, hard-error; if it returns the CWD-based default, warn-and-skip per #738.
  - `enabled=false` → return "disabled" variant. No hard-error regardless of path state.
- **R4.** Hard-error policy: **explicit paths (per-agent or global-config) that don't exist fail loud at agent startup**. Warn-and-skip ONLY when falling through to the CWD-based default, matching #738's existing policy for the unset-global case.
- **R5.** Agent-startup failure isolation: a single agent's KG misconfiguration must not halt server startup. The failed agent is skipped with a `warn!` log; other agents start normally.
- **R6.** `enabled=false` skips KG subsystem construction entirely — `LexicalIngestor`, `SubjectExtractor`, `SubjectEntityResolver` / `subject_extraction_start` / `resolve_pending` are NOT constructed for that agent. No writes, no background tasks, no tokio::spawn.
- **R7.** `enabled=false` does NOT delete existing rows. Rows in the shared-corpus tables belong to the `docs_root_hash`, not the agent; deletion requires #779's CLI.
- **R8.** Shared-corpus semantics via `docs_root_hash`: two agents with the same resolved `docs_root` → identical hash → shared row set in the v27 shared-layer tables per #786.
- **R9.** Startup drift WARN (informational, not blocking): if an agent's resolved `docs_root_hash` is not present in any shared-layer table at agent startup, emit a warn noting "first-run ingestion will populate". This is the existing behavior for fresh hashes; the warn makes it visible.
- **R10.** Tests cover the full behavior matrix (3 enabled-states × 2 docs_root-set-states × 3 path-exists-states where applicable), the hard-error policy for explicit-path cases, and agent isolation (agent A's misconfig doesn't prevent agent B from starting).
- **R11.** Documentation: `crates/mika-agent/CLAUDE.md` KG section + root `CLAUDE.md` + `docs/configuration.md` all describe `[kg]` section schema, behavior matrix, and hard-error policy. `DEFAULT_IDENTITY` constant (`mika-common/src/home.rs:264-266`) and the `well_known_agents.rs` identity-writer add a commented `[kg]` stanza showing the available fields.

## Scope Boundaries

- **Non-goal:** schema DDL. #786 owns.
- **Non-goal:** data migration. #787 owns.
- **Non-goal:** global fallback resolver or `kg_docs_root` `Settings` field. #738 owns.
- **Non-goal:** KG CLI (`mika kg status / purge / validate`). #779 owns. The disabled-agent log line in this plan points at `#779`'s `mika kg purge --agent <name>` for cleanup, but that command lands in #779.
- **Non-goal:** `kg_agent_state` tracking table. Derivable on demand; YAGNI. Committed in ticket.
- **Non-goal:** multi-path docs_root. Single `PathBuf`. Committed in ticket.
- **Non-goal:** runtime DB-backed toggle for `enabled`. File-only. Future enhancement if ever needed (runtime toggles aren't on the roadmap).
- **Non-goal:** fuzzy-matching across near-duplicate entities within a shared corpus. Future ticket post-milestone-#17.

### Deferred to Separate Tasks

- KG CLI for status/purge/validate → **#779** (next ticket in milestone).
- Compound doc on cross-corpus-contamination-as-correctness-bug → post-merge `/ce:compound`. This is novel institutional knowledge (research confirmed no prior doc in this family); worth codifying.

## Context & Research

### Relevant code and patterns

- **`Identity` struct + `load_identity`:** `crates/mika-agent/src/prompt.rs:46-93`. Struct uses `#[serde(default = ...)]` on every field. `load_identity` is **infallible** — on missing or malformed TOML, returns `Identity::default()` silently. Adding `#[serde(default)] pub kg: KgIdentityConfig` deserializes cleanly against existing (kg-less) identity.toml files.
- **Per-agent init-loop with failure isolation:** `crates/mika-agent/src/server/mod.rs:653-680`. `for name in &agent_names { match init_agent(...).await { Ok(state) => agents.insert(...), Err(e) => warn!("failed to initialize agent, skipping") } }`. This is the hook for #778's hard-error: KG config validation happens inside `init_agent`; on `Err`, the existing skip path takes over. No new isolation machinery needed.
- **KG subsystem construction loops:** `server/mod.rs:785-810` (`LexicalIngestor`), `:842-911` (`SubjectExtractor` + `subject_extraction_start`), `:961-1029` (`SubjectEntityResolver` + `resolve_pending`). Each iterates `for (agent_name, agent_state) in &agents { ... }`. **Agents that failed `init_agent` aren't in the HashMap** — they're naturally skipped. When `enabled=false`, we want the same "not in the iteration" effect; easiest path: store `kg_config: KgAgentConfig` on `AgentState`, and each KG loop inspects it + skips when `Disabled`.
- **KG loop's existing "disabled subsystem" log shape:** `server/mod.rs:913-926` already has `None => info!("subject extraction disabled — set MIKA_KG_EXTRACTION_MODEL ... to enable")`. #778 extends with `KgAgentConfig::Disabled => info!("subject extraction disabled — identity.toml [kg].enabled=false")` for consistency.
- **`Settings` access at spawn:** `run_server(settings: &Settings)` at `server/mod.rs:552` has `settings` in scope for the entire function. `init_agent` builds `agent_settings` via `Settings::load_for_agent(global_home, agent_home)` at line 354 — this is where `#778`'s resolver is called (inside `init_agent`, using `agent_settings` for global-fallback access).
- **Startup config-error patterns:** `server/mod.rs:594-611` shows `anyhow::bail!`/`ok_or_else` for process-level config errors (e.g., `MIKA_ROUTING_URL is required for server mode`). `init_agent` line 354: `Settings::load_for_agent(...)?` — `?` bubbles to init-loop's skip path. #778's `KgConfigError` uses the latter (per-agent skip), not the former (process halt).
- **`DEFAULT_IDENTITY` constant + well-known-agent writer:** `crates/mika-common/src/home.rs:264-266` is the canonical template; `crates/mika-agent/src/well_known_agents.rs:143-148` writes via `format!("name = \"{}\"\nemoji = \"{}\"\n", ...)`. Both need updating to include a commented `[kg]` stanza.
- **`kg/config.rs` post-#738/#786:** module created by #738 with `resolve_kg_docs_root(&Settings) -> PathBuf` and `hash_docs_root(&Path) -> String` from #786. #778 adds `resolve_per_agent_docs_root(&Identity, &Settings) -> Result<KgAgentConfig, KgConfigError>` + the `KgAgentConfig` enum + `KgConfigError` enum using `thiserror` (crate convention per root `CLAUDE.md`).
- **`kg/mod.rs` module declarations:** currently lists `chunker, domain_builder, entity_resolver, ingestion_orchestrator, lexical_ingestor, query, subject_extractor`. #738 adds `pub mod config;`; #778 is a no-op on that file (just extends the existing module).

### Institutional learnings

- **`docs/solutions/architecture-patterns/well-known-agent-config-toml-override.md`** — direct precedent for per-agent TOML config extension. Shows the split between `identity.toml` (identity) and `config.toml` (settings) and the `WellKnownAgent.identity_toml` bundling pattern. The ticket committed to `identity.toml` ownership of `[kg]`; this plan honors that, and noting the architectural alternative (putting `[kg]` in `config.toml`) in Key Technical Decisions.
- **`docs/solutions/architecture-patterns/simplified-config-4-source-model.md`** — establishes the `#[serde(default)]` pattern as the single source of compiled-in defaults. `[kg]` section uses this exclusively; no `config/default.toml` equivalent to update.
- **`docs/solutions/architecture-patterns/per-agent-dotenv-config-injection.md`** — reminder that server-mode runs multiple agents in one process; per-agent config reading must not mutate process env or shared state. `resolve_per_agent_docs_root` operates on references, returns owned values — safe by construction.
- **`docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md`** — the closest shape analogue for #778's mixed policy. Uses `SnippetLoadResult` enum with variants for "missing vs invalid vs oversized vs ReadError+required vs ReadError+optional". #778's `KgAgentConfig` enum + `KgConfigError` enum follow the same typed-result pattern — typed returns > sentinel strings. Explicit decision under D-numbers for both hard-error and warn-and-skip branches.
- **`docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md`** — "disabled means NOT in the registry." For #778: `enabled=false` means the agent doesn't appear in the KG construction loops at all (via `KgAgentConfig::Disabled` early-exit), not "in the loop with a flag that filters at call time". Eviction-at-construction matches.
- **`docs/solutions/best-practices/kg-domain-graph-startup-projection-2026-04-22.md`** — established KG subsystem policy is "WARN and continue" for runtime errors. #778 deliberately breaks this for **explicit misconfiguration** of `docs_root`. The policy split ("fail-open on runtime errors; fail-loud on config errors") is a D-decision.
- **`docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`** — names per-agent × per-docs_root fan-out as a cost amplifier. #778 is its correctness-primary counterpart (same root cause, different failure mode). Worth cross-referencing in the compound doc post-merge.
- **`docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`** — the retrospective behind this milestone. Reinforces: **empty results from the dependency source should fail loud, not fall back silently.** Supports #778's hard-error policy for explicit-but-invalid `docs_root`.
- **No prior doc on context-pollution as correctness bug.** The compound doc spawned by #778 will be the first. Flag in Open Questions (Deferred to post-merge).

## Key Technical Decisions

- **`KgAgentConfig` enum with two variants.** `Disabled` (skip KG subsystem entirely) and `Enabled { docs_root: PathBuf, docs_root_hash: String }` (constructors get both values). The enum enforces "either you have a path + hash, or you have nothing" — no partial states. `Option<PathBuf>` would conflate "disabled" with "enabled but unknown path", so an enum is clearer.

- **`KgConfigError` via `thiserror` in `kg/config.rs`.** Variants: `PathNotFound { path: PathBuf, source: std::io::Error }`, `NotADirectory { path: PathBuf }`. `thiserror` derive matches the crate convention per root `CLAUDE.md`; prior KG code used manual `impl Display + Error` or inline `Error(String)` patterns — this is an upgrade toward convention.

- **Mixed hard-error / warn-and-skip policy.** Hard-error when an explicitly-set path (per-agent or global) doesn't exist. Warn-and-skip ONLY when falling through to the CWD-based default (matches #738's existing policy for the unset case). Rationale: operator intent. If you set a path, you meant it to exist and the KG would be catastrophic to ingest from the wrong place silently; if you didn't set anything, the container-friendly default is an expected fallback.

- **Per-agent failure isolation via existing skip pattern.** No new machinery. `init_agent` is already `Result<AgentState>` with caller-side isolation at `server/mod.rs:672-677`. #778 inserts the `resolve_per_agent_docs_root` call inside `init_agent`; on `Err`, the existing `warn!+skip` catches it. Agents Y and Z start normally when agent X's `[kg]` config is bad.

- **`resolve_per_agent_docs_root` called inside `init_agent`, result cached on `AgentState`.** Alternative considered: call the resolver inside each KG construction loop (lexical, extraction, resolution). Rejected — three call sites × re-validation per loop = three log warnings for the same agent, plus three chances for race between `identity.toml` edits and different loops observing different configs. Caching the resolved `KgAgentConfig` on `AgentState` is cheap (one enum per agent) and keeps validation atomic at init.

- **Policy split: fail-loud for operator-set misconfig; fail-open for runtime errors.** The established KG subsystem norm is "WARN and continue" for runtime errors (unhashable doc, LLM timeout, malformed chunk, extraction phase crash) — documented in `docs/solutions/best-practices/kg-domain-graph-startup-projection-2026-04-22.md`. **#778 introduces fail-loud for operator-set misconfiguration of `docs_root` specifically.** The distinction is intent: runtime errors are the system's problem and the user didn't ask for anything specific; a missing explicit `docs_root` is the operator's assertion that something must exist, which if false is a typo or a deploy mistake that would silently contaminate the KG with the wrong corpus (the exact failure mode this ticket is fixing). Fail-loud is the only honest response. The `kg-domain-graph-startup-projection` precedent stays intact for every other KG failure surface — only the config-read path breaks with it, and the break is deliberate and scoped. A reviewer asking "doesn't this violate the warn-and-continue norm?" should be answered: "yes, for exactly one specific surface, because the failure mode at that surface is silent corruption of the agent's reasoning context, which WARN-and-continue can't catch."

- **`docs_root_hash` computed at resolver, cached on `AgentState.kg_config`. Constructor signature generalizes, doesn't replace.** Alternative considered: let `LexicalIngestor::new` / `SubjectExtractor::new` compute the hash from `docs_root` (per #786's Unit 3 current contract). Rejected — once #778 lands, the hash is a first-class per-agent identifier used by three subsystems AND by the drift-WARN check; computing once at resolver and passing precomputed through to constructors avoids three duplicate calls to `hash_docs_root` per agent startup. **Coordination with #786's Unit 3 constructors: committed to generalize, not replace.** `LexicalIngestor::new` and `SubjectExtractor::new` gain an optional `docs_root_hash: Option<String>` parameter (or equivalent — new builder method, additional-arg form): when `Some`, caller provides the hash; when `None`, the constructor falls back to computing it internally via `hash_docs_root(&docs_root)`. #786's test scenarios that assert "both instances compute the same hash" pass unchanged (they use `None`; the constructor computes). #778's new call sites pass `Some(hash)` where the resolver already computed it. Neither ticket's tests break, and the hash-computation contract remains a single-source-of-truth via `hash_docs_root`. Alternative (replace #786's contract entirely, require `docs_root_hash` from every caller) rejected — would require updating #786's test scenarios as part of #778's Unit 4, which is extra scope for no clear safety gain over generalization.

- **Eviction-at-construction for `enabled=false`.** KG loops check `match agent_state.kg_config { Disabled => continue, Enabled { .. } => ... }`. Disabled agents don't get `LexicalIngestor`/`SubjectExtractor`/`SubjectEntityResolver` constructed, don't get `subject_extraction_start` or `resolve_pending` spawned. Zero KG work for disabled agents. Rows in shared tables are preserved untouched — they belong to `docs_root_hash`, not the disabled agent.

- **Identity loader stays infallible.** `load_identity` continues to return `Identity` directly with silent fallback to `Identity::default()` on parse errors. This preserves existing behavior for agents that have malformed top-level TOML (e.g., typo in `name`). The `[kg]` validation is a separate consumer-level step in `resolve_per_agent_docs_root`, which CAN fail. Rationale: coupling identity loading to KG validation would change semantics for existing agents and create unnecessary blast radius.

- **`[kg]` ownership: `identity.toml` (per ticket), not `config.toml`.** The ticket committed this. An architectural case could be made for `config.toml` (closer to agent-scoped configuration than agent identity), but the ticket author (Vincent) picked `identity.toml` and the decision stands. Plan notes the alternative under Open Questions as the one architectural question that was actively debated but resolved.

- **Drift WARN is a separate step at the KG loop level, not at resolver level.** The resolver returns the hash; the KG construction loop runs a `SELECT COUNT(*) FROM kg_chunks WHERE docs_root_hash = ? LIMIT 1` query pre-ingestion and emits the warn if count is 0 AND the agent is enabled. Keeps the resolver pure (no DB access).

## Open Questions

### Resolved during planning

- **Q: `Option<PathBuf>` vs enum for resolver return?** → Enum (`KgAgentConfig::{Disabled, Enabled}`). Clearer semantics, no conflation.
- **Q: Error type via `thiserror` or manual impl?** → `thiserror`. Crate convention.
- **Q: Hard-error policy split?** → Explicit paths hard-error; CWD-default warn-and-skip per #738. Committed.
- **Q: Identity loader fallibility?** → Stays infallible. Validation separates.
- **Q: `[kg]` in `identity.toml` or `config.toml`?** → `identity.toml` per ticket.
- **Q: Hash computation at resolver or at constructor?** → At resolver, cached on `AgentState`. Constructor generalizes to accept `Option<String>` — caller provides precomputed hash OR the constructor falls back to computing internally via `hash_docs_root`. #786's tests (which use the compute-internally path) pass unchanged.
- **Q: How does #778 classify "explicit global" vs "CWD default" source?** → Via #738 API amendment: `resolve_kg_docs_root` returns `(PathBuf, PathSource)` tuple with `EnvVar / ConfigFile / CwdDefault` variants. #778 matches on source to apply the correct policy. #786's single consumer call site updates to destructure the tuple — one-line change.
- **Q: Per-agent failure isolation machinery?** → Existing `init_agent` skip pattern. No new machinery.
- **Q: Drift WARN placement?** → KG loop level (needs DB access), not resolver.

### Deferred to implementation

- **Exact form of the `Option<String>` hash parameter on `LexicalIngestor::new` / `SubjectExtractor::new`.** Options: (a) add a third param `docs_root_hash: Option<String>`; (b) add a builder method `.with_docs_root_hash(hash)` to the existing constructor; (c) add a separate `::new_with_hash(...)` constructor alongside `::new`. Implementer picks. (a) is the simplest and most consistent with existing call sites. Whichever form wins, the contract is: `None` → compute internally via `hash_docs_root`; `Some(hash)` → use as-is.
- **Where the drift-WARN query runs.** Options: (a) inside `LexicalIngestor::new` as a first-call side effect; (b) in the KG loop, once per agent, before the `LexicalIngestor::new` call. Implementer picks; (b) is cleaner separation.
- **Whether `KgAgentConfig::Enabled.docs_root_hash` is a `String` or a dedicated newtype.** `String` matches #786's `pub fn hash_docs_root(&Path) -> String` return; newtype (e.g., `DocsRootHash`) is more type-safe but more code. Not worth a newtype until a second hashing concept exists.
- **`KgIdentityConfig` struct-level default via `#[derive(Default)]` vs field-level `#[serde(default)]`.** Both work. Implementer picks whichever gives the cleaner `serde` trait set.

## Implementation Units

### Unit 1: `KgIdentityConfig` struct + `Identity.kg` field

- [ ] **Unit 1**

**Goal:** Extend the `Identity` struct with a `kg: KgIdentityConfig` field. Default: `enabled=true, docs_root=None`. Existing `identity.toml` files without `[kg]` continue to deserialize successfully.

**Requirements:** R1, R2.

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-agent/src/prompt.rs` — add `KgIdentityConfig` struct and `Identity.kg` field.
- Test: inline `#[cfg(test)] mod tests` in `prompt.rs`.

**Approach:**
- `KgIdentityConfig` shape:
  ```rust
  #[derive(Debug, Deserialize, Clone, Default)]
  pub struct KgIdentityConfig {
      #[serde(default = "default_kg_enabled")]
      pub enabled: bool,
      pub docs_root: Option<PathBuf>,
  }
  fn default_kg_enabled() -> bool { true }
  ```
- Add to `Identity`: `#[serde(default)] pub kg: KgIdentityConfig,`.
- `Default` impl on `KgIdentityConfig` returns `enabled=true, docs_root=None` (via the `default_kg_enabled()` function + `Option::default()`). Matches ticket's committed default.

**Patterns to follow:**
- Existing `Identity` fields at `crates/mika-agent/src/prompt.rs:46-93` (all use `#[serde(default = "...")]` or `#[serde(default)]`).
- Existing `ReflectionConfig` struct in the same file (likely similar shape).

**Test scenarios:**
- Happy path (empty TOML): `toml::from_str::<Identity>("")` → `Identity { name: <default>, emoji: <default>, reflection: None, kg: KgIdentityConfig { enabled: true, docs_root: None } }`.
- Happy path (only name+emoji — matches existing agents): `toml::from_str::<Identity>(r#"name = "Mika"\nemoji = "✦"\n"#)` → `kg` field defaults cleanly.
- Happy path (kg with both fields): `toml::from_str::<Identity>(r#"name = "X"\n[kg]\nenabled = true\ndocs_root = "/path"\n"#)` → `kg.enabled=true, kg.docs_root=Some("/path")`.
- Happy path (kg with only enabled): `[kg]\nenabled = false` → `kg.enabled=false, kg.docs_root=None`.
- Happy path (kg with only docs_root): `[kg]\ndocs_root = "/path"` → `kg.enabled=true, kg.docs_root=Some("/path")`.
- Edge: malformed `[kg]` (e.g., `docs_root = 42`) → TOML deserialization error. Because `load_identity` is infallible and falls back to `Identity::default()`, this means the agent gets `kg=default` silently. Documented behavior; acceptable — malformed top-level `identity.toml` has always silently fallen back.

**Verification:**
- `cargo test -p mika-agent prompt::tests::kg_identity_config` passes all scenarios.
- `cargo build -p mika-agent` passes.

### Unit 2: `KgAgentConfig` enum, `KgConfigError`, `resolve_per_agent_docs_root`

- [ ] **Unit 2**

**Goal:** Add the per-agent resolver to `crates/mika-agent/src/kg/config.rs` (alongside #738's `resolve_kg_docs_root` and #786's `hash_docs_root`). Implements the full behavior matrix with typed return.

**Requirements:** R3, R4.

**Dependencies:** Unit 1. Also depends on #738 having landed (`kg/config.rs` module exists with `resolve_kg_docs_root`) and #786 having landed (`hash_docs_root` exists). Both are in the DAG before this ticket.

**Files:**
- Modify: `crates/mika-agent/src/kg/config.rs` — add `KgAgentConfig`, `KgConfigError`, `resolve_per_agent_docs_root`.
- Test: inline `#[cfg(test)] mod tests` in the same file.

**Approach:**
- `KgAgentConfig` enum:
  ```rust
  #[derive(Debug, Clone)]
  pub enum KgAgentConfig {
      Disabled,
      Enabled { docs_root: PathBuf, docs_root_hash: String },
  }
  ```
- `KgConfigError` enum via `thiserror`:
  ```rust
  #[derive(Debug, thiserror::Error)]
  pub enum KgConfigError {
      #[error("docs_root path does not exist: {path}")]
      PathNotFound { path: PathBuf, #[source] source: std::io::Error },
      #[error("docs_root path is not a directory: {path}")]
      NotADirectory { path: PathBuf },
  }
  ```
- `resolve_per_agent_docs_root(identity: &Identity, settings: &Settings) -> Result<KgAgentConfig, KgConfigError>`:
  1. If `identity.kg.enabled == false` → return `Ok(KgAgentConfig::Disabled)`. Do NOT validate docs_root in this branch.
  2. If `identity.kg.docs_root = Some(path)` (explicit per-agent path):
     - Check `path.try_exists()` — if `Err` or `Ok(false)` → `Err(PathNotFound { path, source })`.
     - Check `path.is_dir()` → if not, `Err(NotADirectory { path })`.
     - Else: compute `hash_docs_root(&path)` and return `Ok(Enabled { docs_root: path, docs_root_hash: hash })`.
  3. If per-agent path unset: call `resolve_kg_docs_root(settings)` (#738's resolver) to get the global-resolved path.
     - If the resolved path came from an explicit source (env var or `settings.kg_docs_root`), validate existence → hard-error on miss (same as step 2).
     - If the resolved path is the CWD-based default, skip existence check here — pass it through. The downstream `LexicalIngestor` existence check (#738) surfaces a warn if the default path doesn't exist. Match #738's policy exactly.
  4. Compute `hash_docs_root(&path)` on the resolved path and return `Ok(Enabled { .. })`.

**How to distinguish "explicit global" from "CWD default"** inside the resolver: `resolve_kg_docs_root` currently returns `PathBuf` — a single value with no source-of-origin info. **Committed answer: bump #738's API to return `(PathBuf, PathSource)` where `PathSource` is an enum with `EnvVar / ConfigFile / CwdDefault`.** `resolve_per_agent_docs_root` then matches on the source to apply the correct policy (hard-error on `EnvVar` or `ConfigFile` if path doesn't exist; warn-and-skip passthrough on `CwdDefault`).

Rationale: the alternative ("#778 inspects env var + settings field directly to classify") couples #778's policy logic to #738's resolution logic. If #738 ever gains a third source (workspace-level config, operator override, whatever), #778's classifier silently misclassifies it as `CwdDefault` and triggers the wrong policy. #738 hasn't deployed yet — per the same migration-immutability reasoning used in #786/#787 coordination, its API can still move. The contract exists specifically to be consumed by #778 and #786; exposing source-of-origin is what the contract wants to look like.

**This requires an amendment to #738's plan (Unit 2):** bump `resolve_kg_docs_root(&Settings) -> PathBuf` to `resolve_kg_docs_root(&Settings) -> (PathBuf, PathSource)` and add the `PathSource` enum adjacent to the function. Three extra lines. Must land before #778 dispatches. Surface to Vincent at peer review — the amendment is trivial but it's a cross-ticket change that must be explicit.

Downstream impact: #786's Unit 3 currently calls `resolve_kg_docs_root(&settings)` at the agent-startup site and uses the returned `PathBuf` directly to construct `LexicalIngestor`. Post-amendment it destructures the tuple: `let (docs_root, _source) = resolve_kg_docs_root(&settings);`. One-line change to #786's call site — not a contract break.

**Patterns to follow:**
- `SnippetLoadResult` typed-return pattern from `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md`.
- #738's `resolve_kg_docs_root` placement and style in the same file.
- `thiserror` derive pattern (root `CLAUDE.md` convention).

**Test scenarios:**
- Happy path (`enabled=false`, any docs_root state): returns `Ok(Disabled)`. Does not touch filesystem.
- Happy path (`enabled=true`, `docs_root=Some(valid-path)`): returns `Ok(Enabled { docs_root, hash })` where hash matches `hash_docs_root(&docs_root)`.
- Hard-error (`enabled=true`, `docs_root=Some(nonexistent-path)`): returns `Err(PathNotFound { path, .. })`.
- Hard-error (`enabled=true`, `docs_root=Some(file-not-dir)`): returns `Err(NotADirectory { path })`.
- Happy path (`enabled=true`, per-agent unset, env `MIKA_KG_DOCS_ROOT` set and valid): returns `Ok(Enabled { resolved_from_env, hash })`.
- Hard-error (`enabled=true`, per-agent unset, env `MIKA_KG_DOCS_ROOT` set but invalid): returns `Err(PathNotFound)`.
- Happy path (`enabled=true`, per-agent unset, global `settings.kg_docs_root=Some(valid)`): returns `Ok(Enabled { .. })`.
- Hard-error (`enabled=true`, per-agent unset, global `settings.kg_docs_root=Some(invalid)`): returns `Err(PathNotFound)`.
- Warn-and-skip-handoff (`enabled=true`, all unset, CWD-default invalid): returns `Ok(Enabled { cwd_default, hash })` — resolver does NOT hard-error; downstream warn-and-skip in `LexicalIngestor` kicks in. Test asserts no error and the returned path matches `CWD/docs/solutions`.
- Contract: compile-time signature binding — `let _: fn(&Identity, &Settings) -> Result<KgAgentConfig, KgConfigError> = resolve_per_agent_docs_root;` in a test. Same approach as #738's Unit 2 and #786's Unit 1.

**Verification:**
- `cargo test -p mika-agent kg::config::tests::resolve_per_agent_docs_root` passes all scenarios.
- `rg "fn resolve_per_agent_docs_root" crates/mika-agent/src/` finds the function.

### Unit 3: `init_agent` integration + `AgentState.kg_config`

- [ ] **Unit 3**

**Goal:** Call `resolve_per_agent_docs_root` inside `init_agent`, store the result on `AgentState`. On `Err`, propagate via `?` — existing init-loop skip pattern at `server/mod.rs:672-677` handles isolation.

**Requirements:** R5 (isolation), part of R3 (integration point).

**Dependencies:** Unit 2.

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs` — add `kg_config: KgAgentConfig` field to `AgentState`; call `resolve_per_agent_docs_root` inside `init_agent` (around line 354 where `agent_settings` is loaded); propagate errors.
- Test: extend integration tests at `crates/mika-agent/tests/` or inline in `server/mod.rs`.

**Approach:**
- `AgentState` gets a new field: `pub kg_config: KgAgentConfig`.
- Inside `init_agent` (line 354 area), after `let agent_settings = Settings::load_for_agent(global_home, agent_home)?;`:
  ```rust
  let kg_config = resolve_per_agent_docs_root(&identity, &agent_settings)
      .with_context(|| format!("failed to resolve [kg] config for agent {name}"))?;
  ```
- Existing init-loop at `server/mod.rs:672-677`: on `Err(e)`, `warn!("failed to initialize agent, skipping")` — already covers the hard-error case. No new code at the loop level.
- If `kg_config` is `Disabled`, `AgentState.kg_config` reflects it; the KG loops in Unit 4 consume.

**Patterns to follow:**
- Existing `init_agent` error propagation via `?` and `.with_context(...)`.
- `AgentState` struct definition (same file; grep for its definition).

**Test scenarios:**
- Happy path: agent with `[kg] enabled=true, docs_root=<valid-path>` → `init_agent` returns `Ok(AgentState { kg_config: Enabled { .. }, .. })`.
- Happy path: agent without `[kg]` (existing agents) → `Ok(AgentState { kg_config: Enabled { cwd_default, hash } })`.
- Happy path: agent with `[kg] enabled=false` → `Ok(AgentState { kg_config: Disabled })`.
- Error path: agent with `[kg] docs_root="/nonexistent"` → `init_agent` returns `Err(PathNotFound)`; init-loop catches via existing `warn!+skip`.
- Integration (multi-agent): init 3 agents; one has bad `[kg]`. Assert: 2 agents are in `agents` HashMap, 1 is not; server continues to run.

**Verification:**
- `cargo test -p mika-agent server::tests::init_agent_kg_config` passes.
- `cargo build --workspace` passes.

### Unit 4: KG construction loops consume `AgentState.kg_config`

- [ ] **Unit 4**

**Goal:** Update the three KG construction loops at `server/mod.rs:785-1029` to consult `AgentState.kg_config`. When `Disabled`, skip that agent's entire KG subsystem (no `LexicalIngestor::new`, no `subject_extraction_start` spawn, no `resolve_pending` spawn). When `Enabled`, pass both `docs_root` and `docs_root_hash` to constructors.

**Requirements:** R6, R7, R8, R9.

**Dependencies:** Unit 3.

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs` — three loops at `:785-810`, `:842-911`, `:961-1029`.
- Modify: `crates/mika-agent/src/kg/lexical_ingestor.rs` — `LexicalIngestor::new` accepts `docs_root_hash: String` alongside `docs_root: PathBuf`. Currently (post-#786 Unit 3) it takes `docs_root` and computes the hash internally; #778 switches to caller-provides-hash.
- Modify: `crates/mika-agent/src/kg/subject_extractor.rs` — `SubjectExtractor::new` same adjustment.
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs` — `SubjectEntityResolver::new` — research showed it does NOT currently take `docs_root`, so it may not need `docs_root_hash` either. Verify during implementation: if the resolver reads from shared tables by `docs_root_hash`, it needs the hash; if it only reads per-agent tables, it doesn't. Plan defers to implementation.

**Approach:**
- Each KG loop pattern becomes:
  ```rust
  for (agent_name, agent_state) in &agents {
      let KgAgentConfig::Enabled { docs_root, docs_root_hash } = &agent_state.kg_config else {
          info!(
              agent = agent_name.as_str(),
              reason = "identity.toml [kg].enabled=false",
              "KG lexical ingestion disabled"
          );
          continue;
      };
      // ... construct LexicalIngestor::new(db, docs_root.clone(), Some(docs_root_hash.clone()), None);
  }
  ```
- **Multi-reason disabled-log distinction.** Each of the three KG loops (lexical, extraction, resolution) has an existing "disabled because subsystem prerequisite is missing" branch (e.g., `None => info!("subject extraction disabled — set MIKA_KG_EXTRACTION_MODEL ... to enable")` at `server/mod.rs:913-926`). #778 adds a **second** disabled branch — `KgAgentConfig::Disabled` — with a different cause. **Both branches must be distinguishable by an operator grepping logs.** Use a structured `reason` field (as in the snippet above) with two values: `"no provider configured"` (existing; e.g., `MIKA_KG_EXTRACTION_MODEL` unset) and `"identity.toml [kg].enabled=false"` (new, from #778). Message stays `"KG <phase> disabled"` across both branches. This way `rg 'disabled' logs/` groups both causes under one message and `rg 'reason=' logs/` splits them when debugging.
- Drift-WARN check, inside the `Enabled` branch, before `LexicalIngestor::new`:
  ```rust
  let existing = db.count_chunks_for_docs_root_hash(docs_root_hash).await?;  // new helper method
  if existing == 0 {
      warn!(
          agent = agent_name.as_str(),
          docs_root = %docs_root.display(),
          docs_root_hash = docs_root_hash.as_str(),
          "agent docs_root_hash has no matching rows in shared corpus — first-run ingestion will populate"
      );
  }
  ```
  Minor helper method needed on `Database` or `AsyncDatabase`: `count_chunks_for_docs_root_hash(&self, hash: &str) -> Result<u64>`. Add in this unit.
  
  **Write-order invariant on the drift-WARN proxy.** The helper uses `kg_chunks` row count as a proxy for "has this `docs_root_hash` had any prior ingestion?" This is only a reliable proxy IF `kg_chunks` is written BEFORE (or atomically with) `kg_extractions` in the composed-write transaction from `docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md`. Verify during implementation: `lexical_ingestor.rs`'s composed-write writes `kg_chunks` first (or in the same transaction as `kg_extractions`), so "0 chunks" reliably implies "no prior ingestion." If a future edit ever writes `kg_extractions` before `kg_chunks` (e.g., to reserve an extraction slot), the drift WARN becomes stale. Add a `#[doc] ///` comment on `count_chunks_for_docs_root_hash` documenting this dependency so a future editor sees it.
- Disabled agents get an `info!` log (not `warn!`) since disabling is operator intent, not a misconfiguration.

**Patterns to follow:**
- Existing KG-loop disabled-branch shape at `server/mod.rs:913-926` (matches the `info!` semantics we want).
- Existing error-handling: `match LexicalIngestor::new(...).ingest_all().await { Ok => info!, Err => warn!("lexical ingestion failed; chunks may be stale") }` — keep this wrapping; #778 just adds the `continue` short-circuit for `Disabled`.

**Test scenarios:**
- Happy path: enabled agent with valid docs_root → `LexicalIngestor::new` called with precomputed hash; ingestion runs.
- Happy path: disabled agent → `LexicalIngestor::new` NOT called; `info!` log emitted; no `subject_extraction_start` spawn; no `resolve_pending` spawn.
- Drift WARN: enabled agent with docs_root_hash that has 0 rows in `kg_chunks` → warn log emitted pre-ingestion.
- No drift WARN: enabled agent with docs_root_hash that has existing rows → no warn; ingestion proceeds normally.
- Integration: 2 agents, one enabled one disabled. Assert: enabled agent has KG state post-startup; disabled agent has none of its writes in `kg_chunks` for its (potentially fake) `docs_root_hash`.
- Integration: 2 agents both enabled, pointing at SAME `docs_root` → both compute same `docs_root_hash` → v27 first-writer-wins semantics (from #786) apply; second agent's `INSERT OR IGNORE` on `kg_extractions` is a no-op.

**Verification:**
- `cargo test -p mika-agent` passes all KG-loop related tests.
- `rg "KgAgentConfig::Disabled" crates/mika-agent/src/server/` finds the three expected skip branches (lexical, extraction, resolution).
- `rg "LexicalIngestor::new\|SubjectExtractor::new\|SubjectEntityResolver::new" crates/mika-agent/src/` confirms each is called with `docs_root_hash` arg where applicable.

### Unit 5: Documentation — `DEFAULT_IDENTITY`, well-known writers, CLAUDE.md, configuration.md

- [ ] **Unit 5**

**Goal:** Every operator-facing surface describes the `[kg]` section, behavior matrix, and hard-error policy. Keep the canonical `identity.toml` template + the well-known-agent writer in sync so new agents get a commented `[kg]` stanza showing the available fields.

**Requirements:** R11.

**Dependencies:** Units 1-4 (docs describe the implemented behavior).

**Files:**
- Modify: `crates/mika-common/src/home.rs` — `DEFAULT_IDENTITY` constant (line ~264-266). Add a commented `[kg]` stanza:
  ```
  name = "..."
  emoji = "..."

  # [kg]
  # enabled = true                    # default: true
  # docs_root = "/path/to/docs"       # optional; falls back to MIKA_KG_DOCS_ROOT / settings.kg_docs_root / CWD/docs/solutions
  ```
- Modify: `crates/mika-agent/src/well_known_agents.rs:143-148` — extend the `format!` to include the same commented `[kg]` block. (Commented — well-known agents don't override docs_root by default.)
- Modify: `crates/mika-agent/CLAUDE.md` — KG section. Add a `### [kg] section in identity.toml` subsection documenting: enabled field, docs_root field, behavior matrix table, hard-error policy for explicit paths, warn-and-skip for CWD-default, and the "disabled agent rows persist until #779's CLI" caveat.
- Modify: `mika/CLAUDE.md` (repo root) — KG bullet in Architecture Summary and/or Conventions. One-sentence note: "Per-agent KG scoping lives in `identity.toml` `[kg]` section; see `crates/mika-agent/CLAUDE.md` KG section for details."
- Modify: `docs/configuration.md` — if it has an `identity.toml` schema reference, add the `[kg]` section there. If not, skip.

**Approach:**
- Keep docs factual and short. Defer philosophical framing ("cross-corpus contamination is anti-correctness masquerading as correctness") to the compound doc that will follow from `/ce:compound` post-merge.
- The `DEFAULT_IDENTITY` and `well_known_agents.rs` edits ship commented fields — they act as in-file documentation showing what's available without imposing a default path.

**Patterns to follow:**
- Existing KG section prose style in `crates/mika-agent/CLAUDE.md`.
- Commented-defaults style: look for precedent in the `.env.example` KG block (from #738's Unit 4).

**Test expectation:** none — pure documentation. Operator-facing spot-check at PR review.

**Verification:**
- `rg "\[kg\]" crates/mika-common/src/home.rs crates/mika-agent/src/well_known_agents.rs` shows the commented stanza in both.
- `rg "identity.toml.*kg\|kg.*identity.toml" crates/mika-agent/CLAUDE.md mika/CLAUDE.md` finds the new references.
- Manual read: an operator onboarding a new agent can follow docs from root `CLAUDE.md` → `mika-agent/CLAUDE.md` → `identity.toml` schema to understand how to configure per-agent docs_root.

## System-Wide Impact

- **Interaction graph:** `init_agent` now invokes `resolve_per_agent_docs_root`; error from that bubbles through to the init-loop's skip path. KG construction loops consult `AgentState.kg_config` to gate subsystem init. No change to agent loop, tools, memory, skills — those subsystems don't care about KG state.
- **Error propagation:** `KgConfigError` flows through `anyhow::Context` during `init_agent`, caught per-agent at `server/mod.rs:672-677`. No new global-halt error paths.
- **State lifecycle risks:**
  - Disabled agent's pre-existing rows: intentionally preserved (not owned by the agent per the shared-corpus model). Cleanup via #779.
  - Resolver runs once per agent per process startup. Not hot path.
  - Drift WARN query runs once per enabled agent per startup (`count_chunks_for_docs_root_hash`). Simple indexed lookup; negligible cost.
- **API surface parity:**
  - `LexicalIngestor::new`, `SubjectExtractor::new` gain a `docs_root_hash: Option<String>` parameter (coordinated with #786's Unit 3 — `None` preserves #786's compute-internally contract; `Some(hash)` is the #778 call sites' new path).
  - `SubjectEntityResolver::new` may or may not gain `docs_root_hash`; deferred to implementation based on whether resolver reads from shared tables.
- **Integration coverage:** Unit 3 and Unit 4 integration tests cover the full agent-startup gating flow. The multi-agent drift-and-isolation scenario (agents with different docs_roots → different hashes → isolated shared tables) is covered in Unit 4.
- **Unchanged invariants:**
  - `load_identity` stays infallible. Existing agents with malformed top-level TOML still get `Identity::default()`.
  - Existing `identity.toml` files without `[kg]` work exactly as today.
  - Per-agent init-loop isolation machinery at `server/mod.rs:672-677` is reused; no new skip paths.
  - Domain graph (`kg_entities`, `kg_relationships`) unchanged — it's projected from registries, not per-agent-scoped.
  - v27 schema (#786) unchanged — `docs_root_hash` is the pre-existing shared-corpus key; #778 populates it from per-agent config.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| **Post-#787 deploy, odds-engine agents see empty KGs** until they point at a real corpus and ingest it. | Expected behavior. The plan is to set `docs_root` for odds-engine agents to a real trading-domain corpus (or to disable KG via `enabled=false` during transition). Vincent's operator discipline. Drift WARN at startup surfaces the "no matching rows" state visibly in logs. |
| **Silent typo in `docs_root`** (e.g., trailing slash, wrong case) produces a valid-but-wrong hash, leading to two agents that "should share" ending up isolated. | Hard-error on explicit-path-missing catches obvious typos. Subtler typos (e.g., trailing slash) get caught by `fs::canonicalize` inside #786's `hash_docs_root` — canonicalized paths resolve equivalently for trailing-slash and non-trailing-slash. Non-existent trailing-slash paths hard-error. The one residual risk: two paths that canonicalize to different-but-semantically-equivalent strings. Unlikely in practice (filesystem paths are fairly unambiguous post-canonicalize). |
| **Init-order race between #786/#787 (schema + coalesce) and #778** — agent startup tries to read from v27 tables before migration completes. | #786's startup guard refuses `Database::open` until coalesce is complete. Agents don't start — and therefore don't call `resolve_per_agent_docs_root` — until DB is v27-ready. #778 inherits this safety. |
| **`init_agent` signature change via new `kg_config` field on `AgentState`** ripples to all callers that construct or read `AgentState`. | `AgentState` has a defined constructor path (inside `init_agent`) and the struct is accessed field-wise throughout `server/mod.rs`. Adding a field is additive. Implementer greps for all `AgentState { ... }` constructor sites to confirm only `init_agent` uses the literal-struct syntax. |
| **`count_chunks_for_docs_root_hash` helper doesn't exist** on `Database`/`AsyncDatabase` today. | Add it as part of Unit 4. Simple indexed SELECT. Add a test alongside the helper. |
| **Constructor API shift on `LexicalIngestor::new` / `SubjectExtractor::new`** — #786's Unit 3 committed to "constructor computes hash from docs_root internally"; #778 Unit 4 generalizes to "optional precomputed hash". | **Generalize, not replace.** `docs_root_hash: Option<String>` parameter: `None` preserves #786's compute-internally contract (its test scenarios pass unchanged); `Some(hash)` uses the precomputed value. Neither ticket's tests need to change on the other ticket's merge. The hash-computation contract remains single-source-of-truth via `hash_docs_root`. See Key Technical Decisions for full framing. |
| **Drift-WARN becomes stale** if a future edit changes the composed-write order in `lexical_ingestor.rs` so `kg_extractions` is written before `kg_chunks`. The `count_chunks_for_docs_root_hash` proxy returns 0 when ingestion has actually started. | Add a doc comment on the helper method declaring the dependency on "kg_chunks written first or atomically-with kg_extractions." If a future ticket restructures the composed-write order, the doc comment surfaces the invariant and the implementer can choose to switch the drift-WARN proxy to a different table (e.g., `kg_extractions`) or explicitly invalidate the assumption. |
| **#738 API amendment required before #778 dispatches** — `resolve_kg_docs_root` returns `(PathBuf, PathSource)` instead of `PathBuf`. If #738 ships without the amendment, #778's `resolve_per_agent_docs_root` can't correctly classify explicit-vs-default source and falls back to option (a') inspection with its drift-coupling risk. | **Default branch (Vincent amends #738 at peer-review time):** amendment lands in #738's plan (three extra lines in Unit 2) before dispatch; #778's Unit 2 implements option (b) as specified. **Fallback branch (Vincent approves dispatching #738 as-is, without amendment):** #778's Unit 2 switches to option (a') — `resolve_per_agent_docs_root` inspects `std::env::var("MIKA_KG_DOCS_ROOT")` + `settings.kg_docs_root` directly to classify source — AND #738's Unit 2 gets a `// If you add a new path source here, update resolve_per_agent_docs_root's classifier in kg/config.rs` comment inside `resolve_kg_docs_root` as drift backstop. **Vincent's explicit branch choice at peer review determines which path mika-dev implements.** |
| **Documentation of `[kg]` drifts out of sync with code** after future edits. | `DEFAULT_IDENTITY` constant is checked by `rg` at PR review in Unit 5's verification. CLAUDE.md is read by Claude and surfaced when operators/agents ask about config. Compound doc post-merge provides durable reference. |
| **Malformed `[kg]` in `identity.toml` silently falls back to default** because `load_identity` is infallible. | Documented behavior inherited from existing pattern — same as malformed `[reflection]` today. The plan doesn't change this. Future ticket could make `load_identity` fallible if this becomes a real problem. |
| **Operator sets `enabled=false` on an agent with existing shared-layer rows** — expects rows to disappear; they don't. | `info!` log at startup explicitly says "shared-corpus rows remain in DB; use `mika kg purge --agent <name>` to clean up (#779)". Documentation in CLAUDE.md repeats this caveat. |

## Ownership and Capability Check (Autonomous-Loop Gate)

Per Milestone #17 dispatch constraint: every step on the AC path must be executable by mika-dev without Vincent's intervention.

| Step | Executor | Capability verified |
|------|----------|---------------------|
| Unit 1 (Identity.kg field) | mika-dev | `cargo build -p mika-agent && cargo test -p mika-agent prompt::tests::kg_identity_config` |
| Unit 2 (resolver) | mika-dev | `cargo build -p mika-agent && cargo test -p mika-agent kg::config::tests::resolve_per_agent_docs_root` |
| Unit 3 (init_agent integration) | mika-dev | `cargo build --workspace && cargo test -p mika-agent server::tests::init_agent_kg_config` |
| Unit 4 (KG loops gating) | mika-dev | `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings` |
| Unit 5 (documentation) | mika-dev | Grep-based verification; manual spot-check at PR review |
| PR creation | mika-dev | Standard `/mika` pipeline |
| CI pass | mika-dev | Standard CI; no new workflow steps |
| Merge | mika-dev | Auto-merge once CI green |
| **Deploy (post-milestone)** | Vincent, post-milestone | With #786/#787/#778 all merged, full milestone deploy. Odds-engine agents' `identity.toml` will need `[kg].docs_root` set to their real corpus (or `enabled=false`) as part of post-merge operator task — NOT on the AC path. |
| Post-merge operator task: set odds-engine `[kg].docs_root` | Vincent, post-milestone | Edit `~/.mika/agents/odds-engine-{ceo,cto,quant}/identity.toml` to add correct `[kg]` section. NOT on AC path. The AC path is shipping the capability; using the capability is operator follow-up. |

No SQL, no manual deploy, no human-in-the-loop on the AC path. Safe for full-autonomous dispatch.

## Sources & References

- **Origin issue:** [senara-solutions/mika#778](https://github.com/senara-solutions/mika/issues/778)
- **Milestone:** [senara-solutions/mika#17](https://github.com/senara-solutions/mika/milestone/17)
- **DAG position:** Blocked by #786 (schema), #787 (coalesced data). Blocks: #779 (CLI reads this config).
- **Upstream plans:**
  - `docs/plans/2026-04-24-005-feat-kg-docs-root-config-plan.md` (#738) — global fallback resolver + `MIKA_KG_DOCS_ROOT`
  - `docs/plans/2026-04-24-006-feat-kg-schema-v27-docs-root-hash-plan.md` (#786) — v27 schema with `docs_root_hash` keying
  - `docs/plans/2026-04-24-007-feat-kg-data-migration-v27-coalesce-plan.md` (#787) — v26→v27 coalesce
- **Institutional learnings:**
  - `docs/solutions/architecture-patterns/well-known-agent-config-toml-override.md` — per-agent TOML extension precedent
  - `docs/solutions/architecture-patterns/simplified-config-4-source-model.md` — serde-default-only rule
  - `docs/solutions/architecture-patterns/per-agent-dotenv-config-injection.md` — multi-agent process-model safety
  - `docs/solutions/integration-issues/always-on-skill-oversized-prompt-loud-failure.md` — typed-return load pattern
  - `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md` — eviction-at-construction precedent
  - `docs/solutions/best-practices/kg-domain-graph-startup-projection-2026-04-22.md` — KG subsystem fail-open norm (being broken here for config errors)
  - `docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md` — per-agent × per-docs_root fan-out cost framing (#778 is the correctness counterpart)
  - `docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md` — fail-loud on dependency-source emptiness
- **Anchor files:**
  - `crates/mika-agent/src/prompt.rs:46-93` — `Identity` struct + `load_identity`
  - `crates/mika-agent/src/server/mod.rs:338-551` — `init_agent` function
  - `crates/mika-agent/src/server/mod.rs:552-702` — `run_server` (settings in scope; init-loop; KG loops)
  - `crates/mika-agent/src/server/mod.rs:672-677` — init-loop skip pattern (reused for #778's hard-error)
  - `crates/mika-agent/src/server/mod.rs:785-810` — `LexicalIngestor` construction loop
  - `crates/mika-agent/src/server/mod.rs:842-911` — `SubjectExtractor` construction loop
  - `crates/mika-agent/src/server/mod.rs:961-1029` — `SubjectEntityResolver` construction loop
  - `crates/mika-agent/src/kg/config.rs` — created by #738, extended by #786, further extended here
  - `crates/mika-agent/src/kg/lexical_ingestor.rs`, `subject_extractor.rs`, `entity_resolver.rs` — constructor signatures
  - `crates/mika-common/src/home.rs:264-266` — `DEFAULT_IDENTITY` constant
  - `crates/mika-agent/src/well_known_agents.rs:143-148` — identity.toml writer
- **Compound-doc candidate (post-merge):** cross-corpus contamination as quality-primary correctness bug. No prior compound doc; #778 is the first in this pattern family. Plan is to spawn `/ce:compound` post-merge to codify.
