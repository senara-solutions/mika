> Metadata extraction: see self-dev skill.

**Step 3a — Iteration mode (when iterating on an existing PR)**

Use this step INSTEAD of Step 3 when the user asks to iterate on an existing PR with specific feedback. Step 3a replaces only the `run_claude_pilot` call — all other steps (1, 2, 4, 5, 6) still apply.

**Detect iteration signals.** The user's request is an iteration (not fresh work) when they want changes to an existing PR — e.g., "iterate on PR #N", "address review feedback", "fix the PR", or specific concerns about a PR that already exists. QA hold/block feedback to address on an existing PR is also an iteration signal.

**If iteration signals are present:**

1. **Find the existing task.** Call `list_tasks` and match by `reference_url` (the issue URL) or label. The item should be `in_progress` or `blocked`. If `blocked`, call `update_task_status` to transition it to `in_progress` before proceeding.

2. **Discover the branch name.** Check the task's metadata for `claude_pilot.branch` (set in Step 6). If not available, query: `gh pr list --search "is:open" --repo senara-solutions/<repo> --json headRefName,number` and match by issue number or PR number.

3. **Compose the prompt with iteration context.** Use `repo#number` in the `prompt` field (this triggers worktree reuse in the handler) and add `iteration_context` with the user's specific feedback:

```json
{
  "prompt": "repo#number",
  "task_id": "<existing task ID>",
  "iteration_context": "Iterate on PR #<N> (branch: <branch>). Push to existing branch — do NOT create a new PR.\n\nChanges requested:\n1. <user's specific feedback verbatim>\n2. <user's specific feedback verbatim>\n\nAddress ONLY these concerns. Do not re-implement the entire feature."
}
```

**Compose `iteration_context` with these rules:**
- Include the PR number, branch name, and "push to existing branch" directive
- Copy the user's specific feedback **verbatim** — do not summarize, paraphrase, or lose detail
- If the user provided QA review feedback, include the QA findings verbatim
- End with "Address ONLY these concerns. Do not re-implement the entire feature."

After calling `run_claude_pilot`, the rest of the workflow is identical to Step 3 — wait for the callback, extract metadata, proceed to Step 6 (close-out). mika-qa triggers automatically via webhook.

**If iteration signals are NOT present:** Use Step 3 (bare `repo#number`, no `iteration_context`).

**Step 3b — Address PR review comments**

Use this step when Vincent asks to "address review comments" or "handle PR feedback" for a specific PR, or when you detect unresolved review comments on an open PR.

Call `address_pr_comments` (not `run_claude_pilot`):
```json
{"pr_url": "<full PR URL>", "worktree_path": "<existing worktree path>", "task_id": "<task UUID>"}
```

The handler fetches review comments from the GitHub API, constructs a focused prompt, and runs claude-pilot in free-text mode. After the callback, proceed to Step 6 (close-out). mika-qa triggers automatically on the PR update via webhook.

---

## Calibration Rules

These rules encode specific failure modes observed in live dev runs. Each rule cites the incident that motivated it.

### Rule 1 — Cross-repo scope drift

If during implementation you discover the code lives in a **different repository** than the ticket was filed on:

1. **Stop work in the current worktree immediately.** Do not create a PR on the wrong repo as a consolation prize.
2. **Open a new worktree on the correct repo** using the same branch name (keeps traceability).
3. **Move the plan doc, solution doc, AND the source change into a single PR** on the correct repo. Pipeline artifacts (plan + solution) must ship with the code they describe.
4. **Do NOT split artifacts across repos.** A docs-only PR on the original repo plus a code-only PR on the correct repo is explicitly forbidden — this was the `mika#485` / `mika-skills#102` failure mode.
5. **Leave a comment on the original ticket** explaining the re-routing and linking the new PR before closing or refiling.

Special case for "skill" tickets: if the skill is a **bundled template** (shipped inside `mika/crates/mika-agent/templates/skills/`), the ticket belongs on the `mika` repo, not `mika-skills`. The `mika-skills` marketplace repo is for community/external skills only.

**Incident:** `mika#485` + `mika-skills#102` on 2026-04-08 — rename in `mika#485`, artifacts orphaned in `mika-skills#102` (docs-only). Split was root cause.
