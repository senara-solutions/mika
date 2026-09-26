# Plan: Signal O points `mika ask` at the wrong sink (#2069)

## Problem

`CLAUDE.md:185` — Signal O, described in the same paragraph as "the RT-005
primary-outcome measurement channel" — carries this instruction:

> **CLI vs server sinks:** for a `mika ask` invocation, replace
> `$MIKA_SPIRIT_LOG_FILE` with `~/.mika/agents/<name>/logs/mika.log.$(date +%F)`

That sink holds almost no `turn_usage`. Measured 2026-08-30 (issue body):
**21 084** events in `/var/log/mika/server.log` against **346** across every
`~/.mika/agents/*/logs/mika.log.*` combined. And the 346 are not the sample the
sentence implies — sampling their `session_id` yields `heartbeat-…`,
`reflection-…`, `reminder-…` (silent-mode background turns) plus a handful of
interactive `mika chat` TUI sessions. Not one is an execution turn.

The instruction does not error. It returns a smaller, plausible, wrong dataset
to whoever runs the brick 5/5 analysis — the most expensive shape of a wrong doc.

**Root cause, verified in code.** Since mika#1727 (`98f9b5be`, "thin-client
ask.rs via A2A message/send") the CLI is no longer the execution surface.
`crates/mika-cli/src/commands/ask.rs:294-345` ships the prompt to the local
mika-spirit daemon over A2A `message/send` at `{spirit_url}/a2a/{agent}` and
renders the returned Task; there is no in-process fallback. The source comment
states it outright: *"the local bookkeeping session created above no longer
records agent turns (spirit owns the execution session)"*. So the `turn_usage`
event is emitted by the spirit process and lands in `$MIKA_SPIRIT_LOG_FILE`,
whatever the entry door.

Writing only the corrected path would leave the sentence re-breakable by the
next reader who "fixes" it back. The reason has to be written down with it.

## Changes

### 1. `CLAUDE.md` — Signal O paragraph (line 185)

Replace the **CLI vs server sinks** sentence with a corrected block that carries:
- the single sink (`$MIKA_SPIRIT_LOG_FILE`) explicitly including `mika ask`;
- the **why** (mika#1727: spirit owns the execution session → the event is born
  server-side regardless of entry door), with the code anchor;
- an explicit "do not read the per-agent log for Signal O", with the measured gap;
- what the per-agent sink genuinely holds, so nobody concludes it is empty or broken.

### 2. `crates/mika-agent/CLAUDE.md` — "Log Sinks" section (line 706) — AC4

The Signal O paragraph cross-references this section, and it duplicates the same
claim in three places. All three need the mika#1727 carve-out:

- **Sink table**, "Per-agent CLI log" row: lists `mika ask` among the processes
  writing there. Its *agent turns* no longer go there.
- **Decision tree for operators**: "If you want to read a specific `mika ask` or
  `mika chat` invocation's events, read `~/.mika/agents/<name>/logs/…`".
- **Common mistake** paragraph: "only CLI invocations write there" — true of the
  file, misleading about `mika ask` since mika#1727.

Adjacent inaccuracy corrected in the same table cell (same defect class — which
process writes which sink): `mika team run` does not write to the per-agent sink
either. `init_team_logging` (`crates/mika-cli/src/main.rs:433`, `:461`) targets
`team::team_dir(global_home, team_name).join("logs")`.

### 3. The same claim, wherever else it is live

Fixing the sentence the ticket points at is not fixing the claim. A repo-wide
sweep found it in three further live surfaces; archived plans that describe the
pre-#1727 topology are left alone as dated records.

- `crates/mika-cli/src/commands/logs.rs:16` — doc-comment on `mika logs`, the very
  command CLAUDE.md tells operators to run to resolve both paths. Lists `mika ask`
  as a symmetric producer of the per-agent sink. Comment only, no behaviour.
- `docs/solutions/best-practices/cross-provider-input-tokens-cache-inclusion-asymmetry-2026-08-20.md`
  § Detection — offers the per-agent log as an alternative sink for the same
  `turn_usage` greps.
- `docs/solutions/documentation-gaps/log-sink-architecture-mismatch-2026-05-06.md`
  — the pre-#1727 doc that seeded the Log Sinks section. Historical record, so it
  gets a dated update note rather than a rewrite.

### 4. Staleness this change itself creates

`research/rt005-physics-pilot/orchestration/README.md:176` warns that Signal O
"still says" to read the per-agent path. Correcting Signal O makes that warning
false, so it is updated in the same commit.

### 5. Compound

`docs/solutions/best-practices/stale-doc-plus-matching-stub-hides-a-dead-measurement-channel-2026-08-30.md`
already documents this failure and closes with a section titled "The doc is still
stale". That section is rewritten to record the correction and the generalised
lesson (grep the claim, not the file), rather than adding a near-duplicate entry
to the store.

## Verification

- `grep -n 'MIKA_SPIRIT_LOG_FILE' CLAUDE.md` — the Signal O paragraph names the
  server log as the sink for `mika ask`, with no surviving "replace with the
  per-agent path" instruction.
- `grep -n 'mika ask' crates/mika-agent/CLAUDE.md` — every remaining occurrence
  in the Log Sinks section is qualified by the mika#1727 carve-out.
- Code anchors cited in both files resolve: `crates/mika-cli/src/commands/ask.rs`
  dispatches over A2A with no in-process fallback; `crates/mika-cli/src/commands/chat.rs`
  still calls `agent::run_agent` in-process (so the per-agent sink description holds).
- `bash scripts/verify-pipeline.sh origin/main` passes (docs + source buckets both
  non-empty: plan + compound doc under `docs/`, two `CLAUDE.md` files as source).

## Definition of Done

- Both files edited, committed on `doc/2069/claude-md-signal-o-envoie-mika-ask-vers`.
- No behavioural code touched — this ticket is documentary and mechanical.
- Out of scope, per the issue: caller `session_id` threading (a code defect,
  tracked separately and blocking for the RT-005 RUN).
- PR opened against `senara-solutions/mika` with `Closes #2069` and
  `mika-platform-qa` added as reviewer.

## Acceptance criteria

- [ ] **AC1** — Le paragraphe Signal O de `mika/CLAUDE.md` nomme `$MIKA_SPIRIT_LOG_FILE` comme le puits de `turn_usage`, y compris pour une invocation `mika ask`.
- [ ] **AC2** — La raison est écrite, pas seulement le chemin corrigé : depuis mika#1727 spirit détient la session d'exécution, donc l'événement naît côté serveur quelle que soit la porte d'entrée.
- [ ] **AC3** — Ce que le puits per-agent contient réellement est nommé (heartbeat et réflexion), pour que personne ne conclue qu'il est vide ou cassé.
- [ ] **AC4** — La section « Log Sinks » de `crates/mika-agent/CLAUDE.md`, citée en renvoi par ce paragraphe, est vérifiée et corrigée si elle porte la même affirmation.
