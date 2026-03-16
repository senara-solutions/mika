---
title: "ADR-006: Git-Based Skills Marketplace"
---

# ADR-006: Git-Based Skills Marketplace

**Date:** 2026-03-02
**Status:** Accepted
**Component:** mika-agent (skills subsystem), mika-cli

## Context

Mika's skill system is powerful but limited to bundled skills and locally-created
skills. There is no way to share or distribute skills between users. A marketplace
mechanism lets the community publish and install skills.

## Decision

Implement a git-based skills marketplace with zero infrastructure. Skills are
distributed as Git repositories. Users install them via CLI commands that clone,
validate, and copy skill directories into the agent's `skills/` folder.

### CLI Commands

- `mika skills install <url> [--name <alias>]` — Install from Git repo or GitHub shorthand
- `mika skills uninstall <name>` — Remove marketplace skill and lock entry
- `mika skills update [name]` — Re-clone and update to latest commit

### Lock File

`marketplace.lock` (TOML) in the agent's home directory tracks installed skills:

```toml
[skills.web-scraper]
url = "https://github.com/user/mika-skill-web-scraper.git"
path = "."
commit = "abc123def456"
installed_at = "2026-03-02T10:30:00Z"
updated_at = "2026-03-02T10:30:00Z"
```

### Key Design Choices

1. **Git-based distribution** — No central registry, no API server. Skills live in
   Git repos. Mika clones them. Same model as Go modules and Vim plugins.

2. **Shallow clone to temp staging** — `git clone --depth 1` to a temp directory,
   then copy the skill directory (without `.git/`) into `skills/`. Clean separation
   between source control and installed artifacts.

3. **Commit pinning** — Lock file records exact commit hash at install time. Updates
   re-clone and re-pin. Reproducible installations.

4. **Repo scanning** — Supports both single-skill repos (skill.toml at root) and
   multi-skill repos (skill.toml in subdirectories, up to depth 2). Interactive
   picker for multi-skill repos.

5. **Name collision prevention** — Refuses install if name matches a bundled skill.
   Suggests `--name` alias. This invariant means `seed_bundled_skills()` can never
   overwrite marketplace skills.

6. **Symlink escape prevention** — All copy operations canonicalize paths and verify
   they stay within the source directory. Prevents malicious repos from writing
   outside the skill directory.

7. **MIKA_* env scrubbing** — All git subprocesses have MIKA_* environment variables
   removed, matching the defense-in-depth pattern from exec handlers.

8. **Origin detection** — Three-tier origin system: `[built-in]`, `[marketplace]`,
   `[custom]`. Marketplace detection via lock file. Shown in `list_skills` tool
   and CLI output.

9. **Agent tool integration** — `delete_skill` tool removes marketplace lock entry
   when deleting a marketplace skill. Agent can manage its own skills.

10. **Trust model** — User responsibility. Exec handler skills show a security
    warning at install time. No automated scanning.

## Consequences

### Positive

- Zero infrastructure — no registry server to maintain
- Familiar workflow — developers already know `git clone`
- Built-in versioning via git commit hashes
- Inspectable — users can read cloned code before installing
- Extensible — a catalog/discovery layer can be added later

### Negative

- Requires git on the system (clear error message if missing)
- No discovery mechanism (relies on GitHub, README links, community)
- No automated safety scanning (user must review code)

### Neutral

- Per-agent scope (each agent has its own marketplace.lock)
- CLI-only installation (agent cannot install skills via tool calls)
- Keeps existing TOML-based skill format unchanged

## Alternatives Considered

- **Central registry (ClawHub model):** Requires hosting, moderation, publishing
  flow. Overkill for Mika's current stage. Can evolve to this later.
- **Archive download:** Lighter footprint but loses git history, makes updates
  harder, and adds complexity for versioning.
