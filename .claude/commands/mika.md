---
name: mika
description: Mika development workflow with quality gates and documentation audit
argument-hint: "[feature description]"
disable-model-invocation: true
---

Run these slash commands in order. Do not do anything else. Do not stop between steps — complete every step through to the end.

1. `/ralph-loop "finish all slash commands" --completion-promise "DONE"`
2. `/workflows:plan $ARGUMENTS`
3. `/workflows:work`
4. `/workflows:review`
5. `/compound-engineering:resolve_todo_parallel`
6. **Documentation audit** — Review the git diff (`git diff main...HEAD`) and update all affected documentation:
   - **Always**: Review `CLAUDE.md` for accuracy (architecture, conventions, commands, env vars, test count, schema version, pending work)
   - **If new env vars**: Update `.env.example` and `docs/configuration.md`
   - **If schema/DB changes**: Update `docs/architecture.md` and CLAUDE.md Architecture section
   - **If new CLI commands or tools**: Update `README.md`, `docs/getting-started.md`, `docs/slash-commands.md`
   - **If skill changes**: Update `docs/skills.md`
   - **If infra changes** (Helm, Docker, K8s): Update `docs/deployment.md`
   - **If new config fields**: Update `docs/configuration.md`
7. `/workflows:compound`
8. Output `<promise>DONE</promise>` when complete

Start with step 1 now.
