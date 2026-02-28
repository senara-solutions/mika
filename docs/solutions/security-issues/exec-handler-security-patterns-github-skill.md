---
title: "Exec handler security patterns: GitHub skill implementation"
date: 2026-02-28
category: security-issues
tags:
  - exec-handler
  - security
  - github-skill
  - code-review
  - allowlist
  - env-scrubbing
severity: high
component: skills-system
related_pr: 36
---

# Exec Handler Security Patterns: GitHub Skill Implementation

## Problem Statement

Mika needed a builtin GitHub skill that wraps the `gh` CLI as an exec handler. Exec handlers spawn child processes that inherit the parent environment, creating security risks: sensitive API keys (`MIKA_ANTHROPIC_API_KEY`, `MIKA_INTERNAL_TOKEN`, etc.) leak to subprocesses, and unrestricted CLI subcommands could allow privilege escalation (`gh auth`, `gh api`, `gh config`).

The initial implementation used a blocklist approach (blocking known-dangerous subcommands) and did not scrub environment variables. Code review by the security-sentinel agent identified these as critical (P1) findings.

## Findings

### Initial Implementation Gaps

1. **Blocklist vs allowlist**: The first handler version blocked specific subcommands (`auth`, `api`, `config`). This is fragile — new `gh` subcommands could introduce attack vectors without updating the blocklist.
2. **Environment variable leakage**: `MIKA_ANTHROPIC_API_KEY`, `MIKA_INTERNAL_TOKEN`, `MIKA_OPENAI_API_KEY`, and `MIKA_BRAVE_API_KEY` were inherited by the `gh` subprocess.
3. **No checksum verification**: The Dockerfile installed `gh` without verifying the download integrity.
4. **Interactive prompt risk**: `gh` prompts for input when auth is missing, which hangs piped exec handlers.

### Security Review Agents Used

- **security-sentinel**: Identified P1 findings (allowlist, env scrubbing)
- **code-simplicity-reviewer**: Identified dead code (jq fallback, redundant auth pre-check)
- **agent-native-reviewer**: Identified keyword gaps, non-git-directory guidance
- **pattern-recognition-specialist**: Confirmed pattern consistency

## Solution

### 1. Allowlist subcommands (not blocklist)

Only 10 safe top-level subcommands are permitted:

```bash
# Allowlist of permitted top-level subcommands
SUBCOMMAND=$(printf '%s\n' "$COMMAND" | awk '{print $1}')
case "$SUBCOMMAND" in
    pr|issue|run|workflow|release|repo|search|label|milestone|project)
        ;;
    *)
        echo "Error: gh subcommand '$SUBCOMMAND' is not allowed." >&2
        exit 1
        ;;
esac
```

Blocked subcommands (by omission): `auth`, `api`, `extension`, `ssh-key`, `config`, `gpg-key`, `secret`, `variable`.

### 2. Scrub sensitive environment variables

```bash
# Scrub sensitive env vars so gh subprocesses cannot leak them
unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY
```

This runs before any `gh` invocation. Uses `unset` (not `export KEY=""`) so variables are fully removed from the process environment.

### 3. Disable interactive prompts

```bash
export GH_PROMPT_DISABLED=1
```

Prevents `gh` from blocking on stdin when auth is missing or parameters are incomplete.

### 4. No shell eval

```bash
set -f  # Disable globbing for safe word splitting

# shellcheck disable=SC2086
if [ -n "$REPO" ]; then
    gh $COMMAND --repo "$REPO" 2>&1
else
    gh $COMMAND 2>&1
fi
```

Arguments are passed directly to `gh` without `eval` or shell interpretation. `set -f` disables glob expansion. The `$REPO` parameter is always quoted and injected via `--repo` flag separately from the command string.

### 5. Checksum verification in Dockerfile

```dockerfile
RUN ARCH=$(dpkg --print-architecture) && \
    GH_VERSION="2.65.0" && \
    wget -qO /tmp/gh.tar.gz "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_${ARCH}.tar.gz" && \
    wget -qO /tmp/gh_checksums.txt "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_checksums.txt" && \
    cd /tmp && grep "gh_${GH_VERSION}_linux_${ARCH}.tar.gz" gh_checksums.txt | sha256sum -c - && \
    tar -xzf /tmp/gh.tar.gz -C /tmp && \
    mv /tmp/gh_${GH_VERSION}_linux_${ARCH}/bin/gh /usr/local/bin/gh && \
    rm -rf /tmp/gh*
```

Version is pinned and SHA256 checksum is verified before extraction.

## Prevention: Exec Handler Security Checklist

Apply these patterns when creating any new exec handler skill:

- [ ] **Allowlist operations**: Enumerate permitted subcommands in a `case` statement; reject everything else
- [ ] **Scrub env vars**: `unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY` at handler start
- [ ] **Disable globbing**: `set -f` near the top of the script
- [ ] **No eval**: Pass arguments directly to the executable, never through `eval` or `sh -c`
- [ ] **Quote variables**: Always `"$VAR"` in argument positions
- [ ] **Disable interactive prompts**: Set tool-specific env vars (e.g., `GH_PROMPT_DISABLED=1`)
- [ ] **Validate inputs early**: Check required fields, exit with clear errors
- [ ] **Verify binary integrity**: Pin versions and verify checksums in Dockerfile
- [ ] **Check tool availability**: `command -v tool >/dev/null 2>&1` with actionable error message
- [ ] **Combine stdout/stderr**: `tool $ARGS 2>&1` so errors reach the agent

## Gotchas

1. **Blocklists are fragile**: New subcommands bypass them silently. Always use allowlists.
2. **Unquoted variables cause splitting**: `gh $COMMAND --repo $REPO` breaks if `$REPO` has spaces. Always quote.
3. **`export KEY=""` is not `unset`**: An empty variable still appears in `env` output. Use `unset`.
4. **Exec handlers inherit all env vars**: The executor (`executor.rs`) only strips `TMUX`/`TMUX_PANE`. Handler-level scrubbing is defense-in-depth until executor-level scrubbing is added (see follow-up todo).

## Technical Details

- **Affected files**: `templates/skills/github/handlers/run.sh`, `Dockerfile.agent`, `crates/mika-agent/src/bundled_skills.rs`
- **Handler pattern**: Exec handler (not builtin Rust) since `gh` is an external binary
- **Skill registration**: `skill!` macro in `bundled_skills.rs` with `+x` flag for handler script

## Related Documentation

- [Skills system docs](../../skills.md) — Handler types, trigger matching, security considerations
- [Deployment docs](../../deployment.md) — Docker image build details
- [Implementation plan](../../plans/2026-02-28-feat-builtin-github-skill-plan.md) — Full feature specification
- [Follow-up todo: executor-level env scrubbing](../../../todos/359-complete-p2-github-skill-env-scrubbing-in-executor.md) — Scrub `MIKA_*` vars in `executor.rs` for all exec handlers
- [GitHub CLI docs](https://cli.github.com/) — Official `gh` reference
- PR #36 — Implementation PR

## Work Log

- 2026-02-28: Feature implemented, security review identified P1 findings, all findings resolved. Solution documented.
