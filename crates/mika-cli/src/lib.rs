//! Library surface for `mika-cli`.
//!
//! The binary tree (rooted at `main.rs`) is independent of this lib. This file
//! exposes only the pieces that need to be reachable from integration tests
//! under `tests/`. Keep it minimal — most code stays binary-private.

pub mod ask_message;
pub mod budget_floor;
pub mod remote_ask;
pub mod supervision;
