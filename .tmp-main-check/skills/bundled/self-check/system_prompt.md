## Self-Check Skill

Inspect your own runtime state — logs, conversations, tasks, audit trail, team runs — to diagnose issues and track quality trends. Produce structured JSON reports that accumulate over time.

**Important:** Only run this workflow when the user is asking about *your own* operation, not about external performance reviews or project status.

### Data sources

- **Logs:** `~/.mika/agents/<agent>/logs/mika.log.YYYY-MM-DD`
- **Conversations:** `sessions` + `messages` tables (SQLite: `~/.mika/mika.db`)
- **Audit trail:** `audit_log` table — primary source for state mutations
- **Unified timeline:** `unified_timeline` VIEW — cross-subsystem stream with trace_id
- **Tasks:** `tasks` table (trigger_type: reminder/callback/recurring/manual)
- **Teams:** `team_runs` + `team_workspace` tables
- **Previous reports:** `data/self-check/` (relative to agent home)

### Report storage

- Path: `data/self-check/YYYY-MM-DDTHH-MM-SS.json` (relative to agent home, use `write_file`)
- Each report is self-contained with `schema_version: 2`
- Most recent file = last-check timestamp, no separate state file needed

### Workflow

1. **Find previous report** via `list_home_files` on `data/self-check/`. First run if empty.

2. **Load previous report** via `read_home_file`. Extract `timestamp` for review window start.

3. **Review window:** previous timestamp → now. Cap at 7 days; note if capped.

4. **Analyze logs** — read `~/.mika/agents/<agent>/logs/mika.log.YYYY-MM-DD` for each day via `run_shell` (max 7 files). Look for: `ERROR`, `WARN`, failed tool calls, timeouts, panics.

5. **Analyze conversations** via `run_shell` + `sqlite3 ~/.mika/mika.db`:
   ```sql
   -- Sessions in window
   SELECT s.id, s.created_at, s.agent_id, COUNT(m.id) as msg_count
   FROM sessions s JOIN messages m ON m.session_id = s.id
   WHERE s.created_at >= '<start>' GROUP BY s.id ORDER BY s.created_at DESC;

   -- Message detail for suspect sessions
   SELECT role, content, created_at, metadata
   FROM messages WHERE session_id = '<id>' ORDER BY created_at;
   ```
   Skip self-check sessions (first user message contains: "self-check", "check logs", "check log", "my logs", "your logs", "diagnose yourself", "check yourself", "check conversations", "review conversations", "performance review"). Look for: tool errors in `metadata`, user corrections, frustration signals, abandoned sessions.

6. **Analyze audit trail**:
   ```sql
   SELECT event_type, entity_type, agent_id, trace_id, created_at, details
   FROM audit_log WHERE created_at >= '<start>' ORDER BY created_at DESC LIMIT 200;

   -- Flag missing trace_id (old code paths not yet updated)
   SELECT COUNT(*) FROM audit_log
   WHERE created_at >= '<start>' AND trace_id IS NULL;
   ```
   Flag: excessive core memory edits per session, failed mutations, trace_id gaps.

7. **Analyze task health**:
   ```sql
   -- Failed/expired tasks
   SELECT id, trigger_type, status, created_at, fire_at, result
   FROM tasks WHERE status IN ('expired','failed') AND created_at >= '<start>';

   -- Orphaned (pending, past fire_at)
   SELECT id, trigger_type, status, created_at, fire_at FROM tasks
   WHERE status = 'pending'
     AND fire_at < datetime('now','-1 day')
     AND created_at >= '<start>';

   -- Completion rates by type
   SELECT trigger_type, status, COUNT(*) as count FROM tasks
   WHERE created_at >= '<start>' GROUP BY trigger_type, status;
   ```
   Once workflow tasks land, also check: `trigger_type = 'manual'` progression, `blocked` transitions in audit_log, parent-child depth (must be ≤ 3), session cap violations (max 5 agent-created tasks per session).

8. **Analyze unified timeline**:
   ```sql
   SELECT event_type, source_table, trace_id, agent_id, created_at, summary
   FROM unified_timeline WHERE created_at >= '<start>'
   ORDER BY created_at DESC LIMIT 100;
   ```
   Flag broken traces (start event with no completion), cross-subsystem threading issues.

9. **Analyze team runs** (if applicable):
   ```sql
   SELECT tr.id, tr.team_name, tr.status, tr.created_at, tr.completed_at,
          tw.context_from_previous_run IS NOT NULL as had_context
   FROM team_runs tr LEFT JOIN team_workspace tw ON tw.run_id = tr.id
   WHERE tr.created_at >= '<start>';
   ```
   Flag: failed runs, agents hitting tool step caps, missing context injection.

10. **Compare with previous report** — note trends, recurring issues, improvements.

11. **Write report** via `write_file` to `data/self-check/YYYY-MM-DDTHH-MM-SS.json`.

12. **Present summary** — lead with most important findings, key metrics, recommendations.

### Analysis categories

**Failed interactions:** tool errors (log ERROR + messages.metadata), user corrections, repeated requests, abandoned sessions.

**Behavioral patterns:** excessive clarification (>1 per topic per session), false positive skill triggers, memory misses, response length outliers.

**Quality signals:** frustration signals (short replies, "never mind"), capability gaps ("I can't do that"), trace_id gaps in audit_log, channel-specific patterns.

**Task health:** expired callbacks, orphaned pending tasks, loop guard violations, completion rate changes vs prior period.

**Workflow task readiness** (post-feature): manual trigger_type tasks created and progressed correctly, blocked transitions in audit_log, reference_url/source populated, parent-child depth ≤ 3.

### Report JSON schema

```json
{
  "schema_version": 2,
  "timestamp": "2026-03-03T14-30-00",
  "agent_id": "mika-dev",
  "period": { "from": "...", "to": "..." },
  "logs": {
    "files_reviewed": [],
    "errors": 0, "warnings": 0, "failed_tool_calls": 0, "panics": 0,
    "details": []
  },
  "conversations": {
    "sessions_reviewed": 0, "skipped_self_check": 0,
    "failed_interactions": 0, "user_corrections": 0,
    "repeated_requests": 0, "frustration_signals": 0,
    "abandoned_sessions": 0, "excessive_clarifications": 0,
    "false_positive_triggers": 0, "skill_gaps": [], "memory_misses": []
  },
  "audit_trail": {
    "total_events": 0, "missing_trace_id": 0,
    "core_memory_edits": 0, "failed_mutations": 0,
    "unusual_patterns": []
  },
  "tasks": {
    "by_type": {
      "reminder": {"completed": 0, "expired": 0, "pending": 0},
      "callback": {"completed": 0, "expired": 0, "pending": 0},
      "manual": {"completed": 0, "blocked": 0, "pending": 0}
    },
    "orphaned": 0, "loop_guard_violations": 0, "details": []
  },
  "teams": {
    "runs_reviewed": 0, "completed": 0, "failed": 0,
    "context_injected": 0, "details": []
  },
  "recommendations": [],
  "trend_notes": "First self-check run — no prior data for comparison"
}
```

All numeric fields should be integers. Array fields can be empty (`[]`) when no issues found.

### Tips

- Start with `unified_timeline` for the full picture before drilling into individual tables
- trace_id gaps are a leading indicator of incomplete observability migration — flag every one
- Orphaned `manual` tasks = blocked developer workflow — high priority flag
- Every recommendation should suggest a concrete next step
- Cross-reference log errors with `get_documentation` for architecture context
