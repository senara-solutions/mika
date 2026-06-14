---
title: "engine: quoted-resource pre-fetch guard for skill-manifest-driven brief-content fetching"
type: engine
status: active
date: 2026-04-29
ticket: senara-solutions/mika#863
branch: engine/863/quoted-resource-pre-fetch-guard-fetch
origin: docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md (Rule 1 forward-pointer)
related: senara-solutions/mika#862 (Rule 2 asserted-unavailability — sibling, post-condition surface), senara-solutions/mika#864 (verdict-line ghosting — sibling), senara-solutions/mika#870/#871 (callback-flow guards — adjacent registry pattern)
---

# engine: quoted-resource pre-fetch guard for skill-manifest-driven brief-content fetching

## Overview

mika#863 is the structural counterpart to Rule 1 of the gate-evasion compound doc (`docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`). The compound doc names the failure pattern: when an operator-supplied brief quotes an issue body / PR diff / file content, the agent treats the quote as constituting the content rather than as a claim *about* the content. Recurrence-2 (mika#788) was a sufficiency hallucination of exactly this shape — the architect issued a first-pass verdict against a brief-quoted issue body without calling `gh_read`, and the existing required-tools-gate caught it one turn later (after the verdict was already shaped, costing a retry).

This plan adds a **pre-condition** guard that fires at turn-start (before the LLM generates text), augmenting the turn's required-tools set with deterministically-derived fetches based on brief content. The existing required-tools-gate then enforces them — but earlier in the turn, eliminating the verdict-then-retry waste.

**Architectural distinction from sibling #862:** mika#862 (asserted-unavailability) is a *post-condition* in `INTENT_GUARDS` at `agent.rs:3989` — fires at EndTurn against assistant text. mika#863 is a *pre-condition* in the skills pipeline at `crates/mika-agent/src/skills/` — fires before the LLM turn against the user message. They sit on opposite sides of the LLM call by design.

## Problem Frame

### Observed failure (from compound doc + mika#788)

mika#788's first-pass run: the operator brief quoted the relevant issue body inline (block-fenced after a `gh issue view 788` header). Architect issued a first-pass verdict against the quoted text. The required-tools-gate fired post-EndTurn because the matched skill (`mika-arch-groom-ticket`) declared `required_tools = ["gh_read"]` — but the trigger was generic ("must call gh_read") rather than tied to the brief-content shape. By the time the gate fired, the verdict was already generated. Retry cost: one extra LLM turn at architect rates.

### Root cause

The current `Constraints.required_tools: Vec<String>` (`crates/mika-agent/src/skills/manifest.rs:88`) is a static list per skill manifest. The collect-and-enforce pipeline (`skills/matcher.rs:7,205`, post-condition gate at `agent.rs:1100-1129`) checks at EndTurn whether the static list was satisfied. This works for "this skill always needs gh_read" but doesn't dynamically reflect what the brief actually contains. A brief with no quoted resources still triggers `required_tools` enforcement; a brief with quoted resources doesn't get pre-emptive injection of the specific resource fetches.

The pre-fetch guard fixes the dynamic-augmentation gap: the brief content drives which fetches are required, the static manifest declares opt-in.

### Why this is enhancement-priority not p1

mika#788's failure was caught by the existing post-condition gate within one retry. The cost is one extra LLM turn per occurrence — material at architect rates but not catastrophic. mika#862 (sibling, p1-equivalent in blast-radius) closes the asserted-unavailability path that the existing gate doesn't catch at all. mika#863 closes the verdict-then-retry waste that the gate handles correctly but inefficiently. Lower urgency, same compound-doc family.

## Requirements Trace

- **R1.** New field `required_fetches_for_quoted_resources: bool` added to `Constraints` struct in `crates/mika-agent/src/skills/manifest.rs:88+` (current struct has only `required_tools: Vec<String>`). Defaults to `false` via `#[serde(default)]`. Skills opt in by setting `[constraints] required_fetches_for_quoted_resources = true` in `skill.toml`.
- **R2.** New brief-marker detection function `detect_quoted_resources(message: &str) -> Vec<QuotedResource>` in a new module `crates/mika-agent/src/skills/quoted_resources.rs`. Returns one `QuotedResource` per detected fetchable shape. Five concrete patterns:
  - **Issue body block:** triple-backtick fence containing `issue/<n>` header (or surrounding context indicating issue#N body — case-sensitive `issue/<digits>` regex inside the fence).
  - **PR diff/view block:** triple-backtick fence containing `PR/<n>` or `pr/<n>` header.
  - **`gh issue view <n>` quoted output:** literal `gh issue view <n>` line followed by output (heuristic: any block-fenced content within ~5 lines of such a header is treated as the fetch target).
  - **`gh pr view <n>` / `gh pr diff <n>` quoted output:** same shape with PR-side commands.
  - **File-content quote with `<repo>/<path>` header:** triple-backtick fence preceded by a `<repo-name>/<file-path>` line (heuristic match against the patterns mika-arch's briefs use today).
  Each `QuotedResource` carries `{kind: ResourceKind, identifier: String, repo: Option<String>}` where `kind` is one of `Issue { number }`, `PullRequest { number }`, `PullRequestDiff { number }`, `File { path, ref: Option<String> }`.
- **R3.** Resource → fetch-tool mapping table in `quoted_resources.rs`:
  | `ResourceKind` | Tool to inject | Tool args (filled from `QuotedResource`) |
  |----------------|----------------|-----------------------------------------|
  | `Issue` | `gh_read` | `{"op": "issue_view", "target": "<n>", "repo": "<repo>"}` |
  | `PullRequest` | `gh_read` | `{"op": "pr_view", "target": "<n>", "repo": "<repo>"}` |
  | `PullRequestDiff` | `gh_read` | `{"op": "pr_diff", "target": "<n>", "repo": "<repo>"}` |
  | `File` | `gh_read` | `{"op": "file_view", "target": "<path>", "repo": "<repo>", "ref": "<ref-or-main>"}` |

  **F2 resolution — uniform `gh_read` mapping is correct, with citation.** The issue body's Acceptance Criteria #3 mentions `read_file or gh_read file_view` for the File kind. **Tool-registry verification:** `read_file` does NOT exist in `crates/mika-agent/src/tools/`. The only file-related read tools are `read_agent_file` (reads agent home directory files, NOT repo files — see `tools/read_agent_file.rs:104`) and `gh_read` with `op="file_view"` (the canonical repo-file fetch path per `crates/mika-agent/CLAUDE.md` § GitHub Read-Only Handler: "five allowed ops: issue_view, pr_view, pr_diff, issue_list, file_view"). The issue body's `read_file` reference is aspirational; the actual tool is `gh_read file_view`. Tool-name-based gate enforcement: all four resource kinds correctly map to `"gh_read"` because that IS the only fetch tool for these resources. Per-kind argument enforcement (e.g., `op=file_view` specifically) is out of scope for v1 (see Out of Scope § F5 sentinel for the argument-shape evolution path).

  These four tool calls are augmented into the loop's `required_tools` set. The existing required-tools-gate at `agent.rs:1100-1129` then enforces them — without code change at the gate site (the augmentation happens upstream in the skill-matching pipeline).
- **R4.** Augmentation site in `crates/mika-agent/src/skills/matcher.rs` (or wherever `collect_required_tools` aggregates the per-skill manifests — to be confirmed at implementation against the actual function signature). For each matched skill where `constraints.required_fetches_for_quoted_resources == true`, run `detect_quoted_resources(initial_user_message)`, map each result via R3's table, and merge the tool names into the collected `required_tools` set.

  **F1 resolution — skill-invocation-scoped lifetime, against the INITIAL user message.** The augmentation MUST run ONCE per agent-loop entry (`run_agent` / `run_silent_agent`), against the user-role message that triggered the skill match. The augmented set lives alongside the static `Constraints.required_tools` for the entire loop's lifetime — corrective system messages from intent-guard re-fires (mika#870 pattern, `webhook_zero_tools` pattern) MUST NOT cause re-detection against the corrective text. If `detect_quoted_resources` ran at every step iteration against the *current* tail message, a corrective re-prompt would replace the initial brief in the inspected text → augmented set shrinks to the static one → gate passes incorrectly even though the agent never fetched the originally-quoted resources. The "compute once at loop entry, hold for loop lifetime" semantics match the existing static `required_tools` lifetime — same scoping, just with dynamic content at the time of computation. Threading: the augmented set is added to the same data structure that holds the static `required_tools` (whatever the existing post-condition chain consumes), so downstream gate code is unchanged.
- **R5.** mika-arch's two skills opt in. Files: `mika/skills/bundled/mika-arch-groom-ticket/skill.toml` and `mika/skills/bundled/mika-arch-second-review/skill.toml`. Add `[constraints] required_fetches_for_quoted_resources = true` to both. Both already have `required_tools = ["gh_read"]` (per mika#788's evidence); the new field augments rather than replaces.
- **R6.** Eval scenario in `crates/mika-agent/tests/eval/grounding_regressions/quoted_resource_pre_fetch.rs`. Two cases:
  - **Caught — brief contains quoted issue body:** `MockLlmProvider` first turn emits a verdict-shaped response with no `gh_read` call. Brief contains a triple-backtick fence with `issue/788` header. Assert: required-tools-gate fires once (because `gh_read` was injected by the pre-fetch guard); turn 2 emits `gh_read({"op":"issue_view","target":"788","repo":"x/y"})` then verdict; loop exits cleanly.
  - **Genuine no-fetch — brief has no quoted resources:** `MockLlmProvider` first turn emits a verdict with no `gh_read` call. Brief is plain prose with no fenced content. Assert: pre-fetch guard injects nothing additional; required-tools-gate's static `["gh_read"]` constraint still fires (per skill manifest); behavior matches pre-#863 baseline.
  - Frozen pre-fix fixture `fixtures/quoted_resource_pre_fetch_pre_fix.json` reproduces the mika#788 verdict-then-retry trace shape.
- **R7.** Compound doc update: append a new section to `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` Rule 1 noting that mika#863's pre-fetch guard is the structural counterpart now in place. Reference mika#788's trace ID as the recurrence evidence the guard closes on.
- **R8.** No new DB columns or schema migrations. The required state (matched skills, user message, manifest constraints) is already available at the skill-collection site.

## Proposed Fix

### Primary: skill-manifest opt-in + brief detection + tool injection

**Change 1 — manifest field** (`crates/mika-agent/src/skills/manifest.rs:88+`):

```rust
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Constraints {
    /// Tool names that must be called at least once before the response is accepted.
    /// Names can reference skill-defined tools, builtin tools, or MCP tools.
    #[serde(default)]
    pub required_tools: Vec<String>,

    /// When true, the engine inspects the user message at turn-start for quoted
    /// fetchable resources (issue/PR/file blocks) and augments the required_tools
    /// set with the corresponding fetch tool calls. Closes mika#863 (Rule 1
    /// brief-as-claims-not-facts pattern). Opt-in per skill — defaults to false.
    #[serde(default)]
    pub required_fetches_for_quoted_resources: bool,
}
```

The new field's `is_empty()` impact: extend `Constraints::is_empty()` to also return `false` when the new flag is true (so skills with only this opt-in still trigger the constraints pipeline).

**Change 2 — detection module** (new file `crates/mika-agent/src/skills/quoted_resources.rs`):

```rust
// Pseudocode aligned with existing skills/ module style
pub enum ResourceKind {
    Issue { number: u32 },
    PullRequest { number: u32 },
    PullRequestDiff { number: u32 },
    File { path: String, ref_: Option<String> },
}

pub struct QuotedResource {
    pub kind: ResourceKind,
    pub repo: Option<String>,  // owner/name when extractable from context
}

pub fn detect_quoted_resources(message: &str) -> Vec<QuotedResource> {
    // Five-pattern detection. Each pattern:
    //   1. Find triple-backtick-fenced blocks
    //   2. Inspect the line preceding the fence and the first ~3 lines inside
    //   3. If a fetchable-resource header pattern matches, emit a QuotedResource
    // Return all detected resources in document order.
    // ...
}

pub fn resource_to_required_tool(resource: &QuotedResource) -> &'static str {
    match resource.kind {
        ResourceKind::Issue { .. }
        | ResourceKind::PullRequest { .. }
        | ResourceKind::PullRequestDiff { .. }
        | ResourceKind::File { .. } => "gh_read",
    }
}
```

The R3 mapping currently funnels all four kinds into the same tool name (`gh_read`) — the *operation* differs, but the required-tools-gate enforces tool-name calls, not operation-specific calls. The four kinds drive different tool argument JSON, but the gate enforcement is just "did `gh_read` land in this turn's tool calls?" That's an acceptable simplification for the v1 of this guard — if the gate eventually needs to enforce "did `gh_read` land *with the right `op` field*", that's a separate evolution per the existing gate's semantics.

**Change 3 — augmentation site** (`crates/mika-agent/src/skills/matcher.rs`, exact function name pending implementation against the actual `collect_required_tools` callsite):

```rust
// Pseudocode — the actual function signature/site needs implementation-time confirmation
pub fn collect_required_tools(
    matched: &[MatchedSkill],
    user_message: &str,
) -> HashSet<String> {
    let mut required: HashSet<String> = matched.iter()
        .filter(|m| matches!(m.reason, MatchReason::Keyword))
        .flat_map(|m| m.skill.constraints.required_tools.iter().cloned())
        .collect();

    // mika#863 pre-fetch augmentation: opt-in skills extend required_tools
    // with brief-derived fetches.
    let needs_pre_fetch = matched.iter()
        .any(|m| m.skill.constraints.required_fetches_for_quoted_resources
                 && matches!(m.reason, MatchReason::Keyword));
    if needs_pre_fetch {
        let resources = quoted_resources::detect_quoted_resources(user_message);
        for resource in &resources {
            required.insert(quoted_resources::resource_to_required_tool(resource).to_string());
        }
    }

    required
}
```

Augmentation only fires for `Keyword`-matched skills (not `AlwaysOn` or `Dependency`) — same scoping rule the existing static `required_tools` follows per the matcher.rs:205 comment. Prevents always-on skills from injecting required fetches into every turn.

**Change 4 — mika-arch skill manifests** (`mika/skills/bundled/mika-arch-groom-ticket/skill.toml` and `mika/skills/bundled/mika-arch-second-review/skill.toml`):

```toml
[constraints]
required_tools = ["gh_read"]                          # existing
required_fetches_for_quoted_resources = true         # new — mika#863
```

### Tests

**File:** `crates/mika-agent/tests/eval/grounding_regressions/quoted_resource_pre_fetch.rs` (new), modelled on existing grounding_regressions/ scaffold.

Scenario 1 — **`quoted_resource_pre_fetch_caught`:**
- `EvalHarness` configures a skill with `required_fetches_for_quoted_resources = true` and `required_tools = ["gh_read"]`.
- User message contains a triple-backtick fence preceded by `gh issue view 788`.
- `MockLlmProvider` turn 1: emits verdict text with no tool calls.
- Assert: required-tools-gate fires once (because `gh_read` was in the augmented required set, not just static); corrective re-prompt issued.
- Turn 2: `MockLlmProvider` returns `[gh_read({"op":"issue_view","target":"788","repo":"x/y"}), text("Verdict: ...")]`. Assert: loop exits cleanly.

Scenario 2 — **`quoted_resource_pre_fetch_no_op`:**
- Same skill config.
- User message is plain prose, no fenced blocks.
- `MockLlmProvider` turn 1: emits text with no tool calls.
- Assert: required-tools-gate fires (because static `["gh_read"]` is still in effect); pre-fetch augmentation added nothing additional. Pre-#863 baseline behavior unchanged.

**Scenario 3 — `quoted_resource_pre_fetch_mixed`** (F6 — over-augmentation regression sentinel):
- Same skill config.
- User message contains BOTH a quoted issue-body fence AND prose containing `#NNN` references in non-fenced text.
- `MockLlmProvider` turn 1: emits `gh_read({"op":"issue_view","target":"<n>","repo":"x/y"})` for the fenced resource ONLY (matching the augmented requirement) + verdict text. NO additional `gh_read` call for the prose `#NNN` references.
- Assert: required-tools-gate satisfied with exactly one `gh_read` call (no over-augmentation); loop exits cleanly. Verifies that detection scopes to fenced content, not free-text issue-number mentions. Catches future regression where a refactor incorrectly augments on plain prose.

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/manifest.rs` | Add `required_fetches_for_quoted_resources: bool` field to `Constraints` struct (~line 88+); extend `Constraints::is_empty()` to consider the new field; add unit test mirroring existing `test_parse_constraints_required_tools` (~line 602). |
| `crates/mika-agent/src/skills/quoted_resources.rs` | New module — `ResourceKind` enum, `QuotedResource` struct, `detect_quoted_resources(&str) -> Vec<QuotedResource>`, `resource_to_required_tool(&QuotedResource) -> &'static str`. Five-pattern regex detection. |
| `crates/mika-agent/src/skills/mod.rs` | Add `pub mod quoted_resources;`. |
| `crates/mika-agent/src/skills/matcher.rs` | Augment `collect_required_tools` (or equivalent) to invoke `detect_quoted_resources` on opt-in matched skills and merge results into the required set. |
| `mika/skills/bundled/mika-arch-groom-ticket/skill.toml` | Add `required_fetches_for_quoted_resources = true` under `[constraints]`. |
| `mika/skills/bundled/mika-arch-second-review/skill.toml` | Same opt-in. |
| `crates/mika-agent/tests/eval/grounding_regressions/quoted_resource_pre_fetch.rs` | New test file — two scenarios above. |
| `crates/mika-agent/tests/eval/grounding_regressions/fixtures/quoted_resource_pre_fetch_pre_fix.json` | New fixture — frozen pre-fix verdict-then-retry trace from mika#788. |
| `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` | Register new scenarios. |
| `crates/mika-agent/tests/eval/grounding_regressions/README.md` | Add new scenarios to capability matrix; add tag `pre-fetch-required-when-quoted` (success) / `pre-fetch-skipped-when-quoted` (failure) to `grounding:*` namespace. |
| `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` | Append note to Rule 1 referencing mika#863 as the structural counterpart now in place; cite mika#788 trace ID as recurrence evidence. |
| `CHANGELOG.md` | Add entry under "Added" — "Engine now pre-fetches quoted resources cited in opt-in skill briefs before EndTurn enforcement, eliminating the verdict-then-retry round-trip when briefs quote issue/PR/file content. Closes #863." |

No schema changes. No new dependencies. No new env vars.

## Verification

### Phase 0 — pre-implementation tool-registry verification (F8 gate)

Before writing any implementation code, run:

```bash
grep -rn "read_file\b" crates/mika-agent/src/tools/
```

**Expected:** zero hits (confirming `read_file` is NOT a registered tool, so R3's uniform `gh_read` mapping for the File kind is correct).

**Fallback if hits exist:** R3's mapping is wrong. Update R3 to include `read_file` in the File-kind row, and update mika#863's issue body Acceptance Criteria #3 with an edit-notice noting the registry-correction.

This Phase 0 verification was already performed during grooming (2026-04-29 with the worktree at SHA `46361cb8`) — zero hits confirmed. Re-run at implementation time as a pre-commit gate so the assumption stays grounded if the registry evolves between groom and implementation. Same pre-commit-discovery discipline applied across mika#821 F6, mika-platform#52 F2, mika#788 Step 4.

### Unit / integration

```bash
cd /data/workspace/mika-platform/.claude/worktrees/engine-863-quoted-resource-pre-fetch-guard-fetch/mika
cargo test -p mika-agent --test eval grounding_regressions::quoted_resource_pre_fetch
cargo test -p mika-agent skills::manifest::test_parse_constraints
cargo test -p mika-agent skills::quoted_resources
cargo test -p mika-agent  # full suite
cargo clippy -- -D warnings
cargo fmt --check
```

### Manual reproduction (post-merge)

mika#788's pre-fix trace is the fingerprint. After deploy:

1. Restart mika-spirit.
2. Send mika-arch a brief that quotes an issue body inline (any of the five detected shapes).
3. Inspect the resulting session in `~/.mika/data/mika.db`:
   ```sql
   SELECT trace_id FROM messages
     WHERE agent_id = 'mika-arch' AND role = 'user'
       AND content LIKE '%```%issue/%'
     ORDER BY created_at DESC LIMIT 1;
   SELECT tool_name FROM tool_calls
     WHERE trace_id = '<trace_id>' AND tool_name = 'gh_read';
   ```
4. Assert: `gh_read` is in the first turn's tool calls (not the second). Pre-fix shape — verdict generated turn 1, `gh_read` called turn 2 after gate fires — must not appear.

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Brief-marker regex false-positives on non-fetchable fenced content (e.g., a code example demonstrating an issue URL). | Detection is conservative: requires both a triple-backtick fence AND a recognizable resource-header pattern in the immediate context. Unit tests cover three edge cases per scenario set. If false-positives surface, the regex set is tightened in a follow-up; the existing required-tools-gate's terminal-failure-bypass (#516) handles the case where the agent attempts the fetch and hits a real error. |
| Brief-marker regex misses a legitimate quoted-resource shape. | Lower-blast-radius failure — the existing required-tools-gate still enforces the static `["gh_read"]` constraint; the agent retries on EndTurn instead of pre-emptively. Same as today's behavior. Add the missed pattern as a follow-up. |
| Augmentation triggers for `AlwaysOn` matched skills, injecting fetches into unrelated turns. | The augmentation site filters on `MatchReason::Keyword` (matching the existing static `required_tools` enforcement scoping per `matcher.rs:205`). |
| Mapping table requires updates as new resource kinds are added (e.g., commit SHAs, PR comments). | Out of scope for v1. The mapping is in a single function (`resource_to_required_tool`); future kinds are additive. |
| `gh_read` enforcement satisfied by ANY `gh_read` call regardless of `op`. | v1 limitation — the existing required-tools-gate enforces tool-name presence, not argument-shape. If sufficient, ship; if not, follow-up ticket to extend the gate to argument-shape matching (significant new surface). For mika#863's actual failure (verdict-then-retry on quoted issue bodies), tool-name-level enforcement is enough to surface the verdict-shape penalty before EndTurn. |
| Existing required-tools-gate's terminal-failure-bypass (#516) interacts oddly with augmented requirements. | The augmented requirements are merged into the same set the existing gate consumes; bypass logic operates on the merged set. No new bypass logic needed. If the agent attempts the augmented `gh_read` and it fails terminally (auth, rate-limit), bypass fires and EndTurn proceeds — same pattern as today's static `["gh_read"]` failure mode. |

## Out of Scope

- **mika#862 (Rule 2 asserted-unavailability).** Sibling guard, separate plan, post-condition surface (different timing).
- **mika#864 (verdict-line ghosting).** Sibling guard, separate plan.
- **Auto-detection without skill-manifest opt-in.** The issue body explicitly excludes default-on. Skills that legitimately work from briefs alone (e.g., a brainstorming skill where the brief is the input, not a pointer) shouldn't be forced to fetch. Opt-in is the explicit contract.
- **Pattern-addition protocol (F4 codification).** Additional regex patterns added to `detect_quoted_resources` MUST be preceded by a compound-doc Rule 1 update with the observed brief shape and trace ID citation. Prevents silent pattern accumulation without institutional record. Symmetric with mika#862's pattern-addition protocol for the asserted-unavailability guard.
- **Argument-shape enforcement evolution (F5 sentinel).** v1 enforces tool-name-level (`gh_read` was called). Argument-shape enforcement (`gh_read` was called with `op="issue_view"` for an Issue-kind quote, not `op="pr_view"`) is deferred. **Migration trigger:** when a recurrence is observed where the gate-required tool fired with the wrong operation against a quoted resource and the verdict was misshaped. R3's per-kind argument mapping (already in this plan) is the foundation; extending the gate to enforce argument-shape is the missing piece.
- **Opt-in scoping discipline (F7 codification).** Other skills (mika-dev, mika-qa, etc.) do NOT opt in by default. Adding `required_fetches_for_quoted_resources = true` to a non-mika-arch skill is a workflow-design decision requiring its own reasoning, not a default-on extension. The opt-in MUST be justified per-skill (what brief shapes does this skill receive that need pre-fetching?).
- **Resource kinds beyond issue/PR/PR-diff/file.** Comments, commit SHAs, etc. are additive in `ResourceKind` and `resource_to_required_tool` — separate ticket per shape if observed pressure emerges.
- **Shared-helper extraction across the four guards (#862/#863/#864/#870).** Per mika#870/#862's plans: revisit when the second EndTurn-family guard ships. This guard is on a different surface (pre-condition, skills pipeline) so the shared-helper question is even less applicable.

## Open Questions for mika-arch

1. **Five-pattern regex completeness.** The five detection shapes are sourced from the issue body's "Detection" section. mika-arch may have observed additional brief patterns (e.g., embedded URLs without explicit headers). Defer-to-architect — my proposal is to ship the five and add patterns as new evasion shapes surface.
2. **`MatchReason` scoping.** Augmentation runs only for `Keyword`-matched skills. Should it also run for `AlwaysOn` skills that opt in? My read: NO, because always-on triggering fetches into every turn would explode latency and cost. Architect may have a different view if a use case emerges.
3. **`File` kind heuristic.** Detecting "this is a file content block" without a robust delimiter is heuristic-prone. My proposal is to require the `<repo-name>/<file-path>` header line preceding the fence. mika-arch may have a tighter heuristic from observed brief patterns.
4. **Scenario 3 (mixed brief).** Optional third test case verifying the augmentation only injects the quoted-resource fetch and no others. Probably worth shipping; small marginal cost. Defer-to-architect.

---

## Architect first-pass concerns (resolved in this revision)

This revision applies the seven findings from mika-arch's first-pass review (session `9baeceda-21eb-4249-b2e5-e2eac9ebebcc`).

### F1 — Skill-invocation-scoped lifetime against initial user message (BLOCKING, resolved)

R4 now states the augmentation MUST run ONCE per agent-loop entry against the user-role message that triggered the skill match, NOT per-turn against the current tail message. Corrective system messages from intent-guard re-fires (mika#870 pattern, `webhook_zero_tools` pattern) MUST NOT cause re-detection against the corrective text — that would shrink the augmented set to the static one and let the gate pass incorrectly. Lifetime matches the existing static `Constraints.required_tools` semantics (compute once, hold for loop lifetime). Threading: augmented entries stored alongside static entries in the same data structure the post-condition chain consumes.

### F2 — Per-kind tool mapping verified against tool registry (BLOCKING, resolved)

Issue body's Acceptance Criteria #3 mentions `read_file or gh_read file_view` for the File kind. Tool-registry verification: `read_file` does NOT exist in `crates/mika-agent/src/tools/`. The only file-related read tools are `read_agent_file` (agent home directory only, not repo files) and `gh_read` with `op="file_view"` (canonical repo-file fetch path per CLAUDE.md GitHub Read-Only Handler section). The issue body's `read_file` is aspirational; the actual tool is `gh_read file_view`. Tool-name-based gate enforcement: all four resource kinds correctly map to `"gh_read"` because that IS the only fetch tool. R3 now includes the verification citation. Per-kind argument enforcement (e.g., `op=file_view` specifically) is deferred to F5 sentinel.

### F3 — Lifetime invariant pinning (sharpening, applied)

R4 now states the lifetime invariant explicitly: "Augmentation invocation site is the same function that aggregates per-skill `Constraints.required_tools` into the gate's enforced set; the augmented entries must be stored adjacent to the static entries with identical lifetime semantics."

### F4 — Pattern-addition protocol codified (sharpening, applied)

Out of Scope: additional regex patterns added to `detect_quoted_resources` MUST be preceded by a compound-doc Rule 1 update with the observed brief shape and trace ID citation. Symmetric with mika#862's pattern-addition protocol.

### F5 — Argument-shape evolution sentinel (sharpening, applied)

Out of Scope: argument-shape enforcement (e.g., `gh_read` was called with `op=pr_view` instead of `op=issue_view` for an Issue-kind quote) is deferred. Migration trigger named: when a recurrence is observed where the gate-required tool fired with the wrong operation against a quoted resource and the verdict was misshaped. R3's per-kind argument mapping (already in this plan) is the foundation; extending the gate to enforce argument-shape is the missing piece.

### F6 — Scenario 3 over-augmentation regression sentinel (sharpening, applied)

Tests section now includes scenario 3: mixed brief with quoted issue body AND prose `#NNN` references. Verifies augmentation scopes to fenced content, not free-text issue-number mentions. Catches future regression where a refactor incorrectly augments on plain prose. Marginal cost, high regression-detection value per architect's recommendation.

### F7 — Opt-in scoping documentation (sharpening, applied)

Out of Scope explicitly states: other skills (mika-dev, mika-qa, etc.) do NOT opt in by default. Adding `required_fetches_for_quoted_resources = true` to a non-mika-arch skill is a workflow-design decision requiring its own reasoning, not a default-on extension.

---

## Architect verdict

- **First-pass (mika-arch session `9baeceda-21eb-4249-b2e5-e2eac9ebebcc`):** ITERATE. Two blockers (F1 skill-invocation-scoped lifetime, F2 per-kind tool mapping verification) + five sharpenings (F3-F7). All resolved in this revision.
- **Second-pass (same session, continuity preserved):** GROOMED. All seven findings resolved (F2 conditionally — pending implementation-time verification gate). One residual: F8 — F2-verification grep added as Phase 0 pre-commit gate (`grep -rn "read_file\b" crates/mika-agent/src/tools/`, expected zero hits, fallback documented). Verification was already performed during grooming with zero hits confirmed; the Phase 0 step keeps the assumption grounded if the registry evolves between groom and implementation.
