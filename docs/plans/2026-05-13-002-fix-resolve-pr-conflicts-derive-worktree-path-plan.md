# Plan: resolve_pr_conflicts derives worktree_path from task context

**Issue:** mika issue#783
**Type:** fix
**Branch:** fix/783/resolve-pr-conflicts-should-derive

## Problem

`resolve_pr_conflicts` requires `worktree_path` as an absolute path the LLM must construct. The canonical sanitization rule (`/` → `-` in branch-derived directory names) lives in `mika-platform/scripts/derive-worktree-path`, not in the tool's schema or prompt. LLMs routinely get this wrong — mika-dev passed slash-separated paths (`feat/286/...`) instead of dash-separated (`feat-286-...`) on two consecutive retries (mika#286 / PR #782), causing handler crashes before any work began.

**Root cause:** LLMs should not construct slugged/sanitized paths. The tool has everything it needs to derive the path itself.

## Architect Review History

- **First-pass (ITERATE):** session `b7600878-0471-4280-a64c-c6999ddec8a0`
  - F1 (BLOCKING): Phase 0 Pin — no verbatim source for modified interfaces
  - F2 (BLOCKING): Tier ordering priority inversion — DB lookup should precede network call
  - F3 (BLOCKING): sqlite3 handler-to-DB coupling — no precedent; boundary crossing
  - **Resolution for F2+F3:** Drop DB tier entirely. No handler currently reads `mika.db` directly (verified: only `self-check/system_prompt.md` mentions sqlite3, as an LLM instruction, not a handler dependency). Two-tier design: `pr_url` derivation → legacy `worktree_path` fallback. Simpler, no new coupling.

## Phase 0 Pin — Verbatim Source

### Current `tools.json` schema

```json
[
  {
    "name": "resolve_pr_conflicts",
    "description": "Resolve merge conflicts on a PR branch by rebasing onto the base branch. Long-running — returns a task ID immediately, results arrive via callback when claude-pilot finishes.",
    "input_schema": {
      "type": "object",
      "properties": {
        "worktree_path": {
          "type": "string",
          "description": "Absolute path to the existing git worktree for the PR branch. The worktree must already exist (created by self-dev or manually)."
        },
        "task_id": {
          "type": "string",
          "description": "The UUID returned by create_task (36-char format like '15383984-a3e7-41bf-ac6f-630ba9a89d63'). Used for log filename and task correlation at /var/log/claude-pilot/{task_id}.log."
        }
      },
      "required": ["worktree_path", "task_id"]
    },
    "handler": {
      "type": "exec",
      "command": "handlers/run.sh",
      "long_running": true,
      "estimated_duration_secs": 600
    }
  }
]
```

### Current `run.sh` handler structure (relevant excerpt)

Lines 69-100 — worktree_path consumption:
```sh
# Parse user-provided fields
WORKTREE_PATH=$(printf '%s\n' "$INPUT" | jq -r '.worktree_path // empty')
USER_TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.task_id // empty')

# mika-platform root
PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
PLATFORM_DIR=$(cd "$PLATFORM_DIR" 2>/dev/null && pwd -P) || ...

if [ -z "$WORKTREE_PATH" ]; then
    echo "Error: worktree_path is required" >&2
    exit 1
fi

# Validate worktree_path is under the expected worktree root
CANONICAL_WORKTREE=$(cd "$WORKTREE_PATH" 2>/dev/null && pwd -P) || CANONICAL_WORKTREE="$WORKTREE_PATH"
EXPECTED_PREFIX="${PLATFORM_DIR}/.claude/worktrees/"
case "$CANONICAL_WORKTREE" in
    "$EXPECTED_PREFIX"*) ;; # OK
    *) echo "Error: worktree_path must be under $EXPECTED_PREFIX" >&2; exit 1 ;;
esac

# Validate worktree_path is a git working tree
if ! git -C "$WORKTREE_PATH" rev-parse --git-dir >/dev/null 2>&1; then
    echo "Error: worktree_path '$WORKTREE_PATH' is not a valid git working tree" >&2
    exit 1
fi
```

### `derive-worktree-path` script interface

From `mika-platform/scripts/derive-worktree-path`:
- **Arguments:** `--branch <ref>` (required), `--repo <name>` (required unless `--no-repo`), `--no-repo` (omit repo suffix)
- **Slug computation:** `slug=$(printf '%s' "$BRANCH" | tr '/' '-')` — the canonical `/` → `-` sanitization
- **Output:** `<PLATFORM>/.claude/worktrees/<slug>/<repo>` (absolute path)
- Example: `--branch feat/286/team-orchestrator --repo mika` → `/home/samidarko/workspace/mika-platform/.claude/worktrees/feat-286-team-orchestrator/mika`

### Sibling `address-pr-comments` pattern (for reference)

`address-pr-comments/tools.json` requires all three: `pr_url`, `worktree_path`, `task_id`. Its handler parses repo + PR number from `pr_url` at lines 115-125. This is the parsing pattern we'll reuse.

## Approach

Two-tier resolution: `pr_url` derivation (primary) → explicit `worktree_path` (deprecated fallback). No DB tier — preserves the handler/storage boundary (no existing handler reads `mika.db` directly).

**Schema change:**
- `required`: `["task_id"]` (was `["worktree_path", "task_id"]`)
- Add `pr_url` (optional) — when provided, handler derives worktree path
- Keep `worktree_path` (optional, deprecated) — backward-compatible fallback
- At least one of `pr_url` or `worktree_path` must be present (handler validates)

**Handler derivation (when `pr_url` provided):**
1. Parse `REPO_FULL` (e.g., `senara-solutions/mika`) and `PR_NUMBER` from URL
2. `REPO_SHORT` = basename (e.g., `mika`)
3. `BRANCH` = `gh pr view $PR_NUMBER --repo $REPO_FULL --json headRefName -q .headRefName`
4. `DERIVED_PATH` = `$PLATFORM_DIR/scripts/derive-worktree-path --branch $BRANCH --repo $REPO_SHORT`
5. If both `pr_url` and `worktree_path` provided: validate they match (log mismatch at warn level, reject with clear error)
6. Use `DERIVED_PATH` as the working worktree path

## Files to Change

### 1. `skills/bundled/resolve-pr-conflicts/tools.json`

New schema:

```json
[
  {
    "name": "resolve_pr_conflicts",
    "description": "Resolve merge conflicts on a PR branch by rebasing onto the base branch. Long-running — returns a task ID immediately, results arrive via callback when claude-pilot finishes. Pass pr_url and the handler derives the worktree path automatically — do NOT construct worktree paths manually.",
    "input_schema": {
      "type": "object",
      "properties": {
        "task_id": {
          "type": "string",
          "description": "The UUID returned by create_task (36-char format like '15383984-a3e7-41bf-ac6f-630ba9a89d63'). Used for log filename and task correlation at /var/log/claude-pilot/{task_id}.log."
        },
        "pr_url": {
          "type": "string",
          "description": "Full GitHub PR URL (e.g., 'https://github.com/senara-solutions/mika/pull/42'). Handler derives the worktree path from the PR's branch name. Preferred over worktree_path."
        },
        "worktree_path": {
          "type": "string",
          "description": "DEPRECATED — use pr_url instead. Absolute path to the existing git worktree for the PR branch. If both pr_url and worktree_path are provided, they must resolve to the same path."
        }
      },
      "required": ["task_id"]
    },
    "handler": {
      "type": "exec",
      "command": "handlers/run.sh",
      "long_running": true,
      "estimated_duration_secs": 600
    }
  }
]
```

### 2. `skills/bundled/resolve-pr-conflicts/handlers/run.sh`

Changes to the handler (insertions between the field-parsing block and the validation block):

**A. Parse new `pr_url` field** (after line 70):
```sh
PR_URL=$(printf '%s\n' "$INPUT" | jq -r '.pr_url // empty')
```

**B. Add `derive_worktree_from_pr()` function** (before the validation block):
```sh
derive_worktree_from_pr() {
    # Parse repo and PR number from URL
    # Pattern: https://github.com/{owner}/{repo}/pull/{number}
    PR_NUMBER=$(printf '%s' "$PR_URL" | sed -n 's|.*/pull/\([0-9]*\).*|\1|p')
    REPO_FULL=$(printf '%s' "$PR_URL" | sed -n 's|https://github.com/\([^/]*/[^/]*\)/pull/.*|\1|p')

    if [ -z "$PR_NUMBER" ] || [ -z "$REPO_FULL" ]; then
        echo "Error: could not parse PR number and repo from pr_url '$PR_URL'" >&2
        return 1
    fi

    REPO_SHORT=$(basename "$REPO_FULL")

    # Get branch name from PR
    BRANCH=$(gh pr view "$PR_NUMBER" --repo "$REPO_FULL" --json headRefName -q .headRefName 2>/dev/null)
    if [ -z "$BRANCH" ]; then
        echo "Error: could not get branch name from PR $PR_NUMBER in $REPO_FULL" >&2
        return 1
    fi

    # Derive worktree path using canonical script
    DERIVED_PATH=$("$PLATFORM_DIR/scripts/derive-worktree-path" --branch "$BRANCH" --repo "$REPO_SHORT")
    if [ -z "$DERIVED_PATH" ]; then
        echo "Error: derive-worktree-path returned empty path for branch '$BRANCH' repo '$REPO_SHORT'" >&2
        return 1
    fi

    printf '%s' "$DERIVED_PATH"
}
```

**C. Replace the hard `worktree_path is required` check** with two-tier resolution:
```sh
# Two-tier worktree path resolution
if [ -n "$PR_URL" ]; then
    DERIVED_PATH=$(derive_worktree_from_pr)
    if [ $? -ne 0 ]; then
        # derive_worktree_from_pr already printed error to stderr
        exit 1
    fi

    # Mismatch validation (defense-in-depth)
    if [ -n "$WORKTREE_PATH" ]; then
        CANONICAL_DERIVED=$(cd "$DERIVED_PATH" 2>/dev/null && pwd -P) || CANONICAL_DERIVED="$DERIVED_PATH"
        CANONICAL_EXPLICIT=$(cd "$WORKTREE_PATH" 2>/dev/null && pwd -P) || CANONICAL_EXPLICIT="$WORKTREE_PATH"
        if [ "$CANONICAL_DERIVED" != "$CANONICAL_EXPLICIT" ]; then
            echo "WARN: derived worktree path ($DERIVED_PATH) != explicit worktree_path ($WORKTREE_PATH). Using derived path." >&2
        fi
    fi

    WORKTREE_PATH="$DERIVED_PATH"
elif [ -z "$WORKTREE_PATH" ]; then
    echo "Error: either pr_url or worktree_path is required" >&2
    exit 1
fi
```

**D. Rest of the handler is unchanged** — the existing validation (prefix check, git working tree check), relay config copy, and claude-pilot spawn all work on `WORKTREE_PATH` as before.

### 3. `skills/bundled/resolve-pr-conflicts/system_prompt.md`

Update the Inputs table and example:

```markdown
### Inputs

| Field | Required | Description |
|-------|----------|-------------|
| `task_id` | Yes | UUID from `create_task` for log correlation |
| `pr_url` | Preferred | Full GitHub PR URL — handler derives the worktree path from the PR's branch name |
| `worktree_path` | Deprecated | Absolute path to the existing git worktree. Use `pr_url` instead. |

At least one of `pr_url` or `worktree_path` must be provided. When `pr_url` is given, the handler derives the correct worktree path automatically using the canonical branch-to-path sanitization rule.
```

Update the example:
```
resolve_pr_conflicts(
  task_id: "15383984-a3e7-41bf-ac6f-630ba9a89d63",
  pr_url: "https://github.com/senara-solutions/mika/pull/42"
)
```

## Non-Goals

- Changing `address-pr-comments` (same pattern but lower severity — it already requires `pr_url`, so the LLM has PR context; file follow-up ticket at p3)
- Direct DB access from handlers (no precedent; preserves handler/storage boundary)
- Removing `worktree_path` entirely (deprecated fallback for backward compatibility)

## Testing

1. **Smoke test:** Call with `task_id` + `pr_url` → verify handler derives correct path and runs
2. **Mismatch detection:** Provide `pr_url` + explicit wrong `worktree_path` → verify WARN logged and derived path used
3. **Legacy path:** Call with `task_id` + `worktree_path` (no `pr_url`) → verify existing behavior preserved
4. **Missing both:** Call with only `task_id` → verify clean error message
5. **Bad PR URL:** Call with malformed `pr_url` → verify structured error (not crash)
6. **Closed/merged PR:** Call with `pr_url` for a merged PR → verify `gh pr view` still returns branch name (it does — GitHub preserves headRefName)

## Risk Assessment

- **Low risk:** Shell script changes only, no Rust code. Schema is backward-compatible.
- **Network dependency:** `gh pr view` requires GitHub API access. This is already true for the rebase prompt inside claude-pilot (`gh pr list` at line 124 of the existing handler). No new external dependency.
- **Race condition:** Between `gh pr view` and the rebase, the PR could be closed/merged. Pre-existing (handler's claude-pilot session already handles this).
