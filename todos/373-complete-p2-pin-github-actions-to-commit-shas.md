---
status: complete
priority: p2
issue_id: 373
tags: [code-review, security, ci-cd, supply-chain]
dependencies: []
---

# Pin GitHub Actions to commit SHAs instead of version tags

## Problem Statement

All GitHub Actions in the three workflow files (ci.yml, release-plz.yml, release.yml) reference actions by mutable version tags (e.g., `actions/checkout@v4`, `dtolnay/rust-toolchain@stable`). A compromised upstream action tag could be force-pushed with malicious code. Pinning to full commit SHAs prevents supply-chain attacks via tag mutation.

## Findings

- **Source:** Security Sentinel review agent
- **Severity:** MEDIUM — supply-chain risk from mutable action tags
- **Files affected:**
  - `.github/workflows/ci.yml` — 3 actions (checkout, rust-toolchain, rust-cache)
  - `.github/workflows/release-plz.yml` — 4 actions (checkout×2, rust-toolchain×2, release-plz×2)
  - `.github/workflows/release.yml` — 4 actions (checkout, rust-toolchain, rust-cache, upload-rust-binary, setup-cross-toolchain)
- **Industry standard:** GitHub's own security hardening guide recommends SHA pinning for third-party actions

## Proposed Solutions

### Option 1: Pin all actions to commit SHAs with version comments (Recommended)
- Replace `uses: actions/checkout@v4` with `uses: actions/checkout@<full-sha>  # v4`
- Use Dependabot or Renovate to keep SHA pins updated
- **Effort:** Small
- **Risk:** Low — version comment preserves readability

### Option 2: Pin only third-party actions, keep first-party (actions/*) on tags
- GitHub-owned actions (actions/checkout, actions/cache) are lower risk
- Pin third-party: dtolnay, Swatinem, taiki-e, release-plz
- **Effort:** Small
- **Risk:** Low — pragmatic middle ground

## Technical Details

- **Affected files:** `.github/workflows/ci.yml`, `.github/workflows/release-plz.yml`, `.github/workflows/release.yml`

## Acceptance Criteria

- [ ] All third-party GitHub Actions are pinned to full commit SHAs
- [ ] Version comments are added next to each SHA for readability
- [ ] Dependabot or Renovate config added for automated SHA updates
