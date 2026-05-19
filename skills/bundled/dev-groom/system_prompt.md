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

### Important
- **Always pass `skill: "dev-groom"`** — required by the schema for engine dispatch-class derivation
- **Always pass the task UUID as `task_id`** (36-char format) when a task exists. Do NOT pass issue references — pass the UUID returned by `create_task`. This ensures logs land at `/var/log/claude-pilot/{uuid}.log`
- **Do NOT do the work inline** — never read source files, analyze the ticket, or write a plan yourself. The grooming workflow runs in the inner session via `/mika-groom-ticket`. Always use `run_claude_pilot_groom`
- Do NOT call `run_claude_pilot_groom` again for the same task while one is already running
- On `Verdict: GROOMED`, the issue body now carries `Branch:` + `Plan:` + `Grooming history:` callouts — the ticket is ready for dispatch via `run_claude_pilot` (dev-pilot)
- On `Verdict: ESCALATE`, surface the architect's reasoning to the operator and halt — do not retry without operator instruction
