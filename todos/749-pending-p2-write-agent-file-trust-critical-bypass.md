---
status: pending
priority: p2
issue_id: 749
tags: [code-review, security, skills]
dependencies: []
---

# write_agent_file can bypass trust-critical skill guard

## Problem Statement

The `review_skill` handler blocks reviewing trust-critical skills (skill-review, self-knowledge, agents-teams) at the code level. However, `write_agent_file` has no awareness of trust-critical skill directories and can write directly to `skills/<trust-critical-skill>/generated/*/system_prompt.md`, bypassing the review guard entirely.

The only protection is a prompt-level instruction: "Do not use `write_agent_file` to persist variants." This is insufficient against prompt injection attacks via malicious callback results or conversation context manipulation.

## Findings

- Security review identified this as a defense-in-depth gap
- `write_agent_file` performs path traversal validation but has no skill directory awareness
- The existing `confirm: true` overwrite flow adds friction but doesn't prevent a determined attack
- Known residual risk documented in `docs/solutions/security-issues/review-skill-builtin-trust-boundary.md`

## Proposed Solutions

### Option A: Path guard in write_agent_file
Add a check that rejects writes targeting trust-critical skill directories:
```rust
if let Ok(rel) = full_path.strip_prefix(base_dir.join("skills")) {
    if let Some(skill_name) = rel.components().next() {
        if is_trust_critical_skill(&skill_name.as_os_str().to_string_lossy()) {
            return ToolOutput::error("Cannot write to trust-critical skill directories.");
        }
    }
}
```
- **Pros:** Code-enforced, immune to prompt injection
- **Cons:** Adds dependency from write_agent_file to bundled_skills module
- **Effort:** Small
- **Risk:** Low

### Option B: Accept as residual risk
Document the gap and rely on prompt-level protection.
- **Pros:** No code changes
- **Cons:** Prompt injection could bypass
- **Effort:** None
- **Risk:** Medium

## Recommended Action

Option A — small, surgical fix that closes the defense-in-depth gap.

## Technical Details

- **Affected files:** `crates/mika-agent/src/tools/write_agent_file.rs`
- **Components:** write_agent_file tool, bundled_skills module

## Acceptance Criteria

- [ ] `write_agent_file` rejects writes to `skills/<trust-critical-skill>/**`
- [ ] Test: writing to `skills/skill-review/generated/foo/system_prompt.md` returns error
- [ ] Test: writing to `skills/web-search/generated/foo/system_prompt.md` succeeds

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-04-09 | Created from security review of #499 | Prompt-only guards insufficient for trust boundaries |

## Resources

- PR: #499 (umbrella fix)
- Security review finding S1
- Related: `docs/solutions/security-issues/review-skill-builtin-trust-boundary.md`
