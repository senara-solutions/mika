---
module: mika-skills
tags: [contract-refactor, dispatch-lib, iterate-loop, mika-1271, contract-design, autonomous-loop, dev-groom]
problem_type: contract-design
category: architecture-patterns
date: 2026-05-25
---

# Pilot-vs-substrate contract split: pilot owns content, dispatch-lib owns workflow

## Problem

The `dev-groom` autonomous-loop pilot originally did everything: read the ticket, generate a plan, invoke the architect for a first-pass review, apply iterations, invoke the architect again for a second-pass, write the canonical body callout, post a comment, push the branch. The structural alternative — dispatch-lib doing architect convergence + body-callout writing — also existed in parallel (`_verify_and_write_body_callout` recovery shim from mika#1123). Over four months of operation, three compounding failure modes emerged:

1. **Recovery layers stacked recursively.** `_post_flight_push` (mika#1268) caught wrote-but-no-commit. Class D recovery (mika#1123) caught wrote-and-committed-but-no-callout. A proposed third layer (`_iterate_recovery`) would have caught architect-misroute-but-recovered-by-LLM. Each layer was adding to the substrate's pipeline-failure detection budget, not removing the underlying contract violation.

2. **Cost regression on every groom.** When the iterate loop landed (mika#1271 sub-PR 7a) and ran alongside the OLD pilot-owns-architect flow, every groom invoked the architect 4 times (pilot pass 1 + pass 2 + iterate-loop pass 1 + pass 2). Body-callout writes happened 2-3 times (pilot organic + Class D recovery + canonical writer), all overlapping.

3. **Pilots structurally couldn't commit.** Pilot session logs from 2026-05-24/25 (`9fb5c2bd`, `b9c8f517`) showed pilots writing files via Edit but never invoking `git add`/`git commit`/`git push` at all. The contract said "pilot is responsible for committing"; the empirical reality was "pilot doesn't commit." Not a tooling failure — a contract-level miss.

## Insight

The pilot can do CONTENT (the plan) OR WORKFLOW (architect calls, git operations, body callouts). Asking it to do BOTH conflates two distinct contracts, and the failure modes compound because no single layer of the substrate knows whether the previous layer succeeded at its work.

The architect-validated refactor (session `0583a902-cd7a-45ab-89be-59e13c8b09ec`, verdict `flip`) split the contract:

- **Pilot owns CONTENT.** Generate the plan via `/ce:plan`. Commit. Push. Exit.
- **dispatch-lib owns WORKFLOW.** Invoke architect (first-pass + second-pass on READY/ITERATE branches). Write canonical body callout on GROOMED. Emit structured `PIPELINE FAILURE` markers on ESCALATE per mika#1033.

The pilot and the substrate each become single-author of their respective surfaces. No detection-recovery layers needed; the substrate's architect convergence + structured failure markers replace silent recovery.

## Implementation pattern

### Two slash commands, one structural authority

- **`/mika-groom-ticket`** (mika-platform) — operator-facing full pipeline. Phase 1-6 including architect calls + body callout + comment. Used when Vincent runs grooming directly from his Claude Code session.
- **`/mika-groom-plan-only`** (mika-platform) — autonomous-loop content-only sibling. Phase 1-3 only: read ticket, generate plan, commit, push, exit. Used when dispatch-lib invokes the pilot via `ENTRY_COMMAND`.

The split is autonomous-loop vs operator-direct — same data, different invocation surface. `/mika-revise-plan` (sub-PR 4 of the refactor) was the first instance of this pattern: a content-only slash command for the iterate-loop ITERATE branch's revise step.

### State machine in dispatch-lib

`_iterate_groom_loop()` in `skills/bundled/_shared/dispatch-lib.sh` runs after the pilot exits. Five terminal states:

```
READY    → second-pass GROOMED → _write_canonical_callout "ready-to-groomed"
READY    → second-pass *      → _escalate_groom "second-pass-after-ready"
ITERATE  → revise → second-pass GROOMED → _write_canonical_callout "iterate-to-groomed"
ITERATE  → revise → second-pass *      → _escalate_groom "second-pass-after-iterate"
ESCALATE (first-pass)                   → _escalate_groom "first-pass"
```

- `_arch_ask` invokes `mika ask --agent mika-arch --format json --verbose --enable-skill <skill>`.
- `_parse_disposition` / `_parse_verdict` tolerate the canonical literal shapes (`Disposition: READY|ITERATE|ESCALATE`, `Verdict: GROOMED|ESCALATE`). Paraphrased shapes are mika#1272's scope.
- `_launch_revise_pilot` invokes claude-pilot with `/mika-revise-plan @<findings-file>` for the ITERATE branch.
- `_write_canonical_callout` is idempotent — checks for the three dispatch-gate signals (Branch line + Plan path + `second-pass (GROOMED)` substring) and skips writing if all three are present.
- `_escalate_groom` preserves architect findings to `$WORKTREE_DIR/.iterate/escalate-<stage>.md` AND appends a structured `PIPELINE FAILURE` block to RESULT. Findings are PRESERVED on ESCALATE (never swept by cleanup); only GROOMED success paths call `_cleanup_iterate_findings`.

### Detect-and-fail-loudly, not detect-and-recover

The refactor explicitly retired the Class D recovery shim (sub-PR 7b). The architect's `(i) Retire` verdict ratified the trade-off: if the iterate loop's architect convergence fails AND the pilot's organic write also drifts, there is no fallback. Dispatch gate fails on the subsequent dispatch — same as pre-mika#1123 production posture. The structural surface is mika#1033's PIPELINE FAILURE markers, not silent recovery.

This is the inverse of the detection-recovery layer compounding pattern: detection collapses *into* the contract, not parallel to it. Each layer the substrate adds creates the next layer's failure mode; eventually the operator needs to look at five different recovery shims to understand what happened. Failing loudly via architect convergence is the alternative — the operator sees the structured failure once, in the dispatch result, with the architect's reasoning preserved at `.iterate/escalate-<stage>.md`.

## When to apply this pattern

The pilot-vs-substrate split applies whenever an autonomous-loop step has BOTH content generation AND workflow control responsibilities, AND failure modes are being papered over by recovery layers in the substrate. Symptoms that indicate the split is the right move:

- N≥3 recovery layers stacking on the same contract (e.g., post-flight push, post-flight callout-recovery, post-flight iterate-recovery).
- Pilot sessions empirically skipping the workflow steps the contract says they own (e.g., zero git commands across the entire session).
- Cost regression where N parallel paths each invoke the same expensive operation (e.g., 4 architect calls per groom).

The fix is mechanical once recognized: split the slash command into operator-direct (full pipeline) and autonomous-content-only siblings. Move the workflow steps from the pilot's slash command into the substrate's outer layer. Retire the recovery shims that catered to the pilot's incomplete contract.

## Anti-pattern: collapsing both into the pilot for simplicity

The first instinct when seeing a 2-layer system with overlapping writes is often "just make the pilot do everything; remove the substrate layer." This was rejected during the architect's three-round refactor decision because:

- The pilot's content-generation contract is LLM-shape work (judgment, context, plan structure).
- The workflow contract is deterministic (architect API calls, git commands, body-callout regex matching).
- Mixing them couples LLM-shape failure modes to deterministic-shape failure modes. A bad plan ALSO breaks git operations. A network blip ALSO breaks the architect verdict.

The substrate's deterministic surface deserves to live in deterministic code (bash + dispatch-lib), not in LLM-prompt-driven slash commands.

## References

- `mika/skills/bundled/_shared/dispatch-lib.sh::_iterate_groom_loop` — the state machine.
- `mika/skills/bundled/_shared/dispatch-lib.sh::_write_canonical_callout` — canonical body-callout writer (sub-PR 6).
- `mika/skills/bundled/_shared/dispatch-lib.sh::_escalate_groom` — structured PIPELINE FAILURE marker (sub-PR 5).
- `mika-platform/.claude/commands/mika-groom-plan-only.md` — autonomous-loop content-only slash command.
- `mika-platform/.claude/commands/mika-revise-plan.md` — ITERATE-branch content-only slash command (pattern precedent).
- `mika-platform/.claude/commands/mika-groom-ticket.md` — operator-direct full pipeline (unchanged by the refactor).
- mika#1271 — contract refactor parent ticket (sub-PRs 1–8 across PRs #1273–#1281, merged 2026-05-25).
- mika#1033 — detect-and-fail-loudly precedent (the structural surface replacing detection-recovery layers).
- mika#1272 — paraphrased disposition handling (extends `_parse_disposition` tolerance; separate ticket).
- Architect session `0583a902-cd7a-45ab-89be-59e13c8b09ec` — three-round contract refactor decision (`flip` / `(i) Retire` / `yes`).
- Memory: `feedback_manual_rescue_is_contract_evidence_not_throughput` — manual rescues are disconfirmation of the contract, not throughput; "stamina-as-platform" is not the thesis.
- Memory: `feedback_binary_staleness_vs_main` — calibration finding from sub-PR 7a's first-test (substrate was on main but binary was stale; `make deploy` was the missing step).
