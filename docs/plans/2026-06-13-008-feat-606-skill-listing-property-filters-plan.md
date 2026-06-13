---
ticket: mika#606
branch: feat/606/skill-listing-property-filters
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/606
execution: code
---

# Plan: skill-listing property filters (mika#606)

## Problem frame

When listing skills via any transport (CLI `mika skills list`, HTTP `/api/v1/skills` dashboard endpoint, future transports), callers cannot filter by skill properties (source, activation). They must filter client-side, which diverges per transport and bloats responses.

## Architecture (committed by issue body)

The issue body already commits to all structural decisions. This plan honors them:

- **`SkillListFilter` struct** — plain Rust struct in skills core, `Option<T>` fields per predicate. Two fields for v1: `source: Option<SkillSource>`, `always_on: Option<bool>`. Room to grow.
- **`list_skills(filter)`** — applies the filter via iterator `.filter()`. No parsing, no transport concerns.
- **Per-transport adapters** — parse native input (CLI flags, HTTP query params) into `SkillListFilter`. No business logic.
- **YAGNI bounds**: two properties only. No generic predicate engine, no builder pattern, no trait abstraction.
- **DRY reference pattern**: `validate_and_resolve_path` in `crates/mika-agent/src/tools/mod.rs` — single named helper, multiple callers.
- **HTTP query-param convention**: flat `?source=bundle&always_on=true` (matches mika#659's `from?: string` pattern).

## Scope boundaries

- New `SkillListFilter` struct + `SkillSource` enum (`Bundle`, `Marketplace`) in `crates/mika-agent/src/skills/` (or wherever skill list-helpers live).
- `list_skills(filter: SkillListFilter)` function or method.
- CLI adapter: `mika skills list --source <bundle|marketplace> --always-on <true|false>` flags → `SkillListFilter`.
- HTTP adapter: query params on dashboard `/api/v1/skills` endpoint → `SkillListFilter`.
- **Out of scope:** filter on other properties (deferred per YAGNI); `mika skills list --json` filter pre-processing on the consumer side (transport adapter is the right boundary); skill-search / fuzzy-match (separate concern); filter persistence (no API state).

## Implementation Units

### U1 — `SkillListFilter` struct + `SkillSource` enum

**Goal:** Filter data type lives in skills core, transport-agnostic.

**Files:**
- Modify: `crates/mika-agent/src/skills/mod.rs` (or `skills/filter.rs` if implementer prefers a separate module)

**Approach:**

```rust
/// Property filter for skill listing (mika#606). Plain data, no behavior.
/// Add a field + a match arm in `apply_filter` when a new property is needed.
#[derive(Debug, Clone, Default)]
pub struct SkillListFilter {
    pub source: Option<SkillSource>,
    pub always_on: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource {
    Bundle,
    Marketplace,
}

impl SkillSource {
    /// Parse from a CLI / HTTP string value. Case-insensitive.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bundle" | "bundled" => Some(Self::Bundle),
            "marketplace" => Some(Self::Marketplace),
            _ => None,
        }
    }
}
```

**Test scenarios:**
- **Default filter:** `SkillListFilter::default()` has `None` for all fields (no filtering).
- **Parse SkillSource:** `"bundle"`, `"Bundle"`, `"BUNDLED"` → `Some(Bundle)`; `"marketplace"` → `Some(Marketplace)`; `"junk"` → `None`.

**Verification:** unit tests in the new module.

### U2 — `list_skills` filter application

**Goal:** A function that takes the filter and returns filtered skill entries.

**Files:**
- Modify: `crates/mika-agent/src/skills/mod.rs` (the existing list helper, or add a new `list_skills_filtered` if the existing one is heavily used unchanged)

**Approach:**

```rust
/// Apply the filter to a skill iterator. Each predicate is independent (AND semantics).
pub fn apply_filter<'a>(
    skills: impl Iterator<Item = &'a SkillEntry>,
    filter: &SkillListFilter,
) -> impl Iterator<Item = &'a SkillEntry> {
    skills.filter(move |s| {
        if let Some(want) = filter.source {
            if s.source() != want { return false; }
        }
        if let Some(want) = filter.always_on {
            if s.always_on != want { return false; }
        }
        true
    })
}
```

The body's hard constraint: "No macros, no builder pattern, no generics over filter types." A plain `match`-style if-let chain inside `.filter()` is the simplest correct shape. Each field independent → AND logic between fields, no precedence concerns.

Existing `list_skills()` callers that don't care about filtering pass `SkillListFilter::default()`.

**Test scenarios:**
- **Empty filter:** all skills pass.
- **Source filter:** only matching-source skills pass.
- **Activation filter:** only matching-always_on skills pass.
- **Composed filter (AND):** both predicates apply; only skills matching ALL conditions pass.
- **No matches:** filter that excludes everything returns empty iterator.

**Verification:** unit tests on a synthetic skill set.

### U3 — CLI adapter

**Goal:** `mika skills list --source <name> --always-on <bool>` parses into `SkillListFilter`.

**Files:**
- Modify: `crates/mika-cli/src/commands/skills.rs` (or wherever the `mika skills list` subcommand is dispatched)
- Modify: `crates/mika-cli/src/cli.rs` (`SkillsListArgs` struct or equivalent)

**Approach:**

```rust
#[derive(clap::Args)]
pub struct SkillsListArgs {
    // ... existing fields ...

    /// Filter by source: bundle or marketplace.
    #[arg(long)]
    pub source: Option<String>,

    /// Filter by always-on state.
    #[arg(long = "always-on")]
    pub always_on: Option<bool>,
}
```

In the command handler:

```rust
let filter = SkillListFilter {
    source: args.source.as_deref().and_then(SkillSource::parse),
    always_on: args.always_on,
};
let skills = apply_filter(registry.entries.iter(), &filter).collect::<Vec<_>>();
```

If `--source` is provided but unparseable, return an error to the user. clap's `Option<bool>` handles "true"/"false" naturally.

**Test scenarios:**
- **`mika skills list` (no flags):** all skills shown (current behavior).
- **`mika skills list --source bundle`:** only bundled skills.
- **`mika skills list --always-on true`:** only always-on skills.
- **`mika skills list --source marketplace --always-on false`:** AND semantics — marketplace skills that are NOT always-on.
- **`mika skills list --source nonsense`:** error message naming the valid values.

**Verification:** integration tests via `assert_cmd` or shell smoke; existing `mika skills list` tests still pass.

### U4 — HTTP adapter

**Goal:** Dashboard `/api/v1/skills` endpoint accepts `?source=` and `?always_on=` query params.

**Files:**
- Modify: `crates/mika-agent/src/server/dashboard.rs` (or the skills endpoint handler)

**Approach:**

```rust
#[derive(serde::Deserialize)]
struct SkillsListQuery {
    source: Option<String>,
    always_on: Option<String>,
}

pub async fn handle_skills_list(
    State(state): State<AppState>,
    Query(q): Query<SkillsListQuery>,
) -> impl IntoResponse {
    let filter = SkillListFilter {
        source: q.source.as_deref().and_then(SkillSource::parse),
        always_on: q.always_on.as_deref().and_then(|s| s.parse::<bool>().ok()),
    };
    let skills = apply_filter(state.skills.entries.iter(), &filter).collect::<Vec<_>>();
    Json(skills)
}
```

Query params are strings (per HTTP convention + mika#659's `from?: string`), parsed via `SkillSource::parse` and `str::parse::<bool>`. Invalid param values are silently ignored (no filter applied for that property) per the established convention — this is consistent with #659's tolerant parsing.

**Test scenarios:**
- **No query params:** all skills returned.
- **`?source=bundle`:** filtered set.
- **`?source=bundle&always_on=true`:** AND filtered.
- **`?source=junk`:** returns all skills (silent invalid-value tolerance per #659 convention).

**Verification:** integration tests on the HTTP endpoint; existing tests pass.

### U5 — Docs update

**Goal:** Document the filter feature in CLAUDE.md.

**Files:**
- Modify: `crates/mika-cli/CLAUDE.md` § Skills CLI (or wherever `mika skills list` is documented)
- Modify: `crates/mika-agent/CLAUDE.md` if the HTTP `/api/v1/skills` endpoint has a documented surface

**Approach:** Add a short note describing the two flags + their HTTP query-param equivalents.

**Verification:** manual read.

## Dependencies / sequencing

- U1 → U2 (U2 uses the filter type from U1)
- U1 → U3 → U4 (U3 and U4 are independent transport adapters, both use U1's filter)
- U5 (docs) ships in same PR; last

## Patterns to follow (cross-cutting)

- `crates/mika-agent/src/tools/mod.rs::validate_and_resolve_path` — named DRY helper called from multiple sites, per body's citation
- `crates/mika-agent/src/server/dashboard.rs` (existing time-range filtering pattern from mika#659) — HTTP query-param convention
- `crates/mika-cli/src/commands/` — existing `mika skills` subcommand structure

## Verification (top-level)

- `cargo test -p mika-agent skills::tests` — passes (existing + new filter tests)
- `cargo test -p mika-cli` — CLI integration tests pass
- `cargo clippy --workspace` clean
- `cargo fmt --all -- --check` clean
- Manual smoke: run `mika skills list --source bundle` and `curl 'http://localhost:8081/api/v1/skills?source=bundle'` — both return the same filtered set

## Risk / known unknowns

- **`SkillEntry::source()` accessor existence.** The plan assumes there's a way to read a skill's source (Bundle vs Marketplace) from the `SkillEntry` type. If the field isn't directly accessible, implementer adds an accessor matching the existing four-tier origin enum (`[built-in]`, `[marketplace]`, `[marketplace/linked]`, `[custom]` — per CLAUDE.md § Skills System). The plan groups `[built-in]` as `Bundle` and the three `marketplace*` as `Marketplace`; `[custom]` may need a third enum variant or be grouped with one of the two — implementer chooses based on what makes the filter useful for operators.
- **HTTP query-param tolerance vs strict parse.** The plan chose tolerance (invalid value → no filter applied for that property) per #659's convention. If operator UX strongly prefers strict (400 on invalid param), this is a small revision. Not a structural concern.
- **`always_on` field name conflict.** If a skill's `always_on` semantics overlap with `enabled`/`disabled`, the filter may need a join. Per CLAUDE.md, `always_on` is the AlwaysOn / Keyword match-reason distinction — independent from the `enabled` tri-state. No conflict.

## Out-of-scope (explicit)

- Filter on other properties (description text, dependencies, required_tools, etc.) — deferred per YAGNI.
- Skill search / fuzzy-match — separate concern.
- Filter persistence / saved-filter API — no state.
- Bitflags or generic predicate engine — explicitly rejected by issue body.
- JSON-body filters on POST — query-string-only.
