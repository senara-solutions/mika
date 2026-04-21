---
title: "feat: Skill variant management tooling (CLI + server API + dashboard)"
type: feat
status: active
date: 2026-04-21
deepened: 2026-04-21
---

# feat: Skill variant management tooling (CLI + server API + dashboard)

## Overview

Add a management layer for per-provider/per-model prompt variants across skills. This introduces a `mika skills variants <op>` CLI subcommand family, matching `/api/v1/skills/variants/*` HTTP endpoints, and a new dashboard "Skills > Variants" page. A new `experimental/` directory tier provides isolated variant authoring that is excluded from runtime resolution, with a `promote` gate to move experimental variants into production `generated/` after validation.

## Problem Frame

The Sprint 2026-04-11 hot-patch chain deleted all 4 existing variants because the minimax self-dev variant silently carried an outdated unconditional `{"action":"allow"}` directive that had been fixed in the base but never propagated. No tooling exists to detect drift, validate integrity, observe staleness, or gate promotion from experimental to production. Before variants can be safely reintroduced, the platform needs management tooling across all three surfaces (CLI, API, dashboard).

## Requirements Trace

- R1. Shared Rust module (`variants.rs`) with variant metadata struct, 4-rule validation gate, reflection report builder
- R2. `experimental/` directory convention excluded from runtime resolution
- R3. `skill.toml` `[variants]` section parsing (optional, backward-compatible)
- R4. CLI `mika skills variants` subcommands: list, status, diff, reflect, validate, promote, regen
- R5. All CLI subcommands support `--format text|json`
- R6. HTTP API endpoints for all variant operations (dashboard auth for reads, internal auth for writes)
- R7. Dashboard "Skills > Variants" page with skill list, variant table, diff viewer, reflect/validate/promote actions
- R8. Unit tests for validation rules, integration tests for reflect/promote, runtime resolver tests

## Scope Boundaries

- No auto-propagation, auto-fix, or auto-delete of variants (warn-don't-cascade principle)
- `regen` is prepare-only in v1 (prints command string, does not spawn claude-pilot)
- Dashboard `Regen` button is hidden in v1
- No DB-backed variant registry -- filesystem-only
- No semantic similarity, imperative density, or unconditional-directive detection (deferred to v2)
- No CI workflow integration (deferred to mika-skills repo follow-up issue)

### Deferred to Separate Tasks

- CI integration for variant validation on PRs: separate mika-skills issue after this lands
- `regen` end-to-end command (spawn claude-pilot): v2
- Operational panel ("Variant usage this week"): v2
- Composite risk scoring: v2

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/skills/index.rs` -- `SkillEntry`, `resolve_prompt()`, `scan_provider_variants()`, `scan_generated_variants()`, `sanitize_model_dir_name()`, `PromptVariantSource`, `ResolvedPrompt`
- `crates/mika-agent/src/skills/manifest.rs` -- `SkillManifest`, `SkillInfo`, `ProviderSkillOverride`
- `crates/mika-agent/src/skills/mod.rs` -- `SkillRegistry`, `validate_markdown_content()`, `SkillDiagnostic`, `DiagnosticLevel`
- `crates/mika-cli/src/cli.rs` -- `SkillsCommand` enum, `OutputFormat`, `SkillLlmAction` subcommand pattern
- `crates/mika-cli/src/commands/skills.rs` -- CLI dispatch pattern with path resolution
- `crates/mika-agent/src/server/mod.rs` -- `build_router()`, dashboard route registration pattern
- `crates/mika-agent/src/server/dashboard.rs` -- handler pattern with `State(state)`, `Json` responses
- `dashboard/src/pages/` -- page component pattern (Tasks.tsx reference)
- `dashboard/src/api/` -- API client module pattern with TanStack Query hooks

### Institutional Learnings

- Provider dirs must match `ProviderKind::config_prefix()` values; model dirs use `sanitize_model_dir_name()` (slash-to-`--`)
- `generated/` is a separate namespace; hand-authored always wins at resolution time
- Reuse `OutputFormat` enum for `--format text|json` -- never create a new one
- Reuse `SkillDiagnostic` with `ok()`/`warn()`/`fail()` constructors for validation output
- Canonicalize paths when comparing for `--link` mode correctness
- Dashboard pages follow the 4-layer pattern: DB getter -> async wrapper -> handler -> route
- Context validation is bidirectional: placeholders vs `[context]` declarations across all variants
- `collect_skill_files` in `build_support/bundled_skills_discover.rs` does NOT recurse into variant dirs -- bundled skill variants require filesystem path at runtime

## Key Technical Decisions

- **Staleness detection uses `mtime` comparison**: Variant file mtime vs base `system_prompt.md` mtime. Simple, filesystem-native, imperfect after git operations (documented limitation). Git-based staleness deferred to v2.
- **`experimental/` is a reserved directory name**: Added to a `RESERVED_VARIANT_DIRS` constant alongside `generated` in `index.rs`. Explicit skip in `scan_provider_variants` prevents future refactors from accidentally resolving experimental variants.
- **`diff` defaults to runtime-resolved variant**: When a provider/model has both hand-authored and generated variants, `diff` targets the one that would win at resolution time. `--source experimental|generated|hand-authored` flag for explicit selection.
- **`promote` blocks when hand-authored variant exists at target**: Returns a clear error explaining the hand-authored variant would shadow the promoted one. No `--force` in v1 -- remove the hand-authored variant manually first.
- **`promote` warns but proceeds for symlinked skills**: Follows the existing `write_skill_variant` pattern -- warn and write through.
- **Required section headings for validation**: Configurable per-skill via optional `[variants] required_sections` in `skill.toml`, with a sensible default empty list. Skills without the section pass this rule trivially.
- **`regen` prints a `mika ask` invocation**: `mika ask --enable-skill skill-review "review and generate variant for <skill> targeting <provider>/<model>"` -- matches how variant generation works today via `review_skill`.
- **Validation on experimental variants**: `validate` runs on all variants by default but `promote` is the actual gate. Experimental variants can be validated explicitly for early feedback without blocking authoring.
- **Variant module is pure computation**: `variants.rs` contains no DB access. All data comes from `SkillEntry` fields and filesystem reads. This keeps it testable and avoids coupling to `AsyncDatabase`.

## Open Questions

### Resolved During Planning

- **What constitutes "stale"?**: `mtime` comparison of variant vs base `system_prompt.md`. Documented limitation for git-normalized timestamps.
- **Should `promote` detect hand-authored shadowing?**: Yes, block with clear error message.
- **Which variant does `diff` target by default?**: Runtime-resolved (hand-authored > generated). `--source` flag for explicit selection.
- **What are "required section headings"?**: Configurable per-skill via `[variants]` in `skill.toml`. Empty default (all pass).
- **What command does `regen` print?**: `mika ask --enable-skill skill-review "..."` invocation string.

### Deferred to Implementation

- Exact diff classification algorithm (pure-wording / semantic / structural) -- will depend on available diff library
- Token counting method for variant size metrics -- may use simple whitespace-split estimation or tiktoken if available
- Dashboard layout fine-tuning -- exact column widths, responsive breakpoints

## Output Structure

```
crates/mika-agent/src/skills/
  variants.rs                          # NEW: shared variant management module

crates/mika-cli/src/commands/
  skills.rs                            # MODIFY: add Variants match arm
  skills_variants.rs                   # NEW: variant subcommand handlers

crates/mika-agent/src/server/
  mod.rs                               # MODIFY: add variant routes
  variants.rs                          # NEW: HTTP handler functions

dashboard/src/
  api/variants.ts                      # NEW: API client + hooks
  pages/SkillVariants.tsx              # NEW: main variants page
  components/VariantDiffViewer.tsx      # NEW: side-by-side diff component
```

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```mermaid
graph TD
    subgraph "Shared Module (variants.rs)"
        VM[VariantMetadata] --> VG[ValidationGate<br/>4 rules]
        VM --> RR[ReflectionReport]
        VM --> DD[DiffReport]
        SC[scan_all_variants] --> VM
    end

    subgraph "CLI"
        CLI[mika skills variants] --> SC
        CLI --> VG
        CLI --> RR
        CLI --> DD
        CLI --> PM[promote_variant]
    end

    subgraph "HTTP API"
        API[/api/v1/skills/variants/] --> SC
        API --> VG
        API --> RR
        API --> DD
        API --> PM
    end

    subgraph "Dashboard"
        UI[SkillVariants page] --> API
    end

    subgraph "Runtime (existing)"
        RP[resolve_prompt] -.->|skips experimental/| VM
    end
```

## Implementation Units

- [x] **Unit 1: Variant metadata and scanning (`variants.rs`)**

**Goal:** Create the shared variant management module with data types and filesystem scanning.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Create: `crates/mika-agent/src/skills/variants.rs`
- Modify: `crates/mika-agent/src/skills/mod.rs` (add `pub mod variants;`)
- Test: `crates/mika-agent/src/skills/variants.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Define `VariantMetadata` struct: skill name, provider, model, source type (hand-authored/generated/experimental), file path, size bytes, line count, last modified timestamp, staleness flag vs base
- Define `VariantSource` enum: `HandAuthored`, `Generated`, `Experimental`
- Define `RESERVED_VARIANT_DIRS: &[&str] = &["generated", "experimental"]` constant
- `scan_all_variants(skill_dir, manifest) -> Vec<VariantMetadata>` walks the skill directory for all three source types. Reuses `sanitize_model_dir_name` and `ProviderKind` gating from `index.rs`
- `scan_experimental_variants(skill_dir, manifest) -> HashMap<String, String>` mirrors `scan_generated_variants` for the `experimental/` directory
- `compute_staleness(base_mtime, variant_mtime) -> bool` simple mtime comparison
- `VariantSummary` struct for cross-skill aggregation: total, stale, experimental counts per skill

**Patterns to follow:**
- `scan_generated_variants()` in `index.rs` for directory walking pattern
- `ProviderKind` gating on subdirectory names
- Symlink defense-in-depth from `scan_generated_variants`

**Test scenarios:**
- Happy path: scan a skill dir with hand-authored, generated, and experimental variants -> correct `VariantMetadata` for each
- Happy path: scan a skill dir with only a base prompt -> empty variant list
- Edge case: provider dir name not matching `ProviderKind` -> skipped
- Edge case: dotfile/dotdir in variant tree -> skipped
- Edge case: symlinked variant directory -> skipped (defense-in-depth)
- Edge case: model name with slashes -> correctly sanitized key
- Happy path: staleness flag set when variant mtime < base mtime
- Edge case: missing base `system_prompt.md` -> variants still listed, staleness marked as unknown

**Verification:**
- `cargo test -p mika-agent` passes with all variant scanning tests green
- Module compiles and is accessible from `crate::skills::variants`

---

- [x] **Unit 2: Validation gate (4 rules)**

**Goal:** Implement the 4-rule validation gate for variant prompts.

**Requirements:** R1, R8

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/skills/variants.rs`
- Test: `crates/mika-agent/src/skills/variants.rs` (inline tests)

**Approach:**
- Define `ValidationRule` enum: `SizeLimit`, `RequiredSections`, `MarkdownWellFormedness`, `ToolReferenceLint`
- Define `ValidationResult` struct: rule, passed, message, severity (using `DiagnosticLevel`)
- `validate_variant(content, manifest, tool_names) -> Vec<ValidationResult>`:
  1. **Size limit**: check content bytes against `manifest.skill.max_prompt_size` (or default `MAX_PROMPT_SNIPPET_SIZE`). Also check `MIN_VARIANT_RATIO` (0.5) against base prompt size.
  2. **Required sections**: parse `[variants] required_sections` from manifest. Check that each listed heading exists in content (markdown `## Heading` or `# Heading` match).
  3. **Markdown well-formedness**: reuse existing `validate_markdown_content()` checks (null bytes, control chars, unclosed code fences).
  4. **Tool reference lint**: scan for `tool_name(` or `` `tool_name` `` patterns in content, cross-reference against provided `tool_names` set. Warn (not fail) on unrecognized references -- false positives are expected.
- Rule 4 accepts tool names as a parameter (caller-provided, not self-resolved) so the module stays pure

**Patterns to follow:**
- `validate_markdown_content()` in `skills/mod.rs` for existing markdown checks
- `SkillDiagnostic::warn()`/`SkillDiagnostic::fail()` for severity classification

**Test scenarios:**
- Happy path: valid variant within size limit, all required sections present, well-formed markdown -> all pass
- Error path: variant exceeds size limit -> SizeLimit fails with byte count details
- Error path: variant below MIN_VARIANT_RATIO of base -> SizeLimit fails
- Edge case: no required sections configured -> RequiredSections passes trivially
- Error path: missing required section heading -> RequiredSections fails listing missing headings
- Error path: unclosed code fence -> MarkdownWellFormedness fails
- Error path: null bytes in content -> MarkdownWellFormedness fails
- Happy path: tool reference matching known tool -> ToolReferenceLint passes
- Edge case: tool reference not matching any known tool -> ToolReferenceLint warns (not fails)
- Edge case: empty tool_names set -> ToolReferenceLint passes (nothing to lint against)

**Verification:**
- All validation rule tests pass
- Validation output uses `SkillDiagnostic` formatting consistently

---

- [x] **Unit 3: Diff and reflection reports**

**Goal:** Implement variant-vs-base diffing with classification and the reflection report builder.

**Requirements:** R1

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/skills/variants.rs`
- Test: `crates/mika-agent/src/skills/variants.rs` (inline tests)

**Approach:**
- `diff_variant(base_content, variant_content) -> DiffReport`: line-by-line diff using the `similar` crate (already a dependency or add if not). Classify changes: structural (heading additions/removals), content (body text changes), cosmetic (whitespace only). Report missing sections from base.
- `DiffReport` struct: added lines, removed lines, classification, missing sections list
- `reflect_skill(skill_dir, manifest) -> ReflectionReport`: reads base prompt, iterates all production variants (`generated/` + hand-authored), computes diff against base for each, builds per-variant impact summary with recommended next actions. Pure computation, no writes.
- `ReflectionReport` struct: base path, base last modified, list of `VariantImpact` (variant metadata + diff summary + staleness + recommended actions)
- Recommended actions are string constants: "Re-review variant", "Variant is up to date", "Consider regenerating"

**Patterns to follow:**
- `similar` crate TextDiff for unified diff output
- Structured report output suitable for both text and JSON serialization

**Test scenarios:**
- Happy path: base and variant with known diffs -> correct classification and line counts
- Happy path: identical base and variant -> "no changes" report
- Edge case: variant has extra sections not in base -> classified as structural addition
- Edge case: base has sections missing from variant -> listed in missing sections
- Happy path: reflect with 2 variants, one stale one fresh -> correct staleness flags and different recommendations
- Edge case: reflect with no variants -> empty impact list, clean report
- Integration: create temp dir with base + variant, edit base, verify reflect reports drift

**Verification:**
- Diff reports correctly classify change types
- Reflection reports never write to the filesystem

---

- [x] **Unit 4: `skill.toml` `[variants]` section and `experimental/` directory convention**

**Goal:** Add `[variants]` section parsing to `SkillManifest` and exclude `experimental/` from runtime resolution.

**Requirements:** R2, R3

**Dependencies:** None (can be done in parallel with Unit 1)

**Files:**
- Modify: `crates/mika-agent/src/skills/manifest.rs`
- Modify: `crates/mika-agent/src/skills/index.rs`
- Test: `crates/mika-agent/src/skills/manifest.rs` (inline tests), `crates/mika-agent/src/skills/index.rs` (inline tests)

**Approach:**
- Add `VariantsConfig` struct: `providers: Option<Vec<String>>`, `max_prompt_size: Option<u64>`, `required_sections: Option<Vec<String>>`
- Add `#[serde(default)] pub variants: VariantsConfig` to `SkillManifest`
- Add `RESERVED_VARIANT_DIRS` constant in `index.rs`
- Add explicit skip for `"experimental"` in `scan_provider_variants()` -- before `ProviderKind` parse, check against reserved names
- `scan_generated_variants` already only walks `generated/`, so no change needed there
- The `experimental` name does not parse as `ProviderKind`, providing implicit defense. The explicit check is defense-in-depth.

**Patterns to follow:**
- `#[serde(default)]` on `SkillManifest` fields for backward compatibility
- `ProviderKind::from_str()` gating in `scan_provider_variants`

**Test scenarios:**
- Happy path: skill.toml with `[variants]` section parses correctly
- Happy path: skill.toml without `[variants]` section -> default empty config
- Happy path: skill.toml with `[variants] required_sections = ["Tools", "Constraints"]` -> parsed correctly
- Edge case: `[variants] providers = []` -> empty vec, not None
- Integration: `resolve_prompt` with experimental variant present -> returns base or generated, never experimental
- Integration: `scan_provider_variants` with `experimental/` directory present -> not included in results

**Verification:**
- Existing skill.toml files continue to parse without changes
- Experimental variants are never loaded into `model_prompts` or `generated_model_prompts`

---

- [x] **Unit 5: Promote operation**

**Goal:** Implement the `promote` operation that moves an experimental variant to generated after validation.

**Requirements:** R1

**Dependencies:** Unit 1, Unit 2, Unit 4

**Files:**
- Modify: `crates/mika-agent/src/skills/variants.rs`
- Test: `crates/mika-agent/src/skills/variants.rs` (inline tests)

**Approach:**
- `promote_variant(skill_dir, manifest, provider, model, tool_names) -> Result<PromoteResult>`:
  1. Resolve source path: `experimental/{provider}/{sanitized_model}/system_prompt.md`
  2. Verify source exists, bail if not
  3. Check for hand-authored variant at `{provider}/{sanitized_model}/system_prompt.md` -- if exists, return error explaining shadowing
  4. Run `validate_variant` on source content -- if any rule fails, return error with violations
  5. Create target directory `generated/{provider}/{sanitized_model}/` if needed
  6. Copy content to target (not move -- copy then delete for atomicity)
  7. Also copy `skill.toml` override if it exists in experimental source
  8. Delete only expected files (`system_prompt.md`, `skill.toml`) from experimental source, then `remove_dir` on the model and provider directories if empty. Do NOT use `remove_dir_all` -- unexpected files should be preserved with a warning.
  9. Return `PromoteResult` with source/target paths
- Detect symlinked skill directory (`symlink_metadata`), warn but proceed (matching `write_skill_variant` pattern)

**Patterns to follow:**
- `write_skill_variant` in `builtin_handlers.rs` for symlink-aware writes
- `std::fs::create_dir_all` for target directory creation

**Test scenarios:**
- Happy path: promote experimental variant -> file copied to generated, experimental deleted
- Happy path: promote with skill.toml override in experimental -> both files copied
- Error path: experimental source does not exist -> clear error
- Error path: hand-authored variant exists at target -> error explaining shadowing
- Error path: validation fails (e.g., oversized) -> error with violation details, no file changes
- Edge case: target generated directory already has a variant -> overwritten (re-promotion)
- Edge case: symlinked skill dir -> warning emitted, promotion proceeds
- Integration: promote then scan -> variant appears in generated, not in experimental

**Verification:**
- Filesystem state correct after promotion
- No writes occur when validation fails or hand-authored variant exists

---

- [x] **Unit 6: CLI subcommands**

**Goal:** Add `mika skills variants <op>` subcommand family with all 7 operations.

**Requirements:** R4, R5

**Dependencies:** Units 1-5

**Files:**
- Modify: `crates/mika-cli/src/cli.rs` (add `Variants` to `SkillsCommand`)
- Create: `crates/mika-cli/src/commands/skills_variants.rs`
- Modify: `crates/mika-cli/src/commands/skills.rs` (add `Variants` match arm)
- Modify: `crates/mika-cli/src/commands/mod.rs` (add `pub mod skills_variants;`)
- Test: `crates/mika-cli/src/commands/skills_variants.rs` (inline tests where applicable)

**Approach:**
- Define `SkillVariantsAction` enum (Subcommand derive):
  - `List { name: String, format: OutputFormat }`
  - `Status { format: OutputFormat }`
  - `Diff { name: String, variant: String, source: Option<VariantSourceFilter>, format: OutputFormat }`
  - `Reflect { name: String, format: OutputFormat }`
  - `Validate { name: Option<String>, format: OutputFormat }`
  - `Promote { name: String, variant: String }`
  - `Regen { name: String, variant: String }`
- Add `Variants { #[command(subcommand)] action: SkillVariantsAction }` to `SkillsCommand`
- `variant` argument uses `provider/model` format (parsed with split on first `/`). Model names with internal slashes (e.g., OpenRouter's `anthropic/claude-sonnet-4`) should be passed as-is -- the CLI applies `sanitize_model_dir_name()` internally. Example: `mika skills variants diff self-dev "anthropic/anthropic/claude-sonnet-4"` splits as provider=`anthropic`, model=`anthropic/claude-sonnet-4`.
- Each subcommand handler resolves skill paths, constructs `SkillRegistry`, calls into `variants.rs` functions
- Text output uses tabular formatting for list/status, unified diff for diff, diagnostic badges for validate
- JSON output uses `serde_json::to_string_pretty`

**Patterns to follow:**
- `SkillLlmAction` pattern for nested subcommands
- `OutputFormat` reuse with `#[arg(long, value_enum, default_value = "text")]`
- `run()` dispatch pattern in `skills.rs`
- `SkillDiagnostic` `[OK]`/`[WARN]`/`[FAIL]` formatting for validate output

**Test scenarios:**
- Happy path: `mika skills variants list self-dev --format json` -> valid JSON array of variant metadata
- Happy path: `mika skills variants status` -> summary table with counts
- Happy path: `mika skills variants validate self-dev` -> diagnostic output with per-rule results
- Edge case: `mika skills variants list nonexistent` -> clear "skill not found" error
- Edge case: `mika skills variants diff self-dev anthropic/claude-sonnet-4-6 --source experimental` -> targets experimental variant specifically
- Happy path: `mika skills variants regen self-dev anthropic/claude-sonnet-4-6` -> prints `mika ask` command string

**Verification:**
- All subcommands compile and dispatch correctly
- `--format json` produces valid JSON for all operations
- `--format text` produces human-readable output

---

- [x] **Unit 7: HTTP API endpoints**

**Goal:** Add variant management REST endpoints to mika-server.

**Requirements:** R6

**Dependencies:** Units 1-5

**Files:**
- Create: `crates/mika-agent/src/server/variants.rs`
- Modify: `crates/mika-agent/src/server/mod.rs` (add routes + `pub mod variants;`)
- Test: `crates/mika-agent/src/server/variants.rs` (inline tests)

**Approach:**
- Handlers follow existing dashboard pattern: `State(state): State<AppState>`, return `impl IntoResponse`
- Lock `state.skills`, clone `Arc<SkillRegistry>`, release lock immediately
- GET endpoints use dashboard auth layer; POST promote uses internal auth layer
- Endpoints:
  - `GET /api/v1/skills/variants` -> `handle_variants_summary`: cross-skill summary
  - `GET /api/v1/skills/{skill}/variants` -> `handle_skill_variants`: per-skill list
  - `GET /api/v1/skills/{skill}/variants/{provider}/{model}` -> `handle_variant_detail`: content + metadata
  - `GET /api/v1/skills/{skill}/variants/{provider}/{model}/diff` -> `handle_variant_diff`: diff payload
  - `GET /api/v1/skills/{skill}/variants/reflect` -> `handle_variant_reflect`: audit report
  - `POST /api/v1/skills/{skill}/variants/{provider}/{model}/validate` -> `handle_variant_validate`: run gate
  - `POST /api/v1/skills/{skill}/variants/{provider}/{model}/promote` -> `handle_variant_promote`: promote
- Path parameters: `skill` by name, `provider`/`model` as path segments
- Variant operations need filesystem access to the skill directory -- resolve via `SkillEntry.dir` from the cloned `SkillRegistry` (NOT `home_dir + "skills/{name}/"` which breaks for `--link` mode symlinked skills)

**Patterns to follow:**
- `dashboard.rs` handler pattern with `State`, `Path`, `Query` extractors
- `internal_error()` helper for error responses
- Auth layer separation in `build_router()`
- **Route ordering:** Static `/reflect` route MUST be registered before `/{provider}/{model}` wildcard routes to avoid being swallowed by Axum's path parameter matching

**Test scenarios:**
- Happy path: GET /api/v1/skills/variants -> JSON summary with variant counts per skill
- Happy path: GET /api/v1/skills/self-dev/variants -> JSON array of variant metadata
- Happy path: GET /api/v1/skills/self-dev/variants/anthropic/claude-sonnet-4-6/diff -> JSON diff report
- Error path: GET /api/v1/skills/nonexistent/variants -> 404
- Error path: POST promote with dashboard token only -> 401/403
- Happy path: POST validate -> JSON with pass/fail per rule

**Verification:**
- Routes registered correctly in `build_router`
- Auth layers applied correctly (GET = dashboard, POST promote = internal)
- Response JSON matches what the dashboard will consume

---

- [x] **Unit 8: Dashboard "Skills > Variants" page**

**Goal:** Add the dashboard page for variant management with skill list, variant table, diff viewer, and actions.

**Requirements:** R7

**Dependencies:** Unit 7

**Files:**
- Create: `dashboard/src/api/variants.ts`
- Create: `dashboard/src/pages/SkillVariants.tsx`
- Create: `dashboard/src/components/VariantDiffViewer.tsx`
- Modify: `dashboard/src/App.tsx` (add route)
- Modify: `dashboard/src/components/Sidebar.tsx` (add nav item)
- Test: manual verification via screenshot

**Approach:**
- API client module with interfaces matching server response shapes and TanStack Query hooks
- Main page layout: left panel with skill list (clickable, shows badges for variant/stale/experimental counts), right panel with variant table for selected skill
- Variant table columns: provider, model, source type, size (bytes/lines), staleness indicator, validation status, last modified
- Base-edit amber banner at top of variant table when any variant's mtime is older than base mtime
- "Diff" button per variant row -> opens `VariantDiffViewer` (side-by-side using `<pre>` blocks with color-coded additions/deletions)
- "Reflect" button -> fetches and inlines the reflection report below the variant table
- "Validate" button per variant row -> calls POST validate, shows inline results
- "Promote" button per experimental variant row -> confirmation modal, calls POST promote (hidden when no internal token available)
- "Regen" button hidden in v1
- Use `lucide-react` icons for actions, Tailwind CSS v4 for styling
- Responsive: single-column on narrow screens

**Patterns to follow:**
- `Tasks.tsx` for list page structure with filters
- `LlmCallDetail.tsx` for detail panel layout
- `dashboard/src/api/tasks.ts` for API module structure
- Sidebar `navItems` array for navigation entry

**Test scenarios:**
- Test expectation: none -- dashboard pages are verified via manual screenshot review. The API module type contracts are validated implicitly by TypeScript compilation.

**Verification:**
- Page renders correctly with mock/real data
- Navigation link appears in sidebar
- All action buttons wire to correct API endpoints
- TypeScript compiles without errors

---

- [x] **Unit 9: Integration tests**

**Goal:** Add integration tests covering the end-to-end variant management flows.

**Requirements:** R8

**Dependencies:** Units 1-5

**Files:**
- Create: `crates/mika-agent/tests/variant_management.rs` (or inline in `variants.rs`)
- Test: self

**Approach:**
- Create temp directories with realistic skill structures (base + variants in all three locations)
- Test reflect flow: create base + variant, modify base, verify reflect reports drift and recommendations
- Test promote flow: create experimental variant, run promote, verify it appears in generated and is removed from experimental
- Test promote failure: create experimental + hand-authored at same key, verify promote is blocked
- Test runtime resolver: verify `resolve_prompt` skips experimental variants at all fallback steps

**Patterns to follow:**
- `tempfile::TempDir` for isolated test directories
- Existing `#[cfg(test)]` patterns in `index.rs` for skill scanning tests

**Test scenarios:**
- Integration: base edit -> reflect -> all variants reported with staleness and recommendations
- Integration: experimental variant -> validate (pass) -> promote -> appears in generated
- Integration: experimental variant -> validate (fail) -> promote blocked -> no filesystem change
- Integration: promote with hand-authored shadow -> blocked with clear error
- Integration: full scan -> resolve_prompt for provider/model with experimental variant -> experimental never returned
- Edge case: promote on empty experimental dir -> clear error

**Verification:**
- All integration tests pass
- Tests are deterministic (no mtime races -- use explicit file timestamps where needed)

## System-Wide Impact

- **Interaction graph:** `variants.rs` is consumed by CLI commands and HTTP handlers. Both surfaces call into the same functions. `SkillEntry` gains no new fields in v1 -- variant scanning for management uses its own `scan_all_variants` independent of the startup-time `scan_provider_variants`.
- **Error propagation:** Validation errors are structured (`Vec<ValidationResult>`) and propagated as-is to CLI/HTTP callers. `promote` errors are `anyhow::Result` with descriptive messages.
- **State lifecycle risks:** `promote` is the only write operation. Copy-then-delete ordering prevents data loss on partial failure. The `SkillRegistry` in memory is NOT updated after promote -- the caller must reload (CLI exits, server would need a registry refresh mechanism or a note that the next restart picks it up).
- **Post-promote registry refresh:** The server's existing `skills_dirty: Arc<AtomicBool>` on `AgentState` can signal that the registry needs a rescan after promote. Set it to `true` after a successful promote so the next message handler reloads. CLI exits after promote so no refresh needed.
- **API surface parity:** All 7 operations available via CLI, 7 via HTTP API, read operations + promote/validate actions via dashboard. `regen` is CLI-only (prints a command string).
- **Unchanged invariants:** `resolve_prompt()` fallback chain is unchanged. `scan_provider_variants()` and `scan_generated_variants()` behavior for existing variants is unchanged. `SkillEntry` struct fields are unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `mtime` staleness is unreliable after git clone/checkout | Document limitation; staleness is advisory, not blocking. Git-based staleness deferred to v2. |
| Tool reference lint has high false-positive potential | Rule 4 warns but never fails -- users see tool references flagged but promotion is not blocked by stale tool references alone |
| `promote` write on symlinked skill affects shared source | Follow existing `write_skill_variant` pattern: warn and proceed. Document that linked skills share variant state. |
| Dashboard page adds bundle size | Lazy-load the SkillVariants page via React.lazy + Suspense |
| `similar` crate dependency (if not already present) | Well-maintained, widely-used Rust diff library. Small dependency footprint. |

## Documentation / Operational Notes

- Update `docs/skills.md` to document the `experimental/` directory convention and the `[variants]` section in `skill.toml`
- Update `crates/mika-cli/CLAUDE.md` to list the new `mika skills variants` subcommands
- Update `crates/mika-agent/CLAUDE.md` to mention the `variants.rs` module in the Skills System section
- Update `docs/openapi/mika-server.yaml` with the new variant endpoints

## Sources & References

- Related issues: #541
- Related code: `crates/mika-agent/src/skills/index.rs` (variant resolution), `crates/mika-agent/src/skills/builtin_handlers.rs` (`review_skill`, `write_skill_variant`)
- Institutional learnings: `docs/solutions/architecture-patterns/per-provider-skill-variant-directories.md`, `docs/solutions/architecture-patterns/harden-write-skill-variant-no-path-input.md`, `docs/solutions/architecture-patterns/cli-format-json-nine-commands.md`
- Related commits: `mika-skills@15bcc45` (variant deletion that motivated this ticket)
