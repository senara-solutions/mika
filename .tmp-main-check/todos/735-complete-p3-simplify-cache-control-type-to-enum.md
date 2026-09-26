---
status: complete
priority: p3
issue_id: "735"
tags: [code-review, quality, simplification]
---

# Simplify CacheControl and SystemContentBlock types

## Problem Statement

`CacheControl.kind` is a `String` but only ever holds `"ephemeral"`. Similarly, `SystemContentBlock.block_type` is a `String` but only ever holds `"text"`. Using String fields for fixed values adds unnecessary allocation and makes invalid states representable.

## Findings

- `CacheControl { kind: String }` could be a unit struct with a custom Serialize impl, or an enum with a single variant
- `SystemContentBlock.block_type` is always `"text"` — the type field could use `#[serde(rename)]` on a unit variant instead of a runtime string
- Current approach works correctly but leaves room for runtime typos (e.g., `"ephmeral"`)

## Proposed Solutions

### Option A: Use enums with serde rename
```rust
#[derive(Serialize)]
#[serde(tag = "type")]
enum CacheControl {
    #[serde(rename = "ephemeral")]
    Ephemeral,
}
```
**Pros:** Zero-alloc, impossible to construct invalid values, idiomatic Rust
**Cons:** More code lines, slightly different API

### Option B: Keep as-is (current)
**Pros:** Simpler code, flexible for future cache types
**Cons:** Runtime string allocation, typo-prone

## Technical Details

- **Affected files:** `crates/mika-common/src/claude.rs`
- **Effort:** Small
- **Risk:** Low — pure refactor, no behavioral change

## Acceptance Criteria

- [ ] `CacheControl` uses enum or const instead of String
- [ ] `SystemContentBlock` uses tagged enum instead of String block_type
- [ ] All existing tests pass
- [ ] Serialized JSON output unchanged
