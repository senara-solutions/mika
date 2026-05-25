---
name: mika-ask-arch
description: Send a message to mika-arch and capture session_id for follow-up correlation
argument-hint: "[--session-id <id>] <message-or-@path>"
---

Send a message to mika-arch with verbose metadata enabled. Used for two-pass plan
review (groom-ticket / second-review).

## Execution

1. **Resolve message body:** If the message starts with `@`, strip the prefix,
   expand `~` to `$HOME`, read the file via the Read tool, and use its content
   as the message body. Otherwise use the literal argument.

2. **Invoke:**

       mika ask --agent mika-arch --format json --verbose [--session-id <id>] "<message>"

3. **Extract and print:**
   - Parse the JSON response. Extract `.content` and print it verbatim (this is mika-arch's reply).
   - Extract `.metadata.session_id` via `jq -r '.metadata.session_id'` (or equivalent JSON path access). Print it on its own line as `session_id: <uuid>` — this preserves the trailer-line shape that downstream consumers (`/mika-groom-ticket`) parse.
   - **Only `.metadata.session_id` is extracted.** The JSON envelope contains additional metadata (`trace_id`, `agent_id`, `provider`, `model`, timestamps, token counts). All other fields are intentionally dropped. If a new field needs surfacing, that is a deliberate spec change — not an incremental extension.
   - **Contract failure mode:** If `mika ask --format json --verbose` produces a payload without `.metadata.session_id`, that is a CLI contract violation — the skill fails loud with a named error, not silent fallback to trailer parsing. The CLI's JSON schema is the contract; consumers depend on it.

## Verdict-keyword discipline

mika-arch's two skills produce a structured disposition:

- **First-pass (`mika-arch-groom-ticket`):**
  `Disposition: READY` | `Disposition: ITERATE` | `Disposition: ESCALATE`
- **Second-pass (`mika-arch-second-review`):**
  `Verdict: GROOMED` | `Verdict: ESCALATE`

Tolerate paraphrased dispositions per the known prompt-adherence drift in
`mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md`.

## Output

The reply content is printed verbatim, followed by a `session_id: <uuid>` line.
The session_id value is extracted from the JSON response's `metadata.session_id`
field — the structured channel is the source of truth. The printed trailer line
is a re-emission for downstream consumers (`/mika-groom-ticket`) that parse by
line matching; it is not the capture source.
