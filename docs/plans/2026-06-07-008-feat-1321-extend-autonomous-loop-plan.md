# Plan — feat(webhook): extend autonomous-loop coverage to sister repos (mika#1321)

## Phase 0 — Pin

**A. Gateway webhook handler** (`crates/mika-gateway/src/github.rs:574 handle_github_webhook`):
- Validates X-Hub-Signature-256
- Parses event payload as `GitHubWebhookEvent`
- Routes via `route_event(event_type, action, check_conclusion)` — **no repo-name filter**
- Returns 200 even if not routable (drops, no error)

**B. Ready-label skill prompt** (`mika/skills/bundled/self-dev-webhook-ready-label/system_prompt.md`):
```
When the message starts with `[GitHub] Issue labeled ready on <repo>#<n>`...
1. Call run_gh("issue edit <n> --remove-label ready") with repo: "<repo>"
2. Call run_gh with args "issue view <n> --json title,body --repo <repo>"
4. Call create_task with reference_url: "https://github.com/<repo>/issues/<n>"
```
**`<repo>` is a placeholder; the LLM extracts it from the message.** Not hardcoded to mika.

**C. webhook-message format** (`webhook_dispatch.rs:65-115`):
- Test fixtures all use `senara-solutions/mika#N` but the matcher/parser are repo-agnostic.

**D. Webhook installation state**:
```
$ for r in claude-pilot-py mika-cloud mika-skills wizzard; do
    echo "$r: $(gh api repos/senara-solutions/$r/hooks --jq 'length')"
  done
→ claude-pilot-py: 0
→ mika-cloud: 0
→ mika-skills: 0
→ wizzard: 0
```
**0 hooks on all 4 sister repos.** This is the only confirmed structural gate.

## Hypothesis (committed)

**The only confirmed gate is webhook installation. The ticket body's Layer 2/3/4 (gateway allowlist, skill prompt, classifier scope) all appear cross-repo-capable already.** Code reading shows:
- Gateway routes by event_type, not repo
- Skill prompt uses `<repo>` placeholder
- Tools (`run_gh`, `run_claude_pilot`) accept `repo:` arg

Layer 1 (operator-action webhook install) closes the gap.

But the bug-author was conservatively listing layers — there COULD be a hidden hardcoded mika-only check I missed in my code-reading. The phased rollout below verifies hypothesis empirically before declaring all 4 sister repos covered.

## Approach (committed)

**Phased rollout, smallest-blast-radius first.**

### Phase A — install webhook on claude-pilot-py
- Operator action via `gh api repos/senara-solutions/claude-pilot-py/hooks --method POST` with the canonical webhook config (events: `issues`, `pull_request`, `check_suite`, `pull_request_review`)
- URL: same gateway endpoint mika uses (`https://<gateway>/github/webhook`)
- Secret: `MIKA_WEBHOOK_SECRET` env value

### Phase B — label-test on smallest safe ticket
- Pick claude-pilot-py#7 or #12 (small, well-scoped, the tickets the ticket body names)
- Apply `ready` label
- Watch mika-dev's audit_events for dispatch attempt
- Verify: PR opens on `senara-solutions/claude-pilot-py`, NOT on mika

### Phase C — if Phase B succeeds, expand
- Install webhooks on mika-cloud, mika-skills, wizzard (same shape)
- Phase B label-test on each (one ticket each, observe)
- Document the rollout in handsoff

### Phase D — regression check on mika
- Confirm mika repo's autonomous loop continues to work
- Apply `ready` to a safe mika ticket post-rollout; verify dispatch behaves identically

### Phase E — close-out
- If all 4 sister repos verified cross-repo-capable end-to-end, close mika#1321
- If any sister repo fails, file a sub-issue with the specific failure (e.g., "claude-pilot-py dispatch creates PR on wrong repo") and keep #1321 open

## Acceptance Criteria

1. **AC1:** Webhooks installed on claude-pilot-py, wizzard, mika-cloud, mika-skills (operator action, verified by `gh api repos/senara-solutions/<r>/hooks --jq 'length' > 0`).

2. **AC2:** Label-test on claude-pilot-py: applying `ready` to claude-pilot-py#7 triggers mika-dev's `self-dev-webhook-ready-label` dispatch chain. Audit-event shows `run_claude_pilot_groom` or `run_claude_pilot` called with `repo: "claude-pilot-py"` in the input_summary.

3. **AC3:** PR from the Phase B test opens on `senara-solutions/claude-pilot-py`, not on `senara-solutions/mika`.

4. **AC4:** mika repo regression check passes — applying `ready` to a mika ticket post-rollout produces identical dispatch behavior to pre-rollout.

5. **AC5:** Handsoff entry documenting the rollout outcome (which repos verified end-to-end, which (if any) failed).

## Files

- **No mika code changes anticipated.** Phase 0 investigation shows the substrate is cross-repo-capable.
- Operator-action only (webhook installation via `gh api`).
- Documentation: `mika-platform/docs/operator/<filename>.md` — runbook for adding new sister repos to the autonomous loop (template for future repos).

## What this plan does NOT do

- **Does not write mika code.** If Phase B reveals an actual hidden mika-only gate, that's a separate sub-issue with its own plan.
- **Does not change CLAUDE.md repo scope.** Already lists sister repos as Read-write.
- **Does not change branch-derive scripts.** Per Phase 0, no evidence they're mika-only.
- **Does not address wizzard's closed-source posture.** Wizzard work via the loop is structural-only (visible diffs); closed-source-content remains operator-only.

## Risk

Medium-low.
- Webhooks could fire unexpected events that mika-dev handles poorly. Mitigated by: events are filtered server-side already (`route_event`); unhandled events return 200 and drop.
- Loop could open PR on wrong repo if dispatch tools assume mika. Mitigated by: skill prompt uses `<repo>` placeholder; tools accept `repo:` arg. Empirical Phase B verification.
- Wizzard's secret-content exposure if a webhook surfaces sensitive issue body to mika-dev. Mitigated by: wizzard work scope-restricted; closed-source content stays in `wizzard/CLAUDE.md`.

## Test plan

1. Phase A: verify webhook installed (gh api).
2. Phase B: label-test, observe end-to-end. Capture audit_events + final PR location.
3. Phase D: regression on mika.
4. Document outcome in handsoff.

## Implementation order

1. Operator runs webhook installation script on claude-pilot-py.
2. Operator (or orchestrator-CC) labels claude-pilot-py#7 with `ready`.
3. Observe loop end-to-end. If PR opens correctly on claude-pilot-py: declare Phase B success.
4. If Phase B fails: capture failure mode, file sub-issue, keep mika#1321 open.
5. If Phase B succeeds: expand to mika-cloud, mika-skills, wizzard.
6. Phase D regression.
7. Close mika#1321 with handsoff documentation.

## Operator-action dependency named

**This ticket cannot be implemented by autonomous loop / Carlos alone.** Webhook installation requires admin permission on the sister repos. Operator (Vincent) is the only one who can run the install commands OR delegate the `admin:org_hook` scope to a machine user.

This is a NAMED operator-asks finding, classification matches #1382 (operator-host-blocked).
