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
- `timeout_secs = 60` — **timeout model clarification:** Skill timeouts are per-skill-match, not per-tool-call. When the bearing skill (#1405, `timeout_secs = 30`) keyword-matches and its `required_tools` includes `gh_read`, the bearing skill's 30s timeout governs that turn — not this skill's 60s. The `gh-read-only` skill's 60s timeout only applies when `gh-read-only` itself keyword-matches directly (i.e., a turn where the user's intent triggers the `gh-read-only` keywords). 60s is appropriate for direct keyword-match scenarios where the agent may issue multiple sequential `gh_read` calls (e.g., listing issues then viewing several). *Citation: review-guide.md § KISS — commit to the timeout model.*

### Step 2 — Create `skills/bundled/gh-read-only/tools.json`

Copy the `gh_read` tool definition from `skills/bundled/mika-arch-groom-ticket/tools.json`. **Implementation-time instruction:** Before copying, verify the current content of `skills/bundled/mika-arch-groom-ticket/tools.json` matches the snapshot below. If the source has changed since this plan was written, use the **current** version from the source file — it is the canonical `gh_read` definition.

*Citation: review-guide.md § DRY — avoid divergent copies of the same definition.*

The snapshot at plan-write time (verified 2026-06-26):

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

### Step 6 — Mika Prime identity allowlist swap (operator post-deploy step)

Mika Prime is operator-provisioned (not in `well_known_agents.rs`). Her `~/.mika/agents/mika-prime/identity.toml` is not version-controlled — it lives only on the runtime host. Therefore, the PR **cannot contain the actual file change**. The allowlist swap is an **operator post-deploy step**, documented in the PR description as a required manual action after merge + deploy.

**PR description must include this operator instruction:**

> **Required post-deploy action:** In `~/.mika/agents/mika-prime/identity.toml`, swap `github` → `gh-read-only` in the `[skills].allowlist` array. The new bundled skill is available after `make deploy` runs `seed_bundled_skills()`. Until this swap is applied, Prime retains full `run_gh` write access.

This is the only viable delivery mechanism — there is no seed/template file for Prime's identity in the repo. The PR satisfies the ticket's AC (a) by shipping the new `gh-read-only` skill itself; the allowlist swap is the operator's responsibility, explicitly called out rather than silently assumed.

*Citation: Unresolved-Decision Gate (mika#1244) — mechanism resolved: operator post-deploy step, not a repo-tracked file change.*

### Step 7 — Critical AC: bearing skill `required_tools` coupling (mandatory land-coupling)

Sibling mika issue#1405 introduces `skills/bundled/bearing/skill.toml` with `required_tools = ["run_gh", "search_memory", "query_knowledge_graph"]`. When this PR removes `github` from Prime's allowlist, `run_gh` ceases to be in her resolved tool surface. The #516 availability filter then silently drops `run_gh` from the required-tools enforcement — the gate passes vacuously.

**The ticket's Critical AC is absolute:** *"The PR is rejected if it does not contain both: (a) the new `gh-read-only` skill + allowlist swap, AND (b) the `required_tools` swap on the bearing skill. Land-coupling is mandatory."*

**Implementation decision (two branches, no fallback):**

1. **If `skills/bundled/bearing/skill.toml` exists at implementation time:** Modify it in this PR — swap `run_gh` → `gh_read` in `required_tools`. This satisfies the land-coupling mandate directly.

2. **If `skills/bundled/bearing/skill.toml` does not exist at implementation time:** **ESCALATE to the operator.** The implementer cannot satisfy AC (b) without the file that #1405 creates. The PR must not ship without the coupling — documentation of a dependency is not enforcement, and the ticket explicitly rejects that path. The escalation surfaces the sequencing conflict for operator resolution (e.g., land #1405 first, or combine both tickets into one PR).

There is no third path. "Document the dependency and ship without the swap" is explicitly removed — the ticket says "mandatory," not "preferred."

*Citation: review-guide.md § YAGNI — the ticket is the spec; the plan cannot unilaterally weaken a stated AC.*

## Files changed

| File | Action | Description |
|------|--------|-------------|
| `skills/bundled/gh-read-only/skill.toml` | Create | Skill manifest with keyword triggers and `required_tools = ["gh_read"]` |
| `skills/bundled/gh-read-only/tools.json` | Create | `gh_read` tool definition (same as architect skills) |
| `skills/bundled/gh-read-only/system_prompt.md` | Create | Minimal read-only GitHub usage instructions |
| `skills/bundled/bearing/skill.toml` | Modify (if exists) or ESCALATE | Swap `run_gh` → `gh_read` in `required_tools`. If file does not exist, ESCALATE to operator — do not ship without the coupling. |

## Acceptance criteria

- [ ] AC1: New bundled skill `gh-read-only` is loaded into the engine with read-only `gh_read` as its sole tool (verified via `cargo test` covering bundled-skill discovery + tool enumeration).
- [ ] AC2: Mika Prime's deployed `identity.toml` allowlist removes `github` and adds `gh-read-only`. Documented as an operator post-deploy step in the PR description (identity.toml is not version-controlled per mika#1244).
- [ ] AC3 (Critical, land-coupling): The bearing skill (`bearing/skill.toml`) `required_tools` is swapped from `run_gh` to `gh_read` IN THIS SAME PR. If `bearing/skill.toml` does not exist on disk at implementation time, ESCALATE to operator rather than ship — no "document and skip" fallback (mika#1405 vacuous-pass risk).
- [ ] AC4: `gh-read-only` keyword set is keyword-matched to GitHub-read intents (`gh`, `issue`, `pr`, `repo`) — pattern mirrors `mika-arch-groom-ticket`. Validated via `validate_skill()` at load.
- [ ] AC5: All existing `mika-arch-*` skills continue to declare `required_tools = ["gh_read"]` — no regression on the architect agents.


## Out of scope

- Making mika-prime a well-known agent in `well_known_agents.rs` — she is operator-provisioned.
- Removing `git-ops` from Prime's allowlist — separate concern (git-ops covers local repo reads).
- Tightening other Prime skills (`web-search`, `shell-exec`, `file-reader`) — separate concern.
- Changes to the architect skills' `gh_read` usage — unchanged.
- Engine changes (`agent.rs`, tool registry) — the builtin handler already exists.

## Risks

- **Vacuous-pass on bearing skill (Critical AC):** If the bearing skill ships with `run_gh` in `required_tools` and `github` is removed from the allowlist, the grounding gate passes vacuously. Mitigated by mandatory land-coupling: if `bearing/skill.toml` exists, this PR modifies it; if it does not exist, implementation ESCALATES to operator rather than shipping without the coupling.
- **Keyword overlap:** `gh-read-only` keywords overlap with the `github` template skill's triggers. No conflict — they won't both be in Prime's allowlist. Other agents using `github` are unaffected (different allowlists).
- **Missing `file_view` op:** Deliberately excluded. If Prime needs it later, extend the tools.json enum — additive, no breaking change.

## Revision history

- rev 2 (2026-06-26): addressed F1 by removing the three-way conditional sequencing in Step 7 and committing to a two-branch decision: modify `bearing/skill.toml` if it exists, or ESCALATE to operator if it does not — no "document and ship" fallback (review-guide.md § YAGNI); addressed F2 by explicitly resolving the `identity.toml` delivery mechanism — the file is not version-controlled, so the PR cannot contain the actual change; the allowlist swap is an operator post-deploy step, documented in the PR description (Unresolved-Decision Gate mika#1244); addressed F3 by adding an implementation-time verification instruction to Step 2 requiring the implementer to check the current `mika-arch-groom-ticket/tools.json` content before copying, using the current version if it has drifted (review-guide.md § DRY); addressed F4 by documenting the timeout model in Step 1's design decisions — skill timeouts are per-skill-match, so the bearing skill's 30s governs when bearing keyword-matches, and `gh-read-only`'s 60s governs only direct keyword-matches (review-guide.md § KISS).
