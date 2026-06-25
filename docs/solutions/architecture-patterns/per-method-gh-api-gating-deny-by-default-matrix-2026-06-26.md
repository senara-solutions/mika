---
module: mika-agent/skills/builtin_handlers
tags: [gh-api, security, deny-by-default, audit, extensibility]
problem_type: architecture-pattern
category: architecture-patterns
---

# Per-Method `gh api` Gating via Deny-by-Default Matrix

**Problem:** The two-list pattern (`GH_API_READ_ALLOWED_PATTERNS` + `GH_API_WRITE_ALLOWED_PATTERNS`) required a new branch in `validate_gh_api_scope()`, a new const array, a new `LazyLock`, and duplicated error messages for each additional HTTP method. Each new method was a code-structure change, not a data change.

**Solution:** Unified method+path allow matrix (`GH_API_ALLOW_MATRIX`) where each entry is a `GhApiAllowEntry` struct with three fields: `method` (HTTP method or `"*"` wildcard), `path_pattern` (regex), and `rule` (human-readable name for audit events and error messages). The matrix is deny-by-default: any `gh api` call whose method+path does not match at least one entry is rejected.

`validate_gh_api_scope()` returns `Result<Option<&'static str>, ToolOutput>`: `Some(rule)` for matched API calls, `None` for non-API calls (type-safe sentinel instead of `""`), `Err` for rejections. The `allowed_by_rule` field in the `gh_api_invocation` audit event enables structured anomaly detection without parsing method+path combinations.

**Why this shape:** Adding a new endpoint requires only a `GhApiAllowEntry` — one struct literal, no code changes. Rule names in audit events enable structured anomaly detection (operators can group invocations by rule and flag unexpected patterns). The matrix acts as a review checkpoint: every new `gh api` surface must pass code review.

**Migration path:** When a new use case emerges (e.g., POST for issue creation), add an entry to `GH_API_ALLOW_MATRIX` with method, path pattern, and rule name. No new consts, no new `LazyLock`, no new branches in `validate_gh_api_scope()`.

**References:** mika#1167 (this change), mika#805 (original read-only allowlist), mika#1153 (PATCH support for milestones), mika#788 (unrestricted `gh api` + audit event design).
