# Plan: Per-Method Gating for `gh api` (mika issue#1167)

## Context

Deferred from mika issue#805 (2026-05-17) after mika issue#788's unrestricted `gh api` + audit event design was ratified. mika issue#1153 subsequently added PATCH support for milestone lifecycle operations.

**Deferral status and operator override rationale:**

The ticket body prescribed two opening criteria: (a) a prompt-injection escape observed in `gh_api_invocation` audit logs, or (b) a second use case requiring narrower gating with audit-event observability judged insufficient. Neither criterion has been formally satisfied. The ticket's "Why this is deferred" section cited #788 and called per-method gating YAGNI until evidence emerged. Vincent's 2026-06-14 reopen was about ticket state management (deferred tickets shouldn't be closed), not about the criteria being met.

This plan proceeds as an **operator-directed extensibility investment**, not because the opening criteria have been met. The operator has chosen to invest in structural extensibility now — collapsing two lists into a unified matrix — so that future method additions (when they arise) are a single-entry change rather than a branch/const/LazyLock triple. This is a conscious override of the YAGNI deferral, accepted as low-cost structural improvement (same 5 patterns, same accept/reject decisions, ~net-zero line count change). The override is documented here per review-guide.md § YAGNI: when proceeding despite unmet deferral criteria, the plan must acknowledge the deferral and state the override rationale.

**Current state (from #805 + #1153):**

The validation lives in `crates/mika-agent/src/skills/builtin_handlers.rs` and uses a **two-list** pattern — separate read and write allowlists with branching logic in `validate_gh_api_scope()`:

- `GH_API_READ_ALLOWED_PATTERNS` (4 patterns, lines ~1641-1647): `branches/{branch}`, `branches` list, `commits/{sha}`, `milestones/{number}`
- `GH_API_WRITE_ALLOWED_PATTERNS` (1 pattern, lines ~1665-1668): `milestones/{number}` (PATCH only)
- `GH_API_READ_COMPILED` / `GH_API_WRITE_COMPILED`: `LazyLock<Vec<Regex>>` compiled counterparts
- `validate_gh_api_scope()` (lines ~1914-1955): three-branch — GET→read list, PATCH→write list, all else→reject
- `extract_api_method()` (lines ~1961+): parses `--method X`, `--method=X`, `-X X` forms
- `extract_api_path()` (lines ~1981+): positional arg extraction with VALUE_FLAGS skip table
- `gh_api_invocation` audit event (lines ~2115-2129): logs `session_id`, `method`, `path` — **no `allowed_by_rule` field**
- 27 tests covering method extraction, scope validation (branches, commits, milestones GET/PATCH, rejections for POST/DELETE/disallowed paths)

**Design goals from ticket:**

1. **Deny-by-default for unsurfaced endpoints** — new `gh api` usage must pass through the allowlist
2. **Method+path discrimination** — agents that need only read access cannot accidentally mutate
3. **Audit enrichment** — `allowed_by_rule` in audit events for structured anomaly detection

**What this ticket changes:** The two-list pattern works for the current GET + PATCH surface. No concrete third-method use case is currently identified — the refactor is an operator-directed extensibility investment, not a response to immediate scaling pressure. The current shape requires a new branch in `validate_gh_api_scope()`, a new const array, a new `LazyLock`, and duplicated error messages for each additional method. The unified matrix collapses all of this into a single extensible table so that future additions (when they arise) are a single struct literal, not a code-structure change. This is accepted as low-cost structural improvement: same 5 patterns, same accept/reject decisions, ~net-zero line count.

## Approach

Replace the two-list pattern (`GH_API_READ_ALLOWED_PATTERNS` + `GH_API_WRITE_ALLOWED_PATTERNS`) with a single **method+path allow matrix** where each entry specifies:
- An HTTP method (or `"*"` wildcard)
- A regex pattern for the API path
- A human-readable rule name (for audit events + error messages)

The initial matrix carries exactly the same 5 entries (4 GET + 1 PATCH) as the current two lists, so runtime accept/reject decisions are identical at ship time. The only behavioral changes are: (1) rule-name propagation to the audit event, and (2) the error message format.

## Implementation

### Step 1 — Define the unified method+path matrix

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Remove:
- `GH_API_READ_ALLOWED_PATTERNS` const (4 entries)
- `GH_API_READ_COMPILED` LazyLock
- `GH_API_WRITE_ALLOWED_PATTERNS` const (1 entry)
- `GH_API_WRITE_COMPILED` LazyLock

Replace with:

```rust
/// Per-method gating entry for `gh api` (mika#1167, evolved from #805 + #1153).
///
/// Each entry defines an allowed method+path combination. The matrix is
/// deny-by-default: any `gh api` call whose method+path does not match
/// at least one entry is rejected by `validate_gh_api_scope()`.
struct GhApiAllowEntry {
    /// HTTP method (case-insensitive match). `"*"` matches any method.
    method: &'static str,
    /// Regex pattern for the API path (leading `/` optional, same as `gh` CLI).
    path_pattern: &'static str,
    /// Human-readable rule name for audit events and error messages.
    rule: &'static str,
}

const GH_API_ALLOW_MATRIX: &[GhApiAllowEntry] = &[
    // -- Read-only (carried forward from #805 + #1153) --
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
    GhApiAllowEntry {
        method: "GET",
        path_pattern: r"^/?repos/[^/]+/[^/]+/milestones/\\d+$",
        rule: "read:milestone",
    },
    // -- Mutations (from #1153) --
    GhApiAllowEntry {
        method: "PATCH",
        path_pattern: r"^/?repos/[^/]+/[^/]+/milestones/\\d+$",
        rule: "write:milestone-update",
    },
];
```

Compile all entries into a single `LazyLock<Vec<CompiledGhApiAllowEntry>>` where each compiled entry carries the `Regex`, the method string, and the rule name:

```rust
struct CompiledGhApiAllowEntry {
    method: &'static str,
    pattern: regex::Regex,
    rule: &'static str,
}

static GH_API_ALLOW_COMPILED: LazyLock<Vec<CompiledGhApiAllowEntry>> = LazyLock::new(|| {
    GH_API_ALLOW_MATRIX
        .iter()
        .map(|e| CompiledGhApiAllowEntry {
            method: e.method,
            pattern: regex::Regex::new(e.path_pattern).unwrap_or_else(|err| {
                panic!("BUG: malformed GH_API_ALLOW_MATRIX regex '{}': {err}", e.path_pattern)
            }),
            rule: e.rule,
        })
        .collect()
});
```

### Step 2 — Rewrite `validate_gh_api_scope()` to use the matrix

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Change the return type from `Result<(), ToolOutput>` to `Result<Option<&'static str>, ToolOutput>`. The `Ok(Some(rule))` variant carries the matched rule name for audit event enrichment (Step 3). `Ok(None)` means "not an API call" — a type-safe contract that avoids sentinel values (review-guide.md § KISS: prefer type-safe alternatives over sentinel values when available at zero cost).

```rust
fn validate_gh_api_scope(args: &[String]) -> Result<Option<&'static str>, ToolOutput> {
    if args.first().map(String::as_str) != Some("api") {
        return Ok(None);  // Not an API call — no rule applies
    }

    let method = extract_api_method(args);
    let path = extract_api_path(args);

    for entry in GH_API_ALLOW_COMPILED.iter() {
        let method_matches = entry.method == "*"
            || entry.method.eq_ignore_ascii_case(method);
        if method_matches && entry.pattern.is_match(path) {
            return Ok(Some(entry.rule));
        }
    }

    // No match — deny with structured error listing allowed rules
    Err(ToolOutput::error(format!(
        "gh api {method} '{path}' is not in the allowed method+path matrix. \
         Allowed combinations: {rules}. \
         Use the appropriate gh subcommand (e.g., gh issue, gh pr) for other operations.",
        rules = GH_API_ALLOW_COMPILED
            .iter()
            .map(|e| format!("{} {}", e.method, e.rule))
            .collect::<Vec<_>>()
            .join(", ")
    )))
}
```

Update the callsite in `run_gh()` (~line 2037) to capture the rule name:

```rust
let matched_rule = match validate_gh_api_scope(&gh_args.args) {
    Ok(rule) => rule,  // Option<&'static str>: None for non-API, Some(rule) for matched API calls
    Err(err) => return err,
};
```

### Step 3 — Enrich the audit event with `allowed_by_rule`

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs` (~lines 2115-2129)

Add `allowed_by_rule` to the `gh_api_invocation` event:

```rust
if gh_args.args.first().map(|s| s.as_str()) == Some("api") {
    let method = extract_api_method(&gh_args.args);
    let path = extract_api_path(&gh_args.args);
    // matched_rule is Option<&'static str>; inside this `if` guard it is always
    // Some(rule) because validate_gh_api_scope() returns Ok(None) only for non-API
    // calls, and we already checked args.first() == "api". Unwrap is safe here;
    // the type system enforces that None never reaches the audit event.
    let rule = matched_rule.unwrap_or("unknown");
    tracing::info!(
        event = "gh_api_invocation",
        session_id = %ctx.session_id,
        method = %method,
        path = %path,
        allowed_by_rule = %rule,
        "gh api invocation"
    );
}
```

The `allowed_by_rule` field enables structured anomaly detection: operators can group invocations by rule and flag unexpected patterns without parsing method+path combinations manually. The `Option<&'static str>` return type ensures the `None` sentinel (non-API calls) cannot silently propagate to the audit event — the `if ... == Some("api")` guard and the Option type provide a double assurance (review-guide.md § KISS).

### Step 4 — Update tests

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Update existing tests to check the `Ok` variant carries the correct rule name:

**Return-value assertions (existing tests updated):**
- `test_gh_api_get_branches_allowed` → `assert_eq!(result.unwrap(), Some("read:branch"))`
- `test_gh_api_get_branches_list_allowed` → `assert_eq!(result.unwrap(), Some("read:branches-list"))`
- `test_gh_api_get_commit_allowed` → `assert_eq!(result.unwrap(), Some("read:commit"))`
- `test_gh_api_leading_slash_allowed` → `assert_eq!(result.unwrap(), Some("read:branch"))`
- `test_gh_api_milestone_get_allowed` → `assert_eq!(result.unwrap(), Some("read:milestone"))`
- `test_gh_api_milestone_get_leading_slash_allowed` → `assert_eq!(result.unwrap(), Some("read:milestone"))`
- `test_gh_api_milestone_patch_allowed` → `assert_eq!(result.unwrap(), Some("write:milestone-update"))`
- `test_gh_api_non_api_subcommand_skipped` → `assert_eq!(result.unwrap(), None)`

**Error message updates (existing rejection tests):**
- `test_gh_api_patch_non_allowed_path_rejected` → update error string from `"not in the write allowlist"` to `"not in the allowed method+path matrix"`
- `test_gh_api_post_rejected` → update from `"is not allowed"` to `"not in the allowed method+path matrix"`
- `test_gh_api_delete_rejected` → same update
- `test_gh_api_milestone_list_not_allowed` → update from `"not in the read-only allowlist"` to `"not in the allowed method+path matrix"`
- `test_gh_api_milestone_post_rejected` → update from `"is not allowed"` to `"not in the allowed method+path matrix"`
- `test_gh_api_milestone_delete_rejected` → same
- `test_gh_api_patch_milestone_no_number_rejected` → same
- `test_gh_api_patch_non_milestone_rejected` → same
- `test_gh_api_disallowed_path_rejected` → keep as-is (just checks `is_err()`)
- `test_gh_api_arbitrary_path_rejected` → keep as-is

**New tests:**
- `test_gh_api_matrix_denies_unmatched_method_on_allowed_path` — GET path with POST method → rejected
- `test_gh_api_matrix_wildcard_method` — if a `*` method entry is added, verify it matches any HTTP method
- `test_gh_api_matrix_all_entries_compile` — iterate `GH_API_ALLOW_MATRIX`, verify each `path_pattern` compiles as valid regex (compile-time guard against copy-paste errors; runs as unit test, catches issues before `LazyLock` first-use panic)

### Step 5 — Update CLAUDE.md documentation

**File:** `crates/mika-agent/CLAUDE.md` § `run_gh — GitHub CLI Handler`

Replace the `gh api` paragraph with:

> **`gh api` per-method gating (#1167, evolved from #805 + #1153):** `gh api` is in the global subcommand allowlist but further restricted via a method+path allow matrix (`GH_API_ALLOW_MATRIX`). Each entry defines an HTTP method + API path regex + rule name. The matrix is deny-by-default: any combination not matching at least one entry is rejected by `validate_gh_api_scope()`. Initial entries: 4 GET (branch, branches-list, commit, milestone) + 1 PATCH (milestone-update). Matrix compiled once via `LazyLock`. Audit event `gh_api_invocation` includes `allowed_by_rule` for structured anomaly detection. Adding a new endpoint requires adding a `GhApiAllowEntry` to the matrix — no other code changes needed.

### Step 6 — Compound the solution

**File:** `docs/solutions/architecture-patterns/per-method-gh-api-gating-deny-by-default-matrix-YYYY-MM-DD.md`

Document:
- **Problem:** Two-list pattern (GET allowlist + PATCH allowlist) doesn't scale — each new method needs a new branch, const array, LazyLock, and error messages
- **Solution:** Unified method+path allow matrix with rule-named entries, deny-by-default, audit event enrichment via `allowed_by_rule`
- **Why this shape:** Adding a new endpoint requires only a `GhApiAllowEntry` — one struct literal, no code changes. Rule names in audit events enable structured anomaly detection. The matrix acts as a review checkpoint: every new `gh api` surface must pass code review.
- **Migration path:** When a new use case emerges (e.g., POST for issue creation), add an entry to `GH_API_ALLOW_MATRIX` with method, path pattern, and rule name.

## Files changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/builtin_handlers.rs` | Remove `GH_API_READ_ALLOWED_PATTERNS` + `GH_API_READ_COMPILED` + `GH_API_WRITE_ALLOWED_PATTERNS` + `GH_API_WRITE_COMPILED`. Add `GhApiAllowEntry` struct + `GH_API_ALLOW_MATRIX` const (5 entries) + `CompiledGhApiAllowEntry` + `GH_API_ALLOW_COMPILED` LazyLock. Rewrite `validate_gh_api_scope()` → return `Result<Option<&'static str>, ToolOutput>`. Update `run_gh()` callsite. Enrich audit event with `allowed_by_rule`. Update ~17 existing tests + add 3 new tests. |
| `crates/mika-agent/CLAUDE.md` | Update `run_gh` section — replace two-list description with matrix description |
| `docs/solutions/architecture-patterns/per-method-gh-api-gating-deny-by-default-matrix-YYYY-MM-DD.md` | New compound doc |

## Risk assessment

**Low risk.** This is a structural refactor of existing validation logic into a unified, extensible shape. The initial matrix carries exactly the same 5 patterns (4 GET + 1 PATCH) as the current two-list design, so accept/reject decisions are identical at ship time. The only behavioral changes are:

1. **Return type enrichment** — `validate_gh_api_scope()` returns `Option<&'static str>`: `Some(rule)` for matched API calls, `None` for non-API calls
2. **Audit event enrichment** — `allowed_by_rule` field added
3. **Error message format** — unified message instead of method-branched messages

**Backward compatibility:** Error message text changes. Agents that parse error messages for retry logic (none known) would see different text. No API surface changes. No schema changes.

**Extension mechanism:** Adding a new allowed endpoint requires only a `GhApiAllowEntry` in `GH_API_ALLOW_MATRIX`. No new consts, no new LazyLocks, no new branches in `validate_gh_api_scope()`.

## Revision history

- rev 2 (2026-06-26): addressed F1 by adding "Deferral status and operator override rationale" section to Context — acknowledges that neither opening criterion (a) nor (b) has been met and explicitly frames this as operator-directed extensibility investment, citing review-guide.md § YAGNI; addressed F2 by reframing "doesn't scale" motivation — removed the scaling-necessity claim, replaced with honest framing as operator-directed extensibility investment with no concrete third-method use case currently identified; addressed F3 by changing `validate_gh_api_scope()` return type from `Result<&'static str, ToolOutput>` to `Result<Option<&'static str>, ToolOutput>` — `None` replaces the `""` sentinel for non-API calls, providing a type-safe contract per review-guide.md § KISS, with updated test assertions and audit event unwrap; addressed F4 by replacing hardcoded `2026-06-26` in compound doc filename with `YYYY-MM-DD` placeholder — implementer should use actual creation date per `docs/solutions/` naming convention.
