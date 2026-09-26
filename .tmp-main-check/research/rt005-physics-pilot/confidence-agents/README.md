# RT-005 physics pilot — `mika-dev-confidence-{high,low}` (brick 2/5)

Two derived `mika-dev` agent configs that are **byte-identical except for one
out-of-band confidence prior block** in their system prompt. They realize the
*injected confidence* factor of the RT-005 physics pilot's 2×2 design.

- `mika-dev-confidence-high` — prior belief **0.95** in peer_b's reliability
- `mika-dev-confidence-low` — prior belief **0.55** in peer_b's reliability

Context: RT-005 studies the interaction of *injected confidence* × *real
reliability* in an agent's planning behavior. `--agent` is an **identity
selector, not a knob** (mika-arch mechanism review), so the confidence factor is
realized as two derived agent configs rather than a runtime flag. Ticket:
mika issue#1888. Pilot protocol: `~/.claude/plans/round-table-005-physics-protocol-2026-07-28.md`.
Implementation plan: `docs/plans/2026-08-03-001-feat-1888-confidence-agent-configs-plan.md`.

This is **scaffold, not keeper** (RT-005 coherence-seat doctrine): a bounded,
reversible experiment realized entirely as committed config assets + a
provisioning script, with **zero engine/Rust changes**.

## Layout

```
confidence-agents/
├── README.md                 # this file
├── shared/
│   ├── identity.toml         # template (name/emoji substituted per level)
│   ├── config.toml           # identical for both agents
│   └── soul-base.md          # shared persona base (the identical remainder)
├── prior-high.md             # THE differing block — confidence 0.95
├── prior-low.md              # THE differing block — confidence 0.55
└── provision.sh              # idempotent assembler → writes both agent dirs
```

Placed under top-level `research/` (not `docs/`) on purpose: this is experiment
scaffolding, not product documentation, and `docs/` carries a build-time
doc-sync contract (`build.rs` / `scripts/sync-agent-docs.sh`) this brick must
not touch.

## Reproduce

```bash
bash research/rt005-physics-pilot/confidence-agents/provision.sh
```

Idempotent — re-running overwrites the three generated files
(`config.toml`, `identity.toml`, `soul.md`) deterministically in each agent dir
under `~/.mika/agents/mika-dev-confidence-{high,low}/`. Override the target root
with `MIKA_AGENTS_ROOT=<dir>` (used by the smoke test below).

## Verify AC1 — diff = only the confidence prior block

`provision.sh` self-asserts this: after assembling both `soul.md` files it
`diff`s them and fails loudly unless every changed line lives inside the
confidence prior (i.e. mentions `0.95` or `0.55`). You can also see the delta
directly in the committed source — the base is literally shared, so:

```bash
cd research/rt005-physics-pilot/confidence-agents
diff <(cat shared/soul-base.md prior-high.md) <(cat shared/soul-base.md prior-low.md)
# reduces to:
diff prior-high.md prior-low.md   # a single numeric change: 0.95 ↔ 0.55
```

**Display-name exception.** `identity.toml`'s `name` necessarily differs
(`Mika Dev Confidence High` vs `... Low`), and `emoji` differs (`▲` vs `▼`), so
`mika agents list` stays unambiguous. These are **display metadata, not the
behavioral prior** — they never enter the confidence treatment. AC1 ("diff = only
the confidence prior block") is scoped to the behavioral config: `config.toml` is
byte-identical, and `identity.toml` is byte-identical apart from these two
documented display fields.

## Verify AC2 — confidence is injected out-of-band (system prompt, not user message)

The prior lives **only** in `soul.md`. At prompt assembly, `soul.md` content is
written verbatim into the **system prompt** by `write_soul_section`
(`crates/mika-agent/src/prompt.rs:505-509`), which `build_system_prompt` calls
first at `crates/mika-agent/src/prompt.rs:578` — before core memory and skills,
and **never** in the user message. So the confidence prior can never contaminate
the task text the agent is asked to solve.

## Verify AC3 — both agents invocable

After `provision.sh`, each agent dir carries a `config.toml`, which is the only
key `mika ask` needs to resolve an agent — `ensure_initialized_for_agent` gates
solely on `config.toml` presence (`crates/mika-cli/src/init.rs:172-196`). No
compiled `WELL_KNOWN_AGENTS` entry is required.

```bash
mika ask --agent mika-dev-confidence-high "<probe>"
mika ask --agent mika-dev-confidence-low  "<probe>"
```

## Teardown (reversibility)

```bash
rm -rf ~/.mika/agents/mika-dev-confidence-high ~/.mika/agents/mika-dev-confidence-low
# or: mika agents delete mika-dev-confidence-high && mika agents delete mika-dev-confidence-low
```

No engine state is touched, so teardown is a plain directory removal.

## Out of scope

The manipulation check (does the 0.95/0.55 prior actually move behavior?) is a
**run-time** gate owned by pilot execution (RT-005 bricks 3+), not by config
creation. This brick only guarantees the knob *exists* and is wired out-of-band.
