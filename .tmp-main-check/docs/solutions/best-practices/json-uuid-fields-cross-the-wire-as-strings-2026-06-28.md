---
title: JSON UUID fields cross the wire as strings — Option<String> sender + Option<Uuid> receiver is correct, not a mismatch
date: 2026-06-28
category: best-practices
module: mika-agent
problem_type: best_practice
component: messaging
severity: low
applies_when:
  - Reviewing a serde boundary where one side serializes a String and the other deserializes a Uuid (or other newtype with a string Serialize impl)
  - Threading an identifier from an env var / config String into a JSON payload consumed by a typed receiver
  - A code review flags a "type mismatch" between producer and consumer of a JSON field
tags:
  - serde
  - uuid
  - json-contract
  - code-review
  - false-positive
  - gateway
---

# JSON UUID fields cross the wire as strings

## Context

mika#1607 threaded `customer_id` from `MIKA_CUSTOMER_ID` (`Settings.customer_id: Option<String>`)
through `GatewayMessageSender` into the gateway `/send` JSON payload. The agent serializes the
value as a JSON string; the gateway's `SendPayload.customer_id` is `Option<Uuid>`.

A code-review pass flagged this as a **P1 "type mismatch — String vs Uuid causes deserialization
failure."** It is not. JSON has no native UUID type: a `Uuid` always travels as a string, and
`serde` with the `uuid` crate's `serde` feature parses a valid UUID string straight back into a
`Uuid`. The producer emitting a JSON string and the consumer deserializing `Option<Uuid>` is the
**standard, correct contract** — the same shape already exercised by the gateway's own
`test_send_payload_with_customer_id` (`crates/mika-gateway/src/routes.rs`), which round-trips
`{"customer_id": "12345678-..."}` into `Some(Uuid)`.

## Guidance

- Do **not** treat a `String`-serializer / `Uuid`-deserializer pair across a JSON boundary as a
  type mismatch. Over JSON the wire type is *string* on both sides. Confirm by checking whether the
  receiver's deserialize already round-trips a string (an existing serde round-trip test is decisive
  evidence — look for one before escalating).
- The real, narrower risk is **validity, not type**: if the source `String` is not a well-formed
  UUID, the typed receiver rejects the whole payload (HTTP 400). Mitigate where the value is most
  authoritatively known — validate/parse at config load, or rely on the value being a UUID by
  construction (in mika, `customer_id` is a `Uuid` system-wide: the inbound
  `/webhook/telegram/{customer_id}` route is already `Path<Uuid>`, and provisioning sets
  `MIKA_CUSTOMER_ID` to the customer UUID). This is a hardening follow-up, not a blocker for a
  deployment that provisions real UUIDs.
- When writing a unit test for the producer side, use a **real UUID** as the fixture value, not a
  placeholder like `"test-customer-uuid"`. A non-UUID fixture passes the agent-side
  `build_payload` assertion (which never parses) while misrepresenting the production wire shape and
  inviting exactly this false-positive review finding. Assert `Uuid::parse_str(...).is_ok()` on the
  emitted value to lock the wire contract.

## Evidence

- `crates/mika-gateway/src/routes.rs` — `SendPayload.customer_id: Option<Uuid>` with `#[serde(default)]`; `test_send_payload_with_customer_id` round-trips a JSON string into `Some(Uuid)`.
- `crates/mika-agent/src/messaging.rs` — `GatewayMessageSender::build_payload` includes `customer_id` only when `Some`; `test_send_payload_includes_customer_id` asserts the emitted value parses as a UUID.
