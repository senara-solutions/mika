---
module: mika-gateway/pairing
tags: [pairing, telegram, guard, silent-failure, state-modeling, one-telegram-one-mika]
problem_type: bug
category: error-handling
---

# A guard whose refusal writes nothing leaves the refused row indistinguishable from an untouched one

## Problem

`handle_pairing` (`crates/mika-gateway/src/routes.rs`) enforces the `one-telegram-one-mika` invariant through the `telegram_chat_id BIGINT UNIQUE` constraint from `migrations/001_customers.sql`. When a second customer presents a pairing token for an already-bound Telegram account, the pairing `UPDATE` fails with SQLSTATE 23505. The handler sent the user a Telegram message and returned.

Nothing was persisted. The refused row stayed `status = 'provisioned'`, `paired_at NULL`, `pairing_token` intact — byte-identical to a customer who had never started pairing.

That ambiguity is not local to the gateway. `GET /admin/customers/{id}` is how the console reads pairing state, so a refusal it cannot see becomes, downstream, "still waiting". The mika-cloud onboarding wizard rendered "Mika is ready!" over a refused pairing; the user's messages went on being answered by the bot that already held the binding, with that agent's memory. Founding incident: senara-solutions/mika-cloud#208.

The guard was right. Only its silence was wrong.

## Solution

Persist the verdict and project it. No condition of the guard changed, and the Telegram refusal message is unchanged to the character.

`migrations/010_customers_pairing_rejection.sql` adds `pairing_rejected_at` and `pairing_rejection_reason` — both nullable, both additive — under a `CHECK` restricting the reason to a closed vocabulary (`telegram_already_linked` today) and a second `CHECK` keeping the pair coherent: both set, or both NULL.

Three code sites, and the second and third are what make it correct rather than merely present:

1. **Write** — a fire-and-forget `UPDATE` in the 23505 branch, keyed on `pairing_token = $2` so it lands on the row that was refused and never on the row holding the binding. The refusal is already decided when control reaches this point; a failed write logs `warn!` and lets it stand.
2. **Clear on success** — the successful-pairing `UPDATE` nulls both columns in the same atomic statement that sets `paired_at`. A separate clearing statement would open a window where a row reads both paired and refused.
3. **Clear on token re-issue** — the `ON CONFLICT` branch of the customer upsert (`UPSERT_CUSTOMER_SQL`) nulls both columns under the same condition that promotes a fresh pairing token. A refused customer stays `provisioned`, which is precisely the branch that mints a new token, so a spent verdict would otherwise surface as a refusal the user has not made yet.

`CUSTOMER_SAFE_COLUMNS` projects both columns. Neither is a secret: the reason names the *class* of refusal, never the customer holding the binding. The existing anti-leak assertions pass unmodified.

## Prevention

- **A guard that can refuse produces three outcomes, not two.** Any consumer whose type carries two will collapse the refusal into the nearest neighbour, and "not yet" is the friendliest lie available. Check what a downstream reader can observe *after* the guard fires; if it is the same thing they would observe had nothing happened, the guard is silent.
- **Every recorded verdict needs a clearing site for each way it can become spent.** Here there were two. Only the success path was found while planning; the token-re-issue path surfaced in review, and missing it would have replaced one stale-state lie with another.
- **A test that reads its own source cannot fail.** The first version of the re-issue test asserted `include_str!("routes.rs").contains("<the exact string in the assertion>")` — permanently green regardless of the code. Extracting the SQL into the named `UPSERT_CUSTOMER_SQL` constant and asserting on that is what turned it into a real test.

`tests/pairing_rejection.rs` is the DB-backed regression (`#[ignore]` like its neighbours — CI provisions no Postgres for this crate). Its first case asserts the guard still refuses with 23505: the invariant is what the rest of the file is built to protect, not to relax.

The console-side half of this fix, and the user-facing shape of the failure, are written up in mika-cloud at `docs/solutions/error-handling/a-correct-refusal-that-persists-nothing-reads-as-never-happened-2026-09-04.md`.
