---
title: "Adding a new get_documentation topic for agent self-service"
date: "2026-03-16"
module: "skills"
severity: "low"
tags:
  - "get-documentation"
  - "builtin-handlers"
  - "build-rs"
  - "docs"
related_files:
  - "crates/mika-agent/build.rs"
  - "crates/mika-agent/src/skills/builtin_handlers.rs"
  - "crates/mika-agent/templates/skills/self-knowledge/system_prompt.md"
---

# Adding a New get_documentation Topic

## Problem Statement

When adding a new documentation file to `docs/` that the agent should be able to access at runtime via the `get_documentation` tool, four files must be updated in sync. Missing any one causes a build failure or silent unavailability.

## Root Cause

The `get_documentation` builtin handler uses compile-time `include_str!` to embed doc files from `OUT_DIR`. The `build.rs` copies docs from `docs/` into `OUT_DIR`. Both must be updated, plus the self-knowledge prompt must list the new topic, and tests must cover it.

## Solution

### Step 1: Add to build.rs DOCS array

```rust
// crates/mika-agent/build.rs
const DOCS: &[&str] = &[
    "architecture.md",
    "browser-control.md",  // ← new entry (alphabetical)
    "configuration.md",
    // ...
];
```

### Step 2: Add include_str! and topic match in builtin_handlers.rs

```rust
// crates/mika-agent/src/skills/builtin_handlers.rs

// Static declaration (alphabetical with others):
static DOC_BROWSER_CONTROL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/docs/browser-control.md"));

// Match arm in get_documentation():
"browser-control" => {
    ToolOutput::success(strip_frontmatter(DOC_BROWSER_CONTROL).to_string())
}

// Update the error message to list the new topic:
_ => ToolOutput::error(
    "Invalid topic. Use one of: architecture, api-spec, browser-control, ..."
)
```

### Step 3: Update self-knowledge system_prompt.md

Add the new topic to the "Available topics" list:

```markdown
- `browser-control` — browser automation setup via Playwright MCP, usage patterns, and security
```

### Step 4: Update the test

Add the new topic to `test_get_documentation_all_embedded_topics`:

```rust
for topic in &[
    "architecture",
    "api-spec",
    "browser-control",  // ← new
    // ...
]
```

### Step 5: Create crate-local fallback

Copy `docs/browser-control.md` to `crates/mika-agent/docs/browser-control.md` for crates.io publishing. Run `scripts/sync-agent-docs.sh` before `cargo publish`.

## Prevention Checklist

When adding a new `get_documentation` topic:

- [ ] Create `docs/{topic}.md` with YAML frontmatter (title, description)
- [ ] Add to `build.rs` DOCS array (alphabetical)
- [ ] Add `static DOC_*` include_str! in `builtin_handlers.rs`
- [ ] Add match arm in `get_documentation()` function
- [ ] Update error message invalid-topic list
- [ ] Update `self-knowledge/system_prompt.md` topics list
- [ ] Add to `test_get_documentation_all_embedded_topics` test
- [ ] Copy to `crates/mika-agent/docs/` (crate-local fallback)
- [ ] `cargo build -p mika-agent` (verifies build.rs + include_str!)
- [ ] `cargo test -p mika-agent -- get_documentation` (verifies all topics)

## Related Documentation

- [Adding a Prompt-Only Built-In Skill](adding-prompt-only-bundled-skill.md)
- [MCP Self-Knowledge Gaps](mcp-self-knowledge-command-hallucination.md)
