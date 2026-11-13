# mika-a2a — Agent-to-Agent Protocol

A2A (Agent-to-Agent) protocol v0.3 implementation for inter-agent communication.

## Architecture

- **Protocol:** JSON-RPC types, task state machine, SSE streaming, A2A client
- **State machine:** submitted -> working -> completed/failed/canceled. Validates transitions.
- **SSE streaming** for `message/send` responses
- **`a2a_call` builtin tool** (120s timeout) sends messages to remote A2A agents

## Client transport policy (mika#2036)

`A2aClient` carries an explicit budget: `DEFAULT_TIMEOUT` = **600 s**, aligned
with the engine total budget (`MIKA_AGENT_TOTAL_TIMEOUT_SECS`) and overridable
via `MIKA_A2A_TIMEOUT_SECS`; it must never sit below the engine total, or the
client abandons a generation the engine is still within its deadline to finish
(mika#2297; before it, 300 s),
not `reqwest`'s default of *no timeout at all*. `A2aClient::new` keeps its
signature and gains it; `with_timeout` overrides it, and `RECOVERY_TIMEOUT`
(30 s) bounds a recovery read. `timeout()` reports what the client actually
enforces, so an error can name the interval it really spent.

**The policy is guarded, not merely written down (mika#2309).** CI job
`a2a-timeout-literal-lint` runs `scripts/check-a2a-timeout-literals.sh` over a
declared perimeter (this crate's `src/`, the two `A2aClient` call sites,
`mika-cli/src/budget_floor.rs` — the send-floor attestation, added by mika#2759 —
and `mika-common/src/llm/`): a budget at a bounding site may not be a *literal*
(`from_secs(600)` and `from_secs(10 * 60)` alike), and a `const … : Duration`
≥ 60 s must be a `DEFAULT_*` backed by a declared `MIKA_*` env var. The predicate
is on the **site that bounds**, never on the value — a TTL or a JWT lifetime is
not a call budget. `crates/mika-agent/tests/a2a_budget_transitivity_2309.rs`
asserts the full chain `client ≥ total > http` by calling the production
resolvers; `>` is strict because the code refuses `total == http`.

**The floor covers the PER-AGENT envelope, and it is held at send time
(mika#2759).** `resolve_timeout_secs` reads the process environment; the
envelopes of mika#2189 live in the agent's **`config.toml`** (mika-arch:
`agent_total_timeout_secs = 900`). The two never met, so a call to mika-arch
resolved **600 s** against a **900 s** envelope and abandoned a generation the
engine still had the right to finish — the 2026-09-11 incident that motivated
300 → 600, reproduced one caller away.

`resolve_send_timeout_with_floor(agent_envelope_secs: Option<u64>)` takes the
envelope as a **third candidate to the same `max`**. Four properties, and each
undoes itself if guessed:

- **At send time, not at construction, because the agent is only knowable from
  the URL.** `A2aClient::new` builds its `reqwest::Client` before the URL is
  decomposed, so no constructor signature could carry the answer. The single
  call site is `mika-cli/src/remote_ask.rs`, held by a source scan whose
  allowlist ships **empty** (`mika2759_le_plancher_a_un_seul_site`): a second
  site would be a second place deriving an agent from a URL, and one of the two
  would drift back into `A2aClient::new`.
- **The envelope comes from spirit's attestation, never resolved locally**
  (`GET /api/v1/agents/{id}/budget`, field `budget.agent_total_timeout_secs`).
  Doctrine mika#2457: *spirit attests, the CLI renders; never the reverse* — a
  CLI reading an agent's `config.toml` itself would print a setting that is not
  the one in force, which is mika#2304's defect one field over.
- **Loopback only**, and that predicate is the inversion of `a2a_call`'s: that
  one *refuses* loopback (SSRF), this one accepts **nothing else**. The target's
  own spirit is the only authority on that target's envelope, and a remote URL's
  attestation is neither reachable nor ours to trust.
- **Absent is not zero** (motif mika#2331). `None` — not a local target,
  unreachable, a non-2xx, a body this version cannot read — makes
  `resolve_send_timeout_with_floor(None)` return, byte for byte, the budget
  `A2aClient::new` would have resolved. The degradation tree is **named** rather
  than discovered: `a2a_floor_not_local` (DEBUG), `a2a_floor_unattested` (WARN,
  with `cause` ∈ `unreachable` | `status_<code>` | `malformed`),
  `a2a_floor_resolved` (DEBUG). Details in `crates/mika-cli/CLAUDE.md`.

`crates/mika-agent/tests/a2a_budget_transitivity_2309.rs` **keeps its declared
perimeter and its three assertions byte for byte** — defaults plus the process
env is still exactly what it covers. Only its `//!` prose moved, because it named
the positive control mika#2759 removed (`mika2309_client_default_is_below_the_arch_envelope`,
retired with its cause per mika#1574) and asserted a divergence that no longer
exists. Its successor on the same site is
`mika2759_le_plancher_couvre_lenveloppe_de_mika_arch`, which pins the `max`
property rather than the number: no envelope value can redden it alone.

`TransportFailure` (in `error`) classifies a `reqwest` failure into unreachable /
timed out / HTTP status / unreadable / interrupted. Its load-bearing question is
`request_was_sent()`: only an **unreachable** server proves no work exists. Every
other failure happened after the bytes left, so an answer may be sitting on the
other side.

**Recovering a lost answer.** `message/send` persists the finished task *before*
serializing the HTTP response, so a response lost in transit names work that is
already on disk. The task id cannot help — the server mints it with
`Uuid::new_v4` and it travels back only in the envelope that was lost. The
caller-supplied `context_id` can: it is persisted on `a2a_task_map`. So
`tasks/get` accepts a `context_id` as its `id`, resolved **after** the task-id
lookup misses — the order means a real task id is never shadowed. `get_task`
returns `Ok(None)` for `TASK_NOT_FOUND`, which is an answer ("nothing was created
under that name"), not a failure.

## A2A Persistence (Orthogonal)

A2A tasks use `trigger_type='a2a'` in the `tasks` table with orthogonal persistence:
- `a2a_task_map` maps A2A task IDs to internal task/session IDs
- `a2a_artifacts` stores A2A-specific artifact data
- `a2a_push_notification_configs` stores push notification configurations

## Server Integration

- **mika-spirit** exposes `/a2a/{agent_name}` (JSON-RPC POST, 2MB limit) and `/a2a/{agent_name}/agent.json` (Agent Card GET), both internal-token auth. See `crates/mika-agent/CLAUDE.md` for server endpoint details.
- **mika-gateway** proxies at `/a2a/{customer_id}/{agent_name}` with API key auth (SHA-256 hashed keys in Postgres `a2a_api_keys` table, migration 003). See `crates/mika-gateway/CLAUDE.md` for gateway details.

## Convention

The A2A Protocol convention documented in root CLAUDE.md: `mika-spirit` exposes endpoints, gateway proxies with API key auth. State machine validates transitions. SSE streaming for `message/send`.
