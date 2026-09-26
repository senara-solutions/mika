"""mitmproxy addon: inject the GitHub credential host-side (mika#2056).

Loaded by mitmdump via `--scripts`, alongside the Anthropic addon, when the
pilot-egress relay chains a CONNECT github.com:443 / api.github.com:443 request
to it (see mika-pilot-egress-proxy `MITM_FORWARD_HOSTS`). mitmproxy terminates
the TLS against the sandbox client using its own CA — which the sandbox trusts
via the combined CA bundle wired in dispatch-lib.sh (GIT_SSL_CAINFO /
SSL_CERT_FILE / CURL_CA_BUNDLE, a *superset* of the system store so ordinary
verification of every other host is unaffected). This addon then rewrites the
decrypted HTTP request:

  * Strips any client-side Authorization / Proxy-Authorization (the sandbox has
    no real credential to present — mika#2056 removed it entirely).
  * Injects the GitHub credential where the token is read HOST-SIDE:
      - api.github.com (gh CLI / REST / GraphQL) → `Authorization: Bearer <tok>`
      - github.com     (git smart-HTTP push/fetch) → `Authorization: Basic
        base64("x-access-token:<tok>")`
    Both forms are accepted by GitHub for PATs, fine-grained PATs, and App
    installation tokens.

Property (the whole point of mika#2056 — the Anthropic invariant, applied to
GitHub):
  * The sandbox NEVER holds the GitHub token. It is neither in the sandbox
    environment nor on the sandbox filesystem. It lives only in this mitmdump
    process (host), read from a host-only file the dispatcher keeps fresh
    (`~/.mika/pilot-gh-token`, 0600, never bind-mounted into the sandbox),
    with an env fallback for the canary/tests.
  * The mitmproxy CA cert (public) is bound into the sandbox trust store; the
    mitmproxy CA PRIVATE key stays host-side. A compromised pilot can validate
    our cert chain (needed to speak TLS with us) but cannot forge a new cert.
  * A compromised in-sandbox dependency can no longer read the PAT, cannot
    exfiltrate it to an allowlisted host, and cannot push to arbitrary repos
    with it — the credential is applied only here, host-side, per request.

This addon does nothing for non-GitHub hosts — the Anthropic addon (loaded in
the same mitmdump) owns api.anthropic.com, and every other CONNECT stays on the
egress-proxy encrypted pass-through path.
"""

from __future__ import annotations

import base64
import os
import sys
from pathlib import Path

from mitmproxy import http

# Host-only token file the dispatcher rewrites before every spawn (dispatch-lib
# `_stage_pilot_gh_token`). mtime-cached so a rotated App-installation token
# propagates on the next request without a mitmdump restart — the same
# freshness contract the Anthropic addon gets from ~/.claude/.credentials.json.
_TOKEN_FILE = Path.home() / ".mika" / "pilot-gh-token"

# api.github.com carries the REST/GraphQL surface (gh CLI); github.com carries
# the git smart-HTTP surface (push/fetch). Each needs a different auth header
# shape. codeload/objects are unauthenticated (public archives, LFS) and stay
# on the plain egress pass-through, so they are deliberately absent here.
_API_HOST = "api.github.com"
_GIT_HOST = "github.com"
_GITHUB_HOSTS = frozenset({_API_HOST, _GIT_HOST})

# Env fallback (canary + tests, where the token rides the process env rather
# than the host file). MIKA_GITHUB_TOKEN mirrors the ~/.mika/.env var name.
_ENV_VARS = ("GH_TOKEN", "MIKA_GITHUB_TOKEN")

_token_cache: dict[str, object] = {"mtime": None, "value": None}


def _read_token() -> str | None:
    """Return the current GitHub token, host-side.

    Precedence:
      1. The dispatcher-staged host-only file (mtime-cached) — authoritative,
         survives App-installation-token rotation because the dispatcher
         rewrites it each spawn.
      2. The process environment (GH_TOKEN / MIKA_GITHUB_TOKEN) — the canary
         and the test suite inject the token this way.

    Never reads anything bound into the sandbox; the file lives under ~/.mika,
    which is not among the sandbox binds.
    """
    try:
        st = _TOKEN_FILE.stat()
    except OSError:
        st = None
    if st is not None:
        if _token_cache["mtime"] == st.st_mtime:
            cached = _token_cache["value"]
            if cached:
                return cached  # type: ignore[return-value]
        else:
            try:
                value = _TOKEN_FILE.read_text(encoding="utf-8").strip()
            except OSError:
                value = ""
            _token_cache["mtime"] = st.st_mtime
            _token_cache["value"] = value or None
            if value:
                return value
    for name in _ENV_VARS:
        env_val = os.environ.get(name)
        if env_val:
            return env_val.strip()
    return None


def _auth_header_for(host: str, token: str) -> str:
    """The Authorization value GitHub expects for this host.

    api.github.com accepts Bearer for every token class. github.com's
    git smart-HTTP endpoint authenticates via HTTP Basic with the token as the
    password and a sentinel username; `x-access-token` is the form that works
    for App installation tokens and PATs alike.
    """
    if host == _API_HOST:
        return f"Bearer {token}"
    basic = base64.b64encode(f"x-access-token:{token}".encode("utf-8")).decode("ascii")
    return f"Basic {basic}"


def requestheaders(flow: http.HTTPFlow) -> None:
    """Inject the GitHub credential as request headers first arrive.

    Injecting in `requestheaders` (not `request`) guarantees the rewrite lands
    before mitmdump forwards headers upstream even under aggressive body
    streaming — the same timing lesson the Anthropic addon records.
    """
    host = flow.request.host
    if host not in _GITHUB_HOSTS:
        return
    token = _read_token()
    if token is None:
        flow.response = http.Response.make(
            503,
            b"mika-pilot-github-auth: no GitHub token available host-side "
            b"(~/.mika/pilot-gh-token absent/empty and GH_TOKEN unset). The "
            b"dispatcher stages the token before each spawn; the sandbox never "
            b"holds it (mika#2056).\n",
            {"Content-Type": "text/plain; charset=utf-8"},
        )
        return
    for header_name in ("authorization", "proxy-authorization"):
        flow.request.headers.pop(header_name, None)
    flow.request.headers["Authorization"] = _auth_header_for(host, token)


def responseheaders(flow: http.HTTPFlow) -> None:
    """Log the upstream verdict once, read-only.

    The egress-proxy front sees only ciphertext on a CONNECT-tunnelled request,
    so this addon is the only place a GitHub status code is observable. Touches
    neither body nor stream — mirrors the Anthropic addon's #1901 tap. A 401
    here means the host-side token is missing scope / expired, and a bare
    `git push` failing inside the sandbox would otherwise be silent.
    """
    if flow.request.host not in _GITHUB_HOSTS or flow.response is None:
        return
    print(
        f"[github-proxy] ALLOW {flow.request.method} {flow.request.path} "
        f"-> {flow.response.status_code}",
        file=sys.stderr,
        flush=True,
    )
