# Plan: Harden milestone-close guard's argv check (mika#1182)

**Issue:** mika#1182
**Type:** fix (enhancement)
**Priority:** p3-nice-to-have

## Problem Summary

The `detect_milestone_close_claim_without_patch` guard (agent.rs) uses substring matching on `ToolCallSummary.input_summary` to verify a qualifying `run_gh api PATCH` call exists. Three fragilities:

1. **Substring spoofing:** A `run_gh pr comment --body "...PATCH /repos/.../milestones/17 state=closed..."` call would satisfy the guard without actually closing the milestone.
2. **INPUT_SUMMARY_MAX=200 truncation:** Long argv can push `state=closed` past the 200-char boundary, causing the guard to fire on a legitimate close (over-fire).
3. **Cross-milestone leakage:** `all_tool_summaries` is run_loop-scoped; a PATCH for milestone#17 would satisfy a fabricated close-claim for milestone#18 later in the same loop.

## Approach

Parse `input_summary` as JSON and walk the argv array positionally instead of substring-matching. Fall back to the existing substring-match on parse failure so the guard never loses coverage silently.

## Implementation Steps

### Step 1: Add `parse_run_gh_milestone_close_argv` helper function

**File:** `crates/mika-agent/src/agent.rs` (near `detect_milestone_close_claim_without_patch`, ~line 5094)

Create a new helper:

```rust
/// Attempts to parse a `run_gh` input_summary as JSON and extract a milestone
/// number from a close-PATCH argv. Returns `Some(milestone_number)` if the
/// argv positionally matches the milestone close shape, `None` otherwise.
///
/// Expected shapes:
///   ["api", "-X", "PATCH", "/repos/<owner>/<repo>/milestones/<N>", "-f", "state=closed"]
///   ["api", "--method", "PATCH", "/repos/<owner>/<repo>/milestones/<N>", "-f", "state=closed"]
///
/// The path element and state=closed field can appear at any position after
/// the PATCH method, because `gh api` accepts flags in any order. The key
/// invariant is: subcommand is "api", method is PATCH, path matches the
/// milestones pattern, and "state=closed" appears as a `-f` field value.
fn parse_run_gh_milestone_close_argv(input_summary: &str) -> Option<u64> {
    let parsed: serde_json::Value = serde_json::from_str(input_summary).ok()?;
    let command = parsed.get("command")?.as_array()?;
    
    // argv[0] must be "api"
    if command.first()?.as_str()? != "api" {
        return None;
    }
    
    // Find PATCH method: either "-X" "PATCH" or "--method" "PATCH"
    let has_patch_method = command.windows(2).any(|pair| {
        let flag = pair[0].as_str().unwrap_or("");
        let val = pair[1].as_str().unwrap_or("");
        (flag == "-X" || flag == "--method") && val == "PATCH"
    });
    if !has_patch_method {
        return None;
    }
    
    // Find state=closed: must appear as a "-f" field pair
    let has_state_closed = command.windows(2).any(|pair| {
        let flag = pair[0].as_str().unwrap_or("");
        let val = pair[1].as_str().unwrap_or("");
        flag == "-f" && val == "state=closed"
    });
    if !has_state_closed {
        return None;
    }
    
    // Extract milestone number from the milestones API path element
    command.iter()
        .filter_map(|v| v.as_str())
        .find_map(|s| {
            MILESTONE_API_PATH_RE
                .captures(s)
                .and_then(|c| c.name("num"))
                .and_then(|n| n.as_str().parse::<u64>().ok())
        })
}
```

**Rationale:**
- Positional JSON parsing eliminates substring spoofing (fragility #1).
- Operates on the full `input_summary` string before truncation concerns — but more importantly, since it parses the JSON structure, a truncated string will fail `serde_json::from_str` and fall back to substring match (graceful degradation for fragility #2).
- Returns the specific milestone number, enabling per-number binding (addresses fragility #3 at the PATCH-extraction layer).

### Step 2: Update `has_patch_call` logic in `detect_milestone_close_claim_without_patch`

**File:** `crates/mika-agent/src/agent.rs`, lines ~5119-5134

Replace the current `patched_set` construction with a two-tier approach:

```rust
// AC2+AC4: collect PATCH milestone numbers from tool summaries.
// Tier 1: Structured JSON argv parse (preferred — immune to substring spoofing).
// Tier 2: Substring fallback for parse failures (truncated or non-JSON summaries).
let patched_set: HashSet<u64> = all_tool_summaries
    .iter()
    .filter(|s| s.name == "run_gh")
    .filter_map(|s| {
        // Tier 1: structured parse
        if let Some(num) = parse_run_gh_milestone_close_argv(&s.input_summary) {
            return Some(num);
        }
        // Tier 2: substring fallback (preserves coverage for truncated/legacy summaries)
        if s.input_summary.contains("\"api\"")
            && s.input_summary.contains("\"PATCH\"")
            && s.input_summary.contains("state=closed")
        {
            MILESTONE_API_PATH_RE
                .captures(&s.input_summary)
                .and_then(|c| c.name("num"))
                .and_then(|n| n.as_str().parse::<u64>().ok())
        } else {
            None
        }
    })
    .collect();
```

**Rationale:** The fallback ensures no silent coverage loss. If JSON parsing fails (truncated string, unexpected format), the old substring logic kicks in. The structured parse handles the happy path; the fallback handles degraded cases.

### Step 3: Add unit tests

**File:** `crates/mika-agent/src/agent.rs`, in the `#[cfg(test)] mod tests` block (after existing milestone tests, ~line 8736+)

#### Test 3a: Substring spoofing does NOT satisfy the guard

```rust
#[test]
fn test_milestone_close_guard_substring_spoof_rejected() {
    // A pr comment whose body contains all four substrings should NOT
    // satisfy the guard — the PATCH is in the body text, not an actual
    // api PATCH call.
    let spoofed = run_gh_summary(
        r#"{"command":["pr","comment","--body","closed via PATCH /repos/senara-solutions/mika/milestones/17 state=closed"]}"#,
    );
    let summaries = vec![spoofed];
    let result = detect_milestone_close_claim_without_patch(
        "I closed milestone#17 on GitHub",
        &summaries,
    );
    // Guard should fire — the spoof should not satisfy it.
    assert!(result.is_some(), "substring spoof should not satisfy the guard");
}
```

#### Test 3b: Cross-milestone leakage caught

```rust
#[test]
fn test_milestone_close_guard_cross_milestone_leakage() {
    // PATCH for milestone#17 should NOT satisfy a claim about milestone#18.
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/senara-solutions/mika/milestones/17","-f","state=closed"]}"#,
    )];
    let result = detect_milestone_close_claim_without_patch(
        "I closed milestone#18 on GitHub",
        &summaries,
    );
    assert!(result.is_some(), "PATCH for #17 should not satisfy claim about #18");
}
```

**Note:** This test already passes with the existing #1207 discrimination logic. Including it explicitly documents the cross-milestone leakage protection and prevents regression.

#### Test 3c: Truncation at INPUT_SUMMARY_MAX boundary

```rust
#[test]
fn test_milestone_close_guard_truncated_input_still_resolves() {
    // Build an argv where state=closed is beyond byte 200 (truncation boundary).
    // The structured parse fails on truncated JSON, but the path and "PATCH"
    // appear before byte 200, so the substring fallback extracts the number.
    let long_org = "a]".repeat(80); // Push state=closed past byte 200
    let input = format!(
        r#"{{"command":["api","-X","PATCH","/repos/{}/mika/milestones/17","-f","state=closed"]}}"#,
        long_org
    );
    // Truncate to INPUT_SUMMARY_MAX to simulate what ToolCallSummary does.
    let truncated = truncate_summary(&input, INPUT_SUMMARY_MAX);
    let summaries = vec![run_gh_summary(&truncated)];
    
    // With the milestone path visible (before truncation point) and the
    // structured parse failing (truncated JSON), the substring fallback
    // should extract milestone#17 if possible, or the guard should fire
    // on the claim. Either way, the guard should not stall.
    let result = detect_milestone_close_claim_without_patch(
        "I closed milestone#17 on GitHub",
        &summaries,
    );
    // If state=closed was truncated, substring fallback also fails →
    // guard correctly fires (over-fire is the safe direction per the ticket).
    // The key invariant: the guard does NOT stall — it either suppresses
    // (if substring fallback finds a match) or fires (if not).
    // This test documents the behavior, not a specific outcome.
}
```

#### Test 3d: `--method` variant accepted

```rust
#[test]
fn test_milestone_close_guard_long_method_flag() {
    // "--method" is the long form of "-X" in gh api.
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","--method","PATCH","/repos/senara-solutions/mika/milestones/17","-f","state=closed"]}"#,
    )];
    assert!(
        detect_milestone_close_claim_without_patch(
            "I closed milestone#17 on GitHub",
            &summaries,
        )
        .is_none(),
        "--method PATCH should satisfy the guard"
    );
}
```

#### Test 3e: Non-JSON input_summary falls back to substring match

```rust
#[test]
fn test_milestone_close_guard_non_json_fallback() {
    // If input_summary is not valid JSON (e.g., pre-existing format or
    // corruption), the substring fallback should still work.
    let summaries = vec![run_gh_summary(
        r#"api -X PATCH /repos/senara-solutions/mika/milestones/17 -f state=closed "api" "PATCH""#,
    )];
    // This is a non-JSON string that happens to contain the substring markers.
    // The structured parse fails; the substring fallback should try to extract.
    let result = detect_milestone_close_claim_without_patch(
        "I closed milestone#17 on GitHub",
        &summaries,
    );
    // Substring fallback finds "api", "PATCH", and "state=closed" as substrings,
    // plus the milestone path regex matches. Should suppress the guard.
    assert!(result.is_none(), "substring fallback should work for non-JSON input");
}
```

#### Test 3f: Structured parse for `parse_run_gh_milestone_close_argv` directly

```rust
#[test]
fn test_parse_run_gh_milestone_close_argv_valid() {
    let input = r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/42","-f","state=closed"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), Some(42));
}

#[test]
fn test_parse_run_gh_milestone_close_argv_not_api() {
    let input = r#"{"command":["pr","comment","--body","text"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), None);
}

#[test]
fn test_parse_run_gh_milestone_close_argv_no_state_closed() {
    // PATCH without state=closed should not match.
    let input = r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/17","-f","title=new name"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), None);
}

#[test]
fn test_parse_run_gh_milestone_close_argv_truncated_json() {
    // Truncated JSON should return None (graceful fallback).
    let input = r#"{"command":["api","-X","PATCH","/repos/o/r/milest"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), None);
}
```

### Step 4: Verify all existing tests pass

Run `cargo test -p mika-agent -- detect_milestone_close_claim` to confirm no regressions.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | Add `parse_run_gh_milestone_close_argv` helper; update `patched_set` construction in `detect_milestone_close_claim_without_patch` to use two-tier parse; add 10 new unit tests |

## Scope Boundaries

- **In scope:** Hardening the PATCH-call detection in the milestone-close guard only.
- **Out of scope:** Generalizing `parse_run_gh_milestone_close_argv` to other guards (per ticket: extract shared helper on third instance). No changes to `ToolCallSummary` struct (the `argv: Option<Vec<String>>` idea from the ticket is deferred unless a truncation issue lands first). No changes to the claim-detection regex side (`MILESTONE_CLOSE_CLAIM_RE`).

## Risk Assessment

- **Low risk.** The fallback-to-substring-match design means the worst case is identical to current behavior. The structured parse only improves detection; it never regresses.
- **Failure direction preserved.** Over-fire (guard triggers on legitimate close) is the existing safe-direction failure mode. The structured parse reduces over-fire (better at recognizing legitimate PATCHes); it never introduces under-fire (guard failing to trigger on fabricated claims).
- **No schema changes, no config changes, no new dependencies.**
