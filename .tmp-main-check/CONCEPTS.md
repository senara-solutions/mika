# Concepts

Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Guards

### Structural guard

A check that makes a forbidden pattern impossible to merge rather than discouraged, by mechanically rejecting it in CI. The distinguishing commitment is stated in the family's own headers as *construct the incapacity, don't promise the restraint*: a rule enforced by prose, a code comment, or an agent prompt is not a structural guard, however emphatic.

A structural guard is deny-by-default only to the extent its parser can model the source it reads. When it meets a form it was not built to parse it must fail closed and say so, because a partial audit and a complete one produce the same green check. A guard that can be wrong quietly is a claim, not a guard.

Where the guard and the component that acts on the value parse it separately, deny-by-default binds the *detection* step too, and in the opposite direction: the guard must read at least as permissively as that component, and may be strict only in what it then allows. A guard stricter than its consumer does not fail closed, because from its own side there is nothing to judge — it stays silent while the consumer proceeds. That gap has no rejection event and no red test by construction, so it is closed by tracing the value into the consumer, not by re-reading the guard.

### Anti-vacuity assertion

An assertion that proves a guard is capable of failing, by exercising it against a deliberately broken form and observing it go red. A guard that has only ever been observed passing has not been shown to test anything.

The broken form is synthesized — built by mutating the current source — rather than fetched from version history, because a reference to a branch stops naming the broken state once the fix merges. A case that cannot be constructed is reported as a failure, never as a skip: a silently skipped anti-vacuity assertion is the exact condition it exists to detect.

It proves the predicate that was written, not the perimeter that was intended. Every input reaching the mutated predicate had already been admitted as something to judge, so going red establishes that the decision is load-bearing and says nothing about what never got that far. Coverage of the detection step is a separate question, asked against the consumer rather than against the guard.

### Review-anchor attestation

The evidence a non-terminal architect disposition (`Disposition: READY`, `Verdict: GROOMED`) must carry to count as a review: at least three anchor lines (`A1:`…) each quoting at least forty characters of the brief, at distinct regions of it. A disposition keyword alone is an acknowledgement, not an attestation, and the engine treats an unattested one as no verdict at all — never as an approval.

"Quoting the brief" means its words, not its bytes: the brief is markdown and a model quotes what it read, so inline markers and apostrophe shape fold away on both sides before comparison, while a paraphrase or an exact quote of a different document is still refused. The comparison runs against the brief message of the turn being answered, never against a file the model opened or an earlier ticket in its session. A first miss earns one corrective re-prompt; a second is escalated with its cause (anchors found, anchors valid, miss reason) as a terminal `ESCALATE`, which downstream reads before any textual verdict tier. It is a runtime guard, not a [structural guard](#structural-guard).

### Sole writer

An audit or log name that exactly one module is allowed to emit, so that a query filtering on that name is the exact population of what that module did and nothing else. The property is held by a [structural guard](#structural-guard) that scans the source for the name outside its owning module, with an allowlist shipped empty; when the guard fires, the second site is removed, never allowlisted. Two spellings of one population, or a second writer of one name, split or pollute the population silently — no decision is wrong, only the operator's count.

Any name the module emits *instead of* the primary one under another disposition (see [gated disposition](#gated-disposition)) is a sole-writer name too, held by the same guard.

## Scans

### Gated disposition

The shape of a periodic scan whose detection runs unconditionally while only its effect is gated: an *armed* setting acts (removes, kills, transitions), an *observe* setting measures and records the same population without acting. Observation exists so an operator can read what the scan would do before letting it do it.

Every line the scan writes says what actually happened, never what was intended: under observe, an event, audit name, or message that names the action is a false record. Where the action's name is already contractual, it is reserved to the armed effect and the observed case gets its own name in the same family; where the existing name is that of the detection, the effect is carried by explicit fields. Either way the event, the audit name, and the message come from one source per disposition, so the surfaces cannot disagree. A write cap that bounds the armed effect also bounds what one observe pass names, so a full dry-run of a backlog takes as many passes as the cap implies.

## Pilot containment

### Pilot sandbox

The isolation boundary a headless development session runs inside: fresh kernel namespaces, a filesystem allowlist rather than the host root, a cleared environment repopulated from a narrow allowlist, and no host credential store bound in. Its governing property is stated as an invariant over what crosses the boundary — no bind-in carries a credential — rather than as a list of excluded files, so a new bind is audited rather than assumed safe.

The boundary constrains what the contained session can reach. It says nothing about what the launch itself exposes to the host, which is a separate question and has to be asked separately.

### Phase 2a / Phase 2b

The two containment postures the pilot sandbox runs in. **Phase 2b** is the full posture: filesystem, network and kernel cuts all active, with outbound traffic forced through a host-side relay. **Phase 2a** is the degraded fallback taken when the relay is unavailable — the filesystem and kernel cuts hold, the network does not.

The distinction is load-bearing beyond confinement strength: the attestation that unlocks the session's wider execution tier is set only under Phase 2b, so a degraded launch keeps the narrower tier rather than silently widening.

### Label churn

The remove→add cycle of the `ready` label that `auto_pull` Phase 2 performs on a stuck-ready ticket. It has two roles and one non-role: it resets the label's age so a rescued ticket whose pilot dies quickly self-throttles for a full threshold window (the mika#1824 D3 throttle), and it is a redundant trigger on a live webhook channel; since mika#2470 it is **never the trigger** — the rescue is dispatched by a direct in-process call of the ready-label handler first, and the churn is skipped altogether when that call did not dispatch but the engine already holds the ticket (`churn_is_moot`). Dispatch-before-churn is a timing margin, not an ordering invariant: the pilot's pgid is written by a spawned task after the handler returns.

### Containment canary

A one-command reproducer that spawns a real sandbox through the same code path a dispatch uses, then asserts both directions: that credentials and host state are unreachable from inside, and that the tools the session legitimately needs still work. It exists because a containment claim read from source is not a containment result — the author is not their own control — so it also offers an interactive mode an external reviewer can enter the sandbox through and probe by hand.

## Dispatch gates

### Grooming-provenance gate

The check that refuses an autonomous implementation dispatch on a ticket unless the loop itself groomed that ticket — a groom ran and converged. It is distinct from the body-marker check, which reads the ticket text for the grooming callout and can be satisfied by anything that writes text into the ticket; the provenance gate reads the loop's own record instead, so a hand-stamped callout does not pass it.

The gate refuses on every degraded case of its own read — no record, an unreadable store, a record aged past retention — never allows by default. A bypass flag on it is an operator decision with a measured expiry, not a standing configuration: a gate that only passes traffic while its bypass is set has never been observed working.

### Groom proof

The record the grooming-provenance gate reads: the loop's own statement that a groom converged, held on the row whose shape no other mechanism is entitled to change during the ticket's life. A record that lives on a row another process rewrites — the dispatch parent, which task reuse re-classes — is not a proof, because the read can fail for reasons unrelated to whether grooming happened.

A groom proof has a retention half-life: once the store prunes it, the ticket reads as never groomed and must be re-groomed through the loop. Grooming done outside the loop mints no proof by construction.

### Task reuse

The pattern by which one ticket keeps one dispatch identity across grooming and implementation: when a groom converges, the same parent task is re-classed from grooming to implementation and the pilot is launched against it, rather than a second parent being created. It exists so that an issue never holds two active dispatch rows at once. Its consequence for any reader is that the parent's class is not a stable fact — a check that needs "this was groomed" must read the [groom proof](#groom-proof), not the parent.

### Dispatch seat

Which *engine* owns a ticket while it is being written on: the autonomous loop, or one of the interactive Claude Code seats (`ssc`, `mpc`). Carried on the GitHub issue as a `dispatch:<seat>` label, read by the engine's seat gate, which refuses a loop dispatch on a ticket another seat has claimed. It is not the exec-slot lease (which bounds how many dispatches one agent runs, per class) and not the PR's `origin:` label (which records who produced an artefact and never changes). The vocabulary is written three times — Rust, `labels.yml`, and the shell literal in dispatch-lib — and guarded in pairs, so an undeclared seat is a label that label-sync deletes silently.

### Seat claim

The loop's own `dispatch:loop` label, stamped by dispatch-lib when it takes a ticket and released before the callback that lets the next dispatch start, with the exit trap as crash/cancel backstop. A live claim, not a provenance: between two dispatches an unclaimed ticket reads as unlabelled, and a human seat may take it. Stale only when a run died without its trap; the next dispatch on that ticket reads it as already owned, does not re-stamp, and releases it on its own way out.

## Flagged ambiguities

- *Guard* had been used for both a structural CI check and an in-process runtime assertion. In this glossary **structural guard** names the CI-enforced kind; a runtime assertion is not one.
