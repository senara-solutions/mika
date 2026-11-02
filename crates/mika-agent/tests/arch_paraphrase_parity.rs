//! Le lecteur Rust du corpus doré des cinq tiers de disposition (mika#2194,
//! vols 3 et 4).
//!
//! Il lit **le même** fichier d'attendus que le bloc bash
//! « mika#2194 — parité des cinq tiers de disposition » de
//! `skills/bundled/_shared/test-dispatch-lib.sh` :
//! `tests/fixtures/arch_paraphrase_texts/expectations.tsv`. Le bash y fait
//! passer `_parse_disposition` / `_parse_verdict` — les lecteurs de
//! **production** — et ce fichier y fait passer
//! `mika_agent::arch_disposition::decide`, qui n'a aucun appelant. Un attendu
//! modifié d'un seul côté redevient rouge de l'autre.
//!
//! # Pourquoi la parité est une preuve DANS CE VOL, et cessera de l'être au vol 5
//!
//! Tant que le bash décide lui-même ses cinq tiers, les deux lecteurs sont deux
//! implémentations et le corpus les confronte. Au vol 5, `_parse_disposition` et
//! `_parse_verdict` deviennent des enveloppes qui appellent la sous-commande :
//! la parité comparerait alors `decide` à lui-même — verte, et vide. La preuve
//! exige donc un commit *pre-switch* **mergé**, et c'est celui-ci. Les cinq
//! assertions auto-nettoyantes ci-dessous sont écrites pour lui survivre : elles
//! portent sur des propriétés **internes au Rust**, jamais sur une comparaison
//! au bash.
//!
//! # Anti-vacuité : trois termes, parce qu'une parité verte sur zéro cas ment
//!
//! Un lecteur de corpus a trois façons de n'exercer rien tout en passant :
//! le fichier d'attendus manque, il ne porte aucune ligne, ou il nomme un corps
//! qui n'existe pas. Les trois sont des **échecs** ici, et le compte de cas est
//! une assertion — jamais un `println!` que personne ne lit.
//!
//! # Cinq assertions auto-nettoyantes
//!
//! Le corpus déclare cinq classes d'exception — `substring-fp`,
//! `french-inert`, `multiline`, `tier-nominal`, `tier-shadowed`. Chacune nomme un
//! comportement que le bash a aujourd'hui et que B1 interdit de corriger pendant
//! la migration. Le jour où l'une cesse, le test **échoue** au lieu de laisser une
//! classe devenue vide passer pour une classe satisfaite. C'est l'option (a) de la
//! Fire-Disposition du plan : la colonne `classe` du TSV **est** l'allowlist —
//! visible au grep, ligne par ligne — et sa population est non vide par
//! construction.
//!
//! # La colonne `tier` n'a qu'un seul lecteur, et c'est celui-ci
//!
//! Avant la bascule, le bash n'a aucun canal de sortie pour le tier : il rend le
//! jeton sur stdout et de la prose sur stderr. Le lecteur bash valide donc le
//! token, et le fragment quand `tier = 2` ; il ignore la colonne. Ce n'est pas une
//! lacune à combler ici — c'est le canal que le vol 5 ouvre (ligne 2 de stdout).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mika_agent::arch_disposition::{ArchPass, decide};

/// Ce qu'une colonne vide porte. **Jamais une chaîne vide** : `IFS=$'\t' read`
/// fusionne deux tabulations consécutives, et le lecteur bash verrait ses
/// colonnes se décaler là où `split('\t')` ne bouge pas. Les deux lecteurs
/// liraient alors des choses différentes du même octet.
const EMPTY: &str = "-";

/// Format de fil de la colonne `pass`.
const PASS_VALUES: &[&str] = &["first", "second"];

/// Format de fil de la colonne `token`, `-` compris.
const TOKEN_VALUES: &[&str] = &[EMPTY, "READY", "ITERATE", "ESCALATE", "GROOMED"];

/// Format de fil de la colonne `tier`. **Fermé** : un tier inventé est une faute
/// de frappe qui passerait, et le tier est précisément ce que le token ne dit
/// pas — 0b et 1a rendent tous deux `ESCALATE`, 1a et 1b tous deux `GROOMED`.
const TIER_VALUES: &[&str] = &["0", "0b", "1a", "1b", "2", EMPTY];

/// Les quatre tiers de tête, ceux que le vol 4 porte. Le tier 2 existait au
/// vol 3 ; `-` n'est pas un tier mais son absence.
const HEAD_TIERS: &[&str] = &["0", "0b", "1a", "1b"];

/// Format de fil de la colonne `classe`. **Fermé** : une septième classe est
/// une décision de corpus, pas une faute de frappe qui passe.
const CLASS_VALUES: &[&str] = &[
    "nominal",
    "substring-fp",
    "french-inert",
    "multiline",
    "tier-nominal",
    "tier-shadowed",
];

#[derive(Debug)]
struct Expectation {
    file: String,
    pass: String,
    token: String,
    fragment: String,
    tier: String,
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
            6,
            "ligne {} du TSV : {} colonnes au lieu de 6 — le format est \
             `fichier · pass · token · fragment · tier · classe`\n  {line:?}",
            lineno + 1,
            cols.len()
        );
        for (idx, name) in [(1, "pass"), (2, "token"), (4, "tier"), (5, "classe")] {
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
            TIER_VALUES.contains(&cols[4]),
            "ligne {} du TSV : `tier` = {:?} hors du format de fil {TIER_VALUES:?}",
            lineno + 1,
            cols[4]
        );
        assert!(
            CLASS_VALUES.contains(&cols[5]),
            "ligne {} du TSV : `classe` = {:?} hors du format de fil {CLASS_VALUES:?}",
            lineno + 1,
            cols[5]
        );
        // Invariant de corpus 1/2 : SEUL le tier 2 produit un fragment. Le bash
        // n'écrit son `(matched: '…')` que depuis sa branche fuzzy ; les quatre
        // tiers de tête décident sur un littéral et n'ont rien à citer.
        assert_eq!(
            cols[3] != EMPTY,
            cols[4] == "2",
            "ligne {} du TSV : un fragment est renseigné si et seulement si \
             `tier` vaut « 2 » — les quatre tiers de tête n'en produisent aucun",
            lineno + 1
        );
        // Invariant de corpus 2/2 : un token absent a DEUX causes, et elles ne se
        // lisent pas pareil (mika#2205). `tier = 0` dit que le moteur a retenu la
        // disposition ; `tier = -` dit qu'aucun tier n'a décidé. La colonne token
        // vaut « - » pour les deux, donc c'est la colonne `tier` qui les sépare —
        // et aucun autre tier ne peut rendre un token vide.
        assert_eq!(
            cols[2] == EMPTY,
            cols[4] == "0" || cols[4] == EMPTY,
            "ligne {} du TSV : `token` = « {EMPTY} » n'est cohérent qu'avec \
             `tier` ∈ {{« 0 », « {EMPTY} »}} — un tier qui décide rend un jeton",
            lineno + 1
        );
        out.push(Expectation {
            file: cols[0].to_string(),
            pass: cols[1].to_string(),
            token: cols[2].to_string(),
            fragment: cols[3].to_string(),
            tier: cols[4].to_string(),
            class: cols[5].to_string(),
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

/// Le tier que le NOM du corps désigne : `tier-<X>-…` → `<X>`.
///
/// C'est la convention de nommage du corpus, et elle est porteuse : un corps de
/// la classe `tier-shadowed` nomme le tier dont il porte le littéral **sans**
/// que ce tier décide. Sans cette convention, « quel tier est éclipsé ici ? »
/// n'aurait pas de réponse mécanique et la classe ne mesurerait que la colonne
/// `tier`, que le test de parité épingle déjà.
fn named_tier(file: &str) -> &str {
    let rest = file.strip_prefix("tier-").unwrap_or_else(|| {
        panic!(
            "{file} : un corps des classes `tier-*` se nomme `tier-<tier>-…` — \
             le nom est ce qui dit de quel tier il parle"
        )
    });
    rest.split('-')
        .next()
        .expect("`split` rend toujours au moins un élément")
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

/// La parité ligne à ligne, et le compte de cas EN TANT QU'ASSERTION.
#[test]
fn mika2194_le_lecteur_rust_rend_les_attendus_du_corpus() {
    let expectations = read_expectations();
    let mut cases = 0usize;

    for e in &expectations {
        let body = read_body(&e.file);
        let got = decide(&body, e.arch_pass());

        match (got.token, e.token.as_str()) {
            (None, EMPTY) => {}
            (None, expected) => panic!(
                "{} : le lecteur ne rend rien, le corpus attend {expected}",
                e.label()
            ),
            (Some(token), EMPTY) => panic!(
                "{} : le lecteur rend {token}, le corpus attend aucun match",
                e.label()
            ),
            (Some(token), expected) => {
                assert_eq!(token, expected, "{} : jeton divergent du corpus", e.label());
            }
        }

        // Le tier, et c'est ce que le token ne dit pas : 0b et 1a rendent tous
        // deux ESCALATE, 1a et 1b tous deux GROOMED. Un tier qui décide à la
        // place d'un autre laisse la colonne token intacte.
        assert_eq!(
            got.tier.as_str(),
            e.tier,
            "{} : tier divergent du corpus — l'ORDRE des cinq tiers a bougé, \
             et le jeton ne s'en aperçoit pas",
            e.label()
        );

        if e.tier == "2" {
            let fragment = got.fragment.unwrap_or_else(|| {
                panic!(
                    "{} : décidée au tier 2 sans fragment — le bash écrit son \
                     `(matched: '…')` sur chaque décision fuzzy",
                    e.label()
                )
            });
            assert_eq!(
                fragment,
                e.fragment,
                "{} : fragment divergent — le bash écrit sur stderr le texte \
                 MATCHÉ, casse de l'entrée comprise",
                e.label()
            );
            // Le fragment doit être une tranche du corps, pas le motif.
            offset_in(&body, fragment);
        } else {
            assert!(
                got.fragment.is_none(),
                "{} : décidée au tier {} avec un fragment {:?} — seul le tier 2 \
                 en produit un, les quatre tiers de tête décident sur un littéral",
                e.label(),
                got.tier.as_str(),
                got.fragment
            );
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
        cases >= 27,
        "le corpus n'exerce que {cases} cas — la Definition of Done demande au \
         moins une entrée par classe et par passe, et un cas positif ET négatif \
         pour chacun des quatre tiers de tête"
    );
}

/// Assertion auto-nettoyante 1/5 — la tolérance SOUS-CHAÎNE.
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
        let fragment = decide(&body, e.arch_pass()).fragment.unwrap_or_else(|| {
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

/// Assertion auto-nettoyante 2/5 — l'inertie sur le FRANÇAIS.
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
        let got = decide(&body, e.arch_pass());
        if let Some(token) = got.token {
            panic!(
                "{} : rend désormais {token} au tier {}. Le tier 2 n'est plus \
                 inerte sur le français : sa portée a changé, et la décision \
                 d'abandon qu'il pilote en aval (mika#2020) avec.",
                e.label(),
                got.tier.as_str()
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

/// Assertion auto-nettoyante 3/5 — rien ne RECOLLE les lignes.
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
        let got = decide(&body, e.arch_pass());
        if let Some(token) = got.token {
            panic!(
                "{} : rend désormais {token} au tier {}. Le lecteur matche à \
                 travers un saut de ligne là où `grep` ne le fait pas — c'est \
                 la signature d'une normalisation des blancs posée avant le \
                 match (`replace('\\n', \" \")`, `split_whitespace().join(\" \")`).",
                e.label(),
                got.tier.as_str()
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

/// Assertion auto-nettoyante 4/5 — chacun des quatre tiers de tête TIRE au
/// moins une fois.
///
/// La classe `tier-nominal` nomme les cas positifs : le tier que le corps
/// désigne est celui qui décide. Deux propriétés sont épinglées ici, et la
/// seconde est celle qui compte : non seulement chaque ligne décide au tier
/// qu'elle déclare, mais **les quatre tiers de tête sont couverts**. Supprimer
/// la dernière ligne du tier 0b laisserait sinon tout le reste vert, et un
/// portage qui ne l'implémenterait plus passerait — les tiers 0b et 1a rendent
/// tous deux `ESCALATE`.
#[test]
fn mika2194_la_classe_tier_nominal_couvre_les_quatre_tiers_de_tete() {
    let expectations = read_expectations();
    let mut covered = BTreeSet::new();

    for e in expectations.iter().filter(|e| e.class == "tier-nominal") {
        assert!(
            HEAD_TIERS.contains(&e.tier.as_str()),
            "{} : déclarée `tier-nominal` au tier {:?} — la classe nomme les cas \
             positifs des quatre tiers de tête {HEAD_TIERS:?}, pas le tier 2 \
             (classe `nominal`) ni l'absence de décision",
            e.label(),
            e.tier
        );
        let body = read_body(&e.file);
        let got = decide(&body, e.arch_pass());
        assert_eq!(
            got.tier.as_str(),
            e.tier,
            "{} : déclarée `tier-nominal` au tier {:?}, décidée au tier {:?} — \
             le tier nommé par ce corps ne tire plus",
            e.label(),
            e.tier,
            got.tier.as_str()
        );
        covered.insert(e.tier.clone());
    }

    let expected: BTreeSet<String> = HEAD_TIERS.iter().map(|t| (*t).to_string()).collect();
    assert_eq!(
        covered, expected,
        "la classe `tier-nominal` ne couvre pas les quatre tiers de tête — un \
         tier sans cas positif n'est implémenté par personne, et le jeton seul \
         ne s'en aperçoit pas"
    );
}

/// Assertion auto-nettoyante 5/5 — un littéral PRÉSENT ne fait pas décider son
/// tier.
///
/// La classe `tier-shadowed` nomme les cas négatifs : le corps porte le littéral
/// d'un tier — nommé par son propre nom de fichier, `tier-<X>-…` — et c'est un
/// autre tier, ou aucun, qui décide. Le marqueur cité au milieu d'une ligne ne
/// supprime rien, la prose qui cite le moteur sans la forme complète de l'ancre
/// n'escalade pas, un libellé hors vocabulaire ne rend rien, et un `ITERATE` en
/// seconde passe tombe au tier 2 par la lacune DÉLIBÉRÉE de la table de reprise.
/// Les quatre doivent être couverts : c'est la moitié négative que le plan exige
/// à côté de chaque positif, et sans elle un prédicat trop large passe.
#[test]
fn mika2194_la_classe_tier_shadowed_couvre_les_quatre_tiers_de_tete() {
    let expectations = read_expectations();
    let mut covered = BTreeSet::new();

    for e in expectations.iter().filter(|e| e.class == "tier-shadowed") {
        let shadowed = named_tier(&e.file);
        assert!(
            HEAD_TIERS.contains(&shadowed),
            "{} : le nom désigne le tier {shadowed:?}, hors des quatre tiers de \
             tête {HEAD_TIERS:?} — un corps `tier-shadowed` nomme le tier qu'il \
             éclipse",
            e.label()
        );
        let body = read_body(&e.file);
        let got = decide(&body, e.arch_pass());
        assert_ne!(
            got.tier.as_str(),
            shadowed,
            "{} : le tier {shadowed:?} décide désormais sur ce corps. C'est le \
             cas NÉGATIF de ce tier : son littéral y est présent et il ne doit \
             pas tirer. Le prédicat du tier s'est élargi — ou le corps a changé.",
            e.label()
        );
        covered.insert(shadowed.to_string());
    }

    let expected: BTreeSet<String> = HEAD_TIERS.iter().map(|t| (*t).to_string()).collect();
    assert_eq!(
        covered, expected,
        "la classe `tier-shadowed` ne couvre pas les quatre tiers de tête — un \
         tier sans cas négatif n'a pas de borne supérieure mesurée"
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

/// Les deux passes et les six classes sont toutes représentées.
///
/// La Definition of Done demande au moins une entrée par classe **et** par
/// passe. Sans cette assertion, supprimer la dernière ligne `second` d'une
/// classe laisserait tout le reste vert. Et aucune classe n'est introduite vide
/// (mika#2201) : les deux que le vol 4 ajoute passent par ici le jour de leur
/// première ligne.
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
