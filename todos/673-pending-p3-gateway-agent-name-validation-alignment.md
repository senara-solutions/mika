---
status: pending
priority: p3
issue_id: "673"
tags: [code-review, security, gateway]
dependencies: []
---

# Align Gateway agent_name Validation with mika-common

## Problem Statement

Gateway `handle_send` at `routes.rs:634-646` validates `agent_name` with `[a-zA-Z0-9_-]` max 64 chars. The canonical `validate_agent_name` in `mika-common` enforces `[a-z0-9-]` only (lowercase, no underscores), max 32 chars, no leading/trailing/consecutive hyphens. The gateway is more permissive.

## Findings

- Security sentinel flagged as low-severity social engineering vector (e.g. `[ADMIN]` prefix)
- Requires internal token access, so exploitability is very low
- Gateway can't easily import mika-common (different crate dependency tree)

## Proposed Solutions

- Duplicate the mika-common validation rules in the gateway handler
- Or accept the discrepancy since agents are created via `validate_agent_name` anyway

## Technical Details

- **Affected files:** `crates/mika-gateway/src/routes.rs:634-646`
- **Effort:** Small
