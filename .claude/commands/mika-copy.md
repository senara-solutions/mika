---
name: mika-copy
description: Copy the orchestrator's last assistant message to the Wayland clipboard
argument-hint: ""
allowed-tools: Bash(scripts/mika-copy-last-assistant:*), Bash(/data/workspace/mika-platform/scripts/mika-copy-last-assistant:*), Bash(D=*:*)
---

!`D="$PWD"; while [ "$D" != "/" ]; do S="$D/scripts/mika-copy-last-assistant"; if [ -x "$S" ]; then exec "$S"; fi; D="$(dirname "$D")"; done; echo "mika-copy: script not found in any ancestor scripts/ directory (looked from $PWD up)" >&2; exit 1`
