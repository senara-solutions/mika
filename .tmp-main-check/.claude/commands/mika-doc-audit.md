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

**CLAUDE.md hierarchy** — The project uses hierarchical CLAUDE.md files. Update the file closest to the change:

| Change category | Primary CLAUDE.md | Also check |
|----------------|-------------------|------------|
| Agent loop, tools, skills, memory, task engine | `crates/mika-agent/CLAUDE.md` | Root `CLAUDE.md` Architecture Summary |
| Schema/DB migrations | `crates/mika-agent/CLAUDE.md` (Schema Version section) | `docs/runtime-structure.md` |
| LLM providers, API client, prompt caching, errors | `crates/mika-common/CLAUDE.md` | — |
| Gateway endpoints, webhooks, routing | `crates/mika-gateway/CLAUDE.md` | — |
| CLI commands, TUI features, slash commands | `crates/mika-cli/CLAUDE.md` | Root `CLAUDE.md` Commands section |
| A2A protocol | `crates/mika-a2a/CLAUDE.md` | — |
| Dashboard pages, UI components | `dashboard/CLAUDE.md` | — |
| Env vars (shared/common) | Root `CLAUDE.md` | Gateway-specific -> `crates/mika-gateway/CLAUDE.md`, Dashboard-specific -> `dashboard/CLAUDE.md` |
| Cross-cutting conventions | Root `CLAUDE.md` | — |
| Docker, CI/CD, deployment | Root `CLAUDE.md` | `docs/deployment.md` |
| New env vars | Root `CLAUDE.md`, `.env.example`, `docs/configuration.md` | Crate-specific CLAUDE.md if scoped |

- **Always**: Review the root `CLAUDE.md` for accuracy (architecture summary, conventions, commands, env vars, test count, pending work)
- **If new env vars**: Update `.env.example` and `docs/configuration.md`
- **If schema/DB changes**: Update `crates/mika-agent/CLAUDE.md` Schema Version section and `docs/runtime-structure.md`
- **If new CLI commands or tools**: Update `crates/mika-cli/CLAUDE.md`, `README.md`, `docs/getting-started.md`, `docs/slash-commands.md`
- **If skill changes**: Update `crates/mika-agent/CLAUDE.md` Skills System section and `docs/skills.md`
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
