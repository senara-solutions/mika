---
name: mika-paste
description: Inject Wayland clipboard contents as the next message to orchestrator-Claude
argument-hint: "[<optional framing prefix>]"
allowed-tools: Bash(wl-paste:*)
---

$ARGUMENTS

!`wl-paste`
