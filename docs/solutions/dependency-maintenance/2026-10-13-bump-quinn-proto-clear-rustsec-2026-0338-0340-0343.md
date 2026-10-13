---
title: Bump quinn-proto to 0.11.19 to clear RUSTSEC-2026-0338, -0340 and -0343
date: 2026-10-13
tags: [dependency, security, cargo-audit, quinn-proto, lockfile]
---

# Bump quinn-proto to 0.11.19 to clear RUSTSEC-2026-0338, -0340 and -0343

## Problem

The Security job (`cargo audit`) flagged `quinn-proto 0.11.17` with three advisories, and the job failed on every open PR whatever the PR changed.

No build target in the workspace activates `quinn-proto`: `cargo tree -i quinn-proto --target all` prints nothing. It is an optional transitive dependency that appears only in `Cargo.lock`. `cargo audit` reads the lockfile, not the build graph, so it reports the advisory anyway.

## Solution

```bash
cargo update -p quinn-proto --precise 0.11.19
```

## The second advisory behind the first

The Security job runs `cargo audit` first and then `cargo deny check`. While `cargo audit` was failing on quinn-proto, `cargo deny` never ran, so its own failure stayed hidden: `unsafe-libyaml` 0.2.11 is flagged unmaintained (RUSTSEC-2024-0449), and no safe upgrade exists. It comes in through `serde_yaml` 0.9, which is deprecated. It is added to `deny.toml` `[[advisories.ignore]]` with its reason, following the existing entries. The real remedy is to replace `serde_yaml`, which is a code change and belongs in its own ticket, mika#2686.

When one step of the Security job fails, read the steps after it: they did not run, and their own failures are not visible yet.

## Pitfalls

- `cargo update -p <crate>` can move unrelated edges. Here it also moved `tempfile`'s `getrandom` edge from 0.4.3 to 0.3.4. Read `git diff Cargo.lock`, revert any hunk the advisory does not need, and confirm the lock with `cargo metadata --locked`.
- `cargo check -p quinn` panics in cargo's feature resolver ("did not find features for … within activated_features") because nothing activates the crate. That panic is itself evidence that the crate is not built. It does not mean the bump is broken.
