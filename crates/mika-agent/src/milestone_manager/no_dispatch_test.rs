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
//! - No `message/send` (the A2A engine door: sending an instruction to another
//!   agent is a write, even though no GitHub call appears in it).
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
    // mika#2958 — la porte moteur. `message/send` est la méthode A2A par
    // laquelle un agent fait agir un autre agent : aucune API GitHub n'y
    // apparaît, et c'est pourtant une écriture. Le jeton entre ici **avec** son
    // exception nommée ci-dessous, et il entre sur une mesure : avant ce
    // ticket, aucun fichier du module ne le portait, donc le scan se resserre
    // sans amnistier une seule occurrence préexistante.
    "message/send",
];

/// Files exempt from the grep — this test file itself contains the tokens as literals.
const EXEMPT_FILES: &[&str] = &["no_dispatch_test.rs"];

/// Exceptions **nommées**, par `(fichier, jeton, raison)`. Trois lignes : le
/// vol 2 de mika#2698 a inscrit la première, le vol 4 de mika#2699 la seconde,
/// mika#2958 la troisième. Une par geste d'écriture que la Phase 2 du manager a
/// reçu par un ticket, et aucune autre — un quatrième geste est un quatrième
/// ticket, avec son propre seuil d'ouverture.
///
/// Une exception par *fichier et par jeton*, jamais par fichier seul : exempter
/// un fichier entier rendrait ses autres jetons invisibles, ce qui est
/// précisément la façon dont un garde structurel meurt sans bruit. La propriété
/// est tenue par `mika2698_une_exception_ne_couvre_quun_seul_jeton_du_fichier`
/// — `verdict_poster.rs` reste donc interdit sur `gh pr merge`, `gh issue edit`
/// et les dix-neuf autres, `pool_writer.rs` l'est sur `gh issue comment`, et le
/// fichier du geste de groom l'est sur les vingt-deux jetons qui ne sont pas le
/// sien.
///
/// **Ce que chaque exception coûte, et ce qui le paie.** Le grep ne protège plus
/// le couple nommé, donc l'AC1 du ticket concerné ne repose plus sur lui seul.
/// Trois choses le portent ensemble, **dans le fichier exempté** : l'épinglage
/// ci-dessous (au fichier et au jeton près), la table de vérité de
/// l'interrupteur, et le test de comportement `armed == false` ⇒ zéro appel —
/// avec son contrôle négatif `armed == true` ⇒ exactement un appel, sans lequel
/// « zéro appel » serait aussi satisfait par un poseur mort.
const EXEMPTED_OCCURRENCES: &[(&str, &str, &str)] = &[
    (
        "verdict_poster.rs",
        "gh issue comment",
        "mika#2698 vol 2 — le seul geste d'écriture autorisé, derrière MIKA_MANAGER_EVALUATE_WRITE",
    ),
    (
        "pool_writer.rs",
        "gh issue edit",
        "mika#2699 vol 4 — pose `ready` + rattache au milestone cible en un geste groupé, derrière MIKA_MANAGER_POOL_WRITE",
    ),
    (
        "groom_trigger.rs",
        "message/send",
        "mika#2958 — déclenche le groom de la relève par la voie moteur (A2A vers mika-dev, jamais un spawn), derrière MIKA_MANAGER_GROOM_WRITE, OFF par défaut",
    ),
];

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

/// mika#2958 — l'allowlist porte **les trois** gestes, et pas un de plus.
///
/// Ce test n'est pas une tautologie : il est le **prix** des gestes d'écriture.
/// Chacun des trois tickets autorise exactement un geste derrière son propre
/// interrupteur, et c'est ici qu'ils se déclarent — avec leur fichier, leur
/// jeton et leur raison. Le vol 1 de mika#2698 exigeait que cette assertion
/// change « EN MÊME TEMPS que la branche ON de l'interrupteur et que le
/// docstring du module » : elle l'a fait trois fois, dans le commit qui a créé
/// `verdict_poster.rs`, dans celui qui a créé `pool_writer.rs`, puis dans celui
/// qui a créé `groom_trigger.rs`.
///
/// Ce qui rougit : une quatrième entrée, un autre fichier, un autre jeton. Aucun
/// des trois ne se justifie par ces trois tickets — un geste d'écriture de plus
/// est un ticket de plus, avec son propre seuil d'ouverture. La Phase 2 du
/// manager a reçu trois gestes d'écriture, un par vol, et c'est tout ce qu'elle
/// a reçu.
///
/// **Et ce qui rougit aussi, le jour où une exception devient inutile.** La
/// dernière boucle vérifie que chaque couple exempté existe *encore* : le
/// fichier est là, et sa source hors commentaires contient *vraiment* le jeton.
/// Une exception qu'un refactor a vidée de son geste cesserait autrement d'être
/// une décision pour devenir un trou permanent dans le grep, sans que rien ne le
/// dise (motif mika#2309 : une dérogation doit expirer par le geste même qui la
/// rend inutile). Le scan est fait sur la source **privée de commentaires**,
/// parce qu'une prose qui cite le jeton n'est pas un geste (mika#2050, signal S).
#[test]
fn mika2958_lallowlist_porte_les_trois_gestes_et_pas_un_de_plus() {
    assert_eq!(
        EXEMPTED_OCCURRENCES.len(),
        3,
        "la Phase 2 du manager n'a reçu que trois gestes d'écriture, un par vol : l'allowlist \
         doit porter exactement trois entrées. Une écriture de plus est un autre ticket, avec \
         son propre seuil d'ouverture : {EXEMPTED_OCCURRENCES:?}"
    );

    let (fichier, jeton, raison) = EXEMPTED_OCCURRENCES[0];
    assert_eq!(
        fichier, "verdict_poster.rs",
        "la première exception nomme le fichier qui porte la branche ON de l'interrupteur \
         d'évaluation, et lui seul"
    );
    assert_eq!(
        jeton, "gh issue comment",
        "le seul jeton que mika#2698 autorise — ni `gh issue edit`, ni `gh pr comment`"
    );
    assert!(
        raison.contains("mika#2698") && raison.contains("MIKA_MANAGER_EVALUATE_WRITE"),
        "la raison doit nommer le ticket et l'interrupteur : {raison}"
    );

    let (fichier, jeton, raison) = EXEMPTED_OCCURRENCES[1];
    assert_eq!(
        fichier, "pool_writer.rs",
        "la seconde exception nomme le fichier qui porte la branche ON de l'interrupteur du \
         bassin, et lui seul"
    );
    assert_eq!(
        jeton, "gh issue edit",
        "le seul jeton que mika#2699 autorise — le geste groupé, ni `gh api -X PATCH`, ni \
         `gh issue comment`"
    );
    assert!(
        raison.contains("mika#2699") && raison.contains("MIKA_MANAGER_POOL_WRITE"),
        "la raison doit nommer le ticket et l'interrupteur : {raison}"
    );

    let (fichier, jeton, raison) = EXEMPTED_OCCURRENCES[2];
    assert_eq!(
        fichier, "groom_trigger.rs",
        "la troisième exception nomme le fichier qui porte la branche ON de l'interrupteur du \
         geste de groom, et lui seul"
    );
    assert_eq!(
        jeton, "message/send",
        "le seul jeton que mika#2958 autorise — la porte moteur, ni un spawn, ni un dispatch \
         d'implémentation"
    );
    assert!(
        raison.contains("mika#2958") && raison.contains("MIKA_MANAGER_GROOM_WRITE"),
        "la raison doit nommer le ticket et l'interrupteur : {raison}"
    );

    // Auto-nettoyage : une exception dont le geste a disparu doit rougir.
    let module_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("milestone_manager");
    for (fichier, jeton, _) in EXEMPTED_OCCURRENCES {
        let chemin = module_dir.join(fichier);
        let source = fs::read_to_string(&chemin).unwrap_or_else(|e| {
            panic!(
                "l'exception nomme {fichier}, que ce module ne porte pas (ou plus) : {e}. \
                 Une dérogation sans site est un trou permanent dans le grep — retirez-la."
            )
        });
        let production = source
            .split_once("#[cfg(test)]")
            .map_or(source.as_str(), |(avant, _)| avant);
        assert!(
            strip_line_comments(production).contains(jeton),
            "l'exception `({fichier}, {jeton})` ne couvre plus aucun geste : la production de \
             {fichier} ne contient pas `{jeton}` hors commentaires. Le geste a été retiré ou \
             réécrit — retirez l'exception dans le même commit (motif mika#2309)."
        );
    }
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

    // mika#2958 — le jeton de la porte moteur, même traitement. Dans un fichier
    // qui n'est pas l'exception nommée, il rougit ; et c'est bien l'exception
    // qui le laisse passer dans le fichier du geste, pas un trou dans le scan.
    let moteur = "let method = \"message/send\";";
    let found = scan_source("cadence.rs", "cadence.rs", moteur, EXEMPTED_OCCURRENCES);
    assert_eq!(
        found.len(),
        1,
        "le scan doit trouver `message/send` dans une source fautive, trouvé : {found:?}"
    );
    assert_eq!(found[0].1, "message/send");
    assert!(
        scan_source(
            "groom_trigger.rs",
            "groom_trigger.rs",
            moteur,
            EXEMPTED_OCCURRENCES
        )
        .is_empty(),
        "l'exception nommée couvre ce couple `(fichier, jeton)` — et seulement lui"
    );
    assert_eq!(
        scan_source(
            "groom_trigger.rs",
            "groom_trigger.rs",
            "let cmd = \"gh issue comment 2958\";",
            EXEMPTED_OCCURRENCES
        )
        .len(),
        1,
        "l'exception ne couvre qu'un jeton : le fichier du geste reste interdit sur les autres"
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
