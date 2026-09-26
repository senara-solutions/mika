---
status: complete
priority: p3
issue_id: "148"
tags: [plan-review, simplicity]
dependencies: []
---

# Consolidate gateway file structure from 7 to 4 files

## Problem Statement
The plan specifies 7 source files (main.rs, config.rs, routes.rs, telegram.rs, routing.rs, pairing.rs, db.rs) for a ~500-700 line crate. This is over-modularized — pairing is one function, routing is ~50 lines, and telegram.rs has minimal logic. Fewer files reduce cognitive overhead and import chains.

**Why it matters:** Over-modularization in a small crate creates unnecessary file-jumping and import complexity. For 500-700 lines total, 4 files is the right granularity.

## Findings
- Source: Code Simplicity Reviewer
- pairing.rs: Single function (~20 lines) → merge into routes.rs
- routing.rs: Customer lookup + forward (~50 lines) → merge into routes.rs
- telegram.rs: Type parsing + sendMessage (~80 lines) → keep if substantial
- Net savings: ~15-20 LOC of mod/use/import boilerplate

## Proposed Solutions

### Option 1: Consolidate to 4 files (Recommended)
```
src/
├── main.rs      # Entry point, startup
├── config.rs    # Settings
├── routes.rs    # All handlers: webhook, send, health, pairing, routing logic
└── telegram.rs  # Telegram types + API client (if substantial)
```
If telegram.rs ends up < 50 lines, merge into routes.rs for 3 files total.
- **Pros**: Less file-jumping, simpler imports, right-sized for crate size
- **Cons**: Larger individual files (but still < 300 lines each)
- **Effort**: Small
- **Risk**: Low

## Acceptance Criteria
- [ ] Gateway crate has 4 or fewer source files
- [ ] No file under 50 lines (merge up if so)
- [ ] All functionality preserved

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Code Simplicity Reviewer flagged over-modularization
