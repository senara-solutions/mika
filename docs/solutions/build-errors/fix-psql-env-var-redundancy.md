---
title: Remove redundant environment variable prefixes from psql invocation
date: 2026-02-24
category: build-errors
severity: low
tags: [shellcheck, bash, psql, SC2097, SC2098, parameter-passing]
affected_modules: [scripts/provision.sh]
root_cause: VAR="$VAR" command prefix pattern was unnecessary because psql -v flags already read variables from the current shell before forking
time_to_resolve: 5 minutes
---

# Remove Redundant Environment Variable Prefixes from psql Invocation

## Problem Symptom

Shellcheck flagged 10 warnings (SC2097 + SC2098) in `scripts/provision.sh`:

```
provision.sh:158: SC2097 (warning): This assignment is only seen by the forked process.
provision.sh:165: SC2098 (warning): This expansion will not see the mentioned assignment.
```

Five SC2097 warnings for each `VAR="$VAR"` prefix line, and five SC2098 warnings for each `${VAR}` expansion in the `-v` flags.

## Root Cause

The script combined two separate variable-passing mechanisms:

```bash
# The problematic pattern
CUSTOMER_ID="$CUSTOMER_ID" \
CUSTOMER_NAME="$CUSTOMER_NAME" \
PLAN="$PLAN" \
TIMEZONE="$TIMEZONE" \
PAIRING_TOKEN="$PAIRING_TOKEN" \
psql "${DATABASE_URL}" \
    -v ON_ERROR_STOP=1 \
    -v customer_id="${CUSTOMER_ID}" \
    -v customer_name="${CUSTOMER_NAME}" \
    -v plan="${PLAN}" \
    -v timezone="${TIMEZONE}" \
    -v pairing_token="${PAIRING_TOKEN}" \
    <<'SQL'
INSERT INTO customers ...
SQL
```

**Why it's wrong:**

1. `VAR="$VAR" command` sets env vars in the **child process only** (SC2097)
2. `-v var="${VAR}"` expands `${VAR}` in the **parent shell before forking** (SC2098)
3. The parent shell already has these variables defined earlier in the script
4. The env var prefix is completely redundant — it adds nothing

The code worked accidentally because the parent shell had the variables. The prefix was a misconception about how psql's `-v` flag works.

## Investigation Steps

1. Installed shellcheck and ran it on all 3 scripts
2. Identified SC2097/SC2098 pattern in provision.sh only
3. Confirmed the variables were already in scope from earlier argument parsing
4. Verified the `-v` flags read from parent shell, not child environment

## Working Solution

Remove the redundant env var prefix lines:

```bash
# BEFORE (10 shellcheck warnings)
CUSTOMER_ID="$CUSTOMER_ID" \
CUSTOMER_NAME="$CUSTOMER_NAME" \
PLAN="$PLAN" \
TIMEZONE="$TIMEZONE" \
PAIRING_TOKEN="$PAIRING_TOKEN" \
psql "${DATABASE_URL}" \
    -v ON_ERROR_STOP=1 \
    -v customer_id="${CUSTOMER_ID}" \
    ...

# AFTER (clean)
psql "${DATABASE_URL}" \
    -v ON_ERROR_STOP=1 \
    -v customer_id="${CUSTOMER_ID}" \
    ...
```

The `-v` flags continue to work unchanged — they expand `${CUSTOMER_ID}` in the parent shell and pass values to psql's `\set` mechanism for use as `:'customer_id'` in SQL.

## Verification

```bash
$ shellcheck scripts/provision.sh scripts/deprovision.sh scripts/heartbeat-all.sh
# (no output — zero warnings across all 3 scripts)
```

## Prevention Strategies

### 1. Run shellcheck in CI

Add to GitHub Actions or pre-commit:

```yaml
# .github/workflows/lint.yml
- name: Shellcheck
  run: shellcheck scripts/*.sh
```

### 2. Correct psql pattern reference

Always use this pattern for parameterized psql in bash:

```bash
psql "${DATABASE_URL}" \
    -v ON_ERROR_STOP=1 \
    -v var_name="${SHELL_VAR}" \
    <<'SQL'
SELECT * FROM table WHERE id = :'var_name'::uuid;
SQL
```

Key points:
- `-v` reads from parent shell — no env prefix needed
- Single-quoted heredoc (`<<'SQL'`) prevents shell expansion in SQL
- `:'var_name'` syntax in SQL references the psql variable (with quoting)

### 3. Common SC2097/SC2098 pitfalls

| Pattern | Correct? | Why |
|---------|----------|-----|
| `psql -v var="${VAL}"` | Yes | `-v` reads from parent shell |
| `VAR="${VAL}" psql -v var="${VAR}"` | No | Redundant env prefix (SC2097/SC2098) |
| `VAR="${VAL}" psql -c "SELECT $VAR"` | No | SQL injection + SC2097 |
| `psql -v var="${VAL}" <<'SQL' ... :'var' ... SQL` | Yes | Safe parameterized pattern |

## Cross-References

- `todos/191-pending-p1-rollback-psql-variable-bug.md` — Related: rollback function missing `-v` flag entirely
- `todos/179-complete-p1-sql-injection-provisioning-scripts.md` — Established the psql `\set` pattern
- `docs/plans/2026-02-24-feat-helm-charts-provisioning-scripts-plan.md` — Original plan specifying parameterized SQL
- [SC2097 documentation](https://www.shellcheck.net/wiki/SC2097)
- [SC2098 documentation](https://www.shellcheck.net/wiki/SC2098)
