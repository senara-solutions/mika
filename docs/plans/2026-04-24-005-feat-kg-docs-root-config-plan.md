---
title: "feat(kg): MIKA_KG_DOCS_ROOT env var + kg_docs_root config field"
type: feat
status: active
date: 2026-04-24
issue: senara-solutions/mika#738
branch: feat/738/lexical-ingestor-cwd-dependent-docs-path-add
milestone: senara-solutions/mika#17
---

# feat(kg): MIKA_KG_DOCS_ROOT env var + kg_docs_root config field

## Overview

`LexicalIngestor` today resolves its docs root via `std::env::current_dir().join("docs/solutions")`. This works inside the container (the Dockerfile COPYs `docs/` into the workdir) but breaks on OpenRC hosts where `supervise-daemon` launches `mika-server` with CWD=`/` — the ingestor logs `docs/solutions not found — skipping lexical ingestion` and the entire lexical / subject / resolution KG pipeline never runs. This plan adds a `MIKA_KG_DOCS_ROOT` env var and `kg_docs_root` config field so operators can point the ingestor at the repo's docs tree regardless of the process CWD, without changing container behavior.

## Problem Frame

Discovered 2026-04-22 while verifying KG milestone #14 deploy on a Gentoo OpenRC host: `/proc/1736/cwd` → `/`, the `docs/solutions not found` log line repeats across restarts, and only the in-memory domain graph survives — the lexical, subject, and resolution layers are empty. The CLI/TUI path doesn't construct `LexicalIngestor` at all, so this is a server-only failure mode and a hard block on populating KG on any non-container host. Milestone #16 (Evaluation) needs a populated KG to run meaningful self-knowledge evals.

Global config is the right level (docs tree is repo-scoped, not agent-scoped), and per-agent docs_root is #778's concern — this ticket lands the env/config surface that #778's fallback chain reads.

## Requirements Trace

- **R1.** New config surface — `MIKA_KG_DOCS_ROOT` env var and `kg_docs_root` field in `Settings`.
- **R2.** Resolution precedence: env > config file > existing CWD-based default.
- **R3.** On-disk fallback absence preserves the current warn-and-skip behavior (non-fatal) — the server still comes up; KG layer stays empty.
- **R4.** Documentation: `.env.example`, `crates/mika-agent/CLAUDE.md`, `docs/configuration.md`.
- **R5.** Unit test: resolution chain across env-set / env-unset / config-set / both-set cases.
- **R6.** Integration test: server startup from CWD != repo-root produces `lexical_ingest_complete` when `MIKA_KG_DOCS_ROOT` points at repo docs, and the existing `docs/solutions not found` warn when unset.
- **R7.** Fallback resolver is a `pub fn` that #778's per-agent resolver can call — don't bury the logic inside `LexicalIngestor::new` or inline it in `server/mod.rs`.

## Scope Boundaries

- **Non-goal:** schema changes. Schema v27 is #786; data migration is #787.
- **Non-goal:** per-agent `docs_root` config. That is #778; this plan lands the global fallback surface #778 reads.
- **Non-goal:** hard-fail on missing path. #778 introduces hard-error startup for per-agent misconfiguration; #738 keeps the existing warn-and-skip global behavior so that agents without any override still come up gracefully on a CWD that happens to lack a `docs/solutions` subdirectory.
- **Non-goal:** removing the OpenRC workaround hint from existing docs. Hosts running with `--chdir` must continue to work.
- **Non-goal:** mika-cloud Helm changes. Helm already passes env vars through `values.yaml`; operators can set `MIKA_KG_DOCS_ROOT` there without a code change in mika-cloud.

## Context & Research

### Relevant code and patterns

- **`Settings` and config cascade:** `crates/mika-common/src/config.rs` — struct at line 534, loader `Settings::load_for_agent` at 1108–1161 uses `config-rs` with a documented 6-step cascade: defaults → global config.toml → agent config.toml → agent `.env` → `~/.mika/.env` → `MIKA_*` env vars (highest). Env vars are wired via `Environment::with_prefix("MIKA").prefix_separator("_").separator("__")` at line 1156.
- **Existing KG config field to mirror** (`config.rs:767-775`):
  ```
  /// KG per-batch LLM call budget — ...
  #[serde(default)]
  pub kg_batch_budget: Option<u32>,
  ```
  Pattern: `Option<T>` with `#[serde(default)]`, documented with a doc-comment that explains semantics and the disable-value convention.
- **`CONFIG_KEYS` registry** (`config.rs:38`): every new config key must have a `ConfigKeyInfo` entry (`key`, `env_var: Some("MIKA_KG_DOCS_ROOT")`, `secret: false`, `description`). Consumed by `mika config get/set/list`, `mika doctor`, and the dashboard.
- **`get_effective_value()` match arm:** also in `config.rs` — has a coverage test that fails CI if a File-backend key is missing its arm.
- **Serial env-var test template** (`config.rs:1531-1543`): `test_disable_bundled_skills_from_env` uses `#[test] #[serial]` + `clean_env()` helper + `unsafe { std::env::set_var / remove_var }`. The `clean_env()` helper (lines 1393–1405) must be updated to remove `MIKA_KG_DOCS_ROOT` too.
- **`LexicalIngestor` construction sites:** two — `crates/mika-agent/src/server/mod.rs:787` (startup ingest loop, the CWD-resolution site) and `crates/mika-agent/src/kg/ingestion_orchestrator.rs` (uses already-resolved `docs_root` from the caller). Only `server/mod.rs` is a construction site that computes the path; `ingestion_orchestrator.rs` just reads `self.docs_root`.
- **Current CWD resolution** (`server/mod.rs:762-768`):
  ```
  let docs_root = std::env::current_dir()
      .unwrap_or_default()
      .join("docs")
      .join("solutions");
  ```
- **Future #778 consumer:** agent identity currently at `crates/mika-agent/src/prompt.rs:46-93` (`Identity` struct, `load_identity()`). #778 will add a `[kg]` section or a sibling loader and call the resolver added here as the unset-fallback path.

### Institutional learnings

- **`docs/solutions/architecture-patterns/simplified-config-4-source-model.md`** — canonical cascade model. `MIKA_KG_DOCS_ROOT` maps natively to `kg_docs_root` (single underscores are literal); no `#[serde(rename)]` needed. Shell env always wins over `.env`.
- **`docs/solutions/architecture-patterns/config-key-rename-across-layers.md`** — full checklist for touching a single config key across all layers. Even though this is an add (not rename), every bullet applies: registry entry, Settings field, `get_effective_value()` arm, `.env.example`, CLAUDE.md, `docs/configuration.md`, test-fixture coverage. Missing a layer creates silent-failure surfaces.
- **`docs/solutions/architecture-patterns/config-key-registry-cli-management.md`** — `ConfigBackend::File` + `env_var: Some(...)` makes `mika config set/get kg_docs_root <path>` work, and env-var override resolves in `resolve_source()` before backend checks. This is why **don't hand-roll a 3-step resolution chain** — the registry handles env > config precedence automatically; the plan only needs to add CWD-based default as the `unwrap_or_else` branch.
- **`docs/solutions/677-configurable-task-session-limit.md`** — mirrored specifically for the consumer-side fallback shape: `settings.field.clone().unwrap_or_else(|| default_expr)`. This plan applies that exact shape inside `resolve_kg_docs_root`. Not mirrored from #677: its direct `std::env::var` read (would bypass the config-rs cascade that this plan depends on) and its integer-typed field (the `PathBuf` type here is load-bearing for early validation at deserialization).
- **`docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`** — the retrospective cited in the ticket body. Broader lesson: "prose/implicit state drifts, LLMs rationalize around it" — a process whose CWD depends on who launched it is exactly the kind of implicit state that breaks the KG pipeline silently. The config field makes the path explicit.
- **`docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md`** — confirms resolution happens outside the composed-write transaction. Fallback chain + warn-on-missing add no transactional risk.

## Key Technical Decisions

- **Use the 4-source config cascade; don't hand-roll a 3-step resolver.** Config-rs's `Environment::with_prefix("MIKA")` source sits above the `File` source, so env-set > config-set is automatic once `kg_docs_root` is added to `Settings`. The only explicit fallback the plan needs is the CWD-based default inside `resolve_kg_docs_root()`, implemented as `settings.kg_docs_root.clone().unwrap_or_else(...)`. Rationale: mirrors `docs/solutions/architecture-patterns/simplified-config-4-source-model.md` and avoids reimplementing precedence logic that config-rs already provides and tests.

- **Resolver lives in `crates/mika-agent/src/kg/config.rs` (new module).** Alternative was `impl Settings` in `mika-common` next to `active_llm_config()`, but `mika-common` has no other KG-specific concepts, and the research agent flagged that co-locating KG logic in `mika-agent/src/kg/` matches existing layering. Re-export via `pub mod config;` in `crates/mika-agent/src/kg/mod.rs`.

- **Resolver is a `pub fn`, not a method on `Settings`.** #778 will need to call it from `prompt.rs` or a sibling module. A free function with an explicit `&Settings` parameter makes the call site obvious and avoids making `Settings` depend on `PathBuf` semantics it doesn't otherwise carry.

- **No validation inside the resolver.** Path existence is checked at the consumer site (`server/mod.rs` continues to emit the existing `docs/solutions not found` warn). Validating inside the resolver would force #738 to bake a missing-path policy that #778 later has to override (hard-error vs warn-and-skip). Keep the resolver pure.

- **Integration test uses `tempdir()` + `std::env::set_current_dir()` rather than a new CI step.** `.github/workflows/ci.yml` runs `cargo test --workspace` from repo root; a unit test that sets its own CWD is cheaper, more portable, and runs on every CI invocation without workflow surgery. Marked `#[serial]` like the other env-touching tests.

- **`kg_docs_root: Option<PathBuf>`.** `PathBuf` over `String` — the consumer needs a path anyway, and early conversion catches bad inputs at deserialization. `Option` over `PathBuf` with a default — `None` is the honest representation of "use the CWD-based default"; `Default::default()` would be `""` which is meaningful-but-wrong.

## Open Questions

### Resolved during planning

- **Q: Use config-rs auto-cascade or hand-rolled 3-level chain?** → Use auto-cascade. Config-rs `Environment` source sits above `File` source natively.
- **Q: Resolver location — `mika-common` or `mika-agent/kg/config.rs`?** → `mika-agent/kg/config.rs`. Keeps KG concepts co-located; `mika-common` doesn't depend on KG.
- **Q: Missing-path handling — hard-fail like #778, or warn-and-skip?** → Warn-and-skip. #738 is the global surface; #778's hard-fail is a per-agent contract concern.
- **Q: CWD-diff test — new CI step or in-test `set_current_dir()`?** → In-test with `tempdir()` and `#[serial]`. Cheaper and portable.
- **Q: mika-cloud Helm update required?** → No. Helm passes env vars through `values.yaml`; operators can set `MIKA_KG_DOCS_ROOT` without a code change downstream.

### Deferred to implementation

- Exact `ConfigKeyInfo` description wording — write it at implementation time by mirroring neighbor entries.
- Exact placement of the `MIKA_KG_DOCS_ROOT` line in the `.env.example` KG block — insert adjacent to the other `MIKA_KG_*` vars at lines 41–52.

## Implementation Units

### Unit 1: Add `kg_docs_root` to `Settings` and `CONFIG_KEYS` registry

- [ ] **Unit 1**

**Goal:** Land the field, registry entry, and `get_effective_value()` match arm so the config-rs cascade wires env → config → `settings.kg_docs_root` automatically.

**Requirements:** R1, R2.

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-common/src/config.rs`
  - Add `pub kg_docs_root: Option<PathBuf>` with `#[serde(default)]` to the `Settings` struct (near `kg_batch_budget` at line 767).
  - Add `ConfigKeyInfo` entry to the `CONFIG_KEYS` static registry (line 38 area): `key: "kg_docs_root"`, `env_var: Some("MIKA_KG_DOCS_ROOT")`, `secret: false`, `description: ...`, `backend: ConfigBackend::File`.
  - Add the `get_effective_value()` match arm for `"kg_docs_root"` returning the field value.
  - Add `MIKA_KG_DOCS_ROOT` to the `clean_env()` test helper (lines 1393–1405).

**Approach:**
- Mirror `kg_batch_budget` field style.
- Mirror `test_disable_bundled_skills_from_env` (lines 1531–1543) for the env-var roundtrip test.
- Use `PathBuf` (not `String`) — forces deserializer to validate path-shape early.

**Patterns to follow:**
- `crates/mika-common/src/config.rs:767-775` — `kg_batch_budget` field.
- `crates/mika-common/src/config.rs:1531-1543` — serial env-var test template.
- `docs/solutions/architecture-patterns/config-key-rename-across-layers.md` — full-layer checklist.

**Test scenarios:**
- Happy path: `Settings::from_str("")` → `kg_docs_root == None` (empty-TOML roundtrip).
- Happy path: `MIKA_KG_DOCS_ROOT=/abs/path` env-set → `Settings::load()` → `kg_docs_root == Some(PathBuf::from("/abs/path"))`.
- Happy path: `config.toml` with `kg_docs_root = "/abs/path"` only → `kg_docs_root == Some(PathBuf::from("/abs/path"))`.
- Integration: env-set AND config-set with different paths → env value wins (verifies cascade).
- Edge: `kg_docs_root = ""` in config.toml → deserializes to `Some(PathBuf::from(""))`; the resolver's consumer (Unit 3) is responsible for the existence check, not the deserializer.
- Integration: `get_effective_value("kg_docs_root")` returns the expected source (File / Env) and value.

**Verification:**
- `cargo test -p mika-common config::tests` passes.
- `get_effective_value()` coverage test passes (CI fails otherwise if the match arm is missing).

### Unit 2: Create `kg::config::resolve_kg_docs_root`

- [ ] **Unit 2**

**Goal:** A pure `pub fn` that returns the resolved docs root without doing I/O beyond what's already in `std::env::current_dir()`. Callable from `server/mod.rs` (Unit 3) and later from #778's per-agent resolver.

**Requirements:** R2, R7.

**Dependencies:** Unit 1.

**Files:**
- Create: `crates/mika-agent/src/kg/config.rs`
- Modify: `crates/mika-agent/src/kg/mod.rs` (add `pub mod config;`)
- Test: `crates/mika-agent/src/kg/config.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Signature: `pub fn resolve_kg_docs_root(settings: &Settings) -> (PathBuf, PathSource)`
- `PathSource` enum, defined adjacent to the function:
  ```
  pub enum PathSource {
      EnvVar,       // path came from MIKA_KG_DOCS_ROOT env var
      ConfigFile,   // path came from settings.kg_docs_root (config.toml)
      CwdDefault,   // path fell through to std::env::current_dir().join("docs/solutions")
  }
  ```
- Body:
  ```
  // Env var check first (config-rs resolves MIKA_KG_DOCS_ROOT into settings.kg_docs_root,
  // but we need to know whether the value came from env vs config file, so re-inspect env).
  if let Ok(env_path) = std::env::var("MIKA_KG_DOCS_ROOT") {
      return (PathBuf::from(env_path), PathSource::EnvVar);
  }
  if let Some(config_path) = settings.kg_docs_root.clone() {
      return (config_path, PathSource::ConfigFile);
  }
  let cwd_default = std::env::current_dir()
      .unwrap_or_default()
      .join("docs")
      .join("solutions");
  (cwd_default, PathSource::CwdDefault)
  ```
- No existence check, no canonicalization — callers remain responsible for I/O and for distinguishing empty-path vs nonexistent-path (see Unit 3).
- Doc comment must explain three things:
  1. Env > config precedence: the resolver re-inspects `MIKA_KG_DOCS_ROOT` directly (not just `settings.kg_docs_root`) to distinguish the env-var case from the config-file case. Config-rs already merges env into `settings.kg_docs_root`, so `Some` could be either source — the re-inspection gives the true source.
  2. **Public contract consumed by #778's per-agent resolver** — signature changes require coordinated update across both tickets. Compile-time binding in `tests` module catches the mechanical drift; this comment addresses the human side.
  3. **Source-of-origin exposed for #778's per-agent policy classifier.** #778 uses `PathSource::{EnvVar, ConfigFile}` to distinguish "operator explicitly set this path; hard-error if it doesn't exist" from `PathSource::CwdDefault` → "fell through to container-friendly default; warn-and-skip if it doesn't exist". **If you add a new source here (e.g., workspace-level config), update `resolve_per_agent_docs_root` in kg/config.rs to classify it correctly.** The exhaustive match on `PathSource` in `resolve_per_agent_docs_root` will force a compile error if variants are added, but this breadcrumb names the why.

**Technical design** (directional, not implementation spec):
```
Settings (from config-rs cascade) + process env
    │
    ├─ MIKA_KG_DOCS_ROOT env var set  ──►  (PathBuf::from(env_val), EnvVar)
    │
    ├─ settings.kg_docs_root = Some(path)  ──►  (path, ConfigFile)
    │
    └─ (neither)                           ──►  (CWD-join("docs/solutions"), CwdDefault)
```

**Patterns to follow:**
- `crates/mika-agent/src/kg/mod.rs` re-export style for existing sibling modules.

**Test scenarios:**
- Happy path (env var wins): env `MIKA_KG_DOCS_ROOT=/e`, `settings.kg_docs_root = Some(PathBuf::from("/c"))` → returns `(PathBuf::from("/e"), PathSource::EnvVar)`. Env takes precedence over config file even when both are set.
- Happy path (config file): env unset, `settings.kg_docs_root = Some(PathBuf::from("/x"))` → returns `(PathBuf::from("/x"), PathSource::ConfigFile)`.
- Happy path (CWD fallback): env unset, `settings.kg_docs_root = None`, CWD = `/tmp/fake-repo` → returns `(/tmp/fake-repo/docs/solutions, PathSource::CwdDefault)`.
- Edge: env set to empty string → `(PathBuf::from(""), PathSource::EnvVar)`. Empty-path classification still applies (Unit 3's distinct warn handles the empty-path case). Source is EnvVar because the operator did explicitly set it to an empty value — policy still "hard-error downstream" if the consumer cares about existence.
- Edge: `settings.kg_docs_root = Some(PathBuf::from(""))` (config file set to empty) → `(PathBuf::from(""), PathSource::ConfigFile)`. Same semantics as above — empty but explicit.
- Contract (signature binding): `let _: fn(&Settings) -> (PathBuf, PathSource) = resolve_kg_docs_root;` inside a `#[test]` module. Compiles-or-fails; no runtime assertion. Prevents silent drift from the public contract #778 depends on.
- Contract (PathSource exhaustiveness): a `#[test]` that matches on all three `PathSource` variants — `match source { PathSource::EnvVar => (), PathSource::ConfigFile => (), PathSource::CwdDefault => () }`. Adding a future variant without updating this test (and by extension, `resolve_per_agent_docs_root` in #778) produces a compile error. Belt-and-suspenders for the breadcrumb in the doc comment.

**Not tested** (preserved by construction, not by induction): the `unwrap_or_default()` branch on `std::env::current_dir()` failure. Today's code uses the same expression; peer review flagged that inducing CWD failure via `tempdir()` + `remove_dir` is platform-dependent (Linux returns the stale path, macOS behavior varies) and flaky on CI. Diff-reading the `unwrap_or_default()` expression against the pre-plan implementation is the verification; test induction would be brittle theatre.

**Verification:**
- `cargo test -p mika-agent kg::config::tests` passes.
- Function is reachable from `crates/mika-agent/src/prompt.rs` (verify by grep after merge; #778 will close the loop).

### Unit 3: Wire resolver into `LexicalIngestor` construction

- [ ] **Unit 3**

**Goal:** Replace the inline CWD resolution at `server/mod.rs:762-768` with a call to `kg::config::resolve_kg_docs_root`. Preserve the existing warn-and-skip when the resolved path does not exist.

**Requirements:** R2, R3, R6.

**Dependencies:** Unit 2.

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs` (replace the current CWD join with a resolver call; add the empty-path warn branch).
- Test: Create `crates/mika-agent/tests/kg_docs_root_resolution.rs` (new file). One test file, one owner, unambiguous. A future ticket may consolidate startup-flow integration tests — that is a consolidation ticket's concern, not this one.

**Approach:**
- Import: `use crate::kg::config::{resolve_kg_docs_root, PathSource};`
- Call:
  ```
  let (docs_root, _source) = resolve_kg_docs_root(&settings);
  // #738 itself doesn't care about `_source` — the warn-and-skip downstream behavior is
  // the same for all three sources. #778 is the consumer that branches on PathSource.
  ```
- Before the existing existence check, distinguish the empty-path case so operators don't chase a misleading `docs/solutions not found` log when the real problem is an empty config value:
  ```
  if docs_root.as_os_str().is_empty() {
      tracing::warn!("kg_docs_root is set to empty string — check MIKA_KG_DOCS_ROOT / config.toml");
      // skip lexical ingest
  } else if !docs_root.exists() {
      // existing warn-and-skip preserved verbatim
  }
  ```
- Keep all other surrounding startup flow exactly as today — only the `let docs_root = ...` expression and the new empty-path branch change.

**Execution note:** Test-first is recommended here. Write the CWD-from-`/tmp` integration test before the server wiring change so the failing-pre / passing-post transition is explicit.

**Patterns to follow:**
- `crates/mika-agent/src/server/mod.rs:762-786` — preserve surrounding startup flow and warn-and-skip exactly; only the `let docs_root = ...` expression changes.
- `serial_test::serial` + `tempdir()` for the CWD-change test (see Unit 1's env-var test pattern).

**Test scenarios:**
- Happy path (current behavior preserved): No env, no config, CWD = repo root → resolver returns `<repo>/docs/solutions`, lexical ingest runs, `lexical_ingest_complete` emitted.
- Happy path (the bug fix): `MIKA_KG_DOCS_ROOT=<repo>/docs/solutions`, CWD = `/tmp/<tempdir>` → resolver returns the repo docs path, lexical ingest runs, `lexical_ingest_complete` emitted.
- Error path (existing behavior preserved): No env, no config, CWD = `/tmp/<tempdir>` → resolver returns `/tmp/<tempdir>/docs/solutions`, the existence check fails, the existing `docs/solutions not found — skipping lexical ingestion` warn fires, server startup completes.
- Error path (new distinct log): `MIKA_KG_DOCS_ROOT=""` (empty string) — or the config field is set to an empty string — → resolver returns an empty `PathBuf`, the new `kg_docs_root is set to empty string — check MIKA_KG_DOCS_ROOT / config.toml` warn fires (not the generic not-found warn), server startup completes. This is the operator-ergonomics scenario Vincent flagged during peer review.
- Error path: `MIKA_KG_DOCS_ROOT=/nonexistent/path`, any CWD → generic warn-and-skip, no panic, no empty-string warn.
- Integration: `ingestion_orchestrator.rs` reingest path (`reingest_and_reextract` at line 191) continues to use `self.docs_root` — verify by grep that no additional changes are needed in that file.

**Verification:**
- `cargo test -p mika-agent --test kg_docs_root_resolution` passes all scenarios above.
- Manual verification (mika-dev-executable, post-merge, not in AC): `cargo build --release && cd /tmp && MIKA_KG_DOCS_ROOT=$REPO/docs/solutions ./target/release/mika-server --agent test` logs `lexical_ingest_complete`.

### Unit 4: Documentation — `.env.example`, CLAUDE.md, configuration.md

- [ ] **Unit 4**

**Goal:** Every operator-facing surface that lists config keys mentions `MIKA_KG_DOCS_ROOT` / `kg_docs_root` with the same semantics.

**Requirements:** R4.

**Dependencies:** Unit 1 (key shape must be final before docs ship).

**Files:**
- Modify: `.env.example` — insert `MIKA_KG_DOCS_ROOT=` in the KG block (currently lines 41–52, adjacent to `MIKA_KG_INGESTION_MODEL`, `MIKA_KG_EXTRACTION_MODEL`, etc.). Include an inline comment referencing the OpenRC workaround note.
- Modify: `crates/mika-agent/CLAUDE.md` — add a short note under `## Knowledge Graph — Subject Extractor` (line 201 area) covering the resolution chain (env > config > CWD) and a one-line "for OpenRC hosts, either set `MIKA_KG_DOCS_ROOT` or use the `--chdir` init-script workaround" pointer. Do NOT remove any existing workaround documentation elsewhere.
- Modify: `docs/configuration.md` — add `kg_docs_root` row to the config-key table near lines 351–354 (neighbors: `kg_ingestion_model`, `kg_extraction_model`, `kg_resolution_model`, `kg_batch_budget`) and add `MIKA_KG_DOCS_ROOT` row to the env-var table near lines 535–538.
- Modify: `CLAUDE.md` (mika repo root) — add `MIKA_KG_DOCS_ROOT` to the `Optional (Knowledge Graph LLM):` block at line 115, immediately after the existing four `MIKA_KG_*` entries at lines 116–119. Description: "Optional override for the docs root `LexicalIngestor` reads. Defaults to `<CWD>/docs/solutions` when unset. Needed on hosts where the service starts with CWD ≠ repo root (e.g., OpenRC `supervise-daemon`). If the path is unset OR empty, the lexical ingest phase skips with a distinct warn — operator-facing so misconfiguration is obvious in logs." (Grooming-time check confirmed the block exists at the cited line.)

**Approach:**
- Mirror the description style of the existing `kg_batch_budget` / `MIKA_KG_BATCH_BUDGET` entries.
- Call out the default: "Defaults to `<CWD>/docs/solutions` when unset (container-native)."
- Call out the OpenRC host case briefly so operators don't need to dig through solution docs.

**Patterns to follow:**
- `.env.example` existing KG block structure (one comment + one env-var line per key).
- `docs/configuration.md` table formatting at lines 351–354 and 535–538.

**Test expectation:** none — pure documentation. CI's markdown-lint and link-check (if present) will flag formatting regressions.

**Verification:**
- Grep `MIKA_KG_DOCS_ROOT` across the repo; every expected doc surface has an entry.
- No grep hit for the env var name that looks like a leftover scaffold (e.g., `TODO`, `FIXME`, placeholder `/path/to/...` without context).

## System-Wide Impact

- **Interaction graph:** New resolver is on the startup path only (`server::agent_task` calls `LexicalIngestor::new`). No effect on the hot path; no effect on CLI/TUI (which doesn't construct `LexicalIngestor`).
- **Error propagation:** Unchanged. Missing path → existing `tracing::warn!` + skip; non-fatal startup.
- **State lifecycle risks:** None. Resolver is pure and side-effect-free.
- **API surface parity:** The new `pub fn resolve_kg_docs_root` is the surface #778 will call. Signature must be stable across this and the next ticket — don't rename or change the parameter type when #778 lands.
- **Integration coverage:** Unit 3's CWD-diff integration test exercises the server startup → resolver → LexicalIngestor path end-to-end. Unit-test-only coverage would miss the `docs_root.exists()` check behavior.
- **Unchanged invariants:**
  - `LexicalIngestor::ingest_single_doc_inner()` composed-write semantics unchanged (resolution happens upstream).
  - `ingestion_orchestrator.rs` `reingest_and_reextract` continues to use `self.docs_root` — no cascade back into that file.
  - CLI/TUI behavior unchanged (still no `LexicalIngestor` call site there).
  - Dockerfile layout unchanged (existing container behavior is the `None` → CWD-based default, which still works because the Dockerfile puts the workdir where the docs live).
  - Existing OpenRC `--chdir` workaround continues to work — it just becomes one of two valid operator paths instead of the only one.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Forgetting a layer in the `CONFIG_KEYS` / `get_effective_value` / `.env.example` / CLAUDE.md / configuration.md chain produces silent-failure surfaces. | `config-key-rename-across-layers.md` checklist is baked into Units 1 and 4. `get_effective_value()` coverage test fails CI if the match arm is missing. |
| Env var + config-rs single-underscore mapping breaks if the field is ever renamed. | `MIKA_KG_DOCS_ROOT` → `kg_docs_root` is native (no `serde(rename)`) — flagged in Key Technical Decisions. |
| #778's downstream resolver silently diverges from the signature this plan ships. | Unit 2 makes `resolve_kg_docs_root(&Settings) -> PathBuf` the public contract. #778's grooming will carry this signature forward; if the parameter type needs changing, it's a #778-time decision, not a silent drift. |
| Test that sets CWD pollutes other tests running in parallel. | Marked `#[serial]` via `serial_test`. Uses `tempdir()` so the directory disappears at test teardown. |
| OpenRC host operators who already use the `--chdir` workaround miss the new env var and never migrate. | Workaround is explicitly preserved. The env var is additive, not replacing. CLAUDE.md note mentions both paths. |
| Operator sets `MIKA_KG_DOCS_ROOT=""` or leaves `kg_docs_root` as empty string in config → resolver returns empty `PathBuf` → operator chases a misleading `docs/solutions not found` log for 20 minutes before realizing the config value itself is empty. | Unit 3's consumer branches on `docs_root.as_os_str().is_empty()` and emits a distinct warn (`kg_docs_root is set to empty string — check MIKA_KG_DOCS_ROOT / config.toml`) before the generic existence check. Test scenario in Unit 3 covers it. |
| CWD-based default lands an empty path if `current_dir()` fails (rare — deleted CWD). | Matches today's behavior via `unwrap_or_default()` → `PathBuf::new().join("docs/solutions")` → existence check fails → warn-and-skip. No regression. |

## Documentation / Operational Notes

- Operators on OpenRC hosts get a one-line fix: set `MIKA_KG_DOCS_ROOT=/path/to/mika-repo/docs/solutions` in `/etc/conf.d/mika-server` (or the per-agent `.env`), restart the service, observe `lexical_ingest_complete` in logs.
- Existing OpenRC init scripts using `directory=/path/to/mika-repo` or `supervise_daemon_args="--chdir /path/to/mika-repo"` continue to work unchanged.
- Container deploys: no action required. The `None` → CWD-based default still resolves to `<workdir>/docs/solutions` as today.
- mika-cloud (Helm): operators who want to set this in K8s can add `MIKA_KG_DOCS_ROOT` to the chart's env block — this is a values.yaml change, not a chart-code change, so it stays out of the mika-cloud repo unless that repo's `values.yaml` defaults need the line for self-documentation (optional follow-up, not part of this plan).

## Ownership and capability check (autonomous-loop gate)

Per Milestone #17 dispatch constraint: every step in this plan must be executable by mika-dev without Vincent's intervention.

| Step | Executor | Capability verified |
|------|----------|---------------------|
| Unit 1 (Settings + registry) | mika-dev | `cargo build -p mika-common && cargo test -p mika-common config::tests` |
| Unit 2 (resolver module) | mika-dev | `cargo build -p mika-agent && cargo test -p mika-agent kg::config::tests` |
| Unit 3 (wire into server) | mika-dev | `cargo test -p mika-agent --test kg_docs_root_resolution` |
| Unit 4 (docs) | mika-dev | Grep-based verification; no runtime gate |
| PR creation | mika-dev | Standard `/mika` pipeline |
| CI pass | mika-dev | Standard CI; no new workflow steps |
| Merge | mika-dev | Auto-merge once CI green; no Vincent approval step on AC path |
| Post-merge verification (optional) | Vincent, post-milestone | Deploy to OpenRC host, verify `lexical_ingest_complete` log — NOT on AC path, NOT blocking milestone close |

No SQL to run, no manual deploy, no human-in-the-loop on the AC path. Safe for full-autonomous dispatch.

## Sources & References

- **Origin issue:** [senara-solutions/mika#738](https://github.com/senara-solutions/mika/issues/738)
- **Milestone:** [senara-solutions/mika#17 — Knowledge Graph: corpus dedup & per-agent config](https://github.com/senara-solutions/mika/milestone/17)
- **Discovery context:** `docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`
- **Downstream consumer:** #778 (per-agent docs_root config) — depends on this resolver's public signature
- **Related patterns:**
  - `docs/solutions/architecture-patterns/simplified-config-4-source-model.md`
  - `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`
  - `docs/solutions/architecture-patterns/config-key-registry-cli-management.md`
  - `docs/solutions/677-configurable-task-session-limit.md`
  - `docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md`
- **Anchor files:**
  - `crates/mika-common/src/config.rs:38` (`CONFIG_KEYS`), `:534` (`Settings` struct), `:767-775` (pattern field), `:1108-1161` (loader), `:1393-1405` (`clean_env`), `:1531-1543` (env-var test template)
  - `crates/mika-agent/src/server/mod.rs:762-787` (construction site)
  - `crates/mika-agent/src/kg/ingestion_orchestrator.rs:75-86,102,191` (downstream consumer)
  - `crates/mika-agent/src/prompt.rs:46-93` (future #778 caller)
  - `.env.example:41-52` (KG env-var block)
  - `docs/configuration.md:351-354,535-538` (config-key and env-var tables)
  - `crates/mika-agent/CLAUDE.md:201` (`## Knowledge Graph — Subject Extractor`)
  - `.github/workflows/ci.yml:16,38,41,44` (test job)
