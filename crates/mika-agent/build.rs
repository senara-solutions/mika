use std::fs;
use std::path::Path;

// Keep in sync with scripts/sync-agent-docs.sh DOCS array.
// CI enforces this via the docs-sync job in .github/workflows/ci.yml.
const DOCS: &[&str] = &[
    "architecture.md",
    "browser-control.md",
    "configuration.md",
    "deployment.md",
    "getting-started.md",
    "runtime-structure.md",
    "skills.md",
    "slash-commands.md",
    "task-system.md",
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

    // Copy dashboard assets into OUT_DIR so rust-embed can reference them via
    // $OUT_DIR/dashboard_dist (with the `interpolate-folder-path` feature).
    // This keeps the embedded path within the crate boundary, fixing `cargo package --verify`.
    copy_dashboard_assets(&manifest_dir, &out_dir);
}

/// Copy dashboard/dist/ into OUT_DIR/dashboard_dist/, filtering dotfiles.
/// If the source directory doesn't exist or is empty, creates an empty destination
/// directory so rust-embed (with `#[allow_missing]`) compiles with zero embedded files.
fn copy_dashboard_assets(manifest_dir: &str, out_dir: &str) {
    let dashboard_src = Path::new(manifest_dir).join("../../dashboard/dist");
    let dashboard_dst = Path::new(out_dir).join("dashboard_dist");

    // Always watch the source directory for additions/removals
    println!("cargo:rerun-if-changed={}", dashboard_src.display());

    // Ensure destination exists (even if empty)
    fs::create_dir_all(&dashboard_dst).unwrap();

    if !dashboard_src.exists() || !dashboard_src.is_dir() {
        println!(
            "cargo:warning=Dashboard assets not found at {} — the embedded dashboard will be empty. \
             Run: npm run build --prefix dashboard",
            dashboard_src.display()
        );
        return;
    }

    // Recursively copy, filtering dotfiles
    let copied = copy_dir_recursive(&dashboard_src, &dashboard_dst);

    if copied == 0 {
        println!(
            "cargo:warning=Dashboard dist directory exists but contains no embeddable files — \
             the embedded dashboard will be empty. Run: npm run build --prefix dashboard"
        );
    }
}

/// Recursively copy files from `src` to `dst`, skipping dotfiles.
/// Emits `cargo:rerun-if-changed` for each copied file.
/// Returns the number of files copied.
fn copy_dir_recursive(src: &Path, dst: &Path) -> usize {
    let mut count = 0;
    let entries = match fs::read_dir(src) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        // Skip dotfiles (.gitkeep, .DS_Store, etc.)
        if name.starts_with('.') {
            continue;
        }

        let src_path = entry.path();
        let dst_path = dst.join(&file_name);

        if src_path.is_dir() {
            fs::create_dir_all(&dst_path).unwrap();
            count += copy_dir_recursive(&src_path, &dst_path);
        } else {
            // Watch each file for content changes
            println!("cargo:rerun-if-changed={}", src_path.display());
            fs::copy(&src_path, &dst_path)
                .unwrap_or_else(|e| panic!("failed to copy {}: {e}", src_path.display()));
            count += 1;
        }
    }

    count
}
