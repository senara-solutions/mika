//! Research scaffolding for ratified experiment protocols.
//!
//! Everything under this module is **disposable experiment apparatus**, not
//! product surface. Modules here exist to make a specific measurement
//! reproducible and are expected to be deleted once their protocol has run.
//! They carry no API stability promise, are not registered as agent tools, and
//! must not be wired into the agent loop.

pub mod mechanism_analyzer;
pub mod peer_b;
