#!/bin/bash
# Thin wrapper for dev-groom: dispatches /mika-groom-ticket via shared claude-pilot plumbing.
# All logic lives in _shared/dispatch-lib.sh — see mika#893.
set -e
# shellcheck source=../../_shared/dispatch-lib.sh
source "$(dirname "$0")/../../_shared/dispatch-lib.sh"
dispatch_claude_pilot "/mika-groom-ticket"
