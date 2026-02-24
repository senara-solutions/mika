---
status: pending
priority: p2
issue_id: "197"
tags: [code-review, operational, agent-native]
dependencies: []
---

# Heartbeat Script Swallows curl Error Details

## Problem Statement
`heartbeat-all.sh` uses `curl -sf ... 2>/dev/null` which suppresses all error output. When a heartbeat fails, the warning message has no HTTP status code. An operator or agent cannot distinguish DNS failure, 401, 429, or timeout.

## Findings
- **Agent-native reviewer**: Warning severity (#6)
- Location: `scripts/heartbeat-all.sh` lines 62-73

## Proposed Solutions

### Option 1: Capture HTTP status code (Recommended)
Replace silent curl with status-code capture:

```bash
HTTP_CODE=$(curl -s -X POST \
    -H "Authorization: Bearer ${MIKA_INTERNAL_TOKEN}" \
    -H "Content-Type: application/json" \
    -o /dev/null -w '%{http_code}' \
    --connect-timeout 5 --max-time 10 \
    "${URL}" 2>/dev/null) || HTTP_CODE="000"
if [[ "$HTTP_CODE" =~ ^2 ]]; then
    SUCCEEDED=$((SUCCEEDED + 1))
else
    echo "  Warning: heartbeat failed for ${cid} (HTTP ${HTTP_CODE})" >&2
    FAILED=$((FAILED + 1))
fi
```

- **Effort**: Small (15 minutes)
- **Risk**: Low

## Technical Details
- **Affected Files**: `scripts/heartbeat-all.sh`

## Acceptance Criteria
- [ ] Failed heartbeats include HTTP status code in warning message
- [ ] HTTP 000 indicates connection failure (DNS/timeout)
- [ ] Successful 2xx status codes counted correctly

## Work Log
### 2026-02-24 - Found during code review
**By:** Agent-native reviewer
