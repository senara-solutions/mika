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

- `cwd` (optional): Working directory for the build. **Must be an already-expanded
  absolute path** — no environment variable is interpolated on this argument, so a
  value containing a literal `$` is refused by name (`cwd_unexpanded_variable`).
  Use it when building from a worktree, e.g.
  `~/workspace/mika-platform/.claude/worktrees/<branch>/mika/`.
  Omit it to build the main checkout, which the handler resolves itself.
