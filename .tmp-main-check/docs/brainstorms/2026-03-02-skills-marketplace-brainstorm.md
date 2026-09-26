# Skills Marketplace Brainstorm

**Date:** 2026-03-02
**Status:** Draft
**Author:** Sami / Claude

## What We're Building

A git-based skills marketplace that lets Mika users share, discover, and install community-created skills from Git repositories. Users install skills via `mika skills install <repo-url>`, which clones the repo into the agent's `skills/` directory and tracks it in a lock file.

### Core User Stories

1. **As a skill author,** I want to publish a Mika skill by pushing it to a Git repo so others can install it.
2. **As a Mika user,** I want to install a community skill from a Git URL so I can extend my agent's capabilities.
3. **As a Mika user,** I want to update installed marketplace skills to get the latest versions.
4. **As a Mika user,** I want to uninstall marketplace skills I no longer need.
5. **As a Mika user,** I want to see which skills are marketplace-installed vs. local/bundled.

### What It Is NOT

- Not a hosted registry or API service — no infrastructure to run.
- Not a format change — keeps Mika's existing TOML-based skill format.
- Not sandboxed — users are responsible for reviewing skills before installing (same trust model as installing a shell script from GitHub).
- Not a central search engine — discovery happens through GitHub, community lists, READMEs.

## Why This Approach

**Git-based distribution with bare clone** was chosen for several reasons:

1. **Zero infrastructure.** No registry server, no API, no database. Skills live in Git repos. Mika clones them.
2. **Familiar model.** Developers already know `git clone`. This is how Go modules, Vim plugins (pre-package-managers), and many tools distribute extensions.
3. **Built-in versioning.** Git provides commit history, tags, branches. The lock file pins to a specific commit hash for reproducibility.
4. **Inspectable.** Users can read the cloned code, diff updates, and even fork/modify installed skills.
5. **Incremental.** A catalog/discovery layer can be added later without changing the install mechanism.

### Alternatives Considered

- **Central registry (ClawHub model):** Requires hosting, moderation, publishing flow. Overkill for Mika's current stage. Can evolve to this later.
- **Archive download:** Lighter footprint but loses git history, makes updates harder, and adds complexity for versioning.
- **Agent Skills open standard (SKILL.md):** Cross-platform compatibility is appealing, but Mika's TOML format is established and battle-tested. Supporting both adds complexity without clear near-term value.

## Key Decisions

1. **Distribution model:** Git repos. Install by cloning. No central infrastructure.
2. **Skill format:** Keep existing TOML-based format (`skill.toml` + `system_prompt.md` + `tools.json` + `handlers/`). No format migration.
3. **Trust model:** User responsibility. Show a warning on install with a reminder to review the code. No automated scanning in v1.
4. **Skill scope:** Per-agent (current model). Install into `~/.mika/agents/<agent>/skills/<name>/`.
5. **CLI scope:** Minimal — `install`, `uninstall`, `update`. No search, no publish scaffolding in v1.
6. **Lock file:** `marketplace.lock` (TOML or JSON) in the agent's home directory tracks installed skills with repo URL, commit hash, and install timestamp.
7. **Agent cannot install:** Installation is CLI-only. The agent cannot install skills via tool calls.
8. **Version pinning:** Pin to HEAD commit at install time. Lock file records exact commit hash. `update` pulls latest and re-pins.
9. **Update UX:** Silent update. Pull, update lock file, print one-liner summary. No interactive confirmation.
10. **Install validation:** Manifest only. Check `skill.toml` exists and parses. Fail at runtime if handlers are broken.
11. **Handler permissions:** Trust git. No auto `chmod +x`. Author's responsibility to set execute bits in the repo.
12. **Name conflicts:** Refuse install if name collides with a bundled skill. Suggest `--name` alias.

## Feature Shape

### CLI Commands

```
mika skills install <url> [--name <alias>]
```
- Clones the repo to a **temp staging directory**
- Scans for all `skill.toml` files in the repo
  - **One skill found:** Copies it into the agent's `skills/` directory
  - **Multiple skills found:** Lists them interactively, user picks which to install (can pick multiple)
  - **No skill.toml found:** Error with clear message
- Validates each selected skill's `skill.toml` parses correctly
- Copies selected skill directory (without `.git`) into `skills/<name>/`
- Records entry in `marketplace.lock` (includes repo URL + path within repo)
- Cleans up temp clone
- Supports GitHub shorthand: `mika skills install user/repo` -> `https://github.com/user/repo`
- Optional `--name` to rename the skill directory (only valid when installing a single skill)
- Refuses install if name collides with a bundled skill (suggests `--name`)

```
mika skills uninstall <name>
```
- Removes the skill directory
- Removes the lock file entry
- Cannot uninstall bundled skills (suggests `disable` instead, matching existing behavior)

```
mika skills update [name]
```
- If name given: re-clones the source repo (from lock file URL), extracts the skill at the recorded path, replaces the local copy, updates lock file commit hash
- If no name: update all marketplace-installed skills
- Prints one-liner summary per skill (old commit -> new commit). No interactive confirmation.

### Lock File (`marketplace.lock`)

```toml
[skills.web-scraper]
url = "https://github.com/user/mika-skill-web-scraper"
path = "."                               # Root of repo (single-skill repo)
commit = "abc123def456"
installed_at = "2026-03-02T10:30:00Z"
updated_at = "2026-03-02T10:30:00Z"

[skills.notion-sync]
url = "https://github.com/user/mika-skills-collection"
path = "notion-sync"                     # Subdirectory (multi-skill repo)
commit = "789def012345"
installed_at = "2026-03-01T14:00:00Z"
updated_at = "2026-03-02T08:00:00Z"
```

### Existing Behavior Changes

- `mika skills` / `list_skills` tool: Add an "origin" indicator — `bundled`, `custom`, or `marketplace` (marketplace detected via lock file entry).
- `mika skills info <name>`: Show marketplace metadata (repo URL, path, installed commit, last updated) for marketplace skills.
- `delete_skill` tool: Allow deleting marketplace skills (removes from lock file too). Agent can manage its own skills.
- Bundled skill re-sync on startup: Skip skills that are marketplace-installed (don't overwrite with bundled versions).
- Installed marketplace skills do NOT contain `.git` — they are plain copies extracted from the clone. This keeps the skills/ directory clean.

### Skill Repo Convention

A publishable Mika skill repo can be structured two ways:

**Single-skill repo** (repo root IS the skill):

```
mika-skill-web-scraper/         # Repo root
  skill.toml                    # Required — detected at root
  system_prompt.md              # Optional
  tools.json                    # Optional
  handlers/                     # Optional (for exec-handler skills)
    run.sh
  README.md                     # Recommended (for discovery)
  LICENSE                       # Recommended
```

**Multi-skill repo** (repo contains multiple skill directories):

```
mika-skills-collection/         # Repo root
  web-scraper/
    skill.toml                  # Detected
    system_prompt.md
    handlers/
  notion-sync/
    skill.toml                  # Detected
    tools.json
  slack-notify/
    skill.toml                  # Detected
    system_prompt.md
  README.md
  LICENSE
```

**Auto-detection behavior:** The installer scans the repo for all `skill.toml` files. Single skill → install directly. Multiple skills → present interactive list. This means no `--path` flag is needed — the user picks from what the scanner finds.

## Resolved Questions

1. **Git dependency:** Require git. It's already in the Docker image and expected on dev machines. Error with a clear message if git is missing. No archive fallback.

2. **Conflict handling:** Refuse install if a marketplace skill has the same name as a bundled skill. Error message suggests using `--name` to install under a different alias.

3. **Handler permissions:** Trust git permissions. Rely on git preserving execute bits. If scripts aren't executable, the skill fails at runtime with a clear error. Author's responsibility.

4. **Startup behavior with broken skills:** Keep current behavior — log warning, skip broken skills. No `mika skills doctor` for v1 (YAGNI).

5. **Agent install capability:** CLI-only. The agent cannot install skills via tool calls. Keeps humans in the loop.

6. **Version pinning:** Pin to HEAD commit at install time. Lock file records exact commit hash. `update` pulls latest and re-pins.

7. **Update UX:** Silent. Pull, update lock, print one-liner. No interactive confirmation.

8. **Install validation depth:** Manifest only. Check `skill.toml` exists and parses. Runtime failures are the author's problem.

9. **Multi-skill repos:** Auto-detect all `skill.toml` files in the repo. One found → install directly. Multiple found → interactive picker. No `--path` flag needed.

## Open Questions

None — all resolved.

## Inspirations

- **OpenClaw ClawHub:** Three-tier distribution (bundled/published/workspace), content-hash versioning, safety scanning. We take the tiered model but skip the central registry for v1.
- **Claude Code plugins:** GitHub-based distribution with `marketplace.json`, namespace scoping. We take the git-based install model but keep our format.
- **Vim plugin managers (vim-plug, Vundle):** Git-clone-into-directory model with a lock/manifest file. Proven pattern for decades.
- **Go modules:** Git-based distribution with commit pinning. No central registry needed (though proxy.golang.org exists as optional cache).
