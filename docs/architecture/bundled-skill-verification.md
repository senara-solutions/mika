# Bundled-skill structural verification (`make verify-bundled-skills`)

**Status:** active · **Introduced:** mika#1575 · **Counterpart to:** mika#1326 AC2

`make verify-bundled-skills` is a build-time / pre-merge gate that asserts structural
invariants on the engine-coupled bundled skills under `skills/bundled/`. It is the
**structural counterpart to mika#1326 AC2**: AC2 catches cross-skill tool-name
*collisions*; this gate catches *incomplete skill-adds* — the gap between "the skill
bundle's files exist" and "the skill bundle works in production".

## Why it exists

mika#1282's post-flight dirty-worktree recovery rescues uncommitted skill-add content
into a draft PR marked `PIPELINE_INCOMPLETE`. The operator then had to **hand-verify**
that the rescued skill bundle was structurally complete — files present, manifest parses,
handlers wire up, `required_tools` tokens consistent, identity allowlist coherent. Peer
review on mika#1326/#1569/#1570 named that precisely:

> "operator reads diff for completeness puts a human on a mechanizable, silent-failure
> check — which is precisely why AC2 is a test and not a code-review guideline."

This gate mechanizes that review. After it lands, the operator's task on a rescued draft
PR shrinks from "verify structure" to "verify content" — the structure is machine-checked.

## Where it sits in the silent-failure defense

Four layers cover the same invariant class at different times:

| Layer | When | Catches |
|-------|------|---------|
| mika#1326 AC2 (`test_bundled_skills_no_cross_skill_tool_name_collision`) | build-test | cross-skill tool-name collisions |
| **`verify-bundled-skills` (this gate)** | **build-time / pre-merge** | **incomplete skill-adds (missing files, unresolvable `required_tools`, allowlist incoherence)** |
| mika#1576 `apply_load_safety_check` coherence guard | runtime (load) | allowlist ↔ `required_tools` coherence at the layer where both are visible |
| mika#516 availability filter | runtime (per-turn) | a `required_tool` not in the resolved surface at call time |

## The five checks

Run against the on-disk `skills/bundled/` tree (the source files a PR changes), reusing
the canonical walker `crates/mika-agent/build_support/bundled_skills_discover.rs`.

1. **Bundle completeness.** Every skill directory (excluding `_shared/` and dotfiles) has
   `skill.toml` **and** `system_prompt.md`.
   - *Predicate note (mika-arch session `d8b4c839`, Decision A):* the `required_tools ⇒
     tools.json` coupling proposed in the original ticket text was **dropped as unsound** —
     a `required_tools` token may legitimately resolve via a builtin or another skill
     (see Check 4), so its presence cannot proxy "this skill ships its own `tools.json`".
     Token validity is owned by Check 4, not Check 1. (`self-dev` and `dev-handsoff` both
     declare `required_tools` with no `tools.json` and are correct designs.)

2. **Manifest parses + minimum fields.** `skill.toml` is valid TOML and declares
   `[skill].name`, `[skill].version`, and — unless `always_on = true` — a `[triggers].keywords`
   **field** (presence, not non-emptiness: `skill-review` deliberately declares
   `keywords = []` as a programmatically-invoked handler skill, #265).

3. **Tool handler resolution.** For each `tools.json` tool: a `builtin` handler's `function`
   is one of `skills::builtin_handlers::KNOWN_BUILTINS` (the handler-dispatch set); an
   `exec` handler's `command` file exists under the skill directory and is executable on disk.

4. **`required_tools` token consistency (allowlist-unaware).** Each `[constraints].required_tools`
   token resolves to *something real somewhere* in the bundled surface: a builtin tool name
   (`tools::BUILTIN_TOOL_NAMES`) **or** a tool declared in **any** bundled skill's `tools.json`
   (`bundled_skills::all_bundled_tool_names()`, which spans both community and engine-coupled
   skills — so e.g. `qa-review`'s `run_shell`, provided by the community `shell-exec` skill,
   resolves). Catches typos and references to nonexistent tools.

5. **Identity allowlist coherence (allowlist-scoped).** For each well-known agent's
   `[skills].allowlist` (`well_known_agents::well_known_skill_allowlists()`): every allowlisted
   name is a bundled skill (`is_bundled_skill`), and every `required_tools` token of an
   allowlisted engine-coupled skill is *reachable through that allowlist* — a builtin, or a
   tool declared by a skill **in the same allowlist**. This is the allowlist-scoped reachability
   Check 4 deliberately defers: Check 4 says "exists somewhere"; Check 5 says "reachable here".

## Fire-disposition and `KNOWN_EXCEPTIONS`

Per mika#1575 F2, all five checks ship **green** against the current source tree, and the
`KNOWN_EXCEPTIONS` constant ships **empty**. `KNOWN_EXCEPTIONS` mirrors
`KNOWN_PRE_EXISTING_COLLISIONS` (bundled_skills.rs, mika#1326 AC2): each entry names the
check, skill, a substring of the failure reason, and a resolution ticket. It is
**self-cleaning** — the binary and a test fail if an entry no longer matches a real failure,
forcing removal of stale exceptions. A non-empty `KNOWN_EXCEPTIONS` MUST be enumerated in the
PR description with each entry's resolution ticket.

## Running it

```bash
make verify-bundled-skills          # human-readable gate; exit 0 = green, non-zero = fail
cargo test --bin verify-bundled-skills   # unit + real-tree green test
```

CI runs `make verify-bundled-skills` as a step in the `check` job on every PR; a non-zero
exit blocks merge. The real-tree invariant is *also* enforced by the
`real_bundled_tree_passes_green` test inside `cargo test`, so the gate has two independent
CI entry points.

## Runtime sibling: the load-time coherence guard (mika#1576)

The build-time gate above proves the *source tree* is coherent. It cannot prove a *running
agent* is coherent, because the effective skill set is computed at startup from the agent's
`identity.toml [skills].allowlist` plus DB and transient overrides — state the build-time
gate never sees. `SkillRegistry::apply_required_tools_coherence_check(agent_id)` closes that
gap.

**The invariant:** an agent must not hold a loaded skill that requires a tool it can't call.

**Why a separate layer is needed.** The per-turn `required_tools` gate (mika#516) is
keyword-scoped — it only fires when a skill keyword-matches *and* the LLM is asked to use the
tool. A structurally-broken allowlist↔`required_tools` pairing (e.g. a skill requiring a tool
provided by a skill the allowlist swap removed — the mika#1406 `github`→`gh-read-only`
scenario) passes every existing check *vacuously* and surfaces only mid-work, when the agent
reaches for a tool that isn't there. Counter: `invariant-enforced-at-dispatch-layer-not-load-layer`.

**Where it runs.** Immediately after `apply_load_safety_check()` at every registry-finalize
site (`crates/mika-cli/src/commands/{ask,chat}.rs`, `crates/mika-agent/src/server/mod.rs`,
and the `list_skills` tool). This is the one point in the load chain where both operands —
the allowlist (already applied) and each surviving skill's `required_tools` — are
simultaneously visible.

**The rule.** For each loaded skill, every `[constraints].required_tools` token must resolve to:

- a builtin tool name — the **full** `tools::BUILTIN_TOOL_NAMES` (which subsumes
  `builtin_handlers::KNOWN_BUILTINS` via the mika#1217 parity test); **the same builtin set
  Check 4 / Check 5 use**, so the load-time and build-time checks never disagree about what
  counts as a builtin; or
- a tool declared in the `tools.json` of some **loaded** skill (allowlist-aware — only
  surviving skills contribute). This makes it the runtime sibling of **Check 5**, not Check 4:
  Check 4 is allowlist-unaware ("exists somewhere"); Check 5 and this guard are allowlist-aware
  ("reachable here").

`mcp__{server}__{tool}` tokens are exempt — they resolve through the MCP client at startup,
not the builtin/skill surface, so firing on them would wrongly skip a skill that can in fact
call the tool (mirrors `validate_skill` step 5b's leniency).

**Fire disposition.** On a fire, the offending skill is **skipped** (evicted, recorded in
`SkillRegistry::skipped()`) and an error-level `required_tool_unresolvable` structured event
is emitted with fields `agent_id`, `skill`, `unresolvable_token`, `available_tool_count`. The
agent starts **degraded** — the broken skill is unavailable until the operator fixes the
coherence violation (adds the providing skill to the allowlist, or drops the dangling token)
and restarts. This is the established `apply_load_safety_check` "load with warning + skip the
broken skill" pattern, **not** refuse-to-start.

**First-deploy expectation (mika#1576 F2).** All existing well-known agent configurations pass
clean. The regression test `well_known_agents::tests::test_well_known_agents_pass_required_tools_coherence`
seeds the full bundled library, applies each well-known allowlist, and asserts zero coherence
fires — the runtime sibling of Check 5's build-time guarantee.

**CLI surface.** `mika skills validate` reports a `[WARN]` coherence diagnostic for any
`required_tools` token that resolves to nothing in the installed-skill surface. The CLI view is
allowlist-*unaware* (disk-surface, like Check 4), so it emits **Warn, not Fail** — a token may
be provided by a dependency that isn't installed in a standalone validate tree (e.g. community-
skill CI) yet resolves fine at runtime; hard-failing there would break that CI. This matches
the Warn severity of `validate_skill` step 5b and Check 4 for the same allowlist-unaware class.
The per-agent runtime guard above is the allowlist-aware authority that hard-skips.

## Out of scope

Marketplace skill verification (bundled only), `tools.json` schema validation against the
canonical tool schema, and the optional checks 6–8 (prompt-size limits, keyword-overlap
detection, schema validation) are deferred follow-ups.
