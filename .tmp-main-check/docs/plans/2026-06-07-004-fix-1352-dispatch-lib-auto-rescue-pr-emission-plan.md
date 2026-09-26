# Plan — fix(self-dev): dispatch-lib auto-rescue PR URL emission (mika#1352)

## Problem

When dispatch-lib's auto-rescue (mika#1282) opens a draft PR after dev-pilot fails to drive git workflow, the rescue emits:

```
Draft PR (dispatch-lib recovery): https://github.com/.../pull/N
```

mika-dev's callback parser at `dispatcher.rs:1780` requires:

```rust
Regex::new(r"(?m)^PR:\s+(https?://github\.com/\S+)").unwrap();
```

Line-anchored `^PR: ` prefix. The auto-rescue's `Draft PR (...)` prefix doesn't match. So `claude_pilot.pr_url` is NOT written to parent metadata → reaper marks parent `callback_delivered_without_pr_url` → parent task false-fails despite PR existing and being reviewable.

## Evidence (from ticket body)

mika#1182 + 5 other tickets in one cycle showed the false-fail pattern. PR#1351 was opened by recovery, mika-qa reviewed it, iteration 1f565655 corrected issues — but parent task `414c8015` forever marked `failed`.

## Fix

Add canonical `PR: ${URL}` line alongside the descriptive `Draft PR (dispatch-lib recovery): ${URL}` line. Same URL, two lines, two purposes:
- `Draft PR (dispatch-lib recovery):` — human-readable signal that recovery owns this
- `PR:` — canonical parser contract from mika#871 R4

1-line addition. Preserves backward-compat with the descriptive emission (audit trail intact).

## Implementation

`mika/skills/bundled/_shared/dispatch-lib.sh` around line 2010:

```bash
if [ -n "$RESCUED_PR_URL" ]; then
    PR_URL="$RESCUED_PR_URL"
    RESULT="${RESULT}
Draft PR (dispatch-lib recovery): ${PR_URL}
PR: ${PR_URL}"
fi
```

## AC

- AC1: dispatch-lib auto-rescue emits both `Draft PR (...)` and canonical `PR: ${URL}` lines.
- AC2: Regression test: callback RESULT containing both forms parses with `claude_pilot.pr_url` populated.
- AC3: Existing dispatch-lib test suite (174 tests) continues to pass.
- AC4: No behavior change for non-rescue path (canonical `PR:` line already emitted at line 87 + 847).

## Defense-in-depth (out of scope, follow-up)

Could also add `gh pr list --head <branch>` fallback in mika-dev callback path. Belt-and-suspenders. Filed as follow-up after this lands if recurrence persists.

## Cross-repo port

dispatch-lib is owned by mika repo (`skills/bundled/_shared/`). Deployed copy at `~/.mika/skills/_shared/` updated by `make deploy`. Single-repo fix.
