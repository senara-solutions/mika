//! Le lecteur Rust du corpus doré du tier 2 de paraphrase (mika#2194, vol 3).
//!
//! Il lit **le même** fichier d'attendus que le bloc bash
//! « mika#2194 — parité du tier 2 de paraphrase » de
//! `skills/bundled/_shared/test-dispatch-lib.sh` :
//! `tests/fixtures/arch_paraphrase_texts/expectations.tsv`. Le bash y fait
//! passer `_parse_disposition_fuzzy` / `_parse_verdict_fuzzy` — les lecteurs de
//! **production** — et ce fichier y fait passer
//! `mika_agent::arch_paraphrase::paraphrase`, qui n'a aucun appelant. Un attendu
//! modifié d'un seul côté redevient rouge de l'autre.
//!
//! # Anti-vacuité : trois termes, parce qu'une parité verte sur zéro cas ment
//!
//! Un lecteur de corpus a trois façons de n'exercer rien tout en passant :
//! le fichier d'attendus manque, il ne porte aucune ligne, ou il nomme un corps
//! qui n'existe pas. Les trois sont des **échecs** ici, et le compte de cas est
//! une assertion — jamais un `println!` que personne ne lit.
//!
//! # Trois assertions auto-nettoyantes
//!
//! Le corpus déclare trois classes d'exception — `substring-fp`,
//! `french-inert`, `multiline`. Chacune nomme un comportement que le bash a
//! aujourd'hui et que B1 interdit de corriger pendant la migration. Le jour où
//! l'une cesse, le test **échoue** au lieu de laisser une classe devenue vide
//! passer pour une classe satisfaite. C'est l'option (a) de la Fire-Disposition
//! du plan : une liste nommée dont la population est non vide par construction,
//! et dont la violation est bruyante.
//!
//! Les trois portent sur une propriété **interne au Rust** — la frontière de
//! mot, le token rendu, le match par ligne — et non sur une comparaison au
//! bash : elles survivent donc à la bascule du vol suivant, où le bash appellera
//! le Rust et où une parité bash↔Rust deviendrait une tautologie.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mika_agent::arch_paraphrase::{ArchPass, Paraphrase, paraphrase};

/// Ce qu'une colonne vide porte. **Jamais une chaîne vide** : `IFS=$'\t' read`
/// fusionne deux tabulations consécutives, et le lecteur bash verrait ses
/// colonnes se décaler là où `split('\t')` ne bouge pas. Les deux lecteurs
/// liraient alors des choses différentes du même octet.
const EMPTY: &str = "-";

/// Format de fil de la colonne `pass`.
const PASS_VALUES: &[&str] = &["first", "second"];

/// Format de fil de la colonne `token`, `-` compris.
const TOKEN_VALUES: &[&str] = &[EMPTY, "READY", "ITERATE", "ESCALATE", "GROOMED"];

/// Format de fil de la colonne `classe`. **Fermé** : une cinquième classe est
/// une décision de corpus, pas une faute de frappe qui passe.
const CLASS_VALUES: &[&str] = &["nominal", "substring-fp", "french-inert", "multiline"];

#[derive(Debug)]
struct Expectation {
    file: String,
    pass: String,
    token: String,
    fragment: String,
    class: String,
}

impl Expectation {
    fn arch_pass(&self) -> ArchPass {
        match self.pass.as_str() {
            "first" => ArchPass::First,
            "second" => ArchPass::Second,
            other => unreachable!("passe hors format de fil : {other:?}"),
        }
    }

    /// Le libellé qui apparaît dans chaque message d'échec : un fichier seul ne
    /// suffit pas, deux lignes peuvent porter le même corps sur deux passes.
    fn label(&self) -> String {
        format!("{} ({})", self.file, self.pass)
    }
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("arch_paraphrase_texts")
}

/// Anti-vacuité 1/3 (le TSV existe) et 2/3 (il porte des lignes).
fn read_expectations() -> Vec<Expectation> {
    let tsv = corpus_dir().join("expectations.tsv");
    let content = std::fs::read_to_string(&tsv).unwrap_or_else(|e| {
        panic!(
            "corpus introuvable : {} ({e}).\n\
             Un lecteur qui ne lit aucun attendu est vert sur zéro cas.",
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
            5,
            "ligne {} du TSV : {} colonnes au lieu de 5 — le format est \
             `fichier · pass · token · fragment · classe`\n  {line:?}",
            lineno + 1,
            cols.len()
        );
        for (idx, name) in [(1, "pass"), (2, "token"), (4, "classe")] {
            assert!(
                !cols[idx].is_empty(),
                "ligne {} du TSV : colonne `{name}` vide — une colonne vide \
                 porte « {EMPTY} », jamais rien",
                lineno + 1
            );
        }
        assert!(
            PASS_VALUES.contains(&cols[1]),
            "ligne {} du TSV : `pass` = {:?} hors du format de fil {PASS_VALUES:?}",
            lineno + 1,
            cols[1]
        );
        assert!(
            TOKEN_VALUES.contains(&cols[2]),
            "ligne {} du TSV : `token` = {:?} hors du format de fil {TOKEN_VALUES:?}",
            lineno + 1,
            cols[2]
        );
        assert!(
            CLASS_VALUES.contains(&cols[4]),
            "ligne {} du TSV : `classe` = {:?} hors du format de fil {CLASS_VALUES:?}",
            lineno + 1,
            cols[4]
        );
        assert_eq!(
            cols[2] == EMPTY,
            cols[3] == EMPTY,
            "ligne {} du TSV : `token` et `fragment` doivent être « {EMPTY} » \
             ensemble ou renseignés ensemble — le bash n'écrit son \
             `(matched: '…')` que quand il écrit un jeton",
            lineno + 1
        );
        out.push(Expectation {
            file: cols[0].to_string(),
            pass: cols[1].to_string(),
            token: cols[2].to_string(),
            fragment: cols[3].to_string(),
            class: cols[4].to_string(),
        });
    }

    assert!(
        !out.is_empty(),
        "le corpus ne porte aucune ligne — une parité verte sur zéro cas ne \
         prouve rien (mika#2194 vol 3, anti-vacuité 2/3)"
    );
    out
}

/// Anti-vacuité 3/3 : un attendu qui désigne un corps absent est un échec, pas
/// un saut. Une ligne qui ne désigne rien ment avec l'autorité d'un inventaire.
fn read_body(file: &str) -> String {
    let path = corpus_dir().join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "le TSV nomme un corps introuvable : {} ({e}).\n\
             (mika#2194 vol 3, anti-vacuité 3/3)",
            path.display()
        )
    })
}

/// La position du fragment DANS le corps — ce qui prouve au passage que c'est
/// bien une tranche de l'entrée et non le motif de la table.
fn offset_in(body: &str, fragment: &str) -> usize {
    let base = body.as_ptr().addr();
    let frag = fragment.as_ptr().addr();
    assert!(
        frag >= base && frag + fragment.len() <= base + body.len(),
        "le fragment rendu n'est pas une tranche du corps — le lecteur rend le \
         MOTIF au lieu du texte matché, et divergera du `grep -o` du bash au \
         premier corps qui capitalise son match"
    );
    frag - base
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Le fragment touche-t-il une frontière de mot des deux côtés ?
fn at_word_boundary(body: &str, fragment: &str) -> bool {
    let at = offset_in(body, fragment);
    let before = body[..at].chars().next_back();
    let after = body[at + fragment.len()..].chars().next();
    !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
}

fn token_of(p: Paraphrase) -> &'static str {
    p.token()
}

/// La parité ligne à ligne, et le compte de cas EN TANT QU'ASSERTION.
#[test]
fn mika2194_le_lecteur_rust_rend_les_attendus_du_corpus() {
    let expectations = read_expectations();
    let mut cases = 0usize;

    for e in &expectations {
        let body = read_body(&e.file);
        let got = paraphrase(&body, e.arch_pass());

        match (&got, e.token.as_str()) {
            (None, EMPTY) => {}
            (None, expected) => panic!(
                "{} : le lecteur ne rend rien, le corpus attend {expected}",
                e.label()
            ),
            (Some((token, _)), EMPTY) => panic!(
                "{} : le lecteur rend {}, le corpus attend aucun match",
                e.label(),
                token_of(*token)
            ),
            (Some((token, fragment)), expected) => {
                assert_eq!(
                    token_of(*token),
                    expected,
                    "{} : jeton divergent du corpus",
                    e.label()
                );
                assert_eq!(
                    *fragment,
                    e.fragment,
                    "{} : fragment divergent — le bash écrit sur stderr le texte \
                     MATCHÉ, casse de l'entrée comprise",
                    e.label()
                );
                // Le fragment doit être une tranche du corps, pas le motif.
                offset_in(&body, fragment);
            }
        }
        cases += 1;
    }

    // Le compte EST une assertion : un affichage ne rougit pas.
    assert_eq!(
        cases,
        expectations.len(),
        "le lecteur a sauté des lignes du corpus"
    );
    assert!(
        cases >= 15,
        "le corpus n'exerce que {cases} cas — la Definition of Done demande au \
         moins une entrée par classe et par passe, soit bien davantage"
    );
}

/// Assertion auto-nettoyante 1/3 — la tolérance SOUS-CHAÎNE.
///
/// Chaque ligne `substring-fp` matche **au milieu d'un mot** : « fundamentally »,
/// « the dispatcher », « the revised plan », « proceeds », « ungroomed ». Si l'un
/// d'eux se met à matcher sur une frontière de mot, c'est que quelqu'un a
/// resserré le lecteur Rust — un `\b…\b`, un `split_whitespace`, un
/// `starts_with` — **sans toucher au bash**, ce que B1 interdit pendant la
/// migration. Le test échoue plutôt que de laisser la divergence passer.
#[test]
fn mika2194_la_classe_substring_fp_matche_toujours_au_milieu_dun_mot() {
    let expectations = read_expectations();
    let mut population = 0usize;

    for e in expectations.iter().filter(|e| e.class == "substring-fp") {
        let body = read_body(&e.file);
        let (_, fragment) = paraphrase(&body, e.arch_pass()).unwrap_or_else(|| {
            panic!(
                "{} : déclarée `substring-fp` mais ne matche plus rien — \
                 le faux positif a cessé, mets le corpus à jour",
                e.label()
            )
        });
        assert!(
            !at_word_boundary(&body, fragment),
            "{} : le fragment {fragment:?} matche désormais sur une frontière \
             de mot. Le lecteur Rust a été resserré et ne porte plus la \
             tolérance du `grep -oi` du bash (B1). Si le resserrement est \
             voulu, il appartient à un ticket qui en débat — et le bash doit \
             bouger dans le même commit.",
            e.label()
        );
        population += 1;
    }

    assert!(
        population > 0,
        "la classe `substring-fp` est vide — une exception sans population est \
         une exception satisfaite par vacuité"
    );
}

/// Assertion auto-nettoyante 2/3 — l'inertie sur le FRANÇAIS.
///
/// Les deux tables sont entièrement en anglais, les réponses de mika-arch sont
/// en français : le tier 2 ne décide quasiment jamais en production. Le jour où
/// l'un de ces corps rend un jeton, le tier a changé de portée — et
/// `test-dispatch-lib.sh` en dépend déjà pour le corps mika#2641.
#[test]
fn mika2194_la_classe_french_inert_ne_rend_toujours_aucun_jeton() {
    let expectations = read_expectations();
    let mut population = 0usize;

    for e in expectations.iter().filter(|e| e.class == "french-inert") {
        assert_eq!(
            e.token,
            EMPTY,
            "{} : déclarée `french-inert` avec un jeton attendu {:?} — \
             la classe nomme les corps que le tier 2 NE lit pas",
            e.label(),
            e.token
        );
        let body = read_body(&e.file);
        if let Some((token, fragment)) = paraphrase(&body, e.arch_pass()) {
            panic!(
                "{} : rend désormais {} sur {fragment:?}. Le tier 2 n'est plus \
                 inerte sur le français : sa portée a changé, et la décision \
                 d'abandon qu'il pilote en aval (mika#2020) avec.",
                e.label(),
                token_of(token)
            );
        }
        population += 1;
    }

    assert!(
        population > 0,
        "la classe `french-inert` est vide — le cas MODAL de la production n'est \
         plus mesuré par personne"
    );
}

/// Assertion auto-nettoyante 3/3 — rien ne RECOLLE les lignes.
///
/// `grep` est orienté ligne : « human\nreview » ne matche pas « human review ».
/// Un `contains` sur le texte entier ne matcherait pas davantage — aucun motif
/// ne porte de `\n` — donc ce que ces lignes épinglent n'est pas le parcours,
/// c'est l'**absence de normalisation des blancs**. Un `replace('\n', " ")` ou
/// un `split_whitespace().join(" ")` glissé avant le match, précisément le genre
/// d'« harmonisation » qu'un portage serviable ajoute, les ferait matcher là où
/// le bash les laisse passer. Le reste du corpus tient sur une ligne : ce test
/// est la seule chose qui s'en apercevrait.
#[test]
fn mika2194_la_classe_multiline_ne_matche_toujours_pas_a_travers_un_saut_de_ligne() {
    let expectations = read_expectations();
    let mut population = 0usize;

    for e in expectations.iter().filter(|e| e.class == "multiline") {
        let body = read_body(&e.file);
        assert!(
            body.lines().count() > 1,
            "{} : déclarée `multiline` mais tient sur une ligne — \
             la classe ne mesure plus rien",
            e.label()
        );
        if let Some((token, fragment)) = paraphrase(&body, e.arch_pass()) {
            panic!(
                "{} : rend désormais {} sur {fragment:?}. Le lecteur matche à \
                 travers un saut de ligne là où `grep` ne le fait pas — c'est \
                 la signature d'une normalisation des blancs posée avant le \
                 match (`replace('\\n', \" \")`, `split_whitespace().join(\" \")`).",
                e.label(),
                token_of(token)
            );
        }
        population += 1;
    }

    assert!(
        population > 0,
        "la classe `multiline` est vide — la sémantique par ligne n'est plus \
         mesurée par personne"
    );
}

/// Le disque et le TSV décrivent le même corpus.
///
/// Sans ce terme, un corps déposé mais non déclaré n'est exercé par personne, et
/// le répertoire grossit en donnant l'impression du contraire.
#[test]
fn mika2194_le_disque_et_le_tsv_decrivent_le_meme_corpus() {
    let declared: BTreeSet<String> = read_expectations().into_iter().map(|e| e.file).collect();

    let mut on_disk = BTreeSet::new();
    for entry in std::fs::read_dir(corpus_dir()).expect("le répertoire du corpus doit exister") {
        let entry = entry.expect("entrée de répertoire lisible");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".md") {
            on_disk.insert(name);
        }
    }

    assert_eq!(
        on_disk, declared,
        "le disque et `expectations.tsv` divergent — un corps non déclaré n'est \
         exercé par aucun des deux lecteurs, et un attendu sans corps fait \
         échouer l'anti-vacuité 3/3"
    );
}

/// Les deux passes et les quatre classes sont toutes représentées.
///
/// La Definition of Done demande au moins une entrée par classe **et** par
/// passe. Sans cette assertion, supprimer la dernière ligne `second` d'une
/// classe laisserait tout le reste vert.
#[test]
fn mika2194_chaque_classe_est_representee_et_les_deux_passes_le_sont() {
    let expectations = read_expectations();

    for class in CLASS_VALUES {
        assert!(
            expectations.iter().any(|e| e.class == *class),
            "la classe `{class}` n'a aucune entrée dans le corpus"
        );
    }
    for pass in PASS_VALUES {
        assert!(
            expectations.iter().any(|e| e.pass == *pass),
            "la passe `{pass}` n'a aucune entrée dans le corpus"
        );
    }
}
