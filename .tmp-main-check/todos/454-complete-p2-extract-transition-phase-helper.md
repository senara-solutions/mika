---
status: complete
priority: p2
issue_id: "454"
tags: [code-review, quality, team-engine]
dependencies: []
---

# Extract transition_phase Helper in TeamEngine

## Problem Statement

Every phase transition in `TeamEngine::execute()` follows the same 5-line pattern (info log + emit PhaseChanged event), repeated 5 times for ~25 lines of duplication.

## Findings

- **Source**: Code simplicity reviewer
- **Location**: `crates/mika-agent/src/teams/engine.rs` (5 occurrences of phase transition pattern)

## Proposed Fix

Extract a `transition_phase(&mut self, phase: TeamPhase)` method:

```rust
fn transition_phase(&mut self, phase: TeamPhase) {
    info!(phase = %phase, iteration = self.run.iteration, "team_phase");
    self.emit_event(TeamEvent::PhaseChanged { phase, iteration: self.run.iteration });
}
```

## Acceptance Criteria

- [ ] Phase transition logic exists in one method
- [ ] All 5 call sites use the helper
- [ ] Tests pass
