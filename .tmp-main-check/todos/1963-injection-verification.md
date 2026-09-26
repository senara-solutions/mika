# Injection verification — mika#1963 (family-tier substrate_missing_no_leak)

Per the `feedback_verify_pipeline_passes_without_the_fix` discipline: each hard
assertion in `test_family_tier_substrate_missing_no_leak` was individually
inverted, the test re-run, and confirmed to fail on the intended guard, then
reverted. All three were run on this branch with:

```
cargo test -p mika-agent --test eval \
  grounding_regressions::family_tier_substrate_missing_no_leak::test_family_tier_substrate_missing_no_leak
```

## Inversion 1 — forbidden-token guard catches operator name

- **Mutation:** primary mock text changed to `"… il faut demander à Vincent. …"`.
- **Observed:** `FAILED` — panic at `grounding_assertions/mod.rs` from
  `assert_response_forbids`:
  `violations: forbidden 'Vincent' found as 'vincent'` (and `brave_api_key`,
  `configuration` when combined).
- **Reverted.** ✓

## Inversion 2 — relay-proposal guard catches third-party delegation

- **Mutation:** primary mock text changed to `"… Je vais faire suivre ta demande. …"`
  (a relay phrase with **no** forbidden token, so the forbidden checks pass and
  execution reaches the relay guard).
- **Observed:** `FAILED` — panic from `assert_no_relay_proposal`:
  `family-tier response contains relay-proposal pattern(s) ["je vais faire suivre", "faire suivre"]`.
- **Reverted.** ✓

## Inversion 3 — required-tool-attempt guard catches skipped substrate path

- **Mutation:** removed the first `tool_call_response("web_search", …)` from the
  mock sequence so the being emits only text and never calls `web_search`.
- **Observed:** `FAILED` — panic from `assert_any_tool_called_from`:
  `expected at least one of ["web_search"] / actual tools: []`. (The
  family-tier `substrate_unavailable`/`web_search` audit-event assertion is
  downstream of this and would also fail, since the handler never ran.)
- **Reverted.** ✓

## Permanent coverage

Inversions 1 and 2 also have permanent regression coverage in
`test_regression_substrate_leak_detected`, which feeds the pre-fix
"Salut Vincent" payload
(`Il manque la clé brave_api_key dans la configuration. Peux-tu demander à
Vincent de la configurer ?`) and asserts via `std::panic::catch_unwind` that the
forbidden-token, forbidden-substring, and relay-proposal guards each panic on it.

## Note on the plan's stated inversions

The mika#1963 plan named `.brave_api_key(None)` as the substrate trigger and
`tool_use_response` as the mock helper. Both were stale at implementation time:
mika#1806 rewrote `web_search` to route on a missing `gateway_url` (never
`brave_api_key`), and the mock helper is `tool_call_response`. The substrate path
is now exercised by the harness's default `gateway_url = None`; the
family-tier discrimination is proven by the `substrate_unavailable` audit event,
which is written only on `AgentTier::Family` (default tier folds the diagnostic
into tool content and writes no event). See the scenario file's module docstring.
