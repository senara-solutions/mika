# Skill keyword design rules

This guide governs the `[triggers] keywords` list in any bundled or community
`skill.toml`. It exists because keyword matching is **substring-only** and
case-insensitive: `crates/mika-agent/src/skills/matcher.rs` matches a skill when
`message_lower.contains(keyword)` is true for any declared keyword. There are no
word boundaries — a keyword matches anywhere it appears as a literal substring.

## The load-bearing rule

**No bare common-English bigrams (or any short common English word) as keywords
under substring-only matching.**

Short, common substrings collide with ordinary prose. The bare keywords `"pr"`,
`"gh"`, `"issue"`, `"github"`, and `"repo"` each match a large slice of English
vocabulary as substrings:

- `"pr"` → approach, approve, appropriate, press, process, problem, project,
  properly, propose, provide, prevent, …
- `"gh"` → thought, high, weight, right, through, fight, light, neighbor, eight,
  although, …
- `"issue"` → tissue, issuer, reissue
- `"repo"` → report, reporter, repository, repose, repossession

Any sufficiently long English text contains at least one of these as a
substring. When such a keyword belongs to a skill that also declares
`[constraints] required_tools`, the required-tools EndTurn gate fires on
incidental mentions — the LLM never intended to call the tool, the response is
rejected, and a `[mika-engine]` re-prompt is injected. This is a live
false-positive surface, not a theoretical one (mika#1650 founding incident:
mika-litha, a CEO-persona agent with zero GitHub work, hit the `gh-read-only`
gate while writing analytical prose and her GLM-5.2 base model confabulated a
security incident from the re-prompt).

## How to write keywords correctly

Make every keyword signal **intent to do the thing**, not just that the topic was
mentioned:

1. **Multi-word intent phrases.** Prefer `"view issue"`, `"merge pr"`,
   `"github pull request"` over bare `"issue"`/`"pr"`/`"github"`. A two-word
   phrase requires both tokens adjacent, which is far rarer in incidental prose.
2. **Domain-specific tokens that don't collide.** Hyphenated or compound
   skill-domain terms (`"groom-ticket"`, `"milestone-review"`) are safe — they
   don't appear inside ordinary English words.
3. **Literal-number patterns** (`"issue #"`, `"pr #"`) catch the
   reference-by-number request shape without colliding on prose.
4. **Drop discussion-noisy bare topics.** People talk *about* GitHub, PRs, and
   issues without intent to fetch them. Bare topic words belong nowhere in a
   required-tools-gated skill's keyword list.

## Tightening trade-off

Dropping a keyword trades false-positives-down for false-negatives-up. Before
removing keywords from a gated skill, verify the kept set still catches the real
request patterns. Add coverage tests that exercise **both axes**: incidental
prose must NOT match, and genuine intent phrasing MUST match. See the
`gh-read-only` regression and coverage tests in
`crates/mika-agent/src/skills/matcher.rs`
(`test_gh_read_only_does_not_fire_on_incidental_prose`,
`test_gh_read_only_fires_on_genuine_fetch_intent`).

## Note on the matcher engine

Switching the matcher from substring `contains()` to word-boundary (`\b…\b`)
matching is a larger structural change that would relax this rule — tracked in
**mika#1878**. Until that lands, this keyword-set discipline is the only defense
against the collision class. Author keyword lists as if substring matching is
permanent, because today it is.

## Design decision: semantic intent-layer declined (mika#1651)

A follow-up design pass (mika#1651, same architectural class as the mika#1575
unsound-proxy ruling) asked whether a **new semantic intent-layer** should sit
between keyword-match and the required-tools EndTurn gate — to distinguish "the
user mentioned GitHub" from "the user wants me to do GitHub work." Three shapes
were weighed: (A) an LLM intent classifier, (B) per-skill `intent_patterns`, and
(C) message-structure heuristics.

**Verdict: declined (mika-arch RATIFY, 2026-07-29).** The existing three-part
defense — narrow multi-word keywords + this authoring discipline + the tracked
word-boundary matcher swap — is the correct permanent shape. Reasoning:

1. **Multi-word intent phrases already ARE the intent-layer.** A two-word phrase
   requires both tokens adjacent, which *is* an intent signal at zero new
   machinery. Shape B (`intent_patterns`) re-implements what the keyword
   discipline above already prescribes — a second place to express the same
   constraint (an Orthogonality/DRY violation; see `review-guide.md`).
2. **A classifier in a deterministic gate is a category mismatch.** The
   required-tools gate is a predictable, one-retry post-condition. Shape A injects
   non-determinism and per-turn cost into a mechanism whose value *is* its
   predictability. Its false-negative failure mode (gate silently fails to fire
   when the user genuinely wanted a fetch → wrong-but-confident answer, no
   correction signal) is worse than the current false-positive (one wasted
   re-prompt).
3. **The root cause is substring matching, and the fix is the matcher swap.** The
   word-boundary (`\b…\b`) upgrade tracked in "Note on the matcher engine" above
   (**mika#1878**) raises the precision floor for *every* skill at once,
   structurally retiring the collision class — it subsumes most of what Shapes
   A/B/C would buy.

Do not re-litigate this by adding a per-skill intent DSL or an in-gate classifier.
If the collision class resurfaces after per-skill tightening, the correct lever is
the word-boundary matcher swap (mika#1878), not a new intent-layer.
