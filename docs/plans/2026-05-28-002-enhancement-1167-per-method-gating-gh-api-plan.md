# Plan: Per-Method Gating for `gh api` (mika#1167)

## Context

This ticket was deferred from mika#805 (2026-05-17) after the operator ratified mika#788's unrestricted `gh api` + audit event design. The current implementation enforces a **GET-only** restriction with a narrow path allowlist via `validate_gh_api_scope()` in `builtin_handlers.rs`. The `gh_api_invocation` audit event (also from #788) provides post-hoc observability for every `gh api` call.

**Current state (from #805, already implemented):**
- `validate_gh_api_scope()` at `builtin_handlers.rs:1858-1895` rejects all non-GET methods
- `GH_API_READ_ALLOWED_PATTERNS` at `builtin_handlers.rs:1641-1645` allows exactly 3 read-only endpoint patterns:
  - `repos/{owner}/{repo}/branches/{branch}` — verify branch existence
  - `repos/{owner}/{repo}/branches` — list branches
  - `repos/{owner}/{repo}/commits/{sha}` — verify commit existence
- `extract_api_method()` at `builtin_handlers.rs:1897-1914` parses `--method X`, `--method=X`, `-X X` forms
- `gh_api_invocation` audit event at `builtin_handlers.rs:2004-2018` logs `session_id`, `method`, `path`
- Tests at `builtin_handlers.rs:2864-2982` cover method extraction and scope validation

**Opening criteria (from ticket body):**
- (a) Prompt-injection escape observed in `gh_api_invocation` audit logs, OR
- (b) A second use case emerges needing specific method+path combinations where audit-event observability is insufficient

**Design tension (from #788 plan § mika#805 disposition):** A per-method gate that allows the mutation methods the platform actually needs (PATCH for milestone close, POST for issue creation, etc.) is blast-radius-equivalent to unrestricted — the dangerous methods are exactly the ones surfaced use cases need. The value of per-method gating is therefore not in reducing blast radius for known use cases, but in:
1. **Deny-by-default for unsurfaced endpoints** — new `gh api` usage must pass through the allowlist, creating a review checkpoint
2. **Discrimination between read and write** — agents that only need read access cannot accidentally mutate
3. **Audit enrichment** — the `allowed_by_rule` field in the audit event provides structured signal for anomaly detection

## Approach

Replace the current monolithic GET-only gate with a **method+path matrix** where each entry specifies:
- An HTTP method (or wildcard)
- A regex pattern for the API path
- A human-readable rule name (for audit + error messages)

The matrix starts with the three existing GET patterns plus new entries for each concrete mutation use case that has been observed in audit logs or operator-approved workflows. Unknown method+path combinations are denied by default.

## Implementation

### Step 1 — Define the method+path matrix type

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Replace `GH_API_READ_ALLOWED_PATTERNS` (lines 1641-1645) and `GH_API_READ_COMPILED` (lines 1647-1656) with a new structure:

```rust
/// Per-method gating entry for `gh api` (mika#1167).
///
/// Each entry defines an allowed method+path combination. The matrix is
/// deny-by-default: any `gh api` call whose method+path does not match
/// at least one entry is rejected.
struct GhApiAllowEntry {
    /// HTTP method (case-insensitive match). `"*"` matches any method.
    method: &'static str,
    /// Regex pattern for the API path (leading `/` optional, same as `gh` CLI).
    path_pattern: &'static str,
    /// Human-readable rule name for audit events and error messages.
    rule: &'static str,
}

const GH_API_ALLOW_MATRIX: &[GhApiAllowEntry] = &[
    // -- Read-only (carried forward from #805) --
    GhApiAllowEntry {
        method: "GET",
        path_pattern: r"^/?repos/[^/]+/[^/]+/branches/[^/]+$",
        rule: "read:branch",
    },
    GhApiAllowEntry {
        method: "GET",
        path_pattern: r"^/?repos/[^/]+/[^/]+/branches$",
        rule: "read:branches-list",
    },
    GhApiAllowEntry {
        method: "GET",
        path_pattern: r"^/?repos/[^/]+/[^/]+/commits/[a-fA-F0-9]+$",
        rule: "read:commit",
    },
    // -- Mutations (add here when opening criteria are met) --
    // Example (milestone close — the #788 repro case):
    // GhApiAllowEntry {
    //     method: "PATCH",
    //     path_pattern: r"^/?repos/[^/]+/[^/]+/milestones/\d+$",
    //     rule: "write:milestone-update",
    // },
];
```

Compile the regex patterns into a `LazyLock<Vec<CompiledGhApiAllowEntry>>` similar to the current `GH_API_READ_COMPILED`, where each compiled entry carries the `Regex` + method + rule name.

### Step 2 — Rewrite `validate_gh_api_scope()` to use the matrix

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs` (lines 1858-1895)

```rust
fn validate_gh_api_scope(args: &[String]) -> Result<&'static str, ToolOutput> {
    if args.first().map(String::as_str) != Some("api") {
        return Ok("");  // Not an API call — skip
    }

    let method = extract_api_method(args);
    let path = args
        .iter()
        .skip(1)
        .find(|s| !s.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("");

    // Find the first matching rule (method + path)
    for entry in GH_API_ALLOW_COMPILED.iter() {
        let method_matches = entry.method == "*"
            || entry.method.eq_ignore_ascii_case(method);
        if method_matches && entry.pattern.is_match(path) {
            return Ok(entry.rule);
        }
    }

    // No match — deny with structured error
    Err(ToolOutput::error(format!(
        "gh api {method} '{path}' is not in the allowed method+path matrix. \
         Allowed combinations: {rules}. \
         Use the appropriate gh subcommand (e.g., gh issue, gh pr) for other operations.",
        rules = GH_API_ALLOW_COMPILED
            .iter()
            .map(|e| format!("{} ({})", e.rule, e.method))
            .collect::<Vec<_>>()
            .join(", ")
    )))
}
```

**Change the return type** from `Result<(), ToolOutput>` to `Result<&'static str, ToolOutput>` — the `Ok` variant now carries the matching rule name for the audit event enrichment (Step 3). Update the callsite in `run_gh()` to capture the rule name.

### Step 3 — Enrich the audit event with the matched rule

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs` (lines 2002-2018)

Update the `gh_api_invocation` audit event to include the `allowed_by_rule` field:

```rust
if gh_args.args.first().map(|s| s.as_str()) == Some("api") {
    let method = extract_api_method(&gh_args.args);
    let path = gh_args.args.get(1).map(|s| s.as_str()).unwrap_or("<missing>");
    tracing::info!(
        event = "gh_api_invocation",
        session_id = %ctx.session_id,
        method = %method,
        path = %path,
        allowed_by_rule = %matched_rule,  // from Step 2's Ok value
        "gh api invocation"
    );
}
```

The `allowed_by_rule` field enables structured anomaly detection: operators can group invocations by rule and flag unexpected patterns without parsing method+path combinations manually.

### Step 4 — Update tests

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Update existing tests (lines 2864-2982) and add new ones:

1. **Update `test_extract_api_method`** — no changes needed (method extraction is orthogonal to gating)

2. **Update existing `test_gh_api_*` tests** — change `is_ok()` assertions to check `Ok(rule_name)`:
   - `test_gh_api_get_branches_allowed` → `assert_eq!(result.unwrap(), "read:branch")`
   - `test_gh_api_get_branches_list_allowed` → `assert_eq!(result.unwrap(), "read:branches-list")`
   - `test_gh_api_get_commit_allowed` → `assert_eq!(result.unwrap(), "read:commit")`
   - `test_gh_api_leading_slash_allowed` → `assert_eq!(result.unwrap(), "read:branch")`

3. **Update rejection tests** — error message changes from "Only GET requests" to the new matrix-based message:
   - `test_gh_api_patch_rejected` — update error string match
   - `test_gh_api_post_rejected` — update error string match
   - `test_gh_api_delete_rejected` — keep as rejection test

4. **New tests for future mutation entries:**
   - `test_gh_api_matrix_denies_unmatched_method_on_allowed_path` — GET path with POST method → rejected
   - `test_gh_api_matrix_denies_allowed_method_on_unmatched_path` — GET with random path → rejected
   - `test_gh_api_matrix_rule_name_propagated` — verify the `Ok` variant carries the correct rule name
   - `test_gh_api_non_api_skipped` — keep existing test (non-api subcommands bypass)

5. **Compile-time guard:** Add a `const_assert`-style test that verifies all `GH_API_ALLOW_MATRIX` entries compile as valid regex. This catches copy-paste errors in the constant before runtime.

### Step 5 — Update CLAUDE.md documentation

**File:** `crates/mika-agent/CLAUDE.md` § `run_gh — GitHub CLI Handler`

Update the paragraph about `gh api` read-only restriction:

> **`gh api` per-method gating (#1167, evolved from #805):** `gh api` is in the global subcommand allowlist but further restricted via a method+path allow matrix. Each entry defines an HTTP method + API path regex pattern. The matrix is deny-by-default: any combination not matching at least one entry is rejected by `validate_gh_api_scope()`. Matrix compiled once via `LazyLock`. Audit event `gh_api_invocation` includes `allowed_by_rule` for structured anomaly detection.

### Step 6 — Compound the solution

**File:** `docs/solutions/architecture-patterns/per-method-gh-api-gating-deny-by-default-matrix-2026-05-28.md`

Document:
- **Problem:** GET-only restriction (#805) blocks legitimate mutation use cases; unrestricted access (#788) provides no pre-hoc defense
- **Solution:** Method+path allow matrix with rule-named entries, deny-by-default, audit event enrichment
- **Why this shape:** Adding a mutation entry requires a code change (reviewed, tested, committed) — the allowlist acts as a review checkpoint. Rule names in audit events enable structured anomaly detection without parsing raw method+path.

## Files changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/builtin_handlers.rs` | Replace `GH_API_READ_ALLOWED_PATTERNS` + `GH_API_READ_COMPILED` with `GH_API_ALLOW_MATRIX` + `GhApiAllowEntry` struct + compiled `LazyLock`. Rewrite `validate_gh_api_scope()` to return matched rule name. Enrich audit event with `allowed_by_rule`. Update tests. |
| `crates/mika-agent/CLAUDE.md` | Update `run_gh` section documentation |
| `docs/solutions/architecture-patterns/per-method-gh-api-gating-deny-by-default-matrix-2026-05-28.md` | New compound doc |

## Risk assessment

**Low risk.** The change is a refactor of existing validation logic into a more extensible shape. The initial matrix carries exactly the same three GET entries as the current `GH_API_READ_ALLOWED_PATTERNS`, so runtime behavior is identical at ship time. The only behavioral change is the return-type enrichment (rule name propagation to audit events) and the error message format.

**Backward compatibility:** The error message text changes — agents that parse error messages for retry logic (none known) would see different text. The validation semantics (accept/reject decisions) are identical with the initial matrix.

**Migration path for adding mutations:** When a concrete mutation use case meets the opening criteria, the operator adds a `GhApiAllowEntry` to `GH_API_ALLOW_MATRIX` with the method, path pattern, and rule name. No other code changes required. This is the intended extension mechanism.
