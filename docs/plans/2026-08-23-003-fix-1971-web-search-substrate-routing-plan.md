---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
ticket: mika#1971
branch: fix/1971/agent-egress-web-search-bypasse
created: 2026-08-23
depth: standard
---

# fix(agent,egress): route `web_search` builtin through mika-gateway substrate

> - **Ticket:** [mika#1971](https://github.com/senara-solutions/mika/issues/1971)
> - **Branch:** `fix/1971/agent-egress-web-search-bypasse`
> - **Plan:** `docs/plans/2026-08-23-003-fix-1971-web-search-substrate-routing-plan.md`
> - **Second-pass:** _(architect review runs in `/ce:review` — this plan is authored by orchestrator-CC, not by `mika-arch-groom-ticket`)_

## Summary

Wire the `web_search` builtin (`crates/mika-agent/src/skills/builtin_handlers.rs:173-243`) to transit via `POST /internal/search` on the mika-gateway substrate, closing the E1-E4 (mika#1806) single-egress invariant end-to-end. The substrate has been shipped and load-tested since 2026-08-18 — this ticket is the last consumer flip. Delete the direct `reqwest::Client` (`HTTP_CLIENT` at `builtin_handlers.rs:56-61`) and its GET to `https://api.search.brave.com/res/v1/web/search`. Thread the gateway URL and internal token into `ToolContext` alongside the existing `brave_api_key` slot, and add a wiremock-backed regression test that asserts (a) the wire body reaches `POST /internal/search` with `{query, max_results}`, (b) an `Authorization: Bearer <token>` header is present, (c) no `X-Subscription-Token` header is present, (d) the upstream Brave URL is never contacted from `mika-agent`.

**Direct switch, no env toggle.** The substrate is already in production; an opt-in `MIKA_WEB_SEARCH_VIA_SUBSTRATE=1` toggle would leak a transition state into a permanent config surface. Rollback = revert this single PR.

## Problem Frame — WHY

mika#1806 (E1-E4 shipped) established the single-egress discipline via `SearchEgressClient`. The module docstring at `crates/mika-gateway/src/egress_search/mod.rs:6-8` declares the end-state: « All mika-spirit agents call this substrate via POST /internal/search — never the upstream API directly. » That contract is not currently held: the builtin `web_search` still opens its own `reqwest::Client` and calls `https://api.search.brave.com/res/v1/web/search` from inside mika-spirit. The invariant fails silently — nothing throws — until a sibling ticket (`fetch_url via egress_fetch`) tries to compose on the assumption of a single egress path.

**Evidence, hand-verified:**
- `crates/mika-agent/src/skills/builtin_handlers.rs:56-61` — `static HTTP_CLIENT: LazyLock<reqwest::Client>` with a 15s timeout. `grep -n HTTP_CLIENT` in the file returns exactly two hits: the definition (`:56`) and the sole usage inside `web_search` (`:196`). No other builtin uses it.
- `crates/mika-agent/src/skills/builtin_handlers.rs:173-243` — `web_search()` reads `ctx.brave_api_key`, calls `HTTP_CLIENT.get(...)` with `X-Subscription-Token` header + `q`/`count` query params, then parses the raw Brave JSON via `format_brave_results()` (walks `body["web"]["results"][*]`).
- `crates/mika-gateway/src/egress_search/mod.rs` — substrate contract verified: `POST /internal/search`, bearer auth via `require_bearer_token` middleware (`routes.rs:258`), request body `SearchRequest{query,max_results:usize=5}`, 200 response `SearchResponse{results:Vec<{title,url,snippet}>, upstream_latency_ms:u32}`, 404 unconfigured, 502 with taxonomy label on errors.
- `crates/mika-common/src/config.rs:787,799` — `Settings.routing_url: Option<String>` and `Settings.internal_token: Option<SecretString>` are already populated at spirit startup; no new config plumbing needed at the `Settings` layer.

**Consequences of the gap:**
1. Q1 (placement), Q2 (isolation), Q3 (partagé no-log), Q4 (STRIP TOTAL) discipline is asserted at the substrate but bypassed at the caller — a Q4 violation is possible from `web_search` today because `ctx.brave_api_key` reaches the wire.
2. The sibling `fetch_url` ticket is blocked on this cleanup; it would inherit the same bypass shape by copy-paste.
3. The CI lint `scripts/verify-egress-uniqueness.sh` was authored to gate mika-gateway files; extending it to mika-agent would immediately fail today until this ticket lands (see § Deferred to Follow-Up Work).

## Requirements

- **R1** — Every runtime path where `web_search` returns results MUST reach the network via `POST http://<gateway>/internal/search`, not via `GET https://api.search.brave.com/...`. (Covers AC1, AC4.)
- **R2** — The direct `HTTP_CLIENT` static and the `reqwest::Client` construction MUST be removed from `crates/mika-agent/src/skills/builtin_handlers.rs`. Since `web_search` is the sole consumer today (`grep -c HTTP_CLIENT builtin_handlers.rs` = 2, both in the definition and its one usage), full removal is the intent — no dead code left behind. (Covers AC2.)
- **R3** — The wire body sent by `web_search` MUST NOT contain any tenant identifier, session id, agent name, chat id, API key, or provider credential. Payload is exactly `{"query": "<string>", "max_results": <usize>}`. Auth is a Bearer header set by the substrate transport layer, never a body field. (Covers AC3.)
- **R4** — A regression test MUST assert, deterministically and offline: (a) `web_search` sends a `POST /internal/search`; (b) the request body deserializes to the substrate's `SearchRequest` shape; (c) the `Authorization: Bearer <MIKA_INTERNAL_TOKEN>` header is present; (d) no `X-Subscription-Token` header is present; (e) the upstream Brave URL is not contacted. (Covers AC4.)
- **R5** — Substrate-configured error responses (404 unconfigured, 502 with taxonomy label) MUST map to LLM-facing error strings that are stable, non-leaky, and shape-preserved with respect to the current handler's error taxonomy (invalid-key / rate-limit / generic). Substrate-native errors (`unauthorized` = substrate bearer auth failed; `upstream_error` = Brave-side; `transport_error` = network-level; `parse_error` = schema drift; `not_implemented` = future variant) each get one distinguishable, actionable message. (Covers AC3, AC4 negative-path.)
- **R6** — The PR body MUST document the migration path (no operator action; visible surface unchanged) and rollback plan (revert single commit). (Covers AC5.)

## Key Technical Decisions

### KTD1 — Direct switch, no env toggle _(user-approved: chosen over `MIKA_WEB_SEARCH_VIA_SUBSTRATE=1` gate — the substrate has been live since E4 2026-08-18 with adversarial no-log test coverage; a rollout toggle would leak a transition state into a permanent config surface. Rollback = revert this PR.)_

Every runtime path flips at once. Covers R1, R2, R6. Governs U1, U2, U5.

### KTD2 — Extend `ToolContext` with `gateway_url: Option<&'a str>` and `internal_token: Option<&'a str>`, populated from `Settings.routing_url` and `Settings.internal_token`

The alternative — reading `std::env::var` inside `web_search` — is hostile to test isolation and diverges from the established codebase pattern (every other tool receives config via `ToolContext`). Both fields are `Option<&'a str>` for symmetry with the existing `brave_api_key` slot. `internal_token` is threaded as `&str` (not `&SecretString`) because the value is scoped to a single-turn borrow that never crosses an `.await` point outside the substrate HTTP call, and constructing a bearer header requires plaintext exposure anyway; production callers use `Settings::internal_token.as_ref().map(|t| t.expose_secret())` at the ToolContext construction site. Covers R1. Governs U2, U3.

**Fail-mode when either field is `None`:** return `ToolOutput::error("Search substrate not configured (gateway_url / internal_token missing). Ensure MIKA_ROUTING_URL and MIKA_INTERNAL_TOKEN are set on mika-spirit.")` — matches the shape of the existing missing-brave-key error at `builtin_handlers.rs:186-193`.

### KTD3 — Retain `ToolContext.brave_api_key` as a slot; drop only the `web_search` read of it. Follow-up ticket removes the field.

The field is threaded through ~15 construction sites (test_utils.rs ×5, teams/engine.rs ×2, server/{a2a,handlers,mod,investigate}.rs, tools/pr_merge_with_gate.rs test, task_engine/engine.rs ×2). Removing the field in this PR would balloon the diff and mix a "route through substrate" change with a "drop dead field" change — two different review lenses, one being trust-critical, the other being cleanup. Keep this PR narrow. Emit a WARN once at spirit startup if `settings.brave_api_key.is_some()` while `settings.routing_url` and `settings.internal_token` are also set, to guide operators to move the key into the gateway's config. Governs U6 (follow-up).

### KTD4 — Format substrate results with a purpose-built `format_substrate_results()`; delete `format_brave_results()`

The substrate returns `SearchResponse{results: Vec<{title,url,snippet}>, upstream_latency_ms: u32}` — already flattened, already stripped of Brave-specific envelope fields. Reusing `format_brave_results()` would require synthesizing a fake Brave-shaped `serde_json::Value` just to feed the walker, which is precisely the field-name coupling we're paying to remove. Building a new formatter against the substrate's typed `SearchResponse` (deserialized via `serde_json::from_slice::<SearchResponseWire>` — where `SearchResponseWire` is a local mirror of the substrate's public `SearchResponse` shape to avoid a cross-crate `pub(crate)` visibility break) is a small, isolated function. Governs U1.

### KTD5 — Regression test uses `wiremock` (workspace dev-dep, already used by `mika-gateway` egress-search tests)

`wiremock = "0.6"` is registered in the workspace root `Cargo.toml` — used by three existing mika-gateway test files (`egress_search/brave.rs`, `tests_e3_request_shape.rs`, `tests_e4_no_log.rs`). Adding it to `crates/mika-agent/Cargo.toml [dev-dependencies]` with `wiremock.workspace = true` is a one-line change. The test spins up an ephemeral `wiremock::MockServer`, sets `.expect(0)` on any request path other than `POST /internal/search`, and asserts on captured headers + body. Covers R4. Governs U4.

### KTD6 — Substrate error → LLM-facing message mapping

Substrate returns a typed error JSON `{"error": "<taxonomy_label>"}` at HTTP 404 (`search_upstream_not_configured`) or 502 (five taxonomy labels defined in `crates/mika-gateway/src/egress_search/mod.rs:203-211`). Deserialize the response body into a local wire enum, then map:

| Substrate HTTP + label | LLM-facing message |
|---|---|
| 404 `search_upstream_not_configured` | `"Search substrate is not configured on the gateway. Ask the operator to set MIKA_BRAVE_API_KEY on mika-gateway."` |
| 502 `not_implemented` | `"Search upstream variant not implemented."` (should not happen with Brave-only wiring today; kept for taxonomy stability) |
| 502 `upstream_error` | `"Search upstream returned an error. Try again in a moment."` |
| 502 `unauthorized` | `"Search substrate rejected upstream credentials. Ask the operator to rotate MIKA_BRAVE_API_KEY on mika-gateway."` |
| 502 `transport_error` | `"Search request failed (transport error contacting upstream). Try again in a moment."` |
| 502 `parse_error` | `"Search substrate could not parse the upstream response (possible schema drift). Escalate."` |
| Any other (defensive) | `format!("Search substrate returned HTTP {status}.")` |
| Transport error to gateway | `"Search substrate unreachable (transport error to gateway). Escalate."` |
| Body-read timeout / size limit | `"Search substrate response could not be read."` |

Covers R5. Governs U1.

## High-Level Technical Design

Before (invariant violation):

```
+----------------+       +-------------------+
| mika-agent     |       | api.search.brave  |
| web_search()   |----->| .com/res/v1/web   |
| HTTP_CLIENT    |  ✗   | /search           |
| (X-Sub-Token)  |       |                   |
+----------------+       +-------------------+
        ✗ bypasses mika-gateway/SearchEgressClient
        ✗ tenant identifier exposure risk (via ctx.brave_api_key)
        ✗ Q4 STRIP TOTAL not enforced at this codepath
```

After (single-egress invariant held):

```
+----------------+   POST     +-----------------------+   GET     +-------------------+
| mika-agent     | /internal/ | mika-gateway          |  X-Sub-   | api.search.brave  |
| web_search()   | search     | SearchEgressClient    |  Token    | .com              |
| (Bearer INT)   |----------->| (brave::execute...)   |---------->|                   |
| body:{q,max}   |            | Q4 log emit only      |           |                   |
+----------------+            +-----------------------+           +-------------------+
        ✓ substrate is the sole egress path
        ✓ no tenant field on the wire (body = {query, max_results})
        ✓ Q1-Q4 invariants enforced substrate-side, hand-verified caller-side
```

## Output Structure

```
crates/mika-agent/
├── Cargo.toml                                   # + wiremock dev-dep (workspace)
├── src/
│   ├── skills/
│   │   └── builtin_handlers.rs                  # M: web_search rewrite; remove HTTP_CLIENT + format_brave_results; add tests
│   ├── tools/
│   │   └── mod.rs                               # M: extend ToolContext with gateway_url + internal_token
│   ├── server/
│   │   ├── mod.rs                               # M: thread routing_url + internal_token into AgentState/ToolContext construction
│   │   ├── handlers.rs                          # M: pass new fields into ToolContext (POST /message path)
│   │   ├── a2a.rs                               # M: pass new fields into ToolContext (A2A path)
│   │   └── investigate.rs                       # M: pass new fields into ToolContext (investigate path)
│   ├── teams/
│   │   └── engine.rs                            # M: thread new fields through team member ToolContext (2 construction sites)
│   ├── task_engine/
│   │   └── engine.rs                            # M: pass None into 2 test-only ToolContext constructions
│   ├── test_utils.rs                            # M: 5 ToolContext test constructions (all None)
│   └── tools/
│       └── pr_merge_with_gate.rs                # M: 1 test ToolContext construction (None)
```

Per-unit `**Files:**` sections are authoritative. This tree is a scope declaration only — the implementer may adjust if a better layout is discovered.

## Implementation Units

### U1. Rewrite `web_search` to POST `/internal/search`; add `format_substrate_results`; delete `format_brave_results` and `HTTP_CLIENT`

- **Goal:** Replace the direct-to-Brave codepath with a substrate-routed one. Delete the static `HTTP_CLIENT` (sole consumer removed) and `format_brave_results` (replaced by `format_substrate_results`). Read `gateway_url` and `internal_token` from `ToolContext`, error out cleanly when either is missing.
- **Requirements:** R1, R2, R3, R5. Cites KTD1, KTD2 (fail-mode), KTD4 (formatter cut), KTD6 (error mapping).
- **Dependencies:** U2 (needs the ToolContext fields to exist).
- **Files:**
  - `crates/mika-agent/src/skills/builtin_handlers.rs` — rewrite `web_search` (currently lines 173-243), delete `HTTP_CLIENT` static (currently lines 56-61) and `format_brave_results` (function reference at line 242 — grep for its full definition and remove), add `format_substrate_results` + local `SearchResponseWire` / `SearchResultWire` / `SubstrateErrorBody` deserialization types.
- **Approach:**
  1. Introduce a module-local `reqwest::Client` built at the call site (or a new, purpose-named `LazyLock<reqwest::Client>` named `SUBSTRATE_HTTP_CLIENT` with a 15s timeout — the substrate hard-caps at 5s internally, so 15s is a safe outer bound). A fresh client per call is also acceptable; the frequency of `web_search` calls is bounded by LLM turn cadence, not by request-per-second load.
  2. Compose the URL: `format!("{}/internal/search", ctx.gateway_url)` — strip a trailing `/` from `gateway_url` if present to avoid `//internal/search`.
  3. Send POST with body `serde_json::json!({"query": query, "max_results": 5})` and header `Authorization: Bearer <internal_token>`. Do NOT set `X-Subscription-Token`. Do NOT set `X-Agent-Name`, `X-Tenant-Id`, or any header carrying tenant context.
  4. On non-2xx: deserialize the JSON body into `SubstrateErrorBody { error: String }`. Map per KTD6 to a `ToolOutput::error` with the tabulated message. If body deserialization fails, fall back to `"Search substrate returned HTTP <status>."`.
  5. On 2xx: deserialize into `SearchResponseWire { results: Vec<SearchResultWire>, upstream_latency_ms: u32 }` and hand off to `format_substrate_results(&resp, query)`.
  6. `format_substrate_results` mirrors the current `format_brave_results` output shape (line-per-result: `1. <title>\n   <url>\n   <snippet>\n`) so the LLM-facing text does not change. The `upstream_latency_ms` field is not surfaced in the LLM output (it is Q4-safe side-channel telemetry for the operator, already logged substrate-side).
  7. Preserve the existing input validation: query length ≤ 10_000 chars, empty-query rejection.
  8. Preserve the `MAX_SEARCH_RESPONSE_BYTES = 1 MiB` guard on the response body (defense-in-depth against a hostile gateway).
- **Execution note:** implement U1 test-first — the wiremock regression test in U4 is the load-bearing artifact for R4 and drives the substrate-request shape. Write the test, watch it fail against the old codepath, then flip the handler.
- **Patterns to follow:** the substrate-side POST handler in `crates/mika-gateway/src/egress_search/mod.rs:318-353`. The error-classification pattern in `crates/mika-agent/src/tools/pr_merge_with_gate.rs::classify_gh_error` (a `#[serde(tag = ...)]`-shaped enum consumed at a stable boundary).
- **Test scenarios:** (unit-level, complementary to U4's wire-level integration test)
  - Happy path: mock 2xx body with two results → `format_substrate_results` produces the expected two-block string.
  - Empty query → `ToolOutput::error("Missing or empty 'query' parameter.")` (preserved from current handler).
  - `gateway_url = None` → the "Search substrate not configured..." error (KTD2 fail-mode).
  - `internal_token = None` → same error.
  - 404 `search_upstream_not_configured` → the KTD6 tabulated message.
  - 502 with each of the five taxonomy labels → each mapped message.
  - 502 with an unknown label → generic HTTP fallback message.
  - Body >1 MiB → truncation error (preserves current MAX_SEARCH_RESPONSE_BYTES guard).
- **Verification:** `cargo test -p mika-agent --lib skills::builtin_handlers::tests` passes. `grep HTTP_CLIENT crates/mika-agent/src/skills/builtin_handlers.rs` returns zero hits. `grep format_brave_results crates/mika-agent/src/skills/builtin_handlers.rs` returns zero hits. `grep "api.search.brave.com" crates/mika-agent/src/skills/builtin_handlers.rs` returns zero hits.

### U2. Extend `ToolContext` with `gateway_url` and `internal_token` fields

- **Goal:** Add the two new borrow-scoped fields to the struct definition. This is the smallest possible schema change — the propagation to construction sites happens in U3.
- **Requirements:** R1, R3. Cites KTD2.
- **Dependencies:** None (leaf).
- **Files:**
  - `crates/mika-agent/src/tools/mod.rs` — add fields to the `pub struct ToolContext<'a>` at lines 101-166.
- **Approach:** Add `pub gateway_url: Option<&'a str>` and `pub internal_token: Option<&'a str>` after the existing `brave_api_key: Option<&'a str>` slot at line 113. Doc-comment each: `/// mika-gateway base URL (from Settings.routing_url). Used by web_search to reach the single-egress substrate at POST /internal/search.` and `/// mika-gateway internal bearer token (from Settings.internal_token, exposed via .expose_secret()). Used by web_search to authenticate to the substrate.`
- **Patterns to follow:** the existing `brave_api_key` and `github_token` fields (same shape, same lifetime, same nullability semantics).
- **Test scenarios:** none — pure schema addition. Compilation of U3 is the load-bearing proof.
- **Test expectation: none — pure struct field addition; the compile-time propagation in U3 is the verification.**
- **Verification:** `cargo check -p mika-agent` compiles up to but not through the missing-field errors at construction sites — those errors are the input to U3.

### U3. Thread `gateway_url` + `internal_token` through every `ToolContext` construction site

- **Goal:** Wire the two new fields end-to-end. Production sites (server/{mod,handlers,a2a,investigate}, teams/engine) pull from `Settings` / `AppState`. Test sites (test_utils, task_engine/engine tests, pr_merge_with_gate tests) pass `None`.
- **Requirements:** R1. Cites KTD2, KTD3.
- **Dependencies:** U2.
- **Files** (from `grep -rn "brave_api_key:" crates/mika-agent/src/`):
  - `crates/mika-agent/src/server/mod.rs` — 2 sites (init at ~L428, ToolContext at ~L1414). Pull from `settings.routing_url.as_deref()` and `settings.internal_token.as_ref().map(|t| t.expose_secret())`. Add to `AgentState` alongside `brave_api_key`.
  - `crates/mika-agent/src/server/handlers.rs` — 1 site at L1359. `state.gateway_url.as_deref()` / `state.internal_token.as_deref()`.
  - `crates/mika-agent/src/server/a2a.rs` — 1 site at L164. Same shape.
  - `crates/mika-agent/src/server/investigate.rs` — 1 site at L761. Pass `None` (investigate is an operator-driven read-only surface; web_search there returns the substrate-not-configured error, which is correct — the investigate agent should not be issuing web searches during triage).
  - `crates/mika-agent/src/teams/engine.rs` — 2 sites (L1326, L1776) + the field on the struct at L111 and 2 construction sites (L284, L331). Thread `routing_url` + `internal_token` alongside `brave_api_key`.
  - `crates/mika-agent/src/test_utils.rs` — 5 sites (L47, L129, L163, L197, L235). All `None`.
  - `crates/mika-agent/src/task_engine/engine.rs` — 2 test sites (L1981, L2396). All `None`.
  - `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — 1 test site at L1815. `None`.
- **Approach:** Rustc will emit "missing field" errors at each construction site once U2 is committed. Walk them in the order above (production first, then tests), matching the pattern of the surrounding `brave_api_key: ...` line.
- **Patterns to follow:** whatever mechanical shape the existing `brave_api_key` threading uses at each site — this is a pure duplication with different names.
- **Test scenarios:** none — mechanical field-propagation. The existing test suite is the regression signal.
- **Test expectation: none — mechanical field propagation. The existing `cargo test -p mika-agent` suite catches any typo.**
- **Verification:** `cargo build -p mika-agent` succeeds. `cargo test -p mika-agent --no-run` compiles. `grep -c "gateway_url:" crates/mika-agent/src/ -r` returns ~14 hits (schema + all construction sites).

### U4. Add wiremock-backed regression test asserting substrate-only egress

- **Goal:** Deterministic proof that `web_search` reaches `POST /internal/search` with the correct body + auth header shape, and does not reach any other URL. Load-bearing artifact for AC4 and R4.
- **Requirements:** R3, R4. Cites KTD5.
- **Dependencies:** U1, U2, U3.
- **Files:**
  - `crates/mika-agent/Cargo.toml` — add `wiremock.workspace = true` to `[dev-dependencies]`.
  - `crates/mika-agent/src/skills/builtin_handlers.rs` — extend the existing `#[cfg(test)] mod tests` block with the new `#[tokio::test]` cases.
- **Approach:**
  1. Introduce a helper `spawn_substrate_mock() -> (MockServer, /* url */ String)` that starts a `wiremock::MockServer::start().await`, registers a `Mock::given(method("POST")).and(path("/internal/search"))` matcher returning a canned 2xx `SearchResponse` JSON, and returns the server + its base URL.
  2. Test 1 (`web_search_routes_via_substrate_and_does_not_contact_brave`): call the handler with a `ToolContext` whose `gateway_url = Some(&mock_url)` and `internal_token = Some("test-token")`. Assert (a) the mock received exactly one request, (b) the request path is `/internal/search`, (c) the request body deserializes to `SearchRequest{query: "kittens", max_results: 5}`, (d) the `Authorization` header equals `"Bearer test-token"`, (e) no `X-Subscription-Token` header is present, (f) no request has been made to any other path (wiremock's default is to reject unregistered paths with 404, which the handler will surface as an error — but for this test we only inspect the one registered mock).
  3. Test 2 (`web_search_reports_substrate_unconfigured_when_gateway_url_missing`): `gateway_url = None` → expect the KTD2 fail-mode error message.
  4. Test 3 (`web_search_reports_substrate_unconfigured_when_internal_token_missing`): `internal_token = None` → same error.
  5. Test 4 (`web_search_maps_substrate_404_search_upstream_not_configured`): register a mock returning 404 with `{"error": "search_upstream_not_configured"}` → expect the KTD6 tabulated message.
  6. Test 5 (`web_search_maps_substrate_502_upstream_error`): register 502 with `{"error": "upstream_error"}` → expect the KTD6 message.
  7. Test 6 (`web_search_maps_substrate_502_unauthorized`): register 502 with `{"error": "unauthorized"}` → expect the KTD6 message naming key rotation on the gateway.
- **Execution note:** the load-bearing invariant here is Test 1 — the body-shape + no-tenant-header assertions are the AC3/R3 proof. Do not weaken those assertions to accommodate a convenient handler shape. If the handler must be rewritten to satisfy them, that is the correct direction.
- **Patterns to follow:** `crates/mika-gateway/src/egress_search/tests_e3_request_shape.rs` (wiremock body-shape assertion pattern) and `crates/mika-gateway/src/egress_search/tests_e4_no_log.rs` (adversarial no-leak assertion pattern).
- **Test scenarios:** enumerated above.
- **Verification:** `cargo test -p mika-agent --lib skills::builtin_handlers::tests::web_search_` runs all six new tests and they pass. `grep -c "api.search.brave.com" crates/mika-agent/` in production sources (excluding tests) returns 0.

### U5. Emit startup WARN when `brave_api_key` is set alongside gateway substrate config

- **Goal:** Guide operators to move `MIKA_BRAVE_API_KEY` from spirit to gateway. Non-blocking WARN, one-shot at server init.
- **Requirements:** R6 (migration path visibility). Cites KTD3.
- **Dependencies:** U3.
- **Files:**
  - `crates/mika-agent/src/server/mod.rs` — add a one-shot check at server startup (near where `Settings` is loaded, before spawning workers). Log `warn!(event = "brave_api_key_in_spirit_config", "MIKA_BRAVE_API_KEY is set on mika-spirit but web_search now routes through the mika-gateway substrate. Move the key to mika-gateway's config (MIKA_BRAVE_API_KEY on the gateway container) and unset it here.")` if `settings.brave_api_key.is_some() && settings.routing_url.is_some() && settings.internal_token.is_some()`.
- **Approach:** single conditional block at startup. Follows the mika-agent convention of one-shot startup warnings (see `check_env_warnings()` in `server/mod.rs` for `GH_TOKEN` — the exact same pattern).
- **Patterns to follow:** `check_env_warnings()` in `crates/mika-agent/src/server/mod.rs` (mika#380 — the `GH_TOKEN` startup scrub-and-warn pattern).
- **Test scenarios:** none — WARN emission is a runtime side-effect; the existing check_env_warnings tests demonstrate the pattern is testable via `tracing_test` if desired, but this WARN is not load-bearing for the substrate cut and does not need its own regression.
- **Test expectation: none — operator-facing WARN, non-behavioral. The pattern mirrors `check_env_warnings()` (mika#380), which is unit-tested elsewhere.**
- **Verification:** manual — set both env vars, start spirit, observe the WARN line once.

## Verification Contract

- **VC1** — `cargo build -p mika-agent --release` succeeds.
- **VC2** — `cargo test -p mika-agent --lib skills::builtin_handlers::tests` — all existing tests plus the six new `web_search_*` tests pass.
- **VC3** — `cargo clippy -p mika-agent -- -D warnings` — zero clippy warnings introduced.
- **VC4** — `cargo fmt --check -p mika-agent` — no formatting drift.
- **VC5** — Grep audits, run at the crate root:
  - `grep HTTP_CLIENT crates/mika-agent/src/skills/builtin_handlers.rs` → 0 hits
  - `grep format_brave_results crates/mika-agent/src/skills/builtin_handlers.rs` → 0 hits
  - `grep "api.search.brave.com" crates/mika-agent/src/skills/builtin_handlers.rs` → 0 hits (may appear in test comments describing what is NOT contacted — those are documentation, not production paths)
  - `grep -c "gateway_url:" crates/mika-agent/src/tools/mod.rs` → 1 (struct field)
  - `grep -c "internal_token:" crates/mika-agent/src/tools/mod.rs` → 1 (struct field)
  - The new `web_search_routes_via_substrate_and_does_not_contact_brave` test present and passing.
- **VC6** — Manual smoke (post-deploy, out of scope for this PR's CI but recorded here): trigger a `web_search` from a mika agent, observe (a) a substrate `search_egress` audit event in the gateway log, (b) no direct-to-Brave request from the mika-spirit container's network.

## Definition of Done

- [ ] U1-U5 landed on branch `fix/1971/agent-egress-web-search-bypasse`.
- [ ] VC1-VC5 verified locally before push.
- [ ] Multi-agent independent code review (compound-engineering:ce-code-review) run in a separate context; verdict = APPROVED-MERGE with no P0/P1 findings.
- [ ] PR opened against `senara-solutions/mika:main` with `Closes #1971`.
- [ ] PR body documents migration path + rollback plan (R6).
- [ ] CI green on the PR before handing off to sami for merge.
- [ ] No merge / push --force / --no-verify / --dangerously-skip-permissions performed by this pipeline (mika-dev owns merge — orchestrator role boundary from mika-platform CLAUDE.md).

## Acceptance criteria

_(transcribed verbatim from mika#1971 body per mika/.claude/commands/mika.md step 2; DoD ≠ AC — both coexist.)_

- [ ] **AC1**: `web_search` builtin calls `POST http://$MIKA_GATEWAY_URL/internal/search` instead of `GET https://api.search.brave.com/res/v1/web/search`
- [ ] **AC2**: HTTP_CLIENT direct removed from builtin_handlers.rs for web_search (keep for other builtins if present)
- [ ] **AC3**: Q1-Q4 STRIP TOTAL preserved end-to-end (zero tenant identifier leaks to Brave)
- [ ] **AC4**: Regression test: mock substrate endpoint receives the request correctly, upstream Brave is not contacted by the builtin
- [ ] **AC5**: Documentation of migration path + rollback plan in PR body

## Scope Boundaries

**In scope:**
- `crates/mika-agent/src/skills/builtin_handlers.rs` — lines 56-61 (HTTP_CLIENT) and 173-243 (web_search) + `format_brave_results` (adjacent, tightly coupled).
- `crates/mika-agent/src/tools/mod.rs` — `ToolContext` struct definition.
- All `ToolContext` construction sites listed in U3.
- `crates/mika-agent/Cargo.toml` — one dev-dep addition.
- `crates/mika-agent/src/server/mod.rs` — one WARN emission (U5).

**Out of scope (by design):**
- **Removing `brave_api_key` from `ToolContext`** — deferred to a follow-up cleanup PR per KTD3. Mixes review lenses.
- **Extending `scripts/verify-egress-uniqueness.sh` to gate mika-agent** — deferred to a follow-up hardening PR. Would fail immediately today without this fix; sequencing dictates it lands after.
- **Touching any other builtin in `builtin_handlers.rs`** — this PR owns lines 56-61 and 173-243 (plus `format_brave_results` wherever it lives in the file), nothing else. If mika#1969 (parallel wave-nuit ticket) modifies the same file, rebase and preserve both diffs (see Wave-nuit note below).
- **Adding a `MIKA_WEB_SEARCH_VIA_SUBSTRATE=1` env toggle** — rejected per KTD1.
- **Any change to `crates/mika-gateway/`** — the substrate is a stable dependency; changes there belong in mika#1806 follow-ups.

### Deferred to Follow-Up Work

- **DF1 — Remove `ToolContext.brave_api_key`.** Now dead in production. Small mechanical PR: delete the field, delete the ~15 construction-site references, delete the read at `builtin_handlers.rs:3273` test comment. File as a p3-nice-to-have follow-up.
- **DF2 — Extend CI lint to gate mika-agent.** `scripts/verify-egress-uniqueness.sh` currently allowlists `crates/mika-gateway/src/egress_search*`. Extend the deny-list to include `crates/mika-agent/` (or more narrowly, `crates/mika-agent/src/skills/builtin_handlers.rs` when Brave-related identifiers appear). This is the structural counterpart of R1 — makes regression at the identifier level impossible without a lint override.
- **DF3 — Delete `MIKA_BRAVE_API_KEY` from spirit `.env.example`** and document the move on the gateway side. Small docs PR.

## Risks & Dependencies

- **R-1 — Wave-nuit N=4 rebase conflict with mika#1969.** mika#1969 may also touch `crates/mika-agent/src/skills/builtin_handlers.rs`. Scope defense: this PR owns exactly lines 56-61 (HTTP_CLIENT static, being removed) and 173-243 (web_search, being rewritten) plus `format_brave_results` (function to be removed). Any other change in the file belongs to the other PR. Rebase order: this PR is p2-normal, mika#1969 priority unknown — resolve the conflict on whichever lands second by preserving both diffs' intended non-overlapping regions.
- **R-2 — `wiremock` build cost.** Adds a small dev-dep. Already present in the workspace root `Cargo.toml`; the added `crates/mika-agent/Cargo.toml [dev-dependencies]` line reuses the workspace pin. No new transitive dependency introduced.
- **R-3 — `Settings.internal_token.expose_secret()` at the ToolContext construction site is a plaintext exposure.** Mitigated by (a) the value is scoped to a single-turn borrow with no `.await` outside the substrate HTTP call, (b) `SecretString` still zeroizes at end-of-scope on the owning `Settings`, (c) it never crosses a serialization boundary. Documented in KTD2.
- **R-4 — Substrate 404 (`search_upstream_not_configured`) surfacing as a user-facing error.** If an operator disables the substrate mid-flight (unlikely — this would require redeploying the gateway with the config unset), `web_search` returns the KTD6 tabulated message rather than crashing. Acceptable — mirrors the current `MIKA_BRAVE_API_KEY not configured` shape.
- **D-1 — mika-gateway substrate is deployed and healthy.** Verified by mika#1806 E1-E4 acceptance (adversarial no-log test, request-shape test, real-network E2 test all green in CI on 2026-08-18). No new dependency introduced by this PR.

## Migration Path & Rollback (for PR body)

**What changes for operators:** nothing observable at the tool surface. The `web_search` tool takes the same input, produces the same output shape. The network hop shifts from mika-spirit → Brave to mika-spirit → mika-gateway → Brave. Latency delta is a single intra-cluster hop plus the substrate's 5s hard-cap (currently: sub-100ms in-cluster).

**Operator action recommended (not required for this PR):**
1. Set `MIKA_BRAVE_API_KEY` on the mika-gateway container (if not already set — see mika#1808 E2 rollout notes).
2. Unset `MIKA_BRAVE_API_KEY` from mika-spirit's env — the value is no longer read by `web_search`. Confirms via the U5 startup WARN that the change was intended.
3. Verify `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN` are set on mika-spirit (they are, for `send_message` — same values are reused).

**Rollback:** `git revert` this single commit. The substrate endpoint remains available; the old direct-to-Brave codepath is restored verbatim. No data migration, no state change, no operator coordination required.

**Evidence the substrate is production-ready:** mika#1806 E4 shipped 2026-08-18. Load-bearing Q4 no-leak test (`crates/mika-gateway/src/egress_search/mod.rs::tests::log_assertion_no_tenant_no_query_no_forbidden_fields`) has been green in CI for 5 days. `search_egress` audit events are being emitted in production with the `{upstream, latency_ms, status}` shape.

## Sources & Research

- Ticket: [mika#1971](https://github.com/senara-solutions/mika/issues/1971)
- Parent milestone: mika#1806 (E1-E4 substrate) — E1 keystone plan at `docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md` (referenced from `crates/mika-gateway/src/egress_search/mod.rs:5`).
- Substrate contract: `crates/mika-gateway/src/egress_search/mod.rs:6-8, 80-357`.
- Substrate route registration: `crates/mika-gateway/src/routes.rs:258`.
- Substrate auth middleware: `require_bearer_token` in `crates/mika-gateway/src/routes.rs`.
- Current handler under repair: `crates/mika-agent/src/skills/builtin_handlers.rs:56-61, 173-243`.
- `ToolContext` schema: `crates/mika-agent/src/tools/mod.rs:101-166`.
- Existing `wiremock`-based test patterns: `crates/mika-gateway/src/egress_search/tests_e3_request_shape.rs`, `tests_e4_no_log.rs`.
- Settings source of truth for `routing_url` + `internal_token`: `crates/mika-common/src/config.rs:787,799`.
- No external research needed — the change is bounded by two files' contracts (substrate route + ToolContext), both already documented in-repo.
