use std::fs;
use std::path::Path;

const DOCS: &[&str] = &[
    "architecture.md",
    "configuration.md",
    "deployment.md",
    "getting-started.md",
    "runtime-structure.md",
    "skills.md",
    "slash-commands.md",
];

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let docs_out = Path::new(&out_dir).join("docs");
    fs::create_dir_all(&docs_out).unwrap();

    // Try workspace root first (normal dev), fall back to crate-local (crates.io)
    let workspace_docs = Path::new(&manifest_dir).join("../../docs");
    let crate_docs = Path::new(&manifest_dir).join("docs");

    let source = if workspace_docs.join(DOCS[0]).exists() {
        &workspace_docs
    } else {
        &crate_docs
    };

    for file in DOCS {
        let src = source.join(file);
        println!("cargo:rerun-if-changed={}", src.display());
        fs::copy(&src, docs_out.join(file))
            .unwrap_or_else(|e| panic!("failed to copy {}: {e}", src.display()));
    }

    // Also copy openapi spec
    let api_src = source.join("openapi/mika-server.yaml");
    println!("cargo:rerun-if-changed={}", api_src.display());
    fs::create_dir_all(docs_out.join("openapi")).unwrap();
    fs::copy(&api_src, docs_out.join("openapi/mika-server.yaml"))
        .unwrap_or_else(|e| panic!("failed to copy {}: {e}", api_src.display()));
}
