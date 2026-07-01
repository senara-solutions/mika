# Deploy window — merged code not yet live

Four PRs merged to `main` in the last hour across the workspace:

```
senara-solutions/mika       #1623  feat(kg): add corpus drift probe        MERGED
senara-solutions/mika       #1626  fix(agent): callback watchdog grace     MERGED
senara-solutions/mika       #1628  fix(dispatch-lib): wip-rescue guard     MERGED
senara-solutions/mika-cloud #71    chore(helm): bump agent resources       MERGED
```

The running services (`mika-spirit`, `mika-gateway`) are still on the previous
build — `mika status` reports the deployed SHA is behind `origin/main`. The
merged code is not live yet.

Current checkout state across sub-repos:

```
mika/            on main, up to date with origin/main
mika-cloud/      on main, up to date with origin/main
mika-skills/     on main, up to date with origin/main
claude-pilot-py/ on main, up to date with origin/main
```

How do you get the merged code onto the running services?
