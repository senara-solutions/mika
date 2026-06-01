# Plan: Fix dispatch-lib auto-rescue PR URL callback contract (mika#1352)

## Problem

When dispatch-lib's dirty-worktree auto-rescue path (mika#1282) opens a draft PR, the PR URL is written to the callback RESULT string in the wrong format:

```
Draft PR (dispatch-lib recovery): https://github.com/senara-solutions/mika/pull/1351
```

The callback metadata parser in `dispatcher.rs` uses a strict regex anchored at line start:

```rust
static RE_PR_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^PR:\s+(https?://github\.com/\S+)").unwrap());
```

This regex requires `^PR:\s+<url>` — the `Draft PR (dispatch-lib recovery):` prefix does not match. The PR URL is never extracted into task metadata, causing:
1. The orphaned-parent reaper sees `pr_url IS NULL` and marks the parent task `failed` with `callback_delivered_without_pr_url`
2. The parent-completer (mika#1162) cannot fire (it requires `pr_url IS NOT NULL`)
3. mika-dev's callback turn receives no `pr_url` in metadata, triggering the failure classification path

Concrete impact: 6 tickets in one cycle hit `callback_delivered_without_pr_url`; 3 of those had rescue PRs that existed and were reviewable.

## Root cause

Single line — `dispatch-lib.sh:1758`:
```bash
RESULT="${RESULT}
Draft PR (dispatch-lib recovery): ${PR_URL}"
```

The exit-trap crash-recovery path at line 86-87 correctly uses `PR: ${_PR_URL}`, but the normal rescue-PR path at line 1758 uses a descriptive label that violates the stdout contract.

## Fix

### Change 1 (primary): Fix the RESULT format in dispatch-lib.sh

**File:** `skills/bundled/_shared/dispatch-lib.sh`  
**Line:** 1758

Change:
```bash
RESULT="${RESULT}
Draft PR (dispatch-lib recovery): ${PR_URL}"
```

To:
```bash
RESULT="${RESULT}
PR: ${PR_URL}
Draft PR (dispatch-lib recovery): ${PR_URL}"
```

Emit the canonical `PR: <url>` line first (for the regex parser), then keep the descriptive line (for human-readable log output). The regex is `(?m)^PR:\s+` with multiline mode, so it matches the first occurrence at a line start. The descriptive line is not at a problematic position since the regex captures the first match.

**Why not just replace the descriptive line?** The descriptive prefix `Draft PR (dispatch-lib recovery):` is valuable in logs and callback text for distinguishing rescue PRs from normal PRs. Keeping both preserves observability without breaking the parser contract.

### Change 2 (defense-in-depth): Add rescue-PR format to the Rust regex as a fallback

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`  
**Lines:** ~1780-1781

Add a second capture pattern to `RE_PR_URL` that also accepts the `Draft PR` prefix:

```rust
static RE_PR_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^(?:Draft )?PR(?:\s*\([^)]*\))?:\s+(https?://github\.com/\S+)").unwrap());
```

This matches:
- `PR: <url>` (normal dev-pilot output — existing contract)
- `Draft PR (dispatch-lib recovery): <url>` (rescue path — new)
- `Draft PR: <url>` (hypothetical minimal rescue label)

The `(?:Draft )?` makes "Draft " optional. The `(?:\s*\([^)]*\))?` makes the parenthetical annotation optional. The core `:\s+<url>` suffix is unchanged.

**Update the doc comment** at lines 1777-1779 to reference both emission sites:
```rust
// PR URL emitted by dev-pilot/handlers/run.sh:398 — `PR: <url>`
// and by dispatch-lib.sh rescue path — `Draft PR (dispatch-lib recovery): <url>`.
// Anchored at line start (multiline) so free-text mentions don't match.
// See mika#871 R4 for the integration contract, mika#1352 for rescue-PR format.
```

### Change 3: Add test coverage for rescue-PR format

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`  
**Location:** Near the existing `RE_PR_URL` test at ~line 2406-2448

Add test cases:
1. `"PR: https://github.com/x/y/pull/1"` — normal format (existing, verify no regression)
2. `"Draft PR (dispatch-lib recovery): https://github.com/x/y/pull/2"` — rescue format (new)
3. `"Draft PR: https://github.com/x/y/pull/3"` — minimal draft format (new)
4. Multi-line input with rescue PR on its own line (new)
5. Verify mid-line `Draft PR (dispatch-lib recovery):` does NOT match (anchored)

### Change 4: Add compound doc

**File:** `docs/solutions/best-practices/dispatch-lib-rescue-pr-callback-contract-2026-06-01.md`

Document:
- The stdout contract for PR URL emission (`^PR:\s+<url>`)
- Both emission sites (dev-pilot run.sh and dispatch-lib rescue path)
- The regex in dispatcher.rs and why defense-in-depth matters
- The failure mode: format mismatch → no metadata → reaper marks failed

## Files changed

| File | Type | Description |
|------|------|-------------|
| `skills/bundled/_shared/dispatch-lib.sh` | Shell | Add canonical `PR:` line before descriptive rescue-PR line |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | Rust | Broaden `RE_PR_URL` regex to accept rescue-PR format; add tests |
| `docs/solutions/best-practices/dispatch-lib-rescue-pr-callback-contract-2026-06-01.md` | Docs | Compound learning doc |

## Verification

1. `cargo test -p mika-agent` — new regex tests pass, existing tests unbroken
2. `cargo clippy` — no warnings
3. Manual: grep `dispatch-lib.sh` for all `RESULT=` assignments to confirm no other format violations exist
4. Post-deploy signal: `grep callback_delivered_without_pr_url server.log` count should drop to only genuine no-PR cases (the 3 "no PR" tickets from the concrete instance)

## Risk

Low. The shell change adds a line; the regex change is backward-compatible (still matches the old `PR:` format). The descriptive line is preserved for log readability. No schema changes, no new tools, no new skills.
