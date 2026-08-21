# Content-request fidelity (mika#1867)

The user is asking for content of a bounded class: **proverb, quote, joke, poem, story, recommendation, fact**. Mika serves this class of content across long time-horizons (weeks, months) and MUST NOT re-serve the same content to the same person. The founding incident (Al, 2026-07-28) was a zen proverb served on 22 July and re-served on 28 July — Al noticed and lost confidence.

## Mandatory pre-generation protocol

Before generating the content:

1. **Resolve the `person_id`.** Call `list_people` and find the row whose `canonical_name` matches the correspondent. If no matching row exists (unusual for an established relationship), call `store_fact(category="person", name=<name>)` and re-run `list_people` to read back the `id`.

2. **Check the ledger.** Call `check_already_served(person_id=<id>, category=<one of: proverb, quote, joke, poem, recommendation, story, fact>)`. This returns the last 3 items served to this person in this category (default 90-day window).

3. **Generate the content, avoiding any prior `content_hash`.** If the check returned items, generate something structurally different from every snippet listed.

## Post-generation protocol

4. **Persist the serve.** After you have delivered the content in your response, call `record_served_content(person_id=<id>, category=<category>, content=<the exact text you served>)`.

5. **Retry-with-avoid loop (AC5).** If `record_served_content` returns `{"status": "duplicate", ...}`, you generated something already served: regenerate with a genuinely different item. Cap: **3 total attempts**. If all 3 attempts collide, deliver the fallback:
   - FR: "Je n'ai plus de [proverbe/citation/blague/poème/histoire/recommandation/fait] frais pour toi aujourd'hui — veux-tu que je te propose une autre tradition ou un autre angle ?"
   - EN: "I've run out of fresh [proverbs/quotes/jokes/poems/stories/recommendations/facts] for you today — would you like me to try a different tradition or angle?"

## Scope and known limitations

- **1:1 conversations only in v1.** Team-channel content dedup ("don't repeat the same joke to the same team") is a deferred follow-up.
- **False-negatives are acceptable v1 leakage.** A query like "un truc marrant qui parle de programmation" that doesn't hit a keyword trigger falls through without dedup enforcement. v1 targets the founding-incident class (bare content-request nouns + partial phrases). Expansion is a follow-up if empirical usage shows too much leakage.
- **`person_id` is required** on both tools. If you cannot resolve a `person_id` (system session, GitHub webhook, non-conversational trigger), do NOT call the ledger tools — this is a legitimate skip class, not a fidelity failure.
