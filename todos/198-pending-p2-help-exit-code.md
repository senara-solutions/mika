---
status: pending
priority: p2
issue_id: "198"
tags: [code-review, agent-native, usability]
dependencies: []
---

# Script --help Should Exit 0, Not 1

## Problem Statement
Both `provision.sh` and `deprovision.sh` have `usage()` functions that always `exit 1`. When invoked with `--help`, they return exit code 1. An agent checking if the script is installed (`provision.sh --help && echo OK`) gets a false negative.

## Findings
- **Agent-native reviewer**: Warning (#8)
- Location: `scripts/provision.sh` line 29, `scripts/deprovision.sh` line 21

## Proposed Solutions

### Option 1: Parameterize exit code (Recommended)
```bash
usage() {
    cat <<USAGE
...
USAGE
    exit "${1:-1}"
}
```
Then `--help) usage 0 ;;` and error paths call `usage` (defaults to 1).

- **Effort**: Small (5 minutes)
- **Risk**: Low

## Technical Details
- **Affected Files**: `scripts/provision.sh`, `scripts/deprovision.sh`, `scripts/heartbeat-all.sh`

## Acceptance Criteria
- [ ] `--help` exits 0 on all 3 scripts
- [ ] Error cases (missing args, unknown flags) still exit 1

## Work Log
### 2026-02-24 - Found during code review
**By:** Agent-native reviewer
