# Plan: Mid-Flight ask_architect Channel — mika#1245

## Problem

The implementing pilot occasionally discovers a real ambiguity at execution time that grooming could not have anticipated. Today the pilot's only option is `AskUserQuestion`, which either gets auto-answered (Tier 1.5 compact-safe), evaluated by deterministic policy (Tier 2), or — in the legacy relay path — forwarded to mika-dev's permission-policy skill. None of these paths reach mika-arch for architectural judgment. The pilot ends its turn, claude-pilot triggers `pipeline_incomplete`, and the question goes nowhere.

This is orthogonal to mika#1244 (upstream grooming gate). mika#1244 ensures GROOMED plans have zero TBDs. This ticket handles ambiguities that emerge DURING implementation — invisible at grooming time.

## Design Decisions

### D1: Reuse `mika ask --agent mika-arch` (Bash tool), not a new engine tool

The pilot already has Tier 1 auto-approval for `mika ask --agent mika-arch` via both Python (`tier1.py` intra-platform dispatch allow-list) and Rust (`permission_pre_classifier.rs`). A Bash call from the pilot session invokes the CLI, which routes to mika-arch's agent loop with full KG access, tool access, and session continuity. No new engine tool, no new callback type, no suspend/resume protocol needed.

**Why not a custom `ask_architect` tool?** A custom tool in the engine's `ToolRegistry` would require:
- New Rust tool struct + handler + registration in `default_tools()`
- New callback task type for async Q&A
- Suspend/resume protocol in claude-pilot SDK
- New permission tier in policy.py

All of this to replicate what `mika ask --agent mika-arch` already does synchronously. The CLI path is simpler, tested, and the pilot's turn blocks until the answer arrives — exactly the semantics we want.

### D2: New `mika-arch-answer-question` skill for mid-flight Q&A disposition

A new bundled skill on mika-arch, distinct from `mika-arch-groom-ticket` (plan review) and `mika-arch-second-review` (second-pass iteration). This skill:
- Receives a structured question (context, decision needed, candidate options)
- Has access to KG, codebase search, and docs
- Returns either an ANSWER with architectural decision + rationale + citations, or ESCALATE with reason

The ANSWER/ESCALATE disposition format mirrors the existing READY/ITERATE/ESCALATE pattern from groom skills.

### D3: Pilot invocation via `--enable-skill mika-arch-answer-question`

The pilot constructs a Bash command:
```bash
mika ask --agent mika-arch --enable-skill mika-arch-answer-question - <<'EOF'
## Architectural Question (mid-flight)

**Issue:** mika#NNN — <title>
**Plan:** docs/plans/<plan-file>.md
**Phase:** /ce:work Phase N — <description>

### Context
<what the pilot has discovered during implementation>

### Decision Needed
<the specific architectural question>

### Candidate Options
1. <option A> — <tradeoff>
2. <option B> — <tradeoff>
EOF
```

This is a synchronous call — the pilot's turn blocks until mika-arch responds. The response is the tool result from the Bash call, which the pilot reads and acts on.

### D4: ESCALATE fallthrough uses existing `send_message` + pause

When mika-arch returns `Disposition: ESCALATE`, the pilot:
1. Reads the ESCALATE reason from mika-arch's response
2. Calls `send_message` to notify Vincent via Telegram with the question context
3. Ends its turn cleanly (not `pipeline_incomplete` — this is a deliberate pause)

The operator answers via the existing Telegram → mika-dev → callback path. This is NOT an automatic resume — the operator manually re-dispatches with `iteration_context` containing the answer. This matches the existing iterate pattern and avoids building new suspend/resume machinery.

### D5: Budget cap — max 2 architect asks per pilot session

Prevents runaway architect consultations. The pilot tracks asks via a simple counter in its execution state. After 2 asks, further ambiguities must be resolved by the pilot's own judgment or by ending the turn with a clear question for the operator.

## Implementation Steps

### Step 1: Create `mika-arch-answer-question` bundled skill

**Files:**
- `skills/bundled/mika-arch-answer-question/skill.toml`
- `skills/bundled/mika-arch-answer-question/system_prompt.md`

**skill.toml:**
```toml
[skill]
name = "mika-arch-answer-question"
version = "0.1.0"
keywords = ["architect", "question", "mid-flight", "answer"]
```

**system_prompt.md contract:**
- Activation: receives a structured question with context, decision needed, and candidate options
- Available tools: `query_knowledge_graph`, `conversation_search`, `recent_chats`, `read_agent_file`, `web_search` (same as groom skills — read-only, no code generation)
- Output format:
  - If answerable: `Disposition: ANSWER` followed by the decision, rationale, and citations (F-list format)
  - If not answerable (requires operator judgment, scope change, business decision): `Disposition: ESCALATE` followed by reason and what the operator needs to decide
- Grounding rule: decisions must cite at least one of: ADR, review-guide.md section, compound doc, codebase convention, or explicit ticket requirement
- Constraint: single-turn response only. No back-and-forth with the pilot. The architect gives a decision or escalates.

### Step 2: Register skill in mika-arch's identity allowlist

**File:** `crates/mika-agent/src/well_known_agents.rs`

Add `"mika-arch-answer-question"` to `build_mika_arch_identity()`'s skills allowlist (currently 3 skills: `mika-arch-groom-ticket`, `mika-arch-second-review`, `mika-arch-groom-milestone`). This becomes 4.

### Step 3: Add `ask_architect` prompt guidance to dev-pilot system prompt

**File:** `skills/bundled/dev-pilot/system_prompt.md`

Add a section explaining when and how to use the mid-flight architect channel:

```markdown
## Mid-Flight Architect Questions

When implementation discovers an ambiguity not covered by the plan — e.g., an invariant 
the plan didn't account for, an API that changed, or a design choice with architectural 
implications — you MAY consult mika-arch before proceeding.

**When to ask:**
- The plan assumed X but the codebase shows Y, and the right resolution isn't obvious
- Two valid implementation approaches exist with different architectural tradeoffs
- A discovered constraint might change the plan's scope or approach

**When NOT to ask:**
- Implementation details within the plan's stated approach (just decide)
- Questions the plan already answers (re-read the plan)
- Questions about tooling, syntax, or API usage (use web_search or docs)

**How to ask:**
```bash
mika ask --agent mika-arch --enable-skill mika-arch-answer-question - <<'ARCH_Q'
## Architectural Question (mid-flight)

**Issue:** <repo>#<number> — <title>
**Plan:** docs/plans/<plan-file>.md
**Phase:** /ce:work Phase N — <description>

### Context
<what you discovered>

### Decision Needed
<the specific question>

### Candidate Options
1. <option A> — <tradeoff>
2. <option B> — <tradeoff>
ARCH_Q
```

**Budget:** Maximum 2 architect asks per session. After that, decide yourself or 
end the turn with a clear question for the operator.

**Reading the response:**
- `Disposition: ANSWER` — follow the architect's decision. Continue implementation.
- `Disposition: ESCALATE` — the question requires operator judgment. Call `send_message` 
  with the question context and the architect's escalation reason. End the turn cleanly 
  (this is a deliberate pause, not a failure).
```

### Step 4: Ensure Tier 1 auto-approval covers the `--enable-skill` variant

**File:** `claude-pilot-py/src/claude_pilot/tier1.py`

The existing intra-platform dispatch allow-list covers `mika ask --agent mika-arch`. Verify it also matches when `--enable-skill mika-arch-answer-question` is present. The current pattern matching in `is_safe_bash_command()` likely uses prefix/substring matching — confirm and adjust if needed.

**File:** `crates/mika-agent/src/server/permission_pre_classifier.rs`

Same verification on the Rust side. The pre-classifier's intra-platform dispatch detection must not reject the `--enable-skill` flag.

Both sides must stay in sync per the cross-language sentinel contract (mika#946).

### Step 5: Add `ask_architect` to the dev-pilot policy file

**File:** `claude-pilot-py/src/claude_pilot/policies/default.yaml` (or equivalent)

Add an explicit `allow` rule for `mika ask --agent mika-arch --enable-skill mika-arch-answer-question` to the deterministic policy evaluator. This provides defense-in-depth — even if Tier 1 matching has a gap, the policy file catches it.

Rule:
```yaml
- id: mid-flight-architect-ask
  tool: Bash
  pattern: "mika ask --agent mika-arch --enable-skill mika-arch-answer-question"
  decision: allow
  reason: "Mid-flight architect consultation (mika#1245)"
```

### Step 6: Update self-dev-callback to recognize architect-ask pauses

**File:** `skills/bundled/self-dev-callback/system_prompt.md`

Add recognition for the case where a pilot session ends after calling `send_message` with an architect ESCALATE question. The callback should:
- Detect the pattern: `Disposition: ESCALATE` in the pilot's output + `send_message` call
- Classify as "architect_escalate_pause" (not pipeline_incomplete, not failure)
- NOT auto-retry — this is a deliberate pause pending operator input
- Surface to operator with clear context: "Pilot paused on architectural question. See Telegram message for details."

### Step 7: Tests

**7a. Skill discovery test:**
Add `"mika-arch-answer-question"` to the static parity assertion in `crates/mika-agent/tests/eval/` (recently added per mika#1325) to ensure the new skill is discovered at build time.

**7b. Tier 1 matching test (Python):**
Add test case in `claude-pilot-py/tests/` verifying that `mika ask --agent mika-arch --enable-skill mika-arch-answer-question - <<'ARCH_Q' ...` is auto-approved by `is_tier1_auto_approve()`.

**7c. Pre-classifier test (Rust):**
Add test case in `crates/mika-agent/src/server/permission_pre_classifier.rs` verifying the `--enable-skill` variant is classified correctly.

**7d. Identity allowlist test:**
Verify `build_mika_arch_identity()` includes `mika-arch-answer-question` in the allowlist by checking the generated identity string.

## Sequencing

Steps 1–2 are the core (new skill + allowlist). Step 3 teaches the pilot. Steps 4–5 ensure permissions. Step 6 handles the pause case. Step 7 validates.

All steps are in `mika/` except Step 4–5 which touch `claude-pilot-py/`. This is a cross-repo feature:
- **Primary repo:** `mika` (skill, identity, dev-pilot prompt, pre-classifier, callback)
- **Secondary repo:** `claude-pilot-py` (tier1 verification, policy rule)

Per CLAUDE.md cross-repo conventions: same branch name `feat/1245/mid-flight-ask-architect` in both repos, primary completed first.

## Risks

1. **Synchronous blocking:** `mika ask` is synchronous — the pilot's turn blocks while mika-arch thinks. mika-arch runs on the same host, so this adds 30–120s of latency per ask. Acceptable given the 2-ask budget cap and the alternative (ending the turn entirely).

2. **mika-arch availability:** If mika-arch is down or misconfigured, the `mika ask` call fails. The pilot should treat this as "unable to consult architect" and proceed with its own judgment (fail-open for implementation, fail-closed for ESCALATE-worthy decisions).

3. **Context leakage:** The pilot sends implementation context to mika-arch. mika-arch is read-only and scoped to the same workspace, so this is acceptable. No secrets are sent (the question is about architecture, not credentials).

4. **Prompt bloat in dev-pilot:** Adding ask_architect guidance increases the dev-pilot system prompt. Mitigated by keeping the guidance concise and structured.

## Out of Scope

- **Automatic resume after ESCALATE:** Operator manually re-dispatches. Building auto-resume requires SDK-level suspend/resume in claude-pilot, which is a separate feature.
- **Multi-turn architect dialogue:** Single-turn only. If the pilot needs clarification on the answer, it asks a new question (counts against the 2-ask budget).
- **Question schema validation:** The question format is prompt-guided, not schema-enforced. Misformatted questions get poor answers from mika-arch, which is self-correcting (pilot learns to format better).
