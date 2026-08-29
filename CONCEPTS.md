# Concepts

Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Guards

### Structural guard

A check that makes a forbidden pattern impossible to merge rather than discouraged, by mechanically rejecting it in CI. The distinguishing commitment is stated in the family's own headers as *construct the incapacity, don't promise the restraint*: a rule enforced by prose, a code comment, or an agent prompt is not a structural guard, however emphatic.

A structural guard is deny-by-default only to the extent its parser can model the source it reads. When it meets a form it was not built to parse it must fail closed and say so, because a partial audit and a complete one produce the same green check. A guard that can be wrong quietly is a claim, not a guard.

### Anti-vacuity assertion

An assertion that proves a guard is capable of failing, by exercising it against a deliberately broken form and observing it go red. A guard that has only ever been observed passing has not been shown to test anything.

The broken form is synthesized — built by mutating the current source — rather than fetched from version history, because a reference to a branch stops naming the broken state once the fix merges. A case that cannot be constructed is reported as a failure, never as a skip: a silently skipped anti-vacuity assertion is the exact condition it exists to detect.

## Pilot containment

### Pilot sandbox

The isolation boundary a headless development session runs inside: fresh kernel namespaces, a filesystem allowlist rather than the host root, a cleared environment repopulated from a narrow allowlist, and no host credential store bound in. Its governing property is stated as an invariant over what crosses the boundary — no bind-in carries a credential — rather than as a list of excluded files, so a new bind is audited rather than assumed safe.

The boundary constrains what the contained session can reach. It says nothing about what the launch itself exposes to the host, which is a separate question and has to be asked separately.

### Phase 2a / Phase 2b

The two containment postures the pilot sandbox runs in. **Phase 2b** is the full posture: filesystem, network and kernel cuts all active, with outbound traffic forced through a host-side relay. **Phase 2a** is the degraded fallback taken when the relay is unavailable — the filesystem and kernel cuts hold, the network does not.

The distinction is load-bearing beyond confinement strength: the attestation that unlocks the session's wider execution tier is set only under Phase 2b, so a degraded launch keeps the narrower tier rather than silently widening.

### Containment canary

A one-command reproducer that spawns a real sandbox through the same code path a dispatch uses, then asserts both directions: that credentials and host state are unreachable from inside, and that the tools the session legitimately needs still work. It exists because a containment claim read from source is not a containment result — the author is not their own control — so it also offers an interactive mode an external reviewer can enter the sandbox through and probe by hand.

## Flagged ambiguities

- *Guard* had been used for both a structural CI check and an in-process runtime assertion. In this glossary **structural guard** names the CI-enforced kind; a runtime assertion is not one.
