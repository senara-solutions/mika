//! mika#2066 AC2 — `mika-spirit --version` reports the commit stamp WITHOUT
//! resolving a home dir or loading settings. A freshly installed, not-yet
//! configured server must still be interrogable for its provenance.

use std::process::Command;

fn stamp(output: &str) -> &str {
    let open = output.find('(').expect("version output has no `(`");
    let close = output.find(')').expect("version output has no `)`");
    output[open + 1..close].trim()
}

#[test]
fn version_reports_a_non_empty_commit_stamp_without_config() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mika-spirit"));
    for (k, _) in std::env::vars() {
        if k.starts_with("MIKA_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd
        .arg("--version")
        .output()
        .expect("failed to run mika-spirit --version");

    assert!(
        out.status.success(),
        "mika-spirit --version exited {:?} (config must not be required); stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).expect("non-utf8 --version output");
    assert!(
        stdout.starts_with("mika-spirit "),
        "expected `mika-spirit <version>`, got: {stdout:?}"
    );
    assert!(
        !stamp(&stdout).is_empty(),
        "commit stamp must be non-empty, got: {stdout:?}"
    );
}
