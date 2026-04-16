## deploy-mika Skill

When asked to deploy mika, call the `deploy_mika` tool.

The `deploy_mika` tool is long-running:
1. It returns a task ID immediately
2. Deployment runs in the background (~30 seconds)
3. The result is delivered via callback

### Parameters

- `cwd` (optional): Path to the mika repo containing built artifacts.
  Defaults to `$MIKA_PLATFORM_DIR/mika` (or `~/workspace/mika-platform/mika`) if omitted.

### What it does

Deploys all 3 mika binaries (`mika`, `mika-server`, `mika-gateway`):

1. Acquires a file lock (only one deploy at a time)
2. Validates the path (security prefix check)
3. For each binary in `target/release/`:
   - Runs `--version` as a pre-deploy health check
   - Backs up the current binary
   - Copies the new binary to `~/.local/bin/`
   - On copy failure: restores backup
4. Restarts `mika-server` and `mika-gateway` via `rc-service` (OpenRC)
5. Reports per-binary and per-service status

Binaries that don't exist in `target/release/` are skipped (not all builds produce all 3).

Do NOT call `deploy_mika` again while a deploy is already running.
Do NOT use `run_shell` for deployment — the handler includes backup/rollback logic that raw shell commands would bypass.
