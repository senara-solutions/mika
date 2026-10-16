"""mitmproxy addon: inject the GitHub credential host-side (mika#2056).

Loaded by mitmdump via `--scripts`, alongside the Anthropic addon, when the
pilot-egress relay chains a CONNECT github.com:443 / api.github.com:443 request
to it (see mika-pilot-egress-proxy `MITM_FORWARD_HOSTS`). mitmproxy terminates
the TLS against the sandbox client using its own CA — which the sandbox trusts
via the combined CA bundle wired in dispatch-lib.sh (GIT_SSL_CAINFO /
SSL_CERT_FILE / CURL_CA_BUNDLE, a *superset* of the system store so ordinary
verification of every other host is unaffected). This addon then rewrites the
decrypted HTTP request:

  * Strips any client-side Authorization / Proxy-Authorization — but, since
    mika#2724, only AFTER reading it: on api.github.com the client must present
    the per-spawn egress sentinel the dispatcher staged (mika#2716), or the
    request is refused 403. The sandbox still holds no real credential; what it
    holds is a justification, and the addon now demands it.
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

The refusal (mika#2724, vol 2 of mika#2716) is bounded to api.github.com. On
github.com the injection stays unconditional, because `git` presents NO
credential in the pilot sandbox (no credential helper, GIT_CONFIG_NOSYSTEM=1, a
gitconfig written from scratch) and GH_TOKEN's only sandbox reader is `gh`,
which talks to api.github.com alone. Requiring a credential on a channel where
none can travel would refuse every push and buy nothing.
"""

from __future__ import annotations

import base64
import hmac
import os
import sys
import time
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


# ---------------------------------------------------------------------------
# mika#2724 — the per-spawn egress sentinel is REQUIRED on api.github.com.
#
# Vol 1 (mika#2716, PR #2723) endowed every pilot sandbox with a sentinel and
# attested it host-side; it armed no refusal, so the legitimate call and the
# accidental one were merely DISTINGUISHABLE. This half refuses the second.
#
# THE FAIL-CLOSED ASYMMETRY, written here because it is a decision and not an
# implementation detail. Launcher-side (`_stage_pilot_egress_sentinel` in
# dispatch-lib.sh) an unreadable registry FAILS OPEN: it degrades to the bare
# placeholder and lets the dispatch live, because killing a dispatch over a
# bookkeeping failure costs a flight and protects nothing. Addon-side the same
# unreadable registry REFUSES, because the alternative is to authorise everyone
# at the exact instant we can no longer tell who is who — which is the property
# the ticket exists to buy. The two sides read the same directory and choose
# opposite defaults on purpose; see § 3.2 of the plan and the operator section
# of docs/operator/environment-variables.md.
# ---------------------------------------------------------------------------

# Same literal as `_PILOT_EGRESS_SENTINEL_PREFIX` in dispatch-lib.sh, and the
# same env var names — deliberately, so the two readers of one switch and one
# registry are greppable together (mika#2165).
_SENTINEL_PREFIX = "proxy-managed-no-secret"
_SENTINEL_SWITCH = "MIKA_PILOT_EGRESS_SENTINEL"
_SENTINEL_DIR_ENV = "_PILOT_EGRESS_SENTINEL_DIR"
_SWITCH_OFF = frozenset({"0", "false", "no", "off", "disabled"})

_ALLOW = "allow"
_DENY_ABSENT = "sentinel-absent"
_DENY_UNKNOWN = "sentinel-unknown"
_DENY_REGISTRY = "registry-unreadable"

# A registry entry outlives its pilot by at most three days. No env var: a TTL
# an operator can set to zero is a registry an operator can empty by accident,
# and the value is not the kind of thing a site needs to differ on.
_REGISTRY_TTL_SECONDS = 72 * 3600
_PURGE_MIN_INTERVAL_SECONDS = 600
_last_purge: float | None = None

# Three reasons, three bodies. Their causes are OPPOSITE — absent is the
# `cargo test` that cleared GH_TOKEN (the mika#2713 substrate), unknown is an
# entry purged too early or a registry that is not the one we think, and
# unreadable is the host's own bookkeeping — so a single body would make the
# difference undiagnosable at the moment it decides the remedy (mika#2205).
# None of them carries the presented value or the host token, nor a digest of
# either: a fingerprint is deferred disclosure (mika#1949).
_DENY_BODIES = {
    _DENY_ABSENT: (
        "mika-pilot-github-auth: DENY sentinel-absent (mika#2716 / mika#2724).\n"
        "\n"
        "This request presented no credential at all, so it cannot be the pilot\n"
        "sandbox as the dispatcher staged it: every spawn is given\n"
        "GH_TOKEN=proxy-managed-no-secret-<nonce>.\n"
        "\n"
        "Remedy: find what cleared or overwrote GH_TOKEN in this process tree.\n"
        "A test harness that sets it to the empty string is the known case\n"
        "(mika#2713). The sandbox never holds a real token, so nothing is\n"
        "leaked by this refusal -- the write to the real forge is what it stops.\n"
        "Rollback: MIKA_PILOT_EGRESS_SENTINEL=0 disarms both halves at once.\n"
    ),
    _DENY_UNKNOWN: (
        "mika-pilot-github-auth: DENY sentinel-unknown (mika#2716 / mika#2724).\n"
        "\n"
        "A credential was presented, it is not the bare placeholder, and it\n"
        "matches no entry of the host-side registry\n"
        "~/.mika/pilot-egress-sentinels/.\n"
        "\n"
        "Remedy: this is a sentinel the host does not know. Either the entry was\n"
        "purged before the pilot finished (TTL 72 h -- a flight older than that\n"
        "needs a respawn), or the registry being read is not the one the\n"
        "dispatcher wrote (check _PILOT_EGRESS_SENTINEL_DIR and HOME of the\n"
        "mitmdump process). List the registry to see which.\n"
        "Rollback: MIKA_PILOT_EGRESS_SENTINEL=0 disarms both halves at once.\n"
    ),
    _DENY_REGISTRY: (
        "mika-pilot-github-auth: DENY registry-unreadable (mika#2716 / mika#2724).\n"
        "\n"
        "The host-side registry ~/.mika/pilot-egress-sentinels/ could not be\n"
        "read, so no credential can be validated. Refusing is deliberate and\n"
        "the exact opposite of the launcher, which fails OPEN on the same\n"
        "failure: authorising everyone at the instant we can no longer tell who\n"
        "is who would forfeit the property this guard exists for.\n"
        "\n"
        "Remedy: the fault is host-side, not in the sandbox. Check that the\n"
        "directory exists, is a directory, and is readable by the mitmdump user\n"
        "(0700, owned by it). It is created by the dispatcher at the first\n"
        "spawn.\n"
        "Rollback: MIKA_PILOT_EGRESS_SENTINEL=0 disarms both halves at once.\n"
    ),
}


def _sentinel_enforced() -> bool:
    """Is the guard armed in THIS process?

    The second reader of the operator switch (mika#2165: a switch its second
    reader does not honour is decorative). Same vocabulary as the shell half,
    and the same default — unset or empty means armed, exactly as
    `${MIKA_PILOT_EGRESS_SENTINEL:-1}` resolves.
    """
    raw = os.environ.get(_SENTINEL_SWITCH)
    if raw is None or not raw.strip():
        return True
    return raw.strip().lower() not in _SWITCH_OFF


def _registry_dir() -> Path:
    """Where the host keeps one file per live sentinel.

    Resolved per call, not at import: mitmdump is long-lived, and a path frozen
    at load would ignore a relocated registry until a proxy restart.
    """
    override = os.environ.get(_SENTINEL_DIR_ENV)
    if override:
        return Path(override)
    return Path.home() / ".mika" / "pilot-egress-sentinels"


def _presented_credential(headers) -> str | None:
    """The credential the CLIENT presents, read before the addon pops it.

    Three schemes, because the clients do not present alike:
      * `Bearer <v>` — the generic form, and what a hand-rolled curl sends.
      * `token <v>`  — what `gh` actually emits. Captured on the wire for this
        ticket against gh v2.92.0 on REST, on GraphQL and through a real
        subcommand: `Authorization: token <tok>`, never Bearer. A Bearer-only
        reader would have 403'd every `gh` call in the loop (risk R2).
      * `Basic <b64>` — decoded, the part after the first colon (the password
        field; git smart-HTTP puts the credential there).

    Proxy-Authorization is read with the same precedence as Authorization, so
    nothing travels unexamined through the second header.

    Returns None rather than raising on anything malformed. A reader that
    throws is a 500 on every request; a reader that returns None lands on
    `sentinel-absent`, which is a refusal with a diagnosis. Never an allow.
    """
    for header_name in ("Authorization", "Proxy-Authorization"):
        raw = headers.get(header_name)
        if not raw:
            continue
        parts = raw.strip().split(None, 1)
        if len(parts) != 2:
            continue
        scheme, value = parts[0].lower(), parts[1].strip()
        if not value:
            continue
        if scheme in ("bearer", "token"):
            return value
        if scheme == "basic":
            try:
                decoded = base64.b64decode(value).decode("utf-8")
            except (ValueError, TypeError):
                continue
            if ":" not in decoded:
                continue
            password = decoded.split(":", 1)[1].strip()
            if password:
                return password
    return None


def _purge_registry(directory: Path) -> None:
    """Drop entries older than the TTL, at most once every ten minutes.

    Without this, one file accumulates per spawn forever. It runs BEFORE the
    returned list is computed and never after a verdict, so a purge can only
    ever remove an entry that was already too old to authorise anything — in
    particular the entry validated during this very call survives, because the
    cutoff is absolute and not relative to what is being looked up.

    Every OSError is swallowed per entry: a deletion we lose is a file that
    will be reconsidered in ten minutes, whereas a purge failure that
    propagated would read as an unreadable registry and refuse a legitimate
    pilot. The throttle uses a monotonic clock, so a host clock step cannot
    stall it forever.
    """
    global _last_purge
    now_mono = time.monotonic()
    if _last_purge is not None and now_mono - _last_purge < _PURGE_MIN_INTERVAL_SECONDS:
        return
    _last_purge = now_mono
    cutoff = time.time() - _REGISTRY_TTL_SECONDS
    try:
        entries = list(os.scandir(directory))
    except OSError:
        return
    for entry in entries:
        try:
            if entry.stat().st_mtime < cutoff:
                os.unlink(entry.path)
        except OSError:
            continue


def _registry_nonces() -> frozenset[str] | None:
    """The nonces the host currently attests, or None if the registry is unreadable.

    None — and ONLY None — means unreadable: directory absent, not a directory,
    PermissionError, any OSError. A registry that is readable and empty returns
    an empty frozenset, which is a DIFFERENT state: it yields
    `sentinel-unknown` (nothing is attested right now) and not
    `registry-unreadable` (we cannot tell). Collapsing the two would name the
    wrong remedy on the day it matters (mika#2205).
    """
    directory = _registry_dir()
    _purge_registry(directory)
    try:
        with os.scandir(directory) as entries:
            return frozenset(entry.name for entry in entries if entry.is_file())
    except OSError:
        return None


def _sentinel_verdict(presented: str | None) -> str:
    """One of _ALLOW / _DENY_ABSENT / _DENY_UNKNOWN / _DENY_REGISTRY.

    The branch ORDER carries the safety:

    1. The bare placeholder is accepted FIRST, without touching the registry.
       That is what makes `MIKA_PILOT_EGRESS_SENTINEL=0` a whole rollback: the
       launcher then presents the bare prefix, and the acceptance must not
       depend on a registry that may be the very thing that is broken.
    2. Nothing presented is `sentinel-absent` — a cause of its own.
    3. An unreadable registry refuses (see the asymmetry note above). Checked
       before the comparison, because a comparison against nothing is not an
       answer.
    4. Otherwise the presented value is compared against every attested nonce
       with `hmac.compare_digest`, never `==`: the comparison bears on a
       justification, so a timing leak is free to avoid. The registry is read
       with scandir and never by looking up a filename derived from the
       presented value — a credential is attacker-influenced input, and
       `registry / presented` would be a path traversal.
    """
    if presented == _SENTINEL_PREFIX:
        return _ALLOW
    if not presented:
        return _DENY_ABSENT
    nonces = _registry_nonces()
    if nonces is None:
        return _DENY_REGISTRY
    candidate = presented.encode("utf-8", "surrogateescape")
    for nonce in nonces:
        expected = f"{_SENTINEL_PREFIX}-{nonce}".encode("utf-8")
        if hmac.compare_digest(candidate, expected):
            return _ALLOW
    return _DENY_UNKNOWN


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
    # The egress sentinel (mika#2724), on api.github.com only. Read the client
    # credential BEFORE the strip below destroys it, and refuse rather than
    # inject when it is not one the host attests. github.com falls through
    # untouched — see the module docstring: no credential can travel there in
    # the pilot sandbox, so the requirement is bounded to the channel that
    # carries one. The 503 above stays ahead of the verdict: a host with no
    # token has nothing to protect, and the operator needs that diagnosis
    # first.
    if host == _API_HOST and _sentinel_enforced():
        verdict = _sentinel_verdict(_presented_credential(flow.request.headers))
        if verdict != _ALLOW:
            # One greppable line per refusal, mirroring the ALLOW tap below.
            # The reason, the method and the path — never the presented value,
            # never the host token, never a digest of either (mika#1949).
            print(
                f"[github-proxy] DENY {verdict} "
                f"{flow.request.method} {flow.request.path}",
                file=sys.stderr,
                flush=True,
            )
            flow.response = http.Response.make(
                403,
                _DENY_BODIES[verdict].encode("utf-8"),
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
