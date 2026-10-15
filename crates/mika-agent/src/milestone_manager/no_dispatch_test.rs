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
//! - No `gh pr comment` / `gh issue comment` (write to GitHub) — **une seule
//!   exception nommée**, voir `EXEMPTED_OCCURRENCES`.
//!
//! If a legitimate use of any of these ever appears (e.g., Phase 2 promotion after the
//! 3 portes are cleared), the fix is to update this test AND the module docstring
//! atomically — never to silently loosen the test.
//!
//! **L'exception du vol 2 de mika#2698 est le cas d'école de cette règle.**
//! `verdict_write.rs` porte `gh issue comment` derrière
//! `MIKA_MANAGER_EVALUATE_WRITE`, et sa déclaration dans `EXEMPTED_OCCURRENCES`
//! est arrivée dans le même commit que le geste, le docstring du module et les
//! tests des deux branches de l'interrupteur. Une exception par couple
//! *(fichier, jeton)* : tous les autres jetons rougissent dans ce fichier aussi.

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

/// Exceptions **nommées**, par `(fichier, jeton, raison)`.
///
/// **Exactement une ligne, et c'est le prix du vol 2 de mika#2698.** Le vol 1
/// livrait cette liste vide — le mode ombre n'était pas un interrupteur
/// désarmé, c'était l'absence de chemin d'écriture. Le vol 2 ouvre un geste, un
/// seul, derrière `MIKA_MANAGER_EVALUATE_WRITE` : `gh issue comment`, dans
/// `verdict_write.rs`, le fichier qui porte la branche ON. La ligne ci-dessous
/// est cette déclaration.
///
/// **Ce que l'exception ne couvre pas, et c'est l'essentiel.** Elle porte sur
/// une paire *(fichier, jeton)*, jamais sur un fichier seul : `gh issue edit`,
/// `gh pr comment`, `gh pr merge` continuent de rougir **y compris dans
/// `verdict_write.rs`**. Exempter un fichier entier rendrait ses autres jetons
/// invisibles, ce qui est précisément la façon dont un garde structurel meurt
/// sans bruit ; `mika2698_une_exception_ne_couvre_quun_seul_jeton_du_fichier`
/// en est la preuve exécutable.
///
/// Et elle n'est **pas** la forme principale de l'AC1 — l'interrupteur l'est.
/// Cette liste rend le geste *visible* ; c'est `MIKA_MANAGER_EVALUATE_WRITE`,
/// OFF par défaut, qui le rend *inatteignable*. Un futur auteur qui écrirait le
/// geste en `.args(["issue", "comment"])` passerait ce scan sans aucune
/// exception : `verdict_write.rs` écrit donc son jeton en clair, à dessein, et
/// dérive l'argv par découpage (voir son docstring). C'est la raison d'être de
/// cette ligne.
const EXEMPTED_OCCURRENCES: &[(&str, &str, &str)] = &[(
    "verdict_write.rs",
    "gh issue comment",
    "mika#2698 vol 2 — le seul geste d'écriture autorisé du manager, derrière \
     MIKA_MANAGER_EVALUATE_WRITE (OFF par défaut). Jeton écrit en clair et argv \
     dérivée par découpage, pour que ce scan le voie et que cette exception soit \
     nécessaire.",
)];

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

/// mika#2698 vol 2 — l'allowlist porte **une** ligne, et la bonne.
///
/// Le vol 1 pinnait cette liste vide ; le vol 2 l'ouvre d'un cran et pinne ce
/// cran exactement. Ce n'est pas une tautologie : c'est le seul endroit où un
/// second geste d'écriture — ou le même geste déplacé dans un autre fichier du
/// module — doit se déclarer pour exister. Le test nomme les trois champs, donc
/// élargir le jeton ou le fichier le fait rougir, y compris quand le code
/// compile et que tout le reste est vert.
#[test]
fn mika2698_lallowlist_declare_un_seul_geste_dans_un_seul_fichier() {
    assert_eq!(
        EXEMPTED_OCCURRENCES.len(),
        1,
        "mika#2698 n'autorise qu'un geste d'écriture. Une seconde entrée est une décision \
         de ticket, pas un détail d'implémentation : {EXEMPTED_OCCURRENCES:?}"
    );
    let (file, token, reason) = EXEMPTED_OCCURRENCES[0];
    assert_eq!(
        file, "verdict_write.rs",
        "le geste vit dans le fichier qui porte la branche ON de MIKA_MANAGER_EVALUATE_WRITE, \
         nulle part ailleurs dans le module"
    );
    assert_eq!(
        token, "gh issue comment",
        "le ticket autorise `gh issue comment` et rien d'autre — ni `gh issue edit`, \
         ni `gh pr comment`, ni `gh pr merge`"
    );
    assert!(
        reason.contains("mika#2698"),
        "une exception sans ticket est une exception que personne ne peut retirer : {reason}"
    );
}

/// L'autre moitié de l'AC1 : le reste du module, lui, reste vide d'exception.
///
/// Sans cette assertion, exempter `evaluator.rs` ou `cadence.rs` pour le même
/// jeton serait un ajout d'une ligne dans une liste qui en porte déjà une — le
/// genre de modification qui se lit comme un entretien. Le geste est **unique**
/// parce qu'il n'a qu'un seul domicile possible.
#[test]
fn mika2698_aucun_autre_fichier_du_module_nest_excepte() {
    let autres: Vec<&str> = EXEMPTED_OCCURRENCES
        .iter()
        .map(|(f, _, _)| *f)
        .filter(|f| *f != "verdict_write.rs")
        .collect();
    assert!(
        autres.is_empty(),
        "seul verdict_write.rs porte une autorité d'écriture ; ces fichiers n'en ont jamais \
         eu et ne doivent pas en gagner par allowlist : {autres:?}"
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

/// L'exception nommée porte sur un couple `(fichier, jeton)` — pas sur un
/// fichier entier. Vérifié sur une source synthétique.
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

/// Et vérifié sur **l'exception réellement livrée**, avec son vrai nom de
/// fichier et `EXEMPTED_OCCURRENCES` tel qu'il est en production.
///
/// Le test ci-dessus prouve que le mécanisme est par couple ; celui-ci prouve
/// que la ligne du vol 2 l'utilise ainsi. Les deux sont nécessaires : une
/// exception correcte dans un mécanisme juste reste fausse si on l'avait écrite
/// par fichier, et ce scan-là est le seul qui le voie.
#[test]
fn mika2698_lexception_de_verdict_write_nouvre_que_son_jeton() {
    // Le geste autorisé passe.
    assert!(
        scan_source(
            "verdict_write.rs",
            "verdict_write.rs",
            "const WRITE_GESTURE: &str = \"gh issue comment\";",
            EXEMPTED_OCCURRENCES
        )
        .is_empty(),
        "l'exception nommée doit laisser passer le geste qu'elle nomme"
    );

    // Un second geste, dans le MÊME fichier, ne passe pas.
    for interdit in ["gh issue edit", "gh pr comment", "gh pr merge"] {
        let source = format!("let cmd = \"{interdit} 2698\";");
        let found = scan_source(
            "verdict_write.rs",
            "verdict_write.rs",
            &source,
            EXEMPTED_OCCURRENCES,
        );
        assert_eq!(
            found.len(),
            1,
            "`{interdit}` doit rougir dans verdict_write.rs : l'exception ne couvre \
             que `gh issue comment`, trouvé : {found:?}"
        );
        assert_eq!(found[0].1, interdit);
    }
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
