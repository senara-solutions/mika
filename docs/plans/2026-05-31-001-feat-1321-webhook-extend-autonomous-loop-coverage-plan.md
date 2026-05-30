# Plan: feat(webhook): extend autonomous-loop coverage beyond mika repo

**Ticket:** mika#1321
**Type:** enhancement
**Priority:** p1-important
**Labels:** agent-core, infrastructure

## Problem

The autonomous dev loop only operates on `senara-solutions/mika`. The other four repos — `claude-pilot-py`, `wizzard`, `mika-cloud`, `mika-skills` — have zero webhook coverage. Applying the `ready` label on those repos is inert: no webhook fires, no dispatch occurs, no PR opens.

**Root cause layers:**

1. **No webhooks installed** on the four repos — `gh api repos/senara-solutions/<repo>/hooks` returns `[]`.
2. **Gateway routing** uses `github_repos` Postgres table for multi-tenant lookup, falling back to `agent_base_url` for single-tenant mode. In single-tenant mode (current dev setup), the fallback routes all repos to the same mika-dev agent — so the gateway itself is already multi-repo ready. In multi-tenant mode, each repo needs a `github_repos` row.
3. **dispatch-lib.sh** hardcodes `senara-solutions/` as the GitHub org prefix in 8 places (lines 84, 215, 216, 237, 679, 1253, 1295, 1665). This works today because all repos are in the same org, but it's a maintenance smell.
4. **Skill prompts** (self-dev, self-dev-callback) hardcode `senara-solutions/` in example URLs and `run_gh` templates. The LLM uses these as patterns.
5. **wizzard** repo lacks `.claude/claude-pilot.json` and `.claude/commands/mika.md` — claude-pilot cannot operate there.
6. **Branch-name and worktree-path derivation** (`scripts/derive-branch-name`, `scripts/derive-worktree-path`) are fully parameterized — no changes needed.
7. **Agent identities** (well_known_agents.rs) have no repo-scoping — no changes needed.
8. **Ready-label webhook skill** extracts repo from the webhook payload dynamically — no changes needed for routing logic.

## Approach

Three phases: operational setup (webhook install + wizzard bootstrap), code hardening (org-prefix parameterization + prompt updates), and verification.

### Phase 1: Operational setup (no code changes)

**Step 1.1 — Install GitHub webhooks on the four repos.**

Use the GitHub App's webhook configuration (preferred) or per-repo webhook installation:

```bash
for REPO in claude-pilot-py wizzard mika-cloud mika-skills; do
  gh api repos/senara-solutions/$REPO/hooks --method POST \
    --field name=web \
    --field active=true \
    --field events='["issues","pull_request","check_suite","pull_request_review"]' \
    --field 'config[url]=<GATEWAY_WEBHOOK_URL>/webhook/github' \
    --field 'config[content_type]=json' \
    --field 'config[secret]=<MIKA_GITHUB_WEBHOOK_SECRET>'
done
```

**Decision:** Use the same webhook secret as the mika repo. The gateway validates HMAC-SHA256 with a single `MIKA_GITHUB_WEBHOOK_SECRET` — all repos must use the same secret unless the gateway is modified to support per-repo secrets.

**Prerequisite check:** Verify the GitHub App installation covers all four repos (Settings → Installations → Repository access). If the App is configured for "Only select repositories", add the four repos there. If webhooks are delivered via the GitHub App (not per-repo hooks), this step may be App-level configuration only.

**Step 1.2 — Bootstrap wizzard for claude-pilot.**

Create the minimum files for claude-pilot to operate in the wizzard repo:

- `wizzard/.claude/claude-pilot.json` — copy from `mika/.claude/claude-pilot.json` and adjust paths.
- `wizzard/.claude/commands/mika.md` — create a minimal `/mika` command appropriate for wizzard's tech stack (Python training pipeline).
- `wizzard/docs/plans/` — create the directory so grooming can write plan files.

**Step 1.3 — Register repos in gateway (multi-tenant only).**

For single-tenant mode (dev): no action needed — `agent_base_url` fallback handles all repos.

For multi-tenant/production: insert rows into `github_repos` for each new repo:

```sql
INSERT INTO github_repos (id, repo_full_name, customer_id, agent_mapping)
VALUES
  (gen_random_uuid(), 'senara-solutions/claude-pilot-py', '<customer_id>', '{}'),
  (gen_random_uuid(), 'senara-solutions/wizzard', '<customer_id>', '{}'),
  (gen_random_uuid(), 'senara-solutions/mika-cloud', '<customer_id>', '{}'),
  (gen_random_uuid(), 'senara-solutions/mika-skills', '<customer_id>', '{}');
```

### Phase 2: Code hardening

**Step 2.1 — Extract org prefix in dispatch-lib.sh.**

Currently `senara-solutions` is hardcoded in 8 places. Extract to a variable derived from the repo's git remote:

```bash
# At the top of _set_up_worktree or in the init section:
GH_ORG=$(git -C "$SUB_REPO_DIR" remote get-url origin 2>/dev/null \
  | sed -n 's|.*github\.com[:/]\([^/]*\)/.*|\1|p')
GH_ORG="${GH_ORG:-senara-solutions}"  # fallback
```

Then replace all `"senara-solutions/$REPO"` with `"$GH_ORG/$REPO"`.

**Files changed:** `skills/bundled/_shared/dispatch-lib.sh`

**Lines to update:**
- Line 84: `gh pr list --repo "$GH_ORG/$REPO"`
- Line 215: `gh issue view "$ISSUE_NUM" --repo "$GH_ORG/$REPO"`
- Line 216: error message
- Line 237: auto-skip JSON
- Line 679: `gh pr list --repo "$GH_ORG/$REPO"`
- Line 1253: `gh issue view` in `_write_canonical_callout`
- Line 1295: `gh issue edit` in `_write_canonical_callout`
- Line 1665: any remaining hardcoded reference

**Risk:** Low. The `GH_ORG` extraction is a best-effort enhancement — the fallback to `senara-solutions` preserves current behavior. The `sed` pattern handles both HTTPS (`github.com/org/repo`) and SSH (`github.com:org/repo`) remote URLs.

**Step 2.2 — Update skill prompt examples to use parameterized org.**

Update prompt examples that hardcode `senara-solutions` to use `<org>` or extract the org from the reference URL.

**Files changed:**
- `skills/bundled/self-dev/system_prompt.md` — Update example `run_gh` calls and GraphQL queries to derive org from `reference_url` rather than hardcoding. Add explicit instruction: "Extract the organization from the task's `reference_url` (format: `https://github.com/ORG/REPO/...`). Do not hardcode the organization name."
- `skills/bundled/self-dev-callback/system_prompt.md` — Same treatment for `run_gh` examples (lines 17, 20, 25, 26, 94, 95, 96, 110, 111, 112).

**Risk:** Medium. Prompt changes affect LLM behavior indirectly. The risk is that the LLM may occasionally fail to extract the org correctly from URLs. Mitigated by keeping `senara-solutions` in examples as the _example value_ while adding the extraction instruction.

**Step 2.3 — Verify permission classifier accepts all repos.**

Check `crates/mika-agent/src/server/permission_pre_classifier.rs` for any repo-scoped allowlist. Based on research, the permission classifier operates on tool names and input patterns, not repo names — no changes expected. Verify and document.

**Files to check:** `crates/mika-agent/src/server/permission_pre_classifier.rs`

### Phase 3: Verification

**Step 3.1 — Smoke test: webhook delivery.**

After webhook installation, trigger a test event:
1. Create a test issue on `claude-pilot-py` with the `ready` label.
2. Verify the gateway receives the webhook (check gateway logs for `issues.labeled` event from `senara-solutions/claude-pilot-py`).
3. Verify mika-dev receives the message with correct repo name.
4. Verify the ready-label handler removes the label and dispatches dev-groom/dev-pilot.

**Step 3.2 — End-to-end: dispatch a real ticket.**

Apply `ready` to `claude-pilot-py#7` or `claude-pilot-py#12` and verify:
1. Auto-groom fires (no Plan callout → dev-groom dispatched).
2. Worktree created at `.claude/worktrees/<branch>/claude-pilot-py/`.
3. Plan committed on the branch in `claude-pilot-py/docs/plans/`.
4. Architect review runs.
5. Dev-pilot dispatches (after grooming).
6. PR opens on `senara-solutions/claude-pilot-py` (not on `senara-solutions/mika`).

**Step 3.3 — Regression: mika repo dispatch still works.**

Dispatch a known-good mika ticket to verify no regressions.

## File changes summary

| File | Change type | Description |
|------|-------------|-------------|
| `skills/bundled/_shared/dispatch-lib.sh` | Modify | Extract `GH_ORG` from git remote, replace 8 hardcoded `senara-solutions` references |
| `skills/bundled/self-dev/system_prompt.md` | Modify | Add org-extraction instruction, update example URLs |
| `skills/bundled/self-dev-callback/system_prompt.md` | Modify | Update hardcoded org references in `run_gh` examples |

## Operational changes (no code)

| Action | Target | Description |
|--------|--------|-------------|
| Install webhooks | claude-pilot-py, wizzard, mika-cloud, mika-skills | `issues`, `pull_request`, `check_suite`, `pull_request_review` events |
| Bootstrap claude-pilot config | wizzard | `.claude/claude-pilot.json`, `.claude/commands/mika.md`, `docs/plans/` |
| Register in gateway DB | All four repos (multi-tenant only) | `github_repos` table rows |

## Out of scope

- **mika-platform autonomous-loop coverage** — meta-repo dispatchers are self-targeting; different semantics.
- **Cross-repo CI integration** — merging companion PRs in lockstep is a milestone-level concern.
- **wizzard closed-source posture** — the autonomous loop handles structural changes; content discipline is separate.
- **Per-repo webhook secrets** — all repos share `MIKA_GITHUB_WEBHOOK_SECRET`; per-repo support is unnecessary.

## Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| LLM misextracts org from reference_url | Low | Medium | Keep `senara-solutions` as example value in prompts; fallback in dispatch-lib |
| Webhook secret mismatch | Low | High | Use same secret as mika repo; verify with `gh api` after install |
| wizzard /mika command inadequate | Medium | Low | Start with minimal stub; iterate after first dispatch |
| Gateway drops events for unregistered repos (multi-tenant) | Low | Medium | Single-tenant fallback covers dev; register before prod deploy |

## Acceptance criteria (from ticket)

1. ✅ Webhooks installed on `claude-pilot-py`, `wizzard`, `mika-cloud`, `mika-skills`.
2. ✅ Gateway accepts `issues.labeled` events from those repos without rejecting.
3. ✅ `self-dev-webhook-ready-label` dispatches against the correct repo per webhook payload.
4. ✅ Applying `ready` label to `claude-pilot-py#7`/`#12` or `wizzard#1` triggers the dispatch chain.
5. ✅ PR opens on the correct repo, not on mika.
6. ✅ Regression test: dispatch a no-op ticket on `claude-pilot-py` end-to-end.
