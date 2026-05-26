# Plan: tool(run_gh) — Restrict `gh api` to read-only endpoint patterns

**Ticket:** mika issue#805
**Type:** enhancement
**Priority:** p3-nice-to-have
**Component:** agent-core

## Problem

`run_gh` already allows the `api` subcommand (added to `GH_ALLOWED_SUBCOMMANDS` at `builtin_handlers.rs:1620`), but there is **no path or method validation** — any `gh api` invocation is permitted, including mutating `PATCH`/`POST`/`DELETE` calls. The existing test at line 2784 explicitly tests that `gh api ... --method PATCH` passes validation, confirming the current wide-open surface.

The ticket asks to restrict `gh api` to **read-only GET requests** against a specific set of endpoint patterns, preventing the agent from performing arbitrary GitHub API mutations through this tool while preserving the orchestrator's ability to verify branch/commit existence.

## Current State

> **Note on issue body staleness (F1, F3 — review-guide.md § citation-or-silence):** The mika#805 issue body states `api` is rejected and references `src/tools/run_gh.rs`. Both are outdated. `run_gh.rs` was absorbed into `builtin_handlers.rs` (the crate restructure predates this issue). `api` was added to `GH_ALLOWED_SUBCOMMANDS` at line 1620 of `builtin_handlers.rs` — verified on this branch at `HEAD`. The issue body's error message also lists `milestone, project` as permitted subcommands, but neither appears in the current `GH_ALLOWED_SUBCOMMANDS` array.

1. `GH_ALLOWED_SUBCOMMANDS` includes `"api"` (line 1620, verified: `["pr", "issue", "run", "workflow", "release", "repo", "search", "label", "api"]`).
2. `extract_api_method()` (line 1839) already parses `--method`/`-X` flags and defaults to `"GET"`.
3. An audit event `gh_api_invocation` logs method + path (lines 1936–1950).
4. No path-pattern or method validation exists beyond the subcommand allowlist.
5. The `gh_read` builtin handler (`GH_READ_ALLOWED_OPS`, line 1078) is a separate read-only tool for mika-arch — it uses a structured `op`/`target` input, not raw `gh api` argv. Not reusable here.
6. `run_gh.rs` does not exist — all `run_gh` logic lives in `crates/mika-agent/src/skills/builtin_handlers.rs`. The issue body's file reference is stale.

## Design

### Approach: Validate `gh api` path + method inline in `run_gh`

Add a `validate_gh_api_scope()` function that runs after `validate_gh_input()` succeeds when `subcommand == "api"`. This function:

1. **Enforces GET-only**: Calls `extract_api_method()` and rejects non-GET methods.
2. **Validates path against an allowlist of regex patterns**: The API path (first non-flag argument after `api`) must match one of the allowed patterns.

### Allowed endpoint patterns

Scoped to the three patterns enumerated in the mika#805 issue body's "Proposed Solution" section: `branches/{branch}`, `branches` (list), `commits/{sha}` (review-guide.md § YAGNI — scope must be right-sized to the stated goal).

```rust
const GH_API_READ_ALLOWED_PATTERNS: &[&str] = &[
    // Branch verification
    r"^/?repos/[^/]+/[^/]+/branches/[^/]+$",   // single branch
    r"^/?repos/[^/]+/[^/]+/branches$",           // list branches
    // Commit verification
    r"^/?repos/[^/]+/[^/]+/commits/[a-fA-F0-9]+$", // single commit by SHA
];
```

The leading `/?` handles both `/repos/...` and `repos/...` forms (gh CLI accepts both).

> **Milestones not included (F2):** The mika#805 issue enumerates exactly three patterns for the branch-existence-verification use case. Milestone read access via `gh api` is a different use case; if needed, open a separate ticket or extend the allowlist in a follow-up PR.

### Where to place the check

In `run_gh()`, immediately after the qa-review scope check (line 1868) and before the PR dedup key computation (line 1873). This mirrors the layered validation pattern: global allowlist → skill-scoped scope → api-specific scope → execution.

### Error message

```
gh api path '{path}' with method '{method}' is not allowed.
Only GET requests to specific read-only endpoints are permitted.
Allowed patterns: repos/{owner}/{repo}/branches[/{branch}], repos/{owner}/{repo}/commits/{sha}.
```

## Implementation

### Step 1: Add `GH_API_READ_ALLOWED_PATTERNS` constant and `validate_gh_api_scope()`

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Add after the `QA_REVIEW_GH_ALLOWED` constant (line 1633):

```rust
/// Allowed read-only `gh api` endpoint patterns (mika#805).
///
/// Only GET requests matching one of these patterns are permitted.
/// Patterns use regex; leading `/` is optional (gh CLI accepts both forms).
/// Compiled once via `LazyLock` — malformed patterns surface immediately on
/// first use rather than silently denying all requests (review-guide.md § KISS).
const GH_API_READ_ALLOWED_PATTERNS: &[&str] = &[
    r"^/?repos/[^/]+/[^/]+/branches/[^/]+$",
    r"^/?repos/[^/]+/[^/]+/branches$",
    r"^/?repos/[^/]+/[^/]+/commits/[a-fA-F0-9]+$",
];

static GH_API_READ_COMPILED: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
    GH_API_READ_ALLOWED_PATTERNS
        .iter()
        .map(|p| regex::Regex::new(p).unwrap_or_else(|e| {
            panic!("BUG: malformed GH_API_READ_ALLOWED_PATTERNS regex '{p}': {e}")
        }))
        .collect()
});
```

> **F4 addressed:** Uses `LazyLock<Vec<Regex>>` (same pattern as `HTTP_CLIENT` at line 56) so regexes compile once at first use. `panic!` on malformed pattern is correct — these are compile-time constants, not user input; a panic at first `gh api` call surfaces the bug immediately in logs rather than silently denying all requests.

Add `validate_gh_api_scope()` after `validate_qa_review_gh_scope()`:

```rust
/// Validate `gh api` invocations: GET-only + path allowlist (mika#805).
///
/// Extracts the HTTP method (default GET) and the API path from argv.
/// Rejects non-GET methods and paths that don't match any allowed pattern.
fn validate_gh_api_scope(args: &[String]) -> Result<(), ToolOutput> {
    if args.first().map(String::as_str) != Some("api") {
        return Ok(());
    }

    let method = extract_api_method(args);
    if !method.eq_ignore_ascii_case("GET") {
        return Err(ToolOutput::error(format!(
            "gh api method '{method}' is not allowed. Only GET requests are permitted \
             through run_gh. Use the appropriate gh subcommand (e.g., gh issue, gh pr) \
             for write operations."
        )));
    }

    // Extract the API path: first positional arg after "api" that doesn't start with "-"
    let path = args.iter().skip(1)
        .find(|s| !s.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("");

    let matched = GH_API_READ_COMPILED.iter().any(|re| re.is_match(path));

    if !matched {
        return Err(ToolOutput::error(format!(
            "gh api path '{path}' is not in the read-only allowlist. \
             Allowed: repos/{{owner}}/{{repo}}/branches[/{{branch}}], \
             repos/{{owner}}/{{repo}}/commits/{{sha}}."
        )));
    }

    Ok(())
}
```

### Step 2: Wire into `run_gh()`

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

In `run_gh()`, add the `gh api` scope check after the qa-review scope check:

```rust
// gh api read-only scope check (mika#805): restrict api subcommand to
// GET-only against allowed endpoint patterns.
if let Err(err) = validate_gh_api_scope(&gh_args.args) {
    return err;
}
```

### Step 3: Add dependency on `regex` crate

**File:** `crates/mika-agent/Cargo.toml`

Check if `regex` is already a dependency. If not, add it. (It likely is — many Rust projects include it transitively or directly.)

### Step 4: Update existing test

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

The existing test `test_run_gh_allowlist_accepts_api` (line 2784) tests a `PATCH` request which should now be rejected. Update it and add comprehensive tests:

```rust
#[test]
fn test_gh_api_get_branches_allowed() {
    let args: Vec<String> = vec!["api", "repos/senara-solutions/mika/branches/main"]
        .into_iter().map(String::from).collect();
    assert!(validate_gh_api_scope(&args).is_ok());
}

#[test]
fn test_gh_api_get_branches_list_allowed() {
    let args: Vec<String> = vec!["api", "repos/senara-solutions/mika/branches"]
        .into_iter().map(String::from).collect();
    assert!(validate_gh_api_scope(&args).is_ok());
}

#[test]
fn test_gh_api_get_commit_allowed() {
    let args: Vec<String> = vec!["api", "repos/senara-solutions/mika/commits/abc123def"]
        .into_iter().map(String::from).collect();
    assert!(validate_gh_api_scope(&args).is_ok());
}

#[test]
fn test_gh_api_leading_slash_allowed() {
    let args: Vec<String> = vec!["api", "/repos/senara-solutions/mika/branches/main"]
        .into_iter().map(String::from).collect();
    assert!(validate_gh_api_scope(&args).is_ok());
}

#[test]
fn test_gh_api_patch_rejected() {
    let args: Vec<String> = vec!["api", "repos/o/r/branches/main", "--method", "PATCH", "-f", "protection=false"]
        .into_iter().map(String::from).collect();
    let result = validate_gh_api_scope(&args);
    assert!(result.is_err());
}

#[test]
fn test_gh_api_post_rejected() {
    let args: Vec<String> = vec!["api", "repos/o/r/issues", "-X", "POST"]
        .into_iter().map(String::from).collect();
    let result = validate_gh_api_scope(&args);
    assert!(result.is_err());
}

#[test]
fn test_gh_api_milestones_not_allowed() {
    // Milestones not in scope per mika#805 (F2 — review-guide.md § YAGNI).
    let args: Vec<String> = vec!["api", "repos/o/r/milestones"]
        .into_iter().map(String::from).collect();
    let result = validate_gh_api_scope(&args);
    assert!(result.is_err());
}

#[test]
fn test_gh_api_disallowed_path_rejected() {
    let args: Vec<String> = vec!["api", "repos/o/r/pulls"]
        .into_iter().map(String::from).collect();
    let result = validate_gh_api_scope(&args);
    assert!(result.is_err());
}

#[test]
fn test_gh_api_arbitrary_path_rejected() {
    let args: Vec<String> = vec!["api", "graphql"]
        .into_iter().map(String::from).collect();
    let result = validate_gh_api_scope(&args);
    assert!(result.is_err());
}

#[test]
fn test_gh_api_non_api_subcommand_skipped() {
    let args: Vec<String> = vec!["pr", "list"]
        .into_iter().map(String::from).collect();
    assert!(validate_gh_api_scope(&args).is_ok());
}
```

Update the existing `test_run_gh_allowlist_accepts_api` test to use a valid GET request:

```rust
#[test]
fn test_run_gh_allowlist_accepts_api() {
    let input = serde_json::json!({
        "command": ["api", "repos/owner/repo/branches/main"]
    });
    let result = validate_gh_input(&input);
    assert!(result.is_ok(), "gh api should be allowed at input validation level");
}
```

### Step 5: Update CLAUDE.md documentation

**File:** `crates/mika-agent/CLAUDE.md`

In the `run_gh` / GitHub CLI handler section, add a note about the `gh api` read-only restriction:

> **`gh api` read-only restriction (#805):** `gh api` is in the global subcommand allowlist but further restricted: only GET requests matching specific endpoint patterns are permitted. Allowed patterns: `repos/{owner}/{repo}/branches[/{branch}]`, `repos/{owner}/{repo}/commits/{sha}`. Non-GET methods and non-matching paths are rejected by `validate_gh_api_scope()`. Extends the three-tier validation: global allowlist → skill-scoped scope (qa-review) → api-specific scope.

## Testing

1. `cargo test -p mika-agent -- validate_gh_api` — unit tests for the new validation function
2. `cargo test -p mika-agent -- test_run_gh_allowlist` — updated existing test
3. `cargo test -p mika-agent -- test_extract_api_method` — existing tests still pass
4. `cargo clippy -p mika-agent` — no warnings
5. `cargo build` — full build succeeds

## Risks

- **Pattern too restrictive:** If future use cases need additional read-only endpoints (e.g., `repos/{owner}/{repo}/tags`, `repos/{owner}/{repo}/releases`, `repos/{owner}/{repo}/milestones`), the allowlist is a single constant to extend. The error message guides the operator to the right place.
- **Path extraction edge case:** The "first non-flag arg after `api`" heuristic could misparse exotic argv orderings. In practice, `gh api <path> [flags]` is the canonical form; the `gh` CLI itself expects the path as the first positional argument.

## Not in scope

- **Write operations via `gh api`:** The `pr_merge_with_gate` tool and `gh_read` handler cover write and structured-read use cases respectively. `run_gh` is the general-purpose escape hatch with appropriate guardrails.
- **GraphQL endpoint:** `gh api graphql` is a powerful mutation vector; remains blocked by the path allowlist.
- **`gh_read` handler changes:** That tool serves a different purpose (structured read-only ops for mika-arch with typed errors). No changes needed.

## Revision history

- rev 2 (2026-05-26): addressed F1 by verifying `api` IS in `GH_ALLOWED_SUBCOMMANDS` at line 1620 (confirmed on branch HEAD), adding staleness note to Current State with line-level citation, noting `run_gh.rs` was absorbed into `builtin_handlers.rs`; addressed F2 by removing milestones pattern from `GH_API_READ_ALLOWED_PATTERNS` — only the three issue-enumerated patterns remain (branches/{branch}, branches, commits/{sha}), added negative test for milestones, updated error messages and docs (review-guide.md § YAGNI); addressed F3 by adding item 6 to Current State confirming `run_gh.rs` does not exist and the issue body reference is stale; addressed F4 by replacing per-call `Regex::new().unwrap_or(false)` with `LazyLock<Vec<Regex>>` (same pattern as `HTTP_CLIENT` at line 56), panic on malformed pattern surfaces config errors immediately (review-guide.md § KISS).
