---
issue: 2313
type: fix
module: dispatch-lib / claude-pilot containment
tags: [substrate, prompt-cache, bwrap, mika-2108, mika-2039, loop-breaker]
problem_type: loop-breaker
---

> [!WARNING]
> **Reverted by mika#2318 (2026-09-18).** The fix described below is no longer in the
> tree: `88b02f97` was reverted in full, minus this document. The code is still
> readable at that SHA.
>
> **The real cause was the egress-proxy relay**, not the missing file — it forwarded
> the client's keep-alive upstream, the request hung ~360 s, and the cache TTL was
> exceeded. Fixed by mika#2313 → PR #2316 (`df13a4e3`), refined by mika#2317 →
> PR #2374 (`55c5fd28`). The `~/.claude.json` emitter was inert with respect to the
> cache, so mika#2318 removed it to reduce the pilot sandbox's surface.
>
> **The `~/.claude.json` hypothesis is untested, not refuted — and the difference
> matters.** Measured 2026-09-18 on `gentux`, the dispatch host:
> `~/.local/bin/mika-pilot-sanitize-claude-json` was **absent**, while its
> `make install` neighbour `mika-pilot-egress-proxy` was present and dated the same
> day. The block's guard requires `-x` on that binary, so the mechanism below never
> ran there — and it said nothing about it, the only `echo` living *inside* the `if`.
> AC4 above (*"a dispatch without the emitter/file stays cache_read = 0 — the
> mechanism is proven, not assumed"*) has no recorded replay. So if the cache goes
> cold again with the relay fixes in place, this document is where the hypothesis is
> found, and `88b02f97` is where the code is.

# fix(2313): the contained pilot never reads the prompt cache

## Problem (P0 loop-breaker, source-verified)
Since the bwrap containment (mika#2108, ~2026-08-30), every sandboxed pilot turn
had `cache_read_input_tokens = 0` and `cache_creation_input_tokens = 100k–250k`:
the full context was re-created every turn. Consequences: ~355 s per turn (the
"6-minute turns" of the #2295 grooms; cpp#177's long TTFT), 10–12 M input tokens
per groom, and the weekly subscription hitting `429` three days before reset.

## Root cause (measured)
`dispatch-lib.sh` blanks `/home` with `--tmpfs /home` and ro-binds `~/.claude/*`
but NOT the file `~/.claude.json`. That file holds the CLI's cached GrowthBook
feature flags (`cachedGrowthBookFeatures`, `cachedExperimentFeatures`), which
govern the prompt-cache `cache_control` strategy. Absent it, the CLI falls back
to a default that never reads cache. Host reproductions (CLI, relay, mitmdump,
claude-pilot direct) all cached fine — the break is the missing file in the
sandbox. Providing a `~/.claude.json` restored the cache: **cache_read 0 → 31226**
on a sandboxed pilot (p0cachetest2, 2026-09-15).

## Fix
- New emitter `scripts/mika-pilot-sanitize-claude-json`: an **allowlist** of
  feature-flag/cache keys only. Account/credential keys (`oauthAccount`,
  `userID`, `machineID`, `bridge*`, tokens) never pass — including any key added
  to `~/.claude.json` in the future (mika#2039 holds by construction).
- `dispatch-lib.sh` (`_run_pilot_sandboxed`): regenerate the sanitized file
  **fresh each dispatch** (the flags carry an expiry, `cachedGrowthBookFeaturesAt`)
  into a per-invocation temp file and `--ro-bind` it to `$HOME/.claude.json`.
  Degrades gracefully (missing/failed emitter → cache-cold, never a lost dispatch).
- `make install` ships the emitter to `~/.local/bin` alongside the egress proxy.

## Tests
- `scripts/test-pilot-sanitize-claude-json.py` (unit, no pilot/network): a
  fixture mixing allowlist keys + every forbidden class → asserts 0 forbidden
  key, 0 secret-shaped value, and that the prompt-cache-governing
  `cachedGrowthBookFeatures` (whose flag NAME contains "oauth") survives.
- **Proof at install (negative + positive)**: a sandboxed pilot at ≥2 turns must
  show `cache_read > 0` at the 2nd turn in its own transcript AND turns < 60 s.
  Negative already established: bbb5ab24 (10.27 M created, 0 read) WITHOUT the
  file.

## Acceptance criteria
- AC1: `scripts/test-pilot-sanitize-claude-json.py` passes — the emitter drops every forbidden key (`oauthAccount`, `userID`, `machineID`, `bridge*`, and unknown/future keys), leaks no secret-shaped value, and preserves `cachedGrowthBookFeatures` (including its oauth-NAMED flag).
- AC2 (positive, at install): a sandboxed pilot at ≥2 turns shows `cache_read > 0` at the **2nd turn in its own transcript**.
- AC3: that pilot's turns complete in **< 60 s** (vs ~355 s cache-cold).
- AC4 (negative): a dispatch without the emitter/file stays `cache_read = 0` — the mechanism is proven, not assumed.
- AC5 (security, mika#2039): no account/credential value ever appears in the emitted `~/.claude.json`; the emitter is an allowlist, verified by AC1.

## Rollback
Revert this commit; `_run_pilot_sandboxed` stops binding `~/.claude.json` and the
pilot returns to cache-cold (the pre-fix state) — no other behaviour changes.
