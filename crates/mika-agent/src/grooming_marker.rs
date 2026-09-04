//! La seule lecture du marqueur de verdict de grooming dans le dépôt (mika#2158).
//!
//! # La décision, écrite ici pour que la prochaine divergence soit une régression
//!
//! **Ce module est la source de vérité du marqueur de verdict.** [`auto_pull::is_groomed`]
//! et [`skills::executor::check_grooming_markers`] l'appellent et n'en portent **pas** de
//! copie. Une regex qui lit `second-pass`, `first-pass`, `seconde passe` ou
//! `première passe` ailleurs que dans ce fichier fait échouer
//! [`tests::no_grooming_regex_outside_this_module`] — délibérément.
//!
//! Cette garde existe parce que la copie s'est déjà produite, avec son propre aveu :
//! `auto_pull.rs` portait une regex commentée *« Mirrors GROOMED_VERDICT_RE in
//! skills/executor.rs (#1725) »*. Elle a copié la forme de 2025 et n'a jamais suivi les deux
//! élargissements d'après (`PARAPHRASED_GROOMED_RE`, puis `SINGLE_PASS_GROOMED_RE` de
//! mika#2012). Les deux prédicats ont donc divergé pendant des mois, silencieusement, dans le
//! sens exact que mika#2158 décrit : un prédicat plus étroit que l'autre gouvernait la
//! promotion, l'autre gouvernait le routage, et rien ne cassait quand ils n'étaient pas
//! d'accord.
//!
//! # Le sens de l'alignement, tranché
//!
//! C'est le **prédicat** qui s'aligne sur la spec, pas l'inverse. `/mika-groom-ticket` phase 3
//! étape 10 prescrit littéralement de sauter à la phase 5 quand la première passe rend
//! `READY` : aucune seconde passe n'est écrite, et le prédicat punissait donc le cas où le
//! grooming s'était **bien** passé. La spec décrit un travail réel ; le prédicat décrivait une
//! phrase. Corollaire assumé : la spec n'a pas à imposer l'anglais pour être lisible par la
//! machine, dans un dépôt qui écrit ses tickets et ses plans en français.
//!
//! # Ce que ce module ne couvre pas
//!
//! Uniquement le **marqueur de verdict**. Les conditions `Branch` et `Plan` restent chez
//! leurs deux appelants, à leur place, et divergentes : `auto_pull` exige
//! `> - **Plan:** \`docs/plans/` là où `executor` se contente de `docs/plans/`. Unifier cette
//! moitié-là **est** le correctif de mika#2120, qui est sous arbitrage opérateur ; l'emporter
//! ici court-circuiterait cet arbitrage. Ce module est le tiroir où mika#2120 déposera sa
//! moitié quand son arbitrage sera rendu.
//!
//! # Le discriminateur a changé de nature
//!
//! Avant mika#2158 il était **lexical** : le mot `second-pass` devait précéder `GROOMED`.
//! Il est désormais **positionnel** : la ligne de callout `Grooming history` est le contexte,
//! et le **dernier** token de verdict qu'elle contient est l'état. Cet ancrage sur le préfixe
//! de ligne `^> - **Grooming history:**` est ce qui préserve la distinction callout/prose —
//! « le ticket a été GROOMED hier » écrit en corps de texte ne rend toujours rien.
//!
//! Deux tokens seulement sont des verdicts : `GROOMED` et `ESCALATE`. `READY` et `ITERATE`
//! sont des **dispositions de passe**, pas des verdicts finaux — à une exception près, la
//! règle AC1 documentée sur [`grooming_verdict`].

use std::sync::LazyLock;

use regex::Regex;

/// Le verdict de grooming lu dans le callout `Grooming history` d'un corps d'issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroomingVerdict {
    /// Le grooming a abouti : le ticket peut partir en implémentation.
    Groomed,
    /// La dernière disposition est un `ESCALATE` sans `GROOMED` postérieur — le ticket est
    /// dans les mains de l'opérateur, pas prêt.
    Escalated,
    /// Aucun verdict lisible : pas de callout, ou un callout dont aucune passe n'a abouti
    /// (`ITERATE` seul, une seconde passe annoncée dont le verdict est illisible).
    Absent,
}

/// La ligne de callout qui porte l'historique de grooming. L'ancrage sur le préfixe de ligne
/// est le discriminateur callout/prose ; il est la raison d'être de cette regex et ne doit pas
/// être relâché.
static CALLOUT_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^> - \*\*Grooming history:\*\*(.*)$")
        .expect("grooming history callout regex must compile")
});

/// Les deux seuls tokens de verdict. `\b` aux deux bouts est le discriminateur structurel qui
/// remplace l'ancienne classe de caractères : il rejette `GROOMEDLY` (pas de frontière après
/// `GROOMED`) comme `UNGROOMED` (pas de frontière avant), tout en acceptant `GROOMED)`,
/// `GROOMED,`, `GROOMED —`, `GROOMED.` et `GROOMED` en fin de ligne.
///
/// La casse compte : `GROOMED` est un token produit par le pipeline, « groomed » en prose
/// française ou anglaise n'en est pas un.
static VERDICT_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(GROOMED|ESCALATE[DS]?)\b").expect("verdict token regex must compile")
});

/// Une première passe dont la disposition est `READY`, dans les deux langues du dépôt.
static FIRST_PASS_READY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(first-pass|première passe|premiere passe)\s*\(\s*READY")
        .expect("first-pass READY regex must compile")
});

/// La marque qu'une passe ultérieure a eu lieu, dans les deux langues du dépôt. Sa présence
/// désarme la règle AC1 : voir [`grooming_verdict`].
static LATER_PASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(second-pass|seconde passe|deuxième passe|deuxieme passe)")
        .expect("later-pass marker regex must compile")
});

/// Concatène, dans l'ordre du document, le contenu de toutes les lignes de callout
/// `Grooming history` du corps.
///
/// Il y en a normalement une. Il peut y en avoir plusieurs : `dispatch-lib` documente le cas
/// où un re-grooming « empile un second callout » (mika#2012). Les traiter comme un seul
/// texte, dans l'ordre, fait tomber la règle du dernier token sur le callout le plus récent —
/// ce qui est l'état courant — sans avoir à choisir arbitrairement une ligne.
fn callout_text(issue_body: &str) -> Option<String> {
    let mut parts = CALLOUT_LINE_RE
        .captures_iter(issue_body)
        .map(|c| c[1].to_string())
        .peekable();
    parts.peek()?;
    Some(parts.collect::<Vec<_>>().join("\n"))
}

/// Le verdict de grooming porté par le corps d'issue.
///
/// # La règle
///
/// 1. Pas de ligne de callout `Grooming history` → [`GroomingVerdict::Absent`].
/// 2. Le **dernier** token de verdict de ce callout fait foi :
///    `… (ESCALATE …) → … (GROOMED …)` rend `Groomed`, et
///    `… (GROOMED) → … (ESCALATE)` rend `Escalated`. L'ordre compte dans les deux sens.
/// 3. Aucun token de verdict, mais une première passe `READY` **et aucune marque de passe
///    ultérieure** → `Groomed`. C'est la règle AC1, et le seul cas où une disposition vaut
///    verdict : `/mika-groom-ticket` phase 3 étape 10 prescrit de sauter à la phase 5 sans
///    seconde passe quand le plan est sain du premier coup, donc le corps ne peut pas porter
///    de verdict de seconde passe — en exiger un revient à punir le grooming qui s'est bien
///    passé.
/// 4. Tout le reste → `Absent`.
///
/// # Pourquoi AC1 est désarmée par une marque de passe ultérieure
///
/// La condition « aucune marque de passe ultérieure » n'est pas une précaution décorative :
/// elle est ce qui distingue *« la seconde passe n'a pas eu lieu parce qu'elle était
/// inutile »* (le chemin prescrit, groomé) de *« la seconde passe a eu lieu et son verdict est
/// illisible »* (une forme cassée, pas groomée). Sans elle, un corps portant
/// `first-pass (READY) → second-pass (GROOMEDLY)` serait déclaré groomé sur la foi de sa
/// première passe, alors qu'il annonce lui-même une seconde passe dont le verdict n'en est
/// pas un.
///
/// `ITERATE` seul reste `Absent` pour la raison symétrique : une première passe qui itère
/// **prescrit** une seconde passe, qui n'a pas eu lieu.
pub fn grooming_verdict(issue_body: &str) -> GroomingVerdict {
    let Some(text) = callout_text(issue_body) else {
        return GroomingVerdict::Absent;
    };

    if let Some(last) = VERDICT_TOKEN_RE.find_iter(&text).last() {
        return if last.as_str().starts_with("GROOMED") {
            GroomingVerdict::Groomed
        } else {
            GroomingVerdict::Escalated
        };
    }

    if FIRST_PASS_READY_RE.is_match(&text) && !LATER_PASS_RE.is_match(&text) {
        return GroomingVerdict::Groomed;
    }

    GroomingVerdict::Absent
}

/// Sucre : le corps porte-t-il un grooming abouti ?
///
/// C'est ce que les deux appelants Rust utilisent ; ils y ajoutent leurs propres conditions
/// `Branch`/`Plan`, qui ne sont pas du ressort de ce module (voir l'en-tête).
pub fn has_groomed_verdict(issue_body: &str) -> bool {
    matches!(grooming_verdict(issue_body), GroomingVerdict::Groomed)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Un corps minimal portant les trois callouts, dont `Grooming history` = `history`.
    fn body_with(history: &str) -> String {
        format!(
            "## Description\n\n\
             > - **Branch:** `fix/123/some-feature`\n\
             > - **Plan:** `docs/plans/2026-06-01-001-some-plan.md` (committed on branch @ abc1234)\n\
             > - **Grooming history:** {history}\n"
        )
    }

    // ── M2 — reconnaissance de l'état (AC1, AC2, AC3) ──

    #[test]
    fn canonical_second_pass_groomed() {
        assert_eq!(
            grooming_verdict(&body_with("first-pass (READY) → second-pass (GROOMED)")),
            GroomingVerdict::Groomed
        );
    }

    #[test]
    fn ac1_first_pass_ready_without_second_pass_is_groomed() {
        // `/mika-groom-ticket` phase 3 étape 10 : plan sain du premier coup, saut en phase 5.
        assert_eq!(
            grooming_verdict(&body_with(
                "mika-arch first-pass (READY) → aucune révision requise"
            )),
            GroomingVerdict::Groomed
        );
    }

    #[test]
    fn ac2_french_second_pass_is_groomed() {
        assert_eq!(
            grooming_verdict(&body_with(
                "mika-arch première passe (ITERATE) → révision → mika-arch seconde passe (GROOMED, session abc)"
            )),
            GroomingVerdict::Groomed
        );
    }

    #[test]
    fn ac1_french_first_pass_ready_without_second_pass_is_groomed() {
        assert_eq!(
            grooming_verdict(&body_with(
                "mika-arch première passe (READY) — rien à redire"
            )),
            GroomingVerdict::Groomed
        );
    }

    #[test]
    fn ac3_groomed_after_escalate_and_arbitration_is_groomed() {
        assert_eq!(
            grooming_verdict(&body_with(
                "mika-arch second-pass (ESCALATE, périmètre) → arbitrage rendu → mika-arch (GROOMED) — session abc"
            )),
            GroomingVerdict::Groomed
        );
    }

    #[test]
    fn verdict_is_read_from_any_producer_not_only_second_pass() {
        // Le producteur qui précède `GROOMED` ne décide plus rien — c'est le point de M2.
        assert_eq!(
            grooming_verdict(&body_with("mika-arch (GROOMED) — ratification hors passe")),
            GroomingVerdict::Groomed
        );
    }

    // ── Formes héritées : les trois regex absorbées de `executor.rs` ──

    #[test]
    fn legacy_parameterized_and_annotated_groomed_forms_still_match() {
        for history in [
            "first-pass (READY) → second-pass (GROOMED, session fd4c1a14)",
            "first-pass (READY) → second-pass (GROOMED — session-id: 550e8400)",
            "first-pass (READY) → second-pass (GROOMED. Full ratification.)",
            // #1725 — `PARAPHRASED_GROOMED_RE`
            "first-pass (READY) → second-pass (READY, paraphrased GROOMED)",
            // mika#2012 — `SINGLE_PASS_GROOMED_RE`
            "first-pass (READY, single-pass GROOMED)",
        ] {
            assert_eq!(
                grooming_verdict(&body_with(history)),
                GroomingVerdict::Groomed,
                "forme héritée non reconnue: {history}"
            );
        }
    }

    // ── M3 — non-régressions (AC4) ──

    #[test]
    fn empty_body_is_absent() {
        assert_eq!(grooming_verdict(""), GroomingVerdict::Absent);
    }

    #[test]
    fn body_without_grooming_history_callout_is_absent() {
        let body = "## Description\n\n> - **Branch:** `fix/123/x`\n";
        assert_eq!(grooming_verdict(body), GroomingVerdict::Absent);
    }

    #[test]
    fn escalate_without_later_groomed_is_escalated() {
        assert_eq!(
            grooming_verdict(&body_with("mika-arch second-pass (ESCALATE, périmètre)")),
            GroomingVerdict::Escalated
        );
        assert!(!has_groomed_verdict(&body_with(
            "mika-arch second-pass (ESCALATE, périmètre)"
        )));
    }

    #[test]
    fn groomed_then_escalate_is_escalated_order_counts_both_ways() {
        assert_eq!(
            grooming_verdict(&body_with(
                "second-pass (GROOMED) → revue de périmètre → mika-arch (ESCALATE)"
            )),
            GroomingVerdict::Escalated
        );
    }

    #[test]
    fn iterate_alone_is_absent() {
        // Une première passe qui itère prescrit une seconde passe, qui n'a pas eu lieu.
        assert_eq!(
            grooming_verdict(&body_with("mika-arch first-pass (ITERATE)")),
            GroomingVerdict::Absent
        );
        assert_eq!(
            grooming_verdict(&body_with("mika-arch première passe (ITERATE)")),
            GroomingVerdict::Absent
        );
    }

    #[test]
    fn prose_groomed_outside_the_callout_is_absent() {
        let body = "## Description\n\n\
                    Le ticket a été GROOMED hier et il est prêt.\n\n\
                    > - **Branch:** `fix/123/x`\n\
                    > - **Plan:** `docs/plans/p.md`\n";
        assert_eq!(grooming_verdict(body), GroomingVerdict::Absent);
    }

    #[test]
    fn word_continuation_after_groomed_is_not_a_verdict() {
        // `GROOMEDLY` n'est pas un verdict — et la marque de seconde passe désarme AC1, donc
        // la première passe READY ne rattrape pas la forme cassée.
        assert_eq!(
            grooming_verdict(&body_with("first-pass (READY) → second-pass (GROOMEDLY)")),
            GroomingVerdict::Absent
        );
    }

    #[test]
    fn ungroomed_is_not_a_verdict() {
        assert_eq!(
            grooming_verdict(&body_with("second-pass (UNGROOMED)")),
            GroomingVerdict::Absent
        );
    }

    #[test]
    fn stacked_callouts_read_in_document_order() {
        // Deux callouts empilés (mika#2012) : le plus récent fait foi.
        let body = "## Description\n\n\
                    > - **Grooming history:** second-pass (GROOMED)\n\
                    > - **Grooming history:** second-pass (ESCALATE, périmètre rouvert)\n";
        assert_eq!(grooming_verdict(body), GroomingVerdict::Escalated);
    }

    // ── M4 — les six corps réels (AC5) ──

    /// Les six fixtures, avec l'état attendu **après** correctif.
    ///
    /// Provenance et interdiction de rafraîchissement :
    /// `crates/mika-agent/tests/fixtures/grooming_bodies/README.md`.
    pub(crate) const FIXTURES: &[(&str, &str, bool)] = &[
        (
            "2127",
            include_str!("../tests/fixtures/grooming_bodies/2127.md"),
            true,
        ),
        (
            "2140",
            include_str!("../tests/fixtures/grooming_bodies/2140.md"),
            true,
        ),
        (
            "2108",
            include_str!("../tests/fixtures/grooming_bodies/2108.md"),
            true,
        ),
        (
            "1772",
            include_str!("../tests/fixtures/grooming_bodies/1772.md"),
            true,
        ),
        (
            "2151",
            include_str!("../tests/fixtures/grooming_bodies/2151.md"),
            true,
        ),
        (
            "2117",
            include_str!("../tests/fixtures/grooming_bodies/2117.md"),
            true,
        ),
    ];

    #[test]
    fn fixture_table() {
        for (ticket, body, expected) in FIXTURES {
            assert_eq!(
                has_groomed_verdict(body),
                *expected,
                "fixture #{ticket}: attendu groomed={expected}, verdict={:?}",
                grooming_verdict(body)
            );
        }
        assert_eq!(
            FIXTURES.len(),
            6,
            "les six corps mesurés doivent être figés"
        );
    }

    // ── M5a — les prédicats s'accordent (AC7) ──

    /// Sur les six corps figés, `auto_pull::is_groomed` et
    /// `executor::check_grooming_markers(..).is_empty()` doivent rendre le **même** verdict.
    /// Un désaccord fait échouer la suite — c'est le point de l'AC7.
    ///
    /// # Réserve honnête, à lire avant de conclure que la divergence est fermée
    ///
    /// Ce test est vert **sans** que la divergence mika#2120 soit close. Les deux prédicats
    /// diffèrent encore sur la condition `Plan` : `auto_pull` exige
    /// `> - **Plan:** \`docs/plans/` là où `executor` se contente de `docs/plans/`. Aucune
    /// des six fixtures ne porte de callout `Plan` préfixé par le dépôt — les six écrivent
    /// la forme nue — donc le croisement passe sur ce jeu-là et sur lui seul.
    ///
    /// mika#2120 est la moitié restante. Prétendre que ce test l'atteste serait
    /// exactement l'attestation-produite-à-côté-de-ce-qu'elle-atteste que mika#2034 a déjà
    /// corrigée ailleurs.
    #[test]
    fn ac7_both_rust_predicates_agree_on_the_frozen_bodies() {
        for (ticket, body, _) in FIXTURES {
            let p1 = crate::auto_pull::is_groomed(body);
            let p2 = crate::skills::executor::check_grooming_markers(body).is_empty();
            assert_eq!(
                p1, p2,
                "fixture #{ticket}: désaccord entre les prédicats — \
                 auto_pull::is_groomed={p1}, executor::check_grooming_markers().is_empty()={p2}"
            );
        }
    }

    /// Le croisement doit rester capable de **détecter** un désaccord, sinon il n'atteste
    /// rien. Un corps portant la forme `Plan` préfixée par le dépôt sépare les deux
    /// prédicats aujourd'hui : c'est exactement la divergence mika#2120, et ce test la
    /// fixe en l'état plutôt que de la laisser invisible.
    ///
    /// **Quand mika#2120 sera rendu, ce test échouera** — c'est voulu : il devra être
    /// supprimé dans le même commit, et `ac7_both_rust_predicates_agree_on_the_frozen_bodies`
    /// deviendra un accord inconditionnel.
    #[test]
    fn mika2120_divergence_is_still_open_and_this_test_pins_it() {
        let repo_prefixed = "## Description\n\n\
             > - **Branch:** `fix/2120/x`\n\
             > - **Plan:** `mika/docs/plans/2026-09-03-001-fix-2120-x-plan.md` (committed @ abc)\n\
             > - **Grooming history:** mika-arch second-pass (GROOMED)\n";

        assert!(
            !crate::auto_pull::is_groomed(repo_prefixed),
            "auto_pull exige le préfixe `docs/plans/` collé au backtick"
        );
        assert!(
            crate::skills::executor::check_grooming_markers(repo_prefixed).is_empty(),
            "executor accepte la forme préfixée par le dépôt"
        );
    }

    // ── M1 — la garde structurelle contre la récidive (AC6) ──

    /// Échoue si une regex portant un marqueur de passe apparaît ailleurs que dans ce module.
    ///
    /// # Pourquoi cette forme
    ///
    /// Le plan laissait le choix entre `include_str!` sur les fichiers voisins (couplage de
    /// compilation : toute la crate se recompile quand `auto_pull.rs` bouge, et un fichier
    /// renommé casse la compilation au lieu de faire échouer un test) et un test
    /// d'intégration lisant `src/` (plus lâche, mais dépendant du répertoire courant).
    ///
    /// `env!("CARGO_MANIFEST_DIR")` lève les deux objections d'un coup : la lecture est faite à
    /// l'exécution (aucun couplage de compilation) depuis un chemin absolu que Cargo garantit
    /// (aucune dépendance au `cwd`). C'est la réponse à l'arbitrage laissé ouvert au §8 du plan.
    ///
    /// # Ce qu'elle détecte
    ///
    /// Une ligne qui construit une `Regex` **et** contient un marqueur de passe. C'est
    /// exactement la forme que ce ticket ferme : une regex de reconnaissance de verdict
    /// recopiée hors de ce module. Une prose de test ou de prompt contenant `first-pass` n'est
    /// pas touchée — elle ne construit pas de regex.
    #[test]
    fn no_grooming_regex_outside_this_module() {
        const PASS_MARKERS: &[&str] = &[
            "second-pass",
            "first-pass",
            "seconde passe",
            "première passe",
        ];

        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_module = src_root.join("grooming_marker.rs");

        let mut offenders = Vec::new();
        let mut stack = vec![src_root.clone()];
        let mut scanned = 0usize;

        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("la garde doit pouvoir lire {}: {e}", dir.display()));
            for entry in entries {
                let path = entry.expect("entrée de répertoire lisible").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") || path == this_module {
                    continue;
                }
                let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                    panic!("la garde doit pouvoir lire {}: {e}", path.display())
                });
                scanned += 1;
                for (n, line) in content.lines().enumerate() {
                    if line.contains("Regex::new") && PASS_MARKERS.iter().any(|m| line.contains(m))
                    {
                        offenders.push(format!(
                            "{}:{}: {}",
                            path.strip_prefix(&src_root).unwrap_or(&path).display(),
                            n + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }

        assert!(
            scanned > 0,
            "la garde n'a scanné aucun fichier — chemin cassé"
        );
        assert!(
            offenders.is_empty(),
            "mika#2158 — une regex de marqueur de grooming vit hors de `grooming_marker.rs`. \
             C'est la copie-avec-commentaire-« Mirrors » qui a fait diverger les deux prédicats \
             pendant des mois. Appelez `grooming_marker::has_groomed_verdict` au lieu de \
             recopier la regex.\n{}",
            offenders.join("\n")
        );
    }
}
