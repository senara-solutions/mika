# Plan: cpp#20 Joint-2 Contract Revision — Adaptation vs Fabrication Distinction

**Ticket:** mika#1410
**Type:** design (architectural revision)
**Repo:** senara-solutions/claude-pilot-py + senara-solutions/mika (coordination)

## Problem

cpp#20 joint-2 shipped `interrupt=True` on **all** policy denials (`permissions.py:245,250,258`). This is the honest-halt contract: when policy denies a tool call, the SDK closes the stdio pipe, the synthetic-emit guard fires (`agent.py:242-259`), and the session terminates with `status="error", subtype="stream_ended_without_result"`.

This contract prevents fabrication (model claiming denied work succeeded) but also prevents adaptation (model substituting allowed tools to accomplish the same goal). Both cases trigger the same hard halt, even though only fabrication is the actual threat.

**Concrete impact:** when the model attempts `find ... -exec grep` (denied as RCE-class by the `DENIED_BASH_PATTERNS` hint), the session crashes instead of letting the model substitute `Grep + Glob + Read` — tools that accomplish the same code-search goal safely. mika#1409 shipped a prevention hint to reduce the *rate* of these attempts, but novel denied patterns the hint doesn't anticipate still crash sessions. The session-fatality class only closes when the contract structurally distinguishes adaptation from fabrication.

## Current State

### Permission evaluation flow (`permissions.py:191-316`)

```
Tier 1 (auto-approve) → Tier 1.5 (compact-safe auto-answer) → Tier 2 (policy evaluation) → Relay/Interactive fallback
```

At Tier 2, `policy.py::evaluate()` returns `PolicyDecision(decision, reason, rule_id)`. The handler maps decisions:
- `allow` (+ chained-danger check) → `PermissionResultAllow` or `PermissionResultDeny(interrupt=True)` on veto
- `deny` → `PermissionResultDeny(message, interrupt=True)` (line 250)
- `escalate` → `PermissionResultDeny(message, interrupt=True)` + notify (line 258)

All three denial paths use `interrupt=True`. No path allows the model to recover and try alternative tools.

### Bundled policy (`policies/permissions.yaml`)

46 rules + default-deny. Default deny reason: "no matching policy rule -- denied by default." The `DENIED_BASH_PATTERNS` hint in the system prompt (`agent.py::_system_prompt_with_hint()`) lists known-bad patterns and suggests alternatives, but operates at the prompt level, not the policy level.

### Synthetic-emit guard (`agent.py:242-259`)

When `interrupt=True` fires, the SDK closes the pipe without a `ResultMessage`. The guard emits `ResultJson(status="error", subtype="stream_ended_without_result")` so dispatch-lib always sees a parseable terminal line. This guard is load-bearing for the current all-halt contract.

## Design

### Core Distinction

| | Adaptation | Fabrication |
|---|---|---|
| **Definition** | Model uses allowed tools to accomplish the denied tool's goal | Model claims the denied tool's side-effect happened |
| **Example** | `find -exec grep` denied → model uses `Grep + Glob` | `rm -rf /data` denied → model says "deleted the directory" |
| **Structural signal** | Model's next tool calls are allowed alternatives | Model's next text claims the denied action succeeded |
| **Correct response** | Let the model continue (`interrupt=False`) | Halt honestly (`interrupt=True`) |
| **Machine-checkable?** | Yes — at policy-rule level (see below) | Yes — at policy-rule level + post-denial fabrication guard |

### Design: `recoverable` field on policy rules

Add an optional `recoverable` boolean to policy rules in `permissions.yaml`:

```yaml
rules:
  - id: bash-find-exec-deny
    tool: Bash
    pattern: "find\\s.*-exec"
    decision: deny
    reason: "find -exec is RCE-class; use Grep + Glob + Read instead"
    recoverable: true   # NEW: model can adapt using allowed tools

  - id: bash-destructive-deny
    tool: Bash
    pattern: "rm\\s+-rf\\s+/"
    decision: deny
    reason: "destructive operation denied"
    # recoverable defaults to false — hard halt
```

**Semantics:**
- `recoverable: true` → `PermissionResultDeny(message=..., interrupt=False)` — model gets the denial as a tool error and can try alternatives
- `recoverable: false` (default) → `PermissionResultDeny(message=..., interrupt=True)` — current behavior, hard halt
- Default deny (no matching rule) → always `interrupt=True` (fail-closed invariant preserved)
- `escalate` decision → always `interrupt=True` regardless of `recoverable` (escalations are loud halts by definition)

### Why per-rule, not per-tool or per-pattern-class

1. **Operator control.** The same tool (`Bash`) has both recoverable denials (`find -exec` → Grep) and non-recoverable denials (`rm -rf /` → no safe alternative). Per-rule granularity lets the operator make the call.
2. **Existing infrastructure.** `PolicyDecision` already carries `rule_id`. Adding `recoverable` to the rule and threading it through `PolicyDecision` is a 1-field extension, not a restructuring.
3. **Machine-checkable.** The classification is in the policy file, not inferred at runtime. An operator can audit exactly which denials are recoverable by reading `permissions.yaml`.

### Enhanced denial message for recoverable denials

When `recoverable: true`, the denial message returned to the model must:
1. State what was denied and why
2. State that the model MUST NOT claim the denied action succeeded
3. Suggest specific alternative tools (from the rule's `reason` field)

The existing `reason` field in policy rules already serves this purpose (e.g., "find -exec is RCE-class; use Grep + Glob + Read instead"). No new field needed — the `reason` text is the adaptation guidance.

### Post-denial fabrication guard (defense-in-depth)

Even with `interrupt=False`, the model might fabricate instead of adapting. Defense-in-depth:

1. **Existing guard (mika-side):** The agent loop's 11 EndTurn post-condition guards include `assert-grounded` which prohibits claims of downstream state without tool confirmation. A fabrication claim after a denied tool call would violate this guard.
2. **New guard (cpp-side, optional enhancement):** Track denied tool calls in `SessionGuardrails`. If the model's next `AssistantMessage` text contains the denied command's output pattern without a subsequent successful tool call, flag as fabrication. This is a heuristic, not a hard gate — the mika-side guard is the structural backstop.

**Decision:** The mika-side `assert-grounded` guard is the primary fabrication defense. The cpp-side heuristic is a nice-to-have for observability (log a warning) but is NOT a gate condition for this ticket. The architectural revision document should note both layers.

## Implementation Steps

### Step 1: Extend `PolicyDecision` and rule schema (claude-pilot-py)

**File:** `src/claude_pilot/policy.py`

- Add `recoverable: bool = False` to `PolicyRule` (Pydantic model)
- Thread `recoverable` through `PolicyDecision` dataclass
- `evaluate()` returns `PolicyDecision(..., recoverable=rule.recoverable)` on rule match
- Default deny (no rule match) always returns `recoverable=False`

**File:** `src/claude_pilot/policies/permissions.yaml`

- Add `recoverable: true` to rules where adaptation is safe (see list below)
- Leave all other rules and the default at `recoverable: false` (implicit default)

### Step 2: Wire `recoverable` into permission handler (claude-pilot-py)

**File:** `src/claude_pilot/permissions.py`

At line 250 (policy deny path):
```python
# Before (current):
return PermissionResultDeny(message=pd.reason, interrupt=True)

# After:
return PermissionResultDeny(message=pd.reason, interrupt=not pd.recoverable)
```

At line 258 (escalate path): **No change.** Escalations are always `interrupt=True` regardless of `recoverable`. The escalate path's `_fire_notify()` side-channel is a deliberate operator alert; recoverable semantics don't apply to loud halts.

At line 245 (chained-danger veto): **No change.** Chained-danger vetoes are structural safety, not policy classification. Always `interrupt=True`.

### Step 3: Classify existing rules (claude-pilot-py)

**File:** `src/claude_pilot/policies/permissions.yaml`

Rules to mark `recoverable: true` (model can substitute allowed native tools):

| Rule ID | Pattern | Why recoverable | Alternative |
|---------|---------|-----------------|-------------|
| (new) `bash-find-exec-deny` | `find.*-exec` | Code-search goal achievable via native tools | Grep + Glob + Read |
| (new) `bash-sed-i-deny` | `sed\s+-i` | File edit goal achievable via Edit tool | Edit |
| (new) `bash-grep-rg-deny` | `^(grep\|rg)\s` | Search goal achievable via Grep tool | Grep |
| (new) `bash-cat-head-tail-deny` | `^(cat\|head\|tail)\s` | File read goal achievable via Read tool | Read |
| (new) `bash-echo-redirect-deny` | `echo\s.*>` | File write goal achievable via Write tool | Write |
| (new) `bash-md5sum-deny` | `^(md5sum\|sha256sum)` | File inspection via Read tool | Read |
| (new) `bash-xargs-deny` | `xargs` | Iteration achievable via Grep + Glob | Grep + Glob |

**Note:** These patterns come directly from the `DENIED_BASH_PATTERNS` hint already in the system prompt (`agent.py::_system_prompt_with_hint()`). The hint tells the model what to use instead; the `recoverable: true` flag lets the model actually try it instead of crashing.

Rules that stay `recoverable: false` (default — hard halt):
- All existing allow/deny rules without a clear substitution path
- Default deny (no matching rule)
- Any future rule where the denied action has destructive side-effects with no safe alternative

### Step 4: Reconcile with system-prompt hint (claude-pilot-py)

**File:** `src/claude_pilot/agent.py`

The `DENIED_BASH_PATTERNS` hint in `_system_prompt_with_hint()` currently says "the permission policy DENIES the Bash patterns below, and a denied Bash call terminates this session immediately (no retry, no recovery)."

Update the hint text to reflect the new contract:
- Patterns with `recoverable: true` rules: "use the native tool instead" (keep existing guidance)
- Remove the "terminates this session immediately" language for recoverable patterns
- Keep the "terminates this session" language for non-recoverable patterns (destructive operations)

The hint remains a prevention layer (reduce attempts); the policy is the enforcement layer (handle attempts that slip through).

### Step 5: Tests (claude-pilot-py)

**File:** `tests/test_permissions.py`

Add:
1. `test_recoverable_deny_returns_interrupt_false()` — rule with `recoverable: true` returns `PermissionResultDeny(interrupt=False)`
2. `test_non_recoverable_deny_returns_interrupt_true()` — rule with `recoverable: false` (or absent) returns `PermissionResultDeny(interrupt=True)`
3. `test_default_deny_always_interrupt_true()` — no matching rule → `interrupt=True` regardless
4. `test_escalate_always_interrupt_true_regardless_of_recoverable()` — escalate ignores `recoverable` field

**File:** `tests/test_agent.py`

Add:
5. `test_recoverable_deny_does_not_terminate_session()` — mock SDK client where a recoverable denial returns the denial as a tool error; model proceeds to next tool call; session completes normally
6. `test_non_recoverable_deny_terminates_session()` — existing `test_agent_emits_synthetic_terminal_on_silent_stream_end` covers this case; add a docstring linking to this design

**File:** `tests/test_rules.py` or `tests/test_policy_devpilot.py`

Add:
7. `test_recoverable_rules_parse_correctly()` — round-trip: load `permissions.yaml`, find rules with `recoverable: true`, verify they parse and evaluate correctly
8. `test_find_exec_denied_as_recoverable()` — specific rule: `find . -exec grep {} \;` matches `bash-find-exec-deny` with `recoverable=True`
9. `test_destructive_bash_denied_as_non_recoverable()` — specific: `rm -rf /` matches no recoverable rule

### Step 6: Architectural revision document (mika)

**File:** `docs/adr/NNN-cpp20-joint2-adaptation-vs-fabrication.md`

Document the contract revision as an ADR:
- **Context:** cpp#20 joint-2 shipped all-halt; mika#1410 identified the adaptation/fabrication distinction
- **Decision:** `recoverable` field on policy rules; per-rule operator control; fail-closed default
- **Consequences:** Novel denied patterns with known alternatives no longer crash sessions; destructive denials still halt; operator can reclassify rules by editing `permissions.yaml`
- **Backward compatibility:** All existing callers see identical behavior (default `recoverable: false` preserves `interrupt=True`)

### Step 7: Document in cpp#20's followup chain (claude-pilot-py)

Add a section to cpp#20's issue body or a comment linking to mika#1410 and the ADR, so the next session-fatality incident has the contract to reference.

## Backward Compatibility

- **Default behavior unchanged.** `recoverable` defaults to `false`. Existing policy files without the field produce identical `interrupt=True` behavior.
- **Policy file schema is additive.** New optional field; old files parse without error (Pydantic `Field(default=False)`).
- **Synthetic-emit guard unchanged.** Still fires for non-recoverable denials where `interrupt=True` closes the pipe.
- **Existing tests pass.** All current tests assert `interrupt=True`; they continue to pass because they test rules without `recoverable: true`.
- **Dispatch-lib unchanged.** The terminal JSON contract (`status`, `subtype`) is unchanged. Recoverable denials that the model adapts to produce normal `status="success"` results. Non-recoverable denials produce the existing `stream_ended_without_result` result.

## Sequencing

This is a single-PR change to `claude-pilot-py` with a companion ADR commit in `mika`. No cross-repo coordination beyond the ADR.

1. **claude-pilot-py PR:** Steps 1-5 + Step 7 (policy schema + handler wiring + rules + hint update + tests + cpp#20 followup note)
2. **mika PR (or same-branch commit):** Step 6 (ADR document)

The change is backward-compatible, so no soak period is needed between the schema extension and rule classification. Both land in one PR.

## Risks

1. **False classification.** A rule marked `recoverable: true` for a pattern that has no real alternative → model loops trying alternatives, wastes turns, eventually hits max_turns. **Mitigation:** Only classify rules where the alternative is a known native tool (Read/Write/Edit/Grep/Glob). Conservative default (`false`) means new rules are safe until explicitly classified.

2. **Fabrication on recoverable denial.** Model claims the denied action's side-effect happened after receiving `interrupt=False`. **Mitigation:** mika-side `assert-grounded` EndTurn guard catches claims of downstream state without tool confirmation. This is existing defense-in-depth, not new code.

3. **Policy file drift.** Operator overlay (`MIKA_PILOT_POLICY_PATH`) doesn't include `recoverable` fields → all denials are hard halts (the default). **Mitigation:** This IS the correct fail-closed behavior. Operators who want recoverable denials add `recoverable: true` to their overlay rules.

## Out of Scope

- **Automatic substitution engine.** The model decides what alternative to use, not the permission system. The policy says "you can try again with different tools"; it doesn't rewrite the tool call.
- **Fabrication detection heuristic in cpp.** The mika-side `assert-grounded` guard is the primary defense. A cpp-side heuristic (track denied calls, check subsequent text) is a future enhancement, not part of this revision.
- **Relay path changes.** The relay fallback (`permissions.py:264-314`) already uses `interrupt=False`. No change needed.
- **cpp#21 rename.** `escalate` → `deny_with_notify` rename is a separate concern (already shipped per the explore findings showing the runtime semantics are in place).
