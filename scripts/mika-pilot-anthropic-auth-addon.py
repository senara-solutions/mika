"""mitmproxy addon: inject Anthropic subscription OAuth on api.anthropic.com.

Loaded by mitmdump via `--scripts` when the pilot-egress relay chains a
CONNECT api.anthropic.com:443 request to it (see mika-pilot-egress-proxy).
mitmproxy handles the TLS termination against the sandbox client using its
own CA (which the sandbox trusts via NODE_EXTRA_CA_CERTS + SSL_CERT_FILE
bind, wired in dispatch-lib.sh). This addon then rewrites the decrypted
HTTP request:

  * Strips client-side Authorization / X-API-Key / Proxy-Authorization
    (defensive — the sandbox has only a placeholder key; whatever it sent
    is not what we want upstream to see).
  * Injects Authorization: Bearer <token> where <token> comes from the
    subscription OAuth cred at ~/.claude/.credentials.json, host-side.
    Token cache is mtime-invalidated so a Claude Code CLI refresh on the
    host propagates on the next request without a mitmdump restart.

Property:
  * Sandbox NEVER sees the subscription token — it stays in the mitmdump
    process (host), read once per file-mtime-change from a file that is
    never bind-mounted into the sandbox.
  * The mitmproxy CA cert (public) is bound into the sandbox trust store;
    the mitmproxy CA PRIVATE key stays host-side. So a compromised pilot
    can validate our cert chain (needed to speak TLS with us) but cannot
    forge a new cert (which would need the private key).

This addon does nothing for non-Anthropic hosts — mitmdump also handles
CONNECT to other hosts (github, registries, etc.) via pass-through when
we don't touch the request in this addon. In practice we chain ONLY
api.anthropic.com CONNECTs to mitmdump from the egress-proxy front (see
handle_host_client dispatch); other hosts stay on the encrypted-tunnel
pass-through path.
"""

from __future__ import annotations

import json
import time
from pathlib import Path

from mitmproxy import http

_CREDS_PATH = Path.home() / ".claude" / ".credentials.json"
_OAUTH_TOKEN_PREFIX = "sk-ant-oat"
_ANTHROPIC_HOST = "api.anthropic.com"

_token_cache: dict[str, object] = {"mtime": None, "value": None}


def _extract_oauth_token(obj) -> str | None:
    """Walk a JSON-decoded object looking for the first string that starts
    with sk-ant-oat. Handles nested/flat/snake_case shapes uniformly."""

    def _walk(node):
        if isinstance(node, dict):
            for v in node.values():
                if isinstance(v, str) and v.startswith(_OAUTH_TOKEN_PREFIX):
                    return v
            for v in node.values():
                found = _walk(v)
                if found is not None:
                    return found
        elif isinstance(node, list):
            for item in node:
                found = _walk(item)
                if found is not None:
                    return found
        return None

    return _walk(obj)


def _read_subscription_token() -> str | None:
    """Return the current OAuth access_token from ~/.claude/.credentials.json.
    Cached by file mtime — a CLI refresh (which rewrites the file) propagates
    on the next request; no addon reload needed."""
    try:
        st = _CREDS_PATH.stat()
    except OSError:
        return None
    if _token_cache["mtime"] == st.st_mtime:
        return _token_cache["value"]  # type: ignore[return-value]
    try:
        with _CREDS_PATH.open("r", encoding="utf-8") as fh:
            data = json.load(fh)
    except (OSError, json.JSONDecodeError):
        return None
    token = _extract_oauth_token(data)
    _token_cache["mtime"] = st.st_mtime
    _token_cache["value"] = token
    return token


def request(flow: http.HTTPFlow) -> None:
    """Called by mitmproxy on every request. We only rewrite for api.anthropic.com."""
    if flow.request.host != _ANTHROPIC_HOST:
        return
    token = _read_subscription_token()
    if token is None:
        flow.response = http.Response.make(
            503,
            b"mika-pilot-anthropic-auth: subscription OAuth token not found in "
            b"~/.claude/.credentials.json; host operator must be logged in to "
            b"Claude Code for the pilot to authenticate.\n",
            {"Content-Type": "text/plain; charset=utf-8"},
        )
        return
    # Strip any client-side auth (sandbox has only a placeholder value).
    for header_name in ("authorization", "x-api-key", "proxy-authorization"):
        flow.request.headers.pop(header_name, None)
    flow.request.headers["Authorization"] = f"Bearer {token}"
    # Soft-warn if token appears expired (mitmdump log — visible in stderr).
    # Anthropic will 401 anyway but this makes it grep-able.
    # (We don't have direct access to expires_at without another parse pass.)
