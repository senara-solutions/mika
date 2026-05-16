//! Library surface for `mika-cli`.
//!
//! The binary tree (rooted at `main.rs`) is independent of this lib. This file
//! exposes only the pieces that need to be reachable from integration tests
//! under `tests/`. Keep it minimal — most code stays binary-private.

pub mod supervision;
