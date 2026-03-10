---
status: complete
priority: p3
issue_id: 604
tags: [code-review, security, ux]
dependencies: []
---

# API key prompt shows plaintext (no echo suppression)

## Problem Statement

The `mika setup` API key prompt uses `stdin().read_line()` which displays the key as the user types. This exposes the secret to shoulder surfing and screen recording.

## Proposed Solutions

Use `rpassword::read_password()` or `dialoguer` (already a dependency) with password input mode.

- Effort: Small
- Risk: Low

## Acceptance Criteria

- [x] API key input is not echoed to terminal
