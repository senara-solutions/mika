# Plan — `mika-dev-confidence-{high,low}` agent configs (RT-005 brick 2/5)

**Ticket:** mika issue#1888
**Type:** feat (research scaffold)
**Branch:** `feat/1888/research-mika-dev-confidence-high-low`

## Context

RT-005 physics pilot (ratified 2026-07-28, `~/.claude/plans/round-table-005-physics-protocol-2026-07-28.md`) studies the **interaction** of *injected confidence* × *real reliability* in an agent's planning behavior. The 2×2 design needs a confidence knob. mika-arch's mechanism review established that `--agent` is an **identity selector, not a knob** — so the confidence factor is realized as **two derived agent configs** that are byte-identical except for one out-of-band prior block in the system prompt:

- `mika-dev-confidence-high` — prior belief **0.95** in peer_b's reliability
- `mika-dev-confidence-low` — prior belief **0.55** in peer_b's reliability

This is **brick 2/5**, independent (wave 1). It does **not** depend on brick 1 (`peer_b` module), brick 3 (orchestration), brick 4 (token instrumentation), or brick 5 (mechanism analyzer). Deliverable scope is purely: the two configs exist, differ only by the prior, and are invocable.

## Design decision — scaffold, not compiled well-known agents

Three provisioning routes exist (from codebase investigation):

| Route | Mechanism | Verdict |
|---|---|---|
| A. Compiled well-known agents | Add two entries to `WELL_KNOWN_AGENTS` (`crates/mika-agent/src/well_known_agents.rs`) | **Rejected** — permanent engine infra for a bounded, jettisonable experiment. RT-005 doctrine (coherence seat): peer scaffold is *scaffold, not keeper* → do not build standing infra. |
| B. New `PromptContext.confidence_preamble` field | Thread a per-agent free-text block through prompt assembly | **Rejected** — over-engineering. `soul.md` is **already** the out-of-band system-prompt channel (`write_soul_section`, `prompt.rs:505,578`). YAGNI. |
| **C. Committed templates + idempotent provisioning script** | Assemble two on-disk agent dirs under `~/.mika/agents/` from committed template files | **Chosen** — no engine change, reproducible, reversible, and the PR diff *itself* proves AC1 (only the prior block differs). |

**Why C is correct here.** The out-of-band injection point is a solved problem: `soul.md` content is written verbatim into the system prompt via `write_soul_section` (`crates/mika-agent/src/prompt.rs:505-507`, called at line 578), *after* core memory and *before* skills — never in the user message. Any agent dir carrying a `config.toml` is invocable via `mika ask --agent <name>` (`crates/mika-cli/src/init.rs:172-196`; no compiled allowlist — many custom agents already live on disk). So the entire ticket is satisfiable with committed config assets + a provisioning script, zero Rust changes. This keeps the research brick a clean, reversible scaffold.

## Requirements

1. A committed, byte-diffable pair of soul prior blocks — the **only** thing that differs between the two agents.
2. Shared, committed `identity.toml`, `config.toml`, and base `soul.md` — identical for both agents (derived from the `mika-dev` shape per mika-arch's "2 configs agent dérivées").
3. An idempotent provisioning script that assembles and writes both agent dirs under `~/.mika/agents/mika-dev-confidence-{high,low}/`.
4. The prior lives **only** in `soul.md` (system-prompt channel) — never in a user-message path.
5. Both agents invocable via `mika ask --agent mika-dev-confidence-{high,low}`.
6. Documented reproduction + teardown (reversibility).

## Implementation

### File layout (new, repo-root `research/` subtree — outside `docs/` to avoid doc-sync entanglement)

```
research/rt005-physics-pilot/confidence-agents/
├── README.md                 # repro + teardown + AC mapping
├── shared/
│   ├── identity.toml         # identical for both agents
│   ├── config.toml           # identical for both agents (openrouter glm-5.2, mika-dev base)
│   └── soul-base.md          # shared persona base (the identical remainder)
├── prior-high.md             # THE differing block — confidence 0.95
├── prior-low.md              # THE differing block — confidence 0.55
└── provision.sh              # idempotent assembler → writes both agent dirs
```

Placed under a top-level `research/` dir (not `docs/`) because these are experiment scaffolding, not product docs, and `docs/` carries a build-time doc-sync contract (`build.rs` / `scripts/sync-agent-docs.sh`) we don't want to touch.

### Step 1 — Shared config assets (`shared/`)

- **`shared/config.toml`** — copy the `mika-dev` shape: `llm_provider = "openrouter"`, `openrouter_model = "z-ai/glm-5.2"`, `llm_max_tokens = 8192`, `log_level = "info"`. Committed as a literal so the two agents are reproducibly identical and not coupled to `mika-dev`'s (reconcilable) on-disk state.
- **`shared/identity.toml`** — `name`/`emoji` set per-agent by the script (display names differ, but that is metadata, not the prior — see note below); `[kg] enabled = false`; `[reflection] enabled = false`; `[skills].allowlist` = the `mika-dev` allowlist. Both agents get the **same** allowlist.
- **`shared/soul-base.md`** — the shared persona base. Start from `mika-dev`'s soul (role, boundaries, evidence→action), which is the "identité de base" both agents share.

> **Note on `name`/`emoji`:** the `identity.toml` `name` field necessarily differs (`Mika Dev Confidence High` vs `... Low`) — this is a display label, not a behavioral input, and is not part of the system-prompt prior. AC1 ("diff = only the confidence prior block") is scoped to the **behavioral** config. The script keeps `identity.toml` otherwise byte-identical and documents this display-name exception in the README so the reviewer isn't surprised. (Alternative considered: give both the *same* `name` — rejected, it makes `mika agents list` ambiguous.)

### Step 2 — The prior blocks (`prior-high.md`, `prior-low.md`)

The two files are structurally identical, differing only in the confidence value and the high/low label. Framed as a **belief about peer_b's reliability** (a prior), not a directive about verification effort — this is the round-2 reformulation that dissolves the tautology (RT-005 finding: an instruction to "verify less" would smuggle the outcome into the treatment).

`prior-high.md`:
```markdown
## Prior on peer_b reliability

You have a collaborator, **peer_b**, that proposes answers to sub-problems.
Your prior belief about peer_b's reliability: you estimate a **0.95**
probability that any given peer_b answer is correct.
```

`prior-low.md` — identical except **0.95 → 0.55**.

The assembled `soul.md` = `soul-base.md` + `\n` + `prior-{high,low}.md`. Because the base is literally shared, `diff <(cat soul-base.md prior-high.md) <(cat soul-base.md prior-low.md)` reduces to `diff prior-high.md prior-low.md` — a single-line delta.

### Step 3 — Provisioning script (`provision.sh`)

Idempotent, no dependency on `mika-dev`'s live on-disk state:

1. For each of `{high, low}`:
   - `AGENT_HOME="$HOME/.mika/agents/mika-dev-confidence-$level"`; `mkdir -p`.
   - Write `config.toml` from `shared/config.toml` (identical).
   - Write `identity.toml` from `shared/identity.toml` with the per-level `name`/`emoji` substituted.
   - Write `soul.md` = `shared/soul-base.md` + `prior-$level.md`.
2. Self-assert AC1: `diff` the two assembled `soul.md` files and confirm the only differing lines are inside the prior block (fail loudly otherwise).
3. Print the two `mika ask --agent ...` invocation lines for the operator.

Re-running the script overwrites the three generated files deterministically (idempotent). No `mika agents create` dependency — the script writes the minimal file set directly, which is sufficient for `mika ask` resolution (`config.toml` presence is the resolution key).

### Step 4 — README (`README.md`)

- One-paragraph context + link to the RT-005 plan and this plan.
- **Reproduce:** `bash research/rt005-physics-pilot/confidence-agents/provision.sh`.
- **Verify AC1:** `diff` command (script also self-asserts).
- **Verify AC3:** `mika ask --agent mika-dev-confidence-high "<probe>"` and `...-low`.
- **Teardown (reversibility):** `rm -rf ~/.mika/agents/mika-dev-confidence-{high,low}` (or `mika agents delete`).
- The display-name exception note (Step 1).

## Verification contract

- **AC1 (diff = only prior block):** `provision.sh` self-asserts via `diff` of the two assembled `soul.md`; the committed `prior-high.md`/`prior-low.md` pair makes the delta reviewable directly in the PR (a single numeric change 0.95↔0.55). `identity.toml`/`config.toml` written identically (modulo the documented display-name field).
- **AC2 (out-of-band, in system_prompt):** the prior is placed only in `soul.md`, which flows into the system prompt via `write_soul_section` (`crates/mika-agent/src/prompt.rs:505-507`, invoked at line 578) — never into a user message. Cited in the README.
- **AC3 (both invocable):** after `provision.sh`, both dirs carry `config.toml`, satisfying `ensure_initialized_for_agent` (`crates/mika-cli/src/init.rs:172-196`); documented probe commands confirm live invocation.
- **No engine regression:** zero Rust changes — `cargo build`/`cargo test` unaffected. The script is shell-only; `bash -n provision.sh` (syntax) + one local run is the check.

## Definition of Done

- `research/rt005-physics-pilot/confidence-agents/` committed with `shared/{identity.toml,config.toml,soul-base.md}`, `prior-{high,low}.md`, `provision.sh`, `README.md`.
- `provision.sh` runs idempotently and self-asserts the single-block diff.
- Both agents invocable via `mika ask --agent mika-dev-confidence-{high,low}` (documented probe).
- No Rust/engine changes; no `docs/` doc-sync impact.
- README documents reproduce + teardown + the AC2 out-of-band citation + the display-name exception.

## Out of scope (other RT-005 bricks)

- `peer_b` internal module (brick 1), orchestration script (brick 3), token-usage instrumentation patch (brick 4), offline mechanism analyzer (brick 5).
- The manipulation check (does the 0.95/0.55 prior actually move behavior?) — a **run-time** gate owned by the pilot execution (brick 3+), not by config creation. This brick only guarantees the knob *exists* and is wired out-of-band.

## Acceptance criteria

Transcribed verbatim from mika issue#1888:

- [ ] 2 configs créées, diff = uniquement le bloc-prior de confiance.
- [ ] Confiance injectée **hors-bande** (pas dans le message utilisateur — dans le system_prompt), pour ne pas contaminer la tâche.
- [ ] Les 2 agents restent invocables via `mika ask --agent mika-dev-confidence-{high,low}`.
