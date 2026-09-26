#!/bin/bash
# Thin wrapper: dispatches claude-pilot via shared plumbing for the groom path.
# Entry command derives from $SKILL in the lib (case switch). See mika#1173.
set -e
# shellcheck source=../../_shared/dispatch-lib.sh
source "$(dirname "$0")/../../_shared/dispatch-lib.sh"
dispatch_claude_pilot
