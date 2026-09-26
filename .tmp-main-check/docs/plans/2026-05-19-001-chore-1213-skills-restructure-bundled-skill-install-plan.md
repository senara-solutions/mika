# Plan: Restructure bundled-skill install — canonical runtime library + per-agent symlinks (mika#1213)

type: chore (refactor)
ticket: mika#1213
date: 2026-05-19
replaces: mika#1209 (closed)
groomed-via: peer-Claude pass-2 GROOMED (mika-arch blocked by mika#1207)

## Problem

`mika skills update --agent <name>` currently writes per-agent copies of bundled-skill content at `~/.mika/agents/<name>/skills/<skill>/`. When a source skill file is removed in a deploy, the deployed copies persist as silent stale handlers (mika#1197 incident, 2026-05-18, where `qa-review/run_gh.sh` deletion required manual `find + delete` to clean up).

The structural fix: replace per-agent copies with a **canonical runtime library plus per-agent symlinks**. Dissolves the silent-stale-file bug class while removing duplication of identical skill content across the four well-known agents.

## Context

- **No external mika users.** Vincent is the sole operator. KISS — no migration logic, no compat shims, no announcements. `make deploy` after this lands wipes existing per-agent skill dirs and re-creates them as symlinks.
- **Binary-blob seed model (already in place).** `crates/mika-agent/build.rs` walks `skills/bundled/` at compile time and bakes the entire content into `BUNDLED_SKILL_MANIFESTS` in the binary. At runtime, `seed_bundled_skills_if_needed()` extracts that blob to disk. The deployed "copies" are extractions from the binary, not copies from source files. `Dockerfile.agent` does NOT copy `skills/bundled/` into the image — only the binary needs to be there.
- **Tool dispatch is gated in-memory, not by filesystem state.** The active skill set per agent is computed at session start from `BUNDLED_SKILL_MANIFESTS ∩ identity_allowlist` (with DB-layer `skill_overrides` on top). Filesystem layout — symlinks, library directories, on-disk handler scripts — is the **handler-execution substrate**, not the **tool-discovery surface**. Consequence: **dangling symlinks or orphan library directories for de-allowlisted/retired skills are inert at runtime**. The LLM cannot see or try to invoke a skill that isn't in the manifest-set ∩ allowlist. This is the load-bearing invariant that makes the restructure safe.
- **Mika OS is our Docker artifact.** `mika/Dockerfile.agent` is owned in-tree. Runtime fs layout in the container is our design call. The library + symlink model works in both Vincent's dev machine and Mika OS containers — both extract the library from the binary at runtime to `~/.mika/skills/`, both create per-agent symlinks under `~/.mika/agents/<name>/skills/`.
- **Forward-looking design for marketplace skills (`mika-skills` repo)**: "as above so below" with inverted defaults — marketplace would be copy-by-default with `--link` opt-in. **Marketplace is out of scope for this ticket** — nobody uses it yet.
- **Ruled out:** (a) the original `--prune` flag fix (mika#1209) — solves the symptom, leaves the duplication. (b) keeping copies and adding rsync-style delete semantics — same per-agent duplication problem. (c) symlinking directly to source-tree `skills/bundled/` — doesn't work in Docker because source isn't copied into the image; the binary IS the source.

## What's being changed

### 1. Canonical runtime library at `~/.mika/skills/`

Modify `seed_bundled_skills_if_needed()` to write extracted skill content to a single shared location, not per-agent paths. Hash-comparison gate (compare binary's manifest hash vs. recorded hash on disk) ensures re-extraction only runs when the binary changes — idempotent across restarts. Hash-record file name TBD at plan phase (e.g., `.manifest-hash` at `~/.mika/skills/.manifest-hash`).

### 2. Library extraction is sync-shaped

After `seed_bundled_skills_if_needed()` returns, `~/.mika/skills/` contains **exactly** the directories in the current binary's `BUNDLED_SKILL_MANIFESTS` — no more, no less. Extract new/changed skills, remove directories for skills no longer in the manifest set.

This is safe because:
- (a) The library is platform-managed, same contract as `is_bundled_skill()` enforces everywhere else (`delete_skill`, `update_skill`, `install`, `review_skill` all refuse operator mutation of bundled skills).
- (b) Orphan directories would be inert at runtime per the in-memory dispatch gate invariant, but cleaning them up keeps on-disk state honest and prevents `mika doctor`-style drift.

### 3. Per-agent skill dirs become symlinks

Modify `mika skills update --agent <name>` to walk the agent's identity allowlist and create one `ln -sf ~/.mika/skills/<skill> ~/.mika/agents/<name>/skills/<skill>` per allowlisted bundled skill. On re-run, remove symlinks for skills no longer in the allowlist. **Non-bundled (custom/marketplace) skill directories under `~/.mika/agents/<name>/skills/` are untouched** — this restructure is bundled-only.

### 4. `--copy` per-skill opt-out

`mika skills install <skill> --copy --agent <name>` materializes a real per-agent directory by copying from the library, rather than symlinking. Library sync (#2) does NOT touch `--copy`'d agent directories — the operator who used `--copy` owns that lifecycle. Rare escape hatch for hot-patching a deployed skill independently of the library.

### 5. Documentation

- Update `docs/runtime-structure.md` to reflect the new `~/.mika/skills/` top-level directory and the symlink shape under `~/.mika/agents/<name>/skills/`. **This doc is `include_str!`'d into the binary via `build.rs` and consumed by the `self-knowledge` skill** — a stale entry there has runtime visibility for mika-dev's self-introspection, so the doc must land in the same PR as the code change.
- Update `docs/solutions/architecture/removing-bundled-skill.md` to drop the "users can manually `rm -rf`" guidance (no longer required — library sync handles it).

### 6. Close mika#1209

The `--prune` flag becomes moot — there are no per-agent copies left to prune. Already closed (2026-05-19) in favor of this ticket.

## Acceptance criteria

- **AC1.** After `seed_bundled_skills_if_needed()` returns, `~/.mika/skills/` contains exactly the set of directories in the current binary's `BUNDLED_SKILL_MANIFESTS`. Hash-compared, idempotent on no-change.
- **AC2.** `mika skills update --agent <name>` creates `~/.mika/agents/<name>/skills/<skill>` as a symlink into `~/.mika/skills/<skill>` for each skill in the agent's identity allowlist, and removes symlinks for bundled skills no longer in the allowlist.
- **AC3.** Existing per-agent skill directories at `~/.mika/agents/<name>/skills/<skill>/` for bundled skills are replaced with symlinks into `~/.mika/skills/<skill>/`. Non-bundled skill directories under the same path are untouched.
- **AC4.** `mika skills install <skill> --copy --agent <name>` materializes a real directory copy under the agent's skill dir. The library sync in AC1 does not touch `--copy`'d agent directories.
- **AC5.** Unit tests cover the sync-shaped invariant (AC1), the symlink creation + cleanup (AC2), and the `--copy` opt-out (AC4). Integration test covers the end-to-end deploy → seed → update → agent-startup path against a representative bundled agent (mika-dev recommended — 26-skill allowlist exercises the path width).

## Repo-specific check the pipeline should catch

The runtime-structure doc (`docs/runtime-structure.md`) is `include_str!`'d via `build.rs` into the `get_documentation` tool surface for the `self-knowledge` skill. Doc-audit must verify the runtime-structure entry matches the new `~/.mika/skills/` layout, not just check the doc exists.

## Out of scope

- Marketplace skill restructure (separate ticket when marketplace use cases appear)
- Migration of existing copies via standalone migration tool (sole operator, no compat — `make deploy` does the right thing)
- Hash-record filename canonicalization (plan-phase decision, `~/.mika/skills/.manifest-hash` is the obvious shape)

## Related

- mika#1209 — original `--prune` proposal; closed in favor
- mika#1197 — deploy incident that triggered discovery
- `project_skill_dispatch_gated_in_memory.md` — load-bearing invariant memory
- `crates/mika-agent/CLAUDE.md` § Build-Time Discovery — binary-blob model docs
- `crates/mika-agent/src/well_known_agents.rs` — identity-driven allowlist
- `Dockerfile.agent` — confirms no source-tree copy of `skills/bundled/` into image
