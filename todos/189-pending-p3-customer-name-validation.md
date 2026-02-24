---
status: pending
priority: p3
issue_id: "189"
tags: [code-review, security, plan-review]
dependencies: ["179"]
---

# CUSTOMER_NAME Input Validation and AWS Tags Sanitization

## Problem Statement
CUSTOMER_NAME has no input validation — no length check, no character restriction. It's passed to AWS tags (which have character/length restrictions), Helm `--set` (where special chars could break YAML), and psql. Additionally, `force-delete-without-recovery` in deprovision.sh permanently destroys AWS secrets with no recovery window.

## Proposed Solutions

### Add input validation after parsing:
```bash
if [[ ! "$CUSTOMER_NAME" =~ ^[a-zA-Z0-9\ \'\.\-]{1,100}$ ]]; then
    echo "Error: customer_name must be 1-100 chars, alphanumeric/spaces/hyphens/periods only"
    exit 1
fi
```

### Use recovery window for deprovision AWS secret deletion:
```bash
aws secretsmanager delete-secret --recovery-window-in-days 7 ...
```
Keep `--force-delete-without-recovery` only in provision.sh rollback path.

## Acceptance Criteria
- [ ] CUSTOMER_NAME validated before use
- [ ] Deprovision uses 7-day recovery window (not force-delete)

## Work Log

### 2026-02-24 - Plan Review Finding
**By:** Security sentinel
