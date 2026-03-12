---
title: Conditional Investigation Tool Registration with Config Struct
date: 2026-03-12
status: documented
category: architecture-patterns
tags: [investigation, tools, conditional-registration, github-api, config-struct]
modules:
  - mika-agent (server/investigate.rs — CreateGithubIssueTool, InvestigationToolsConfig, InvestigationParams)
  - mika-common (config.rs — github_token, github_repo Settings fields)
severity: low
symptoms:
  - Investigation agent is purely read-only with no write actions
  - Users must manually create GitHub issues after discovering bugs during investigation
  - Investigation tools initialization uses OnceCell with no way to pass config
---

## Problem

The investigation agent had no write capabilities. Users who discovered bugs during
investigation sessions had to manually create GitHub issues, losing the context
(session ID, agent ID, trace ID) that would help with debugging.

Additionally, `build_investigation_tools()` took individual parameters that triggered
clippy's `too_many_arguments` lint as the parameter count grew.

## Solution

### 1. Config struct pattern for tool builders

Instead of passing individual parameters to `build_investigation_tools()`, bundle them
into `InvestigationToolsConfig`:

```rust
struct InvestigationToolsConfig {
    http_client: reqwest::Client,
    github_token: Option<String>,
    github_repo: Option<String>,
}
```

Similarly, `InvestigationParams` bundles parameters for `run_investigation()`:

```rust
struct InvestigationParams<'a> {
    db: &'a AsyncDatabase,
    http_client: &'a reqwest::Client,
    session_id: &'a str,
    agent_id: &'a str,
    // ...
}
```

### 2. Conditional tool registration

The GitHub issue tool is only registered when both env vars are set and valid:

```rust
if let (Some(token), Some(repo)) = (config.github_token, config.github_repo)
    && !token.is_empty()
    && !repo.is_empty()
{
    // Validate owner/repo format
    if repo.matches('/').count() == 1 && !repo.starts_with('/') && !repo.ends_with('/') {
        registry.register(Box::new(CreateGithubIssueTool { ... }));
    }
}
```

The system prompt is also conditional — it only mentions GitHub issue creation when
the tool is registered (`has_github_tool` flag).

### 3. Context propagation

The investigation agent's `ToolContext` now receives real `session_id` and `trace_id`
from the investigation request (previously dummy values). This enables the GitHub
issue tool to append meaningful investigation context metadata to created issues.

## Key Decisions

- **No new crates:** Uses existing `reqwest` for GitHub API calls
- **OnceCell for tool registry:** Investigation tools are initialized once per process.
  The `has_github_tool` flag is computed at init time and passed to prompt builders.
- **Graceful degradation:** When GitHub env vars aren't set, the tool simply isn't
  registered and the system prompt doesn't mention it.
- **Input validation:** Title max 256 chars, body max 10,000 chars (matching GitHub limits).
  Repo format validated as `owner/repo`.

## Gotchas

- The `OnceCell` lazy initialization means GitHub config changes require a server restart
- The `ToolContext.session_id` and `trace_id` must be populated from the investigation
  request body, not from dummy values, for the context footer to be useful
- GitHub API error mapping should cover 401, 403, 404, 422, 429 status codes with
  human-readable messages
