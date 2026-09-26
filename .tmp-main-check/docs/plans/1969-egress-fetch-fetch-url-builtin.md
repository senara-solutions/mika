---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: mika#1969
branch: feat/1969/gateway-agent-fetch-url-via-egress-fetch
---

# feat(gateway,agent): fetch_url via egress_fetch — GET-only lecture-seule sur egress contrôlé (MSC Q1)

## Problem Frame

mika-secretary (MSC) is blocked on French administrative flows because `web_search` (Brave via mika-gateway substrate mika#1889/#1911/#1912/#1914) returns only ~3-line snippets. MSC will not act on `service-public.fr` / `ants.gouv.fr` / `impots.gouv.fr` / `data.gouv.fr` procedures from snippets alone — she needs the full page body.

**Narrow need:** GET a URL, read text/HTML, return it. No JavaScript. No cookies. No session. No POST. Zero user-tunable knobs. A headless browser is a two-order-of-magnitude overshoot for reading government prose.

**Why this shape (evidence-anchored):** The existing `crates/mika-gateway/src/egress_search/` module (mika#1807 E1) is the canonical controlled-egress substrate: marker types (`SearchEgressClient`), `pub(crate)` strict, `POST /internal/search` handler, Q4 STRIP TOTAL log discipline, CI lint that grep-fails cross-crate imports. Reusing `SearchEgressClient` for arbitrary GETs would grep-fail the CI lint (`scripts/verify-egress-uniqueness.sh` guards the marker discipline). The right architectural answer is a **mirror module** — `crates/mika-gateway/src/egress_fetch/` — that carries the same shape for a different egress class.

## Requirements

- **R1** — A new HTTP endpoint `POST /internal/fetch` on the gateway performs a GET against a caller-provided URL and returns the response body (text/plain or text/html, un-parsed), bounded by size and time.
- **R2** — Only URLs whose host matches a compile-time allowlist (`service-public.fr`, `ants.gouv.fr`, `impots.gouv.fr`, `data.gouv.fr`, plus any subdomain of these) are accepted. Any other host is rejected with a structured error.
- **R3** — A new agent-side builtin tool `fetch_url` (registered in `crates/mika-agent/src/skills/builtin_handlers.rs`) calls the gateway endpoint on the agent's behalf.
- **R4** — The gateway substrate preserves the Q4 STRIP TOTAL discipline established in `egress_search`: zero tenant identifiers, zero URL bytes, zero response bytes in any log field emitted by the module.
- **R5** — `scripts/verify-egress-uniqueness.sh` is extended so the new module's authoritative identifier tokens (the allowlist host substrings) cannot appear in any file outside the authorized module tree.
- **R6** — Regression tests exercise both success (allowlisted URL returns body) and rejection (non-allowlisted URL returns structured error) paths.
- **R7** — A best-practices solution doc captures the "mirror substrate module for a new egress class" pattern for future extensions.

## Acceptance criteria

- [ ] **AC1** — `crates/mika-gateway/src/egress_fetch/` module exists with `FetchEgressClient` marker type + `POST /internal/fetch` endpoint wired into `AppState` and `routes.rs`. `FetchEgressClient` is `pub(crate)` only; no conversion path exists from a bare `reqwest::Client`.
- [ ] **AC2** — Agent-side builtin `fetch_url` in `crates/mika-agent/src/skills/builtin_handlers.rs` performs GET only via the gateway substrate. Compile-time allowlist enforcement lives in the gateway module (single source of truth); the agent surface is deliberately thin.
- [ ] **AC3** — `scripts/verify-egress-uniqueness.sh` grep-fails the build if any of the four gouv.fr host substrings appear in a source file outside `crates/mika-gateway/src/egress_fetch/`, `crates/mika-gateway/tests/egress_fetch/` (if used), or the authorized docs/scripts allowlist entries. Legacy allowlist for `builtin_handlers.rs` is NOT added — the builtin does not name the hosts; it delegates.
- [ ] **AC4** — Q1-Q4 STRIP TOTAL discipline preserved: the `egress_fetch` module emits exactly two `tracing::info!` events (`fetch_requested`, `fetch_egress`) with the field-set `{event, upstream, host_class, status, latency_ms, message}`. Zero tenant identifiers, zero URL bytes (`host_class` collapses the allowlist match to a bounded label — see KTD3), zero response bytes. Enforced by a `CapturingLayer` / `FieldVisitor` test mirroring `crates/mika-gateway/src/egress_search/mod.rs::tests::log_assertion_no_tenant_no_query_no_forbidden_fields`.
- [ ] **AC5** — Cross-repo follow-up filed to extend the mika#1810 iptables policy (owned by `mika-cloud`) to permit egress to `service-public.fr` / `ants.gouv.fr` / `impots.gouv.fr` / `data.gouv.fr`. Tracked as a mika-cloud issue linked from this PR — **not gating this PR** because iptables lives in a different repo and the pre-firewall path is already functional for MSC (see § Scope Boundaries).
- [ ] **AC6** — Regression tests cover: (a) fetch to an allowlisted host returns 200 + body bytes (validated against a mock server bound to an allowlisted-shape hostname); (b) fetch to a non-allowlisted host returns a `SecurityError::HostNotAllowed` variant mapped to HTTP 403; (c) fetch to an allowlisted host but a non-GET wire request signature is impossible by API construction (there is no method knob).
- [ ] **AC7** — Solution doc `docs/solutions/best-practices/mirror-substrate-module-for-new-egress-class-2026-08-23.md` written, with YAML frontmatter (`module`, `tags`, `problem_type`, `category`), capturing the mirror-module pattern (marker isolation + CI lint extension + Q4 preservation) so a future third egress class (webhook-fetch, DNS-lookup, …) can follow the same shape.

## Definition of Done

- [ ] Code compiles clean (`cargo build --workspace`).
- [ ] Clippy clean (`cargo clippy --workspace --all-targets -- -D warnings`).
- [ ] Formatted (`cargo fmt`).
- [ ] All acceptance criteria checked (AC1–AC7).
- [ ] Regression tests pass (`cargo test -p mika-gateway egress_fetch` and `cargo test -p mika-agent fetch_url`).
- [ ] CI lint script self-check passes locally: `bash scripts/verify-egress-uniqueness.sh` exits 0 on a clean tree, exits 1 when a canary reference is planted.
- [ ] Solution doc committed at path in AC7.
- [ ] PR body includes verbatim `git diff --stat main..HEAD` output.
- [ ] PR body includes `Closes #1969`.
- [ ] mika-cloud sibling issue filed for iptables extension (linked in PR body via `Companion follow-up:` line).

## Key Technical Decisions

### KTD1 — Agent → Gateway wire transport for `fetch_url`

The builtin `fetch_url` calls the gateway's `POST /internal/fetch` — it does **not** perform the outbound HTTP itself. This is the load-bearing decision that keeps the controlled-egress invariant intact: allowlist enforcement, Q4 discipline, and the iptables perimeter (once mika#1810 is extended, AC5) all live in one place — the gateway. Duplicating any of those in the agent surface would allow future divergence (a subtle prompt injection could push an agent-side allowlist out of sync with the iptables reality).

The agent already knows how to reach the gateway: `Settings` carries `routing_url` (the gateway base URL) and `internal_token` (the shared bearer secret). The `fetch_url` builtin threads both via new fields on `ToolContext` — `gateway_url: Option<&str>` and `internal_token: Option<&str>` — following the same optional-pattern shape that `brave_api_key: Option<&str>` and `github_token: Option<&str>` already use (`crates/mika-agent/src/tools/mod.rs`). When either is absent the builtin returns a configuration error rather than falling back to direct egress; the fail-closed default matches the substrate invariant.

Alternatives considered:
- **Direct upstream (bypass gateway):** rejected — would require duplicating the allowlist and log discipline in `builtin_handlers.rs`, and — since the current `web_search` builtin still hits Brave directly (LEGACY_ALLOWLIST entry in `verify-egress-uniqueness.sh:65-68`) — would compound the pre-E1 legacy shape we are actively trying to migrate away from.
- **Static reference to `Settings`:** rejected — `Settings` is intentionally not exposed to `ToolContext`; threading two named fields is narrower and reviewable.

### KTD2 — Allowlist as compile-time constant, not env-tunable

The four gouv.fr hosts are hardcoded in `egress_fetch/mod.rs` as a `pub(crate) const ALLOWED_HOSTS: &[&str] = &[…]`. Extension is a code change + deploy — never a runtime knob. Same reasoning as `INTERNAL_REPOS` in `crates/mika-gateway/src/github.rs:1382` (documented in the gateway CLAUDE.md § GitHub Webhook Integration): the allowlist is security-adjacent, and turning it into an env var makes the security envelope operator-mutable rather than reviewer-mutable.

### KTD3 — `host_class` log field replaces raw host in Q4 emissions

The Q4 STRIP TOTAL invariant in `egress_search` allows the `upstream` field to carry a bounded static string (`"brave"`) because a single upstream is unambiguously identified by a per-request-independent value. For `egress_fetch` the outbound host is caller-influenced (a caller with access to the tool could probe which of the four hosts the substrate cooperated with). We resolve this by:

- The `upstream` field carries the static string `"gouv_fr"` (identifies the substrate variant, upstream-independent).
- A new `host_class` field carries the matched-allowlist-entry label (`"service_public"`, `"ants"`, `"impots"`, `"data_gouv"`). This is bounded (four labels) and carries no per-request bits beyond the allowlist bit itself — which the caller already knows because they chose the URL.

Raw URL bytes never appear in any log field. The `CapturingLayer` test in AC4 enforces this via the `all_field_values` cross-source assertion pattern already used in `egress_search`.

### KTD4 — Response body cap = 1 MiB (mirror `MAX_SEARCH_RESPONSE_BYTES`)

`egress_fetch` uses the same 1,048,576-byte cap as `MAX_SEARCH_RESPONSE_BYTES` in `crates/mika-agent/src/skills/builtin_handlers.rs:53`. A gouv.fr page routinely runs 100-500 KB of prose + markup; 1 MiB is comfortable headroom without opening a memory-exhaustion vector. Enforced two ways per the existing substrate pattern: (a) `resp.content_length()` header check, (b) `resp.bytes().await` length re-check (defensive against upstream returning no Content-Length).

### KTD5 — Timeout budget: 15s hard cap, 10s per-request

The `web_search` builtin uses `.timeout(15)` on its `reqwest::Client`. `egress_search` uses a 5s hard cap because search is latency-sensitive. Government pages can be slower (heavier assets, TLS handshake, sometimes older origin infra); 15s hard cap with a 10s per-request timeout mirrors the `web_search` budget while preserving headroom for one retry inside the hard cap.

### KTD6 — HTTP method: GET only, by API construction

The `POST /internal/fetch` request body carries a `{url: String}` payload — no `method` field. The substrate hardcodes `.get(&url)` in the outbound call. There is no code path from the wire payload to a non-GET method; the type system enforces it. This is the "construct the incapacity, don't promise the restraint" pattern from `docs/solutions/best-practices/security-hardening-patterns.md` (referenced by `verify-egress-uniqueness.sh:8`).

## Implementation Units

### U1. egress_fetch module scaffold

**Goal:** Establish the marker-type discipline, error taxonomy, and empty handler stub. No outbound HTTP yet.

**Requirements:** R1, R4.

**Dependencies:** none.

**Files:**
- `crates/mika-gateway/src/egress_fetch/mod.rs` (new)
- `crates/mika-gateway/src/lib.rs` — add `pub(crate) mod egress_fetch;`

**Approach:**
Mirror the shape of `crates/mika-gateway/src/egress_search/mod.rs` lines 1-233:
- Module doc-comment naming Q1-Q4 discipline (Q1 module-in-gateway placement; Q2 quadruple isolation with the four points spelled out; Q3 multi-tenant sharing; Q4 STRIP TOTAL). Replace the "search" nouns with "fetch" nouns; cite mika#1969 in the placement line.
- `pub(crate) struct FetchEgressClient { inner: reqwest::Client, upstream: FetchUpstream }` — private inner, no `pub` conversion.
- `pub(crate) enum FetchUpstream { GouvFr(GouvFrConfig) }` — one variant now; comments reserve the shape for future egress classes.
- `pub(crate) struct GouvFrConfig { /* no per-tenant fields */ }` — empty struct today; here to make future config additions namespaced.
- `pub(crate) const ALLOWED_HOSTS: &[&str] = &["service-public.fr", "ants.gouv.fr", "impots.gouv.fr", "data.gouv.fr"];` — the source of truth referenced by KTD2.
- `pub(crate) const FETCH_HARD_TIMEOUT_SECS: u64 = 15;` and `pub(crate) const MAX_FETCH_RESPONSE_BYTES: usize = 1_048_576;` — KTD4/KTD5 constants.
- `pub(crate) enum FetchError { HostNotAllowed, InvalidUrl, ResponseTooLarge, UpstreamStatus(u16), Transport, Timeout }` (thiserror-derived, mirroring `SearchError`). `http_status()` and `tracing_status()` methods for consistent HTTP mapping and Q4-safe error labels.
- `pub(crate) fn build_client() -> reqwest::Client` — dedicated builder mirroring `egress_search::build_client()`, with the 10s per-request timeout / 15s hard cap split from KTD5.
- Empty stub `pub(crate) async fn handle_internal_fetch(...) -> (StatusCode, Json<serde_json::Value>)` that returns 501 NOT_IMPLEMENTED for now (U3 fills it in). Stub keeps U3's wiring commit small and reviewable.

**Patterns to follow:** `crates/mika-gateway/src/egress_search/mod.rs` lines 45-233 (marker types, config, error taxonomy, `build_client` factory).

**Test scenarios:**
- `provider_name_stable` — `FetchUpstream::GouvFr(...).provider_name()` returns exactly `"gouv_fr"` (regression guard on KTD3 label).
- `fetch_error_status_mapping_is_stable` — every `FetchError` variant maps to a stable `StatusCode`; add a comment naming the operator-visible dashboard that keys off the mapping.
- `fetch_error_tracing_status_is_stable` — every `FetchError` variant maps to a stable `&'static str` label suitable for Q4 emission.
- `allowed_hosts_are_lowercase` — every `ALLOWED_HOSTS` entry equals its `to_lowercase()`; guards against future entries that would break case-insensitive matching in U2.
- `allowed_hosts_contain_expected_four` — literal set membership check for the four gouv.fr hosts; regression guard against accidental deletion.

**Verification:** `cargo build -p mika-gateway` compiles with the new module and stub handler. `cargo test -p mika-gateway egress_fetch::tests` passes the five shape tests above.

---

### U2. egress_fetch allowlist + fetch execution

**Goal:** Fill in the substrate: URL parsing, allowlist enforcement, outbound GET, response size/timeout limits, Q4-disciplined logging.

**Requirements:** R1, R2, R4.

**Dependencies:** U1.

**Files:**
- `crates/mika-gateway/src/egress_fetch/mod.rs` (extend from U1)
- `crates/mika-gateway/src/egress_fetch/gouv_fr.rs` (new — mirrors `egress_search/brave.rs` structure)

**Approach:**
1. Add wire types in `mod.rs`:
   ```rust
   #[derive(Debug, Deserialize, Serialize)]
   pub(crate) struct FetchRequest {
       pub url: String,  // NEVER log this field
   }

   #[derive(Debug, Deserialize, Serialize)]
   pub(crate) struct FetchResponse {
       pub body: String,          // text/plain or text/html, un-parsed
       pub content_type: String,  // upstream Content-Type header, un-parsed
       pub bytes_read: u32,       // bounded side-channel (like upstream_latency_ms)
   }
   ```
2. In `gouv_fr.rs`, implement `pub(crate) async fn execute_gouv_fr_fetch(client: &reqwest::Client, config: &GouvFrConfig, req: FetchRequest) -> Result<FetchResponse, FetchError>`:
   - Parse URL via `url::Url::parse()`. On failure → `FetchError::InvalidUrl`.
   - Extract `host_str()`. If `None` → `FetchError::InvalidUrl`. Reject non-HTTPS schemes → `FetchError::InvalidUrl` (comment: the gouv.fr sites all serve HTTPS; refusing HTTP prevents downgrade probes).
   - Lowercase the host, then check `ALLOWED_HOSTS` membership. Match by suffix — a host `www.service-public.fr` matches `service-public.fr` when the host ends with the entry AND the char immediately preceding the match is `.` or start-of-string. This catches subdomains cleanly without false-positive matches like `evilservice-public.fr` (the leading char is neither `.` nor start).
   - On mismatch → `FetchError::HostNotAllowed`.
   - `client.get(&req.url).send().await` with the 10s per-request timeout. Map transport failures to `FetchError::Transport`, timeouts to `FetchError::Timeout`.
   - Check `resp.status()`. On non-2xx → `FetchError::UpstreamStatus(status)`.
   - Read Content-Type header (default `application/octet-stream`); store un-parsed for return.
   - `resp.content_length()` gate → `FetchError::ResponseTooLarge` before pulling bytes.
   - `resp.bytes().await` with re-check of `.len() > MAX_FETCH_RESPONSE_BYTES` → `FetchError::ResponseTooLarge`.
   - UTF-8 decode with lossy replacement (`String::from_utf8_lossy`) — gouv.fr pages are UTF-8 in practice; lossy protects against odd origin encodings without a hard fail.
3. In `mod.rs::FetchEgressClient::fetch`, mirror `SearchEgressClient::search`:
   - Emit `info!(event = "fetch_requested", upstream = "gouv_fr")` — no other fields.
   - Start `Instant`.
   - Match `self.upstream` → dispatch to `gouv_fr::execute_gouv_fr_fetch`.
   - Compute latency_ms.
   - Determine `host_class` — this needs the parsed host BEFORE the outcome is folded away. Structure: parse the URL host in `mod.rs` before dispatch, pass `host_class: &'static str` (`"service_public" | "ants" | "impots" | "data_gouv" | "unknown"`) into a helper that emits `emit_audit_event(upstream, host_class, latency_ms, outcome_status)`. When the URL fails to parse, `host_class = "unknown"` — the audit event still records the taxonomy label. **Do NOT emit `host_class` from `execute_gouv_fr_fetch`'s inner scope** — that leaks upstream-side timing correlation.
   - Stamp `bytes_read` on the successful response before returning.

**Patterns to follow:** `crates/mika-gateway/src/egress_search/brave.rs` for the execute-fn shape; `egress_search/mod.rs:251-306` for the `search()` outer method + `emit_audit_event()` + `outcome_status()` helpers.

**Test scenarios:**
- `execute_returns_host_not_allowed_for_evil_host` — URL `https://evil.com/foo` returns `Err(HostNotAllowed)`. Load-bearing regression against silent-allow bugs.
- `execute_returns_host_not_allowed_for_prefix_lookalike` — URL `https://evilservice-public.fr/foo` returns `Err(HostNotAllowed)` (guards the suffix-match implementation against the naive `.ends_with()` bug).
- `execute_returns_invalid_url_for_http_scheme` — URL `http://service-public.fr/foo` returns `Err(InvalidUrl)` (HTTPS-only invariant).
- `execute_returns_invalid_url_for_unparseable` — URL `"not a url"` returns `Err(InvalidUrl)`.
- `execute_success_returns_body_and_bytes_read` — against `mockito`-mocked upstream bound to `127.0.0.1` with a `Host: service-public.fr` header override — returns a `FetchResponse` with the body and `bytes_read` matching the mock payload size.
- `execute_returns_response_too_large_on_content_length` — mocked upstream returns `Content-Length: 2000000` → `Err(ResponseTooLarge)` before body pull.
- `execute_returns_response_too_large_on_bytes_recheck` — mocked upstream returns body > 1 MiB with no Content-Length header → `Err(ResponseTooLarge)` on the post-read check.
- `execute_returns_upstream_status_on_4xx` — mocked 404 → `Err(UpstreamStatus(404))`.
- `execute_returns_transport_error_on_unreachable_upstream` — pointed at `http://127.0.0.1:1/does-not-exist` (bogus port), returns `Err(Transport)`. Mirrors `egress_search::tests::search_returns_transport_error_on_unreachable_upstream`.

**Test scenarios (Q4 discipline — the load-bearing test):**
- `log_assertion_no_tenant_no_url_no_forbidden_fields` — mirror `egress_search/mod.rs:653-737` exactly, adapted for fetch:
  - `CapturingLayer` + `FieldVisitor` with `ALLOWED_FIELDS = &["event", "upstream", "host_class", "status", "latency_ms", "message"]`.
  - Inject a sensitive URL `"https://service-public.fr/TENANT-42-SECRET-PATH-do-not-leak"`.
  - Run `client.fetch(req).await` (offline-mocked, transport error is fine — the assertion works on both success and failure paths per Q4 discipline).
  - Assert exactly 2 events from our module (`fetch_requested`, `fetch_egress`).
  - Assert `forbidden_fields` is empty on every emitted event.
  - Assert the sensitive URL bytes appear in NO field value ANY event (including reqwest/hyper events — the cross-source assertion).
  - Assert `host_class` on the audit event is exactly `"service_public"` (the allowlist label — not the raw host).
  - Assert `latency_ms` is populated.

**Verification:** `cargo test -p mika-gateway egress_fetch` — all above tests pass. The Q4 log-discipline test is the AC4 gate — a failure here is the blocking signal for the whole substrate.

---

### U3. Wire `/internal/fetch` into gateway server

**Goal:** Register the endpoint, thread the client through `AppState`, construct it once at startup.

**Requirements:** R1.

**Dependencies:** U1, U2.

**Files:**
- `crates/mika-gateway/src/routes.rs` — add route registration + `AppState` field
- `crates/mika-gateway/src/main.rs` — construct `FetchEgressClient` once, thread into `AppState`
- All `AppState { … }` struct-literal construction sites (test fixtures, orchestrator_inbox, github tests) — add `fetch_egress_client: None`

**Approach:**
1. In `routes.rs::AppState`, add:
   ```rust
   pub(crate) fetch_egress_client: Option<egress_fetch::SharedFetchEgressClient>,
   ```
   Mirror the `search_egress_client` field at line 152.
2. In `routes.rs::routes()`, add:
   ```rust
   .route("/internal/fetch", post(egress_fetch::handle_internal_fetch))
   ```
   Sibling of the `/internal/search` route at line 258.
3. In `main.rs`, after the existing `search_egress_client` construction (line 172), add a parallel block:
   ```rust
   let fetch_egress_client = /* always construct — no upstream selection knob (KTD2) */
       Some(Arc::new(egress_fetch::FetchEgressClient::new(
           egress_fetch::FetchUpstream::GouvFr(egress_fetch::GouvFrConfig {}),
       )));
   ```
   The construction is unconditional today — the module contains no upstream selection env var; extending the allowlist is a code change.
4. Thread `fetch_egress_client` into `AppState { …, fetch_egress_client, }` at line 221.
5. Fill in the U1 stub — `handle_internal_fetch` reads `state.fetch_egress_client`, returns 404 if `None`, otherwise calls `.fetch(payload).await` and maps to HTTP response. Mirror `handle_internal_search` at `egress_search/mod.rs:318-353` shape-for-shape.
6. Add `fetch_egress_client: None` at every other `AppState { … }` construction site — grep-audit before commit (see § Verification).

**Patterns to follow:** `crates/mika-gateway/src/routes.rs:149-152` (AppState field + doc-comment). `crates/mika-gateway/src/main.rs:172-221` (bootstrap + thread-through). `crates/mika-gateway/src/egress_search/mod.rs:318-353` (handler impl).

**Test scenarios:**
- `route_registered` — spin up the router in a test, `POST /internal/fetch` returns something other than 404 route-not-found. Distinguishes route missing from client not-configured.
- `handler_returns_404_when_client_not_configured` — `AppState { fetch_egress_client: None, .. }` returns 404 with error body `{"error": "fetch_upstream_not_configured"}`.
- `handler_returns_200_on_successful_fetch` — mocked substrate returns Ok — handler returns 200 with serialized `FetchResponse`.
- `handler_returns_403_on_host_not_allowed` — substrate returns `HostNotAllowed` → handler returns 403 with `{"error": "host_not_allowed"}`. Note: this diverges from `SearchError::UpstreamStatus`'s 502 mapping; a security-taxonomy rejection deserves a distinct status.

**Verification:**
- `cargo build -p mika-gateway` compiles with no missing-field errors — this is the grep-audit for AppState construction sites.
- `grep -rn "AppState {" crates/mika-gateway/src/ | wc -l` before and after — count must match after adding the field (the compiler will fail every unpatched site, providing the enforcement).
- `cargo test -p mika-gateway egress_fetch::handler_tests` passes the four handler tests.

---

### U4. Extend `verify-egress-uniqueness.sh` for the new module

**Goal:** Ensure the gouv.fr host substrings cannot appear in any file outside the authorized module tree.

**Requirements:** R5.

**Dependencies:** U1, U2 (the module tree must exist to be authorized).

**Files:**
- `scripts/verify-egress-uniqueness.sh` (extend)

**Approach:**
1. Add the four gouv.fr host substrings to `PATTERNS`:
   ```bash
   PATTERNS=(
       "api.search.brave.com"
       "service-public.fr"
       "ants.gouv.fr"
       "impots.gouv.fr"
       "data.gouv.fr"
       # Future upstreams — extend as new egress classes are added
   )
   ```
2. Add the fetch module + tests + docs paths to `AUTHORIZED_PATHS`:
   ```bash
   AUTHORIZED_PATHS=(
       # ... existing search entries ...
       "crates/mika-gateway/src/egress_fetch/"
       "crates/mika-gateway/src/egress_fetch.rs"
       "crates/mika-gateway/docs/egress-fetch.md"      # if added later
       "docs/plans/1969-egress-fetch-fetch-url-builtin.md"
       "docs/solutions/best-practices/mirror-substrate-module-for-new-egress-class-2026-08-23.md"
       "scripts/verify-egress-uniqueness.sh"           # self-reference for PATTERNS array
   )
   ```
3. **No LEGACY_ALLOWLIST entry for `builtin_handlers.rs`** — the `fetch_url` builtin (U5) does not name the hosts; it delegates to the gateway. This is a load-bearing difference from `web_search` (which still names Brave directly in its handler); documenting the absence in a script comment prevents a future reviewer from adding a legacy entry defensively.
4. Update the script header comment to name mika#1969 as the second controlled-egress class, and note the mirror-module pattern is documented in the AC7 solution doc.

**Patterns to follow:** `scripts/verify-egress-uniqueness.sh` — the existing script is the pattern; extend, don't rewrite.

**Test scenarios:**
- `script_exits_0_on_clean_tree` — run `bash scripts/verify-egress-uniqueness.sh` on the working tree with the new module in place; must exit 0.
- `script_exits_1_on_planted_canary_in_agent` — plant `let _ = "service-public.fr";` in `crates/mika-agent/src/lib.rs`; script exits 1 with the violation named. Revert.
- `script_exits_1_on_planted_canary_in_docs` — plant `service-public.fr` in `docs/architecture.md`; script exits 1. Revert.
- `script_exits_0_when_pattern_is_inside_authorized_path` — the module doc-comments and constants inside `crates/mika-gateway/src/egress_fetch/` naming the hosts must NOT trigger the script. Verified by clean-tree run above.

**Verification:** The canary tests are run interactively during implementation (not left in the tree). The clean-tree run is the CI signal.

---

### U5. `fetch_url` builtin in mika-agent

**Goal:** Expose the substrate to the agent LLM as a builtin tool that calls the gateway.

**Requirements:** R3.

**Dependencies:** U3 (gateway endpoint must exist).

**Files:**
- `crates/mika-agent/src/tools/mod.rs` — extend `ToolContext` with `gateway_url: Option<&'a str>` and `internal_token: Option<&'a str>`
- `crates/mika-agent/src/skills/builtin_handlers.rs` — add `fetch_url` handler + register in `KNOWN_BUILTINS` + dispatch in `execute()`
- All `ToolContext::new_for_tests(...)` or `ToolContext { ... }` construction sites — thread the two new fields (defaulting to `None`).
- All production-side `ToolContext` construction sites — thread `settings.routing_url.as_deref()` and `settings.internal_token.as_deref().map(SecretString::expose_secret)` (audit exact accessor names when implementing).

**Approach:**
1. Extend `ToolContext` per KTD1 — two new optional string fields, matching the `brave_api_key: Option<&str>` shape.
2. Add `"fetch_url"` to `KNOWN_BUILTINS` array (`builtin_handlers.rs:39-47`) — insertion-sorted position.
3. Add dispatch arm in `execute()` (`builtin_handlers.rs:79`):
   ```rust
   "fetch_url" => fetch_url(&input, ctx).await,
   ```
4. Implement `async fn fetch_url(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput`:
   - Extract `url` from input. Reject empty/missing → `ToolOutput::error("Missing or empty 'url' parameter.")`.
   - Length check: `> 2048 chars` → error (URL RFC upper bound guidance; guards against pathological inputs).
   - Read `ctx.gateway_url` and `ctx.internal_token`. Either missing → `ToolOutput::error("fetch_url is not configured for this agent (missing gateway or internal token).")`.
   - Construct `POST {gateway_url}/internal/fetch` with `Authorization: Bearer {internal_token}` and JSON body `{"url": <url>}`.
   - Use the existing `HTTP_CLIENT` static at `builtin_handlers.rs:56-61` (already has a 15s timeout that matches KTD5).
   - On 2xx: deserialize `{body, content_type, bytes_read}` from the response, return `ToolOutput::success(body)`. Truncation to `MAX_OUTPUT_LEN` is applied by the existing `execute()` wrapper — no per-handler cap needed here.
   - On 4xx: parse the `{"error": "<label>"}` body and surface it verbatim to the LLM as `ToolOutput::error(format!("Fetch rejected: {label}"))`. The classifier labels (`host_not_allowed`, `invalid_url`, `response_too_large`) are actionable for the LLM.
   - On 5xx / transport / timeout: `ToolOutput::error("Fetch upstream unavailable.")`. Do NOT leak upstream detail — Q4 discipline extends to the LLM-visible surface (a prompt-injection attacker could otherwise use error messages as an oracle).
5. Register the tool definition (schema) — add `fetch_url` to whatever surface `web_search` uses (this is the same surface the built-in `KNOWN_BUILTINS` binds to; the tool schema likely lives in a manifest file or in `skills/bundled/*/tools.json` — audit at implementation and mirror `web_search`'s registration).

**Patterns to follow:** `crates/mika-agent/src/skills/builtin_handlers.rs` — `web_search` at line 173-243 is the pattern. Copy the shape (input validation, error mapping, response size caps handled by the wrapper), differ on the transport (gateway substrate instead of direct Brave).

**Test scenarios:**
- `test_fetch_url_missing_url` — no `url` field → returns `ToolOutput::error` with "Missing or empty".
- `test_fetch_url_empty_url` — `url: "  "` (whitespace) → returns error.
- `test_fetch_url_too_long` — 3000-char URL → returns error with length guidance.
- `test_fetch_url_no_gateway_configured` — `ctx.gateway_url = None` → returns configuration error.
- `test_fetch_url_no_internal_token_configured` — `ctx.internal_token = None` → returns configuration error.
- `test_fetch_url_success` — mocked gateway returns 200 + body — tool returns `ToolOutput::success` with the body.
- `test_fetch_url_forwards_host_not_allowed` — mocked gateway returns 403 + `{"error": "host_not_allowed"}` — tool returns `ToolOutput::error` naming `host_not_allowed`.
- `test_fetch_url_returns_generic_error_on_5xx` — mocked gateway returns 502 — tool returns generic "upstream unavailable" (no upstream leak).
- `test_fetch_url_in_known_builtins` — assert `KNOWN_BUILTINS.contains(&"fetch_url")`. Mirrors `test_web_search_in_known_builtins` at line 3281.

**Verification:** `cargo test -p mika-agent fetch_url` passes all nine tests. `cargo build -p mika-agent` compiles all `ToolContext` construction sites (compiler catches unpatched sites — this is the grep-audit).

---

### U6. Solution doc — mirror substrate module pattern

**Goal:** Capture the "when to mirror an existing substrate module vs extend it" decision pattern so a future third egress class can follow it without archaeological research.

**Requirements:** R7.

**Dependencies:** U1–U5 must be substantively done so the doc reflects reality.

**Files:**
- `docs/solutions/best-practices/mirror-substrate-module-for-new-egress-class-2026-08-23.md` (new)

**Approach:**
Write in the shape of the existing `docs/solutions/best-practices/*` files (audit two or three of them for structural conventions at implementation time). Sections:
- YAML frontmatter: `module: mika-gateway`, `tags: [egress, substrate, controlled-egress, mirror-pattern]`, `problem_type: architecture`, `category: best-practices`.
- **Context** — the situation: an egress class that shares the *pattern* of an existing substrate (marker isolation + Q4 discipline + CI lint) but not the *upstream* (search vs. GET-only fetch, different allowlist, different response shape).
- **Decision** — mirror the module, don't extend the existing one. Naming: `crates/mika-gateway/src/egress_<class>/`. Marker type: `<Class>EgressClient`. Endpoint: `POST /internal/<class>`.
- **Load-bearing invariants** — what to preserve from the source module (Q1-Q4 discipline, `pub(crate)` scope, CI lint extension) and what may vary per-class (allowlist shape, response schema, timeout budget, host_class taxonomy).
- **Anti-pattern** — extending `SearchEgressClient` to serve non-search calls. Fails the CI lint by design (Q2 point 3); the CI lint is not a bug to work around, it's the enforcement of the marker-type invariant.
- **Checklist for the next class** — the ordered items an implementer needs to hit (module scaffold → substrate impl → routes wiring → CI lint extension → agent builtin → Q4 test).
- **Sibling references** — link to `docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md` (the original E1 plan) and this plan (mika#1969) as the two worked examples.

**Patterns to follow:** Structural conventions from existing `docs/solutions/best-practices/*.md` files (frontmatter shape, section ordering). Content mirrors the compound-engineering doctrine — capture the decision that led here, not the code.

**Test scenarios:** Doc-audit review (implicit in the pipeline's `/ce:review` and `/mika-doc-audit` steps).

**Verification:** File exists at the AC7 path, contains the four required frontmatter fields, references both worked-example plans. Reviewer confirms the checklist is actionable (spot-check by asking: "Could I follow this to add `egress_dns` next week?").

---

### U7. Cross-repo follow-up — mika-cloud iptables policy

**Goal:** File the mika-cloud sibling issue for iptables extension (AC5 gate).

**Requirements:** AC5 (via cross-repo sibling — not gating this PR).

**Dependencies:** none (independent of U1–U6).

**Files:**
- No files in this repo. File is a GitHub issue on `senara-solutions/mika-cloud`.

**Approach:**
Create issue via `gh issue create --repo senara-solutions/mika-cloud`:
- Title: `feat(iptables): extend egress policy for mika#1969 egress_fetch gouv.fr allowlist`
- Body: names mika#1969 as the sibling, lists the four gouv.fr hosts to allow, requests the same shape as the mika#1810 Brave firewall rule (referenced as pattern), and marks priority p2-normal (the substrate is functional pre-firewall; iptables is defense-in-depth E4-scope).
- Labels: `enhancement`, `infrastructure`, `p2-normal`.
- Link back to this PR in the body.

**Patterns to follow:** `mika-cloud` issue conventions — audit the last three infra-labelled issues on that repo for the shape.

**Test scenarios:** None (issue creation is a one-shot ops action).

**Verification:** Issue created; issue URL captured in the mika#1969 PR body under `Companion follow-up:`.

---

## Scope Boundaries

### In scope
- Gateway substrate module (`egress_fetch/`) with marker-type isolation, allowlist enforcement, Q4 STRIP TOTAL logging.
- `POST /internal/fetch` HTTP endpoint on the gateway.
- Agent-side `fetch_url` builtin calling the gateway substrate.
- CI lint extension covering the new module.
- Regression tests for success + rejection + Q4 discipline.
- Solution doc capturing the mirror-module pattern.

### Deferred to Follow-Up Work
- **mika-cloud iptables extension (AC5)** — sibling issue on `mika-cloud` tracking the E4-scope firewall extension. Not gating this PR; the substrate is functional pre-firewall (allowlist enforcement in application code is the primary defense; iptables is defense-in-depth once mika#1810's policy is in place for `mika-cloud`).
- **Migrate `web_search` to the gateway substrate** — the ticket explicitly notes this as orthogonal: "web_search bypasse encore le substrate gateway". A follow-up ticket removes the `builtin_handlers.rs` entry from the CI lint's `LEGACY_ALLOWLIST`. Separate ticket, not this PR.
- **Extension to non-gouv.fr hosts** — every new host is a code change + deploy per KTD2. When MSC (or another agent) needs a new allowlisted host, the change is a small PR against `ALLOWED_HOSTS` + `verify-egress-uniqueness.sh` — not a runtime knob.
- **Content-type validation / body parsing** — the substrate returns raw bytes. Parsing (HTML → text, JSON validation, encoding detection) is an agent-side concern if ever needed; keeping the substrate un-opinionated preserves reusability.
- **Retry / backoff on transient upstream failures** — v1 does zero retries. Government sites are moderately reliable in practice; MSC can retry via the LLM loop if a fetch fails. Adding retry is a small follow-up if latency SLO becomes a real issue.

### Outside this product's identity
- HTTP methods other than GET (POST, PUT, DELETE). The substrate is by-design read-only; enabling writes would need a separate substrate with its own threat model (KTD6).
- Cookies, sessions, JavaScript execution, or any browser-shaped behavior. That is what mika#1974-sibling browser tooling (if it exists) covers; the current ticket is deliberately scoped to lightweight text retrieval.
- Egress to arbitrary URLs (user-input-driven). The allowlist is compile-time by KTD2; there is no admin API to add hosts.

## Risks & Mitigations

- **Risk 1:** Q4 discipline regresses silently — a future PR adds a `warn!` line inside the fetch path that carries the URL. **Mitigation:** the `log_assertion_no_tenant_no_url_no_forbidden_fields` test (AC4) fails at CI time on any such regression. The test is intentionally strict.
- **Risk 2:** Allowlist suffix-match is subtly wrong (e.g., `evilservice-public.fr` matches `service-public.fr` under naive `.ends_with()`). **Mitigation:** dedicated test scenario `execute_returns_host_not_allowed_for_prefix_lookalike` in U2 catches this at implementation time.
- **Risk 3:** The `ToolContext` extension (two new fields) touches every construction site; missing one causes compile failure that could look like a merge conflict. **Mitigation:** the compiler is the grep-audit (KTD1 discipline). The plan calls out that all sites will fail to compile — this IS the enforcement, not a bug.
- **Risk 4:** Concurrent PR mika#1971 (`fix/1971/agent-egress-web-search-bypasse`) also touches `crates/mika-agent/src/skills/builtin_handlers.rs`. **Mitigation:** if mika#1971 lands first, rebase and adjust; if mika#1969 lands first, mika#1971's rebase is trivial (this PR adds a new `fetch_url` handler; #1971 modifies the existing `web_search` handler — different lines, different concerns). The `execute()` dispatch table and `KNOWN_BUILTINS` array are the only shared surfaces; both are additive-friendly.
- **Risk 5:** Government sites (particularly `impots.gouv.fr`) sometimes rate-limit aggressively. **Mitigation:** the substrate returns `UpstreamStatus(429)` verbatim; the LLM can back off naturally. Adding automatic retry is deferred (see Scope Boundaries).

## Verification Contract

**Structural gates (pre-merge):**
1. `cargo build --workspace` clean.
2. `cargo clippy --workspace --all-targets -- -D warnings` clean.
3. `cargo fmt --check` clean.
4. `bash scripts/verify-egress-uniqueness.sh` exits 0.
5. `cargo test -p mika-gateway egress_fetch` — all tests pass, including the Q4 log-assertion.
6. `cargo test -p mika-agent fetch_url` — all tests pass.

**Behavioral gates (implementer-verified, not fully automatable):**
1. A manual `curl` (`POST /internal/fetch` with `{"url": "https://service-public.fr/"}` and bearer auth) against a locally-running gateway returns a 200 with the page body.
2. The same `curl` with `{"url": "https://google.com/"}` returns a 403 with `{"error": "host_not_allowed"}`.
3. `mika-arch`/`mika-dev`/mika-prime does NOT gain the `fetch_url` skill via any allowlist — the tool is discovered by any agent with the `web-search`-adjacent skill surface. Confirm the tool is only offered to agents that need it (MSC and any secretariat-tier agent), not to platform-orchestrator agents.

**Observability gates (post-deploy, monitored not enforced):**
1. `grep fetch_egress $MIKA_SPIRIT_LOG_FILE | jq .` shows only the allowed field set. Any surprise field is a Q4 regression signal.
2. `grep fetch_requested $MIKA_SPIRIT_LOG_FILE | wc -l` and `grep fetch_egress $MIKA_SPIRIT_LOG_FILE | wc -l` return the same count (paired-event invariant).
3. No hits for the raw gouv.fr host substrings in any log field value (the Q4 test is the pre-deploy check; the log grep is the post-deploy trust-but-verify).

## Sources & Research

- **Primary substrate reference:** `crates/mika-gateway/src/egress_search/mod.rs` (753 lines) — the E1 canonical pattern this plan mirrors.
- **CI lint reference:** `scripts/verify-egress-uniqueness.sh` (125 lines) — the marker-discipline enforcement the plan extends.
- **Load-bearing test reference:** `crates/mika-gateway/src/egress_search/mod.rs::tests::log_assertion_no_tenant_no_query_no_forbidden_fields` (lines 653-737) — the Q4 test shape adapted for AC4.
- **Existing builtin reference:** `crates/mika-agent/src/skills/builtin_handlers.rs::web_search` (lines 173-243) — the builtin shape mirrored for `fetch_url`.
- **Origin plan:** `docs/plans/2026-08-18-1807-e1-egress-substrate-plan.md` — the E1 substrate plan documenting Q1-Q4 discipline.
- **Founding requirement:** MSC Q1 relais 2026-08-23 (`/data/workspace/mika-secretary/demandes-mpc.md`) — the operational blocker this plan resolves.
- **Sibling shipped tickets:** mika#1889 (E1 substrate), mika#1911 (E2 Brave client), mika#1912 (E3), mika#1914 (E4).
- **Parent milestone:** mika#1806 (egress-controlled search backend) — this ticket extends the milestone's substrate model to a second egress class.
- **Cross-repo iptables sibling:** mika#1810 (`mika-cloud`) — E4 runtime firewall referenced by AC5.
