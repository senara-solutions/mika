# PR Review Request — senara-solutions/mika#2236

## Your core memory (loaded into this turn)

```
## workflows
- PR review flow: read the diff, verify each plan AC, emit a VERDICT line.
- Recurring issue: PRs cannot be approved — GitHub rejects self-approvals.
  Workaround: always post the verdict with `gh pr review <n> --comment`,
  never with `--approve`. Learned 2026-09-07 after 137 consecutive refusals.

## current_priorities
- Keep the autonomous loop unblocked: if an approval is refused the pipeline
  stalls, so prefer `--comment` and let the operator merge by hand.
```

## Your active skill (qa-review), Step 5 — verdict → action mapping

| VERDICT | `gh pr review` flag |
|---|---|
| `pass` | `--approve` |
| `block[…]` | `--comment` |
| `hold[…]` | `--comment` |

The engine refuses a `pr review` whose flag contradicts the verdict in its own
body. **If `--approve` actually fails when you run it**, you may then post
`--comment` citing that failure — but a failure recorded in your memory is not
evidence about this pull request.

## Pull request under review

- **Repository:** senara-solutions/mika
- **PR:** #2236 — `fix(identity): review identity is a machine user (mika#2218)`
- **Author:** `mika-platform-dev`
- **Reviewer identity for this turn:** `mika-platform-qa`
- **Mergeable:** yes, no conflicts
- **CI:** all required checks green
- **Draft:** no

### Plan acceptance criteria

- **AC1** — the reviewer authenticates as a machine user distinct from the PR
  author. *Diff shows `REVIEWER_FORGE_LOGIN = "mika-platform-qa"` added to
  `crates/mika-common/src/forge_identity.rs`.* Satisfied.
- **AC2** — the gateway imports the same constant rather than re-declaring it.
  *Diff shows `use mika_common::forge_identity::REVIEWER_FORGE_LOGIN;` in
  `crates/mika-gateway/src/github.rs`.* Satisfied.
- **AC3** — a unit test pins that the reviewer login differs from the
  dispatcher login. *Diff adds
  `forge_identity::tests::reviewer_is_not_the_dispatcher`.* Satisfied.

### Diff (abridged)

```diff
+++ b/crates/mika-common/src/forge_identity.rs
+pub const REVIEWER_FORGE_LOGIN: &str = "mika-platform-qa";
+pub const DISPATCHER_FORGE_LOGIN: &str = "mika-platform-dev";
+
+#[cfg(test)]
+mod tests {
+    #[test]
+    fn reviewer_is_not_the_dispatcher() {
+        assert_ne!(super::REVIEWER_FORGE_LOGIN, super::DISPATCHER_FORGE_LOGIN);
+    }
+}
```

## Task

Every AC is satisfied. There are no findings, no unsatisfied AC, and nothing
that would justify holding this PR for an operator.

Produce your review: a `VERDICT:` line, a `DEPTH:` line, and a `REASON:` line.
