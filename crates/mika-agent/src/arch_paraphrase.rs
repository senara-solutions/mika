//! Le tier 2 de paraphrase des réponses d'architecte, porté en Rust (mika#2194, vol 3).
//!
//! # Ce module n'a AUCUN appelant de production, et c'est la propriété du vol
//!
//! `skills/bundled/_shared/dispatch-lib.sh::_parse_disposition_fuzzy` et
//! `::_parse_verdict_fuzzy` restent les lecteurs de production, mot pour mot.
//! Ce module est l'autre moitié d'un **corpus doré** : un répertoire de corps,
//! un fichier d'attendus, et **deux** lecteurs qui le lisent — le bash dans
//! `test-dispatch-lib.sh`, le Rust dans
//! `crates/mika-agent/tests/arch_paraphrase_parity.rs`.
//!
//! C'est la méthode du corps de mika#2194 — *« strangler, jamais big-bang —
//! chaque bascule est précédée de TESTS DORÉS »* — et c'est la seule fenêtre où
//! la parité est une **preuve** : après la bascule du vol suivant, le bash
//! appellera le Rust, et un lecteur comparé à lui-même est toujours d'accord
//! avec lui-même.
//!
//! # Les trois tolérances sont PORTÉES, pas choisies — et B1 interdit de les resserrer
//!
//! Le match est **insensible à la casse**, **en sous-chaîne** et **par ligne**.
//! Aucune des trois n'est un détail d'implémentation : chacune est la tolérance
//! du `grep -oi "$pat"` du bash, et la borne B1 de
//! `docs/architecture/dispatch-lib-migration.md` § 3 interdit de resserrer,
//! traduire ou « harmoniser » une tolérance pendant la migration. Un
//! resserrement de ce tier décide d'un **abandon de ticket** en aval
//! (mika#2020) : il appartient à un ticket qui en débat, jamais à un portage.
//!
//! - **Sous-chaîne.** `grep -oi "fundamental"` matche dans « fundamentally
//!   sound », donc un éloge est lu `ESCALATE`. Le code le sait à moitié — le
//!   commentaire du bash dit avoir remplacé `ship` par `ship it` à cause de
//!   « relationship » — et les quatre motifs restants (`fundamental`,
//!   `dispatch`, `revise`, `proceed`) n'ont pas eu ce traitement. Le corpus les
//!   épingle en classe `substring-fp` au lieu de les corriger.
//! - **Casse.** `to_ascii_lowercase` et non `to_lowercase` : `grep -i` en locale
//!   C plie l'ASCII et rien d'autre, et surtout le pliage ASCII **préserve la
//!   longueur en octets**, ce dont dépend le découpage du fragment ci-dessous.
//!   `'İ'.to_lowercase()` rend deux `char` et décalerait tous les index.
//! - **Par ligne.** `grep` est orienté ligne : un `human\nreview` ne matche
//!   **pas** le motif `human review`. Aucun motif des deux tables ne contient de
//!   `\n`, donc le parcours ligne à ligne ci-dessous rend exactement le même
//!   ensemble qu'un `find` sur le texte entier ; il est écrit ainsi parce que
//!   c'est la forme du bash, pas parce qu'il changerait un verdict. Ce que la
//!   classe `multiline` du corpus épingle n'est donc pas le parcours, c'est
//!   l'**absence de normalisation des blancs** : un `text.replace('\n', " ")`
//!   ou un `split_whitespace().join(" ")` glissé avant le match — le genre
//!   d'« harmonisation » qu'un portage serviable ajoute — ferait matcher ces
//!   corps que le bash laisse passer. C'est exactement ce que B1 interdit.
//!
//! # Le fragment est une tranche du TEXTE, jamais le motif
//!
//! Le bash écrit sur stderr `… (matched: '$snippet')`, et `$snippet` sort de
//! `grep -o` : c'est le texte **tel qu'il est écrit dans l'entrée**, casse
//! comprise. Un lecteur qui rendrait le motif (`&'static str`, minuscule) serait
//! d'accord avec le bash sur un corps en minuscules et divergerait au premier
//! corps qui capitalise. Le fragment rendu ici est donc une sous-tranche de
//! `text`, et le lecteur de corpus le vérifie par son adresse.
//!
//! # Priorité : le plus conservateur gagne
//!
//! `ESCALATE > ITERATE > READY` en première passe, `ESCALATE > GROOMED` en
//! seconde — la raison du bash, reprise telle quelle. Les tables ci-dessous sont
//! **dans l'ordre du bash**, parce que l'ordre décide quel fragment est rendu
//! quand deux motifs d'une même classe matchent, et parce que les classes y sont
//! déjà rangées par priorité : parcourir la table dans l'ordre et rendre au
//! premier match est exactement la désambiguïsation du bash.

/// Quelle passe d'architecte a produit le texte, donc quelle table l'interroge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchPass {
    /// `mika-arch-groom-ticket` — `_parse_disposition_fuzzy`.
    First,
    /// `mika-arch-second-review` — `_parse_verdict_fuzzy`.
    Second,
}

/// Le vocabulaire fermé des jetons que ce tier peut rendre (mika#2201, classe A).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paraphrase {
    Ready,
    Iterate,
    Escalate,
    Groomed,
}

impl Paraphrase {
    /// Le jeton canonique, tel que le bash l'écrit sur stdout.
    pub fn token(self) -> &'static str {
        match self {
            Paraphrase::Ready => "READY",
            Paraphrase::Iterate => "ITERATE",
            Paraphrase::Escalate => "ESCALATE",
            Paraphrase::Groomed => "GROOMED",
        }
    }
}

/// Première passe — transcription littérale de `dispatch-lib.sh:8833-8855`.
///
/// Les trois classes dans l'ordre du bash, qui est aussi l'ordre de priorité.
const FIRST_PASS_TABLE: &[(Paraphrase, &[&str])] = &[
    (
        Paraphrase::Escalate,
        &[
            "escalate",
            "human review",
            "cannot proceed",
            "fundamental",
            "out of scope for",
        ],
    ),
    (
        Paraphrase::Iterate,
        &[
            "needs revision",
            "another pass",
            "revise",
            "address the following",
            "concerns that require",
        ],
    ),
    // « affirmative forward-motion signals only — no negated-absence », dit le bash.
    (
        Paraphrase::Ready,
        &[
            "proceed",
            "ship it",
            "dispatch",
            "good to go",
            "plan is clean",
        ],
    ),
];

/// Seconde passe — transcription littérale de `dispatch-lib.sh:8882-8897`.
///
/// `ship it` et non `ship` : le bash porte au site la raison (« avoids substring
/// false positives in "relationship", "ownership", "leadership" »). C'est le
/// correctif partiel de la classe `substring-fp` — il n'a jamais été appliqué
/// aux quatre motifs de la première passe.
const SECOND_PASS_TABLE: &[(Paraphrase, &[&str])] = &[
    (
        Paraphrase::Escalate,
        &[
            "escalate",
            "cannot approve",
            "human review needed",
            "fundamental issues remain",
        ],
    ),
    (
        Paraphrase::Groomed,
        &[
            "groomed",
            "approved",
            "plan is ready",
            "ship it",
            "no remaining concerns",
        ],
    ),
];

/// Le premier match d'un des motifs, dans l'ordre des motifs puis des lignes.
///
/// C'est `for pat in …; do grep -oi "$pat" | head -1 … break; done` : le motif
/// est testé contre le texte **entier** avant de passer au suivant, et `head -1`
/// retient la première occurrence dans l'ordre de lecture. Donc l'ordre des
/// motifs domine la position dans le texte, et ce n'est pas un hasard de
/// l'écriture : c'est ce qui décide du fragment écrit sur stderr.
///
/// Le texte est parcouru **ligne à ligne**, comme `grep`. Aucun motif des deux
/// tables ne contient de `\n`, donc cela rend le même ensemble qu'un `find` sur
/// le texte entier ; ce qui compte est ce que la forme **interdit** d'ajouter —
/// une normalisation des blancs, qui ferait matcher à travers un saut de ligne
/// ce que le bash laisse passer (classe `multiline` du corpus).
fn first_match<'a>(text: &'a str, patterns: &[&str]) -> Option<&'a str> {
    for pat in patterns {
        for line in text.lines() {
            // Le pliage ASCII préserve la longueur en octets, donc l'index rendu
            // par `find` sur la copie minuscule est valide dans `line`.
            if let Some(at) = line.to_ascii_lowercase().find(pat) {
                return Some(&line[at..at + pat.len()]);
            }
        }
    }
    None
}

/// Le jeton qu'une réponse d'architecte paraphrasée porte, et le fragment qui l'a décidé.
///
/// `None` quand aucun motif ne matche — le bash n'écrit alors rien sur stdout, et
/// l'appelant retombe sur son propre défaut. C'est le cas **modal** sur ce dépôt :
/// les deux tables sont entièrement en anglais et les réponses de mika-arch sont
/// en français (classe `french-inert` du corpus).
pub fn paraphrase(text: &str, pass: ArchPass) -> Option<(Paraphrase, &str)> {
    let table = match pass {
        ArchPass::First => FIRST_PASS_TABLE,
        ArchPass::Second => SECOND_PASS_TABLE,
    };
    for (token, patterns) in table {
        if let Some(fragment) = first_match(text, patterns) {
            return Some((*token, fragment));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le plus conservateur gagne, première passe.
    #[test]
    fn mika2194_la_priorite_de_la_premiere_passe_est_escalate_iterate_ready() {
        let text = "The plan is clean and I would ship it, but it needs revision \
                    and it needs human review first.";
        assert_eq!(
            paraphrase(text, ArchPass::First),
            Some((Paraphrase::Escalate, "human review"))
        );

        let text = "The plan is clean and I would ship it, but it needs revision first.";
        assert_eq!(
            paraphrase(text, ArchPass::First),
            Some((Paraphrase::Iterate, "needs revision"))
        );
    }

    /// Le plus conservateur gagne, seconde passe.
    #[test]
    fn mika2194_la_priorite_de_la_seconde_passe_est_escalate_groomed() {
        let text = "Mostly approved, but I cannot approve section 4.";
        assert_eq!(
            paraphrase(text, ArchPass::Second),
            Some((Paraphrase::Escalate, "cannot approve"))
        );
    }

    /// L'ordre des MOTIFS domine la position dans le texte.
    #[test]
    fn mika2194_lordre_des_motifs_decide_du_fragment() {
        // « ship it » est en seconde position de la table READY, « proceed » en
        // première : c'est `proceed` qui est rendu bien qu'il vienne après.
        let text = "Ship it, and the loop may then proceed.";
        assert_eq!(
            paraphrase(text, ArchPass::First),
            Some((Paraphrase::Ready, "proceed"))
        );
    }

    /// Le fragment porte la casse de l'ENTRÉE, comme `grep -o`.
    #[test]
    fn mika2194_le_fragment_porte_la_casse_de_lentree() {
        let (token, fragment) =
            paraphrase("ESCALATE this one.", ArchPass::First).expect("un match");
        assert_eq!(token, Paraphrase::Escalate);
        assert_eq!(fragment, "ESCALATE");
    }

    /// Un motif à espace coupé par un `\n` ne matche pas — et rien ne recolle.
    #[test]
    fn mika2194_un_motif_coupe_par_un_retour_a_la_ligne_ne_matche_pas() {
        assert_eq!(
            paraphrase("it needs human\nreview first.", ArchPass::First),
            None
        );
        // Contrôle positif : la même phrase sur une ligne matche.
        assert_eq!(
            paraphrase("it needs human review first.", ArchPass::First),
            Some((Paraphrase::Escalate, "human review"))
        );
    }

    /// Le match est en sous-chaîne — mesuré, pas corrigé (B1).
    #[test]
    fn mika2194_le_match_est_en_sous_chaine_et_cest_la_tolerance_portee() {
        assert_eq!(
            paraphrase("The plan is fundamentally sound.", ArchPass::First),
            Some((Paraphrase::Escalate, "fundamental"))
        );
        assert_eq!(
            paraphrase("The dispatcher is untouched.", ArchPass::First),
            Some((Paraphrase::Ready, "dispatch"))
        );
    }

    /// Contrôle négatif : un texte sans aucun motif ne rend rien.
    #[test]
    fn mika2194_un_texte_sans_motif_ne_rend_rien() {
        assert_eq!(
            paraphrase("Je relis le plan révisé.", ArchPass::First),
            None
        );
        assert_eq!(
            paraphrase("Je relis le plan révisé.", ArchPass::Second),
            None
        );
    }

    /// Les motifs sont en ASCII minuscule, condition du pliage à longueur constante.
    #[test]
    fn mika2194_les_motifs_sont_en_ascii_minuscule() {
        for (_, patterns) in FIRST_PASS_TABLE.iter().chain(SECOND_PASS_TABLE.iter()) {
            for pat in *patterns {
                assert!(
                    pat.is_ascii() && *pat == pat.to_ascii_lowercase(),
                    "motif non-ASCII ou non minuscule : {pat:?} — \
                     `first_match` plie l'entrée en ASCII minuscule, donc un motif \
                     qui n'est pas déjà sous cette forme ne matcherait jamais"
                );
            }
        }
    }
}
