//! mika#2066 AC2 — `mika-gateway --version` reports the commit stamp WITHOUT
//! loading configuration. Before this ticket the binary died on
//! `missing configuration field "database_url"` before it could answer.

use std::process::Command;

fn stamp(output: &str) -> &str {
    let open = output.find('(').expect("version output has no `(`");
    let close = output.find(')').expect("version output has no `)`");
    output[open + 1..close].trim()
}

#[test]
fn version_reports_a_non_empty_commit_stamp_without_config() {
    // Deliberately clear every MIKA_* var so reaching the settings loader would
    // fail — a successful exit proves the version flag short-circuits config.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mika-gateway"));
    for (k, _) in std::env::vars() {
        if k.starts_with("MIKA_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd
        .arg("--version")
        .output()
        .expect("failed to run mika-gateway --version");

    assert!(
        out.status.success(),
        "mika-gateway --version exited {:?} (config must not be required); stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).expect("non-utf8 --version output");
    assert!(
        stdout.starts_with("mika-gateway "),
        "expected `mika-gateway <version>`, got: {stdout:?}"
    );
    assert!(
        !stamp(&stdout).is_empty(),
        "commit stamp must be non-empty, got: {stdout:?}"
    );
}
