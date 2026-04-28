## dev-pilot Skill

Use `run_claude_pilot` to dispatch a headless Claude Code implementation session via the claude-pilot CLI.

### When to use
- User wants to run Claude Code on a project (implement a feature, fix a bug, refactor, etc.)

### How it works
1. Call `run_claude_pilot` with a `prompt` in `repo#number` format (e.g., `mika-skills#8`)
2. The handler derives the branch, creates a worktree, and runs `/mika #number` in the target repo
3. The tool is **long-running**: it returns immediately with a task ID, results arrive later via callback
4. During the run, claude-pilot will separately call back to mika-dev for permission decisions and questions — handle those as they arrive
5. When claude-pilot finishes, you receive structured results: status, session_id, turns, cost, duration

### Important
- **Always pass the task UUID as `task_id`** when a task exists (36-char format like `15383984-a3e7-41bf-ac6f-630ba9a89d63`). Do NOT pass issue references (e.g., `mika-284`) — pass the UUID returned by `create_task`. This ensures logs land at `/var/log/claude-pilot/{uuid}.log` and everything correlates under one ID. Never let `task_id` auto-generate when you have a task.
- Do NOT call `run_claude_pilot` again for the same task while one is already running
- For non-issue tasks, pass a free-text prompt — it runs as-is without worktree setup
- claude-pilot handles its own permission relay via `mika --agent mika-dev ask` — you don't need to manage tmux for it
