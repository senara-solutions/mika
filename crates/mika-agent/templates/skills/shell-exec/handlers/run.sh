#!/bin/sh
# Shell execution handler.
# Input: JSON on stdin with "command" and optional "working_dir" fields
# Output: command output on stdout, errors on stderr
#
# SECURITY: This handler executes arbitrary commands. Use responsibly.

command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }

INPUT=$(cat)

# --- mika#2449: shared-checkout guard inputs, read BEFORE the scrub ---------
# The scrub loop below unsets every MIKA_* variable, MIKA_PLATFORM_DIR and
# MIKA_GUARD_SHARED_CHECKOUT included. Both are read here, resolved, and
# passed to the guard as ARGUMENTS (never through the child's environment):
# a guard that read them after the scrub would see nothing and fall open on
# every host, indistinguishable from a host with nothing to protect.
# Same resolution as dispatch-lib.sh (`PLATFORM_DIR=…; pwd -P`) and
# deploy-mika/handlers/run.sh, on purpose — `~/workspace` is a symlink to
# `/data/workspace` on the measured host, and the guard compares against the
# physical path.
_GUARD_PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
_GUARD_PLATFORM_DIR=$(cd "$_GUARD_PLATFORM_DIR" 2>/dev/null && pwd -P) || _GUARD_PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
_GUARD_BYPASS="${MIKA_GUARD_SHARED_CHECKOUT:-1}"

# Scrub all MIKA_* env vars so subprocesses cannot leak secrets
# Mirrors the Rust executor's scrub_mika_env_vars() wildcard approach
for _mika_var in $(env | grep '^MIKA_' | cut -d= -f1); do unset "$_mika_var"; done
# Scrub GH_TOKEN to prevent identity collision if leaked from .env (#380)
unset GH_TOKEN 2>/dev/null

# Parse JSON fields
COMMAND=$(printf '%s\n' "$INPUT" | jq -r '.command // empty')
WORKDIR=$(printf '%s\n' "$INPUT" | jq -r '.working_dir // empty')

if [ -z "$COMMAND" ]; then
    echo "Error: no command provided" >&2
    exit 1
fi

# Block commands that have dedicated skill handlers (security: force use of controlled wrappers)
FIRST_WORD=$(printf '%s\n' "$COMMAND" | awk '{print $1}')
case "$FIRST_WORD" in
    gws)  echo "Error: Use the dedicated run_gws skill instead of run_shell for security." >&2; exit 1 ;;
    gh)   echo "Error: Use the dedicated run_gh skill instead of run_shell for security." >&2; exit 1 ;;
esac

# --- shell-exec L3 hardening (mika#1957, F3 from mika#1798) ---
# The FIRST_WORD case above only inspects the first token, so every shape that
# reaches a gated CLI through a subshell, a path prefix, or a statement
# separator walks straight past it: `sh -c 'gws ...'`, `bash -c "gws ..."`,
# `eval "gws ..."`, `echo 'gws ...' | sh`, `/usr/bin/gws ...`, `pwd; gws ...`,
# `$(gws ...)`. Those calls never enter the run_gws/run_gh builtin handler, so
# none of the non-transit doctrine's L1-L4 layers fire either.
#
# Lexical scan of the whole command string closes the class. The boundary is
# "any character that cannot be part of a command identifier", which subsumes
# whitespace, both quote characters, `;`, `|`, `&`, backtick, `$`, `(`, and `/`.
# Excluding `.` and `-` from the boundary keeps ordinary paths and flags usable:
# `.github/...`, `gh-pages`, and `/tmp/gws.log` are not matches.
#
# Defense-in-depth, NOT a sole gate. The scan is lexical, so anything that
# hides the literal token from a byte-level match still gets through, and that
# is broader than obfuscation-by-encoding. Measured gaps, deliberately not
# chased here (arms race with no fixed point short of parsing shell grammar):
#   - token splitting:        g""ws gmail ...   g''ws gmail ...   gw\s gmail ...
#   - glob expansion:         /usr/bin/gw[s] gmail ...   /usr/bin/gw? gmail ...
#   - variable assembly:      A=g; B=ws; $A$B gmail ...
#   - encoded payloads:       echo <base64> | base64 -d | sh
#   - renamed/aliased binary: PATH=/tmp:$PATH gws-alias gmail ...
# For all of the above, the registry ban (L2) and the execute-time guard (L4)
# from mika#1798 remain the last-mile checks — they fire at the real tool call,
# where the command has already been resolved. What this scan buys is the
# casual and the incidental path, which is where the observed traffic is.
# A raw-HTTP call to the underlying API (curl https://gmail.googleapis.com/...)
# is covered by no layer at all; see the doctrine doc's bypass-class list.
if printf '%s\n' "$COMMAND" | grep -Eq '(^|[^A-Za-z0-9_.-])(gws|gh)([^A-Za-z0-9_.-]|$)'; then
    echo "Error: shell-exec refuses commands that route to skill-gated CLIs (gws, gh). Use the dedicated run_gws or run_gh skill instead." >&2
    exit 1
fi
# --- end shell-exec L3 hardening ---

# --- shell-exec egress containment (mika#1991) ---
# T1 mission-passeport (2026-08-24) measured the bypass this block closes:
# over one shot, fetch_url was called 0 times and run_shell+curl 78 times,
# and www.bouscat.fr — a host NOT on the fetch_url allowlist — was reached
# with no obstacle. The compile-time gouv.fr allowlist that made fetch_url
# acceptable under single-controlled-egress constrained NONE of that traffic:
# it walked past the allowlisted door through run_shell.
#
# Doctrine (docs/solutions/best-practices/optional-path-is-no-guarantee-2026-08-30.md):
# a guarantee carried by an OPTIONAL path is not a guarantee. The allowlist
# only describes the traffic of callers who choose the enforced door. The fix
# is to construct the incapacity, not to persuade the model: run_shell must be
# unable to reach the network directly, so egress is FORCED onto the substrate
# (fetch_url / web_search), which is the single allowlist-enforcing path.
#
# We do NOT re-implement the allowlist here. Duplicating the four gouv.fr hosts
# into this handler would create a second source of truth — the very
# scattered-guarantee anti-pattern this ticket is about — and would trip
# scripts/verify-egress-uniqueness.sh, whose whole job is to keep the allowlist
# the sole property of crates/mika-gateway/src/egress_fetch/. So this gate is
# unconditional: any direct HTTP fetcher is refused, allowlisted host or not.
# An off-allowlist host is therefore refused (bypass closed); an allowlisted
# host is still reachable — through fetch_url, which enforces the allowlist.
#
# Boundary-aware lexical scan, same class as the L3 block above: the boundary
# is "any character that cannot be part of a command identifier", so `/`
# (path-qualified `/usr/bin/curl`), whitespace, quotes, `;`, `|`, `&`,
# backtick, `$`, `(` all delimit a real invocation, while `.` and `-` are
# excluded so `curl.log`, `wget.txt`, and `libcurl-dev` are not matches.
#
# Defense-in-depth, NOT a sole gate — identical stance to the L3 block. The
# scan is lexical, so token-splitting, glob/variable assembly, base64-then-sh,
# and non-curl transports (nc, /dev/tcp, a python urllib one-liner) still slip
# a byte-level match. The complete closure is the network layer: a fresh
# network namespace with no route plus a forced redirect to the substrate
# gateway (mika#1969 AC5 iptables policy / the bwrap `--unshare-net` +
# unix-socket proxy the dev-pilot already uses). This block buys the casual
# and incidental path, which is exactly where the measured traffic lived.
if printf '%s\n' "$COMMAND" | grep -Eq '(^|[^A-Za-z0-9_.-])(curl|wget)([^A-Za-z0-9_.-]|$)'; then
    echo "Error: shell-exec refuses direct network egress (curl, wget). Agent egress MUST go through the allowlist-enforcing substrate: use the fetch_url builtin to GET a page, or web_search to search. This closes the run_shell egress bypass (mika#1991)." >&2
    exit 1
fi
# --- end shell-exec egress containment ---

# --- mika#2639: python installer containment ---
# Measured producer (server.log, trace e9432d40-bdff-11f1-837b-27cfaa9b231b):
# mika-qa, through this very handler, ran `pip install -e . -q
# --break-system-packages` at 01:26:08/15/18Z on 2026-10-02 while reviewing the
# claude-pilot PR cpp#273. `pip install -e` of a repo whose `[console_scripts]`
# declares `claude-pilot` REWRITES ~/.local/bin/claude-pilot with the system
# Python's shebang, so every pilot died at launch. Disk trace dated 01:26:20Z
# (`dist-info`, `INSTALLER=pip`, `direct_url = file:///tmp/tmp.Eqlt2PdYwK`);
# 2 h 45 of dead rail (mika#2634). Over 48 h, 43 run_shell calls from mika-qa
# carried `pip install`, 19 of them aimed at claude-pilot; only the 5 carrying
# `--break-system-packages` could write — the 38 others were refused by Gentoo's
# externally-managed pip (PEP 668). THE HOST'S PROTECTION HELD BY ACCIDENT, NOT
# BY DESIGN, until the agent learned to opt out of it.
#
# Why here and not in BUILD_COMMAND_FAMILY (mika#2423): that guard's predicate
# is conditioned on the tool budget — `if timeout_secs >= BUILD_FLOOR_SECS {
# return None; }` — and exempts `long_running` / `detaches_command` handlers by
# construction. But `pip install` into the host's environment is no safer under
# 300 s than under 30 s. Arming it there would cover the measured population BY
# ACCIDENT (shell-exec declares timeout_secs = 30) and leave every
# wider-budget handler uncovered — and the day somebody raises that budget the
# pip half evaporates with no test going red. That is the mika#2205 class: a
# guard inert over part of its population reads exactly like a guard that works.
# The two compose; neither repairs the other. mika#2423 arbitrates a BUDGET,
# this block arbitrates CONTAINMENT.
#
# The predicate INVERTS the obvious direction, and that inversion carries
# everything: rather than expressing "a path that is not a venv" (inexpressible
# in ERE without a lookbehind), step 1 replaces venv-qualified installers with a
# sentinel, and steps 2-3 then scan what is LEFT. The sentinel carries no `pip`
# substring, so it cannot be matched by the rules that follow.
#
# Three limits, named rather than discovered:
#   - PROSE FALSE POSITIVE. The trailing boundary is `([^A-Za-z0-9_.-]|$)` and
#     not `([[:space:]]|$)`, without which `pip install;` escapes trivially. The
#     price: `grep -rn "pip install -e" docs/` is refused. Same property as the
#     `gh` scan above, same assumed stance; the refusal body names the
#     token-splitting workaround without ever handing over an install template.
#   - VENV NAME OUTSIDE THE LIST. The root allowlist is
#     {.venv, venv, .virtualenv, virtualenv, env} on an EXACT segment: a venv
#     named otherwise (`myenv/bin/pip`, `/opt/conda/envs/x/bin/pip`) is REFUSED
#     — fail-closed, with the universal `uv run` substitute left open. `env` is
#     the loosest member (`/usr/local/env/bin/pip` would be neutralised); it is
#     kept because `python -m venv env` is common, and its looseness is said
#     here rather than left to be found.
#   - VENV ONLY IN COMMAND POSITION. A venv installer is neutralised only at
#     the start of the line or right after a separator (`;` `&` `|` `(` `{`
#     backtick `!`), whitespace and one opening quote allowed. Anywhere else
#     a venv path is an ARGUMENT, not the interpreter that runs:
#     `python3 -X .venv/bin/python -m pip install` runs the HOST's pip, and a
#     path-anywhere sentinel swallowed its `-m pip`. Price: a venv installer
#     behind a wrapper (`env FOO=1 …`, `timeout 60 …`, `sh -c "…"`) is
#     REFUSED — fail-closed, same stance as the venv-name allowlist above.
#   - THE mika#1957 EVASIONS REMAIN THE EVASIONS. Token splitting
#     (`p""ip install`), variable assembly (`P=pip; $P install`,
#     `pip${IFS}install`), backslash-newline (grep reads line by line), base64
#     payloads: this reads a command line, not what it executes.
# Defense-in-depth, NOT a sole gate. The wall is putting shell-exec itself
# inside the sandbox (mika#2141) — its own ticket, named and out of scope.
#
# RULE SHAPE, and one correction to the plan's four-rule enumeration. The
# enumerated `pip install` / `pip3 install` / `python -m pip install` /
# `uv pip install` forms are ONE boundary-aware lexical fact, not four: in every
# one of them the token `pip` is preceded by a non-identifier character and
# followed by an install subcommand, so four separate rules would leave three of
# them unable to fire alone — the dead-branch shape mika#2205 condemns. They are
# therefore one rule. What genuinely needs a second rule is the adjacency form
# `python -mpip install`, which Python accepts and whose missing space defeats
# the left boundary. `pipx` needs no rule of its own: `pip` followed by `x`
# fails the RIGHT boundary, so the installer alternation names it directly —
# which is why both neutralisers of step 1 carry a right boundary too, or
# `.venv/bin/pipx install` would lose its `pip` before step 2 looked.
_PY_SENTINEL='__MIKA_VENV_INSTALLER__'
# Cheap pre-filter: every rule below requires the literal `pip`, `pipx`, `uv`
# or `setup.py` (the env/config opt-outs of step 3 are conjoined with a named
# installer), so a command carrying none of them cannot match one.
# Deliberately a substring test with no boundary — it must not be able to
# exclude anything a rule would catch, and a false positive only costs the sed
# pipeline below.
if printf '%s\n' "$COMMAND" | grep -Eq 'pip|uv|setup\.py'; then
    # Step 1 — neutralise the PERMITTED form, then look for the forbidden one.
    # Command position only (see the limits above). The right boundary is
    # consumed and re-emitted, so the expression runs twice: a separator eaten
    # by one match is the left boundary of the next.
    _PY_VENV_RE='(^[[:space:]]*|[;&|({`!][[:space:]]*)(["'"'"']?)([^[:space:]"'"'"';&|()<>`]*/)?(\.venv|venv|\.virtualenv|virtualenv|env)/bin/(pip[0-9.]*|python[0-9.]*)([^A-Za-z0-9_.-]|$)'
    # …and drop a `-m pip` / `-mpip` that follows a neutralised venv python:
    # `"$W/.venv/bin/python" -m pip install -e "$W"` must PASS, because the
    # discriminant is WHICH ENVIRONMENT, never which binary — and
    # `python -m venv … && .venv/bin/python -m pip install` is a common idiom.
    # The `--user` family still refuses that form (rule D below). `-m pipx`
    # is not `-m pip`: the right boundary keeps it for step 2.
    _PY_DASH_M_RE="${_PY_SENTINEL}"'["'"'"']?[[:space:]]+(-[^[:space:]]+[[:space:]]+)*-m[[:space:]]*pip([^A-Za-z0-9_.-]|$)'
    _PY_SCAN=$(printf '%s\n' "$COMMAND" \
        | sed -E -e "s#${_PY_VENV_RE}#\\1\\2${_PY_SENTINEL}\\6#g" \
            -e "s#${_PY_VENV_RE}#\\1\\2${_PY_SENTINEL}\\6#g" \
        | sed -E "s#${_PY_DASH_M_RE}#${_PY_SENTINEL}\\2#g")

    # Step 2 — the installer token plus an install subcommand. A closing quote
    # is OPTIONAL after the binary: without it `"/usr/bin/pip" install foo`
    # walks past, i.e. the guard is bypassable with one quote character. The
    # gap tolerates options between binary and subcommand, each optionally
    # followed by ONE separate argument (`pip --cache-dir /tmp/c install`,
    # `pip --python /usr/bin/python3 install`) — without the argument term only
    # `--flag=value` passed and a separate value stopped the match before
    # `install`. The subcommand itself may be quoted (`pip 'install'`).
    _PY_GAP='([[:space:]]+-[^[:space:]]+([[:space:]]+[^-[:space:]][^[:space:]]*)?)*[[:space:]]+["'"'"']?'
    _PY_INSTALLER_RE='(^|[^A-Za-z0-9_.-])(pip[0-9.]*|pipx)["'"'"']?'"$_PY_GAP"'install([^A-Za-z0-9_.-]|$)'
    # …and the no-space adjacency the boundary above cannot see.
    _PY_DASH_MPIP_RE='(^|[^A-Za-z0-9_.-])python[0-9.]*["'"'"']?[[:space:]]+-m[[:space:]]*pip'"$_PY_GAP"'install([^A-Za-z0-9_.-]|$)'
    # …and `uv tool install|upgrade`, which writes ~/.local/bin from any
    # environment and ignores PEP 668. It is the workspace's own claude-pilot
    # install command (`make deploy`), so a model refused on `pip install` that
    # reaches for the next documented installer would replay mika#2634.
    # `uvx` / `uv tool run` / `uv run` stay open: they install nothing on PATH.
    _PY_UV_TOOL_RE='(^|[^A-Za-z0-9_.-])uv["'"'"']?'"$_PY_GAP"'tool[[:space:]]+(install|upgrade)([^A-Za-z0-9_.-]|$)'

    # Step 3 — a flag aimed at the HOST, even on a venv-qualified form. Read on
    # the RAW command, never on $_PY_SCAN: a venv pip is neutralised there while
    # `--user` still writes to ~/.local.
    #
    # THREE terms in conjunction, where the plan carried two — and the third is a
    # measured correction, not a refinement. With flag+verb alone,
    # `./configure --prefix=/usr && make install` is refused: both terms are
    # true and neither is a Python install. Requiring that an installer also be
    # NAMED in the raw command keeps the plan's motivating case
    # (`"$W/.venv/bin/pip" install --user -e .`, which writes to ~/.local
    # despite its venv pip) and drops that false positive.
    #
    # The host opt-out has three spellings, not one: the flag, its PIP_*
    # environment twin (`PIP_BREAK_SYSTEM_PACKAGES=1 pip install`), and the
    # persistent `pip config set global.break-system-packages true`, which
    # takes no install verb and is therefore its own conjunction below. The
    # agent learned to opt out of PEP 668 once; the env and config routes are
    # the same opt-out under another name. `setup.py install|develop --user`
    # writes console_scripts into ~/.local/bin like pip does, so setuptools is
    # a named installer here too.
    _PY_HOST_FLAG_RE='(^|[^A-Za-z0-9_.-])(--(break-system-packages|user|target|prefix|system)([^A-Za-z0-9_.-]|$)|PIP_(BREAK_SYSTEM_PACKAGES|USER|TARGET|PREFIX)=)'
    _PY_INSTALL_VERB_RE='(^|[^A-Za-z0-9_.-])(install|develop)([^A-Za-z0-9_.-]|$)'
    _PY_TOOL_RE='(^|[^A-Za-z0-9_.-])(pip[0-9.]*|pipx|uv|setup\.py)([^A-Za-z0-9_.-]|$)'
    _PY_PIP_CONFIG_RE='(^|[^A-Za-z0-9_.-])config([[:space:]]+-[^[:space:]]+)*[[:space:]]+set[[:space:]]+[A-Za-z0-9_-]+\.(break-system-packages|user|target|prefix)([^A-Za-z0-9_.-]|$)'

    # SOLE WRITER of the refusal token, so `tool_calls.output LIKE '%REFUS
    # (python-installer-guard, mika#2639)%'` counts one population and not two.
    # The two motifs are a WIRE FORMAT (an operator GROUPs BY them); a third
    # would make the reading table in the root CLAUDE.md false in silence.
    # Refusal goes to STDERR with exit 1 — the channel of this file's two elders
    # (the gws/gh scan and the egress containment), so a reader finds the fourth
    # scan where the first two are. It reaches `tool_calls.output` all the same:
    # on a non-zero exit `execute_exec` combines stdout and stderr and prefixes
    # `Exit code: 1`. The body names the SUBSTITUTE and never an install
    # template (doctrine mika#2520 / mika#2292).
    _refuse_python_installer() {
        echo "REFUS (python-installer-guard, mika#2639): $1" >&2
        echo "shell-exec refuses to install into the host's Python environment." >&2
        echo "To exercise a Python repo, run it from the worktree with 'uv run <cmd>'" >&2
        echo "(e.g. 'uv run pytest'), or from a venv explicitly rooted under that" >&2
        echo "worktree. Never the host's environment: on 2026-10-02 a '[console_scripts]'" >&2
        echo "entry point rewrote ~/.local/bin and killed the production claude-pilot" >&2
        echo "launcher for 2 h 45 (mika#2634)." >&2
        echo "If you were only SEARCHING for this string, split the token: grep 'pip[ ]install'." >&2
        exit 1
    }

    if printf '%s\n' "$_PY_SCAN" | grep -Eq "$_PY_INSTALLER_RE" \
        || printf '%s\n' "$_PY_SCAN" | grep -Eq "$_PY_DASH_MPIP_RE" \
        || printf '%s\n' "$_PY_SCAN" | grep -Eq "$_PY_UV_TOOL_RE"; then
        _refuse_python_installer host_installer
    fi
    if printf '%s\n' "$COMMAND" | grep -Eq "$_PY_TOOL_RE" \
        && { { printf '%s\n' "$COMMAND" | grep -Eq "$_PY_HOST_FLAG_RE" \
            && printf '%s\n' "$COMMAND" | grep -Eq "$_PY_INSTALL_VERB_RE"; } \
            || printf '%s\n' "$COMMAND" | grep -Eq "$_PY_PIP_CONFIG_RE"; }; then
        _refuse_python_installer host_target_flag
    fi
fi
# --- end python installer containment ---

# --- mika#2449: shared-checkout guard — the primary checkout is a deployment
# checkout, and run_shell must be UNABLE to break its invariant.
#
# Measured producer (tool_calls 1835d8bb / eba3682f / 930f5200, 2026-09-20):
# mika-qa, through this very handler, ran `cd ~/workspace/mika-platform/mika &&
# git checkout <ref> -- <paths>` during three QA reviews and restored nothing
# (`git checkout -- <paths>` re-reads the INDEX the previous command just
# overwrote — an inert "cleanup"). Result: 15 staged+modified files on main,
# `git pull --ff-only` refused, the orchestrator's rebuild blocked (#2446).
# Recurring class, n ≥ 5 since 09-06. The qa-review prompt already prescribed
# a throwaway detached worktree (§ 2B); prompt-only enforcement does not hold at
# loop substrate (mika#2120). This is the structural half.
#
# mika#2107's guard cannot cover this population by construction: its T1
# exempts every session not rooted in a linked worktree, and mika-qa has no
# worktree — its cwd is the skill directory and the `cd` is inside the command.
# Hence the guard's second mode, `--decide-primary`, which shares the parser
# and inverts the population: target = a PRIMARY checkout under the platform
# dir (`.git` is a directory). Predicate = the deployment-checkout invariant
# (D7 allow-list: fetch / pull --ff-only / merge --ff-only / worktree * /
# branch without -f/-m/-c / push / any read pass; checkout <ref>, stash,
# reset, non-ff merge, branch -f … are refused; unknown verbs fail closed).
#
# FAIL-OPEN on a missing script, and it is SAID: a tenant without a platform
# checkout has nothing to protect (silence); a host WITH a platform and WITHOUT
# the script prints the fail-open line below — the only thing separating
# "nothing to refuse" from "nothing is armed" (mika#2107, same sentence). A
# guard that crashes (exit ≠ 0/1) also falls open and says so: a broken guard
# must not take run_shell down for every agent. A predicate that HOLDS
# refuses — that is its purpose, not an outage.
#
# The refusal goes to STDOUT with exit 1, so it lands in `tool_calls.output`
# prefixed `Exit code: 1` — an SQL surface without a new event (D8):
#   SELECT agent_id, count(*) FROM tool_calls
#    WHERE tool_name = 'run_shell'
#      AND output LIKE '%REFUS (shared-checkout-guard, mika#2449)%' GROUP BY 1;
# This handler never writes that token itself — it relays the guard's text
# (SOLE WRITER of the token is the guard; pinned by test-shell-exec-guard.sh).
#
# The override is the same lever as mika#2107 — MIKA_GUARD_SHARED_CHECKOUT=0,
# read above, before the scrub — and it is SAID ON EVERY CALL: a bypass set
# for an intervention and forgotten would otherwise be a guard silently absent
# for all agents, indistinguishable from a guard that had nothing to refuse
# (F3; mika#2329: for a switch, liveness IS the information). One line per
# call rather than "first call": the handler is one process per call, it has
# no "first". The guard is still invoked under the override so the bypass is
# journaled in ~/.mika/state/shared-checkout-guard.log (`mode=primary bypass`).
_GUARD_SCRIPT="$_GUARD_PLATFORM_DIR/mika/scripts/guard-shared-checkout"
_GUARD_CWD="${WORKDIR:-$PWD}"
if [ "$_GUARD_BYPASS" = 0 ]; then
    echo "shell-exec: shared-checkout guard disarmed by MIKA_GUARD_SHARED_CHECKOUT=0 (operator override)" >&2
    if [ -f "$_GUARD_SCRIPT" ]; then
        MIKA_GUARD_SHARED_CHECKOUT=0 bash "$_GUARD_SCRIPT" --decide-primary "$_GUARD_PLATFORM_DIR" "$_GUARD_CWD" "$COMMAND" >/dev/null 2>&1 || true
    fi
elif [ -f "$_GUARD_SCRIPT" ]; then
    _GUARD_REASON=$(bash "$_GUARD_SCRIPT" --decide-primary "$_GUARD_PLATFORM_DIR" "$_GUARD_CWD" "$COMMAND" 2>/dev/null)
    _GUARD_STATUS=$?
    if [ "$_GUARD_STATUS" -eq 1 ]; then
        printf '%s\n' "$_GUARD_REASON"
        exit 1
    elif [ "$_GUARD_STATUS" -ne 0 ]; then
        echo "shell-exec: shared-checkout guard exited $_GUARD_STATUS at $_GUARD_SCRIPT (fail-open)" >&2
    fi
elif [ -d "$_GUARD_PLATFORM_DIR" ]; then
    echo "shell-exec: shared-checkout guard not found at $_GUARD_SCRIPT (fail-open)" >&2
fi
# --- end shared-checkout guard ---

if [ -n "$WORKDIR" ] && [ -d "$WORKDIR" ]; then
    cd "$WORKDIR" || exit 1
fi

eval "$COMMAND" 2>&1
