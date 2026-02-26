---
title: "Self-knowledge skill code review: 10 findings resolution"
date: 2026-02-26
category: code-review-workflow
severity: medium
components:
  - crates/mika-agent/src/skills/builtin_handlers.rs
  - crates/mika-agent/src/skills/index.rs
  - crates/mika-agent/src/server/openapi.rs
  - crates/mika-gateway/src/openapi.rs
  - crates/mika-gateway/src/main.rs
  - templates/skills/self-knowledge/tools.json
  - templates/skills/self-knowledge/system_prompt.md
  - scripts/generate-openapi.sh
tags:
  - skills-system
  - self-knowledge
  - code-review
  - YAGNI
  - error-handling
  - drift-detection
  - openapi
  - builtin-handlers
symptoms: "YAGNI violation with search_docs tool, inconsistent output truncation between builtin and exec/http handlers, no drift detection for OpenAPI specs, swallowed IO errors, missing validation for builtin function names at load time, error suppression in generate-openapi.sh"
root_cause: "New builtin handler dispatch path missed safeguards established in exec/http handlers; search_docs over-engineered for small documents; OpenAPI generation workflow lacked freshness enforcement"
resolution: "Removed search_docs (YAGNI), added output truncation to builtin handlers, added OpenAPI drift-detection tests, logged IO errors, validated builtin names at load time, removed 2>/dev/null from scripts"
time_to_resolve: "~1 hour"
---

# Self-Knowledge Skill Code Review: 10 Findings Resolution

## Context

Multi-agent code review of the self-knowledge skill implementation (20 modified + 7 new files). Five specialized review agents analyzed security, architecture, patterns, performance, and simplicity. This document captures the synthesized findings and applied fixes.

## Solution

### P1 Findings (Fixed Before Commit)

**1. Removed `search_docs` tool (YAGNI)**

The three embedded documents total ~13.6KB combined. The LLM can read full documents via `get_architecture_overview` (5.5KB) or `get_api_spec` (4KB) and extract what it needs. `search_docs` returned stripped-context line matches that were worse for LLM comprehension than full documents.

Removed from: `builtin_handlers.rs` (function + 3 tests + match arm), `tools.json`, `system_prompt.md`. Tool surface reduced from 4 to 3.

**2. Applied output truncation to builtin handler results**

The `executor.rs` applied `MAX_OUTPUT_LEN` (10,000 chars) truncation to exec/http handlers. Builtin handlers bypassed the executor entirely and sent raw output to Claude with no size cap.

```rust
const MAX_OUTPUT_LEN: usize = 10_000;

fn truncate_output(output: &mut ToolOutput) {
    if output.content.len() > MAX_OUTPUT_LEN {
        output.content.truncate(MAX_OUTPUT_LEN);
        output.content.push_str("\n... (truncated at 10000 chars)");
    }
}
```

Applied in `execute()` after every handler returns, ensuring consistent limits regardless of handler type.

**3. Added OpenAPI spec drift-detection tests**

Committed YAML files can drift from utoipa annotations if a developer changes handler annotations but forgets to run `generate-openapi.sh`. Added tests in both agent and gateway `openapi.rs`:

```rust
#[test]
fn test_committed_spec_is_current() {
    let generated = agent_openapi_yaml();
    let committed = include_str!("../../../../docs/openapi/mika-server.yaml");
    assert_eq!(
        generated, committed,
        "Agent OpenAPI spec is out of date. Run ./scripts/generate-openapi.sh to update."
    );
}
```

### P2 Findings (Fixed)

**4. Changed `#[allow(dead_code)] mod openapi` to `pub mod openapi`** in `mika-gateway/src/main.rs`. Follows the same pattern as the mika-agent crate.

**5. Logged IO error in `get_cli_reference`** instead of swallowing it with `Err(_)`. Added `tracing::warn!(error = %e, path = %path.display(), "failed to read CLI reference")`.

**6. Validated builtin function names at skill loading time.** Added `KNOWN_BUILTINS` constant and validation filter in `load_tools_json()` that skips tools with unknown builtin function names:

```rust
pub const KNOWN_BUILTINS: &[&str] = &[
    "get_cli_reference", "get_api_spec", "get_architecture_overview",
];
```

A typo in `tools.json` (e.g., `"get_clii_reference"`) is now caught at startup with a warning, not at runtime.

**7. Removed `2>/dev/null` from `generate-openapi.sh`.** The script already uses `set -euo pipefail`, so compilation errors now fail loudly instead of being silently swallowed.

### P3 Findings

**8. Symlink check on CLI reference write** — Documented as accepted risk given directory permissions model (`0o700`).

**9. Added `execute()` dispatch test** — `test_execute_unknown_function_returns_error` verifies unknown function names return errors. Also added truncation unit tests.

**10. Token budget impact of always_on tools** — Documented for monitoring. 4 always_on tool definitions add ~200 tokens per API call.

## Root Cause Analysis

Three themes:

1. **New code path, no precedent.** Builtin handlers were a new dispatch mechanism. The existing exec/http handlers had accumulated safeguards over time, but the builtin path was built from scratch and missed the same guards.

2. **Over-engineering a search tool.** `search_docs` was built assuming the LLM would need search capability for large documents. In practice, the LLM handles complete documents well, and substring search on YAML/markdown produces misleading fragments.

3. **Manual workflow without CI enforcement.** OpenAPI spec generation required running a script then committing output. The `2>/dev/null` suppression compounded this by hiding failures. Drift-detection tests now make `cargo test` the enforcement point.

## Prevention

### Code Review Checklist

- [ ] Justify every new tool against existing capabilities — can an existing tool handle this?
- [ ] Verify output truncation applies uniformly across all handler types
- [ ] Audit every `#[allow(...)]` annotation — add inline justification comments
- [ ] Confirm all error paths are logged or propagated, never silently swallowed
- [ ] Validate string-based dispatch identifiers at load time, not first invocation
- [ ] Check shell scripts for unconditional stderr suppression (`2>/dev/null`)
- [ ] Document token budget impact for always_on or system-prompt-injected content
- [ ] Verify generated artifacts have freshness mechanisms (drift-detection tests)

### Key Patterns Established

**Centralized output post-processing:** Apply truncation/sanitization once at the dispatch level after handlers return, not inside each handler variant. This prevents gaps when new variants are added.

**Load-time validation with descriptive errors:** Map string identifiers to handlers eagerly during initialization. Include available options in the error message for quick diagnosis.

**Explicit error logging with fallback:** When a fallback value is acceptable, still log the error with context (path, error type) so operators can diagnose issues.

## Related Documentation

- [Agent API self-knowledge](../logic-errors/agent-api-self-knowledge-and-skill-origin-awareness.md) — API channel self-knowledge with origin tagging
- [Agent CLI self-knowledge](../logic-errors/agent-cli-self-knowledge-and-skill-triggers.md) — CLI channel self-knowledge with drift-detection tests
- [Filesystem skill registry](../architecture-decisions/filesystem-skill-registry-implementation.md) — Runtime skill loading architecture
- [Skills system reference](../../skills.md) — Complete skills system guide
- [Architecture overview](../../architecture/OVERVIEW.md) — System design overview

## Verification

After applying fixes:
1. `cargo build` — all crates compile
2. `cargo test` — all tests pass (7 new tests added)
3. `cargo clippy` — no new warnings
4. `./scripts/generate-openapi.sh` — completes without error
5. Self-knowledge skill has 3 tools (not 4) after `search_docs` removal
