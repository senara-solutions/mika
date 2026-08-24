// mika-gateway library surface.
//
// This crate is primarily a binary (`src/main.rs`). The library target exists
// to expose modules whose correctness is asserted from outside the binary —
// currently just the `voice` module, whose `trybuild` compile-fail fixtures
// under `tests/voice_lane_compile_fail/` need to `use mika_gateway::voice::...`.
//
// Do NOT add unrelated modules here. If a module wants a library surface, it
// must have a documented external consumer (a compile-fail fixture, another
// crate, or a doc-test) — otherwise it stays private to the binary.
//
// See senara-solutions/mika#1796.

pub mod voice;
