## dev-groom Skill

Use `run_claude_pilot_groom` to dispatch a headless Claude Code grooming session via the claude-pilot CLI. The session runs the `/mika-groom-ticket` pipeline (two-pass mika-arch review → plan-on-branch).

### When to use
- A ticket needs grooming before implementation (operator-direct via `/mika-groom-ticket`, or autonomous when a `ready`-labelled ticket lacks a Plan callout, or milestone-cascade pre-flight when a child lacks a Plan callout).

### How it works
1. Call `run_claude_pilot_groom` with `skill: "dev-groom"` and a `prompt` in `repo#number` format (e.g., `mika#214`)
2. The handler derives the branch, creates a worktree, and runs `/mika-groom-ticket <repo>#<number>` in the inner Claude Code session
3. The tool is **long-running**: it returns immediately with a task ID, results arrive later via callback
4. During the run, claude-pilot will separately call back to mika-dev for permission decisions — handle those as they arrive
5. When claude-pilot finishes, you receive a callback with the architect verdict (`Verdict: GROOMED` or `Verdict: ESCALATE`)

### Authority bounds

The dev-groom pilot's scope is **content-only**: read the ticket, generate a plan, commit it. All git push operations are dispatch-lib's responsibility (`_push_branch`). The pilot MUST NOT execute any of:

- `git push --force`, `git push --force-with-lease`, `git push -f`
- `git push` (plain — push of any kind is out of scope)
- Any other destructive remote operation

If local/remote divergence is observed, the pilot does NOT resolve it — dispatch-lib's `_set_up_worktree` handles branch reconciliation. The pilot's responsibility ends at the commit. (mika#1318 founding incident: a dev-groom pilot ran `git push --force-with-lease` from inside its worktree, destroying substrate-fix work on the remote.)

### Important
- **Always pass `skill: "dev-groom"`** — required by the schema for engine dispatch-class derivation
- **Always pass the task UUID as `task_id`** (36-char format) when a task exists. Do NOT pass issue references — pass the UUID returned by `create_task`. This ensures logs land at `/var/log/claude-pilot/{uuid}.log`
- **Do NOT do the work inline** — never read source files, analyze the ticket, or write a plan yourself. The grooming workflow runs in the inner session via `/mika-groom-ticket`. Always use `run_claude_pilot_groom`
- **Do NOT emit `Verdict: GROOMED` or `Verdict: ESCALATE` in your dispatch response.** The verdict is produced by the inner Claude Code session and arrives via callback from claude-pilot — not from your turn. Your dispatch response should be: `"Dispatched grooming for <ref>. Awaiting architect verdict via callback (task: <task_id>)."` The engine rejects fabricated Verdict lines via the dev-groom fabrication guard (mika#1133).
- Do NOT call `run_claude_pilot_groom` again for the same task while one is already running
- On `Verdict: GROOMED` callback, the issue body now carries `Branch:` + `Plan:` + `Grooming history:` callouts — the ticket is ready for dispatch via `run_claude_pilot` (dev-pilot)
- On `Verdict: ESCALATE` callback, surface the architect's reasoning to the operator and halt — do not retry without operator instruction
