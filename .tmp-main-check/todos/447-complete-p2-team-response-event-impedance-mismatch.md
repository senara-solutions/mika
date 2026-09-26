---
status: complete
priority: p2
issue_id: "447"
tags: [code-review, architecture]
dependencies: []
---

# TeamResponse/TeamEvent Impedance Mismatch

## Problem Statement

`TeamResponse` (4 variants) is a lossy projection of `TeamEvent` (7 variants). `Deliverable` and `RunFailed` are silently dropped in the callback. `CriticReview` loses structured data.

## Fix

Send `TeamEvent` directly over the mpsc channel. Have `tick_team_mode()` handle all variants. Eliminates the translation layer, silent drops, and information loss.

## Acceptance Criteria

- [ ] `TeamResponse` removed
- [ ] Channel uses `TeamEvent` directly
- [ ] `tick_team_mode()` handles all `TeamEvent` variants
- [ ] Worker sends run completion via `TeamEvent::Deliverable` / `TeamEvent::RunFailed`
