---
status: ready
priority: p3
issue_id: "149"
tags: [plan-review, simplicity, security]
dependencies: []
---

# Consolidate error messages from 5 to 3 categories

## Problem Statement
The plan specifies 5 different user-facing error messages that leak internal state: "not paired", "container unavailable", "message too long", "try again later", "something went wrong". Distinct error messages help attackers enumerate system state (is account paired? is container running?).

**Why it matters:** Fewer, more generic error messages improve both security (less enumeration) and simplicity (fewer code paths).

## Findings
- Source: Code Simplicity Reviewer, Security Sentinel (L-3)
- Current plan: 5 distinct messages revealing internal state
- Proposed: 3 generic categories

## Proposed Solutions

### Option 1: Three generic error categories (Recommended)
1. "I'm having trouble right now. Please try again in a moment." (all transient errors)
2. "Please pair your account first. Visit [link]." (not paired — this is user-actionable)
3. Silent drop + log (internal errors the user can't act on)
- **Pros**: Less information leakage, simpler code, better UX
- **Cons**: Harder to debug from user reports (but logs have details)
- **Effort**: Small
- **Risk**: Low

## Acceptance Criteria
- [ ] User-facing error messages reduced to 3 or fewer categories
- [ ] Error messages don't reveal container status or internal architecture
- [ ] Detailed errors logged server-side for debugging

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Code Simplicity Reviewer and Security Sentinel aligned on fewer, safer error messages
