# Plan: feat(mika-prime): minimal gh-read-only skill replacing github in allowlist

**Ticket:** mika issue#1406
**Type:** Enhancement (privilege-correctness)
**Branch:** `feat/1406/mika-prime-minimal-gh-read-only-skill`

## Problem

Mika Prime's bearing-keeper role is read-only by design — she points, the hand strikes. Her current allowlist includes the `github` skill which provides `run_gh` (full GitHub read+write access). Over-privilege: correctly grounded with too-broad capability. The read-only `gh_read` builtin already exists and is used by all three architect skills (`mika-arch-groom-ticket`, `mika-arch-second-review`, `mika-arch-groom-milestone`). Same pattern fits a bearing-keeper.

## Solution overview

Three changes, one PR:

1. **New bundled skill `gh-read-only`** — exposes only `gh_read` (read-only builtin), keyword-matched to GitHub-read intents.
2. **Mika Prime identity allowlist swap** — document the manual runtime change: remove `github`, add `gh-read-only`. (Mika Prime is operator-provisioned, not a well-known agent in `well_known_agents.rs`.)
3. **Critical AC: bearing skill `required_tools` swap** — coordinate with sibling mika issue#1405 to ensure the bearing skill's `required_tools` uses `gh_read` instead of `run_gh`.

## Detailed steps

### Step 1 — Create `skills/bundled/gh-read-only/skill.toml`

```toml
[skill]
name = "gh-read-only"
description = "Read-only GitHub access — view issues, PRs, diffs, and issue lists without write capability."
version = "0.1.0"
always_on = false
timeout_secs = 60

[triggers]
keywords = [
    "issue", "issues", "pr", "pull request", "pull requests",
    "github", "gh", "repo", "repository",
    "view issue", "view pr", "check issue", "check pr",
    "list issues", "pr diff", "issue list",
]

[constraints]
required_tools = ["gh_read"]
```

**Design decisions:**
- `always_on = false` — matches the architect skills' pattern. Read-only GitHub access is only needed when the turn involves GitHub-related intents.
- `required_tools = ["gh_read"]` — structural enforcement that the tool is called when the skill keyword-matches, same pattern as the architect skills.
- Keywords cover the bearing-keeper's primary GitHub read scenarios: viewing issues/PRs for operational state assessment.

### Step 2 — Create `skills/bundled/gh-read-only/tools.json`

Copy the `gh_read` tool definition from `skills/bundled/mika-arch-groom-ticket/tools.json` verbatim. This is the same builtin handler — the skill just provides a different keyword-trigger surface.

```json
[
  {
    "name": "gh_read",
    "description": "Read-only GitHub CLI operations. Supports: issue_view (view issue details), pr_view (view PR details), pr_diff (get PR diff), issue_list (list issues with optional milestone/label filter).",
    "input_schema": {
      "type": "object",
      "properties": {
        "op": {
          "type": "string",
          "enum": ["issue_view", "pr_view", "pr_diff", "issue_list"],
          "description": "The read operation to perform."
        },
        "target": {
          "type": "string",
          "description": "Issue/PR number for view/diff ops, or milestone number/label name for issue_list filter. Optional for issue_list."
        },
        "repo": {
          "type": "string",
          "description": "Repository in owner/repo format (e.g., 'senara-solutions/mika')."
        }
      },
      "required": ["op", "repo"]
    },
    "handler": {
      "type": "builtin",
      "function": "gh_read"
    }
  }
]
```

**Note:** `file_view` is intentionally excluded from the `op` enum. The bearing-keeper reads issues, PRs, and diffs — not source files. The builtin handler supports `file_view` but the skill-scoped tool definition limits the surface. If Prime needs file viewing later, the enum can be extended.

### Step 3 — Create `skills/bundled/gh-read-only/system_prompt.md`

Minimal system prompt for read-only GitHub operations:

```markdown
# GitHub Read-Only Access

You have read-only access to GitHub via the `gh_read` tool. Use it to:

- **View issues** (`issue_view`): Read issue details — title, body, labels, state, comments.
- **View PRs** (`pr_view`): Read pull request details — title, body, review state, checks.
- **View PR diffs** (`pr_diff`): Read the code diff for a pull request.
- **List issues** (`issue_list`): List issues filtered by milestone or label.

Always specify the `repo` parameter in `owner/repo` format.

You cannot create, edit, close, or comment on issues or PRs. You cannot merge PRs, add labels, or modify any GitHub state. For write operations, the appropriate agent with full GitHub access must handle the request.
```

### Step 4 — No engine changes required

The `gh_read` builtin handler already exists at `crates/mika-agent/src/skills/builtin_handlers.rs`. The skill is discovered at build time by `build.rs` walking `skills/bundled/`. No changes to `well_known_agents.rs`, `tools/mod.rs`, or any Rust code.

### Step 5 — Build verification

```bash
cargo build  # Verifies build.rs discovers the new skill
cargo test -p mika-agent  # Run tests to check no regressions
```

### Step 6 — Mika Prime identity allowlist swap (runtime, documented)

Mika Prime is operator-provisioned (not in `well_known_agents.rs`). The allowlist change is a manual runtime operation:

**In `~/.mika/agents/mika-prime/identity.toml`:**
```diff
 [skills]
 allowlist = [
-    "github",
+    "gh-read-only",
     # ... other skills unchanged
 ]
```

This step is documented in the PR description. The operator applies it after merge + deploy. The bundled skill becomes available via `seed_bundled_skills()` on next `make deploy`; the allowlist swap activates it for Prime.

### Step 7 — Critical AC: bearing skill `required_tools` coupling

Sibling mika issue#1405 introduces `skills/bundled/bearing/skill.toml` with `required_tools = ["run_gh", "search_memory", "query_knowledge_graph"]`. When this PR removes `github` from Prime's allowlist, `run_gh` ceases to be in her resolved tool surface. The #516 availability filter then silently drops `run_gh` from the required-tools enforcement — the gate passes vacuously.

**Sequencing:**
- **If #1405 lands first:** This PR modifies `skills/bundled/bearing/skill.toml` to swap `required_tools` from `["run_gh", ...]` to `["gh_read", ...]`.
- **If this PR lands first:** The bearing skill doesn't exist yet. Document in the PR description that #1405 MUST use `gh_read` (not `run_gh`) in its `required_tools`, since `github` is no longer in Prime's allowlist. Add a code comment or PR cross-reference.
- **Same-PR (preferred):** Both changes land together. This is the ticket's stated preference ("Land-coupling is mandatory").

**Implementation:** Check at implementation time whether `skills/bundled/bearing/` exists. If it does, modify it. If not, note the dependency in the PR description.

## Files changed

| File | Action | Description |
|------|--------|-------------|
| `skills/bundled/gh-read-only/skill.toml` | Create | Skill manifest with keyword triggers and `required_tools = ["gh_read"]` |
| `skills/bundled/gh-read-only/tools.json` | Create | `gh_read` tool definition (same as architect skills) |
| `skills/bundled/gh-read-only/system_prompt.md` | Create | Minimal read-only GitHub usage instructions |
| `skills/bundled/bearing/skill.toml` | Modify (conditional) | Swap `run_gh` → `gh_read` in `required_tools` (only if #1405 has landed) |

## Out of scope

- Making mika-prime a well-known agent in `well_known_agents.rs` — she is operator-provisioned.
- Removing `git-ops` from Prime's allowlist — separate concern (git-ops covers local repo reads).
- Tightening other Prime skills (`web-search`, `shell-exec`, `file-reader`) — separate concern.
- Changes to the architect skills' `gh_read` usage — unchanged.
- Engine changes (`agent.rs`, tool registry) — the builtin handler already exists.

## Risks

- **Vacuous-pass on bearing skill (Critical AC):** If the bearing skill ships with `run_gh` in `required_tools` and `github` is removed from the allowlist, the grounding gate passes vacuously. Mitigated by same-PR coupling or mandatory sequencing documentation.
- **Keyword overlap:** `gh-read-only` keywords overlap with the `github` template skill's triggers. No conflict — they won't both be in Prime's allowlist. Other agents using `github` are unaffected (different allowlists).
- **Missing `file_view` op:** Deliberately excluded. If Prime needs it later, extend the tools.json enum — additive, no breaking change.
