//! `mika plan-callout --body-file <path> [--raw]` (mika#2194 R2).
//!
//! Le canal par lequel `dispatch-lib.sh` interroge le lecteur unique du callout
//! `Plan` ([`mika_agent::plan_callout`]) au lieu d'en porter une seconde
//! implémentation en PCRE.
//!
//! # Le corps passe par un FICHIER, jamais par un argument
//!
//! Un corps d'issue porte des retours à la ligne, des backticks et des `$` : le
//! passer en argv ré-introduirait la classe de panne de portée de guillemets
//! **dans le geste même qui prétend la fermer** (c'est l'une des trois classes
//! que mika#2194 cite — cpp#157). Le refus est **structurel** : il n'y a pas de
//! variante positionnelle à cette sous-commande, donc il n'y a rien à
//! contourner.
//!
//! # Trois codes de sortie, et le troisième est le livrable
//!
//! | code | stdout | sens |
//! |---|---|---|
//! | `0` | le chemin, une ligne | un callout a été lu |
//! | `1` | vide | **aucun callout** — la réponse est « non » |
//! | `≥2` | vide, diagnostic sur stderr | **je n'ai pas pu regarder** |
//!
//! Aujourd'hui `_extract_plan_path` rend `1` dans les deux derniers cas, et
//! `_detect_plan_on_branch` fait `|| return 0` — donc une erreur de lecture est
//! lue comme « pas de plan ». La population « fichier illisible » est **créée
//! par la migration** (avant elle, le corps était en variable : il n'y avait pas
//! de fichier), et ne pas la distinguer fabriquerait un silence qui n'existait
//! pas. Ce n'est donc pas un changement de comportement : c'est le traitement
//! d'un état **nouveau**, et il est dit plutôt que découvert.
//!
//! # Le chemin court, avant toute résolution
//!
//! Cette sous-commande sort dans le `match &cli.command` de tête de `main.rs`,
//! à côté de `Token` et `CredentialHelper`. Elle fait **strictement moins** que
//! `Token` : ni `dotenv`, ni `Settings`, ni `home`, ni base, ni réseau. Un
//! prédicat pur n'a aucune raison de résoudre un agent — et c'est ce qui rend
//! le coût d'un démarrage de process acceptable sur un chemin appelé **une fois
//! par dispatch**.
//!
//! # La borne sur le chemin multi-ligne
//!
//! `[^`]+` n'exclut pas `\n` en Rust, donc un callout dont le backtick ferme
//! plus loin dans le corps capture un chemin **multi-ligne** (divergence n°3 de
//! mika#2194, mesurée). Le motif est conservé à l'identique — le resserrer est
//! ce que la borne B1 refuse — et c'est **ce canal-ci** qui borne, parce que
//! c'est ici que la valeur devient une ligne de stdout : un chemin multi-ligne
//! n'est pas représentable dans un contrat qui dit « le chemin, une ligne ».
//!
//! Le refus est donc un `≥2` nommé (`path_not_single_line`), et il est
//! strictement plus sûr que l'état d'avant, où l'un des deux lecteurs rendait
//! un chemin absurde et l'autre un chemin tronqué, sans que personne l'ait
//! mesuré.

use std::path::Path;

use mika_agent::plan_callout::{FenceHandling, plan_callout};

/// Le code de sortie « je n'ai pas pu regarder ».
///
/// Un seul site de définition : c'est un **format de fil** lu par
/// `_detect_plan_on_branch`, qui distingue `1` (aucun callout) de `≥2` (refus
/// bruyant). Deux littéraux pourraient diverger.
pub const EXIT_UNREADABLE: i32 = 2;

/// Le préfixe des motifs de refus, repris par les surfaces opérateur.
const REFUSAL_PREFIX: &str = "REFUSED (plan-callout, mika#2194)";

/// Le résultat de la sous-commande, avant de devenir un code de sortie.
///
/// Un `enum` plutôt qu'un `Result<Option<String>, _>` : les trois issues sont
/// trois réponses différentes à la question posée, et « aucun callout » n'est
/// ni une erreur ni un succès partiel. C'est ce qui rend la fonction pure
/// testable sans process.
#[derive(Debug, PartialEq, Eq)]
pub enum CalloutOutcome {
    /// Un callout a été lu. Code `0`, le chemin sur stdout.
    Found(String),
    /// Aucun callout. Code `1`, stdout vide.
    Absent,
    /// On n'a pas pu regarder, ou la réponse n'est pas représentable. Code
    /// `≥2`, stdout vide, motif sur stderr.
    Unreadable {
        reason: &'static str,
        detail: String,
    },
}

/// La décision, séparée de toute entrée-sortie.
///
/// `body` est le contenu déjà lu ; la lecture du fichier et l'écriture des flux
/// vivent dans [`run`]. Ce découpage est ce qui permet d'exercer les trois
/// issues — y compris `path_not_single_line`, qui demanderait sinon un corps
/// forgé sur disque — sans lancer un process.
pub fn decide(body: &str, raw_form: bool) -> CalloutOutcome {
    // `Keep` : la parité EXACTE avec le comportement du bash d'avant la
    // bascule. Demander `Strip` ici serait une correction de comportement, que
    // les bornes du ticket interdisent pendant la migration.
    let Some(callout) = plan_callout(body, FenceHandling::Keep) else {
        return CalloutOutcome::Absent;
    };

    let path = if raw_form {
        callout.raw
    } else {
        callout.normalized
    };

    if path.contains('\n') || path.contains('\r') {
        return CalloutOutcome::Unreadable {
            reason: "path_not_single_line",
            detail: format!(
                "le callout capture un chemin de {} octets contenant un retour \
                 à la ligne ; le contrat de cette sous-commande est « le chemin, \
                 une ligne ». Le callout est probablement non terminé (backtick \
                 de fermeture manquant sur sa ligne).",
                path.len()
            ),
        };
    }

    CalloutOutcome::Found(path)
}

/// Lit le corps, décide, et rend le code de sortie.
///
/// Ne rend pas `anyhow::Result` : les trois issues sont des **codes**, et
/// laisser `main` transformer une erreur en `1` écraserait la distinction entre
/// « aucun callout » et « je n'ai pas pu regarder » — c'est-à-dire le livrable
/// de R2.
pub fn run(body_file: &Path, raw_form: bool) -> i32 {
    let body = match std::fs::read_to_string(body_file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "{REFUSAL_PREFIX} body_file_unreadable: {} — {e}",
                body_file.display()
            );
            return EXIT_UNREADABLE;
        }
    };

    match decide(&body, raw_form) {
        CalloutOutcome::Found(path) => {
            println!("{path}");
            0
        }
        CalloutOutcome::Absent => 1,
        CalloutOutcome::Unreadable { reason, detail } => {
            eprintln!("{REFUSAL_PREFIX} {reason}: {detail}");
            EXIT_UNREADABLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARE: &str = "> - **Plan:** `docs/plans/x-plan.md` (committed @ `abc`)";
    const PREFIXED: &str = "> - **Plan:** `mika/docs/plans/x-plan.md` (committed @ `abc`)";

    /// V8 — le code `0` et la forme par défaut (normalisée, celle que
    /// `dispatch-lib` résout contre `$WORKTREE_DIR`).
    #[test]
    fn mika2194_forme_par_defaut_est_normalisee() {
        assert_eq!(
            decide(PREFIXED, false),
            CalloutOutcome::Found("docs/plans/x-plan.md".to_string())
        );
    }

    /// `--raw` rend la forme que `auto_pull` veut : préfixe conservé.
    #[test]
    fn mika2194_raw_conserve_le_prefixe() {
        assert_eq!(
            decide(PREFIXED, true),
            CalloutOutcome::Found("mika/docs/plans/x-plan.md".to_string())
        );
    }

    /// Sur une forme nue les deux drapeaux coïncident — et l'asserter est ce
    /// qui distingue « `--raw` est lu » de « `--raw` ne fait rien ».
    #[test]
    fn mika2194_sur_une_forme_nue_les_deux_drapeaux_coincident() {
        let expected = CalloutOutcome::Found("docs/plans/x-plan.md".to_string());
        assert_eq!(decide(BARE, false), expected);
        assert_eq!(decide(BARE, true), expected);
    }

    /// V8 — le code `1` : « aucun callout » est une réponse, pas une panne.
    #[test]
    fn mika2194_aucun_callout_est_absent_et_non_illisible() {
        for body in [
            "aucun callout ici",
            "> - **Plan:** `docs/brainstorms/x.md`",
            "",
        ] {
            assert_eq!(
                decide(body, false),
                CalloutOutcome::Absent,
                "corps : {body:?}"
            );
        }
    }

    /// V8 — le code `≥2` sur la divergence n°3, et le motif est nommé.
    ///
    /// C'est le seul refus que `decide` peut produire sans système de fichiers,
    /// et il n'est pas théorique : `backtick-late-close.md` du corpus doré le
    /// déclenche.
    #[test]
    fn mika2194_un_chemin_multiligne_est_refuse_avec_son_motif() {
        // Backtick non terminé sur la ligne du callout, backtick plus loin dans
        // le corps : la capture traverse les lignes.
        let body = "> - **Plan:** `docs/plans/x-plan.md\nune autre ligne avec un `backtick`\n";
        match decide(body, false) {
            CalloutOutcome::Unreadable { reason, .. } => {
                assert_eq!(reason, "path_not_single_line");
            }
            other => panic!("attendu un refus nommé, lu {other:?}"),
        }
    }

    /// Contrôle négatif du refus ci-dessus : un corps sain n'est jamais refusé.
    ///
    /// Sans lui, « la borne décide » est indistinguable de « la borne refuse
    /// tout », et la sous-commande pourrait refuser chaque dispatch avec tous
    /// les autres tests au vert.
    #[test]
    fn mika2194_un_corps_sain_nest_jamais_refuse() {
        for body in [BARE, PREFIXED] {
            match decide(body, false) {
                CalloutOutcome::Found(_) => {}
                other => panic!("un corps sain doit être lu, lu {other:?}"),
            }
        }
    }

    /// Le corpus doré est le même de part et d'autre du canal.
    ///
    /// Ce test lit le corpus de `mika-agent` depuis `mika-cli` : c'est ce qui
    /// atteste que la sous-commande répond comme le lecteur unique, plutôt que
    /// comme une troisième interprétation. Un corpus qui divergerait d'un
    /// crate à l'autre serait la duplication que ce ticket retire, déplacée
    /// d'un cran.
    #[test]
    fn mika2194_le_canal_repond_comme_le_lecteur_sur_le_corpus_dore() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../mika-agent/tests/fixtures/plan_callout_bodies");
        let tsv = corpus.join("expectations.tsv");
        let content = std::fs::read_to_string(&tsv)
            .unwrap_or_else(|e| panic!("corpus introuvable : {} ({e})", tsv.display()));

        let mut checked = 0usize;
        for line in content.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            assert_eq!(cols.len(), 6, "format du TSV : {line:?}");
            let (file, rc, normalized, parity) = (cols[0], cols[1], cols[3], cols[4]);

            let body = std::fs::read_to_string(corpus.join(file))
                .unwrap_or_else(|e| panic!("corps introuvable : {file} ({e})"));

            match decide(&body, false) {
                CalloutOutcome::Found(p) => {
                    assert_eq!(rc, "0", "{file}: lu un callout alors que rc={rc}");
                    assert_eq!(p, normalized, "{file}: chemin normalisé divergent");
                }
                CalloutOutcome::Absent => {
                    assert_eq!(rc, "1", "{file}: aucun callout alors que rc={rc}");
                }
                CalloutOutcome::Unreadable { reason, .. } => {
                    // La seule population légitime de refus dans ce corpus est
                    // la divergence n°3, et elle est déclarée comme telle.
                    assert_eq!(
                        parity, "divergent-multiline",
                        "{file}: refus `{reason}` sur une ligne qui ne le déclare pas"
                    );
                }
            }
            checked += 1;
        }

        assert!(
            checked > 0,
            "aucun cas exercé — le canal n'a été comparé à rien (anti-vacuité)"
        );
        println!("mika#2194 — canal CLI comparé au lecteur sur {checked} cas");
    }
}
