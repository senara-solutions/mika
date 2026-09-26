# Skill keyword design rules

This guide governs the `[triggers] keywords` list in any bundled or community
`skill.toml`. Keyword matching is **word-boundary** and case-insensitive: since
mika#1878, `crates/mika-agent/src/skills/matcher.rs` compiles each skill's
keyword list into an alternation regex and calls
`Regex::is_match(&message_lower)`. Anchors are emitted conditionally per
keyword edge — `\b` is added only where the keyword's leading/trailing char is
a word character (`[A-Za-z0-9_]`), so `"issue #"` and `"/mika-groom-ticket"`
still match despite their non-word edges. Historically (pre-mika#1878) the
matcher was `message_lower.contains(keyword)` and the entire collision class
below was live.

## The load-bearing rule (post-mika#1878: relaxed to belt-and-suspenders)

**Under word-boundary matching, bare common-English tokens no longer collide
on incidental prose.** Authors of new gated skills can now include
`"gh"`, `"pr"`, `"issue"`, or `"repo"` without the historical false-positive
class firing on `"thought"`/`"approach"`/`"tissue"`/`"report"`. The matcher
enforces this structurally.

The **belt-and-suspenders** discipline still applies: prefer intent-phrase
keywords over bare topic words. Word-boundary matching stops substring
collisions but it does NOT distinguish "the user mentioned PRs conversationally"
from "the user wants to fetch a PR." Intent-phrase keywords like `"view pr"`
and `"merge pr"` do that discrimination; bare `"pr"` does not.

The historical collision surface (illustrative — no longer live under
word-boundary matching):

- `"pr"` → approach, approve, appropriate, press, process, problem, project,
  properly, propose, provide, prevent, …
- `"gh"` → thought, high, weight, right, through, fight, light, neighbor, eight,
  although, …
- `"issue"` → tissue, issuer, reissue
- `"repo"` → report, reporter, repository, repose, repossession

Under the old substring matcher, any sufficiently long English text contained
at least one of these as a substring. When such a keyword belonged to a skill
that also declared `[constraints] required_tools`, the required-tools EndTurn
gate fired on incidental mentions — the LLM never intended to call the tool,
the response was rejected, and a `[mika-engine]` re-prompt was injected. This
was the founding-incident class (mika#1650: mika-litha, a CEO-persona agent
with zero GitHub work, hit the `gh-read-only` gate while writing analytical
prose and her GLM-5.2 base model confabulated a security incident from the
re-prompt). mika#1878 retires this class structurally.

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

The matcher uses **word-boundary regex matching** (Rust `regex` crate `\b`,
Unicode-aware by default) since mika#1878. The substring-collision class is
retired structurally — every skill's keyword list benefits at once, and
per-skill keyword tightening (the historical mika#1650-style discipline) is
now belt-and-suspenders rather than the sole defense.

**Implementation shape.** `matcher.rs::build_matcher_regex()` builds one
alternation regex per skill on each `match_skills()` call. `\b` anchors are
emitted per keyword edge only when the edge character is a word char
(`[A-Za-z0-9_]`), so:

- `"gh"` → `\bgh\b` — anchors on both edges.
- `"view issue"` → `\bview issue\b` — anchors on both edges; internal space
  is a natural non-word char, so tokens must appear adjacent.
- `"issue #"` → `\bissue #` — no trailing `\b` (trailing `#` is non-word,
  and an unconditional trailing `\b` would demand a following word char that
  we don't want to require).
- `"/mika-groom-ticket"` → `/mika\-groom\-ticket\b` — no leading `\b`
  (leading `/` is non-word).
- `"[GitHub]"` → `\[GitHub\]` — both edges non-word, no anchors emitted;
  bracket-shaped tokens fall back to literal substring semantics, which is
  the correct intent for webhook-shape keywords.

**Coverage tests (mika#1878).** See `matcher.rs::tests` —
`test_word_boundary_bare_bigram_does_not_collide_on_prose`,
`test_word_boundary_bare_bigram_still_fires_on_standalone_token`,
`test_word_boundary_multiword_keyword_requires_adjacent_tokens`,
`test_word_boundary_punctuation_suffixed_keyword_still_fires`,
`test_word_boundary_slash_prefixed_keyword_still_fires`, and siblings.
