---
title: "feat(ci,skills): make verify-bundled-skills target — structural counterpart to AC2"
date: 2026-06-26
type: feat
issue: mika#1575
branch: feat/1575/ci-skills-make-verify-bundled-skills
groomed_by: mika-arch session 1874bb55 (F1/F2/F3 govern)
status: check-1 predicate pending mika-arch third-pass ratification (see Open Decisions)
---

# feat(ci,skills): `make verify-bundled-skills` — structural counterpart to AC2

## Summary

Add a `verify-bundled-skills` binary + `make verify-bundled-skills` target + a CI gate that
asserts structural invariants on the engine-coupled bundled skills under `skills/bundled/`. This is
the **pre-merge structural counterpart to mika#1326 AC2**: AC2 catches cross-skill tool-name
collisions at build-test time; this gate catches *incomplete skill-adds* (missing bundle files,
unresolvable `required_tools` tokens, identity-allowlist incoherence) at pre-merge time, so the
operator's "review the rescued draft PR for completeness" task (mika#1282 dirty-worktree recovery)
becomes "review for content correctness" — structure is already machine-verified.

Five checks (1–5). The binary fails non-zero with explicit per-failure errors; CI blocks merge on
failure. Per F2, all five checks ship **green on current data** and `KNOWN_EXCEPTIONS` ships
**empty**.

---

## Problem Frame

mika#1282's post-flight dirty-worktree recovery rescues uncommitted skill-add content into a draft
PR marked PIPELINE_INCOMPLETE. The operator must then hand-verify that the rescued skill bundle is
structurally complete (files present, manifest parses, handlers wire up, `required_tools` tokens
consistent, identity allowlist coherent). Peer review on mika#1326/#1569/#1570 named this precisely:
*"operator reads diff for completeness puts a human on a mechanizable, silent-failure check — which
is precisely why AC2 is a test and not a code-review guideline."* The durable answer is a build-time
/ pre-merge structural gate — this ticket.

---

## Requirements traceability

| AC | Requirement | Unit |
|----|-------------|------|
| AC1 | `make verify-bundled-skills` target runs the check binary | U4 |
| AC2 | `verify-bundled-skills` binary at `crates/mika-agent/src/bin/verify_bundled_skills.rs` implementing checks 1–5 | U1, U2, U3 |
| AC3 | CI gate in `.github/workflows/ci.yml` runs the target on every PR; failure blocks merge | U5 |
| AC4 | Tests for the verify binary — fixtures covering each PASS/FAIL case for checks 1–5 | U6 |
| AC5 | Documentation at `docs/architecture/` describing the verify target as the structural counterpart to AC2 | U7 |

---

## Open Decisions

**OD-1 — RESOLVED 2026-06-26 (mika-arch third-pass session `d8b4c839`): Decision (A) RATIFIED.**
Drop the `required_tools ⇒ tools.json` coupling. **Check 1 = `skill.toml` + `system_prompt.md`
presence only.** OD-2 (check-4 option (a) union) explicitly approved ("no architect objection").
Routing chain: Mika Prime (session `0000…`) ruled this architect-scope and routed to mika-arch with
the recommendation below, which mika-arch ratified. U1 is now unblocked.

**OD-1 (original framing) — Check 1 `required_tools ⇒ tools.json` coupling.**
Grounding against real data found a forced contradiction: F3 check-1 ("`required_tools` present +
no `tools.json` → FAIL") fires on `self-dev` (`required_tools = ["run_claude_pilot"]`, no
`tools.json`) and `dev-handsoff` (`required_tools = ["write_agent_file"]`, no `tools.json`), which
violates F2 ("all five green on first run"). The only literal fix (add a `tools.json` declaring
those tools) re-declares `run_claude_pilot` (already in `dev-pilot/tools.json`) → trips mika#1326
AC2 collision. `KNOWN_EXCEPTIONS` is semantically wrong (these are correct design, not bugs; the
self-cleaning assertion would pin them forever). Mika Prime ruled this **architect-scope** (it
overturns a ratified F3 predicate) and routed it to mika-arch with the recommendation below.

- **Recommended (pending ratification):** Check 1 validates `skill.toml` + `system_prompt.md`
  presence ONLY; drop the `required_tools ⇒ tools.json` coupling. Tool-reference validity is owned
  entirely by Check 4 (F1's allowlist-unaware resolution). Rationale: F1 permits `required_tools` to
  resolve via a builtin or *any other* bundled skill's `tools.json`, so `required_tools` presence
  cannot validly proxy "this skill ships its own `tools.json`" — the proxy is circular.
- **U1 (Check 1) MUST NOT be implemented until mika-arch ratifies.** Build U2/U3/U4/U5/U6(2-5)/U7
  first. When the ruling lands, implement U1 per the ratified predicate and the matching tests.

**OD-2 — Check 4 option (a) breadth (carried on implementer warrant — Prime ruled spec-completion).**
F1 option (a) names "a builtin handler registered in `builtin_handlers.rs`" but core `ToolRegistry`
builtins (e.g. `write_agent_file`) live in a different registry and are not in
`builtin_handlers.rs`'s 7-fn dispatch (`KNOWN_BUILTINS`). Broaden option (a) to the **union of
`builtin_handlers::KNOWN_BUILTINS` + the core `ToolRegistry` default tool-name set**. This completes
F1's plain intent ("validate the token resolves to something real") without changing it. No
ratification required; flagged to mika-arch as FYI.

---

## Key Technical Decisions

**KTD-1 — Source of truth: walk the real source tree, not the compile-time snapshot.**
The verify binary walks `skills/bundled/` **on disk** (the actual source files a PR changes), reusing
the canonical discovery helper `crates/mika-agent/build_support/bundled_skills_discover.rs` via a
`#[path = ...]` mod attribute (the same pattern `build.rs` and the integration tests use). Rationale:
(a) a pre-merge gate should assert against the files on disk, not a `BUNDLED_SKILL_MANIFESTS` snapshot
that only updates on rebuild; (b) Check 3's Exec executable-bit check needs `std::fs` metadata on the
real handler file; (c) avoids widening the crate's `pub` surface (`all_bundled_skills()`,
`SkillFile`, `BundledSkill` are private). The binary reads `skill.toml` (via the `toml` crate),
`tools.json` (via `serde_json`), and file metadata directly.

**KTD-2 — Builtin name resolution set.** Check 3 (Builtin handler resolution) and Check 4 option (a)
resolve against `builtin_handlers::KNOWN_BUILTINS` (already `pub`). For the *core ToolRegistry*
builtin set (OD-2), expose a `pub` accessor returning the default tool names (or reuse an existing
one if present) — scoped to this need, not a general API. Resolve the exact accessor during
implementation with compiler feedback.

**KTD-3 — Identity allowlist access for Check 5.** Check 5 reads each well-known agent's
`[skills].allowlist` from `well_known_agents.rs` (`MIKA_DEV_IDENTITY`, `MIKA_QA_IDENTITY`,
`MIKA_RELAY_IDENTITY`, and the computed `build_mika_arch_identity()`). These consts are private; add a
`pub` accessor that returns the per-agent allowlist skill-name set (parsed from the identity TOML, or
a structured getter). The binary then asserts: every allowlisted name exists as a bundled skill, and
every `required_tools` token of every allowlisted skill is transitively resolvable through the
allowlist's skills + builtins.

**KTD-4 — `KNOWN_EXCEPTIONS` self-cleaning pattern.** Mirror `bundled_skills.rs:1563`
`KNOWN_PRE_EXISTING_COLLISIONS` exactly: a `const KNOWN_EXCEPTIONS: &[(…, &str /*ticket*/)]` plus a
test that fails when an entry no longer matches an actual failure ("stale exception, remove it").
Ships **empty** (per F2). If a future skill-add needs an exception too large to fix in-PR, the entry
cites its resolution ticket and the PR body enumerates it.

**KTD-5 — Output + exit semantics.** The binary prints one human-readable line per failure
(`CHECK n FAIL: <skill> — <reason>`), a summary, and exits `0` (all pass) / `1` (any unallowlisted
failure). Designed to read well in CI logs, mirroring the existing `byte-slice-lint` /
`loop-select-lint` script gates.

---

## High-Level Technical Design

```
verify_bundled_skills (bin)
   │
   ├─ discover()  ──────────────►  build_support/bundled_skills_discover.rs  (#[path], real dir walk)
   │      └─ Vec<DiscoveredSkill { name, dir, has_skill_toml, has_system_prompt, has_tools_json,
   │                               skill_toml: toml::Value, tools: Vec<ToolDef> }>
   │
   ├─ builtin_name_set()  ──────►  builtin_handlers::KNOWN_BUILTINS  ∪  core ToolRegistry names
   ├─ identity_allowlists() ────►  well_known_agents:: pub accessor (DEV/QA/RELAY/ARCH)
   │
   ├─ check1_completeness()   (skill.toml + system_prompt.md ; tools.json per ratified OD-1)
   ├─ check2_manifest()       (valid TOML; [skill] name+version; [triggers] keywords unless always_on)
   ├─ check3_handler_res()    (Builtin{function} ∈ builtin set ; Exec{command} file exists + +x)
   ├─ check4_token_consistency() (each required_tools token ∈ builtin set ∪ own tools ∪ any skill's tools)
   ├─ check5_identity_coherence() (allowlist names exist ; tokens transitively resolvable in allowlist)
   │
   └─ KNOWN_EXCEPTIONS (empty) + self-cleaning assertion → aggregate → exit 0|1
```

Grounded facts (verified against the worktree, HEAD `f15c2839`):
- `skill.toml` schema: `keywords` live under **`[triggers]`** (e.g. `gh-read-only`), `required_tools`
  under **`[constraints]`**. `always_on` under `[skill]`.
- `tools.json`: array of `{name, description, input_schema, handler}`; `handler.type ∈
  {"builtin","exec"}`; builtin → `handler.function`; exec → `handler.command` (e.g.
  `"handlers/run.sh"`).
- Skills with `[constraints] required_tools`: `gh-read-only`, `mika-arch-groom-milestone`,
  `qa-review`, `mika-arch-groom-ticket`, `skill-review`, `mika-arch-second-review`, `dev-groom`,
  `dev-handsoff`, `self-dev`. Of these, `dev-handsoff` and `self-dev` have **no** `tools.json` (OD-1).

---

## Implementation Units

### U1. Check 1 — bundle completeness  **(BLOCKED on OD-1 / mika-arch third-pass)**

- **Goal:** Assert every skill dir under `skills/bundled/` (excluding `_shared/` and dotfiles) has the
  required files, per the ratified Check-1 predicate.
- **Dependencies:** OD-1 ruling; U2's discovery scaffold.
- **Files:** `crates/mika-agent/src/bin/verify_bundled_skills.rs` (check1 fn + tests).
- **Approach:** Implement per ratified predicate. Recommended shape: `skill.toml` + `system_prompt.md`
  always required; `tools.json` coupling DROPPED. If mika-arch ratifies (B) instead, implement the
  coupling + the named AC2-safe resolution for self-dev/dev-handsoff.
- **Test scenarios:** PASS — prompt-only skill (no tools.json, no required_tools); skill with all
  three files. FAIL — dir missing `skill.toml`; dir missing `system_prompt.md`. (If coupling kept:
  required_tools + no tools.json → FAIL.) Plus: the real `skills/bundled/` tree passes green.
- **Execution note:** Do not author until OD-1 is ratified — building the recommended drop
  pre-ratification constructs an overturn of a ratified architect resolution (Prime ruling).

### U2. Binary scaffold + discovery + checks 2 & 3

- **Goal:** The binary entrypoint, source-tree discovery (KTD-1), and Checks 2 (manifest parses +
  minimum fields) and 3 (handler resolution).
- **Dependencies:** none (KTD-1, KTD-2).
- **Files:** `crates/mika-agent/src/bin/verify_bundled_skills.rs`; `Cargo.toml` (`[[bin]]` entry if
  the bin isn't auto-discovered — `src/bin/` is auto-discovered, so likely none).
- **Approach:** Walk `skills/bundled/` via the shared discovery helper; for each skill parse
  `skill.toml` (toml), `tools.json` (serde_json). Check 2: valid TOML, `[skill].name`,
  `[skill].version`, `[triggers].keywords` non-empty unless `[skill].always_on = true`. Check 3: for
  each tool, `builtin` → `handler.function ∈ KNOWN_BUILTINS`; `exec` → handler file exists at
  `<skill_dir>/<handler.command>` and is executable (`std::fs` mode `& 0o111 != 0`).
- **Patterns to follow:** `bundled_skills.rs` AC2 test (iteration over skills, tools.json parse,
  aggregate-then-assert); existing `scripts/check-*.sh` CLI-gate ergonomics.
- **Test scenarios:** Check 2 PASS — valid manifest; always_on skill without keywords. FAIL —
  malformed TOML; missing name; missing version; non-always_on missing keywords. Check 3 PASS —
  `gh_read` builtin resolves; `dev-pilot` exec `handlers/run.sh` exists+executable. FAIL — builtin
  `function` not in set; exec command file missing; exec command file not executable.

### U3. Check 4 (token consistency) + Check 5 (identity coherence) + builtin/allowlist accessors

- **Goal:** Checks 4 and 5, plus the `pub` accessors they need (KTD-2 core tool names, KTD-3 identity
  allowlists).
- **Dependencies:** U2.
- **Files:** `crates/mika-agent/src/bin/verify_bundled_skills.rs`;
  `crates/mika-agent/src/skills/builtin_handlers.rs` or a tool-registry module (pub core-tool-name
  accessor, OD-2/KTD-2); `crates/mika-agent/src/well_known_agents.rs` (pub allowlist accessor, KTD-3).
- **Approach:** Build `builtin_name_set = KNOWN_BUILTINS ∪ core_tool_names`. Check 4: for each skill
  with `required_tools`, each token ∈ `builtin_name_set` OR declared in own `tools.json` OR in ANY
  skill's `tools.json` (allowlist-unaware, per F1). Check 5: for each well-known agent allowlist,
  every name is a bundled skill, and every `required_tools` token of allowlisted skills is resolvable
  via `builtin_name_set` ∪ tools declared by skills *in that allowlist* (allowlist-scoped — the
  reachability F1 defers here).
- **Test scenarios:** Check 4 PASS — `gh_read` (builtin); `run_claude_pilot` (cross-skill, in
  dev-pilot); `write_agent_file` (core builtin). FAIL — `required_tools = ["nonexistent_tool"]`. Check
  5 PASS — real DEV/QA/RELAY/ARCH allowlists cohere. FAIL — allowlist names a non-bundled skill;
  allowlist skill's token unreachable within the allowlist's skill set + builtins.

### U4. Makefile target (AC1)

- **Goal:** `make verify-bundled-skills` builds and runs the binary.
- **Dependencies:** U2 (binary must exist to run).
- **Files:** `Makefile`.
- **Approach:** Add a `.PHONY` target mirroring existing structural-gate targets (`test-dispatch-symmetry`):
  `cargo run -q --bin verify-bundled-skills` (or `cargo run -p mika-agent --bin verify_bundled_skills`).
  Confirm the bin name the cargo manifest exposes.
- **Test scenarios:** Test expectation: none — Makefile wiring; behavior covered by U6 + CI (U5).

### U5. CI gate (AC3)

- **Goal:** A new, additive job/step in `.github/workflows/ci.yml` running `make verify-bundled-skills`
  on every PR; failure blocks merge.
- **Dependencies:** U4.
- **Files:** `.github/workflows/ci.yml`.
- **Approach:** Mirror the existing `byte-slice-lint` / `loop-select-lint` job shape (checkout +
  toolchain + run). Additive — does not modify existing jobs.
- **Test scenarios:** Test expectation: none — CI config; validated by the job running green on this PR.

### U6. Tests / fixtures for checks (AC4)

- **Goal:** Inline `#[cfg(test)]` tests covering each PASS/FAIL case for checks 2–5 now, and check 1
  once U1 lands; plus the `KNOWN_EXCEPTIONS` self-cleaning assertion (KTD-4); plus a "real tree passes
  green" test.
- **Dependencies:** U2, U3 (U1 portion gated on OD-1).
- **Files:** `crates/mika-agent/src/bin/verify_bundled_skills.rs` (`#[cfg(test)] mod tests`); synthetic
  in-memory skill fixtures (build `DiscoveredSkill` values in-test rather than temp dirs where
  possible; use a `tempfile` dir only for the executable-bit Exec case).
- **Approach:** Pure-function checks take a `&[DiscoveredSkill]` + the builtin/allowlist sets so tests
  pass synthetic fixtures without touching the real FS (except the one exec-bit case). Add the
  self-cleaning `KNOWN_EXCEPTIONS` test mirroring `bundled_skills.rs:1579`.
- **Test scenarios:** Enumerated per-check under U1/U2/U3; plus `KNOWN_EXCEPTIONS` empty-by-default
  asserted; plus stale-exception self-clean assertion fires when an entry doesn't match a real failure.

### U7. Documentation (AC5)

- **Goal:** `docs/architecture/` doc describing the verify target as the structural counterpart to AC2.
- **Dependencies:** none (can write in parallel; finalize check-1 prose after OD-1).
- **Files:** `docs/architecture/bundled-skill-verification.md` (new). Update root `CLAUDE.md` §"Adding
  a New Bundled Skill" to reference the gate. Note doc-sync: if the doc must be readable from the crate
  (it's architecture, not `docs/solutions/`), confirm whether `scripts/sync-agent-docs.sh` / the
  `docs-sync` CI job scopes it — `docs/architecture/` is not currently synced into the crate, so a new
  file there should not trip docs-sync; verify during implementation.
- **Approach:** Describe the three-layer silent-failure defense (AC2 collision test @ build-test;
  this gate @ pre-merge; #516 availability filter @ runtime), the five checks, the `KNOWN_EXCEPTIONS`
  mechanism, and the OD-1 reconciliation outcome.
- **Test scenarios:** Test expectation: none — documentation.

---

## Scope Boundaries

**In scope:** checks 1–5 on `skills/bundled/`; Makefile target; CI gate; tests; `docs/architecture/`
doc; the OD-2 check-4 broadening; pub accessors needed by checks 3/4/5.

**Out of scope:** runtime invariants (`apply_load_safety_check`); marketplace skill verification
(bundled only); `tools.json` schema validation against the canonical tool schema; optional checks
6–8 (prompt-size limits, keyword-overlap, schema validation).

### Deferred to Follow-Up Work
- Optional checks 6–8 (file a follow-up if/when needed).
- If mika-arch rules (B) on OD-1, the self-dev/dev-handsoff skill changes it mandates may warrant a
  companion ticket.

---

## Risks & Dependencies

- **OD-1 ratification is on the critical path for U1 only.** All other units proceed independently.
- **Pub-surface additions (KTD-2/KTD-3):** keep minimal and single-purpose; a reviewer may prefer an
  existing accessor — resolve with compiler feedback, don't speculatively widen the API.
- **CI cost:** one extra `cargo run` of a tiny binary — negligible next to existing build/test jobs.
- **Composition with mika#1326 AC2 / mika#516:** this gate must not duplicate or contradict AC2's
  collision check; it targets the orthogonal completeness class.
