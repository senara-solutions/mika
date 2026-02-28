---
status: complete
priority: p1
issue_id: 346
tags: [code-review, security, shell-script]
dependencies: [345]
---

# read.sh Input Parsing Uses grep Regex Instead of jq

## Problem Statement

`read.sh` line 7 parses the input JSON using grep regex:
```sh
PATH_VALUE=$(echo "$INPUT" | grep -o '"path":"[^"]*"' | head -1 | cut -d'"' -f4)
```

This is fragile with escaped characters. Since `jq` is already a dependency (used for output on line 24), the input parsing should also use `jq` for safety and consistency.

## Findings

- **Source:** security-sentinel
- **Location:** `templates/skills/file-reader/handlers/read.sh:7`
- **Evidence:** grep-based JSON parsing is fragile with escaped quotes

## Proposed Solutions

### Option A: Use jq for input parsing (Recommended)
Replace grep with: `PATH_VALUE=$(echo "$INPUT" | jq -r '.path // empty')`
- Pros: Safe, consistent with output construction
- Cons: None (jq already a dependency)
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [x] `read.sh` uses `jq` for input JSON parsing
- [x] Handles paths with special characters correctly

## Work Log

| Date | Action | Result |
|------|--------|--------|
| 2026-02-28 | Identified during code review | Pending |
| 2026-02-28 | Replaced grep-based parsing with jq | Complete |
