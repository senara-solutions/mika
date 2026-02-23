---
title: "Parallel 7-Agent Code Review Methodology: Mika Home Directory Agent Core"
date: 2026-02-24
category: code-review-workflow
tags:
  - code-review
  - multi-agent
  - parallel-processing
  - rust
  - mika
  - synthesis
  - workflow-pattern
  - deduplication
severity: informational
component: full-stack
status: resolved
problem_type: workflow_pattern
root_cause: "Large, cross-cutting feature branches benefit from specialized parallel review agents that each evaluate different dimensions of code quality"
symptoms:
  - "Large feature branch (3262 additions, 23 files) required thorough multi-dimensional review"
  - "Manual review of significant memory redesign, tool consolidation, and home directory system prone to missing edge cases"
  - "Need to validate security, performance, architecture consistency, patterns, agent behavior, implementation quality, and best practices simultaneously"
findings_summary:
  total: 18
  p1_critical: 3
  p2_important: 8
  p3_nice_to_have: 7
  agents_used: 7
  deduplication_rate: "3-4 agents independently flagged same critical issues"
related_artifacts:
  - commit: "abedc6b"
  - findings_commit: "1e160a1"
  - findings_range: "todos/038-055"
  - review_agents:
    - security-sentinel
    - performance-oracle
    - architecture-strategist
    - pattern-recognition-specialist
    - agent-native-reviewer
    - learnings-researcher
    - code-simplicity-reviewer
---

# Parallel 7-Agent Code Review Methodology

## Problem Statement

A large feature branch implementing Mika's home directory system, memory redesign, and tool consolidation (commit `abedc6b` — 23 files, +3,262/-411 lines) was submitted for code review. Manual sequential review of a branch this size is slow and prone to missing edge cases across security, performance, architecture, and code quality dimensions. A systematic parallel review methodology was needed.

## Solution: 7-Agent Parallel Review Architecture

### Agent Selection

Seven specialized agents were launched simultaneously, each analyzing the entire codebase from a different lens:

| Agent | Focus Area | Key Findings |
|-------|-----------|--------------|
| **security-sentinel** | Encryption, secrets, API key handling, data exposure | Plaintext metadata in memory_events (P2) |
| **performance-oracle** | Allocations, clones, O(n) operations, latency | send_message clone overhead (P2), HMAC key reconstruction (P3), prompt allocations (P3) |
| **architecture-strategist** | System design, layer boundaries, Phase 2 readiness | Stale onboarding flag (P2), missing update_commitment tool (P2), migrations without transactions (P2) |
| **pattern-recognition-specialist** | Consistency, duplication, naming conventions | Core memory defaults duplication (P2), test helper duplication (P2), decrypt pattern duplication (P3) |
| **agent-native-reviewer** | Agent loop semantics, tool behavior, memory interaction | Preference search bug (P1), events write-only (P1), no agent reset command (P2) |
| **learnings-researcher** | Past solutions, idiomatic patterns, conventions | Validated findings against prior code review docs |
| **code-simplicity-reviewer** | Unnecessary complexity, YAGNI violations, dead code | Heartbeat scaffolding YAGNI (P3), unused dependencies (P3) |

### Agent Selection Matrix for Different Project Types

| Project Type | Core Agents | Conditional Agents |
|-------------|-------------|-------------------|
| Rust (crypto/database) | security-sentinel, performance-oracle, architecture-strategist, pattern-recognition-specialist | agent-native-reviewer (if agent loop), data-integrity-guardian (if migrations) |
| Rails (web app) | kieran-rails-reviewer, security-sentinel, performance-oracle, architecture-strategist | dhh-rails-reviewer, julik-frontend-races-reviewer (if Stimulus/JS) |
| TypeScript | kieran-typescript-reviewer, security-sentinel, performance-oracle | pattern-recognition-specialist, code-simplicity-reviewer |
| Any PR with migrations | Always add: schema-drift-detector, data-migration-expert, deployment-verification-agent |

### Always Include (Regardless of Project Type)

- **agent-native-reviewer** — Verifies new features are agent-accessible
- **learnings-researcher** — Searches `docs/solutions/` for past issues related to modules and patterns
- **code-simplicity-reviewer** — Final pass for YAGNI violations and unnecessary complexity

## Execution Workflow

### Phase 1: Launch 7 Agents in Parallel

All agents analyze the same codebase concurrently. Each agent produces independent findings without influencing others.

```
┌─────────────────────────────────────────────────────────────┐
│                    Feature Branch                            │
│              (23 files, +3262/-411 lines)                    │
└──────────┬──────────┬──────────┬──────────┬─────────────────┘
           │          │          │          │
     ┌─────▼───┐ ┌───▼────┐ ┌──▼───┐ ┌───▼────┐  ... (7 total)
     │Security │ │ Perf   │ │Arch  │ │Pattern │
     │Sentinel │ │Oracle  │ │Strat │ │Recog   │
     └─────┬───┘ └───┬────┘ └──┬───┘ └───┬────┘
           │          │          │          │
           ▼          ▼          ▼          ▼
     ┌─────────────────────────────────────────┐
     │       Synthesis & Deduplication          │
     └─────────────────────────────────────────┘
```

### Phase 2: Collect and Extract Agent Outputs

Agent outputs are JSONL conversation logs. Extract final summaries using:

```python
# Extract final assistant text from JSONL output file
import json
with open(output_file) as f:
    for line in f:
        obj = json.loads(line)
        if obj.get('type') == 'result':
            content = obj['message']['content']
            for c in content:
                if isinstance(c, dict) and c['type'] == 'text':
                    print(c['text'])
```

**Fallback strategy:** If `TaskOutput` times out, read the agent's JSONL output file directly with `tail` and extract the final text blocks.

### Phase 3: Synthesize Findings

1. **Collect** all findings from all agents
2. **Deduplicate** overlapping findings (cross-agent consensus is signal, not noise)
3. **Prioritize** by severity and impact:
   - **P1 Critical** — Broken functionality, security vulnerabilities, data corruption risks
   - **P2 Important** — Phase 2 blockers, performance issues, architectural concerns
   - **P3 Nice-to-Have** — Optimizations, cleanup, consistency improvements
4. **Create structured todo files** with YAML frontmatter for each finding

### Phase 4: Create Todo Files in Parallel

Group findings by priority tier and launch 3 sub-agents (one per tier) to create structured todo files simultaneously:

```
P1 sub-agent → todos/038-040 (3 critical findings)
P2 sub-agent → todos/041-048 (8 important findings)
P3 sub-agent → todos/049-055 (7 nice-to-have findings)
```

## Results: 18 Findings Across 3 Priority Tiers

### P1 Critical (3 findings)

| ID | Finding | Reported By |
|----|---------|------------|
| 038 | Preference search uses exact-key HMAC lookup instead of substring scan | pattern-recognition, agent-native, architecture (3 agents) |
| 039 | Events can be stored but never retrieved or searched | agent-native, architecture (2 agents) |
| 040 | Home directory tests missing `#[serial]` annotation — env var race conditions | security-sentinel |

### P2 Important (8 findings)

| ID | Finding | Reported By |
|----|---------|------------|
| 041 | Plaintext metadata in memory_events (`target_key` stores "person:Alice Chen") | security-sentinel |
| 042 | `is_onboarding` computed once at startup, never refreshed in CLI loop | architecture-strategist |
| 043 | Agent can create commitments but cannot mark them completed/cancelled | agent-native-reviewer |
| 044 | `send_message` clones entire `MessagesRequest` each iteration | performance-oracle |
| 045 | Test helpers (`test_key`/`test_db`/`test_ctx`) duplicated across 4 modules (~80 LOC) | pattern-recognition |
| 046 | Core memory section names appear in 5 locations, defaults in 3 locations | pattern-recognition |
| 047 | Migrations v1/v2/v3 lack explicit transaction wrapping | architecture-strategist |
| 048 | CLI `/reset` command has no agent-accessible equivalent | agent-native-reviewer |

### P3 Nice-to-Have (7 findings)

| ID | Finding | Reported By |
|----|---------|------------|
| 049 | Heartbeat scaffolding is YAGNI (no-op tool) | code-simplicity |
| 050 | Unused dependencies in Cargo.toml | code-simplicity |
| 051 | Unnecessary string allocations in prompt assembly (`format!` vs `write!`) | performance-oracle |
| 052 | HMAC key reconstructed on every call (should cache in `EncryptionKey`) | performance-oracle |
| 053 | `db_path` uses `to_string_lossy` which can silently corrupt paths | security-sentinel |
| 054 | Decrypt-or-skip pattern duplicated ~150 lines in db.rs | pattern-recognition |
| 055 | `due_date` parameter name incorrect for events (should be `event_date`) | pattern-recognition |

## Key Observations

### Cross-Agent Consensus as Signal

When multiple agents independently flag the same issue, it's a strong indicator of significance:

- **Preference search bug (#038)**: Flagged by 3-4 agents (pattern-recognition, agent-native, architecture-strategist). Each found it from a different angle — naming inconsistency, broken agent capability, and architectural mismatch.
- **Events write-only (#039)**: Found by 2 agents who noticed the store-but-never-retrieve gap from capability and architecture perspectives.

### Agent-Native Reviewer Catches Unique Issues

The agent-native-reviewer found capability gaps (missing update tool, no search for events, no agent reset) that no other agent would catch. It evaluates the system from the LLM's perspective — "if I were the agent, what can't I do?"

### Learnings Researcher Validates Patterns

The learnings-researcher surfaced relevant patterns from prior code reviews (Python v1 24-finding review, Rust v2 13-finding resolution), confirming that:
- The file-conflict-aware parallel resolution strategy from the previous review should be applied when resolving these 18 findings
- Prevention checklists from prior reviews caught some but not all of the new issues

## Prevention Strategies

### For Running Parallel Code Reviews

1. **Select agents based on project type** — Use the agent selection matrix above. Don't run Rails reviewers on Rust code.
2. **Always include meta-agents** — agent-native-reviewer, learnings-researcher, and code-simplicity-reviewer catch issues domain experts miss.
3. **Plan for JSONL extraction** — Agent outputs are conversation logs, not clean reports. Have extraction tooling ready.
4. **Handle timeouts gracefully** — TaskOutput can timeout while agents are still running. Fall back to reading output files directly.
5. **Create todos in parallel by tier** — Three sub-agents (P1/P2/P3) creating structured todo files simultaneously is faster than sequential creation.

### For Resolving Findings

Use the file-conflict-aware parallel resolution strategy documented in `parallel-agent-code-review-resolution.md`:

| Condition | Strategy |
|-----------|----------|
| Findings touch different files | Run in parallel |
| Findings touch same file but different sections | Combine into one agent |
| Finding touches many files (cleanup/refactor) | Run last, alone |
| Finding requires architectural change not yet needed | Defer with documented rationale |

### Deduplication Strategy

1. Build a matrix of finding-to-agent mappings
2. Findings reported by 3+ agents get auto-promoted to P1 (unless purely cosmetic)
3. Merge overlapping findings into a single todo with all reporters listed
4. Keep the most specific description and merge proposed solutions

### Structured Todo Template

Each finding becomes a markdown file with:
- YAML frontmatter: `status`, `priority`, `issue_id`, `tags`, `dependencies`
- Problem Statement with location and reporter
- Proposed Solutions with effort estimates
- Acceptance Criteria checklist
- Work Log for tracking progress

## Artifact Locations

- **Feature branch commit:** `abedc6b` (23 files, +3,262/-411 lines)
- **Review findings commit:** `1e160a1` (18 todo files)
- **Findings files:** `todos/038-055`
- **Feature code:** `crates/mika-agent/src/` and `crates/mika-common/src/`

## Related Documentation

- `docs/solutions/code-review/multi-agent-mvp-code-review.md` — Original 7-agent review of Python v1 (24 findings)
- `docs/solutions/code-review-workflow/parallel-agent-code-review-resolution.md` — How to resolve findings using file-conflict-aware parallel execution (13 findings, 3 rounds)
- `docs/plans/2026-02-23-feat-mika-home-directory-agent-core-plan.md` — Implementation plan for the reviewed feature

## Lessons Learned

1. **7 agents is the sweet spot for comprehensive Rust reviews.** Security, performance, architecture, patterns, agent-native, learnings, and simplicity cover all major quality dimensions without excessive overlap.

2. **Cross-agent consensus is the strongest prioritization signal.** When 3+ agents independently find the same issue from different angles, it's almost certainly a real problem worth fixing immediately.

3. **The agent-native-reviewer is irreplaceable for agent systems.** No other agent thinks "what can the LLM not do?" — it found 4 of the 18 findings including 2 P1 critical issues.

4. **Structured todo output enables systematic remediation.** YAML frontmatter + markdown templates make findings searchable, triageable, and trackable across sessions.

5. **Parallel todo creation (3 sub-agents by tier) cuts synthesis time significantly.** Instead of creating 18 files sequentially, 3 sub-agents working on their tier's findings finish in roughly the time of creating 7 files.

6. **The learnings-researcher compounds previous review knowledge.** It found relevant patterns from prior code reviews that validated current findings and provided resolution strategies.
