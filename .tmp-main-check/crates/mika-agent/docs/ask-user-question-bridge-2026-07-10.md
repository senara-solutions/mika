# AskUserQuestion callback bridge — TUI ↔ spirit wire protocol (mika#1734)

Sub-ticket **D** of mika#1727 (TUI thin-client refactor). Sibling of sub-ticket
C (`permission-decision-protocol-2026-07-06.md`) — shares the SSE channel and
auth surface via the discriminated-union pattern.

## Executive summary

When mika-spirit's agent loop needs to route an `AskUserQuestion` tool call to
the operator (structured multi-choice question, not a Y/N permission
decision), the TUI receives the question payload on the same SSE stream that
sub-C uses for permission requests, discriminated by the outer `event:` tag.
The operator's answers POST-back to a sibling `/answer` endpoint (parallel to
sub-C's `/decide`), and a server-side hold-timeout materializes a
`Timeout { reason: "operator-timeout" }` result if the operator does not
respond within `Settings.permission_hold_timeout_secs` (default 300s).

## Ratification chain

Ratified for shipping by samidarko-claude via inbox message
`2026-07-10-p-from-samidarko-re-1734-next.md` following the mika#1733 PR#1760
merge. Same discipline as #1733: verify existing wire before building new,
groom+implement, wire-first + gated follow-up split, PR.

## AC1 — Shared SSE channel + discriminated event type

**Ratified**: One SSE channel (`GET /api/v1/dashboard/permissions/stream`)
carries both `PermissionRequest` and `AskUserQuestion` variants, discriminated
by the outer `event:` field via `serde(tag = "event", rename_all =
"snake_case")`. Consumer branches on the discriminant.

Wire shape of the `AskUserQuestion` variant:

```json
{
  "event": "ask_user_question",
  "request_id": "550e8400-e29b-41d4-a716-446655440000",
  "questions": [
    {
      "question": "Which coffee?",
      "options": [
        {"label": "Espresso", "description": "Strong shot"},
        {"label": "Latte", "description": "Milky"}
      ],
      "multiSelect": false
    }
  ]
}
```

`multiSelect` uses camelCase on the wire (serde's `rename_all = "camelCase"`)
to match the shape used by `AskUserQuestion` in `claude-agent-sdk`.
`description` is required-but-may-be-empty; consumers should tolerate the
empty case.

Discriminated-union rationale (reduces connection count + auth surface):
consumers open one bearer-authenticated SSE and receive both event types;
downstream renderers branch on `event`. This is the same channel PR#1741
introduced for sub-C; #1734 tightens the pre-existing `AskUserQuestion`
placeholder from `serde_json::Value` to the structured type.

## AC2 — POST-back `/answer` endpoint + validation

**Ratified**: `POST /api/v1/dashboard/permissions/{request_id}/answer` accepts:

```json
{
  "answers": {
    "0": "Espresso",
    "1": "Small"
  }
}
```

**Answer key convention**: string form of the question **index** (`"0"`,
`"1"`, …). Chosen over question-text-keyed because (a) unambiguous even if a
question text is edited or duplicated within a single request lifetime,
(b) stable against renderer choices, (c) matches the natural iteration order
in the emitted frame. Consumers derive the key by enumerating the `questions`
array received on the SSE frame.

Validation, closed-world (`#[serde(deny_unknown_fields)]` on the body plus
runtime checks on the map):

| Failure | HTTP status | Response body |
|---|---|---|
| unknown `request_id` (never registered or already resolved) | 404 | `{"error":"unknown_request","request_id":"…"}` |
| answers map is missing a question index | 400 | `{"error":"missing_answer","question_index":N}` |
| answer for index `N` is not one of the declared option labels | 400 | `{"error":"invalid_option_label","question_index":N,"supplied":"…"}` |
| answers map contains an extra key (unparseable index or out-of-range) | 400 | `{"error":"extra_key","key":"…"}` |
| classifier's oneshot receiver was dropped before answer arrived | 409 | `{"error":"classifier_dropped","detail":"…"}` |
| body contains an unknown top-level field | 400 | serde-generated |

Failed validation leaves the pending entry intact so a corrected retry can
succeed — mirrors the sub-C ratification-preservation clause: no silent
drops.

## AC3 — Server-side hold-timeout materializes operator-timeout

**Ratified**: `PermissionsChannel::register_pending_ask()` spawns a
`tokio::time::sleep(timeout)` watcher when a request is registered. On
expiry, if the pending entry still exists (no `resolve_answer` has fired),
the watcher removes it and sends `AnswerResult::Timeout { reason:
"operator-timeout" }` on the classifier's oneshot. The agent loop treats
this as a `Deny { reason: "operator-timeout" }`-shaped continuation per the
claude-pilot cpp#20 joint-2 discipline the ticket cites.

`timeout` argument comes from `Settings.permission_hold_timeout_secs` at the
classifier-emit site — same knob shipped in mika#1733 AC3. No new config
key. Default 300s (5 minutes).

Race handling: if `resolve_answer` fires first, it takes the entry out of
the map before the timeout watcher wakes, so the watcher's `remove()` returns
`None` and it exits silently. No double-fire.

## AC4 — Auth model mirrors sub-C

**Ratified**: `/answer` is registered under the same middleware chain as
`/decide` (bearer via `MIKA_INTERNAL_TOKEN` or `MIKA_DASHBOARD_TOKEN`, via
`server::mod.rs`'s `route_layer(middleware::from_fn_with_state(…))`). No new
auth code; no `decision_authority` semantics apply because AskUserQuestion
is not a permission-decision override — it's a structured question.

## AC5 — TUI-side stub consumer

**Ratified**: `crates/mika-cli/examples/ask_user_question_stub.rs` — one-file
demo that subscribes to the SSE stream, logs each `ask_user_question` frame,
and can POST a canned reply (first option of each question). Also logs
`permission_request` frames but does NOT POST a decision (that surface
belongs to the actual TUI, not this stub).

Run:

```bash
MIKA_INTERNAL_TOKEN=<token> \
  cargo run --example ask_user_question_stub -p mika-cli -- \
    --spirit-url http://localhost:8080
```

`--dry-run` logs frames without POSTing. TUI's actual rendering is out of
scope; that lands in mika#1727.

## Implementation call-sites map

| Component | File | Change |
|---|---|---|
| Frame variant tightening | `crates/mika-agent/src/server/permissions_stream.rs::PermissionStreamFrame::AskUserQuestion` | `questions: serde_json::Value` → `Vec<AskQuestion>` |
| Question types | Same | New `AskQuestion` + `AskOption` structs (`camelCase` for `multiSelect`) |
| Answer wire types | Same | New `PermissionAnswerRequest`, `AnswerResult`, `AnswerError` |
| Pending storage | Same | New `pending_asks: Arc<Mutex<HashMap<Uuid, PendingAsk>>>` on `PermissionsChannel` |
| Register + timeout | Same | `PermissionsChannel::register_pending_ask()` spawns the hold-timeout watcher |
| Peek | Same | `PermissionsChannel::peek_pending_ask()` returns a `Vec<AskQuestion>` snapshot |
| Resolve + validate | Same | `PermissionsChannel::resolve_answer()` — atomic remove-and-validate |
| HTTP handler | Same | `handle_permission_answer()` — status code + error body mapping |
| Route wiring | `crates/mika-agent/src/server/mod.rs` | New route `/dashboard/permissions/{request_id}/answer` |
| TUI stub | `crates/mika-cli/examples/ask_user_question_stub.rs` | New — subscribes + canned answers |

## Ratification-preservation clause

This doc is the SOURCE OF TRUTH for sub-D's design. Implementation PRs cite
the sections here (e.g., "implements AC3 per
`ask-user-question-bridge-2026-07-10.md § AC3`"). If a future implementer
discovers a genuine architectural blocker requiring a design amendment, the
implementer:

1. Halts implementation.
2. Drafts a proposed amendment to this doc.
3. Routes through samidarko outbox for Vincent + Prime re-ratification.
4. Does NOT ship implementation code that diverges from this doc without
   ratification.

## Cross-links

- **Parent ticket**: `senara-solutions/mika#1734` (sub-D of mika#1727 fan-out).
- **Sibling protocol**: `crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md` (sub-C AC1-AC8) — shares the SSE channel and auth surface.
- **claude-pilot precedent**: `senara-solutions/claude-pilot-py` `AskUserQuestion` handling in `src/claude_pilot/permissions.py` (both interactive path and `try_tier_1_5_auto_answer` compact-safe shortcut).
- **Closing PR**: mika#1727 (TUI thin-client refactor) — actual TUI-side rendering lands there.
