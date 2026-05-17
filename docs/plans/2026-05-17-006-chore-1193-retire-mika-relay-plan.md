---
type: chore
issue: mika#1193
parent: mika#1188 (milestone: Deprecate mika-relay)
depends_on: mika#1192 (merged + soaked ≥7 days)
title: Retire mika-relay agent + permission-policy skill + config refs
date: 2026-05-17
---

# Plan: Retire mika-relay agent (mika#1193, Phase C of mika#1188)

## Phase 0 — Pin

**Base anchors at grooming time:**
- `mika` HEAD: `72021b78482f1c313156e7630d626865415dede3`
- `claude-pilot-py` HEAD: `86bd3eebc39ac053cd71a7660f793b943958f7fd`

**Source surfaces touched (verbatim quotes at base SHA):**

### `mika/crates/mika-agent/src/well_known_agents.rs` (mika @ 72021b78)

`MIKA_RELAY` struct (`well_known_agents.rs:206-216`):

```rust
pub static MIKA_RELAY: WellKnownAgent = WellKnownAgent {
    name: "mika-relay",
    display_name: "Relay",
    emoji: "🔑",
    soul: MIKA_RELAY_SOUL,
    // Empty: mika-relay uses identity allowlist, not denylist (#815).
    disabled_skills: &[],
    config_toml: Some(MIKA_RELAY_CONFIG),
    identity_source: Some(IdentitySource::Static(MIKA_RELAY_IDENTITY)),
    llm_overrides: &[],
};
```

`MIKA_RELAY_IDENTITY` (`well_known_agents.rs:220-227`):

```rust
const MIKA_RELAY_IDENTITY: &str = "\
name = \"Relay\"\n\
emoji = \"🔑\"\n\
\n\
[skills]\n\
allowlist = [\n\
  \"permission-policy\",\n\
]\n";
```

`MIKA_RELAY_SOUL` at `well_known_agents.rs:828` (multi-line raw string).

`MIKA_RELAY_CONFIG` at `well_known_agents.rs:847` (multi-line raw string).

`WELL_KNOWN_AGENTS` array (`well_known_agents.rs:386`):

```rust
pub static WELL_KNOWN_AGENTS: &[&WellKnownAgent] = &[&MIKA_DEV, &MIKA_QA, &MIKA_RELAY, &MIKA_ARCH];
```

Sentinel comment (`well_known_agents.rs:388-396`):

```rust
/// Platform agents that can be dispatched via `mika ask --agent <peer>` without
/// requiring LLM permission classification. These are intra-platform peers that
/// claude-pilot and mika-relay should structurally allow.
///
/// # Sentinel — cross-language duplication (mika#935, architect F2)
///
/// This list is duplicated in `claude-pilot-py/src/claude_pilot/tier1.py` as
/// `INTRA_PLATFORM_AGENTS`. If this list grows beyond 5 entries OR diverges
/// between languages, escalate to build-time codegen.
```

Test references (`well_known_agents.rs:1421, 1437, 1521, 1737, 1738`) — five tests use `MIKA_RELAY_IDENTITY` / `MIKA_RELAY_CONFIG`. All must be removed or refactored.

### `mika/skills/bundled/permission-policy/` (mika @ 72021b78)

Directory contents at base SHA:

```
mika/skills/bundled/permission-policy/
├── skill.toml
└── system_prompt.md   (69 lines)
```

Build-time discovery: `mika/crates/mika-agent/build.rs` walks `skills/bundled/` and generates `BUNDLED_SKILL_MANIFESTS`. Deleting the directory removes the skill from discovery automatically (no code change in `build.rs`).

### `.claude/claude-pilot.json` (5 copies, identical)

```json
{
  "command": "mika",
  "args": ["--agent", "mika-relay", "ask"],
  "timeout": 120000
}
```

Locations:
- `mika-platform/.claude/claude-pilot.json`
- `mika/.claude/claude-pilot.json`
- `mika-skills/.claude/claude-pilot.json`
- `mika-cloud/.claude/claude-pilot.json`
- `claude-pilot-py/.claude/claude-pilot.json`

### `claude-pilot-py/src/claude_pilot/transport.py` (cp @ 86bd3ee)

The full `invoke_command` async function (lines 45-135). Phase C removes the call from `permissions.py` but the transport function itself can stay (dead code; useful for emergency re-enable via rollback). Decision to delete vs. keep deferred to Phase C's `/ce:work` step.

## Goal

Remove every active reference to `mika-relay` after Phase B has proven the deterministic policy path. Code-, config-, and DB-level deletion. Documentation may retain historical references with deprecation callouts.

## Concrete changes

### Change 1 — `mika/crates/mika-agent/src/well_known_agents.rs`

- Delete `MIKA_RELAY` static (lines 206-216).
- Delete `MIKA_RELAY_IDENTITY` const (lines 220-227).
- Delete `MIKA_RELAY_SOUL` const (line 828 + multi-line body).
- Delete `MIKA_RELAY_CONFIG` const (line 847 + multi-line body).
- Remove `&MIKA_RELAY` from `WELL_KNOWN_AGENTS` array (line 386). New form: `&[&MIKA_DEV, &MIKA_QA, &MIKA_ARCH]`.
- Update or delete the 5 test sites at lines 1421, 1437, 1521, 1737, 1738. Use `cargo test -p mika-agent well_known_agents` to flush all referencing tests.
- The sentinel comment at lines 388-396 about `INTRA_PLATFORM_AGENTS` cross-language duplication stays — that contract still applies to mika-arch/mika-dev/mika-qa. **Per architect NF1: drop the stale `mika-relay` mention from the comment prose.** The duplication-sentinel contract (5-entry threshold, codegen escalation) is unchanged; only the prose mention of mika-relay is updated. Change "claude-pilot and mika-relay should structurally allow" → "claude-pilot should structurally allow."

### Change 2 — `mika/skills/bundled/permission-policy/`

- `git rm -r mika/skills/bundled/permission-policy/`.
- `build.rs` walks the directory at build time; no code change needed. Re-build verifies absence from `BUNDLED_SKILL_MANIFESTS`.

### Change 3 — `permission-policy` references in non-relay code

**Per architect NF6: Phase 0 grep result pinned at base SHA.**

Grep at base SHA `72021b78` produces these hits in `well_known_agents.rs`:

| Line | Context | Disposition in Phase C |
|---|---|---|
| 127 | Inside `MIKA_DEV_IDENTITY` or similar — verify by full slice at `/ce:work` | Audit, likely remove |
| 199 | Doc-comment inside MIKA_RELAY block | Deleted in Change 1 |
| 219 | Doc-comment inside MIKA_RELAY_IDENTITY block | Deleted in Change 1 |
| 226 | Inside `MIKA_RELAY_IDENTITY` string body | Deleted in Change 1 |
| 836 | Inside `MIKA_RELAY_SOUL` string body | Deleted in Change 1 |
| 841 | Inside `MIKA_RELAY_SOUL` string body | Deleted in Change 1 |
| 1088 | Inside a write-capable skills `const` list used for read-only-agent invariant check | **Remove this entry** |
| 1216 | Test assertion: `relay_identity.contains("\"permission-policy\"")` | Deleted with the test in Change 1 |
| 1217 | Test assertion: error message | Deleted with the test in Change 1 |
| 1419 | Comment inside a deleted test | Deleted with the test in Change 1 |

The non-trivial hit is **line 1088** — `"permission-policy"` is listed in a write-capable-skills const used to enforce the read-only-agent invariant. After Phase C, the skill no longer exists, so this entry is dead code. Remove it.

All other hits are either inside MIKA_RELAY_* blocks (deleted in Change 1) or inside relay-specific tests (deleted in Change 1).

Verbatim slice for line 1088 context (`well_known_agents.rs:1085-1095`):

```rust
        "resolve-pr-conflicts",
        "self-check",
        // QA skills that front pr_merge/run_gh-write
        "qa-review",
        "qa-review-build-callback",
        // Skill management
        "skill-review",
        // Permission policy executes side-effects
        "permission-policy",
    ];
```

Remove the `// Permission policy executes side-effects` comment and the `"permission-policy"` entry. Pre-implementation diff in PR description enumerates this and confirms no other unaccounted hits emerged between base SHA and merge-time.

### Change 4 — `.claude/claude-pilot.json` (5 copies)

Delete in all five locations. Phase B verified `transport.py` handles missing config gracefully (B-AC5). If any of the 5 files diverged during Phase B's soak window (unlikely but possible), the PR description must enumerate the diff before deletion.

### Change 5 — Delete `claude-pilot-py/src/claude_pilot/transport.py`

**Per architect F2: committed to full deletion (was 5b in the original plan's fork).**

- `git rm claude-pilot-py/src/claude_pilot/transport.py`.
- `git rm claude-pilot-py/tests/test_transport.py` (and any other test referencing transport directly).
- Remove the import of `transport` from `permissions.py`. After Phase B, the call to `transport.invoke_command` was already gated behind `MIKA_PILOT_POLICY_DISABLED`; Phase C removes both the gate and the dead-code branch.
- Remove `MIKA_PILOT_POLICY_DISABLED` env-var reading (Phase B's emergency rollback lever). Phase C is itself the irreversible step — the lever has nowhere to roll back to. If someone tries to set the var post-deploy, ignore silently (no-op env vars don't warrant warning noise).

Reasoning for full deletion (the architect's F2 explicit ratification):
- Phase C's purpose is "remove every active reference to mika-relay." `transport.py` is the entire relay-invocation surface.
- The `[claude-pilot] ` payload prefix and the comment citing "relay's parsing convention" in `transport.py` would fail C-AC1's grep (`rg -i "mika-relay" ... -t python` → zero hits).
- The dead-code defensive-keep argument was valid during Phase B's soak. After 7+ days post-Phase-B without rollback, the file is provably unused.
- Future subprocess transports (hypothetical non-relay consumers) can reintroduce a transport module then — YAGNI handles that.

C-AC1's grep now passes by construction.

### Change 6 — DB cleanup migration

Provide a Rust migration under `mika/crates/mika-agent/src/db/migrations/` (or `mika/migrations/`, per workspace convention — verify at `/ce:work` time). Schema version v36 (next after v35 KG outcome expansion at #1154).

**Per architect F1: explicit per-table DELETEs in reverse-dependency order — not cascading.**

SQLite's `ON DELETE CASCADE` requires `PRAGMA foreign_keys = ON` per-connection, off by default. Rather than depending on the migration runner's connection settings (which would need to be pinned in Phase 0), the migration is self-contained and correct regardless of pragma state:

```sql
-- v36: mika#1193 retire mika-relay agent.
-- Self-contained: explicit deletes in reverse-dependency order. Correctness does
-- NOT depend on PRAGMA foreign_keys being ON.

DELETE FROM tool_calls
  WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

DELETE FROM llm_calls
  WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

DELETE FROM messages
  WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

DELETE FROM tasks
  WHERE agent_id = 'mika-relay';

DELETE FROM sessions
  WHERE agent_id = 'mika-relay';

DELETE FROM agents
  WHERE id = 'mika-relay';
```

Order matters: descendants before ancestors. Each statement is idempotent (no-op if rows are already gone).

Verify table coverage at `/ce:work` time by running `pragma_foreign_key_list` for each table that has an `agent_id` or `session_id` column; the PR description must include the enumeration as evidence the reverse-dependency list above is exhaustive at base SHA.

**Migration runtime safety gate (per architect NF7):** before executing the DELETEs, the migration prints:

```
WARN: This migration deletes all mika-relay data (sessions, messages, tool_calls,
llm_calls, tasks, agents). This is non-reversible. Back up ~/.mika/data/mika.db
before proceeding.

Set MIKA_MIGRATION_CONFIRMED=1 to continue.
```

If `MIKA_MIGRATION_CONFIRMED` is not set, the migration exits non-zero without modifying data. Operator must explicitly opt in. This is a one-time gate; the env var is read in the migration body, not at process startup. Five lines of code; converts the PR-description backup-reminder into a runtime safety net.

Migration is non-reversible. PR description still includes the backup-snapshot reminder, redundant with the runtime gate by design.

### Change 7 — Documentation updates

- `mika-platform/CLAUDE.md` — dev-loop diagram (if it mentions relay); remove standalone relay references; update the well-known-agents enumeration.
- `mika/CLAUDE.md` — well-known-agents list; `MIKA_DEV_MODE` description.
- Historical files (`docs/plans/721-dedicated-mika-relay-agent.md`, `docs/solutions/best-practices/*relay*`) retain references; add a single deprecation callout to the original `721-*.md` plan: "Superseded by mika#1188 milestone (2026-05-17)." No retroactive edits to other solution docs.

## Acceptance criteria

- **C-AC1.** `rg -i "mika-relay" mika/ claude-pilot-py/ mika-skills/ mika-cloud/ -t rust -t python -t toml -t json` returns zero hits. Doc-only (`.md`) references retained with deprecation callouts.
- **C-AC2.** `cd mika && cargo test -p mika-agent` passes. `cd claude-pilot-py && uv run pytest` passes. No broken test fixtures or eval-harness references.
- **C-AC3.** 7-day post-deploy soak: `sqlite3 ~/.mika/data/mika.db "SELECT count(*) FROM messages WHERE agent_id='mika-relay' AND created_at > '<deploy-date>'"` → 0. No fabrication-class failures in autonomous-loop runs (original milestone-level AC4).
- **C-AC4.** DB cleanup migration runs idempotently on operator's `mika.db`; row counts for `mika-relay` agent rows = 0 across all referencing tables post-run. Schema version increments to v36 (or next available).
- **C-AC5.** `make deploy` builds cleanly; `mika status` shows agents = `{mika-dev, mika-qa, mika-arch}` only.

## Risks

- **Consumer outside claude-pilot still calls `mika-relay`.** Before merging, grep all repos for `--agent mika-relay` and `agent_id='mika-relay'`. Any hit is a blocker — route caller to tier1 + policy or escalate.
- **DB cascade misses a table.** Schema v35 has 30+ tables; some may reference `agent_id` without FK. Mitigation: dry-run on production-data copy; PR description enumerates affected tables.
- **`build.rs` build cache.** After deleting `permission-policy/`, dev environments may need `cargo clean` to regenerate `BUNDLED_SKILL_MANIFESTS`. CI builds from scratch — production unaffected.
- **Multi-repo config drift between Phase B and Phase C.** Phase B leaves the 5 `claude-pilot.json` files in place. If someone edits one during the 7-day soak, Phase C's PR notices and surfaces the diff.
- **Test refactor surface area.** 5 test sites reference `MIKA_RELAY_IDENTITY` / `MIKA_RELAY_CONFIG`. Some may be reusable as documentation patterns for the remaining three agents; others should just be deleted. Per-test triage in `/ce:work`.

## Out of scope

- Replacing `transport.py` infrastructure with an alternative subprocess transport (no other consumer).
- New CLI verbs (Phase B introduced `mika notify`; Phase C consumes it indirectly via Phase B's escalation path).
- Renaming `INTRA_PLATFORM_AGENTS` or restructuring the cross-language duplication sentinel (still 3 entries < 5 threshold).

## Verification

- `cargo test -p mika-agent` — green.
- `cargo clippy -p mika-agent` — no new warnings.
- `make deploy` — clean build; `mika status` reports 3 well-known agents.
- DB query: `sqlite3 ~/.mika/data/mika.db "SELECT count(*) FROM agents WHERE id='mika-relay'"` → 0 post-migration.
- 7-day post-deploy soak metric: messages from `mika-relay` = 0; autonomous-loop incident log = 0 relay-class entries.

## Rollback

Single PR revert restores the relay agent code. DB cleanup is **non-reversible** without a snapshot — operators must back up `~/.mika/data/mika.db` before running the migration. PR description's "Post-Deploy Monitoring" section is explicit on this.

## Sequencing

Depends on Phase B (mika#1192) merged + soaked ≥7 days. Cannot ship before B's policy file is the only dispatch path in production for a full week.

## Related

- Parent milestone: mika#1188
- Depends on: mika#1191 (Phase A), mika#1192 (Phase B)
- mika#1161 — relay drift incident (motivation)
- mika#935 — Rust pre-classifier (engine-side; unaffected by this cleanup but the cross-language sentinel remains relevant)
- `mika/docs/plans/721-dedicated-mika-relay-agent.md` — original plan that created `mika-relay`; receives a "Superseded by mika#1188" deprecation callout in Change 7
