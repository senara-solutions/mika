---
status: complete
priority: p2
issue_id: 349
tags: [code-review, security, toctou]
dependencies: []
---

# TOCTOU Race in `read_and_validate_image()`

## Problem Statement

There is a Time-of-Check-Time-of-Use race between metadata validation and the actual file read in `read_and_validate_image()`. The sequence `fs::canonicalize()` -> `fs::metadata()` (size/type check) -> `fs::read()` creates a window where the file could be swapped between the metadata check and the read, bypassing the 5MB size limit or the regular-file check.

## Findings

- **Source:** security-sentinel
- **Location:** `crates/mika-agent/src/skills/executor.rs:113-152`
- **Evidence:** Separate `metadata()` and `read()` calls with no atomic guarantee
- **Exploitability:** Low in typical deployment (single-user container), but relevant if skills can be installed from untrusted sources

## Proposed Solutions

### Option A: Use `File::open` + `take()` + `read_to_end()` (Recommended)
Open the file once, use `take(MAX_IMAGE_SIZE + 1)` to cap the actual bytes read regardless of what happened between metadata check and read. Re-check byte count after reading.
- Effort: Small
- Risk: Low

```rust
let mut file = fs::File::open(&canonical)?;
let mut bytes = Vec::with_capacity(metadata.len() as usize);
use std::io::Read;
file.take(MAX_IMAGE_SIZE + 1).read_to_end(&mut bytes)?;
if bytes.len() as u64 > MAX_IMAGE_SIZE {
    return Err("image too large".into());
}
```

### Option B: Keep current approach with documentation
Document that the TOCTOU window is accepted risk given container isolation.
- Effort: Trivial
- Risk: None (no code change)

## Acceptance Criteria

- [ ] File read is capped at `MAX_IMAGE_SIZE + 1` bytes regardless of metadata
- [ ] Post-read size validation catches files that grew between metadata and read
- [ ] Existing tests still pass

## Work Log

| Date | Action | Result |
|------|--------|--------|
| 2026-02-28 | Identified during code review | Pending |
| 2026-02-28 | Fixed: use File::open + take() + read_to_end() for capped reads | Complete |
