#!/bin/bash
# Thin wrapper: dispatches claude-pilot via shared plumbing.
# Entry command is derived from $SKILL in the lib — see mika#893, mika#932.
set -e
# shellcheck source=../../_shared/dispatch-lib.sh
source "$(dirname "$0")/../../_shared/dispatch-lib.sh"
dispatch_claude_pilot
