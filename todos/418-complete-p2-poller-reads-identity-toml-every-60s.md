---
status: complete
priority: p2
issue_id: "418"
tags: [code-review, performance, reflection]
dependencies: []
---

# Poller Reads identity.toml From Disk Every 60 Seconds

## Problem Statement

`check_and_fire_reflection()` calls `load_identity_async()` on every 60-second poll tick, even when reflection is disabled (which is the default). This is a filesystem read + TOML parse on every tick, unnecessary for the 23+ hours per day when reflection has not yet fired.

## Findings

- **Performance oracle**: "While each individual read is cheap (<1KB), this is unnecessary overhead in a tight polling loop. The identity file is edited rarely."

## Proposed Solutions

### Option A: Cache identity at scheduler construction (Recommended)
Load the `ReflectionConfig` (or just `enabled + time`) once when `ReminderScheduler` is created. If hot-reloading is desired, reload on a longer interval (e.g., every 10 minutes) or on a file watcher.
- **Effort**: Small
- **Risk**: Low (config changes require restart or longer delay)

### Option B: Early-exit with cached enabled flag
Cache just the `enabled` flag at construction. If false, skip the entire function without any I/O.
- **Effort**: Small
- **Risk**: Low

## Technical Details

- **Affected file**: `crates/mika-agent/src/scheduler.rs` (line 334)

## Acceptance Criteria

- [ ] Identity.toml not read on every 60s tick
- [ ] Reflection still fires correctly when enabled
