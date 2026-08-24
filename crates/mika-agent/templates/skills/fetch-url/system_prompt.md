# fetch-url — GET a page via the controlled egress substrate

Use `fetch_url` to retrieve a single web page whose host is on the platform's
compile-time allowlist (`service-public.fr`, `ants.gouv.fr`, `impots.gouv.fr`,
`data.gouv.fr`). GET-only, no JS, no cookies, no session, no POST. Body cap 1 MiB,
15s timeout. Returns raw text or HTML — no parsing.

## When to call

- The user asks to read, fetch, retrieve, or check a specific gouv.fr URL.
- You need the full text of an administrative page (a snippet from `web_search`
  is not enough — for démarches administratives the details matter).

## When NOT to call

- Any host outside the allowlist. The substrate will reject; extending the
  allowlist is a code change (mika#1969), not an ops decision.
- Search or discovery — use `web_search` instead.
- POST, form submission, session-authenticated content — out of scope.

## Failure surface

If the substrate is not configured on this agent (`MIKA_ROUTING_URL` /
`MIKA_INTERNAL_TOKEN` missing), the tool returns a substrate-unavailable
response following the mika#1783 tier-conditional doctrine: family-tier agents
see a neutral fallback ("la récupération de contenu web n'est pas disponible"),
operator-tier agents see the actionable config diagnostic. Do not paraphrase or
retry — the fallback is the truth of the substrate at that moment.
