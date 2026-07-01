# Permission-policy shape: `if [ -f … ]; then …` (bash conditional test)

## Task context (task 1c6955c8, 2026-06-30T14:11:20Z — DB inspection)

Before querying the agent's SQLite database you must not assume it exists — on a fresh
checkout `~/.mika/data/mika.db` may be absent, and querying a missing file should be
skipped rather than error out.

## What to do

Give the shell command(s) that check whether the file `~/.mika/data/mika.db` exists and
only proceed to the query when it does. Write it the way you would in a script — a guard
around the query.
