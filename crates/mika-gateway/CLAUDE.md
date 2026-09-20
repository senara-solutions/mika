# mika-gateway — Webhook Router

Telegram and GitHub webhook router with Postgres customer registry. Handles text messages, images, and GitHub App events. Env-var-only config.

## Endpoints

| Endpoint | Method | Auth | Purpose |
|----------|--------|------|---------|
| `/webhook/telegram` | POST | Webhook secret | Inbound Telegram messages (single-bot mode) |
| `/webhook/telegram/{customer_id}` | POST | Per-customer webhook secret | Inbound Telegram messages (per-customer bot) |
| `/webhook/github` | POST | HMAC-SHA256 | Inbound GitHub App events |
| `/send` | POST | Internal token | Outbound relay with `agent_name` identification |
| `/health`, `/readyz`, `/livez` | GET | None | Health probes |
| `/version` | GET | None | Returns `{"version":"<semver>","git_hash":"<short-hash>"}` |
| `/webhook/dlq` | GET | Internal token | List DLQ entries (pending + dead) |
| `/webhook/dlq/{delivery_id}/replay` | POST | Internal token | Replay a single DLQ entry |
| `/webhook/dlq/replay-all` | POST | Internal token | Replay all dead DLQ entries |
| `/a2a/{customer_id}/{agent_name}` | POST | API key | A2A protocol proxy (2MB limit) |
| `/a2a/{customer_id}/{agent_name}/agent.json` | GET | None | Agent Card proxy |
| `/orchestrator/inbox/{orchestrator_id}/message` | POST | Internal token | Persist spawn→orchestrator message (256KB limit, mika#1189) |
| `/orchestrator/inbox/{orchestrator_id}/stream` | GET | Internal token | SSE stream of inbox messages with cursor replay (mika#1189) |
| `/admin/customers` | POST | Internal token | Register/re-register a per-customer Telegram bot (mika#1609). **Error body (mika#2191):** always `{"error": "<message>"}` — unchanged shape — plus `upstream_status: <int>` when the Telegram upstream actually **refused** the token (401 on a dead token, 403, 429, any status `>= 400`). The field answers "did the upstream refuse, and with what status", not "which HTTP byte crossed the wire": it is **absent** when no refusal is established — `getMe` answering 200 with an unusable body, a locally-built `BadRequest`, a network failure that never got a response, and the `bot_username` mismatch, which lives on the success arm. The gateway's own status stays 400 on every branch. The `error` message of the 401 branch is a **wire format**: mika-cloud's `classify_gateway_error` still matches the substring `invalid bot_token` (mika-cloud#205) until it removes that rung, so rewording it is a cross-repo break to be dated. |
| `/admin/customers` | GET | Internal token | List customers filtered by `status`, `paired`, `stale_after_minutes` — orphan sweep (mika#1820) |
| `/admin/customers/{customer_id}` | GET | Internal token | Read a customer record — safe fields only, never `bot_token` / `pairing_token` value / `webhook_secret` (mika#1820) |
| `/admin/customers/{customer_id}/unlink` | POST | Internal token | Release a customer's Telegram binding server-side (mika#1749) |
| `/admin/tenants/{customer_id}/recurring-tasks` | GET | **Admin read token** (`MIKA_GATEWAY_ADMIN_READ_TOKEN`) | Read-only proxy of the tenant's recurring-task registry (`GET /api/v1/recurring-tasks` on the pod) — metadata only, no message content; the internal (write) token is refused with 403; `customer_id` must be a UUID present in `customers` before any forward (mika#2360). Each served call writes an `audit_events` row (`tool_name = 'gateway_admin_read'`, `target_key = 'tenant:{uuid}'`). |

## GitHub Webhook Integration

HMAC-SHA256 signature validation via `X-Hub-Signature-256`. Event routing:
- `issues.assigned` and `issues.labeled` and `issue_comment.created` and `pull_request_review.submitted` and `pull_request.closed` and `check_suite.completed(failure/timed_out/success)` -> mika-dev
- `pull_request.opened/synchronize/review_requested/ready_for_review` -> mika-qa (`ready_for_review` added mika#1822 — draft→ready transitions were previously dropped as "not routable", stranding every wip-rescue-shaped PR without an autonomous review; same pitfall documented for `.github/workflows/*.yml` triggers in `docs/solutions/best-practices/gha-pr-workflow-event-pitfalls-2026-04-29.md` lines 38-53)
- **Review-requested reviewer filter (mika#1655):** `pull_request.review_requested` routes to mika-qa via `route_event`, but a post-route guard (`is_suppressed_review_request` in `github.rs`) drops the event unless `requested_reviewer.login == QA_REVIEWER_LOGIN` (`mika-platform-qa`). This makes operator-driven re-requests (`gh api .../requested_reviewers` for the QA bot) trigger an autonomous qa-review while preventing human-reviewer requests (or team requests carrying `requested_team` instead of `requested_reviewer`) from spinning up a full qa-review session. Fail-closed: a missing/unresolvable reviewer is suppressed. The guard sits between routing and the `info!` dispatch log, alongside the skill denylist (#845) and synchronize no-diff (#886) guards.
- Delivery UUID dedup via 10k-entry LRU cache
- 256KB body limit
- Multi-tenant routing via `github_repos` table lookup with `agent_base_url` fallback for single-tenant mode
- **Internal-repo allowlist (#1382):** `INTERNAL_REPOS` const in `github.rs` lists org-internal repos (`senara-solutions/mika`, `mika-cloud`, `mika-skills`, `claude-pilot-py`, `mika-platform`, `wizzard`) that resolve to `agent_base_url` without requiring a `github_repos` row. Enables cross-repo `ready`-label dispatch for the autonomous loop. Explicit `github_repos` registrations always take precedence. Unknown external repos still drop in multi-tenant mode (closed allowlist). Adding a repo is a code change + deploy (not env-tunable — security-adjacent).
- Machine user assignee filtering: `MIKA_GITHUB_APP_LOGIN` in per-agent `.env` (e.g., `~/.mika/agents/mika-dev/.env`) should match the machine user login (e.g., `mika-platform-dev`). Filtering logic lives in the self-dev skill prompt, not in gateway code.
- **Webhook skill denylist (#845):** `WEBHOOK_SKILL_DENYLIST` const in `github.rs` blocks operator-only skills from being triggered via webhook events. For `issues.labeled` events, the label name is checked against the denylist (case-insensitive). Denylisted events are dropped with `StatusCode::OK` (prevents GitHub retries) and a `warn!` log. Currently contains `"dev-groom"`. This is Layer 3 defense-in-depth — Layer 1 (`well_known_agents.rs` `disabled_skills`) is the primary check at the agent level.
- **Synchronize no-diff guard (#886):** For `pull_request.synchronize` events, the gateway compares `before` and `after` commit SHAs (from the webhook payload) via the GitHub Compare API (`GET /repos/{repo}/compare/{before}...{after}`). If zero files changed (trailer-only amend, commit-message-only push), the event is suppressed — no mika-qa session is created. Prevents cross-session duplicate APPROVED reviews. Requires `MIKA_GITHUB_APP_ID`, `MIKA_GITHUB_APP_PRIVATE_KEY`, and `MIKA_GITHUB_APP_INSTALLATION_ID` to be set. Fail-open on all error paths: API timeout (5s), HTTP errors, missing credentials, token refresh failure, malformed response. Guard runs between the skill denylist check (step 9b) and semaphore acquisition (step 10). Structured log event: `webhook_synchronize_no_diff_change`.
- **Body truncation caps (#911):** `format_event_text` truncates webhook body fields per event type via `truncate_body()`. Two named constants: `DEFAULT_GITHUB_BODY_TRUNCATION_CHARS = 2_000` (issue, PR, comment) and `GITHUB_REVIEW_BODY_TRUNCATION_CHARS = 16_000` (pull_request_review only). The review cap is higher because mika-qa review bodies are structured-and-long (3–5 KB typical: DIFF ANALYSIS + PLAN-AC VERIFICATION + BUILD VERIFICATION + VERDICT) and the engine's verdict parser depends on the VERDICT token surviving transport. See `docs/solutions/best-practices/gateway-truncation-cap-per-event-type-calibration-2026-05-01.md` for the per-event-type calibration principle.

### Inbound delivery retry (#589)

The spawned forwarding task retries on HTTP 429/5xx or request timeouts using the fixed schedule `[2s, 5s, 15s, 60s, 300s]` with ±25% per-attempt jitter (prevents synchronized retry bursts on the same agent). Permanent failures (HTTP 4xx other than 429, non-localhost connection errors, or unresolvable route) stop retries immediately. Localhost connection errors are retryable (#1293) — the agent may be restarting during a deploy. Route resolution (`github_repos` lookup + `agent_mapping`) is cached across retries — a single Postgres query per event regardless of retry count.

Semaphore lifecycle during retry: the 30-permit `webhook_semaphore` (shared with Telegram) is released during each retry sleep and re-acquired via `try_acquire_owned` before the next attempt. If the semaphore is full on re-acquire, the retry is abandoned with a dedicated ERROR log (`semaphore at capacity during retry`), distinct from the `retry budget exhausted` ERROR emitted when all 6 attempts return a retryable failure.

The delivery LRU cache has no TTL (size-based eviction only). Under extreme webhook volume (>10k deliveries during a single 300s retry sleep), the `X-GitHub-Delivery` entry may be evicted and a GitHub redelivery would bypass gateway dedup. Agent-side idempotency (task unique index on `reference_url`) mitigates double-processing.

### Dead-letter queue (#590)

Events that exhaust the retry budget or are abandoned due to semaphore pressure are persisted in the `webhook_deliveries` Postgres table (migration 006) with `status='pending'`. A background tokio task wakes every 30s, selects pending rows past their exponential backoff window (`30s * 2^attempts`, capped at 1h), and re-attempts delivery via the same `forward_to_resolved_route()` path. Route is re-resolved on each worker attempt (container URLs may change). After 10 worker attempts, status transitions to `'dead'`.

The DLQ respects the shared 30-permit webhook semaphore — if all permits are held, the worker skips forwarding for that tick. Manual replay via `POST /webhook/dlq/{id}/replay` and `POST /webhook/dlq/replay-all` follows the same semaphore-gated delivery path. CLI: `mika webhook list-dead`, `mika webhook replay <id>`, `mika webhook replay-all`.

## Agent Identification & Reply Routing

- Outbound messages carry `agent_name` in the `/send` payload; gateway prepends `[agent_name]` to Telegram text (**except when `agent_name == mika_common::agent::DEFAULT_AGENT` (`"mika"`)** — single-agent customers, including every family-tier customer, see raw text with no `[mika]` parasite; suppression added after family-tier launch 2026-07-16 when the prefix leaked into a family member's first-hour greeting) and stores `(telegram_message_id, chat_id, agent_name)` in `outbound_messages` Postgres table
- Parses `reply_to_message` from Telegram updates; looks up the originating agent via `outbound_messages` and forwards the inbound message with `"agent": "<name>"` to the correct agent in the container
- Periodic cleanup: purges `outbound_messages` older than 7 days (batched, every ~100 webhooks)

## Outbound Text Rendering (mika#2126 → mika#2291)

**This section is the documentation debt mika#2291 paid on the way.** Until it was
written, all three rendering decisions lived only in doc-comments — which is
plausibly why "the gateway does not apply Telegram rendering" was filed as an
oversight rather than found as a dated decision.

`telegram.rs::send_message_impl` is the **single** `sendMessage` call site of the
crate; both `TelegramClient` and `CustomerTelegramClient` converge on it, so the
~12 `let _ = tg.send_message(...)` sites inherit everything below without having to
remember it.

**One recognizer, two renderings** (`telegram_markdown.rs`):

```
tokenize(text) -> Vec<Segment>        // recognition, once
    render_html(&segments)  -> String // armed mode (default)
    render_plain(&segments) -> String // floor: disarmed mode AND the fallback
```

- **HTML, never MarkdownV2.** MarkdownV2 requires escaping eighteen characters over
  the *entire* text, URLs included; HTML mode reserves three (`<`, `>`, `&`) and only
  outside tags, so the escaping is local instead of global. A third mode goes back
  through grooming, not into the payload.
- **The fallback is what makes it tenable.** On a 400 *while `parse_mode` was set*,
  the gateway re-sends **once**, without `parse_mode`, with the plain text. The worst
  case of the HTML path is therefore exactly the pre-mika#2291 behaviour minus the raw
  markers: **a message can no longer be lost because of a rendering.** The trigger is
  the **status 400**, never a substring of Telegram's `description` (mika#2179's rule:
  classes come from the variant, not from the rendered message). 401 / 403 / 429 / 5xx
  are returned unchanged — replaying them would double a doomed call and, on 429,
  worsen the limit. The second send has no fallback of its own.
- **The recognition table is closed.** `**bold**` / `__bold__`, `*ital*`, `_ital_`
  (not intra-word), `~~struck~~`, `` `code` ``, ` ```fenced``` `, `[label](url)`,
  `# ` headings and `* ` / `+ ` bullets. **Everything else stays verbatim** — `>`
  quotes, `1. ` lists, `---`, four-space indentation, tables. That is mika#2126's AC3
  doctrine: a fix that rewrites a healthy message has repaired nothing, it has added a
  second way to break it.
- **No markdown parser, and the reason is a tested property.** mika#2126 froze
  byte-for-byte preservation including double spaces and a tab; a CommonMark
  round-trip normalizes whitespace and over-interprets conversational prose. The day
  the output must carry tables or nested lists, a parser becomes justifiable — but
  that constraint must be renegotiated explicitly first, because the two are
  incompatible.
- **`strip_markdown_around_urls` (mika#2126) is unchanged** and kept as a second pass
  on the **plain path only**, via `plain_body` — the single construction site of the
  plain body, which is what makes "disarming and the fallback emit the same byte" a
  property of the code. It is deliberately *not* run before `render_html`: it rewrites
  `[label](url)` as `label : url` and would destroy the `Link` that `render_html` must
  emit as `<a href>`. So in the **armed mode — the default — what holds mika#2126's
  founding defect is the recognizer, not the net**: paired decoration around a URL
  becomes a tag (so the link's boundary is the tag) and unpaired decoration stays
  verbatim (so the URL passes intact). `mika2291_r5_*` controls exactly that.
- **`[agent] ` prefixes survive both renderings.** `routes.rs` composes
  `format!("[{name}] {text}")` *before* the send, and `resolve_reply_agent` reads that
  prefix back off the quoted text to route the user's reply. Frozen by
  `mika2291_n13_*` through `parse_agent_prefix` itself; widening the recognizer to
  bare `[text]` would drop the primary route onto its DB fallback — a degradation that
  breaks nothing visible.

**Kill-switch: `MIKA_TELEGRAM_HTML_RENDER`**, default **armed**, read once per process
(`OnceLock`), not hot-swappable — set it in the EnvironmentFile / ConfigMap **before**
startup. `0` / `false` / `off` / `no` disarm; absent, empty, or unrecognized stays
armed (a typo must not silently switch rendering off) with a WARN naming the value
between quotes. The field is `Option<String>` and **never `bool`**: under config-rs a
`bool` makes any non-boolean value a hard `GatewaySettings::load` error, so a typo in
a p2 cosmetic flag would stop the gateway from booting. No `bool` lives in
`GatewaySettings`.

**Operator surfaces** (`$MIKA_GATEWAY_LOG_FILE` or stdout). No `audit_events` row per
send: for a population expected to be empty, the log is enough and a row per message
would be churn.

- `telegram_html_render_fallback` (WARN — `chat_id`, `description`, `len_html`).
  **Expected regime: zero lines.** Any occurrence is a message the HTML rendering
  broke and the fallback saved — both proof the net works and a recognizer case to
  fix. The message body is **never** logged.
- `telegram_html_fallback_failed` (WARN). **Expected regime: zero lines.** This is the
  population where the user actually receives nothing; without the event it would be
  indistinguishable from an ordinary 502.
- `telegram_html_render_disabled` (INFO, once at startup, only when disarmed) and
  `telegram_html_render_unrecognized_value` (WARN). The first exists because the
  silence of a disarmed renderer looks exactly like the silence of a healthy one
  (mika#2205).

**Post-deploy probe, with its halt.** Ask a tenant for a reply carrying emphasis and
check Telegram shows it **rendered** with no `*` visible. Then over 48 h:
`telegram_html_fallback_failed` must be empty (any line is a user who received
nothing — treat first), and `telegram_html_render_fallback` should be empty too — a
few isolated lines mean *read the `description` and fix the recognizer*, not disarm.
**Halt at a sustained rate (> 1 % of sends): disarm with
`MIKA_TELEGRAM_HTML_RENDER=0`, fix the recognizer, then re-arm** — a net carrying
nominal traffic is no longer a net, and it hides the signal that would show the fault
(mika#2334 doctrine). **Second halt:** if raw markdown **reappears** while
`telegram_html_render_fallback` is empty, do **not** widen the recognizer — it means
the text left by a path that does not traverse `send_message_impl`, and establishing
which path comes before any fix. There is no such path today, so that finding would be
information about the architecture before it is information about the rendering.

**Known adjacent gap, deliberately out of scope.** `mika-common/src/telegram.rs`
claims the gateway refuses text at 4096 "as it will be sent, prefix included"; that
mirror guard **does not exist** — `handle_send` only checks `text.len() > 50_000`
bytes (`routes.rs`). The unfinished half of mika#2134, unrelated to rendering, and
covered incidentally by the fallback (a length 400 fails identically on the second
send and returns that error). Follow-up ticket to open.

## User-Facing Copy and Locale (mika#2025)

**The gateway's own copy never traverses a persona**, so mika#2023 (the agent's
English greeting) does not and cannot fix it: six of these messages are served
*before* pairing, when there is no customer row and no agent to call.

`copy.rs` is the **single producer**. `render(UserMessage, Locale) -> &'static
str` matches on the **pair** with **no `_ =>` arm** — the compiler, not a
reviewer, forces every new message and every new language to decide, on the
`hosting_ground_truth_line` model (mika#2290, mika#2292). Fifteen static keys, no
interpolation, two languages: an i18n framework would cost more than it returns,
and a third language is a deliberate act the `match` makes non-forgettable.

**Two guards, because the regression is invisible to behaviour.** A seventeenth
hard-coded English literal at a send site makes no decision wrong — it restores
the defect on one key, silently. So `routes::tests::mika2025_v10_*` scans
production sources and refuses a string literal in any argument of
`send_message`; `copy::tests::mika2025_v11_*` refuses a wildcard arm. **The V10
allowlist (`SEND_MESSAGE_LITERAL_ALLOWED`) ships empty and stays empty**: when
it reddens, the resolution is to add a `copy::` key, never an entry — an entry
decides that one message is English-only for every user and needs its own ticket
(the rule mika#2323 had to write for `ACTOR_READING_PREDICATES_ALLOWED`). Both
carry a negative control on a fabricated input *and* an anti-vacuity count over
the real tree, so "the scan found nothing" is distinguishable from "the scan
looked at nothing" (mika#2205).

**The signal is `message.from.language_code`, and the account column is
deferred.** `resolve_locale` is the sole reader: prefix before the first `-`,
ASCII-case-insensitive, `fr` → French, everything else → English. `from` was
never deserialized before mika#2025, which is why no language signal existed
anywhere in the process — the fix is one `#[serde(default)]` field and **zero
migration**. `customers` has no locale column and adding one needs a *writer*,
which is the `mika-cloud` console: until someone writes it the column is NULL and
nothing changes for the user. The cascade is written gate-by-gate so that column
inserts itself as one more rung in front of this one. **Named limit:**
`language_code` is the language of the client's *Telegram interface*, not of
their Mika account — a francophone whose phone is in English still gets English,
and this does not close that. **Reopening criterion, a measurement not a hunch:**
open the `mika-cloud` ticket the day a tenant is observed receiving a language
that is not theirs *despite* this fix.

**`/unlink`: the action leads, the warning follows.** The reported behaviour was
a reader skimming "⚠️ … cannot be undone" and replying with the command they
already knew — `/unlink` — instead of the one on the last line they never
reached. Line order is the only half of the salience that survives
`MIKA_TELEGRAM_HTML_RENDER=0` **and** mika#2291's plain-text fallback; the
backticks are a reinforcement that degrades into a legible quotation. **The
command is deliberately not bolded** — a raw `**` is precisely the marker whose
cost mika#2291 measured on a live tenant.

**Three states, three answers.** `ParsedMessage::Unlink` now carries the suffix
the parser had always computed and discarded, so a *tried* confirmation
(`/unlink oui`) gets a reply that says so and quotes it back, instead of the same
reminder a bare `/unlink` gets. This half is **not optional**: serving the copy
in French makes `/unlink confirmer` *more* likely, so the localization enlarges
the population of the third state. `confirmer` therefore joins `confirm` as a
recognized form — an input tolerance, never a second interface: the copy keeps
prescribing `/unlink confirm` in both languages, and
`copy::tests::mika2025_the_prescribed_command_is_the_parsed_command` is the only
thing joining the two modules that spell it.

**Operator surfaces** (`$MIKA_GATEWAY_LOG_FILE` or stdout). No `audit_events`
row: the gateway sees every Telegram message of every tenant, and a row per
resolved message is the churn mika#2131 bounds.

- `gateway_locale_resolved` (INFO — `chat_id`, `locale`, `locale_source`).
  Emitted on **command** paths only (`/start`, `/unlink`, `/unlink confirm`);
  the `Text` path, which carries the volume, emits nothing. This is the answer to
  "does this tenant get French, and through which gate?" without reading the
  database (`llm_budget_resolved`'s lesson, mika#2293: *a setting you cannot
  observe is not a setting*). `locale_source: default` is the **floor**, not a
  gate — it covers an absent sender, an absent or unrecognized tag, **and an
  explicit `en`**, so it does not by itself prove Telegram sent nothing.
- `unlink_suffix_unrecognized` (INFO — `chat_id`, `locale`). **Expected regime:
  NON-empty.** It measures whether the `confirmer` alias covers the forms people
  actually type. **The refused suffix is never logged** — it is user content, at
  the standard mika#2126 set and mika#2291 restated; it is quoted to the *user*,
  not to the operator.

**Post-deploy probe, and its three halts.** Replay on a francophone tenant:
`/start` with an invalid invite, then `/unlink`, then `/unlink confirmer`.
Expected: three French replies, the action on the first line of the second, the
third releasing the binding.

- **Halt 1 — replies stay English.** Read `gateway_locale_resolved` **before
  touching the normalization**: `locale_source: "default"` means no gate selected
  for that client, which is the population the deferred column would cover — a
  result, not a fault. *No line at all* means the deployed binary predates the
  fix: establish the deployment first (class mika#2340).
- **Halt 2 — the user reads raw backticks.** Check `MIKA_TELEGRAM_HTML_RENDER`
  and `telegram_html_render_fallback` **before removing the markdown from the
  copy**: that is the named degradation, and if the fallback is firing it is
  mika#2291 that has something to say, not this copy.
- **Halt 3 — `unlink_suffix_unrecognized` is empty.** Do not conclude the third
  state was imaginary: the event only fires on a *suffix*, and its absence may
  simply mean nobody typed one. Confirm at least one `/unlink` was served before
  concluding anything (mika#2205).

## Search Substrate (mika#1807 / mika#1971 → mika#2407)

**This section is the documentation debt mika#2407 paid on the way.** Until it was
written, this file — the documentation of the component that carries these
variables — contained zero occurrences of "brave" or "search upstream", and the
only place `MIKA_SEARCH_UPSTREAM` was described at all was
`docs/egress-search-searxng-contingency.md`, a contingency note about a
hypothetical replacement upstream. **The variable that decides activation was
documented only in a document about its successor.**

Since mika#1971 every agent's `web_search` routes through here: the builtin no
longer reads a key of its own, it POSTs `/internal/search`. Three variables:

| variable | effect |
|---|---|
| `MIKA_SEARCH_UPSTREAM` | The selector. `brave` is the only recognized value; an unrecognized one refuses startup. **Absent ⇒ `search_egress_client = None` ⇒ `POST /internal/search` answers `404 search_upstream_not_configured`, whatever the key is worth.** |
| `MIKA_BRAVE_API_KEY` | The key. Required when the selector is `brave` — enforced at startup since mika#1807. |
| `MIKA_SEARCH_REQUIRED` | mika#2407. Declares that this deployment **expects** search; declared and unresolved ⇒ the gateway refuses to start. |

**The validation was asymmetric, and the silent half is the one that fired.**
`GatewaySettings::validate` has always hard-failed on a selector that *lies*
(`='zorglub'`) and on a selector without its key (`='brave'` alone). It said
nothing about a selector that is *missing* — including the shape where a key is
present and the selector is not, which is a half-configuration whose form states
the intent, since nobody posts a search API key by accident. That fourth row is
the measured state of 2026-09-18.

**The founding incident.** The rotation to image `main-e1342dfa` (2026-09-18,
~14:00) moved search behind the gateway; the gateway secret carried neither
variable. Six tenants lost web search for about twenty hours. **Nothing was
red**: `handle_readiness` tests `state.ready` and a `SELECT 1`, `ready.store(true)`
follows boot unconditionally, and `/health`, `/readyz` and `/livez` share that
handler — so the deployment control that had the charge of catching this was
green. The tenants' agents, handed a 404 and a neutral substrate fallback
(doctrine mika#1783, which forbids operator tokens like "api key" or
"configuration" at the family tier), paraphrased it as *« l'outil de recherche web
a un souci de configuration côté serveur (clé API manquante) »* — wrong,
unactionable, and a faithful reading of what the substrate returned.

**Two stages, and they are not redundant.** The guard reads what the pod
*believes*; the smoke reads what the pod *does*.

1. **Startup guard** (`assert_search_substrate_expectation`, `settings.rs`).
   Structural: the pod does not start, so the Kubernetes rollout fails on its
   own — no scheduler to wire, no upstream request spent. **It reads
   configuration and never the network** (KTD3): probing the upstream at boot
   would let a crash-looping pod spend the shared monthly quota it exists to
   protect, and would make an upstream outage enough to keep Telegram, GitHub
   webhooks and A2A from starting at all.
2. **External smoke** (`scripts/smoke-search-substrate`). One real search, from
   outside, reading no variable of the pod. This is what covers the objection
   the guard cannot answer: *a rotation that dropped `MIKA_SEARCH_UPSTREAM`
   could equally have dropped `MIKA_SEARCH_REQUIRED`* — and a declaration nobody
   made guards nothing. Exit `0` healthy / `1` substrate broken (404
   `search_upstream_not_configured`, or 502 `unauthorized` = key present and
   refused) / `2` **nothing verified**. The third is not a pass: a smoke that
   cannot authenticate has checked nothing, and `/internal/search` sits behind
   `require_bearer_token`, the same middleware as `/send`, so the caller needs
   `MIKA_INTERNAL_TOKEN` — which the deployment pipeline already holds.

**`/readyz` deliberately does NOT go red without search.** Taking a gateway that
routes Telegram, GitHub webhooks and A2A out of service because web search is
absent would withdraw a component healthy at 95 %. The startup guard is
acceptable precisely because it is **conditioned on an explicit declaration**:
an operator who sets `MIKA_SEARCH_REQUIRED=1` is asking for exactly that
behaviour. The difference is not one of degree — only one of the two was asked
for.

**Operator surfaces.** `search_upstream_resolved` (INFO, once at startup, **on
every branch, healthy one included**): `upstream`, `upstream_source`,
`api_key_present` (**boolean only** — never the value, never a prefix, never a
length; Q4 STRIP TOTAL extends to the construction site), `required`,
`required_source`, `endpoint_is_default`. Its **absence** while the gateway runs
means the deployed binary predates the fix (class mika#2340) and never "search is
fine". `search_upstream_key_without_selector` (WARN) — the half-configuration;
the repairing gesture is the opposite of the obvious one, add the **selector**.
`search_required_unrecognized_value` (WARN) — names the value between quotes so a
stray space is visible (mika#2220).

**Post-rotation probe, with its halts.**

```bash
grep search_upstream_resolved <gateway-log> | jq '{upstream, upstream_source, api_key_present, required}'
scripts/smoke-search-substrate https://<gateway> ; echo "exit=$?"
```

- `upstream: "brave"` + `exit=0` → healthy, the expected regime.
- `upstream: "none"` with `api_key_present: true` → half-configuration. **Halt:
  do not add another key** — the remedy is `MIKA_SEARCH_UPSTREAM=brave`.
- `exit=1` on `unauthorized` while `api_key_present: true` → the key is there and
  refused: a key rotation, not a code fix.
- `exit=2` → **this is not a green.** The smoke verified nothing (token,
  network); establish why before concluding anything about the substrate.
- `search_upstream_resolved` **absent** from the log while the gateway runs →
  the deployed binary predates the fix. **Halt: establish the deployment before
  touching code.**

**Out of scope, deliberately.** The 429 backoff/retry on the shared free-tier
rate limit (1 req/s) — split to p2 by the operator on mika#2407; it was a real
hardening need and was not the 2026-09-18 failure. `values.yaml`, the
`setup-*.sh` scripts and the ordering of the smoke inside the rotation pipeline
live in `mika-cloud`, outside this workspace — follow-up ticket.

## A2A Auth

API keys are SHA-256 hashed and stored in Postgres `a2a_api_keys` table (migration 003); validated via `validate_a2a_api_key()` with expiry and revocation checks. See `crates/mika-a2a/CLAUDE.md` for A2A protocol details.

## Request Logging

`tower_http::trace::TraceLayer` middleware logs method, path, status code, and latency for every request. `inject_request_meta` middleware (inner to TraceLayer) copies method+path from request into response extensions so `on_response` emits them as top-level JSON event fields (not just nested in the `spans` array). Health probe paths (`/health`, `/readyz`, `/livez`, `/version`) are logged at DEBUG level to reduce noise from Kubernetes checks; all other routes log at INFO level. 5xx responses are logged at WARN. Connection-level failures (timeouts, stream errors) are logged at ERROR with classification.

## Postgres Migrations

- Migration 002: creates `outbound_messages` table
- Migration 003: creates `a2a_api_keys` table
- Migration 004: creates `github_repos` table (maps `repo_full_name` -> `customer_id` for multi-tenant GitHub webhook routing)
- Migration 005: adds `agent_mapping JSONB NOT NULL DEFAULT '{}'` to `github_repos` for per-repo agent name overrides (keys are default agent names from `route_event()`, values are customer's replacement names; `apply_agent_mapping()` validates names via `is_valid_agent_name()` and falls back to defaults for invalid values)
- Migration 006: creates `webhook_deliveries` table for dead-letter queue (delivery_id PK, event_type, target_agent, repo_full_name, payload, request_id, status CHECK IN pending/delivered/dead, attempts, last_attempt_at, last_error). Partial indexes on `(status, last_attempt_at) WHERE status='pending'` and `(created_at DESC) WHERE status='dead'`.
- Migration 007: creates `orchestrator_inbox_messages` table for the orchestrator inbox v2 channel (mika#1189). Columns: `id BIGSERIAL PK`, `orchestrator_id TEXT NOT NULL`, `spawn_id TEXT` (nullable), `kind TEXT NOT NULL CHECK IN ('handoff','update','ack')`, `body JSONB NOT NULL`, `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`, `delivered_at TIMESTAMPTZ`. Indexes: `(orchestrator_id, id)` for cursor replay; partial `(orchestrator_id, created_at) WHERE delivered_at IS NULL` for diagnostic queries on undelivered rows (retention sweeps purge by `created_at` alone and don't use this partial index — see migration comment).
- Migration 008: adds `bot_token`, `bot_username`, `webhook_secret` columns to `customers` table (per-customer Telegram bot support, mika#1454). All nullable — NULL means single-bot mode fallback.

## build.rs

- `cargo::rerun-if-changed=migrations` so new migration files invalidate the incremental compilation cache (SQLx `migrate!()` is a compile-time proc macro)
- Captures short git hash via `git rev-parse --short HEAD` into `GIT_HASH` env var for the `/version` endpoint (falls back to `"unknown"` when `.git` is absent); watches `.git/HEAD` and `.git/refs` for rebuild on new commits

## Gateway Environment Variables

- `MIKA_DATABASE_URL` — Postgres connection string
- `MIKA_TELEGRAM_BOT_TOKEN` — Telegram Bot API token. When configured, the gateway builds the global `TelegramClient` for outbound delivery via `/send` (operator agents without `customer_id`), independent of `MIKA_TELEGRAM_SINGLE_BOT_MODE`. Required in single-bot mode; optional but enables outbound-only delivery in per-customer mode (mika#1590).
- `MIKA_TELEGRAM_WEBHOOK_SECRET` — 64-char hex secret for inbound webhook validation. Required only in single-bot mode (inbound registration).
- `MIKA_TELEGRAM_WEBHOOK_URL` — Public HTTPS URL for inbound Telegram webhook delivery. Required only in single-bot mode (inbound registration).
- `MIKA_TELEGRAM_SINGLE_BOT_MODE` — Exclusively controls **inbound global webhook registration**. When `1` or `true`, the gateway registers the global webhook with Telegram for inbound messages (requires all three: `MIKA_TELEGRAM_BOT_TOKEN`, `MIKA_TELEGRAM_WEBHOOK_SECRET`, `MIKA_TELEGRAM_WEBHOOK_URL`). Default: off (per-customer inbound mode). **Semantic narrowing (mika#1590):** pre-fix, this flag gated both inbound webhook registration and outbound client construction; post-fix, it gates inbound registration only — the global outbound client is built whenever `MIKA_TELEGRAM_BOT_TOKEN` is configured.
- `MIKA_TELEGRAM_HTML_RENDER` — Outbound Telegram rendering kill-switch (mika#2291). **Default: armed** (`parse_mode=HTML`, with a one-shot plain-text fallback on 400). `0` / `false` / `off` / `no` disarm; absent, empty, or unrecognized stays **armed** with a WARN naming the value. `Option<String>`, never `bool` — a `bool` would make a typo a hard `load()` error and stop the gateway from booting. Read once per process, **not hot-swappable**: set it before startup. See § *Outbound Text Rendering*.
- `MIKA_INTERNAL_TOKEN` — Shared 64-char hex bearer token
- `MIKA_GATEWAY_ADMIN_READ_TOKEN` — Admin **read-only** bearer token (mika#2360). Opens `GET /admin/tenants/{customer_id}/recurring-tasks` and nothing else. Optional: when absent the route answers 404 (an INFO line at startup says so). Must be **distinct** from `MIKA_INTERNAL_TOKEN` — an equal value disarms the route with a WARN rather than silently voiding the read/write segregation. Never fails startup: a malformed value disarms the route, it does not take the gateway down.
- `MIKA_AGENTS_NAMESPACE` — K8s namespace where agent pods run (default: `mika-agents`). Used for FQDN construction in cross-namespace DNS resolution (`http://mika-{id}.{ns}.svc.cluster.local:8080`). Override for environment-scoped namespaces (e.g. `mika-agents-prd`).
- `MIKA_GITHUB_WEBHOOK_SECRET` — Secret for validating inbound GitHub App webhooks via HMAC-SHA256. Arbitrary string (not hex-constrained like Telegram). When absent, `POST /webhook/github` returns 404.
- `MIKA_GITHUB_APP_ID` — GitHub App ID (u64). Required for the synchronize no-diff guard (#886).
- `MIKA_GITHUB_APP_PRIVATE_KEY` — GitHub App private key (base64-encoded PEM). Required for the synchronize no-diff guard (#886). Encode with: `base64 -w0 < your-app.pem`.
- `MIKA_GITHUB_APP_INSTALLATION_ID` — GitHub App installation ID (u64). Required for the synchronize no-diff guard (#886). All 3 GitHub App vars must be set; when incomplete, the no-diff guard is disabled (fail-open).
- `MIKA_GATEWAY_EXTERNAL_URL` — Public HTTPS base URL of the gateway (e.g., `https://gateway.mika.example.com`). Required for per-customer webhook registration via `POST /admin/customers`. The endpoint constructs `{gateway_external_url}/webhook/telegram/{customer_id}` as the Telegram webhook URL.
- `MIKA_SEARCH_UPSTREAM` — Search-substrate selector (mika#1807). `brave` is the only recognized value in v1; an unrecognized one refuses startup. **Absent ⇒ `POST /internal/search` answers `404 search_upstream_not_configured` whatever `MIKA_BRAVE_API_KEY` is worth** — the asymmetry mika#2407 was filed for. See § *Search Substrate*.
- `MIKA_BRAVE_API_KEY` — Search upstream API key, **on the gateway** and not on the agent (since mika#1971 the agent's `web_search` POSTs `/internal/search`). Required when `MIKA_SEARCH_UPSTREAM=brave`, enforced at startup.
- `MIKA_SEARCH_REQUIRED` — Declares this deployment **expects** the search substrate (mika#2407). Declared and unresolved ⇒ the gateway refuses to start, so the rollout fails instead of serving mute tenants. Absent or empty ⇒ not required; an unrecognized value ⇒ **required**, with a WARN naming it. `Option<String>`, never `bool`, for the same F8 reason as `MIKA_TELEGRAM_HTML_RENDER`.
- `MIKA_BRAVE_ENDPOINT` — Optional upstream endpoint override, for integration tests and self-hosted mirrors. Reported as `endpoint_is_default` on `search_upstream_resolved`.
- `MIKA_ORCHESTRATOR_INBOX_ENABLED` — Orchestrator inbox feature flag (mika#1189). Default off — `/orchestrator/inbox/*` endpoints return 404. Set `1` (or `true`, case-insensitive) to enable dual-write with the mika-platform#100 filesystem inbox. `2` (gateway-only cutover) is reserved for a future ticket and currently treated as disabled. Note: bearer auth gates the endpoints by token; orchestrator/spawn distinction is carried by path (`orchestrator_id`) and `spawn_id` field, not by separate tokens — multi-operator deployments will need per-operator scoping before exposure beyond a solo operator.

## Orchestrator Inbox (mika#1189)

Real-time spawn→orchestrator coordination channel. Spawned Claude Code tenants POST handoff messages addressed to their parent orchestrator's id; the orchestrator subscribes via SSE and surfaces messages in-session. Replaces the operator-mediated paste cycle of mika-platform#100 (filesystem-inbox protocol remains operational during dual-write migration).

- Persistence: Postgres table `orchestrator_inbox_messages` (migration 007). `id BIGSERIAL` is the SSE cursor — clients reconnect with `Last-Event-Id: <last-seen>` and the server replays rows with `id > cursor`. `delivered_at` is observational; cursor replay is authoritative.
- Wire shape: POST returns `201 {"id": <bigserial>}`. SSE events emit `id: <n>` + `event: message` + `data: <JSON envelope>`. KeepAlive pings every 30s. Long-poll loop reads rows every `ORCHESTRATOR_INBOX_POLL_INTERVAL` (1.5s by default).
- Retention: background tokio task (spawned in `main.rs`) purges rows older than `ORCHESTRATOR_INBOX_RETENTION_DAYS` (7 days) once an hour. Constants live at the top of `orchestrator_inbox.rs` — code-edit-tunable per plan NF3.
- Validation: `orchestrator_id` path segment must be 1-128 chars `[A-Za-z0-9_-]`; `kind` must be one of `handoff`, `update`, `ack` (mirrors the migration CHECK constraint).
- Structured log events: `orchestrator_inbox_message_received`, `orchestrator_inbox_subscriber_connected`, `orchestrator_inbox_retention_purged`.
