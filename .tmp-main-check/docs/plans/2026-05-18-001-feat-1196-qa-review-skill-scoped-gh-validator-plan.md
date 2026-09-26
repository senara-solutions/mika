---
type: feat
issue: 1196
repo: mika
branch: feat/1196/qa-review-re-narrow-run-gh-allowlist-via
status: groomed
---

# mika#1196 — Re-narrow qa-review `run_gh` allowlist via skill-aware argv validator

## Context

mika#1168 variant b2 (commit `ea105794`, shipped in PR #1197) removed qa-review's per-skill `run_gh` exec handler so the dispatch-ack handler (`run_gh issue edit <n> --remove-label ready`) stops hitting qa-review's narrow allowlist on the ready-label dispatch path. The structural fix is correct — handler shadowing was the bug — but it widens qa-review's reachable `run_gh` subcommand surface to the global `GH_ALLOWED_SUBCOMMANDS` (`crates/mika-agent/src/skills/builtin_handlers.rs:1619`).

The ce:review SEC-1 finding (P1, confidence 0.72) on PR #1197 surfaced the trade-off: when qa-review is the active intent and the LLM is processing an adversarial PR diff, a successful prompt injection can now reach `pr merge`, `api -X PATCH/POST/DELETE`, `issue close`, `pr edit`, etc. The only remaining barrier is the qa-review system prompt at `skills/bundled/qa-review/system_prompt.md:582` ("Do NOT merge PRs") — per `feedback_prompt_enforcement_fragile.md`, prompt-level "MUST" rules don't bind structurally.

This ticket re-introduces qa-review's narrow scope at the **handler dispatch layer** rather than the per-skill handler layer. Single dispatch authority is preserved; the shadowing footgun (mika#1168's actual cause) is not re-introduced.

## Phase 0 — Pin (verbatim slices at base SHA `1f10fb22`)

Base SHA: `1f10fb2249ea4c44e521c2e67fb7a220444c0e73` (origin/main HEAD at the moment of branch creation; also the branch's first commit).

### Pin 1 — `validate_gh_input` (`crates/mika-agent/src/skills/builtin_handlers.rs:1758–1792`)

```rust
/// Validate and parse `run_gh` input into structured args.
///
/// Checks: shared parse + allowlist + repo-smuggling.
fn validate_gh_input(input: &serde_json::Value) -> Result<GhArgs, ToolOutput> {
    let args = parse_command_array(input)?;

    // Validate subcommand against allowlist
    let subcommand = &args[0];
    if !GH_ALLOWED_SUBCOMMANDS.contains(&subcommand.as_str()) {
        return Err(ToolOutput::error(format!(
            "gh subcommand '{subcommand}' is not allowed. \
             Permitted: {}.",
            GH_ALLOWED_SUBCOMMANDS.join(", ")
        )));
    }

    // Reject --repo / -R smuggling in the command array (including --repo=value form)
    if args
        .iter()
        .any(|s| s == "--repo" || s == "-R" || s.starts_with("--repo="))
    {
        return Err(ToolOutput::error(
            "Do not include --repo in the command array. Use the separate 'repo' parameter instead."
                .to_string(),
        ));
    }

    let repo = input
        .get("repo")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    Ok(GhArgs { args, repo })
}
```

Pinned facts: signature is `fn validate_gh_input(input: &serde_json::Value) -> Result<GhArgs, ToolOutput>` (private, no `ctx`); returns `GhArgs { args: Vec<String>, repo: Option<String> }`; subcommand allowlist consulted first, then repo-smuggling guard, then `repo` extraction.

### Pin 2 — `run_gh` entry with `ctx` (`crates/mika-agent/src/skills/builtin_handlers.rs:1819–1834`)

```rust
async fn run_gh(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput {
    let gh_args = match validate_gh_input(input) {
        Ok(args) => args,
        Err(err) => return err,
    };

    // Compute PR dedup key for `pr review` commands.
    let pr_dedup_key = if is_pr_review_command(&gh_args.args) {
        debug_assert!(
            ctx.pr_reviews_posted.is_some(),
            "pr_reviews_posted must be threaded for production pr review calls"
        );
        Some(make_pr_dedup_key(&gh_args.args, gh_args.repo.as_deref()))
    } else {
        None
    };
```

Pinned facts: `ctx: &ToolContext<'_>` is in scope from line 1819 onward; the new scope-validator call site is between line 1823 (end of `validate_gh_input` match) and line 1825 (start of `pr_dedup_key` computation). `gh_args.args: Vec<String>` is available for the new helper to consume.

### Pin 3 — `SkillPathInfo` + `ToolContext.active_skill_paths` (`crates/mika-agent/src/tools/mod.rs:86–91, :129`)

```rust
pub struct SkillPathInfo {
    /// Skill name (e.g., "self-dev").
    pub skill_name: String,
    /// Path relative to agent home (e.g., "skills/self-dev/system_prompt.md").
    pub prompt_relative_path: String,
}
```

```rust
    /// Active skill prompts already injected into the system prompt.
    /// Used by read tools (e.g., `read_agent_file`) to detect redundant fetches.
    /// Empty in silent mode and tests by default.
    pub active_skill_paths: &'a [SkillPathInfo],
```

Pinned facts: the field is named `skill_name: String` (not `name`, `id`, or `path`) — the predicate `info.skill_name == "qa-review"` compiles against this verbatim. `active_skill_paths` is `&'a [SkillPathInfo]` (a slice; iterator-friendly without cloning). Per the doc comment, "Empty in silent mode and tests by default" — confirms the early-return-on-empty conservative bypass is intended behavior.

### Pin 4 — `active_skill_paths` population (`crates/mika-agent/src/agent.rs:2310–2330`)

```rust
    // Build active skill paths for context-redundancy checks in read tools.
    // Each matched skill's system_prompt.md is already injected into the system prompt
    // above — tools can use this list to detect and redirect redundant file reads.
    let active_skill_paths: Vec<SkillPathInfo> = matched_entries
        .iter()
        .filter(|e| !e.prompt_snippet.is_empty())
        .filter_map(|e| {
            match e.dir.strip_prefix(params.home_dir) {
                Ok(rel) => Some(SkillPathInfo {
                    skill_name: e.manifest.skill.name.clone(),
                    prompt_relative_path: rel
                        .join("system_prompt.md")
                        .to_string_lossy()
                        .into_owned(),
                }),
                Err(_) => {
                    warn!(
                        skill = %e.manifest.skill.name,
                        dir = %e.dir.display(),
                        "active_skill_paths: skill dir not under home_dir, excluded from redundancy check"
                    );
                    None
                }
            }
        })
        .collect();
```

Pinned facts: `e.manifest.skill.name` is the source of `SkillPathInfo.skill_name` — so the predicate-string `"qa-review"` must match `skills/bundled/qa-review/skill.toml`'s `[skill] name = "qa-review"` exactly (it does, verified at `skills/bundled/qa-review/skill.toml:2`). Population happens once per agent-loop turn before `ToolContext` construction at `agent.rs:2422`. Skills excluded via the `prompt_snippet.is_empty()` filter (line 2312) or the `strip_prefix(home_dir)` failure path (lines 2322-2328) won't appear in `active_skill_paths` — neither failure mode applies to qa-review on mika-qa (qa-review has a non-empty system_prompt.md and lives under `~/.mika/agents/mika-qa/skills/qa-review/`, a path under `home_dir`).

## Surface (file:line citations)

| Path | Role |
|------|------|
| `crates/mika-agent/src/skills/builtin_handlers.rs:1619` | `GH_ALLOWED_SUBCOMMANDS` global allowlist constant |
| `crates/mika-agent/src/skills/builtin_handlers.rs:1758–1792` | `validate_gh_input()` — current argv validator (subcommand allowlist + repo-smuggling guard) |
| `crates/mika-agent/src/skills/builtin_handlers.rs:1819–1834` | `run_gh()` handler entry — receives `&ToolContext<'_>`; calls `validate_gh_input` at line 1820 |
| `crates/mika-agent/src/tools/mod.rs:129` | `ToolContext.active_skill_paths: &'a [SkillPathInfo]` |
| `crates/mika-agent/src/agent.rs:2310–2320` | Where `active_skill_paths` is populated from `matched_entries` |
| `skills/bundled/qa-review/skill.toml:5` | `always_on = true` (qa-review present on every turn of mika-qa) |
| `skills/bundled/qa-review/system_prompt.md:586` | The current "Permitted: pr, issue, run, workflow, release, repo, search, label, api" line that must be re-narrowed to match the runtime |
| `skills/bundled/qa-review/system_prompt.md:132` | Existing "Do NOT use `run_gh("api ...")` to read the plan file" — confirms qa-review does not need `api` for its core flow |
| `crates/mika-agent/src/well_known_agents.rs:1546,1554,1587,1613,1649,1652,1688` | Explicit `db.set_skill_enabled("<agent>", "qa-review", false)` calls for mika-dev, mika-relay, custom-agent (validates the "qa-review only on mika-qa" assumption) |
| `crates/mika-agent/tests/qa_review_run_gh_shadowing_guard.rs` | Existing regression-guard test pattern — model for the new scope-validator test |
| `git show d011773f:skills/bundled/qa-review/handlers/run_gh.sh` | Historical narrow allowlist (recoverable from pre-deletion commit): `pr review`, `pr diff`, `pr list`, `issue view` |

## Design

### Detection predicate

```rust
let qa_review_active = ctx
    .active_skill_paths
    .iter()
    .any(|info| info.skill_name == "qa-review");
```

**Why this predicate is sufficient.** qa-review is `always_on = true` on the **qa-review agent only** — mika-dev, mika-relay, custom-agent and every other agent explicitly disable it (`well_known_agents.rs:1546…1688`). So `"qa-review" ∈ active_skill_paths` is true iff the current turn is on mika-qa. The dispatch-ack flow runs on mika-dev; its `active_skill_paths` will never contain qa-review; the global allowlist applies. AC2 is structurally satisfied by the agent-scoping invariant, not by intent classification.

**Why not `MatchReason::Keyword`?** `MatchReason` (Keyword | AlwaysOn | Dependency) exists in `crates/mika-agent/src/skills/matcher.rs:10`, but `SkillPathInfo` does not currently carry it. The ticket's "keyword-matched on `review`, `pr`, `qa`, `pull request`" framing is descriptive, not load-bearing — agent-scoping does the discrimination work here. Plumbing `MatchReason` through `SkillPathInfo` is a separate (and currently unmotivated) change.

### Narrow allowlist

```rust
/// qa-review's narrow gh subcommand+verb scope.
/// Mirrors the pre-mika#1168-b2 handler at d011773f:skills/bundled/qa-review/handlers/run_gh.sh.
const QA_REVIEW_GH_ALLOWED: &[(&str, &str)] = &[
    ("pr", "review"),
    ("pr", "diff"),
    ("pr", "list"),
    ("issue", "view"),
];
```

Positive list (anything not present rejects). Mirrors `d011773f` verbatim. `gh api` is **not** on the list; this is intentional and matches both the historical handler and the existing system-prompt guidance at `:132` ("Do NOT use `run_gh("api ...")` to read the plan file").

### Insertion point

A new helper, called from `run_gh` immediately after `validate_gh_input` returns `Ok` and **before** the PR-dedup logic at the current `:1825`:

```rust
fn validate_qa_review_gh_scope(args: &[String], ctx: &ToolContext<'_>) -> Result<(), ToolOutput> {
    let qa_review_active = ctx
        .active_skill_paths
        .iter()
        .any(|info| info.skill_name == "qa-review");
    if !qa_review_active {
        return Ok(());
    }
    let subcommand = args.first().map(String::as_str).unwrap_or("");
    let verb = args.get(1).map(String::as_str).unwrap_or("");
    if QA_REVIEW_GH_ALLOWED
        .iter()
        .any(|(s, v)| *s == subcommand && *v == verb)
    {
        return Ok(());
    }
    Err(ToolOutput::error(format!(
        "gh subcommand '{subcommand} {verb}' is not in qa-review's scope. \
         Permitted (qa-review): pr review, pr diff, pr list, issue view. \
         Source: skills/bundled/qa-review/skill.toml (always_on=true) + \
         builtin_handlers.rs::validate_qa_review_gh_scope (mika#1196)."
    )))
}
```

**Why a separate helper, not a `ctx` parameter added to `validate_gh_input`.** `validate_gh_input` has 8+ test callers (`builtin_handlers.rs:2716, 2725, 2741, 2779, 2787, 2858, 2866, 3061`) that pass `&serde_json::Value` only and exercise pure argv-parsing concerns. Threading `ctx` into all of them inflates the change without earning anything: skill-scoping is genuinely orthogonal to argv-parsing (the former depends on runtime context; the latter does not). The two-helper shape stays surgical and keeps the existing tests focused.

**Why before dedup, not after.** A scope violation should produce a scope error, not a `duplicate_pr_review` error masking a scope violation that happened to coincide with a retry. Conventionally: input-shape (validate_gh_input) → context-scope (validate_qa_review_gh_scope) → idempotency (dedup) → audit (gh api logging) → spawn.

### System-prompt reconciliation

Update `skills/bundled/qa-review/system_prompt.md`:
- **Line 586** "Permitted: pr, issue, run, workflow, release, repo, search, label, api" → "Permitted (qa-review scope): `pr review`, `pr diff`, `pr list`, `issue view`. Any other subcommand+verb rejects with a structured `validate_qa_review_gh_scope` error citing mika#1196."
- Drop the "Use `gh api` for milestone/project mutations" sentence on the same line (mutation guidance does not apply to qa-review; mutations are mika-dev territory).
- **No change to line 132** — its "Do NOT use `run_gh("api ...")`" guidance already aligns; the validator now enforces what the prompt requested.
- Bump `skill.toml` `version = "0.8.0"` → `"0.9.0"`.

Prompt-and-runtime parity is the structural goal: the prompt should reflect what the runtime permits. The previous shape promised wider access than was safe; this shape promises only what is enforced.

### AC3 test placement — eval-suite convention adopted

AC3 names `tests/eval/test_qa_review_run_gh_scope_validator.rs`. This is an established workspace convention: `crates/mika-agent/tests/eval/` is the integration-test directory using `EvalHarness` + `MockLlmProvider` to exercise the full `run_agent()` path deterministically, with submodules registered in `crates/mika-agent/tests/eval.rs`. Sibling tests at `tests/eval/test_callback_turn.rs`, `tests/eval/test_completion_claim_guard.rs`, `tests/eval/test_pr_review_idempotency.rs`, `tests/eval/test_auto_groom_dispatch.rs`, `tests/eval/test_webhook_zero_tools_guard.rs`. The plan **honors the convention**: the new test lives at `crates/mika-agent/tests/eval/test_qa_review_run_gh_scope_validator.rs` and is registered in `tests/eval.rs`.

**Two-layer test plan.** The eval-suite directory hosts agent-loop integration tests, not internal-validator unit tests. The existing convention for testing `run_gh`'s internal validators (e.g., `validate_gh_input`) is inline `#[cfg(test)] mod tests` in `builtin_handlers.rs` — 8 such tests exist at `builtin_handlers.rs:2675–2900` (sibling examples: `test_run_gh_allowlist_accepts_valid`, `test_run_gh_allowlist_rejects_removed_subcommands`, etc.). The plan keeps both layers:

- **Layer A (load-bearing validator tests)** — inline `#[cfg(test)] mod tests` for `validate_qa_review_gh_scope` in `builtin_handlers.rs`, covering all 10 direct-helper cases. Matches the existing `test_run_gh_*` convention; fast, focused, no harness setup.
- **Layer B (eval-suite wiring test)** — at the AC's path `crates/mika-agent/tests/eval/test_qa_review_run_gh_scope_validator.rs`, covering 2 scenarios via `EvalHarness` + `MockLlmProvider`: (1) qa-review-active rejects `pr merge`, (2) qa-review-not-active accepts `issue edit --remove-label ready`. Verifies the wiring of the validator into the agent-loop dispatch path (via the real `run_gh` builtin) end-to-end.

Layer A is the regression guard for the validator itself; Layer B is the regression guard for "the validator stays wired into `run_gh`'s dispatch and active_skill_paths threading isn't broken by a future refactor."

## Execution steps

1. **Add constant.** In `crates/mika-agent/src/skills/builtin_handlers.rs` near `GH_ALLOWED_SUBCOMMANDS` (line 1619), add the `QA_REVIEW_GH_ALLOWED: &[(&str, &str)]` table verbatim from Design § "Narrow allowlist".

2. **Add helper.** In the same file, add `fn validate_qa_review_gh_scope(args: &[String], ctx: &ToolContext<'_>) -> Result<(), ToolOutput>` verbatim from Design § "Insertion point". Place it directly after `validate_gh_input` to keep the two related validators co-located.

3. **Wire into `run_gh`.** In `run_gh` (line 1819), after the `match validate_gh_input(input)` block, before the PR-dedup logic, insert:
   ```rust
   if let Err(err) = validate_qa_review_gh_scope(&gh_args.args, ctx) {
       return err;
   }
   ```

4. **Reconcile system prompt.** Edit `skills/bundled/qa-review/system_prompt.md` line 586 per Design § "System-prompt reconciliation". Verify the surrounding paragraph still scans cleanly (the `--repo` sibling-parameter explanation upstream of the Permitted clause stays unchanged).

5. **Bump skill version.** `skills/bundled/qa-review/skill.toml`: `version = "0.8.0"` → `"0.9.0"`.

6. **Add Layer A — inline validator tests in `builtin_handlers.rs`.** Append `#[tokio::test]`/`#[test]` cases to the existing `mod tests` at the bottom of `builtin_handlers.rs` (siblings to `test_run_gh_allowlist_accepts_valid` at :2711, `test_run_gh_allowlist_rejects_removed_subcommands` at :2722, etc.). Helper `validate_qa_review_gh_scope` is private (`pub(super)` is sufficient — already visible to the inline test module). Each case is isolated, no shared state. Test name prefix: `test_validate_qa_review_gh_scope_*`. Cases:

   | # | Case | active_skill_paths | command | Expected |
   |---|------|--------------------|---------|----------|
   | 1 | qa-review active, `pr merge` rejects | `[qa-review]` | `["pr","merge","123"]` | `Err` containing `"qa-review's scope"` and `"mika#1196"` |
   | 2 | qa-review active, `api -X PATCH` rejects | `[qa-review]` | `["api","-X","PATCH","/repos/.../milestones/N"]` | `Err` containing `"qa-review's scope"` |
   | 3 | qa-review active, `issue close` rejects | `[qa-review]` | `["issue","close","123"]` | `Err` |
   | 4 | qa-review active, `pr edit` rejects | `[qa-review]` | `["pr","edit","123","--add-label","x"]` | `Err` |
   | 5 | qa-review active, `pr review` accepts | `[qa-review]` | `["pr","review","123","--approve"]` | `Ok` |
   | 6 | qa-review active, `pr diff` accepts | `[qa-review]` | `["pr","diff","123"]` | `Ok` |
   | 7 | qa-review active, `pr list` accepts | `[qa-review]` | `["pr","list"]` | `Ok` |
   | 8 | qa-review active, `issue view` accepts | `[qa-review]` | `["issue","view","123"]` | `Ok` |
   | 9 | qa-review NOT active, `issue edit --remove-label ready` accepts | `[]` (or `[self-dev-webhook-ready-label]`) | `["issue","edit","123","--remove-label","ready"]` | `Ok` |
   | 10 | qa-review NOT active, `pr merge` accepts (global allowlist applies) | `[]` | `["pr","merge","123"]` | `Ok` |

   Inline tests construct a `ToolContext` via the same shape used by existing inline tests at :2711-2900 (typically via a local `test_ctx()` helper or direct struct literal). `active_skill_paths` is set to a slice of `SkillPathInfo { skill_name, prompt_relative_path }` constructed in the test.

7. **Add Layer B — eval-suite wiring test.** Create `crates/mika-agent/tests/eval/test_qa_review_run_gh_scope_validator.rs` and register the module in `crates/mika-agent/tests/eval.rs` (`pub mod test_qa_review_run_gh_scope_validator;` inside the `mod eval { ... }` block). The test follows the `test_pr_review_idempotency.rs` shape: `EvalHarness` + `MockLlmProvider` emit a tool-call sequence; assertions inspect the trace.

   **Two scenarios, each as a single `#[tokio::test]`:**

   - **Scenario 1 — qa-review active rejects forbidden subcommand.** Provision a `mika-qa`-shaped agent identity in the harness's `home_dir` tempdir with `qa-review` skill enabled (mirroring the `MIKA_QA_IDENTITY` shape from `well_known_agents.rs`). `MockLlmProvider` emits a tool_use(`run_gh`, `{"command":["pr","merge","123"]}`). Run one turn. Assert the captured tool_result for the `run_gh` call contains the literal string `"qa-review's scope"` AND `"mika#1196"`. Assert no subprocess spawn occurred (i.e., the rejection happened before subprocess dispatch — verifiable via absence of any `gh_api_invocation` trace event or by inspecting the tool result for the structured scope error rather than a subprocess-failure error).
   - **Scenario 2 — qa-review NOT active accepts dispatch-ack.** Provision a `mika-dev`-shaped identity without `qa-review` in its skill allowlist (per `well_known_agents.rs:1546` `db.set_skill_enabled("mika-dev", "qa-review", false)`). `MockLlmProvider` emits a tool_use(`run_gh`, `{"command":["issue","edit","123","--remove-label","ready"]}`). Run one turn. Assert the tool_result for the `run_gh` call does NOT contain `"qa-review's scope"`. Since the real subprocess would attempt to call `gh` and may fail in the test environment (no live `gh`/`GH_TOKEN`), assert specifically against the *scope-rejection error string's absence*, not subprocess success — the wiring contract is "validator does not fire," not "subprocess succeeds."

   **What this layer tests vs. Layer A:** Layer A verifies the validator function's behavior in isolation (correctness of the predicate and the rejection message). Layer B verifies the wiring contract: `active_skill_paths` flows from the agent loop into `ctx`, the validator is called inside `run_gh`'s dispatch, and the predicate's identity-discrimination invariant (qa-review only on mika-qa) holds end-to-end. A regression in any of those three threading hops would fail Layer B but might pass Layer A.

   **Cross-mode coverage caveat (eval-suite limitation).** The eval suite runs `run_agent()` in conversation mode, which populates `active_skill_paths` from `matched_entries`. Silent/team/investigate modes are not covered by Layer B. They remain covered by Layer A's `active_skill_paths: &[]` case (case 9), which models the silent-mode shape.

8. **Verify existing shadowing-guard test still passes.** `cargo test -p mika-agent --test qa_review_run_gh_shadowing_guard` must remain green; the per-skill `tools.json` registration stays absent (the helper is at the handler layer, not the skill layer).

## Acceptance-criteria reconciliation

| AC | Implementation surface |
|----|------------------------|
| AC1 — qa-review active → `pr merge`, `api`, `issue close`, `pr edit` reject with structured error citing scope | Step 2 (helper) + step 6 cases 1–4 (Layer A inline) + step 7 Scenario 1 (Layer B eval) |
| AC2 — qa-review not active → `issue edit --remove-label ready` succeeds | Step 2's early-return when `qa_review_active == false` + step 6 case 9 (Layer A inline) + step 7 Scenario 2 (Layer B eval) |
| AC3 — hermetic regression guard at `crates/mika-agent/tests/eval/test_qa_review_run_gh_scope_validator.rs` covers both sides | Step 7 (Layer B eval test at AC3's path, registered in `tests/eval.rs`). Layer A (step 6) is the load-bearing validator regression guard at the established `test_run_gh_*` inline-test convention; Layer B is the wiring regression guard at the AC's path. Both layers ship together. |

## Out of scope

- Generalizing skill-scoped argv validation into a framework (`SkillScopedValidator` trait, per-skill TOML allowlists, etc.). YAGNI — only qa-review currently needs this. Two-skill demand would justify; one does not.
- Re-introducing `gh api` argument gating for non-qa-review skills (`api -X PATCH` audit-but-allow). Tracked under mika#788 territory; orthogonal here.
- Updating `permission-policy` tier1.py rules. Tier1 operates at a different layer (deterministic-policy preflight) and is orthogonal to the handler-layer scope check.
- Plumbing `MatchReason` into `SkillPathInfo`. Not currently motivated — agent-scoping already discriminates.

## Risks

1. **Mode coverage.** `active_skill_paths` is `&[]` in silent/team/investigate modes (per `crates/mika-agent/CLAUDE.md:96`). qa-review's `run_gh` calls in those modes would bypass the scope check. **Verification:** qa-review's verdict-posting flow runs in conversation mode (the LLM turn that processes a PR diff and emits the `VERDICT:` trailer). Silent and team modes do not run qa-review verdicts on mika-qa. The risk would materialize only if a future skill change runs qa-review's tool surface from a non-conversation mode — out of scope today.

2. **Agent-scoping drift.** The detection predicate depends on the invariant "qa-review is enabled only on mika-qa." If a future agent enables qa-review without re-scoping its dispatch-ack flow, the validator would fire on that agent. **Mitigation:** The well-known-agents test at `crates/mika-agent/src/well_known_agents.rs:1503` already encodes `let qa_only = ["qa-review", "qa-review-build-callback", "skill-review"]` and asserts cross-agent allowlist symmetry. The new test (case 9) implicitly re-asserts the invariant by passing `active_skill_paths: &[]` for the dispatch-ack scenario.

3. **Layer B test setup brittleness.** The Layer B test provisions a mika-qa-shaped identity in the harness `home_dir` tempdir to drive `active_skill_paths` population. If the well-known-agents identity shape changes (`MIKA_QA_IDENTITY` content, `[skills].allowlist` semantics, qa-review's `always_on=true` flag, etc.), Layer B may silently start passing without exercising the validator. Mitigation: Layer A is the load-bearing test; Layer B's assertion checks for a literal string ("qa-review's scope") that only the new validator can produce — a wiring break that silently bypasses the validator would fail Scenario 1's assertion. Cross-check: Layer A also asserts the same literal, so a string change in the validator gets caught at both layers.

4. **Error-message friction.** The structured error names the helper (`validate_qa_review_gh_scope`) and the ticket (`mika#1196`) so post-incident grep finds the source quickly. If qa-review's LLM hits the error mid-verdict, it should redirect to `pr review` (the canonical posting verb). The error string explicitly enumerates the allowed pairs to make this loop short.

## Verification

```bash
# Layer A (inline validator tests in builtin_handlers.rs):
cargo test -p mika-agent --lib test_validate_qa_review_gh_scope

# Layer B (eval-suite wiring test):
cargo test -p mika-agent --test eval test_qa_review_run_gh_scope_validator

# Existing shadowing-guard still passes:
cargo test -p mika-agent --test qa_review_run_gh_shadowing_guard

# Existing run_gh inline tests (regression guard for the validator chain):
cargo test -p mika-agent --lib test_run_gh

# No new clippy lints on the touched module:
cargo clippy -p mika-agent --tests -- -D warnings

# Smoke (mika-qa, conversation mode):
# 1. Trigger a qa-review verdict turn against a PR.
# 2. Verify `run_gh pr review --body ... --approve` succeeds end-to-end.
# 3. Manually force a `run_gh pr merge` (via prompt-inject test fixture) — verify it rejects with the structured error containing "mika#1196".

# Audit trail:
# Verify the existing `tracing::info!(event = "gh_api_invocation", ...)` on line 1900-ish does not fire on the qa-review agent (because api never reaches the audit point — it rejects at scope).
```

## Related

- **mika#1168** PR #1197 — the b2 fix that motivated this follow-up. ce:review SEC-1 finding (P1, 0.72) is the proximate trigger.
- **`docs/plans/2026-05-17-003-bug-1168-dispatch-loss-co-causes-plan.md`** — predecessor plan that documents why per-skill handler shadowing was retired.
- **`feedback_prompt_enforcement_fragile.md`** — validates the structural-not-prompt choice this plan implements.
- **mika#788** — `gh api` audit / arbitrary-method gating; orthogonal but adjacent.
- **`crates/mika-agent/tests/qa_review_run_gh_shadowing_guard.rs`** — sibling test that locks in the negative invariant (per-skill `run_gh` stays out of qa-review's `tools.json`).
