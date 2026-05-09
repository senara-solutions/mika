---
module: infrastructure
tags: [makefile, deploy, atomic-install, etxtbsy]
problem_type: operational
category: infrastructure
---

# Makefile deploy chain decomposed into build/install/restart verbs

## Problem

`make deploy` chained `build-dashboard build stop install restart check-ngrok`. The `stop` before `install` was necessary because plain `cp` to a running binary triggers `ETXTBSY` on Linux (kernel denies write access to files being executed). This meant `make install` could not be invoked independently while services were running — every deploy interrupted in-flight work.

## Solution

1. **Atomic install:** Changed `install` target from plain `cp` to `cp .tmp` + `mv` pattern. `cp` writes to a new `.tmp` file (no ETXTBSY), then `mv` atomically replaces the inode. Running processes keep the old inode in memory; new processes pick up the new binary on next exec.

2. **Removed `stop` from deploy chain:** Since `install` is now safe to run while services are running, `stop` is redundant — `restart` (which is `rc-service restart`, i.e., stop+start) handles the service flip.

New chain: `deploy: build-dashboard build install restart check-ngrok`

## Key insight

The `cp .tmp` + `mv` pattern is the standard Unix safe-replacement pattern. On the same filesystem, `mv` is an atomic `rename(2)` syscall that replaces the directory entry without affecting processes holding the old inode. Cross-filesystem `mv` degrades to copy+delete (same as old `cp`, sub-millisecond window for ~50MB binaries).

## Operator benefit

Operators can now run `make build && make install` while services are still running (preparing binaries at leisure), then `make restart` at a quiescent boundary when no in-flight work would be interrupted. This is the foundation for quiescent-boundary deploy discipline.
