---
name: mika-doc-audit
description: Audit and update documentation based on code changes
argument-hint: "[scope or instruction, e.g. 'since last commit', 'full audit', 'only CLAUDE.md']"
---

$ARGUMENTS

Review the git diff and update all affected documentation.

If a specific scope was provided above, use it (e.g. "since last commit" = `git diff HEAD~1`, 
"since last merge" = `git diff main...HEAD`, "full audit" = review everything regardless of diff).

If no scope was provided, default to `git diff main...HEAD`.

Process:
1. Run the appropriate git diff to identify changed files
2. Categorize changes: schema, CLI, skills, config, infra, env vars, tools, architecture
3. For each category affected, update the corresponding docs:

- **Always**: Review `CLAUDE.md` for accuracy (architecture, conventions, commands, env vars, test count, schema version, pending work)
- **If new env vars**: Update `.env.example` and `docs/configuration.md`
- **If schema/DB changes**: Update `docs/architecture.md` and CLAUDE.md Architecture section
- **If new CLI commands or tools**: Update `README.md`, `docs/getting-started.md`, `docs/slash-commands.md`
- **If skill changes**: Update `docs/skills.md`
- **If infra changes** (Docker, deployment): Update `docs/deployment.md`
- **If new config fields**: Update `docs/configuration.md`
- **If new slash commands**: Update `docs/slash-commands.md`

4. Show a summary of what was updated and why
5. Run `bash scripts/sync-agent-docs.sh` to sync crate-local doc copies (CI enforces this via the `docs-sync` job — PRs that skip this step will fail)
6. After syncing, run `cargo build -p mika-agent` to verify the build.rs picks them up
7. Commit the doc changes with message: "docs: update documentation for recent changes"

**Important:** `docs/` is the single source of truth. The `build.rs` in `crates/mika-agent/`
copies docs into `OUT_DIR` at build time. Crate-local copies in `crates/mika-agent/docs/` must
stay in sync — CI enforces this via the `docs-sync` job on every PR. Step 5 handles this
automatically; never skip it.

Do NOT invent information. Only document what exists in the code. If unsure about
a detail, check the source file before writing.
