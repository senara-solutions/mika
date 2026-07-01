# Substrate diagnostic — task table snapshot

The dispatch queue looks wedged. Nothing has dispatched in the last two hours
despite two `ready`-labelled issues. Here is the current `tasks` table state
(from `sqlite3 ~/.mika/data/mika.db`):

```
id        trigger_type  action_type    status     parent_task_id  reference_url                                   updated_at
--------  ------------  -------------  ---------  --------------  ---------------------------------------------  --------------------
7a3f...   manual        none           completed  (null)          https://github.com/senara-solutions/mika/issues/1620  2026-06-29T09:14:03Z
787d4f..  callback      resume_agent   pending    7a3f...          (null)                                         2026-06-29T09:15:41Z
b91c...   manual        none           pending    (null)          https://github.com/senara-solutions/mika/issues/1622  2026-06-29T07:02:10Z
```

Additional context:

- Task `7a3f...` is the parent supervisor task for issue #1620. It transitioned
  to `completed` at 09:14:03 — the PR was opened and the dispatch finished.
- Task `787d4f...` is its `callback` child. It is still `pending` at 09:15:41,
  after the parent completed. There is no live subprocess for it (`/proc` shows
  no matching PID). It has been pending for ~2 hours.
- Task `b91c...` is a `manual` supervisor task for issue #1622, created at
  07:02:10. It is `pending` and has **no callback child row at all** — it was
  created but a dispatch was never fired for it. Issue #1622 carries the `ready`
  label. This row predates the engine-side ready-label webhook handler change.

The dispatch slot appears occupied. What is wedged, and what is the fix for each?
