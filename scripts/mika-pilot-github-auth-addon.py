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

The JUSTIFICATION-based refusal (mika#2724, vol 2 of mika#2716) is bounded to
api.github.com. On github.com no credential is DEMANDED, because `git` presents
none in the pilot sandbox (no credential helper, GIT_CONFIG_NOSYSTEM=1, a
gitconfig written from scratch) and GH_TOKEN's only sandbox reader is `gh`,
which talks to api.github.com alone. Requiring a credential on a channel where
none can travel would refuse every push and buy nothing.

But github.com is not therefore unbounded (mika#2736). The measurement that
closed the sentinel question also showed the proxy needs to be TOLD nothing: a
`git-receive-pack` carries its repository in the URL and its ref commands in
clear at the head of the body. So instead of a justification the addon applies a
BOUND on what it already sees — four rules, in `request` below. A bound asks
nothing of the client, which makes it strictly stronger on this half than a
sentinel would have been: no process of the sandbox gets around it by presenting
itself well. What stays out of reach is the force-push (receive-pack carries no
force flag, so a non-fast-forward is indistinguishable from an ordinary update
without the remote's prior ref state); it is a NAMED limit, not an oversight.
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

# A SENTINEL registry entry outlives its pilot by at most three days. No env
# var: a TTL an operator can set to zero is a registry an operator can empty by
# accident.
_REGISTRY_TTL_SECONDS = 72 * 3600

# mika#2766: the PERIMETER registry gets its OWN TTL, and the two values answer
# different questions. 72 h is calibrated on "can a flight still be read three
# days later?" (mika#2724). This registry answers "is this perimeter still in
# flight?", and a three-day-old entry never is.
#
# THE ERROR IS ASYMMETRIC, and this is the whole reason the value is derived
# rather than picked. `mitmdump` is shared between dispatches, so the reader's
# verdict is the UNION of the live entries — and `_push_scope_verdict` only
# degrades when the registry is empty or unreadable, never per entry. So with
# two concurrent dispatches, purging the entry of a pilot that is STILL ALIVE
# does not degrade to the static bound: the registry still holds its
# neighbour's entry, so it is neither empty nor unreadable, and the live
# pilot's legitimate push lands on `_DENY_SCOPE_REPO`.
#
#   too LONG  : an entry outlives a SIGKILLed pilot; the union stays bounded by
#               the dispatchable repositories, so nothing new is reachable.
#   too SHORT : a live pilot is 403'd while a neighbour holds the registry
#               non-empty — the loop stops, and on an innocent pilot.
#
# The TTL is therefore a FLOOR on the longest spawn, never a housekeeping
# bound one tightens to look tidy.
#
# THE MEASURE, so the next person to touch this reads a number and not an
# adjective. There is no wall-clock timeout on a spawn anywhere in
# `dispatch-lib.sh` (the only `timeout` in that file is the 15 s smoke test),
# so the real bound is the turn ceiling — 200 turns for a `loop-substrate`
# ticket. Measured over the 718 pilot transcripts on this host
# (`~/.mika/data/pilot-transcripts`, one per spawn, last timestamp minus
# first): median 11.9 min, p90 55.2 min, p99 1.89 h, LONGEST 4.40 h. 12 h is
# 2.7x that longest spawn. 4 h would already be under it, which is the side
# that stops the loop.
#
# No env var here either, and the reason is stronger than for its elder: by the
# asymmetry above, a TTL an operator can set to zero is not a registry emptied
# by accident, it is a 403 on every live pilot whose neighbour holds the
# registry non-empty. The worst of the two imaginable switches.
_PUSH_SCOPE_TTL_SECONDS = 12 * 3600

_PURGE_MIN_INTERVAL_SECONDS = 600

# One throttle PER directory, not one for the whole process. `_purge_registry`
# takes its directory as an argument, so a single global clock means the first
# directory purged in a window starves every other one until the window closes
# -- and the registry read on the next request is the starved one as often as
# not. With a dict the throttle keeps its purpose (no scandir per request) and
# loses the coupling between two registries that have nothing to do with each
# other (mika#2751 § 2.5).
_last_purge: dict[str, float] = {}

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


def _purge_registry(directory: Path, ttl_seconds: int) -> None:
    """Drop entries older than `ttl_seconds`, at most once every ten minutes.

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

    The throttle is keyed by the directory, never global: this addon purges
    more than one registry, and a global clock would let the first one purged
    in a ten-minute window keep every other one stale for the rest of it.

    The TTL comes IN, for the second half of that same reason (mika#2766). The
    throttle stopped being a module-level float once this function served two
    registries; the TTL is the other coupling of the same shape, and the two
    registries answer questions whose answers age differently
    (`_REGISTRY_TTL_SECONDS` vs `_PUSH_SCOPE_TTL_SECONDS`). A `if directory ==
    …` inside the purge would be one more reader for a datum the CALLER already
    holds.
    """
    key = str(directory)
    now_mono = time.monotonic()
    last = _last_purge.get(key)
    if last is not None and now_mono - last < _PURGE_MIN_INTERVAL_SECONDS:
        return
    _last_purge[key] = now_mono
    cutoff = time.time() - ttl_seconds
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
    _purge_registry(directory, _REGISTRY_TTL_SECONDS)
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


# ---------------------------------------------------------------------------
# mika#2736 — on github.com a `git-receive-pack` is BOUNDED, not questioned.
#
# WHY A BOUND AND NOT A SENTINEL. No channel can carry a justification that
# attests the legitimate PATH rather than the sandbox: `http.extraHeader` via
# GIT_CONFIG_KEY_n does reach the proxy, and a second process in the same
# environment presents the very same valid header while pushing wherever it
# likes. A bound on what the proxy ALREADY SEES — the repository in the URL,
# the ref commands in clear at the head of the body — depends on no cooperation
# from the client at all, so presenting itself well buys an attacker nothing.
#
# WHAT EACH RULE CLOSES, and the one thing it does not:
#   * push-repo-foreign  — exfiltration to a third-party repository. Irreparable
#     once it lands: a push to someone else's repo cannot be taken back.
#   * push-ref-delete    — the destruction of a ref.
#   * push-ref-exotic    — tags and arbitrary refs/*, which no pilot writes.
#   * push-ref-protected — a write straight onto the integration branch.
#   * The force-push on the spawn's own ref (the founding incident, mika#1318)
#     stays OUT OF REACH here: receive-pack states `<old> <new> <ref>` and
#     carries NO force flag, so a rewrite is byte-identical to an ordinary
#     update unless the proxy holds the remote's prior ref state. Naming the
#     limit is the deliverable; pretending to cover it would be worse than the
#     gap. Tightening to the perimeter the SPAWN declared is mika#2751, below,
#     and it changes nothing about P4: the targeted ref becomes the right one,
#     the nature of the update stays invisible.
# ---------------------------------------------------------------------------

# The one repository path prefix a pilot may write to. STATIC, and it stays:
# mika#2751 tightens to the repository the host itself declared at spawn, but
# SUBORDINATELY — see `_push_scope_verdict` below. This bound keeps applying
# whenever no perimeter is declared or readable.
_PUSH_ALLOWED_OWNER = "senara-solutions"

# Whole-name match, never a substring: `refs/heads/feat/2736/main-bound` is
# ordinary pilot work, and a bound that refused it would refuse real flights.
_PUSH_PROTECTED_REFS = frozenset({"refs/heads/main", "refs/heads/master"})
_PUSH_REF_PREFIX = "refs/heads/"

_RECEIVE_PACK_SUFFIX = "/git-receive-pack"

# Its own switch, NOT _SENTINEL_SWITCH. The two guards fail for unrelated
# reasons — one on a host registry, one on a body too large to read — and an
# operator disarming either must not disarm the other unknowingly (mika#2165:
# one switch, and every reader of it honours it).
_PUSH_BOUND_SWITCH = "MIKA_PILOT_EGRESS_PUSH_BOUND"

_DENY_REPO_FOREIGN = "push-repo-foreign"
_DENY_REF_DELETE = "push-ref-delete"
_DENY_REF_EXOTIC = "push-ref-exotic"
_DENY_REF_PROTECTED = "push-ref-protected"
_DENY_PUSH_OPAQUE = "push-body-unreadable"

# ---------------------------------------------------------------------------
# THE DECLARED PERIMETER (mika#2751 for the repository, mika#2776 for the branch
# — vols 2 and 4 of mika#2736)
#
# The bound above is static: it authorises `refs/heads/<anything>` under
# `senara-solutions/<any repo>`. So a pilot dispatched on one ticket may still
# push to mika-platform, mika-cloud, mika-skills — subclass P3 of the threat
# model, its RELATIONAL half ("is this THE repository of THIS dispatch?" rather
# than "is this a branch?").
#
# The perimeter travels by a HOST FILE RE-READ PER REQUEST, never by the
# mitmdump environment. Measured, not assumed (§ 2.1 of the plan):
# `_ensure_pilot_helper` returns 0 when the port is already alive, so the
# daemon is shared across dispatches and its environment is the FIRST
# dispatch's, forever. A perimeter passed by env would therefore name the wrong
# repository for every spawn but one — silently, and in the widening direction.
# `~/.mika` is not among the sandbox binds, so the registry is unforgeable from
# inside the sandbox: the same host/sandbox asymmetry mika#2724 draws its
# sentinel from.
#
# UNION SEMANTICS, imposed and not chosen (§ 2.2). One shared unix socket and
# no credential on github.com means a request cannot be attributed to a spawn.
# So the verdict is "does SOME live entry declare this repository?", not "does
# THIS spawn's entry". For the repository dimension that is cheap: a pilot has
# exactly one possible `origin` (one parent `.git` is bind-mounted, § 2.3) and
# `DISPATCHABLE_REPO_BASENAMES` holds four names, so the union can never exceed
# four repos and is insensitive to the TTL.
#
# THE UNION IS BY ENTRY, NOT BY DIMENSION (mika#2776). The branch dimension
# makes that distinction load-bearing where the repository dimension alone could
# not show it. A per-dimension union — the set of all declared repos, the set of
# all declared refs, judged independently — would authorise the CROSS PRODUCT:
# with `(mika, feat/A)` and `(mika-platform, fix/B)` both in flight, a push of
# `mika → fix/B` satisfies both sets and lands, which is the exact scenario the
# ticket opens on. So an entry is read as a COUPLE, the repository selects which
# couples may speak, and only the refs of THOSE couples are allowed. The union
# over branches is still unbounded in cardinality — one more name per live
# dispatch — which is why the branch dimension was worth nothing until the
# entries learned to die with their spawn (mika#2766), and why it ships after.
#
# DEGRADATION, not refusal, on every failure of this channel — the exact
# opposite posture to the sentinel's, deliberately. The sentinel answers "is
# this the pilot?" and an unreadable registry means we no longer know, so it
# refuses. This one answers "is it the right repository?" and an unreadable
# registry means we never learned, so it falls back to the static bound. Two
# contracts, two registries, two switches; collapsing them would make one datum
# carry both (§ 3.2).
# ---------------------------------------------------------------------------

_DENY_SCOPE_REPO = "push-scope-repo"

# The branch dimension's own motif, distinct from the repository's by
# requirement and not by taste (mika#2205, point 1 of mika#2776): the two
# refusals have DISJOINT remedies — one sends the operator to the worktree's
# `origin`, the other to the refspec the push resolved — so arriving in the same
# bytes would make them undiagnosable at the moment they decide what to do.
_DENY_SCOPE_REF = "push-scope-ref"

# Not a refusal: an ALLOW that says why the tightening did not apply. Three
# distinct tokens for three OPPOSITE causes (mika#2205) — absent is a dispatch
# that declared nothing (free-text mode, or a launcher that fail-opened),
# unreadable is the host's own bookkeeping, ref-absent is a live entry written
# by a binary that predates the branch dimension — and they name three
# different remedies. The third one in particular must NOT read as a fault: the
# host registry survives a deployment, so an entry without `ref=` is the normal
# state for one window after the upgrade (point 2 of mika#2776), and the only
# correct response is to keep applying the repository dimension and say so.
_DEGRADE_SCOPE_ABSENT = "scope-absent"
_DEGRADE_SCOPE_UNREADABLE = "scope-unreadable"
_DEGRADE_SCOPE_REF_ABSENT = "scope-ref-absent"
_PUSH_SCOPE_DEGRADED = frozenset(
    {_DEGRADE_SCOPE_ABSENT, _DEGRADE_SCOPE_UNREADABLE, _DEGRADE_SCOPE_REF_ABSENT}
)

# Its own switch, subordinate to `_PUSH_BOUND_SWITCH` and never a substitute
# for it: point 3 of the ticket asks that the TIGHTENING be disarmable without
# disarming the static bound. The converse is deliberately impossible — see
# `_push_scope_verdict`. ONE switch for the WHOLE tightening, both dimensions
# together: point 3 of mika#2776 refuses a third name on this surface, because
# a recourse bigger than the problem is how a surface stops being understood.
_PUSH_SCOPE_SWITCH = "MIKA_PILOT_EGRESS_PUSH_SCOPE"
_PUSH_SCOPE_DIR_ENV = "_PILOT_EGRESS_PUSH_SCOPE_DIR"

# `key=value` lines, two keys today. Unknown keys are IGNORED on read, which is
# what made `ref=` a pure addition when the branch dimension landed: no
# migration, no format version, and no second reader to update.
_PUSH_SCOPE_REPO_KEY = "repo="
_PUSH_SCOPE_REF_KEY = "ref="

_HEX_DIGITS = frozenset(b"0123456789abcdefABCDEF")

# Seven reasons, seven bodies, and no body names another's motif — the remedies
# are disjoint, so a shared body would make the difference undiagnosable at the
# moment it decides what to do (mika#2205, the same reasoning as the three
# sentinel bodies above). `{subject}` is the offending repository or ref: that
# is CLIENT data, not a host secret, so naming it discloses nothing. mika#1949
# forbids naming the JUSTIFICATION presented, never the target aimed at.
_PUSH_DENY_BODIES = {
    _DENY_REPO_FOREIGN: (
        "This push targets a repository outside the perimeter: {subject}\n"
        "\n"
        "A pilot sandbox may write only under the `" + _PUSH_ALLOWED_OWNER + "`\n"
        "organisation. A push landing in a third-party repository is the one\n"
        "outcome of this class that cannot be undone afterwards, which is why it\n"
        "is refused rather than merely logged.\n"
        "\n"
        "Remedy: check the `origin` URL of the worktree this push came from. The\n"
        "dispatcher sets it; a rewritten remote is the case to look for.\n"
    ),
    _DENY_REF_DELETE: (
        "This push DELETES a ref: {subject}\n"
        "\n"
        "A deletion is read off the protocol itself -- the command carries an\n"
        "all-zero new object id -- and never off a heuristic. Nothing a pilot is\n"
        "dispatched to do removes a ref on the forge.\n"
        "\n"
        "Remedy: if a branch really must go, delete it from the host, outside the\n"
        "sandbox. Inside it, find what ran a `git push --delete` or a `:ref`\n"
        "refspec.\n"
    ),
    _DENY_REF_EXOTIC: (
        "This push writes a ref outside `" + _PUSH_REF_PREFIX + "`: {subject}\n"
        "\n"
        "A pilot pushes branches and nothing else. Tags, notes, and arbitrary\n"
        "`refs/*` entries are write surfaces it never needs, and each of them is\n"
        "a way to leave content on the forge that the ordinary branch view does\n"
        "not show.\n"
        "\n"
        "Remedy: push the work as a branch under the dispatch prefix. A release\n"
        "tag is an operator action, taken outside the sandbox.\n"
    ),
    _DENY_REF_PROTECTED: (
        "This push writes straight onto an integration branch: {subject}\n"
        "\n"
        "Pilot work reaches the integration branch through a pull request and its\n"
        "pipeline, never directly. This refusal is the structural half of that\n"
        "rule: a convention nothing enforces is a convention one bad refspec\n"
        "undoes.\n"
        "\n"
        "Remedy: push the dispatch branch and open a PR. If the branch name was\n"
        "resolved from a variable, that variable is what to inspect.\n"
    ),
    _DENY_PUSH_OPAQUE: (
        "The command section of this push could not be read, so what it touches\n"
        "cannot be judged -- and it is refused rather than waved through.\n"
        "\n"
        "The likely cause is SIZE: mitmproxy streams a body past its threshold\n"
        "(`stream_large_bodies=10m`) and the content is then simply not here to\n"
        "inspect. A pilot pack is of the order of a kilobyte, so a push this\n"
        "large is already the anomaly. A malformed pkt-line section lands here\n"
        "too. Allowing on unreadability would hand anyone a trivial bypass:\n"
        "grow the body.\n"
        "\n"
        "Remedy: look at what made the push this big -- a committed artifact, a\n"
        "vendored tree, a binary. If the size is legitimate, use the rollback\n"
        "below; it is the reason a fail-closed bound is acceptable on the path of\n"
        "the autonomous loop.\n"
    ),
    _DENY_SCOPE_REPO: (
        "This push targets a repository the dispatch did not declare: {subject}\n"
        "\n"
        "It is inside the `" + _PUSH_ALLOWED_OWNER + "` organisation, so the\n"
        "static bound let it through -- but no live entry of the host-side\n"
        "perimeter registry names it. A pilot has exactly one possible `origin`\n"
        "(one parent .git is bind-mounted into the sandbox), so a push to a\n"
        "sibling repository of the organisation is never ordinary work.\n"
        "\n"
        "Remedy: check which repository the dispatch declared, host-side, by\n"
        "listing ~/.mika/pilot-push-scopes/ (one file per live spawn, each a\n"
        "`repo=` line). If the declared value is wrong, the fault is in the\n"
        "launcher that wrote it; if the pushed value is wrong, look at the\n"
        "`origin` URL of the worktree. The rollback below disarms this\n"
        "tightening alone and leaves the organisation bound in place.\n"
    ),
    _DENY_SCOPE_REF: (
        "This push targets a branch the dispatch did not declare: {subject}\n"
        "\n"
        "The REPOSITORY is one the dispatch declared, so the repository\n"
        "dimension of the perimeter let it through -- but no live entry naming\n"
        "that repository declares this ref. The case this closes is a push onto\n"
        "ANOTHER in-flight dispatch's branch of the same repository: the\n"
        "organisation is right, the ref is an ordinary branch under\n"
        "`" + _PUSH_REF_PREFIX + "`, and it is neither `main` nor `master`, so\n"
        "every static rule is satisfied and only the declared branch can refuse\n"
        "it.\n"
        "\n"
        "Remedy: this is a REFSPEC fault, not a remote fault -- look at what\n"
        "resolved the branch name at the push site, not at the `origin` URL.\n"
        "Check which branch the dispatch declared, host-side, by listing\n"
        "~/.mika/pilot-push-scopes/ (one file per live spawn, each a `repo=`\n"
        "line and, since mika#2776, a `ref=` line). An entry with no `ref=` at\n"
        "all declares no branch and cannot produce this refusal: it degrades to\n"
        "the repository dimension instead. The rollback below disarms this\n"
        "tightening alone and leaves the organisation bound in place.\n"
    ),
}

# Which ticket each motif belongs to. Named per motif rather than hardcoded,
# so a refused operator lands on the flight that decided the refusal and not on
# whichever one happened to write the formatting.
_PUSH_DENY_TICKETS = {
    _DENY_SCOPE_REPO: "mika#2751",
    _DENY_SCOPE_REF: "mika#2776",
}
_PUSH_DENY_DEFAULT_TICKET = "mika#2736"

_PUSH_DENY_FOOTER = (
    "Rollback: " + _PUSH_BOUND_SWITCH + "=0 disarms THIS bound alone, in the\n"
    "environment of the mitmdump process. It leaves the api.github.com refusal\n"
    "of mika#2724 and the host-side credential injection of mika#2056 intact.\n"
    + _PUSH_SCOPE_SWITCH + "=0 disarms the declared-perimeter tightening of\n"
    "mika#2751 only, leaving the organisation bound above armed. It disarms\n"
    "BOTH of its dimensions together -- the repository and the branch of\n"
    "mika#2776 -- because the perimeter is one rule read from one registry, and\n"
    "a second switch on this surface would be a recourse bigger than the\n"
    "problem.\n"
)


def _push_bound_enforced() -> bool:
    """Is the bound armed in THIS process?

    Same vocabulary and same default as `_sentinel_enforced` — unset or empty
    means armed — but a DIFFERENT variable, on purpose: see the note on
    `_PUSH_BOUND_SWITCH`.
    """
    raw = os.environ.get(_PUSH_BOUND_SWITCH)
    if raw is None or not raw.strip():
        return True
    return raw.strip().lower() not in _SWITCH_OFF


def _receive_pack_repo(path: str) -> str | None:
    """`<owner>/<repo>` when this path is a push, None when it is not.

    None means "not a push" and lets the request through untouched. That is the
    whole non-regression surface of this ticket: `git-upload-pack` (the fetch),
    `info/refs?service=git-receive-pack` (the advertisement, a GET), and every
    other github.com path keep the behaviour they had before. The bound bears
    on the only verb that WRITES — a fetch reads public data, destroys nothing
    and exfiltrates nothing.

    The query string is cut before matching, so the advertisement cannot be
    mistaken for the push just because it names the service.
    """
    route = path.split("?", 1)[0]
    if not route.endswith(_RECEIVE_PACK_SUFFIX):
        return None
    prefix = route[: -len(_RECEIVE_PACK_SUFFIX)]
    parts = prefix.split("/")
    # A smart-HTTP push path is exactly `/<owner>/<repo>[.git]/git-receive-pack`,
    # so the split yields ['', owner, repo] and nothing else. Anything deeper or
    # shallower is not a repository root and is not judged.
    if len(parts) != 3 or parts[0] != "" or not parts[1] or not parts[2]:
        return None
    owner, repo = parts[1], parts[2]
    if repo.endswith(".git"):
        repo = repo[: -len(".git")]
    if not repo:
        return None
    return f"{owner}/{repo}"


def _parse_ref_command(payload: bytes) -> tuple[str, str, str] | None:
    """One `<old> SP <new> SP <ref>` triplet, or None if the line is not one.

    The capability list rides on the FIRST command only, after a NUL; it is cut
    before parsing so the reader bears on the refname and never on whatever
    capabilities git happens to negotiate (`side-band-64k`, `agent=…`,
    `atomic`, …). Returning None on any other shape — a `shallow` line, a push
    certificate, a truncated triplet — is deliberate: an unrecognised command
    section is `push-body-unreadable`, which refuses with a diagnosis, rather
    than an empty command list, which would allow.
    """
    line = payload.split(b"\x00", 1)[0].strip()
    parts = line.split(b" ")
    if len(parts) != 3:
        return None
    try:
        old, new, ref = (part.decode("utf-8") for part in parts)
    except UnicodeDecodeError:
        return None
    if not old or not new or not ref:
        return None
    return old, new, ref


def _ref_commands(body: bytes | None) -> list[tuple[str, str, str]] | None:
    """The ref commands at the head of a receive-pack body, or None.

    AN EMPTY LIST AND None ARE TWO DIFFERENT STATES, and confusing them is the
    one defect able to stop the autonomous loop. Before streaming a large push
    git sends a 4-byte authentication probe: a bare flush-pkt, no command at
    all. That is an empty list, and it MUST pass — a guard demanding "at least
    one command" would refuse it and therefore refuse every push big enough to
    be streamed, with nothing saying so before the first dispatch. None is a
    body that exists but does not decode, or no body at all; § 3.3 of the plan
    settles that case as a refusal.

    Only the head is read. Past mitmproxy's streaming threshold the body goes
    out chunked, and the commands stay in front of the pack — which is why this
    runs in `request` (body buffered) and not in `requestheaders`.
    """
    if body is None:
        return None
    if body == b"":
        # Zero bytes is the degenerate form of the same probe: nothing is being
        # asked for, so there is nothing to refuse.
        return []
    commands: list[tuple[str, str, str]] = []
    index, total = 0, len(body)
    while True:
        if index + 4 > total:
            # The section ended without a flush-pkt: we never reached a point
            # where the command list was complete, so we do not know it.
            return None
        header = body[index : index + 4]
        if any(char not in _HEX_DIGITS for char in header):
            return None
        length = int(header, 16)
        if length == 0:
            return commands  # flush-pkt: the command section is over
        if length < 4 or index + length > total:
            return None
        parsed = _parse_ref_command(body[index + 4 : index + length])
        if parsed is None:
            return None
        commands.append(parsed)
        index += length


def _push_verdict(repo: str, commands: list[tuple[str, str, str]]) -> tuple[str, str]:
    """`(_ALLOW, "")`, or `(motif, the repository or ref in cause)`.

    Pure. The rules are independent, so the branch order carries no safety —
    but the order of the REPORT matters: EVERY command is examined, and the
    first violating one names both the motif and the subject. A bound that
    judged only the first command would be defeated by a harmless command
    placed in front of it.

    The deletion test comes before the protected-branch test for diagnosis, not
    for safety: a deletion of `refs/heads/main` is a deletion first, and that is
    the remedy an operator needs named.
    """
    if repo.split("/", 1)[0].lower() != _PUSH_ALLOWED_OWNER:
        return _DENY_REPO_FOREIGN, repo
    for _old, new, ref in commands:
        # An all-zero new object id IS the protocol's way of saying "delete";
        # width-agnostic, so a sha256 repository reads the same as a sha1 one.
        if new.strip("0") == "":
            return _DENY_REF_DELETE, ref
        if not ref.startswith(_PUSH_REF_PREFIX) or ref == _PUSH_REF_PREFIX:
            return _DENY_REF_EXOTIC, ref
        if ref in _PUSH_PROTECTED_REFS:
            return _DENY_REF_PROTECTED, ref
    return _ALLOW, ""


def _push_scope_enforced() -> bool:
    """Is the declared-perimeter tightening armed in THIS process?

    THE ONLY READER of `MIKA_PILOT_EGRESS_PUSH_SCOPE` in the whole repository,
    and deliberately so: the refusal lives here, so the switch is read here and
    nowhere else. The launcher writes the perimeter UNCONDITIONALLY — a
    repository name is not a secret, and an entry nobody enforces does nothing.
    A second reader is exactly how mika#2165 gets paid: a switch one of its two
    readers ignores is decorative.

    Same vocabulary and same default as the two switches above: unset or empty
    means armed.
    """
    raw = os.environ.get(_PUSH_SCOPE_SWITCH)
    if raw is None or not raw.strip():
        return True
    return raw.strip().lower() not in _SWITCH_OFF


def _push_scope_dir() -> Path:
    """Where the host keeps one file per declared perimeter.

    A registry of its own, not a field added to the sentinel entry: the two data
    have opposite failure postures (that one refuses, this one degrades),
    different switches, and — since mika#2766 — different lifetimes
    (`_PUSH_SCOPE_TTL_SECONDS` here, `_REGISTRY_TTL_SECONDS` there). Resolved per
    call for the same reason as `_registry_dir` — mitmdump is long-lived, and the
    staging suites export HOME after sourcing the library.
    """
    override = os.environ.get(_PUSH_SCOPE_DIR_ENV)
    if override:
        return Path(override)
    return Path.home() / ".mika" / "pilot-push-scopes"


def _push_scope_entries() -> list[tuple[str, str | None]] | None:
    """The `(repository, ref)` couples live dispatches declare, or None.

    ONE COUPLE PER ENTRY, never two independent sets (mika#2776). A flat
    per-dimension union would authorise the cross product — see THE UNION IS BY
    ENTRY above — so the pairing is the whole point of the return type, and a
    list rather than a set because two spawns on the same couple are not an
    error to deduplicate away.

    None — and ONLY None — means the registry could not be read at all:
    directory absent, not a directory, any OSError. A registry that IS readable
    but declares no `repo=` returns an empty list, which is a DIFFERENT state
    and lands on a different stderr token and a different remedy (mika#2205).
    Both degrade to the static bound; conflating them would name the wrong fault
    on the day it matters.

    An entry that is individually unreadable is SKIPPED, never promoted to a
    whole-registry failure: one spawn's bookkeeping going bad must not widen
    nor narrow the verdict for its neighbours. Unknown keys are ignored, so a
    future key stays a pure addition the way `ref=` was.

    An entry with NO `repo=` yields no couple at all: the repository is what
    selects which couples may speak, so a ref with no repository could only ever
    widen. An entry whose `ref=` is absent — or present with an empty value,
    which reads the same — yields `None` in second position, and that is a
    DECLARATION OF NOTHING rather than a declaration of the empty branch: the
    host registry survives a deployment, so a pre-upgrade entry is the normal
    state for one window (point 2 of mika#2776).

    The repository is normalised the way `_receive_pack_repo` normalises the
    URL — lowercased, `.git` suffix dropped — so the two forms meet already
    comparable. GitHub is case-insensitive on `owner/repo`; a case-sensitive
    comparison there would refuse a legitimate push on a capital letter. The
    ref is NOT normalised, and that asymmetry is deliberate: git refs are
    case-sensitive, `refs/heads/Feat/X` and `refs/heads/feat/X` are two
    different branches, so lowercasing here would be a LOOSENING of the bound.
    """
    directory = _push_scope_dir()
    # Its OWN TTL, shorter than the sentinel registry's: this one answers "is
    # this perimeter still in flight?" (mika#2766). Passed in rather than read
    # inside the purge, so the two registries never share one number.
    _purge_registry(directory, _PUSH_SCOPE_TTL_SECONDS)
    try:
        entries = list(os.scandir(directory))
    except FileNotFoundError:
        # NO DIRECTORY AT ALL IS NOT A HOST FAULT, and this is the one place
        # where this registry parts company with the sentinel's (where an
        # absent directory IS unreadable). The launcher creates this one with
        # the first perimeter it stages, so its absence means nothing was ever
        # declared -- `scope-absent`, whose remedy is "find why the dispatch
        # declared nothing". Calling it unreadable would send an operator
        # looking at directory permissions for a launcher that never ran
        # (mika#2205: naming the wrong remedy is the whole cost).
        return frozenset()
    except OSError:
        # The path exists but cannot be scanned: a regular file where a
        # directory belongs, EACCES, anything else. That IS host bookkeeping.
        return None
    couples: list[tuple[str, str | None]] = []
    for entry in entries:
        try:
            if not entry.is_file():
                continue
            raw = Path(entry.path).read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        declared_repo = ""
        declared_ref: str | None = None
        for line in raw.splitlines():
            stripped = line.strip()
            if stripped.startswith(_PUSH_SCOPE_REPO_KEY):
                value = stripped[len(_PUSH_SCOPE_REPO_KEY):].strip().lower()
                if value.endswith(".git"):
                    value = value[: -len(".git")]
                if value:
                    declared_repo = value
            elif stripped.startswith(_PUSH_SCOPE_REF_KEY):
                # Case preserved: see the docstring on why lowercasing a ref
                # would loosen the bound rather than normalise it.
                value = stripped[len(_PUSH_SCOPE_REF_KEY):].strip()
                if value:
                    declared_ref = value
        if declared_repo:
            couples.append((declared_repo, declared_ref))
    return couples


def _push_scope_verdict(repo: str, refs: list[str]) -> tuple[str, str]:
    """`(_ALLOW, "")`, a degradation token, or a `push-scope-*` refusal.

    SUBORDINATE BY CONSTRUCTION. This function is only ever reached on a push
    the static bound has ALREADY authorised, so its only new outcomes are
    refusals — there is no path by which it turns a refusal into an allow, and
    no `return` of it can open anything. The property is structural, not a
    thing to re-verify at each reading (§ 3.1 of the plan).

    TWO DIMENSIONS, READ IN ORDER, and the order is the one that can be
    diagnosed. The repository is judged first because it is what SELECTS the
    couples whose ref may speak; only then is the branch judged, against the
    refs of those couples alone. So a push to an undeclared repository reports
    `push-scope-repo` even when its ref is also wrong, which is the right
    reading: the remedy is the remote, and the refspec question does not arise
    until the remote is right.

    An empty `refs` returns ALLOW rather than refusing: the 4-byte
    authentication probe that precedes every streamed push carries NO command
    section, and a bound that 403s it refuses every push large enough to be
    streamed — the one failure mode able to stop the loop (point 4 of the
    ticket, asserted at CASE 18 and CASE 24).

    The degradation tokens are returned rather than logged here so the decision
    stays pure and the caller owns every write to stderr.
    """
    if not _push_scope_enforced():
        return _ALLOW, ""
    entries = _push_scope_entries()
    if entries is None:
        return _DEGRADE_SCOPE_UNREADABLE, ""
    if not entries:
        return _DEGRADE_SCOPE_ABSENT, ""
    declared = [ref for (name, ref) in entries if name == repo.strip().lower()]
    if not declared:
        return _DENY_SCOPE_REPO, repo
    if not refs:
        return _ALLOW, ""
    if any(ref is None for ref in declared):
        # At least one live entry naming this repository declares no branch, so
        # the branch dimension cannot be applied WITHOUT risking the refusal of
        # a legitimate push: that entry is the pre-upgrade state, and what it
        # would have declared is unknown rather than empty. Degrade to the
        # repository dimension — which has already passed — and say so on one
        # greppable line.
        return _DEGRADE_SCOPE_REF_ABSENT, ""
    allowed = set(declared)
    for ref in refs:
        if ref.strip() not in allowed:
            return _DENY_SCOPE_REF, ref
    return _ALLOW, ""


def _push_deny_body(reason: str, subject: str) -> str:
    """The 403 body: the motif, the target, the remedy, and the rollback."""
    ticket = _PUSH_DENY_TICKETS.get(reason, _PUSH_DENY_DEFAULT_TICKET)
    return (
        f"mika-pilot-github-auth: DENY {reason} ({ticket}).\n"
        "\n"
        + _PUSH_DENY_BODIES[reason].format(subject=subject)
        + "\n"
        + _PUSH_DENY_FOOTER
    )


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
    # inject when it is not one the host attests. github.com is asked for no
    # justification — see the module docstring: none can travel there in the
    # pilot sandbox, so the requirement is bounded to the channel that carries
    # one. github.com is bounded differently, in `request` below, on what the
    # proxy already sees (mika#2736).
    # The 503 above stays ahead of the verdict: a host with no
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


def request(flow: http.HTTPFlow) -> None:
    """Bound what a `git-receive-pack` may touch (mika#2736, mika#2751).

    WHY THIS HOOK AND NOT `requestheaders`, where the injection lives. The
    verdict needs the BODY: the repository is in the URL, but the ref commands
    are in the first pkt-lines, and in `requestheaders` the body is not there
    yet. So the two concerns deliberately sit in two hooks — inject as early as
    possible, judge as soon as there is something to judge. Past mitmproxy's
    streaming threshold the body goes out chunked with the commands still at its
    head, which is exactly the case this placement handles and the earlier hook
    could not.

    A request `requestheaders` already answered is left alone, so the
    api.github.com cut of mika#2724 is never re-judged and never double-logged.
    """
    if flow.response is not None:
        return
    # api.github.com has no receive-pack endpoint; the bound must not invent one
    # there, and vol 2 already owns that host entirely.
    if flow.request.host != _GIT_HOST:
        return
    if not _push_bound_enforced():
        return
    repo = _receive_pack_repo(flow.request.path)
    if repo is None:
        return
    try:
        content = flow.request.content
    except Exception:  # noqa: BLE001 — see below
        # mitmproxy raises here when the body was streamed or its encoding is
        # broken. A reader that let this propagate would be a 500 on the loop's
        # push path: louder than the 403 we chose, and undiagnosable. Treated as
        # the same state as a None body, which it is.
        content = None
    commands = _ref_commands(content)
    if commands is None:
        reason, subject = _DENY_PUSH_OPAQUE, ""
    else:
        reason, subject = _push_verdict(repo, commands)
    if reason == _ALLOW:
        # The declared-perimeter tightening (mika#2751 for the repository,
        # mika#2776 for the branch) sits HERE and nowhere else: strictly under
        # the static bound, reached only on a push mika#2736 already authorised.
        # That placement is what makes point 2 of the ticket — an absent or
        # unreadable perimeter must keep the static bound applying — true by
        # construction rather than by vigilance.
        #
        # `commands` is non-None on this branch (an unreadable body landed on
        # `_DENY_PUSH_OPAQUE` above and never reaches here), so the ref list is
        # read off the protocol the static bound already decoded — the same
        # triplets, no second parse to keep in agreement with the first.
        refs = [ref for _old, _new, ref in commands]
        scope, scope_subject = _push_scope_verdict(repo, refs)
        if scope in _PUSH_SCOPE_DEGRADED:
            # An ALLOW that says why it did not tighten. Same line shape as the
            # DENY tap, different verb: `grep scope-absent` on the mitmdump log
            # is the operator probe, and a line per dispatch means the
            # tightening never bites — the silent failure to watch for.
            print(
                f"[github-proxy] DEGRADE {scope} "
                f"{flow.request.method} {flow.request.path}",
                file=sys.stderr,
                flush=True,
            )
            return
        if scope == _ALLOW:
            return
        reason, subject = scope, scope_subject
    # The exact line shape of vol 2 — one motif more in an existing vocabulary
    # beats a second form of line to grep. The motif, the method and the path;
    # never the presented credential, never the host token, never a digest of
    # either (mika#1949: a fingerprint is deferred disclosure).
    print(
        f"[github-proxy] DENY {reason} {flow.request.method} {flow.request.path}",
        file=sys.stderr,
        flush=True,
    )
    # Strip the credential `requestheaders` injected a moment ago. mitmproxy
    # will not forward a short-circuited request, so this changes no observable
    # behaviour today; it means the host credential is absent from a refused
    # request by construction rather than by trust in that short-circuit.
    for header_name in ("authorization", "proxy-authorization"):
        flow.request.headers.pop(header_name, None)
    flow.response = http.Response.make(
        403,
        _push_deny_body(reason, subject).encode("utf-8"),
        {"Content-Type": "text/plain; charset=utf-8"},
    )


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
