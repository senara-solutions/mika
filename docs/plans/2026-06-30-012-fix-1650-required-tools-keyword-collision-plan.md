---
issue: 1650
type: fix
date: 2026-06-30
---

# Plan — fix(skills): required-tools-gate keyword substring-collision (mika#1650)

## Problem

`skills/bundled/gh-read-only/skill.toml` declares bare common-English bigrams as keywords: `"issue"`, `"issues"`, `"pr"`, `"github"`, `"gh"`, `"repo"`, `"repository"`. The matcher (`crates/mika-agent/src/skills/matcher.rs:50`) uses lowercase `message_lower.contains(kw)` with no word boundaries. Substring collisions with ordinary English vocabulary:

- `"pr"` matches: approach, approve, appropriate, press, process, problem, project, properly, propose, provide, prevent...
- `"gh"` matches: thought, high, weight, right, through, fight, light, neighbor, eight, although...
- `"issue"` matches: tissue, issuer, reissue
- `"repo"` matches: report, reporter, repository (legitimate), depository, repose, repossession

Any prose with one of these substrings forces `required_tools = ["gh_read"]` to fire, regardless of user intent to fetch GitHub data.

**Hard evidence (2026-06-29 ~12 UTC):** mika-litha (newly-provisioned local CEO of odds-engine team, zero GitHub work in her domain) hit this gate while writing analytical prose. She mentioned "issue" and "pr" in meta-discussion about other agents' behavior, the gate fired, GLM-5.2 misinterpreted the `[mika-engine]` re-prompt as a prompt-injection attack and confabulated a security incident.

## Architectural lineage

- mika#463 — required-tools gate match-reason conditioning (Keyword vs AlwaysOn vs Dependency). The keyword-driven path is the one affected.
- mika#1645 (CLOSED today) — sibling structural-grounding fix (qa equivalence-claim grounding).
- mika#1646 (groomed) — sibling structural-grounding fix (mika-dev destructive-action re-execute).

## Audit findings (codebase grounded)

Other skills declaring `required_tools = ["gh_read"]` (audit class):
- `skills/bundled/mika-arch-groom-ticket/skill.toml` — keywords: `groom-ticket`, `review-plan`, `architect-review`, `arch-review`, `first-review`, `plan review`, `architecture review`. All multi-word or hyphenated, no bare-bigram collision.
- `skills/bundled/mika-arch-groom-milestone/skill.toml` — keywords: `groom-milestone`, `milestone-review`, `milestone-groom`. Hyphenated, no collision.
- `skills/bundled/mika-arch-second-review/skill.toml` — keywords: `second-review`, `groom-iteration`, `iterate-review`, `second pass`, `follow-up review`. Multi-word/hyphenated, no collision.

**Audit conclusion:** only `gh-read-only` exhibits the bare-bigram pattern. The three mika-arch skills are safe as-is. AC2's audit step results in zero additional edits.

## Fix shape

Single-file edit to `skills/bundled/gh-read-only/skill.toml`. Replace the bare common-bigram keywords with multi-word **intent phrases** that signal user-intent-to-fetch-GitHub-data, not just "the topic was mentioned":

```toml
[triggers]
keywords = [
    "github issue", "github pr", "github pull request",
    "pull request", "pull requests",
    "view issue", "view pr", "view pull request",
    "check issue", "check pr", "check pull request",
    "list issues", "list prs", "list pull requests",
    "open issues", "open prs", "open pull requests",
    "pr diff", "pr body", "issue body", "issue list",
    "fetch issue", "fetch pr",
    "merge pr", "close pr", "close issue",
    "issue #", "pr #",
]
```

Design rules applied:
1. **No bare single common English words.** Every keyword is either multi-word OR a domain-specific phrase that doesn't substring-collide with English vocabulary.
2. **Keep `"pull request"`/`"pull requests"`.** Multi-word, no English collision.
3. **Drop `"github"`, `"repo"`, `"repository"` as bare keywords.** Discussion-noisy. Use compound phrases (`"github issue"`, `"github pr"`).
4. **Drop `"issue"`, `"issues"`, `"pr"`, `"gh"` as bare keywords.** Replace with intent-phrase variants.
5. **Preserve coverage:** the kept set catches genuine fetch-intent: `"view issue #1645"`, `"list open PRs"`, `"check pr 1644"`, `"merge pr 1678"`, `"pr diff"`.

## Implementation outline

1. **Read current `skills/bundled/gh-read-only/skill.toml`** to confirm baseline. (Done in pre-groom; pre-fix keywords listed in §Problem.)

2. **Replace `keywords = [...]` block** with the new intent-phrase set above. Single-block replacement, no other changes to the file.

3. **Audit verification:** read all 3 mika-arch skill manifests (`mika-arch-groom-ticket`, `mika-arch-groom-milestone`, `mika-arch-second-review`) and confirm — in the PR body — that they don't exhibit the bare-bigram pattern. Audit result: zero edits required for those three. Audit documented in PR body for AC2 evidence.

4. **Regression test (AC3):** add a test exercising the matcher on a Litha-style prose sample (`"the issue with that pr approach is..."`) against the fixed gh-read-only manifest. Gate must NOT fire (no keyword match → no required_tools constraint → no engine retry).

5. **Coverage test (AC4):** add a test with `"view issue #123"` and `"merge pr 1678"` against the fixed manifest. Gate MUST fire (genuine intent caught).

6. **Documentation (AC5):** add a section to `docs/architecture/skill-authoring-guide.md` (if it exists) OR create `docs/architecture/skill-keyword-design-rules.md` documenting the "no bare common-bigram keywords with substring-only matching" rule. Two paragraphs max. Reference back to mika#1650.

## Acceptance criteria

- **AC1** — `skills/bundled/gh-read-only/skill.toml` keyword list replaced per §Fix shape. Bare `"pr"`, `"gh"`, `"issue"`, `"issues"`, `"github"`, `"repo"`, `"repository"` removed. Multi-word intent phrases added covering view/check/list/merge/close/fetch/diff/body intents + literal-number patterns (`"issue #"`, `"pr #"`).

- **AC2** — Audit of `mika-arch-groom-ticket`, `mika-arch-groom-milestone`, `mika-arch-second-review` keyword lists documented in PR body. Result: no changes required (their keywords are multi-word/hyphenated, not bare common bigrams).

- **AC3** — Regression test: prose sample `"the issue with that pr approach is too risky"` (or similar) against the fixed manifest. Matcher does NOT match `gh-read-only`'s keywords. Test added under `crates/mika-agent/src/skills/matcher.rs`'s `#[cfg(test)] mod tests` OR equivalent fixture path.

- **AC4** — Coverage test: prose samples `"view issue #123"`, `"list open prs"`, `"merge pr 1678"` against the fixed manifest. Matcher DOES match — verifying the intent-phrase set covers real user requests.

- **AC5** — Documentation: skill-authoring guide section "Keyword design rules" added with the load-bearing rule: **no bare common-bigram keywords with substring-only matching**. Location: `docs/architecture/skill-keyword-design-rules.md` (new) or extend existing skill-authoring doc if one exists.

## Out of scope

- **Intent-layer architecture** — a structural layer between keyword-match and required-tools-gate enforcement that filters on user-intent-to-do-X (vs incidental mention). Per Mika Prime's bearing read on 2026-06-29 ~13:30 UTC: that's design-bearing (changes the gate's decomposition), routes to mika-arch via a separate ticket. mika#1650 is the tactical fix for the current matcher's collision rate.
- **bbytaa-claude's provisioning template defect.** Litha got `gh-read-only` on her allowlist because the generic operator-assistant template included it for a CEO-persona that doesn't need GitHub. Provisioning-template fix is n=1 today; flag for n=2.
- **GLM-5.2 confabulation-under-re-prompt.** Model-quality concern separate from substrate. Recorded as n=1 observation; escalated as swap-calibration evidence.
- **Word-boundary matching in the matcher engine.** Changing `message_lower.contains(kw)` to `\b(kw)\b` regex matching is a larger structural change. This ticket fixes the keyword set; matcher-engine changes are a separate axis.

## Files involved

- `skills/bundled/gh-read-only/skill.toml` — single keyword-list replacement
- `crates/mika-agent/src/skills/matcher.rs` — AC3/AC4 test additions (or co-located fixture path)
- `docs/architecture/skill-keyword-design-rules.md` — NEW (AC5)
- No engine code changes; no schema migration

## Verification

- **Static:** read PR diff. `skills/bundled/gh-read-only/skill.toml` keyword block is the only edit aside from tests + docs. PR body includes audit table for 3 mika-arch skills (zero changes).
- **Unit tests (AC3 + AC4):** `cargo test -p mika-agent skills::matcher` covers regression + coverage cases.
- **Manual smoke:** start mika-litha (or any agent with `gh-read-only` allowlisted), send a turn like "the issue with that pr approach is too risky" — confirm no `[mika-engine]` re-prompt fires. Then send "view issue #1660" — confirm `gh_read` requirement fires correctly.

## References

- mika-litha founding incident: conversation 2026-06-29 ~12 UTC, glm-5.2 base
- Mika Prime bearing read: session `00000000-0000-0000-0000-000000000000`, 2026-06-29 ~13:30 UTC
- Match function: `crates/mika-agent/src/skills/matcher.rs:50`
- Required-tools gate: `crates/mika-agent/src/agent_loop/mod.rs:4626` (`collect_required_tools`)
- mika#463 — match-reason conditioning
- mika#1645 / mika#1646 — sibling structural-grounding fixes
- Audit class: `skills/bundled/mika-arch-groom-ticket/skill.toml`, `skills/bundled/mika-arch-groom-milestone/skill.toml`, `skills/bundled/mika-arch-second-review/skill.toml` (all clean)
