fn main() {
    println!("cargo::rerun-if-changed=migrations");

    // mika#2066 AC3 — the git commit stamp is captured once, in mika-common's
    // build.rs, and read via `mika_common::build_info`. It used to be captured
    // here too (mika#354, for the `/version` endpoint); that duplication is gone.
}
