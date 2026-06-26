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

## Out of scope

Runtime invariants (`apply_load_safety_check`, mika#1576), marketplace skill verification
(bundled only), `tools.json` schema validation against the canonical tool schema, and the
optional checks 6–8 (prompt-size limits, keyword-overlap detection, schema validation) are
deferred follow-ups.
