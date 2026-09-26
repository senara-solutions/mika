---
status: complete
priority: p3
issue_id: 535
tags: [code-review, architecture, teams]
dependencies: []
---

# Checkpoint Serialization Lacks Version Envelope

## Problem Statement

The checkpoint is `serde_json::to_string(&self.run)` stored as plain JSON. If any serialized types change shape (fields added/removed, enum variants reordered), deserialization of existing checkpoints will fail. Team runs can be suspended for days with 90-day timeout maximums.

**Severity:** P3 — Deployment hazard for future schema changes.

## Findings

- `crates/mika-agent/src/teams/engine.rs` — serialization with no version marker
- `crates/mika-agent/src/teams/mod.rs` — deserialization with no migration path
- Types lack `#[serde(default)]` on fields

## Proposed Solutions

1. **Wrap in version envelope**: `{"version": 1, "data": {...}}`
   - On deserialize, match on version and apply transforms
   - Effort: Small
   - Risk: Low

2. **Add `#[serde(default)]` to all serialized fields**
   - New fields degrade gracefully
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] Checkpoint JSON includes version marker
- [ ] Deserialization handles version mismatch gracefully
