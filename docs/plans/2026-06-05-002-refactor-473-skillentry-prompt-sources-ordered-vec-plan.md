# Plan: SkillEntry prompt sources as ordered Vec (#473)

**Issue:** senara-solutions/mika#473
**Type:** refactor
**Risk:** medium — touches SkillEntry which is referenced across ~20 files (skills module, CLI, agent tests, KG domain builder)

## Problem

`SkillEntry` carries two parallel prompt maps:

- `model_prompts: HashMap<String, String>` — hand-authored variants
- `generated_model_prompts: HashMap<String, String>` — auto-generated variants

`resolve_prompt()` walks them in a hardcoded 4-step fallback chain: hand-authored → generated (requesting key) → generated (canonical key) → root. Adding a third source (e.g., marketplace-shipped variants) requires a new field on `SkillEntry`, a new scan function in `skills::index`, a new branch in `resolve_prompt`, and updating every test fixture that initializes `SkillEntry`. This is O(n) edits per new source.

## Design

### New type: `PromptSource` enum

```rust
/// Identifies the origin tier of a prompt map.
/// Variants are ordered by priority — earlier variants win in `resolve_prompt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSource {
    /// Hand-authored model variant under `<provider>/<model>/`.
    HandAuthored,
    /// Auto-generated variant under `generated/<provider>/<model>/`.
    Generated,
    // Future: Marketplace, RemoteFetched, etc.
}
```

Note: `variants.rs` already has a `VariantSource` enum for the management/scanning surface (includes `Experimental`). `PromptSource` is the runtime-resolution counterpart — it only includes sources that participate in `resolve_prompt`. The two enums are intentionally separate: `VariantSource` is for listing/diffing/reflection; `PromptSource` is for resolution priority.

### Replace dual maps with ordered Vec

```rust
pub struct SkillEntry {
    // ... existing fields ...

    /// Ordered prompt sources. Each entry is a (source tier, key→prompt map).
    /// Resolution walks sources in order; first match wins.
    /// Constructed at scan time with HandAuthored first, Generated second.
    pub prompt_sources: Vec<(PromptSource, HashMap<String, String>)>,

    // model_prompts and generated_model_prompts fields REMOVED
}
```

### Updated `resolve_prompt`

The new implementation becomes a single iterator chain:

```rust
pub fn resolve_prompt(&self, provider: &str, model: &str) -> ResolvedPrompt<'_> {
    let requesting_key = format!("{}/{}", provider, sanitize_model_dir_name(model));

    // Walk sources in priority order — first match wins
    for (source, map) in &self.prompt_sources {
        if let Some(prompt) = map.get(&requesting_key) {
            return ResolvedPrompt {
                text: prompt,
                source: source.to_variant_source(),
                key: Some(requesting_key),
            };
        }

        // Canonical-key fallback only for Generated sources
        // (hand-authored variants are authored against their requesting provider explicitly)
        if *source == PromptSource::Generated {
            let (canonical_provider, canonical_model) =
                resolve_canonical_provider_model(provider, model);
            if canonical_provider != provider || canonical_model != model {
                let canonical_key = format!(
                    "{}/{}",
                    canonical_provider,
                    sanitize_model_dir_name(canonical_model)
                );
                if let Some(prompt) = map.get(&canonical_key) {
                    return ResolvedPrompt {
                        text: prompt,
                        source: PromptVariantSource::GeneratedCanonical,
                        key: Some(canonical_key),
                    };
                }
            }
        }
    }

    // No variant matched — fall back to root prompt
    ResolvedPrompt {
        text: &self.prompt_snippet,
        source: PromptVariantSource::Base,
        key: None,
    }
}
```

### PromptVariantSource mapping

`PromptVariantSource` (the existing 4-variant enum) stays unchanged for backward compatibility — it describes the resolution *step*, not the source tier. A helper method on `PromptSource` maps to the appropriate variant:

```rust
impl PromptSource {
    fn to_variant_source(&self) -> PromptVariantSource {
        match self {
            PromptSource::HandAuthored => PromptVariantSource::HandAuthoredModel,
            PromptSource::Generated => PromptVariantSource::GeneratedModel,
        }
    }
}
```

`GeneratedCanonical` is handled inline in the canonical-key fallback (remains a Generated-only behavior).

### Accessor methods for backward compatibility

To minimize churn, add convenience accessor methods on `SkillEntry`:

```rust
impl SkillEntry {
    /// Access hand-authored model prompts (first source tier).
    pub fn model_prompts(&self) -> &HashMap<String, String> {
        self.prompt_sources
            .iter()
            .find(|(s, _)| *s == PromptSource::HandAuthored)
            .map(|(_, m)| m)
            .unwrap_or(&EMPTY_MAP)
    }

    /// Access generated model prompts (second source tier).
    pub fn generated_model_prompts(&self) -> &HashMap<String, String> {
        self.prompt_sources
            .iter()
            .find(|(s, _)| *s == PromptSource::Generated)
            .map(|(_, m)| m)
            .unwrap_or(&EMPTY_MAP)
    }
}

static EMPTY_MAP: std::sync::LazyLock<HashMap<String, String>> =
    std::sync::LazyLock::new(HashMap::new);
```

These methods let the CLI display code and tests continue using `.model_prompts()` and `.generated_model_prompts()` with minimal diff — just adding `()` to existing field accesses. Later, callers that only care about "all variants" can iterate `prompt_sources` directly.

## Implementation Steps

### Step 1: Add `PromptSource` enum and `prompt_sources` field

**File:** `crates/mika-agent/src/skills/index.rs`

1. Define `PromptSource` enum (near `PromptVariantSource`).
2. Add `prompt_sources: Vec<(PromptSource, HashMap<String, String>)>` field to `SkillEntry`.
3. Remove `model_prompts` and `generated_model_prompts` fields from `SkillEntry`.
4. Add `model_prompts()` and `generated_model_prompts()` accessor methods.
5. Add `EMPTY_MAP` static for the accessor fallback.

### Step 2: Update `VariantScanResult` and construction

**File:** `crates/mika-agent/src/skills/index.rs`

1. Replace `model_prompts` and `generated_model_prompts` fields in `VariantScanResult` with `prompt_sources: Vec<(PromptSource, HashMap<String, String>)>`.
2. Update `scan_provider_variants()` to construct the vec: `[(HandAuthored, model_prompts), (Generated, generated_model_prompts)]`.
3. Update the `SkillEntry` construction site (around line 574) to use the new shape.
4. Update bundled-skill construction (the code path that builds `SkillEntry` from `BUNDLED_SKILL_MANIFESTS`) similarly.

### Step 3: Rewrite `resolve_prompt`

**File:** `crates/mika-agent/src/skills/index.rs`

Replace the current 4-step `if let` chain with the iterator-based implementation described above. The canonical-key fallback for `Generated` sources is kept as a source-specific behavior inside the loop body.

### Step 4: Update `variant_providers()` and `variant_models()`

**File:** `crates/mika-agent/src/skills/index.rs`

Both methods iterate `model_prompts.keys()` directly. Update them to iterate all maps in `prompt_sources` instead.

### Step 5: Update CLI display code

**File:** `crates/mika-cli/src/commands/skills.rs`

Replace `entry.model_prompts` and `entry.generated_model_prompts` direct field accesses with method calls `entry.model_prompts()` and `entry.generated_model_prompts()`. ~6 sites.

### Step 6: Update test fixtures

**Files:** Multiple test files across `crates/mika-agent/`

All test code that constructs `SkillEntry` with `model_prompts: HashMap::new(), generated_model_prompts: HashMap::new()` needs updating to use `prompt_sources: vec![(PromptSource::HandAuthored, HashMap::new()), (PromptSource::Generated, HashMap::new())]`.

Affected files (~20 sites):
- `crates/mika-agent/src/skills/mod.rs` (1 site)
- `crates/mika-agent/src/skills/review_filter.rs` (1 site)
- `crates/mika-agent/src/skills/matcher.rs` (1 site)
- `crates/mika-agent/src/skills/index.rs` (~30 sites in tests)
- `crates/mika-agent/src/agent.rs` (3 sites)
- `crates/mika-agent/src/kg/domain_builder.rs` (1 site)
- `crates/mika-agent/tests/eval/*.rs` (~8 files)

To reduce this churn, consider a constructor helper:

```rust
#[cfg(test)]
impl SkillEntry {
    /// Empty prompt sources for test fixtures.
    pub fn empty_prompt_sources() -> Vec<(PromptSource, HashMap<String, String>)> {
        vec![
            (PromptSource::HandAuthored, HashMap::new()),
            (PromptSource::Generated, HashMap::new()),
        ]
    }
}
```

### Step 7: Verify existing tests pass

Run `cargo test -p mika-agent` and `cargo clippy`. The `resolve_prompt` tests are the critical validation — they cover all four fallback steps and must produce identical `PromptVariantSource` values.

## Non-Goals

- **No new sources in this PR.** This is a structural refactor only. The ticket says "file this as the refactor to do before adding a third" — we're preparing the shape, not adding the third source.
- **No changes to `variants.rs`** — the `VariantSource` enum for management/scanning is a separate concern and already supports three tiers (HandAuthored, Generated, Experimental). The Experimental tier doesn't participate in `resolve_prompt` and isn't a prompt source.
- **No changes to `PromptVariantSource`** — the existing 4-variant enum stays unchanged for API/logging compatibility.
- **No serialization changes** — the accessor methods maintain the same shape for JSON output in the CLI.

## Testing

- All existing `resolve_prompt` tests must pass unchanged (same inputs → same outputs).
- All existing test fixtures compile with the new shape.
- `cargo test -p mika-agent` passes.
- `cargo clippy` clean.
- No behavioral changes — this is a pure refactor.

## Rollback

Revert the PR. No data migration, no schema changes, no runtime state.
