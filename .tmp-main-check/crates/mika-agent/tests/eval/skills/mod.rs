//! Per-skill eval scenarios.
//!
//! Each module here exercises one bundled skill's I/O contract end-to-end
//! through the agent loop with a synthetic skill registry and `MockLlmProvider`.
//! These tests validate engine *handling* of a skill's declared output shape,
//! not the production prompt's wording.

pub mod mika_arch_fire_disposition_gate;
pub mod mika_arch_groom_milestone;
