---
status: complete
priority: p3
issue_id: 378
tags: [code-review, simplicity, config]
dependencies: []
---

# Remove default-restating lines from release-plz.toml

## Problem Statement

Several lines in `release-plz.toml` restate the tool's defaults, adding visual noise without changing behavior. Removing them makes the config file more focused on intentional customizations.

## Findings

- **Source:** Code Simplicity Reviewer agent
- **Lines that restate defaults:**
  - `changelog_update = true` (default)
  - `dependencies_update = true` (default)
  - `allow_dirty = false` (default)
  - `features_always_increment_minor = false` (default)
  - `semver_check = false` — this one may be intentional (default is true), keep it

## Proposed Solutions

### Option 1: Remove redundant defaults, keep intentional overrides (Recommended)
- Remove lines that match release-plz defaults
- Keep `semver_check = false` since it overrides the default (true)
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `release-plz.toml`

## Acceptance Criteria

- [ ] Default-restating lines removed from release-plz.toml
- [ ] `semver_check = false` retained (intentional override)
- [ ] Config file only contains intentional customizations
