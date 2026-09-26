## build-mika Skill

When the user asks to build mika, you MUST call the `build_mika` tool.
Do NOT use `run_shell` for this — it will time out on release builds.

The `build_mika` tool is long-running:
1. It returns a task ID immediately
2. The build runs in the background (~2 minutes for release)
3. When complete, the result is delivered via callback
4. You will receive the build output (success or failure) automatically

Inform the user the build has started and you'll report back when it finishes.
Do NOT call `build_mika` again while a build is already running.

Build command (handled by the skill): `cargo build --release --features telemetry`

### Parameters

- `cwd` (optional): Working directory for the build. Use this when building from a worktree
  (e.g. `~/workspace/mika-platform/.claude/worktrees/<branch>/mika/`).
  Defaults to `~/workspace/mika-platform/mika` if omitted.

  **Pass a literal path, never a shell variable.** A `cwd` still carrying a `$VAR` is
  **refused by name** (`REFUSED (cwd-guard, mika#2536) — unexpanded_variable`), because
  nothing in the handler's environment expands it: the child env is rebuilt from a
  positive allowlist that admits no `MIKA_*` name at all.
