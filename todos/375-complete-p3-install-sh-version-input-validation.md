---
status: complete
priority: p3
issue_id: 375
tags: [code-review, security, installer]
dependencies: []
---

# Add input validation to install.sh VERSION parameter

## Problem Statement

The `install.sh` script accepts a VERSION parameter that is interpolated directly into a GitHub API URL without validation. While the risk is limited (curl to a nonexistent URL would just 404), sanitizing input is defensive best practice.

## Findings

- **Source:** Security Sentinel review agent
- **Severity:** LOW — limited practical risk since VERSION goes into a URL path
- **Current behavior:** `VERSION` is used as `https://api.github.com/repos/.../releases/tags/v${VERSION}`

## Proposed Solutions

### Option 1: Add regex validation for VERSION (Recommended)
- Validate VERSION matches semver pattern: `^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$`
- Exit with error if invalid
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `install.sh`

## Acceptance Criteria

- [ ] VERSION parameter validated against semver pattern before use
- [ ] Invalid VERSION values produce a clear error message
