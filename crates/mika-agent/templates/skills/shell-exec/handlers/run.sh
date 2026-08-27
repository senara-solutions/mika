#!/bin/sh
# Shell execution handler.
# Input: JSON on stdin with "command" and optional "working_dir" fields
# Output: command output on stdout, errors on stderr
#
# SECURITY: This handler executes arbitrary commands. Use responsibly.

command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }

INPUT=$(cat)

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

if [ -n "$WORKDIR" ] && [ -d "$WORKDIR" ]; then
    cd "$WORKDIR" || exit 1
fi

eval "$COMMAND" 2>&1
