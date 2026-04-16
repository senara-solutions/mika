use std::fs;
use std::path::Path;

// Shared discovery logic for `skills/bundled/`. Lives outside `src/` so both
// this build script and the crate's integration tests can consume it without
// circular dependencies.
#[path = "build_support/bundled_skills_discover.rs"]
mod bundled_skills_discover;

use bundled_skills_discover::discover_bundled_skills;

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

    // Walk `skills/bundled/` at the workspace root and generate a Rust source file
    // at $OUT_DIR/bundled_skills_generated.rs declaring `static ENTRIES: &[BundledSkill]`.
    // Consumed by crates/mika-agent/src/bundled_skills.rs via include!().
    generate_bundled_skills_table(&manifest_dir, &out_dir);
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

/// Walk `skills/bundled/` at the workspace root and generate a Rust source file
/// declaring `static ENTRIES: &[BundledSkill] = &[ ... ]`.
///
/// Design:
/// - Production `skills/bundled/` is empty in this ticket (only `.gitkeep`).
/// - An empty or missing directory produces `ENTRIES: &[BundledSkill] = &[];`.
/// - Subdirectories without `skill.toml` are skipped (handles `.gitkeep`, stray files).
/// - File discovery is shallow plus one level of `handlers/` nesting, matching the
///   existing `crates/mika-agent/templates/skills/<name>/` layout.
/// - Files under `handlers/` ending in `.sh` are marked `executable: true`.
/// - Symlinks are filtered for defense-in-depth (matches runtime symlink refusal
///   in `bundled_skills.rs::write_skill`).
/// - Absolute paths are embedded in `include_str!` to avoid any relative-path
///   resolution surprises when the generated file is `include!()`d.
///
/// The generator does NOT parse `skill.toml` — validation is runtime's job
/// (`SkillRegistry::validate_loaded()`).
fn generate_bundled_skills_table(manifest_dir: &str, out_dir: &str) {
    let bundled_root = Path::new(manifest_dir).join("../../skills/bundled");
    let output_path = Path::new(out_dir).join("bundled_skills_generated.rs");

    // Always watch the base directory — new subdirectories should trigger a rebuild.
    println!("cargo:rerun-if-changed={}", bundled_root.display());

    let mut out = String::new();
    out.push_str("// @generated by build.rs — do not edit by hand.\n");
    out.push_str("// Source: <workspace>/skills/bundled/*/\n\n");

    let entries = discover_bundled_skills(&bundled_root);

    // Also emit rerun-if-changed for each discovered skill directory (and its
    // handlers/ subdirectory when present). Without this, adding a new file to
    // an already-discovered skill doesn't touch the parent `skills/bundled/`
    // mtime and cargo won't re-run build.rs until the directory itself changes.
    for entry in &entries {
        println!("cargo:rerun-if-changed={}", entry.abs_dir.display());
        let handlers = entry.abs_dir.join("handlers");
        if handlers.exists() {
            println!("cargo:rerun-if-changed={}", handlers.display());
        }
    }

    if entries.is_empty() {
        out.push_str("static ENTRIES: &[BundledSkill] = &[];\n");
    } else {
        // Emit one `static SKILL_<INDEX>_ENTRY_FILES: &[SkillFile]` per skill.
        // Index-based identifiers avoid collisions from case-variant directory
        // names (`foo` vs `FOO`) or hyphen-vs-underscore ambiguity
        // (`my-skill` vs `my_skill`), both of which would otherwise produce
        // duplicate Rust static names and an opaque compile error.
        // The human-readable name travels in the `name:` field of each
        // BundledSkill, where Debug-formatted string literals handle all
        // escaping correctly.
        for (idx, entry) in entries.iter().enumerate() {
            out.push_str(&format!(
                "static SKILL_{idx}_ENTRY_FILES: &[SkillFile] = &[\n"
            ));
            for file in &entry.files {
                // Use {:?} (Rust Debug formatter) so all string literals in the
                // generated code are escaped correctly — handles backslashes,
                // double-quotes, and non-ASCII control characters uniformly
                // for both `path` and `content` include_str! arguments.
                let abs = file
                    .abs_path
                    .to_str()
                    .unwrap_or_else(|| panic!("non-utf8 path: {}", file.abs_path.display()));
                out.push_str(&format!(
                    "    SkillFile {{ path: {rel:?}, content: include_str!({abs:?}), executable: {exe} }},\n",
                    rel = file.rel_path,
                    abs = abs,
                    exe = file.executable,
                ));
                // Watch each embedded file for content changes.
                println!("cargo:rerun-if-changed={}", file.abs_path.display());
            }
            out.push_str("];\n\n");
        }

        out.push_str("static ENTRIES: &[BundledSkill] = &[\n");
        for (idx, entry) in entries.iter().enumerate() {
            out.push_str(&format!(
                "    BundledSkill {{ name: {name:?}, files: SKILL_{idx}_ENTRY_FILES }},\n",
                name = entry.name,
            ));
        }
        out.push_str("];\n");
    }

    fs::write(&output_path, out)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", output_path.display()));
}

/// Recursively copy files from `src` to `dst`, skipping dotfiles and symlinks.
/// Emits `cargo:rerun-if-changed` for each copied file.
/// Returns the number of files copied.
fn copy_dir_recursive(src: &Path, dst: &Path) -> usize {
    let mut count = 0;
    let entries = match fs::read_dir(src) {
        Ok(entries) => entries,
        Err(e) => panic!("failed to read directory {}: {e}", src.display()),
    };

    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("failed to read entry in {}: {e}", src.display()));
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        // Skip dotfiles (.gitkeep, .DS_Store, etc.)
        if name.starts_with('.') {
            continue;
        }

        // Skip symlinks to prevent embedding unintended files (defense-in-depth)
        let file_type = entry
            .file_type()
            .unwrap_or_else(|e| panic!("cannot stat {}: {e}", entry.path().display()));
        if file_type.is_symlink() {
            println!(
                "cargo:warning=Skipping symlink in dashboard dist: {}",
                entry.path().display()
            );
            continue;
        }

        let src_path = entry.path();
        let dst_path = dst.join(&file_name);

        if file_type.is_dir() {
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
