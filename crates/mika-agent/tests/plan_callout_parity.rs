//! Le lecteur **Rust** du corpus doré du callout `Plan` (mika#2194 R4).
//!
//! Son jumeau bash est le bloc « mika#2194 » de
//! `skills/bundled/_shared/test-dispatch-lib.sh`. Les deux lisent **le même**
//! fichier d'attendus, `tests/fixtures/plan_callout_bodies/expectations.tsv`,
//! et c'est tout l'objet de R4 : avant ce ticket les deux lecteurs avaient
//! chacun un corpus soigné, et **aucune entrée n'était commune**. Personne
//! n'avait jamais exécuté les deux sur la même entrée.
//!
//! # Ce que ce fichier asserte, et ce qu'il ne peut pas asserter
//!
//! Il asserte les attendus du TSV sous `FenceHandling::Keep` (la politique de
//! `dispatch-lib`, donc la moitié dont la parité était inconnue) **plus** la
//! relation `Keep`↔`Strip` que la colonne `parity` déclare.
//!
//! Il ne peut pas asserter que le **bash** est d'accord : ça demande de faire
//! tourner du shell, et c'est le rôle de son jumeau. Ce que la composition des
//! deux achète est le corpus commun, pas un lecteur qui en remplacerait un.
//!
//! # L'anti-vacuité n'est pas décorative
//!
//! Une parité verte sur zéro cas est le mode de panne que R4 nomme. Ce fichier
//! **échoue** si le TSV est absent, vide, ou nomme un corps introuvable, et il
//! **imprime son compte de cas** — le seul moyen qu'un lecteur ait de dire
//! qu'il a regardé quelque chose.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mika_agent::plan_callout::{FenceHandling, plan_callout};

/// La sentinelle des attendus que le TSV ne peut pas porter : un chemin
/// contenant un retour à la ligne.
const MULTILINE_SENTINEL: &str = "<multiline>";

/// Les valeurs légales de la colonne `parity`. **Format de fil** : elles sont
/// lues par les deux lecteurs, donc deux orthographes d'une même relation
/// couperaient une population en deux sans le dire.
const PARITY_VALUES: &[&str] = &[
    "equal",
    "divergent-fences",
    "divergent-bash-legacy",
    "divergent-multiline",
];

/// Idem pour `phase`.
const PHASE_VALUES: &[&str] = &["both", "pre-switch"];

#[derive(Debug)]
struct Expectation {
    file: String,
    rc: u8,
    raw: String,
    normalized: String,
    parity: String,
    phase: String,
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("plan_callout_bodies")
}

fn read_expectations() -> Vec<Expectation> {
    let tsv = corpus_dir().join("expectations.tsv");
    let content = std::fs::read_to_string(&tsv).unwrap_or_else(|e| {
        panic!(
            "corpus introuvable : {} ({e}).\n\
             Une parité qui ne lit aucun attendu est verte sur zéro cas — \
             c'est le mode de panne que mika#2194 R4 nomme.",
            tsv.display()
        )
    });

    let mut out = Vec::new();
    for (lineno, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            cols.len(),
            6,
            "ligne {} du TSV : {} colonnes au lieu de 6 — le format est \
             `fichier · rc · raw · normalized · parity · phase`\n  {line:?}",
            lineno + 1,
            cols.len()
        );
        let rc: u8 = cols[1].parse().unwrap_or_else(|_| {
            panic!(
                "ligne {} du TSV : `rc` doit être 0 ou 1, lu {:?}",
                lineno + 1,
                cols[1]
            )
        });
        assert!(
            rc <= 1,
            "ligne {} du TSV : `rc` doit être 0 ou 1, lu {rc}",
            lineno + 1
        );
        assert!(
            PARITY_VALUES.contains(&cols[4]),
            "ligne {} du TSV : `parity` = {:?} hors du format de fil {PARITY_VALUES:?}",
            lineno + 1,
            cols[4]
        );
        assert!(
            PHASE_VALUES.contains(&cols[5]),
            "ligne {} du TSV : `phase` = {:?} hors du format de fil {PHASE_VALUES:?}",
            lineno + 1,
            cols[5]
        );
        out.push(Expectation {
            file: cols[0].to_string(),
            rc,
            raw: cols[2].to_string(),
            normalized: cols[3].to_string(),
            parity: cols[4].to_string(),
            phase: cols[5].to_string(),
        });
    }

    assert!(
        !out.is_empty(),
        "le corpus est vide — une parité verte sur zéro cas ne prouve rien \
         (mika#2194 R4, anti-vacuité)"
    );
    out
}

fn read_body(file: &str) -> String {
    let path = corpus_dir().join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "le TSV nomme un corps introuvable : {} ({e}).\n\
             Un attendu qui ne désigne rien est une ligne qui mente avec \
             l'autorité d'un inventaire.",
            path.display()
        )
    })
}

/// V2 — les attendus du corpus, sous la politique de `dispatch-lib`.
#[test]
fn mika2194_le_corpus_est_lu_sous_keep() {
    let expectations = read_expectations();
    let mut checked = 0usize;

    for e in &expectations {
        let body = read_body(&e.file);
        let got = plan_callout(&body, FenceHandling::Keep);

        match (e.rc, &got) {
            (1, Some(c)) => panic!(
                "{}: attendu AUCUN callout (rc=1), lu raw={:?}",
                e.file, c.raw
            ),
            (0, None) => panic!("{}: attendu un callout (rc=0), rien lu", e.file),
            (1, None) => {}
            (0, Some(c)) => {
                if e.parity == "divergent-multiline" {
                    // Attendu que le TSV ne peut pas porter : la capture
                    // traverse les lignes. On asserte la propriété, pas une
                    // égalité de chaîne.
                    assert_eq!(
                        e.raw, MULTILINE_SENTINEL,
                        "{}: une ligne `divergent-multiline` doit porter la \
                         sentinelle {MULTILINE_SENTINEL:?} en `raw`",
                        e.file
                    );
                    assert!(
                        c.raw.contains('\n'),
                        "{}: attendu une capture multi-ligne, lu raw={:?}\n\
                         Si la capture a cessé de traverser les lignes, le \
                         motif a changé : c'est un changement de tolérance, \
                         que les bornes du ticket interdisent pendant la \
                         migration.",
                        e.file,
                        c.raw
                    );
                } else {
                    assert_eq!(c.raw, e.raw, "{}: `raw` diverge de l'attendu", e.file);
                    assert_eq!(
                        c.normalized, e.normalized,
                        "{}: `normalized` diverge de l'attendu",
                        e.file
                    );
                }
            }
            _ => unreachable!("rc est borné à 0|1 par read_expectations"),
        }
        checked += 1;
    }

    assert!(checked > 0, "aucun cas exercé (anti-vacuité)");
    println!("mika#2194 parité Rust — {checked} cas exercés sous FenceHandling::Keep");
}

/// L'assertion **auto-nettoyante** de la Fire-Disposition.
///
/// Une ligne `divergent-fences` déclare que les deux politiques ne rendent
/// **pas** la même chose. Le jour où la divergence est tranchée, ce test
/// échoue et la ligne doit être retirée : elle ne peut pas devenir périmée en
/// silence. C'est la propriété qui distingue une exception d'un contournement.
///
/// Son pendant négatif est dans le même test, et il est porteur : une ligne
/// `equal` dont les deux politiques divergeraient est une divergence que
/// personne n'a déclarée.
#[test]
fn mika2194_les_divergences_declarees_sont_reelles_et_les_autres_absentes() {
    let expectations = read_expectations();
    let mut divergences_declarees = 0usize;
    let mut egalites_verifiees = 0usize;

    for e in &expectations {
        let body = read_body(&e.file);
        let keep = plan_callout(&body, FenceHandling::Keep).map(|c| c.raw);
        let strip = plan_callout(&body, FenceHandling::Strip).map(|c| c.raw);

        if e.parity == "divergent-fences" {
            assert_ne!(
                keep, strip,
                "{}: la ligne déclare `divergent-fences`, mais les deux \
                 politiques rendent la MÊME valeur ({keep:?}).\n\n\
                 ASSERTION AUTO-NETTOYANTE : la divergence a été tranchée. \
                 Retirez cette ligne du TSV (et le fixture s'il ne mesure plus \
                 rien) — ne la laissez pas devenir périmée en silence.",
                e.file
            );
            divergences_declarees += 1;
        } else {
            assert_eq!(
                keep, strip,
                "{}: la ligne ne déclare aucune divergence de fences, mais les \
                 deux politiques rendent des valeurs différentes.\n\n\
                 Une divergence que personne n'a déclarée est exactement ce que \
                 mika#2194 existe pour rendre impossible. Déclarez-la \
                 (`divergent-fences`) ou réparez le lecteur.",
                e.file
            );
            egalites_verifiees += 1;
        }
    }

    assert!(
        divergences_declarees > 0,
        "aucune divergence déclarée — si la Fire-Disposition de mika#2194 a été \
         résolue, ce test doit être retiré avec elle ; s'il reste, il ne \
         vérifie plus rien (classe mika#2205)"
    );
    assert!(
        egalites_verifiees > 0,
        "aucune égalité vérifiée — le contrôle négatif de ce test est vide, \
         donc « la garde décide » est indistinguable de « la garde exige \
         toujours une divergence »"
    );
    println!(
        "mika#2194 — {divergences_declarees} divergence(s) déclarée(s) et \
         réelle(s), {egalites_verifiees} égalité(s) vérifiée(s)"
    );
}

/// `phase` et `parity` sont cohérentes, et c'est le seul usage que le lecteur
/// **Rust** a de `phase` — le bash est celui qui s'en sert pour décider ce
/// qu'il exerce.
///
/// La contrainte : une ligne `pre-switch` doit déclarer une divergence. Sortir
/// une ligne de la passe post-bascule n'a de sens que si le bash d'avant s'y
/// comportait autrement ; une ligne `equal` marquée `pre-switch` serait un cas
/// que plus personne n'exerce, sans raison écrite.
///
/// L'inverse n'est pas vrai et ne doit pas l'être : `divergent-fences` est
/// `both`, parce que sa divergence est interne au Rust (`Keep`↔`Strip`) et
/// reste donc mesurable après la bascule.
#[test]
fn mika2194_une_ligne_pre_switch_declare_une_divergence() {
    let expectations = read_expectations();
    let mut pre_switch = 0usize;

    for e in &expectations {
        if e.phase == "pre-switch" {
            assert_ne!(
                e.parity, "equal",
                "{}: la ligne est `pre-switch` mais ne déclare aucune \
                 divergence.\n\n\
                 Sortir un cas de la passe post-bascule n'a de sens que si le \
                 bash d'avant s'y comportait autrement. Tel quel, ce cas n'est \
                 plus exercé par personne et rien ne dit pourquoi.",
                e.file
            );
            pre_switch += 1;
        }
    }

    println!("mika#2194 — {pre_switch} ligne(s) hors de la passe post-bascule");
}

/// Le ticket qui a **tranché** la divergence `divergent-fences`, et vers lequel
/// les trois surfaces qui en parlent doivent pointer (mika#2609 AC3).
///
/// Figé ici plutôt que déduit : c'est l'ancrage. Un futur réveil qui change le
/// ticket de référence met cette constante à jour, et les trois surfaces avec.
const DIVERGENCE_FENCES_TRANCHEE_PAR: &str = "mika#2609";

/// La formule d'un renvoi qui ne nomme personne.
///
/// C'est **la** forme de la référence croisée morte que mika#2609 ferme : le
/// TSV renvoyait à « Ticket de suivi » et aucun ticket n'existait. Une
/// occurrence est tolérée **seulement** si un `mika#<n>` l'accompagne sur la
/// même ligne.
const FORMULE_SANS_NUMERO: &str = "ticket de suivi";

/// Les surfaces exemptées du contrôle de référence croisée. **LIVRÉE VIDE.**
///
/// Matérialisée — nommée, grep-visible, à côté du test — précisément pour que
/// son absence de contenu soit un fait lisible et non un oubli, et pour qu'une
/// future exemption soit un ajout visible en revue. Doctrine mika#2201 :
/// **quand le détecteur tire, on corrige la référence, on n'ajoute pas une
/// ligne ici.**
///
/// Gardée vide par l'assertion auto-nettoyante de
/// [`mika2609_le_renvoi_au_suivi_de_la_divergence_porte_un_numero`] : une
/// entrée qui ne désigne plus une surface réelle (fichier absent, ou surface
/// devenue conforme) fait **échouer** le test. Une exemption ne peut donc pas
/// devenir périmée en silence.
const CROSS_REFERENCE_EXEMPTIONS: &[CrossReferenceExemption] = &[];

struct CrossReferenceExemption {
    /// Nom de fichier, relatif au répertoire du corpus.
    file: &'static str,
    /// Pourquoi cette surface est exemptée. Jamais vide.
    #[allow(dead_code, reason = "lu par l'humain en revue, pas par le test")]
    reason: &'static str,
}

/// Une référence de la forme `README.md § <titre>` trouvée dans le TSV.
#[derive(Debug)]
struct SectionReference {
    lineno: usize,
    title: String,
}

/// Le texte porte-t-il un `mika#<n>` (un `#` suivi d'au moins un chiffre) ?
///
/// Écrit à la main plutôt qu'avec `regex` : les `dependencies` du crate ne sont
/// pas accessibles depuis un test d'intégration, et le prédicat tient en six
/// lignes.
fn cites_a_ticket_number(text: &str) -> bool {
    text.match_indices("mika#")
        .any(|(i, m)| text[i + m.len()..].starts_with(|c: char| c.is_ascii_digit()))
}

/// Les lignes portant [`FORMULE_SANS_NUMERO`] **sans** `mika#<n>` sur la même
/// ligne, avec leur numéro de ligne (1-indexé).
fn lignes_sans_numero_adjacent(content: &str) -> Vec<(usize, String)> {
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            line.to_lowercase().contains(FORMULE_SANS_NUMERO) && !cites_a_ticket_number(line)
        })
        .map(|(i, line)| (i + 1, line.trim().to_string()))
        .collect()
}

/// Le bloc de commentaire qui **précède immédiatement** la ligne de données de
/// `file` dans le TSV, lignes `#` jointes.
///
/// Le site que mika#2609 R1 cible est ce bloc, pas le fichier entier : un
/// renvoi laissé dans un bloc voisin ne nomme pas cette divergence-ci.
fn bloc_de_commentaire_precedant(tsv: &str, file: &str) -> String {
    let lines: Vec<&str> = tsv.lines().collect();
    let Some(data_idx) = lines
        .iter()
        .position(|l| l.split('\t').next() == Some(file))
    else {
        return String::new();
    };

    let mut start = data_idx;
    while start > 0 && lines[start - 1].trim_start().starts_with('#') {
        start -= 1;
    }
    lines[start..data_idx].join("\n")
}

/// Les titres de section du README voisin, dans l'ordre.
fn readme_headings(readme: &str) -> Vec<String> {
    readme
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start_matches('#');
            let hashes = line.len() - rest.len();
            if (1..=6).contains(&hashes) && rest.starts_with(' ') {
                Some(normalize_section_title(rest))
            } else {
                None
            }
        })
        .collect()
}

/// Normalise un titre de section pour la comparaison : espaces rognés,
/// ponctuation finale retirée, casse repliée.
fn normalize_section_title(title: &str) -> String {
    title
        .trim()
        .trim_end_matches(['.', ',', ';', ':', '»', '"', '\''])
        .trim()
        .to_lowercase()
}

/// Les références `README.md § <titre>` portées par le TSV.
///
/// Le titre court jusqu'à la fin de la ligne : c'est la forme que la référence
/// morte avait (« `README.md § La divergence fences.` »), et borner plus tôt
/// demanderait un délimiteur que rien ne garantit.
fn section_references(tsv: &str) -> Vec<SectionReference> {
    const NEEDLE: &str = "README.md §";
    tsv.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            line.find(NEEDLE).map(|at| SectionReference {
                lineno: i + 1,
                title: normalize_section_title(&line[at + NEEDLE.len()..]),
            })
        })
        .collect()
}

/// Les fichiers déclarés `divergent-fences` dans le TSV.
fn fixtures_divergent_fences() -> Vec<String> {
    read_expectations()
        .into_iter()
        .filter(|e| e.parity == "divergent-fences")
        .map(|e| e.file)
        .collect()
}

/// R6(a) — **tant qu'une divergence `divergent-fences` est déclarée, les trois
/// surfaces qui en parlent nomment le ticket qui l'a tranchée.**
///
/// Ferme la **classe** plutôt que l'instance. La référence morte du 2026-09-30
/// (« voir « Ticket de suivi » dans README.md § La divergence fences », alors
/// que ni la section ni le ticket n'existaient) a survécu à un refactor du
/// fichier qui la portait — mika#2608 a inséré trois fixtures et décalé la
/// ligne de 112 à 132 — **sans qu'aucun test ne rougisse**. Ce n'est pas une
/// coquille, c'est une classe : un renvoi qui ne nomme personne ne peut pas
/// devenir faux, il l'est déjà.
///
/// Deux termes, et le second est celui qui a mordu à l'écriture :
/// 1. les trois surfaces citent [`DIVERGENCE_FENCES_TRANCHEE_PAR`] ;
/// 2. aucune ne porte [`FORMULE_SANS_NUMERO`] sans `mika#<n>` sur la ligne.
#[test]
fn mika2609_le_renvoi_au_suivi_de_la_divergence_porte_un_numero() {
    let tsv_path = corpus_dir().join("expectations.tsv");
    let readme_path = corpus_dir().join("README.md");

    let tsv = std::fs::read_to_string(&tsv_path)
        .unwrap_or_else(|e| panic!("TSV illisible : {} ({e})", tsv_path.display()));
    let readme = std::fs::read_to_string(&readme_path).unwrap_or_else(|e| {
        panic!(
            "README du corpus illisible : {} ({e}).\n\
             Un détecteur de référence croisée qui ne peut pas lire la cible \
             de la référence ne vérifie rien (classe mika#2205).",
            readme_path.display()
        )
    });

    let divergentes = fixtures_divergent_fences();
    assert!(
        !divergentes.is_empty(),
        "aucune ligne `divergent-fences` dans le TSV — ce détecteur n'exerce \
         plus aucune surface.\n\n\
         Si la divergence a été retirée du corpus, retirez ce test avec elle ; \
         s'il reste, il est vert sans rien regarder (classe mika#2205)."
    );

    // L'allowlist est auto-nettoyante : une entrée doit désigner un fichier
    // réel ET non conforme, sinon elle est périmée et le dit.
    for ex in CROSS_REFERENCE_EXEMPTIONS {
        assert!(
            !ex.reason.trim().is_empty(),
            "exemption de {:?} sans raison écrite",
            ex.file
        );
        let path = corpus_dir().join(ex.file);
        let body = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "l'exemption nomme une surface absente : {} ({e}).\n\n\
                 ASSERTION AUTO-NETTOYANTE : retirez l'entrée de \
                 CROSS_REFERENCE_EXEMPTIONS.",
                path.display()
            )
        });
        let non_conforme =
            !cites_a_ticket_number(&body) || !lignes_sans_numero_adjacent(&body).is_empty();
        assert!(
            non_conforme,
            "{} est exempté mais il est devenu CONFORME.\n\n\
             ASSERTION AUTO-NETTOYANTE : retirez l'entrée de \
             CROSS_REFERENCE_EXEMPTIONS — une exemption ne doit pas devenir \
             périmée en silence.",
            ex.file
        );
    }
    let exempte = |file: &str| CROSS_REFERENCE_EXEMPTIONS.iter().any(|e| e.file == file);

    let mut surfaces: Vec<(String, String)> = Vec::new();
    if !exempte("expectations.tsv") {
        // Le site est le bloc de commentaire de la divergence, pas le TSV
        // entier : un numéro laissé dans un bloc voisin ne nomme pas celle-ci.
        for file in &divergentes {
            surfaces.push((
                format!("expectations.tsv (bloc de {file})"),
                bloc_de_commentaire_precedant(&tsv, file),
            ));
        }
    }
    if !exempte("README.md") {
        surfaces.push(("README.md".to_string(), readme.clone()));
    }
    for file in &divergentes {
        if !exempte(file) {
            surfaces.push((file.clone(), read_body(file)));
        }
    }

    assert!(
        !surfaces.is_empty(),
        "toutes les surfaces sont exemptées — le détecteur ne regarde rien"
    );

    for (name, content) in &surfaces {
        assert!(
            content.contains(DIVERGENCE_FENCES_TRANCHEE_PAR),
            "{name} ne cite pas {DIVERGENCE_FENCES_TRANCHEE_PAR}, le ticket qui \
             a tranché la divergence `divergent-fences`.\n\n\
             Une surface qui décrit une divergence assumée doit nommer la \
             décision qui l'assume, par son NUMÉRO — jamais par un renvoi à une \
             section (un numéro de ligne et un titre de section pourrissent en \
             silence ; un numéro de ticket, non)."
        );

        let orphelines = lignes_sans_numero_adjacent(content);
        assert!(
            orphelines.is_empty(),
            "{name} porte la formule {FORMULE_SANS_NUMERO:?} sans \
             `mika#<n>` sur la même ligne : {orphelines:?}\n\n\
             C'est exactement la référence morte que mika#2609 a fermée. \
             Nommez le ticket sur la ligne."
        );
    }

    println!(
        "mika#2609 — {} surface(s) vérifiée(s) pour {} divergence(s) \
         `divergent-fences`, {} exemption(s)",
        surfaces.len(),
        divergentes.len(),
        CROSS_REFERENCE_EXEMPTIONS.len()
    );
}

/// R6(b) — **un renvoi `README.md § <titre>` désigne un heading qui existe.**
///
/// C'est l'assertion qui **aurait rougi le 2026-09-30** : le TSV renvoyait à
/// « README.md § La divergence fences » et le README voisin, 111 lignes, n'avait
/// aucune section de ce nom. Et elle aurait survécu au refactor de mika#2608,
/// parce qu'elle ne porte ni numéro de ligne ni ordre.
///
/// L'anti-vacuité porte sur le **parseur de headings**, non sur le nombre de
/// références : zéro référence est l'état **sain** depuis mika#2609 (R1 renvoie
/// au ticket, pas à une section), tandis qu'un parseur qui ne trouverait aucun
/// heading laisserait passer n'importe quel renvoi.
#[test]
fn mika2609_un_renvoi_a_une_section_du_readme_designe_un_heading_reel() {
    let tsv_path = corpus_dir().join("expectations.tsv");
    let readme_path = corpus_dir().join("README.md");

    let tsv = std::fs::read_to_string(&tsv_path)
        .unwrap_or_else(|e| panic!("TSV illisible : {} ({e})", tsv_path.display()));
    let readme = std::fs::read_to_string(&readme_path)
        .unwrap_or_else(|e| panic!("README illisible : {} ({e})", readme_path.display()));

    let headings = readme_headings(&readme);
    assert!(
        !headings.is_empty(),
        "aucun heading lu dans {} — le parseur de sections ne voit rien, donc \
         ce test laisserait passer N'IMPORTE QUEL renvoi (classe mika#2205).",
        readme_path.display()
    );

    let refs = section_references(&tsv);
    for r in &refs {
        assert!(
            headings.contains(&r.title),
            "ligne {} du TSV renvoie à « README.md § {} », qui ne correspond à \
             aucune section du README voisin.\n\n\
             Sections disponibles : {:?}\n\n\
             Un renvoi vers une section inexistante pointe dans le vide sans \
             jamais échouer — c'est le défaut que mika#2609 a mesuré. Nommez la \
             section telle qu'elle est écrite, ou renvoyez au TICKET.",
            r.lineno,
            r.title,
            headings
        );
    }

    println!(
        "mika#2609 — {} renvoi(s) `README.md § …` confronté(s) à {} section(s) \
         du README (zéro renvoi est l'état sain : R1 renvoie au ticket)",
        refs.len(),
        headings.len()
    );
}

/// Le corpus et le TSV se recouvrent exactement.
///
/// Sans ça, un fixture ajouté et jamais déclaré est un cas que **personne**
/// n'exerce — et il a l'air d'être couvert parce qu'il est dans le répertoire.
/// La direction inverse (un attendu sans corps) est déjà tenue par
/// [`read_body`], mais les deux sont assertées ici pour que le message le dise.
#[test]
fn mika2194_chaque_corps_du_corpus_est_declare() {
    let declared: BTreeSet<String> = read_expectations().into_iter().map(|e| e.file).collect();

    let mut on_disk = BTreeSet::new();
    for entry in std::fs::read_dir(corpus_dir()).expect("répertoire de corpus lisible") {
        let path = entry.expect("entrée lisible").path();
        if path.extension().is_some_and(|x| x == "md") {
            let name = path
                .file_name()
                .expect("nom de fichier")
                .to_string_lossy()
                .to_string();
            // Le README documente le corpus, il n'en est pas un membre.
            if name != "README.md" {
                on_disk.insert(name);
            }
        }
    }

    assert!(!on_disk.is_empty(), "aucun corps sur disque (anti-vacuité)");

    let undeclared: Vec<&String> = on_disk.difference(&declared).collect();
    assert!(
        undeclared.is_empty(),
        "des corps du corpus ne sont déclarés dans aucun attendu : {undeclared:?}\n\n\
         Un fixture non déclaré n'est exercé par personne, tout en ayant l'air \
         couvert. Déclarez-le dans expectations.tsv, ou retirez-le."
    );

    let orphans: Vec<&String> = declared.difference(&on_disk).collect();
    assert!(
        orphans.is_empty(),
        "des attendus nomment un corps absent du répertoire : {orphans:?}"
    );
}
