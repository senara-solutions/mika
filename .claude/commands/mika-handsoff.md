---
name: mika-handsoff
description: Write/extend today's handsoff log per HANDSOFF-CONTRACT, commit it, halt before push, emit terminal handsoff block
argument-hint: "[optional one-line focus]"
---

This command operationalizes `docs/logs/HANDSOFF-CONTRACT.md` (mika-platform#80) for operator-Claude in conversational mode. Format, merge, commit, and push specs live in the contract — this command does not duplicate them. Read the contract before first use.

## Phase 0 — Collect spawned session completions

**Guard:** Skip this phase entirely if:
- `MIKA_SPAWN_ID` environment variable is set (this session is itself a spawned tenant — do not consume the orchestrator's inbox)
- `~/.mika/orchestrator/inbox/` does not exist or contains no `*.json` files

**When inbox has entries:**

1. Read each `*.json` file in `~/.mika/orchestrator/inbox/`. For each file:
   - Parse as JSON. If parsing fails (malformed), log a warning to the operator (`⚠ Skipping malformed inbox entry: <filename>`) and skip that entry.
   - Extract: `spawn_id`, `summary`, `log_path`, `branch`, `pushed`, `completed_at`.
2. Collect all successfully parsed entries into a **"Spawned sessions completed since last handsoff"** section. This section feeds into Phase 2 synthesis — include it in the handsoff log body so the operator sees what spawned tenants finished. Format as one line per entry:
   ```
   - `<spawn_id>`: <summary> | branch: `<branch>` (pushed: <yes/no>) | log: `<log_path>` | completed: <completed_at>
   ```
3. Archive processed entries (both valid and malformed — malformed entries should not block future runs):
   ```bash
   mkdir -p ~/.mika/orchestrator/inbox/archive/
   mv ~/.mika/orchestrator/inbox/*.json ~/.mika/orchestrator/inbox/archive/
   ```

## Phase 1 — Locate today's log

### Workspace-detection guard

Before any file operations, verify `mika-platform/docs/logs/` is reachable. Determine the meta-repo root:
- If cwd is inside the mika-platform repo (check `git -C . rev-parse --show-toplevel` and verify basename is `mika-platform`), use that as the root.
- Otherwise, check `../mika-platform/`, `../../mika-platform/`, or the workspace root from `CLAUDE_PROJECT_ROOT`.

If `docs/logs/` cannot be located, **halt immediately** with:

> Cannot locate mika-platform workspace from current directory. Invoke from inside the mika-platform workspace.

Do not proceed.

> **Sync warning:** This subsection mirrors `.claude/commands/mika-onboarding.md` Phase 1 § Workspace-detection guard. Update both together when the heuristic changes.

### Locate the file

```bash
TODAY=$(date +%Y-%m-%d)
```

Search for `docs/logs/$TODAY - *.md` (use Glob or `ls`).

- **No match** → new file. Proceed to Phase 3 for slug derivation.
- **Single match** → continuation. Read the existing file and apply HANDSOFF-CONTRACT merge rules (§ Merge rules) in Phase 2.
- **Multiple matches** → **halt as bug** per HANDSOFF-CONTRACT hard rule #3. Surface the file list to the operator and stop. Do not consolidate.

### Detached-HEAD guard

Before proceeding to synthesis or operator questions, verify HEAD is not detached:
```bash
git -C <meta-repo-root> symbolic-ref --short HEAD 2>/dev/null
```

If this fails (non-zero exit), HEAD is detached. **Halt the entire flow** — do not synthesize, do not ask questions, do not write any file. Surface to the operator:

> mika-platform meta-repo HEAD is detached. Commits would be unreachable. Checkout a branch (e.g., `git -C mika-platform checkout main`) before re-invoking `/mika-handsoff`.

## Phase 2 — Synthesize content

**New-file ordering:** When Phase 1 found no existing file, run Phase 3 (slug derivation) before Phase 2 so the day's focus theme informs synthesis. For continuations, the slug is already known — run Phase 2 directly.

Synthesize the handsoff log content from session context:
- Session transcript (what was discussed, what actions were taken)
- Recent tool calls and their outcomes
- Tickets and PRs referenced, opened, updated, or merged
- Decisions made or deferred
- Blockers encountered

Use the HANDSOFF-CONTRACT section template. Include only sections that have content — drop empty sections (hard rule #6).

**Do not interrogate the operator.** Synthesize from what is available in session context. If context is thin (e.g., compacted session), produce terse sections. The operator can edit the file before commit.

**For continuation (single match in Phase 1):** Apply HANDSOFF-CONTRACT merge rules — append to existing sections per the table in the contract. Never rewrite prior `## Story so far` content. Rewrite `## What to do next session` wholesale.

## Phase 3 — AskUserQuestion budget

**Hard cap: at most one question per invocation.**

The only permitted question: when Phase 1 found no existing file AND `$ARGUMENTS` is empty, ask for the slug:

> What one-line focus/slug should I use for today's handsoff log filename?
> (e.g., "kg milestone shipped", "mika-dev dispatch fixes")

If `$ARGUMENTS` is provided, derive the slug from it (lowercase, hyphens, concise). No question needed.

If this is a continuation (file already exists), no question is needed — the slug is already determined.

**Everything else is synthesized. Do not ask follow-up questions about content.**

## Phase 4 — Write, stage, show diff, commit

1. Write the handsoff log file to `docs/logs/<TODAY> - <slug>.md`.
2. Stage the exact file only (HANDSOFF-CONTRACT hard rule #1 — never `git add -A`):
   ```bash
   git -C <meta-repo-root> add "docs/logs/<TODAY> - <slug>.md"
   ```
3. Show the staged diff so the operator can review before commit:
   ```bash
   git -C <meta-repo-root> diff --cached -- "docs/logs/<TODAY> - <slug>.md"
   ```
   This works for both new files and continuations (unstaged changes to untracked/tracked files alike become visible after staging).
4. Commit per HANDSOFF-CONTRACT § Commit message format (new-day vs continuation form). Read the contract for the exact format strings.
5. Trailers per HANDSOFF-CONTRACT § Trailers. Use the current model identifier for `Co-Authored-By`. Include `Mika-Session-Id` only if session metadata is available; omit silently if not.

## Phase 5 — Halt before push

Show the exact push command:
```
git -C <meta-repo-root> push origin <branch>
```

Where `<branch>` comes from `git -C <meta-repo-root> branch --show-current`.

**Wait for the operator's explicit approval before pushing.** Do not auto-push.

> Per memory `feedback_coordination_branch_on_origin`: operator owns the publish decision. Push coordination branches to origin at session-wrap, but only with explicit approval.

## Phase 6 — Emit terminal handsoff block

Print exactly this block (substituting actual values):

```
=== Handsoff ===
Log:    docs/logs/<filename>
Branch: <branch>  (pushed: yes/no)
Summary: <one line — same as commit message tail>
Next session: read the log.
```

No restated next-session list, no restated blockers, no restated decisions. Single source of truth lives in the file.

### Inbox write (spawned tenants only)

If the `MIKA_SPAWN_ID` environment variable is set, write an inbox entry for the orchestrator so it can discover this tenant's completion on its next `/mika-handsoff` invocation:

```bash
mkdir -p ~/.mika/orchestrator/inbox/
cat > ~/.mika/orchestrator/inbox/${MIKA_SPAWN_ID}.json << INBOX_EOF
{
  "spawn_id": "$MIKA_SPAWN_ID",
  "status": "complete",
  "summary": "<one line — same as commit message tail>",
  "completed_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "log_path": "docs/logs/<filename>",
  "branch": "<branch>",
  "pushed": <true|false>
}
INBOX_EOF
```

Field provenance: `spawn_id` from env var, `summary`/`log_path`/`branch`/`pushed` from the terminal block values above, `status` is literal `"complete"`, `completed_at` is current UTC time. The `pushed` field is a JSON boolean (`true` or `false`), not a string.

If `MIKA_SPAWN_ID` is not set, skip this step silently.

---

## Discipline

- **Shared concerns** (format, merge rules, commit format, push gate, hard rules) → `docs/logs/HANDSOFF-CONTRACT.md`. This command does not duplicate them.
- **One AskUserQuestion max** — slug-only, new-file-only.
- **Synthesize from session, don't quiz operator.**
- **Halt-and-ask before push** (HANDSOFF-CONTRACT push discipline, operator path).
- **Terminal block is signal, not restated content.**
- **Detached-HEAD halts entire flow** (no commit, no push, no file write on detached HEAD).

## Related

- `docs/logs/HANDSOFF-CONTRACT.md` — canonical format/merge/commit/push spec (mika-platform#80)
- `mika#967` — autonomous `dev-handsoff` bundled skill (same contract, different dispatch shape)
- Memory: `feedback_coordination_branch_on_origin` — push gate rationale
- Design thread: 67d85cfa (2026-05-04)
- Orchestrator inbox protocol (mika-platform#100) — Phase 0 reads inbox, Phase 6 writes inbox. `MIKA_SPAWN_ID` env var set by `scripts/mika-platform-spawn` for non-bare spawns.
