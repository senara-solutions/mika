-- Persist the `one-telegram-one-mika` guard's refusal verdict (mika-cloud#208).
--
-- The guard itself is correct and unchanged: a Telegram account talks to
-- exactly one Mika. What was missing is any trace of its verdict. When
-- `handle_pairing` hits SQLSTATE 23505 on the `telegram_chat_id UNIQUE`
-- constraint it sends the user a Telegram message and returns — leaving the
-- refused customer row at `status = 'provisioned'`, `paired_at = NULL`,
-- `pairing_token` intact. That is byte-identical to a customer who has not
-- started pairing yet, so the console could not tell "refused" from "still
-- waiting" and the onboarding wizard announced completion on a pairing that
-- never happened.
--
-- These two columns are the verdict, written by
-- `crates/mika-gateway/src/routes.rs::handle_pairing` and read by the console
-- through `GET /admin/customers/{id}`. They are additive, nullable, and carry
-- no secret: adding them changes no pairing decision.
ALTER TABLE customers ADD COLUMN pairing_rejected_at TIMESTAMPTZ;
ALTER TABLE customers ADD COLUMN pairing_rejection_reason TEXT;

-- Closed reason vocabulary. The console maps a reason to a specific remedy, so
-- a free-text string would push gateway wording into the UI and drift. One
-- member today; the CHECK is what makes adding a second one deliberate.
-- The second clause keeps the pair coherent: both set, or both NULL — never a
-- timestamp with no reason, and never a reason with no timestamp.
ALTER TABLE customers ADD CONSTRAINT customers_pairing_rejection_reason_check
    CHECK (
        pairing_rejection_reason IS NULL
        OR pairing_rejection_reason IN ('telegram_already_linked')
    );

ALTER TABLE customers ADD CONSTRAINT customers_pairing_rejection_coherent_check
    CHECK (
        (pairing_rejected_at IS NULL AND pairing_rejection_reason IS NULL)
        OR (pairing_rejected_at IS NOT NULL AND pairing_rejection_reason IS NOT NULL)
    );
