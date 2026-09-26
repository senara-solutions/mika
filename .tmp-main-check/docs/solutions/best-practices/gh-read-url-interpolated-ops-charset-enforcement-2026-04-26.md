---
title: "gh_read URL-interpolated ops require charset enforcement on all user-controlled URL components"
date: 2026-04-26
category: best-practices
module: skills/builtin_handlers
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding a new op to gh_read that uses `gh api` with user-controlled URL components
  - Extending any builtin handler that interpolates user input into subprocess URLs
  - Reviewing security of path/ref/repo parameters passed to GitHub API calls
tags:
  - gh-read
  - url-injection
  - charset-enforcement
  - security
  - builtin-handlers
  - file-view
---

# gh_read URL-interpolated ops require charset enforcement on all user-controlled URL components

## Context

The `gh_read` handler's original four ops (`issue_view`, `pr_view`, `pr_diff`, `issue_list`) pass user input exclusively through `gh` CLI flags like `--repo` and positional arguments. The `gh` CLI handles flag parsing safely — Tokio's `Command::args` prevents shell injection, and the flag-based API doesn't expose URL construction to the caller.

When `file_view` was added (#817), it switched to `gh api /repos/{repo}/contents/{path}?ref={ref}` — a raw URL path constructed via `format!()`. This introduced a new attack surface: user-controlled values are now interpolated directly into the URL string, not passed as structured CLI flags.

## Guidance

**Every user-controlled value interpolated into a `gh api` URL must have charset enforcement.** The `path` parameter had charset enforcement from the start (architect finding #1 caught URL-decoding attacks via `%`). But `ref` initially only had leading-dash and length checks — the plan's D6 decision ("no further character class enforcement" on ref) was made without considering the URL interpolation context.

The code review caught the gap: a `ref` value like `main&foo=bar` would produce `?ref=main&foo=bar`, injecting an extra query parameter. A `ref` like `main#fragment` would truncate the URL at the fragment separator.

**Apply the same `[A-Za-z0-9._/-]` charset to all URL-interpolated parameters:**

```rust
// Charset enforcement on ref — prevents URL injection via query-string
// metacharacters. The ref is interpolated into `?ref={ref}` in the
// gh api URL — chars like `?`, `&`, `#`, `%` would corrupt the URL.
if !r
    .chars()
    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
{
    return Err(ToolOutput::error(
        GhReadError::MalformedRequest(format!(
            "'ref' contains disallowed characters. Only [A-Za-z0-9._/-] are permitted: '{r}'."
        ))
        .to_json(),
    ));
}
```

**Defense ordering matters.** For `path`, three checks run in sequence:
1. No leading `-` (anti-flag-smuggling)
2. No leading `/` (repo-root-relative)
3. No `..` (anti-traversal, defense-in-depth)
4. Charset `[A-Za-z0-9._/-]` (primary defense — rejects `%`, space, shell metacharacters)

The `..` check is defense-in-depth — the charset check is the strong guard. Without charset enforcement, `foo%2F..%2Fbaz` bypasses the literal `..` check because `%2F` only decodes to `/` server-side after GitHub's URL decoder runs.

## Why This Matters

The distinction between "flag-based CLI ops" and "URL-interpolated API ops" is invisible at the `gh_read` dispatch level — both are ops in the same handler, validated by the same function, using the same `GhReadArgs` struct. A future developer adding a sixth op might not realize that URL-interpolated ops need stricter validation than flag-based ops.

The `repo` parameter is also interpolated into the file_view URL path but uses the same pre-existing validation as the four flag-based ops (non-empty, no leading `-`). This is a known gap: a `repo` value like `owner/repo?foo=bar` passes validation and corrupts the URL. It's deferred because repo validation applies uniformly to all five ops (the plan explicitly deferred per-op asymmetric validation).

## When to Apply

- Adding any new `gh_read` op that uses `gh api` instead of `gh <subcommand>`
- Adding user-controlled parameters to existing `gh api` URL templates
- Reviewing or extending the `validate_gh_read_input` function
- Building any new builtin handler that constructs subprocess URLs from user input

## Examples

**Before (ref with only flag-smuggling check — vulnerable to URL injection):**

```rust
if let Some(ref r) = r#ref {
    if r.starts_with('-') {
        return Err(/* ... */);
    }
    if r.len() > 256 {
        return Err(/* ... */);
    }
    // No charset check — ref="main&foo=bar" passes and injects into URL
}
```

**After (ref with charset enforcement — URL-safe):**

```rust
if let Some(ref r) = r#ref {
    if r.starts_with('-') {
        return Err(/* ... */);
    }
    if r.len() > 256 {
        return Err(/* ... */);
    }
    // Charset enforcement: prevent URL injection
    if !r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-')) {
        return Err(/* ... */);
    }
}
```

**The `--repo` refactor pattern (per-op flag responsibility):**

When an op uses `gh api` (URL-based) instead of `gh <subcommand>` (flag-based), the `--repo` flag must not be appended. The refactor moves `--repo` into each op's arm of `build_gh_read_command` so the dispatch site doesn't need per-op exceptions:

```rust
// Each op arm now owns its --repo responsibility
"issue_view" => {
    vec!["issue", "view", target, "--json", fields, "--repo", repo]
}
"file_view" => {
    // gh api takes repo in URL path, not as --repo flag
    vec!["api", format!("/repos/{repo}/contents/{path}?ref={ref}"), "--method", "GET"]
}
```

## Related

- senara-solutions/mika#817 — file_view op implementation
- senara-solutions/mika#811 / PR #813 — original gh_read with four flag-based ops
- `docs/architecture/review-guide.md` §6 — citation-or-silence discipline this op enables
- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — flagged the tool-surface limit
