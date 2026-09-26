---
title: Bump rand to 0.9.3 to clear RUSTSEC-2026-0097
date: 2026-04-11
issue: 539
tags: [dependency, security, cargo-deny, rand]
---

# Bump rand to 0.9.3 to clear RUSTSEC-2026-0097

## Problem

CI security job (`cargo deny check advisories`) flagged RUSTSEC-2026-0097 affecting `rand >= 0.7, < 0.9.3`. Our lockfile had `rand 0.9.2`. This blocked the Sprint 2026-04-11 pipeline.

## Solution

1. `cargo update -p rand@0.9.2 --precise 0.9.3` — patch-level bump within semver range
2. Removed two stale `deny.toml` exemptions that were no longer needed:
   - RUSTSEC-2023-0071 (rsa Marvin Attack) — `rsa` crate no longer in dependency tree
   - RUSTSEC-2026-0002 (lru IterMut unsoundness) — `cargo deny` no longer flags it (advisory may have been narrowed or the affected version range updated)

## Verification

- `cargo deny check advisories` → clean pass
- `cargo check` → compiles
- `cargo test` → all tests pass (no behavioral changes from patch bump)

## Lessons

- **Periodically audit `deny.toml` exemptions.** Exemptions can become stale as transitive dependencies change. When bumping dependencies for security fixes, also check if existing exemptions can be removed.
- **Use `cargo update -p <crate> --precise <version>`** for targeted lockfile updates without touching unrelated deps.
- **`cargo tree -i <crate>`** is the fastest way to verify whether a crate is still in the dependency tree before removing an exemption.
