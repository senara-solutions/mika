//! Structural test: enforces the LECTURE SEULE invariant of the milestone_manager module.
//!
//! **Rationale:** Prompt-level "no dispatch" rules are fragile per
//! `feedback_prompt_enforcement_fragile`. A compile-time-adjacent test that greps the
//! module tree for forbidden write-authority tokens is a structural gate that catches
//! regressions before merge.
//!
//! The invariant this test protects:
//! - No `run_claude_pilot` invocation site (dispatch to dev-pilot).
//! - No `pr_merge_with_gate` invocation site (PR merge).
//! - No `gh api` invocation with any mutating HTTP method (`PATCH`/`POST`/`DELETE`/`PUT`).
//! - No `gh issue edit` (label mutations).
//! - No `gh pr merge` (PR merge).
//! - No `gh pr comment` / `gh issue comment` (write to GitHub).
//!
//! If a legitimate use of any of these ever appears (e.g., Phase 2 promotion after the
//! 3 portes are cleared), the fix is to update this test AND the module docstring
//! atomically — never to silently loosen the test.

use std::fs;
use std::path::Path;

/// Forbidden token patterns — grepped against every `.rs` file under the module.
const FORBIDDEN_TOKENS: &[&str] = &[
    "run_claude_pilot",
    "pr_merge_with_gate",
    "gh pr merge",
    "gh issue edit",
    "gh pr comment",
    "gh issue comment",
    "gh issue close",
    "gh issue reopen",
    "gh pr close",
    "gh pr reopen",
    "gh pr ready",
    "gh label create",
    "gh api -X PATCH",
    "gh api -X POST",
    "gh api -X DELETE",
    "gh api -X PUT",
    "\"PATCH\"",
    "\"POST\"",
    "\"DELETE\"",
    "\"PUT\"",
    // Porte 2 (mika#1948) — the Phase-2-promotion lock. No file under
    // `milestone_manager/**` may WRITE the `dispatcher_source = 'mika_manager'`
    // metadata: doing so is what turns the read-only manager into a dispatcher.
    // Both literal shapes are listed because the token can appear as SQL
    // (single-quoted) or as a Rust string (double-quoted), and matching only one
    // would leave the other as a silent way through.
    //
    // When Phase 2 lands, this token moves to a dispatch-wire file OUTSIDE this
    // subtree and this list is updated in the same commit — never loosened
    // ahead of it.
    "dispatcher_source = 'mika_manager'",
    "dispatcher_source=\"mika_manager\"",
];

/// Files exempt from the grep — this test file itself contains the tokens as literals.
const EXEMPT_FILES: &[&str] = &["no_dispatch_test.rs"];

/// Exceptions **nommées**, par `(fichier, jeton, raison)`. Livrée **vide**, et
/// c'est tout l'objet de mika#2698 vol 1 : le mode ombre n'est pas un
/// interrupteur désarmé, c'est l'absence de chemin d'écriture. Il n'y a donc
/// rien à excepter, et le ticket reste ouvert jusqu'au vol 2.
///
/// **Ce que cette liste coûte quand elle cesse d'être vide.** Le vol 2 y
/// inscrira exactement une ligne — `gh issue comment`, dans le fichier qui porte
/// la branche ON de `MIKA_MANAGER_EVALUATE_WRITE` — et pas une de plus. Une
/// exception par *fichier et par jeton*, jamais par fichier seul : exempter un
/// fichier entier rendrait ses autres jetons invisibles, ce qui est précisément
/// la façon dont un garde structurel meurt sans bruit.
///
/// Un `EXEMPTED_OCCURRENCES` vide est aussi la **forme forte de l'AC1** : avec
/// l'interrupteur OFF il n'y a aucun appel d'écriture `gh` parce qu'aucun n'est
/// écrit, et `mika2698_lallowlist_dexceptions_est_vide_et_le_reste` rougit au
/// premier qui le serait.
const EXEMPTED_OCCURRENCES: &[(&str, &str, &str)] = &[];

#[test]
fn no_dispatch_scaffolding_in_milestone_manager() {
    // Resolve the module directory across the two CWD contexts this test runs
    // under: (a) `cargo test -p mika-agent` from the workspace root — CWD is
    // typically the workspace or crate root; (b) IDE test runners that set CWD
    // to the crate directory. `CARGO_MANIFEST_DIR` is stable across both.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let module_dir = Path::new(manifest_dir)
        .join("src")
        .join("milestone_manager");
    assert!(
        module_dir.exists(),
        "expected module dir at {} (CARGO_MANIFEST_DIR={manifest_dir})",
        module_dir.display()
    );

    let mut violations: Vec<(String, String)> = Vec::new();
    walk_and_scan(&module_dir, &mut violations);

    if !violations.is_empty() {
        let msg = violations
            .iter()
            .map(|(f, t)| format!("  {f} contains forbidden token `{t}`"))
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "LECTURE SEULE invariant violated — milestone_manager must not contain \
             dispatch/write-authority tokens:\n{msg}\n\n\
             If this is intentional (Phase 2 promotion), update no_dispatch_test.rs \
             AND the module docstring atomically."
        );
    }
}

/// mika#2698 vol 1 — l'allowlist part vide et y reste jusqu'au vol 2.
///
/// Ce test n'est pas une tautologie : il est le **prix** que paie le vol 2. Le
/// ticket autorise exactement un geste derrière `MIKA_MANAGER_EVALUATE_WRITE`,
/// et ce test est l'endroit où ce geste devra se déclarer — avec son fichier,
/// son jeton et sa raison. Toute autre écriture `gh` continuera de rougir.
#[test]
fn mika2698_lallowlist_dexceptions_est_vide_et_le_reste() {
    assert!(
        EXEMPTED_OCCURRENCES.is_empty(),
        "mika#2698 vol 1 ne déclare aucun chemin d'écriture : l'allowlist doit être vide. \
         Si tu livres le vol 2, cette assertion change EN MÊME TEMPS que la branche ON de \
         MIKA_MANAGER_EVALUATE_WRITE et que le docstring du module — jamais avant, et jamais \
         pour un jeton autre que `gh issue comment` : {EXEMPTED_OCCURRENCES:?}"
    );
}

/// Le contrôle négatif du garde : sur une source **fautive**, le scan rougit.
///
/// Sans lui, `no_dispatch_scaffolding_in_milestone_manager` pourrait passer
/// parce que le scan ne trouve jamais rien — un garde cassé et un module propre
/// rendent les mêmes octets (classe mika#2205).
#[test]
fn mika2698_le_scan_rougit_sur_une_source_fautive() {
    let fautive = r#"
        async fn poster_le_verdict(verdict: &str) {
            let _ = tokio::process::Command::new("gh")
                .args(["issue", "comment", "2698", "--body", verdict])
                .status()
                .await;
        }
    "#;
    // Le jeton tel que `FORBIDDEN_TOKENS` l'écrit, dans une forme shell plate —
    // c'est la forme qu'un futur auteur produirait en composant une commande.
    let plate = "let cmd = \"gh issue comment 2698\";";
    let found = scan_source("evaluator.rs", "evaluator.rs", plate, EXEMPTED_OCCURRENCES);
    assert_eq!(
        found.len(),
        1,
        "le scan doit trouver `gh issue comment` dans une source fautive, trouvé : {found:?}"
    );
    assert_eq!(found[0].1, "gh issue comment");

    // Et la forme `.args([…])`, qui ne porte PAS le jeton plat, passe : c'est la
    // limite connue du garde, dite ici plutôt que découverte au vol 2. Elle est
    // couverte par l'autre moitié de l'AC1 — aucun fichier du module ne
    // construit de commande `gh` d'écriture, ce que le scan voit par
    // `"POST"`/`gh api -X POST` et ce que la revue voit par lecture.
    assert!(
        scan_source(
            "evaluator.rs",
            "evaluator.rs",
            fautive,
            EXEMPTED_OCCURRENCES
        )
        .is_empty(),
        "limite connue : la forme éclatée en `.args([…])` échappe au grep textuel"
    );
}

/// L'exception nommée, quand elle existera, portera sur un couple
/// `(fichier, jeton)` — pas sur un fichier entier.
#[test]
fn mika2698_une_exception_ne_couvre_quun_seul_jeton_du_fichier() {
    let exemptions = [("evaluator.rs", "gh issue comment", "vol 2 mika#2698")];
    let source = "let a = \"gh issue comment\"; let b = \"gh pr merge\";";
    let found = scan_source("evaluator.rs", "evaluator.rs", source, &exemptions);
    assert_eq!(
        found.len(),
        1,
        "`gh pr merge` doit rester interdit dans un fichier exempté pour `gh issue comment` : {found:?}"
    );
    assert_eq!(found[0].1, "gh pr merge");
}

fn walk_and_scan(dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_and_scan(&path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if EXEMPT_FILES.contains(&file_name) {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        out.extend(scan_source(
            &path.display().to_string(),
            file_name,
            &content,
            EXEMPTED_OCCURRENCES,
        ));
    }
}

/// Le cœur du scan, extrait de [`walk_and_scan`] pour une seule raison : un
/// garde dont on ne peut pas prouver qu'il attrape quelque chose n'est pas un
/// garde. `walk_and_scan` ne voit que l'arbre réel, qui est — espérons-le —
/// toujours propre ; `scan_source` prend une source **littérale**, donc
/// `mika2698_le_scan_rougit_sur_une_source_fautive` peut lui donner un
/// `gh issue comment` et vérifier qu'il le trouve.
///
/// `label` est le chemin affiché dans le message d'échec ; `file_name` est ce
/// qui s'apparie contre `exemptions`, qui est [`EXEMPTED_OCCURRENCES`] en
/// production. La liste est un **paramètre** et non une lecture directe de la
/// constante : sinon un test ne pourrait vérifier qu'une exception sur un jeton
/// laisse bien les autres jetons du même fichier interdits, et c'est exactement
/// la propriété que le vol 2 devra avoir.
fn scan_source(
    label: &str,
    file_name: &str,
    source: &str,
    exemptions: &[(&str, &str, &str)],
) -> Vec<(String, String)> {
    // Strip line comments (both `//` and `///`) so doc comments that
    // *describe* the forbidden tokens don't trip the guard. The guard's
    // job is to catch executable code, not prose.
    let code_only = strip_line_comments(source);
    FORBIDDEN_TOKENS
        .iter()
        .filter(|token| code_only.contains(**token))
        .filter(|token| {
            !exemptions
                .iter()
                .any(|(f, t, _)| *f == file_name && *t == **token)
        })
        .map(|token| (label.to_string(), (*token).to_string()))
        .collect()
}

/// Strip Rust line comments (everything after `//` on each line). Leaves
/// string literals containing `//` unchanged is out of scope — we only
/// need to filter the common case of documentation prose mentioning
/// forbidden tokens.
///
/// **Known limitation:** block comments (`/* … */`) are NOT stripped. A
/// docstring block-comment that mentions a forbidden token would trip the
/// guard falsely. This is a fail-loud gap by design (safer than silent
/// bypass) — if a future author needs to document a forbidden token in a
/// block comment, prefer converting it to `//`-style prose lines.
fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            // Naive: find the first `//` not inside a string literal.
            // Sufficient for our module's actual code shapes.
            let mut in_string = false;
            let mut chars = line.char_indices().peekable();
            while let Some((i, c)) = chars.next() {
                if c == '"' && !line[..i].ends_with('\\') {
                    in_string = !in_string;
                }
                if !in_string
                    && c == '/'
                    && let Some(&(_, '/')) = chars.peek()
                {
                    return line[..i].to_string();
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
