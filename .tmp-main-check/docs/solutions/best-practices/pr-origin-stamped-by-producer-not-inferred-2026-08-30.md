---
module: skills/bundled/_shared/dispatch-lib.sh
tags: [dispatch-lib, pr-origin, measurement, instrument-honesty, mika2026, loop-substrate]
problem_type: unmeasurable-state
category: best-practices
---

# An origin marker belongs to the producer, at the moment of production (mika#2026)

## Problem

The cadence reading asks for merged PRs **split by origin** — produced by the autonomous
loop, or opened by hand. The "merged" half was exact. The origin half had no instrument at
all, so any split anyone produced was an estimate dressed as a measurement.

Every candidate source was checked on 2026-08-28 and again on 2026-08-30:

| Source | Verdict |
|---|---|
| `tasks.metadata.$.claude_pilot.pr_url` | **43 rows across all repos since forever.** The five loop PRs merged 2026-08-27 (#2014–#2018): zero rows. |
| GitHub author | `samidarko` for loop PRs and by-hand PRs alike. |
| Branch name | The orchestrator uses the same `scripts/derive-branch-name` as the loop — `fix/1962/…` is indistinguishable either way. |
| `dev_runs` table | Does not exist. |
| `pilot_transcripts`, `audit_events` on `run_claude_pilot` | Empty. |

## Why the existing counter under-counted — the named cause

`pr_url` rides a **four-link text channel**:

```
dispatch-lib discovers the PR  →  "PR: <url>" line inside RESULT  →  callback traverses
mika-dev + task-engine  →  regex ^PR:\s+ (dispatcher.rs, extract_callback_fields)  →  DB write
```

`extract_callback_fields` only writes `pr_url` when a well-formed callback reaches the
engine. So the counter measures **well-formed callbacks that reached the engine**, not PRs
the loop produced. It is an instrument measuring its own plumbing.

The corroborating detail is sharper than the count: the two rows that *did* land in the
2026-08-26/27 window (#2019, #2021) came from tasks that are themselves `failed`. Channel
success is not even correlated with task success.

Hardening the four links was rejected. None of them has the PR for a subject, and the
result would still live in a database whose loss takes the measurement with it.

## Solution

**The fact lives on the artefact, stamped by its producer, at the moment of production.**

`_stamp_pr_origin <repo> <pr_ref> [origin]` in `dispatch-lib.sh` applies an `origin:loop`
label at each of the three — and only three — points where dispatch-lib holds a PR it just
produced (crash-recovery discovery, normal post-session discovery, mika#1396 rescue
creation). In shell, never through the pilot's prompt: prompt enforcement is precisely what
fails at loop substrate.

Read side: `scripts/pr-origin-report.sh` — one command, one window, split by origin.

## The three disciplines that keep it from lying

**1. Never reconstruct after the fact.** Not by branch name, not by author, not by time
window. All three are identical between the loop and by-hand work, so all three fail exactly
on the day the answer matters. (`dependabot` is the one exception, and it is not an
inference: the author *is* the producer.)

**2. An absent marker reads "unknown", never "by hand."** A default that resembles an answer
is how an instrument lies. The report's mutation test pins this: flipping the fallback
category from `unknown` to `manual` breaks four assertions.

**3. A cut-off recorded, not guessed — and not probed either.** Absence of the label is only
informative after the marker went live. That instant is never hand-written into the script (a
constant written while coding is wrong the moment the deploy lags), and — the trap we walked
into first — it must not be read off a file's mtime. `seed_support_dirs` rewrites the
installed `dispatch-lib.sh` unconditionally on every daemon start (`std::fs::write`, no hash
gate), so an mtime tracks the **last restart**, not the first stamp. On a host that bounces
several times a day the cut-off would walk forward continuously and quietly re-open the blind
window after every restart — a lie with a plausible-looking source.

The producer records it instead: `_record_pr_origin_epoch` writes the instant of its first
successful stamp, exactly once, to `~/.mika/state/pr-origin-epoch`. Resolution is then
`MIKA_PR_ORIGIN_EPOCH` → that file → undetermined, and undetermined means the report
classifies **nothing** and prints the fix.

The error direction is deliberately asymmetric. A label present counts as loop whatever its
date; the epoch only governs what *silence* means. And silence is tested against the PR's
**opening** date, not its merge: a loop PR opened before the marker went live and merged
after it was never in a position to carry a label, so it reads `unknown` — calling it
"not-loop" would be a confident false answer about precisely the PRs in flight across the
cutover.

## Reusable shape

Three ideas transfer to any "we can't see X" ticket:

- **A measurement carried by a side-channel measures the side-channel.** If the fact is about
  an artefact, put it on the artefact.
- **Have the producer record its own start.** We first tried probing the installed file's
  mtime — plausible, self-maintaining, and wrong, because something else rewrites that file
  on every restart. A timestamp is only as good as the event that wrote it: make the code
  whose behaviour you are dating write the date.
- **Fetch on the axis you filter on.** `gh pr list --state merged` pages by *creation* date;
  filtering the result by `mergedAt` drops long-lived PRs merged inside the window and
  under-counts in silence. The fetch now carries `--search "merged:>=… merged:<=…"`.
- **Design the error direction before the happy path.** Decide which way an instrument should
  be wrong when it is wrong, then check that the fallback fails that way. Mutation-test the
  fallback specifically; that is where a comfortable lie hides.

## Two conflicts the instrument surfaces instead of resolving

Two of the three callsites reach a PR dispatch-lib *discovered* on the branch rather than
created, and the orchestrator derives branch names with the same script the loop uses — so a
by-hand PR can already be sitting there. The producer therefore claims only an **unclaimed**
PR: an existing `origin:*` is never overwritten, and the skip is named on stderr. When a PR
does end up carrying two origins, the reader reports it in a `CONFLIT` bucket rather than
picking a winner by `if/elif` precedence. Resolving a disagreement by precedence is how a
report hides one.

## Known coverage limit, stated rather than papered over

Only `origin:loop` has a structural producer in this repo. `origin:spawn` and `origin:manual`
are vocabulary the reader understands and a human or spawn may apply; their automatic
stamping lives outside `mika`. This is enough for the question asked: after the epoch,
`origin:loop` present ⇒ loop, absent ⇒ not-loop. Before it, `unknown`.

## Files

- `skills/bundled/_shared/dispatch-lib.sh` — `_stamp_pr_origin` + three fail-open callsites
- `scripts/pr-origin-report.sh` — the reader
- `.github/labels.yml` — `origin:loop` / `origin:spawn` / `origin:manual`
- `skills/bundled/_shared/tests/test_stamp_pr_origin.sh`, `scripts/test-pr-origin-report.sh`
- `make test-pr-origin`, wired into CI
