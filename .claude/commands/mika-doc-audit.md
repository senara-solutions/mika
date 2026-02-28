---
name: mika-doc-audit
description: Audit and update documentation based on code changes
---

Review the git diff (`git diff main...HEAD`) and update all affected documentation:

- **Always**: Review `CLAUDE.md` for accuracy (architecture, conventions, commands, env vars, test count, schema version, pending work)
- **If new env vars**: Update `.env.example` and `docs/configuration.md`
- **If schema/DB changes**: Update `docs/architecture.md` and CLAUDE.md Architecture section
- **If new CLI commands or tools**: Update `README.md`, `docs/getting-started.md`, `docs/slash-commands.md`
- **If skill changes**: Update `docs/skills.md`
- **If infra changes** (Helm, Docker, K8s): Update `docs/deployment.md`
- **If new config fields**: Update `docs/configuration.md`
