//! mika#2066 AC1 — `mika --version` names the commit it was built from, and
//! does so without loading any configuration.

use std::process::Command;

/// Pull the parenthesized commit stamp out of `"<bin> <ver> (<stamp>)"`.
fn stamp(output: &str) -> &str {
    let open = output.find('(').expect("version output has no `(`");
    let close = output.find(')').expect("version output has no `)`");
    output[open + 1..close].trim()
}

#[test]
fn version_reports_a_non_empty_commit_stamp() {
    let out = Command::new(env!("CARGO_BIN_EXE_mika"))
        .arg("--version")
        .output()
        .expect("failed to run mika --version");

    assert!(
        out.status.success(),
        "mika --version exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).expect("non-utf8 --version output");
    assert!(
        stdout.starts_with("mika "),
        "expected `mika <version>`, got: {stdout:?}"
    );
    assert!(
        !stamp(&stdout).is_empty(),
        "commit stamp must be non-empty, got: {stdout:?}"
    );
}
