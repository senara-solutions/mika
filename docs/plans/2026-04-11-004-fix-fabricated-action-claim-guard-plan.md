# Fix: Fabricated Action-Claim Guard (#308)

## Problem

mika-qa fabricated a PR comment URL without executing any tool call. The agent produced "Comment posted: https://github.com/senara-solutions/mika/pull/307#issuecomment-4146200192" with zero tool calls in the turn. The URL returns HTTP 404 — the comment never existed.

The agent had `run_gh` available (all builtins are always registered) but chose to hallucinate instead of calling it. The qa-review skill (external marketplace skill) does not declare `required_tools = ["run_gh"]`, so the existing required-tools gate did not fire.

## Root Cause

The agent loop has three EndTurn post-conditions:
1. **Text-based tool call detection** — re-prompts when LLM outputs XML tool calls as text
2. **Required-tools gate** — re-prompts when declared `required_tools` weren't called
3. **Completion-claim guard** — re-prompts when agent claims "merged"/"deployed" without updating work items

None of these catch the case where the agent claims to have performed an action (posted, commented, created) and includes a fabricated URL, but made zero tool calls. This is a general class of fabrication that can occur with any skill, not just qa-review.

## Solution

Add a **4th post-condition: fabricated action-claim guard** in the EndTurn chain. This guard detects when:

1. The assistant's response contains a **GitHub URL** that looks like a created resource (e.g., `#issuecomment-<id>`, `#discussion_r<id>`, `/issues/<n>`, `/pull/<n>`)
2. AND the response contains an **action-claim verb** (e.g., "posted", "commented", "created", "submitted", "opened", "reviewed")
3. AND **zero tool calls** were made in the current turn (`tools_called.is_empty()`)

When all three conditions are met, reject the response and re-prompt once (same pattern as the other guards).

### Design Decisions

- **Zero-tool-call gate, not per-URL tracking:** We don't track which tool produced which URL. Instead, the simpler heuristic is: if the agent claims an action with a URL but never called any tool, it's fabricated. If it called at least one tool, we trust the tool output (the grounding rule in the system prompt covers residual risk).
- **GitHub URLs only (for now):** Scoping to GitHub URLs minimizes false positives. Can be extended later.
- **Order in the chain:** Fires after completion-claim guard (4th position). All four guards are mutually exclusive per turn (each sets a `_retry_done` flag).
- **Applies to all modes:** Conversation and silent — fabrication is equally harmful in both.

## Implementation

### File: `crates/mika-agent/src/agent.rs`

1. **Add regex** for GitHub resource URLs:
   ```rust
   static GITHUB_RESOURCE_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
       Regex::new(r"https://github\.com/[^\s)>\]]+(?:#issuecomment-\d+|#discussion_r\d+|#pullrequestreview-\d+|/(?:issues|pull)/\d+)")
           .expect("github resource url regex must compile")
   });
   ```

2. **Add regex** for action-claim verbs:
   ```rust
   static ACTION_CLAIM_RE: LazyLock<Regex> = LazyLock::new(|| {
       Regex::new(r"(?i)\b(posted|commented|created|submitted|opened|reviewed|published|left a (?:comment|review))\b")
           .expect("action claim regex must compile")
   });
   ```

3. **Add detection function:**
   ```rust
   fn detect_fabricated_action_claim(text: &str) -> Option<(&str, &str)> {
       // Fast path: must contain github.com
       if !text.contains("github.com/") {
           return None;
       }
       let url_match = GITHUB_RESOURCE_URL_RE.find(text)?;
       let verb_match = ACTION_CLAIM_RE.find(text)?;
       Some((verb_match.as_str(), url_match.as_str()))
   }
   ```

4. **Add post-condition** in `run_loop()` after the completion-claim guard:
   ```rust
   // Fabricated action-claim guard: if the agent claims to have performed
   // an action (posted, commented, etc.) with a GitHub URL but made zero
   // tool calls, reject and re-prompt. See #308.
   if matches!(response.stop_reason, LlmStopReason::EndTurn)
       && !fabricated_action_retry_done
       && tools_called.is_empty()
   {
       if let Some((verb, url)) = detect_fabricated_action_claim(&text) {
           fabricated_action_retry_done = true;
           // ... push assistant response, inject correction, continue
       }
   }
   ```

5. **Add `fabricated_action_retry_done` flag** alongside the other retry flags.

### File: `crates/mika-agent/src/prompt.rs`

6. **Strengthen the grounding rule** with an explicit URL fabrication example:
   ```
   BAD: No tool calls → you say "Comment posted: https://github.com/…#issuecomment-123"
   GOOD: Call run_gh to post the comment → report the URL from the tool result
   ```

### Tests

7. **Unit tests** for `detect_fabricated_action_claim`:
   - Detects "Comment posted: https://github.com/org/repo/pull/307#issuecomment-123"
   - Detects "I've reviewed the PR: https://github.com/org/repo/pull/42#pullrequestreview-99"
   - Returns None for plain GitHub repo URLs without resource anchors
   - Returns None when no action verb is present
   - Returns None for discussion text that mentions URLs without claiming action

8. **Integration test** (eval harness): agent loop with zero tool calls and fabricated URL → guard fires and re-prompts.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | Add guard logic, regex, detection function, retry flag |
| `crates/mika-agent/src/prompt.rs` | Add URL fabrication BAD/GOOD example to grounding rule |

## Testing

- `cargo test -p mika-agent` — unit tests for detection function
- `cargo test -p mika-agent --test eval` — eval harness integration test
- Manual: trigger mika-qa with a PR review request, verify it calls `run_gh` or gets re-prompted
