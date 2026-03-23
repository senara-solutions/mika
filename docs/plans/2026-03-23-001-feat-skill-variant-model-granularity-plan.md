---
title: Extend skill variant directories to support provider + model granularity
type: feat
status: active
date: 2026-03-23
issue: 246
prerequisite: "#241 (per-provider skill variants) — merged"
---

# Extend Skill Variant Directories to Support Provider + Model Granularity

## Overview

Issue #241 added per-provider skill variant directories (`anthropic/`, `minimax/`, etc.) with a two-level resolution chain: provider-specific prompt → root fallback. This plan extends that to three-level resolution by adding model-specific subdirectories within provider directories, enabling prompts tuned for specific models (e.g., Claude Sonnet 4.6 vs Opus 4.6, MiniMax M2.7 vs M2.5).

## Problem Statement

Different models from the same provider behave differently with the same prompt. A Claude Sonnet 4.6 prompt may need different tool calling guidance than Opus 4.6. A MiniMax M2.7 prompt differs from M2.5. The current per-provider granularity is insufficient — we need per-model granularity.

## Proposed Solution

### Directory Layout

Model-specific files live in subdirectories within provider directories, named after the model identifier:

```
skills/self-dev/
├── skill.toml                           # Root manifest (required)
├── system_prompt.md                     # Root prompt (level 3 fallback)
├── anthropic/
│   ├── system_prompt.md                 # All Anthropic models (level 2)
│   ├── skill.toml                       # Provider-level overrides
│   ├── claude-sonnet-4-6/
│   │   ├── system_prompt.md             # Sonnet 4.6 specific (level 1)
│   │   └── skill.toml                   # Model-level overrides (optional)
│   └── claude-opus-4/
│       └── system_prompt.md             # Opus 4 specific (level 1)
├── minimax/
│   ├── system_prompt.md                 # All MiniMax models (level 2)
│   └── MiniMax-M2.7/
│       └── system_prompt.md             # M2.7 specific (level 1)
└── openrouter/
    └── system_prompt.md                 # All OpenRouter models (level 2)
```

### Three-Level Resolution Order (Most Specific Wins)

At prompt injection time, given active provider `P` and model `M`:

1. `{provider}/{model}/system_prompt.md` → model-specific prompt
2. `{provider}/system_prompt.md` → provider-specific prompt
3. `system_prompt.md` → root fallback

Same resolution for sparse manifest overrides and timeout:

1. Model-level `skill.toml` → model-specific overrides
2. Provider-level `skill.toml` → provider-specific overrides
3. Root `skill.toml` → default values

### Model Directory Name Convention

Model directory names must match the `llm_model` config value **exactly** (case-sensitive). This is the string returned by `LlmProvider::model_name()`.

Examples:
- Anthropic: `claude-sonnet-4-6`, `claude-opus-4`
- OpenAI: `gpt-4o`, `gpt-4o-mini`
- Groq: `llama-3.3-70b-versatile`
- MiniMax: `MiniMax-M2.7` (mixed case)
- Google: `gemini-2.0-flash`

**Slash sanitization:** Some providers (notably OpenRouter) use model names containing forward slashes (e.g., `anthropic/claude-sonnet-4`). Since `/` is a filesystem path separator, these cannot be directory names directly. A deterministic `sanitize_model_dir_name()` function replaces `/` with `--` for directory names and applies the same transform at resolution time.

```
OpenRouter model: "anthropic/claude-sonnet-4"
Directory name:   "anthropic--claude-sonnet-4"
```

The sanitization function is intentionally minimal — only `/` is replaced. Other characters that are legal in filenames (dots, hyphens, mixed case) are preserved as-is for maximum readability.

## Technical Approach

### Phase 1: Data Model Changes

#### 1.1 New Fields on `SkillEntry` (`crates/mika-agent/src/skills/index.rs`)

Add two new HashMaps for model-level data, using a composite key `"{provider}/{sanitized_model}"`:

```rust
pub struct SkillEntry {
    // ... existing fields ...
    pub provider_prompts: HashMap<String, String>,
    pub provider_overrides: HashMap<String, ProviderSkillFields>,
    // NEW: model-specific variants
    /// Model-specific prompt snippet overrides.
    /// Key = "{provider}/{sanitized_model}" (e.g., "anthropic/claude-sonnet-4-6").
    /// Empty map if no model variants exist. Populated eagerly at scan time.
    pub model_prompts: HashMap<String, String>,
    /// Model-specific manifest field overrides.
    /// Key = "{provider}/{sanitized_model}", value = sparse override fields.
    /// Empty map if no model variants exist.
    pub model_overrides: HashMap<String, ProviderSkillFields>,
}
```

The composite key format uses `"provider/sanitized_model"` which is unambiguous because model names have their slashes replaced with `--` by the sanitization function.

#### 1.2 Helper Methods on `SkillEntry`

Update existing methods and add new ones:

```rust
impl SkillEntry {
    /// Effective timeout: model override > provider override > root.
    pub fn effective_timeout(&self, provider: &str, model: &str) -> u64 {
        let model_key = format!("{}/{}", provider, sanitize_model_dir_name(model));
        self.model_overrides
            .get(&model_key)
            .and_then(|o| o.timeout_secs)
            .or_else(|| {
                self.provider_overrides
                    .get(provider)
                    .and_then(|o| o.timeout_secs)
            })
            .unwrap_or(self.manifest.skill.timeout_secs)
    }

    /// Resolve the best prompt for a given provider + model combination.
    /// Three-level fallback: model-specific > provider-specific > root.
    pub fn resolve_prompt(&self, provider: &str, model: &str) -> &str {
        let model_key = format!("{}/{}", provider, sanitize_model_dir_name(model));
        if let Some(prompt) = self.model_prompts.get(&model_key) {
            return prompt;
        }
        if let Some(prompt) = self.provider_prompts.get(provider) {
            return prompt;
        }
        &self.prompt_snippet
    }

    /// Sorted set of all provider names that have any variant.
    pub fn variant_providers(&self) -> BTreeSet<&str> {
        // Unchanged — still returns provider names from provider_prompts + provider_overrides
        // Model variants are nested under their provider in display
    }

    /// Model variants for a specific provider.
    pub fn variant_models(&self, provider: &str) -> BTreeSet<&str> {
        let prefix = format!("{provider}/");
        let mut models = BTreeSet::new();
        for key in self.model_prompts.keys() {
            if let Some(model) = key.strip_prefix(&prefix) {
                models.insert(model);
            }
        }
        for key in self.model_overrides.keys() {
            if let Some(model) = key.strip_prefix(&prefix) {
                models.insert(model);
            }
        }
        models
    }

    /// Total number of distinct variant entries (providers + models).
    pub fn variant_count(&self) -> usize {
        let mut all_keys = BTreeSet::new();
        // Provider-level keys
        for key in self.provider_prompts.keys() {
            all_keys.insert(key.as_str());
        }
        for key in self.provider_overrides.keys() {
            all_keys.insert(key.as_str());
        }
        // Model-level composite keys
        for key in self.model_prompts.keys() {
            all_keys.insert(key.as_str());
        }
        for key in self.model_overrides.keys() {
            all_keys.insert(key.as_str());
        }
        all_keys.len()
    }
}
```

#### 1.3 Sanitize Function (`crates/mika-agent/src/skills/index.rs`)

```rust
/// Sanitize a model name for use as a directory name.
/// Replaces '/' with '--' to avoid filesystem path conflicts.
/// Applied at both scan time (directory discovery) and resolution time (lookup).
pub(crate) fn sanitize_model_dir_name(model: &str) -> String {
    model.replace('/', "--")
}
```

This is a public(crate) function because it's needed in both `index.rs` (scanning) and `agent.rs` (resolution, via `resolve_prompt()`/`effective_timeout()`).

### Phase 2: Scanning Changes

#### 2.1 Extend `scan_provider_variants()` (`crates/mika-agent/src/skills/index.rs`)

The existing function scans immediate subdirs of the skill directory for provider variants. Extend it to also scan subdirs within each provider directory for model variants:

```rust
fn scan_provider_variants(
    skill_dir: &Path,
    manifest: &SkillManifest,
) -> (
    HashMap<String, String>,           // provider_prompts
    HashMap<String, ProviderSkillFields>, // provider_overrides
    HashMap<String, String>,           // model_prompts (NEW)
    HashMap<String, ProviderSkillFields>, // model_overrides (NEW)
)
```

**Change the return type** from a 2-tuple to a 4-tuple (or introduce a `VariantScanResult` struct for clarity).

Inside the existing provider directory loop, after loading provider-level prompt and override, iterate subdirectories of the provider dir:

```rust
// After loading provider prompt/override...
// Scan model subdirectories within this provider directory
if let Ok(model_rd) = std::fs::read_dir(&path) {
    for model_entry in model_rd.flatten() {
        let model_path = model_entry.path();
        if !model_path.is_dir() {
            continue;
        }
        let model_name = match model_path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip dotfiles/dotdirs
        if model_name.starts_with('.') {
            continue;
        }
        let composite_key = format!("{}/{}", subdir_name, model_name);
        let mut model_has_content = false;

        // Load model-specific prompt
        let model_prompt_path = model_path.join("system_prompt.md");
        if model_prompt_path.exists() {
            let snippet = load_snippet_with_limit(&model_prompt_path, max_size);
            if !snippet.is_empty() {
                model_prompts.insert(composite_key.clone(), snippet);
                model_has_content = true;
            }
        }

        // Load model-specific skill.toml override
        let model_override_path = model_path.join("skill.toml");
        if model_override_path.exists() {
            // Same parsing as provider-level...
            model_has_content = true;
        }

        if !model_has_content {
            warn!(
                skill = %manifest.skill.name,
                provider = %subdir_name,
                model = %model_name,
                "model variant directory is empty (no system_prompt.md or skill.toml)"
            );
        }
    }
}
```

**Key behaviors:**
- Dotfiles/dotdirs (`.git/`, `.DS_Store`) are skipped inside provider directories
- Model directory names are **not** validated against any enum (they are arbitrary strings)
- The `max_prompt_size` ceiling from the root manifest applies to model prompts too
- Empty model variant directories produce a warning (same pattern as empty provider dirs)

#### 2.2 Update `scan_skills_dir()` Call Site

Update the destructuring at the `scan_provider_variants()` call site (~line 226) to capture the new return values:

```rust
let (provider_prompts, provider_overrides, model_prompts, model_overrides) =
    scan_provider_variants(&path, &manifest);
```

And pass all four maps into the `SkillEntry` constructor.

### Phase 3: Resolution Changes

#### 3.1 Update `inject_skills_and_resolve_tools()` (`crates/mika-agent/src/agent.rs`)

Add `model_name` parameter and use the new `resolve_prompt()` method:

```rust
fn inject_skills_and_resolve_tools(
    matched: &[&SkillEntry],
    tools: &ToolRegistry,
    system: &mut String,
    provider_name: &str,
    model_name: &str,  // NEW
) -> Vec<mika_common::claude::ToolDefinition> {
    // ...
    for entry in matched {
        // Three-level resolution via SkillEntry helper
        let prompt = entry.resolve_prompt(provider_name, model_name);
        // rest unchanged...
    }
}
```

#### 3.2 Update `max_skill_timeout()` (`crates/mika-agent/src/agent.rs`)

```rust
fn max_skill_timeout(matched: &[&SkillEntry], provider_name: &str, model_name: &str) -> u64 {
    matched
        .iter()
        .map(|e| e.effective_timeout(provider_name, model_name))
        .max()
        .unwrap_or(TOOL_TIMEOUT_SECS)
}
```

#### 3.3 Update All Three Call Sites

All three call sites already have access to `llm.model_name()` via the `&dyn LlmProvider` reference:

**Conversation mode** (~line 786):
```rust
let provider = llm.provider_name();
let model = llm.model_name();
let mut skill_tool_defs =
    inject_skills_and_resolve_tools(&matched, tools, &mut system, provider, model);
let skill_timeout = max_skill_timeout(&matched, provider, model);
```

**Silent mode** (~line 1479):
```rust
let provider = llm.provider_name();
let model = llm.model_name();
let skill_tool_defs = inject_skills_and_resolve_tools(&matched, tools, &mut system, provider, model);
let skill_timeout = max_skill_timeout(&matched, provider, model);
```

**Team mode** (~line 1696):
```rust
let provider = llm.provider_name();
let model = llm.model_name();
let mut skill_tool_defs =
    inject_skills_and_resolve_tools(&matched, tools, &mut system, provider, model);
let skill_timeout = max_skill_timeout(&matched, provider, model);
```

### Phase 4: CLI Display Changes

#### 4.1 Update `mika skills info <name>` (`crates/mika-cli/src/commands/skills.rs`)

Extend the provider variant display to show model variants nested under each provider:

```rust
// Show provider and model variants
let providers = entry.variant_providers();
if !providers.is_empty() || entry.variant_count() > 0 {
    println!("    Variants:    {} total", entry.variant_count());
    for provider in &providers {
        let has_prompt = entry.provider_prompts.contains_key(*provider);
        let has_override = entry.provider_overrides.contains_key(*provider);
        let parts: Vec<&str> = [
            if has_prompt { Some("prompt") } else { None },
            if has_override { Some("overrides") } else { None },
        ].into_iter().flatten().collect();
        println!("      - {} ({})", provider, parts.join(", "));

        // Show model variants under this provider
        let models = entry.variant_models(provider);
        for model in &models {
            let model_key = format!("{provider}/{model}");
            let has_model_prompt = entry.model_prompts.contains_key(&model_key);
            let has_model_override = entry.model_overrides.contains_key(&model_key);
            let model_parts: Vec<&str> = [
                if has_model_prompt { Some("prompt") } else { None },
                if has_model_override { Some("overrides") } else { None },
            ].into_iter().flatten().collect();
            println!("        └─ {} ({})", model, model_parts.join(", "));
        }
    }
}
```

**Example output:**
```
  Skill: web-search
    ...
    Variants:    4 total
      - anthropic (prompt)
        └─ claude-sonnet-4-6 (prompt)
        └─ claude-opus-4 (prompt, overrides)
      - minimax (prompt)
        └─ MiniMax-M2.7 (prompt)
```

#### 4.2 Update `mika skills list` Badge

The existing `[variants: N]` badge uses `variant_count()`, which now includes model variants. No code change needed — the count naturally reflects the new total.

#### 4.3 Update TUI `/skills` Handler (`crates/mika-cli/src/tui/commands/handlers.rs`)

The TUI handler shows a compact variant indicator. Update similarly to show model variant count if present.

### Phase 5: Validation Changes

#### 5.1 Extend `validate_skill()` (`crates/mika-agent/src/skills/index.rs`)

Inside the provider variant validation block (~line 480), after validating provider-level content, add model directory validation:

```rust
// Inside the `if subdir_name.parse::<ProviderKind>().is_ok()` block...

// Validate model subdirectories within this provider
if let Ok(model_rd) = std::fs::read_dir(&sub_path) {
    for model_entry in model_rd.flatten() {
        let model_path = model_entry.path();
        if !model_path.is_dir() {
            continue;
        }
        let model_name = match model_path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if model_name.starts_with('.') {
            continue;
        }

        let model_has_prompt = model_path.join("system_prompt.md").exists();
        let model_has_override = model_path.join("skill.toml").exists();

        if !model_has_prompt && !model_has_override {
            diags.push(SkillDiagnostic::warn(format!(
                "model variant '{subdir_name}/{model_name}/' is empty (no system_prompt.md or skill.toml)"
            )));
            continue;
        }

        // Validate model prompt size
        if model_has_prompt {
            // Same size validation as provider-level...
        }

        // Validate model override parseability and identity fields
        if model_has_override {
            // Same validation as provider-level...
        }

        // Warn if model subdir contains tools.json
        if model_path.join("tools.json").exists() {
            diags.push(SkillDiagnostic::warn(format!(
                "model '{subdir_name}/{model_name}/tools.json' is not supported — tools cannot be overridden per-model"
            )));
        }

        // Warn about unexpected nesting deeper than model level
        if let Ok(deep_rd) = std::fs::read_dir(&model_path) {
            for deep_entry in deep_rd.flatten() {
                if deep_entry.path().is_dir() {
                    let deep_name = deep_entry.file_name().to_string_lossy().to_string();
                    if !deep_name.starts_with('.') {
                        diags.push(SkillDiagnostic::warn(format!(
                            "unexpected subdirectory '{subdir_name}/{model_name}/{deep_name}/' — only two levels of nesting supported (provider/model)"
                        )));
                    }
                }
            }
        }

        diags.push(SkillDiagnostic::ok(format!(
            "model variant '{subdir_name}/{model_name}/' valid"
        )));
    }
}
```

### Phase 6: Tests

#### 6.1 Scan Tests (`crates/mika-agent/src/skills/index.rs`)

```rust
#[test]
fn test_scan_with_model_variant_prompt() {
    // Create: skill_dir/anthropic/claude-sonnet-4-6/system_prompt.md
    // Verify: model_prompts["anthropic/claude-sonnet-4-6"] is populated
}

#[test]
fn test_scan_with_model_variant_override() {
    // Create: skill_dir/openai/gpt-4o/skill.toml with timeout_secs = 120
    // Verify: model_overrides["openai/gpt-4o"].timeout_secs == Some(120)
}

#[test]
fn test_scan_model_with_slash_in_name() {
    // Create: skill_dir/openrouter/anthropic--claude-sonnet-4/system_prompt.md
    // Verify: model_prompts["openrouter/anthropic--claude-sonnet-4"] is populated
    // Verify: sanitize_model_dir_name("anthropic/claude-sonnet-4") produces the right key
}

#[test]
fn test_scan_skips_dotdirs_inside_provider() {
    // Create: skill_dir/anthropic/.git/system_prompt.md
    // Verify: model_prompts is empty (dotdirs ignored)
}

#[test]
fn test_scan_empty_model_dir_warned() {
    // Create: skill_dir/anthropic/claude-opus-4/ (empty)
    // Verify: warning logged but no panic
}

#[test]
fn test_scan_multiple_model_variants() {
    // Create variants for 2 models under anthropic + 1 under minimax
    // Verify: all 3 model variants loaded, plus provider variants
}
```

#### 6.2 Resolution Tests (`crates/mika-agent/src/agent.rs`)

```rust
#[test]
fn test_inject_skills_uses_model_prompt() {
    // Entry with model_prompts["anthropic/claude-sonnet-4-6"] = "Model-specific prompt."
    // Call with provider="anthropic", model="claude-sonnet-4-6"
    // Verify: system prompt contains "Model-specific prompt."
}

#[test]
fn test_inject_skills_falls_back_to_provider() {
    // Entry with provider_prompts["anthropic"] = "Provider prompt."
    // No model variant for "claude-opus-4"
    // Call with provider="anthropic", model="claude-opus-4"
    // Verify: system prompt contains "Provider prompt."
}

#[test]
fn test_inject_skills_falls_back_to_root() {
    // Entry with prompt_snippet = "Root prompt."
    // No provider or model variant for "groq"/"llama-3.3-70b-versatile"
    // Verify: system prompt contains "Root prompt."
}

#[test]
fn test_inject_skills_model_with_slash() {
    // Entry with model_prompts["openrouter/anthropic--claude-sonnet-4"] = "OpenRouter model prompt."
    // Call with provider="openrouter", model="anthropic/claude-sonnet-4"
    // Verify: sanitization matches and model prompt is used
}
```

#### 6.3 Effective Timeout Tests

```rust
#[test]
fn test_effective_timeout_model_override() {
    // model_overrides["anthropic/claude-sonnet-4-6"].timeout_secs = Some(120)
    // provider_overrides["anthropic"].timeout_secs = Some(90)
    // root timeout = 30
    // effective_timeout("anthropic", "claude-sonnet-4-6") == 120
}

#[test]
fn test_effective_timeout_falls_back_to_provider() {
    // No model override for "claude-opus-4"
    // provider_overrides["anthropic"].timeout_secs = Some(90)
    // effective_timeout("anthropic", "claude-opus-4") == 90
}

#[test]
fn test_effective_timeout_falls_back_to_root() {
    // No model or provider override for "groq"/"llama"
    // effective_timeout("groq", "llama") == root timeout
}
```

#### 6.4 Variant Count/Display Tests

```rust
#[test]
fn test_variant_count_includes_models() {
    // 2 provider variants + 3 model variants = 5 total
}

#[test]
fn test_variant_models_for_provider() {
    // model_prompts["anthropic/claude-sonnet-4-6"] and model_overrides["anthropic/claude-opus-4"]
    // variant_models("anthropic") returns {"claude-opus-4", "claude-sonnet-4-6"}
}
```

#### 6.5 Validation Tests

```rust
#[test]
fn test_validate_model_variant_valid() {
    // Create provider + model dir with valid content
    // Verify: OK diagnostics for model variant
}

#[test]
fn test_validate_model_variant_tools_json_warn() {
    // Create tools.json in model dir
    // Verify: warning diagnostic
}

#[test]
fn test_validate_model_variant_empty_warn() {
    // Create empty model dir
    // Verify: warning diagnostic
}

#[test]
fn test_validate_model_variant_deep_nesting_warn() {
    // Create subdir inside model dir
    // Verify: warning about max nesting depth
}
```

## System-Wide Impact

### Interaction Graph

1. `scan_skills_dir()` → `scan_provider_variants()` (extended) → `SkillEntry` with 4 maps
2. `run_agent_loop()` → `inject_skills_and_resolve_tools()` (new signature) → `SkillEntry::resolve_prompt()` (new method)
3. `max_skill_timeout()` → `SkillEntry::effective_timeout()` (new signature)
4. `mika skills info` → `SkillEntry::variant_models()` (new method) → nested display

No cross-crate changes needed — `ProviderSkillFields` is reused at model level.

### Error Propagation

Scanning errors (unreadable directories, malformed TOML) are handled identically to provider-level: `warn!` and skip. No new error types needed.

### State Lifecycle Risks

None. Variant data is eagerly loaded at startup into `SkillEntry` and is immutable for the process lifetime. Model switching via `/model` or `--model` changes only the lookup key at resolution time.

### API Surface Parity

- `inject_skills_and_resolve_tools()` — internal function, not API surface
- `effective_timeout()` — internal method on `SkillEntry`
- CLI output format changes are additive (more info shown)
- No HTTP API changes
- No database changes

## Acceptance Criteria

- [ ] `scan_provider_variants()` scans subdirectories within provider directories as model variants
- [ ] `SkillEntry` carries `model_prompts` and `model_overrides` HashMaps
- [ ] `resolve_prompt(provider, model)` implements three-level fallback: model → provider → root
- [ ] `effective_timeout(provider, model)` implements three-level fallback for timeouts
- [ ] `inject_skills_and_resolve_tools()` takes `model_name` parameter and uses `resolve_prompt()`
- [ ] All three call sites (conversation, silent, team) pass `llm.model_name()` to the updated functions
- [ ] `sanitize_model_dir_name()` replaces `/` with `--` for filesystem-safe model directory names
- [ ] Dotfiles/dotdirs inside provider directories are skipped (`.git/`, `.DS_Store`)
- [ ] Model directory names match `llm_model` config value exactly (case-sensitive)
- [ ] Existing per-provider variants continue to work unchanged (backward compatible)
- [ ] `mika skills info <name>` shows model variants nested under providers with tree formatting
- [ ] `variant_count()` includes both provider and model variants in the total
- [ ] `validate_skill()` validates model directories (empty check, tools.json warning, identity field warning, max nesting depth)
- [ ] Tests cover: three-level prompt fallback, three-level timeout fallback, slash sanitization, dotdir skipping, empty model dirs, multiple model variants, validation diagnostics
- [ ] `cargo test` passes
- [ ] `cargo clippy` clean

## Dependencies & Risks

### Dependencies

- **None.** All prerequisite work (per-provider variants from #241) is merged. The `LlmProvider` trait already exposes `model_name()`. No schema changes needed.

### Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Model names with `/` in them (OpenRouter) | High — directory name collision | `sanitize_model_dir_name()` replaces `/` with `--`, applied deterministically at scan and resolution time |
| Case sensitivity across platforms | Medium — "works on my machine" between macOS (case-insensitive FS) and Linux (case-sensitive) | Document case-sensitive matching. Could add a validation warning for case mismatches in future |
| Increased scan time with many model variants | Low — scanning is startup-only | Bounded by filesystem iteration speed; model dirs are small and few |
| Breaking `effective_timeout()` signature | Low — all call sites are in `agent.rs` | Three call sites, all updated together |

## Open Questions

1. **Should bundled skills support model variants?** Currently, the `skill!` macro embeds files at compile time. Model variants would require additional `include_str!` entries. **Recommendation:** Defer — no bundled skill currently needs model-level variants. The infrastructure supports it via `seed_bundled_skills()` if needed later.

2. **Should `mika skills list` show which variant is "active"?** This requires threading the current provider/model into the CLI display function. **Recommendation:** Defer to a follow-up. The current `[variants: N]` badge and `mika skills info` detail view are sufficient for v1.

3. **Should there be a `--verbose` flag on `mika skills list`?** The issue mentions it but the flag doesn't exist. **Recommendation:** Use existing `mika skills info <name>` for detailed variant display. The badge on `list` shows enough.

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/index.rs` | `SkillEntry` fields, `sanitize_model_dir_name()`, `scan_provider_variants()` return type and model scanning, `effective_timeout()` signature, `resolve_prompt()` new method, `variant_models()` new method, `variant_count()` update, `validate_skill()` model dir validation, tests |
| `crates/mika-agent/src/agent.rs` | `inject_skills_and_resolve_tools()` signature + `resolve_prompt()` usage, `max_skill_timeout()` signature, 3 call sites, test helpers + new tests |
| `crates/mika-cli/src/commands/skills.rs` | `show_skill_detail()` — nested model variant display under providers |
| `crates/mika-cli/src/tui/commands/handlers.rs` | `/skills` detail view — model variant count/names |

## Documentation Updates

- Update `docs/skills.md` with model variant directory convention and examples
- Update `CLAUDE.md` skill variant section to mention model granularity
- Solution doc: `docs/solutions/architecture-patterns/per-provider-skill-variant-directories.md` — extend with model-level section

## Sources & References

- Issue: [#246](https://github.com/senara-solutions/mika/issues/246) — Extend skill variant directories to support provider + model granularity
- Prerequisite: [#241](https://github.com/senara-solutions/mika/issues/241) — Per-provider skill variant directories (merged)
- Previous plan: `docs/plans/2026-03-22-002-feat-per-provider-skill-variants-plan.md`
- Solution doc: `docs/solutions/architecture-patterns/per-provider-skill-variant-directories.md`
- ADR: `docs/adr/002-filesystem-skill-registry.md` — eagerly-loaded skill scanning convention
