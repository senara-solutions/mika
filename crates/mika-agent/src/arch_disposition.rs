//! L'échelle des cinq tiers d'une réponse d'architecte, portée en Rust (mika#2194, vol 4).
//!
//! # Un module VOISIN de `arch_paraphrase`, et c'est une contrainte, pas un goût
//!
//! Le tier 2 — les deux tables de paraphrase — vit dans
//! [`crate::arch_paraphrase`] depuis le vol 3, avec son corpus doré. La borne B1
//! de `docs/architecture/dispatch-lib-migration.md` § 3 interdit de resserrer,
//! traduire ou « harmoniser » une tolérance pendant la migration : ce module
//! **appelle** `arch_paraphrase::paraphrase` et ne touche à aucune de ses deux
//! tables. Les tiers 0, 0b, 1a et 1b, plus l'entrée unique, vivent ici.
//!
//! # Ce module n'a AUCUN appelant de production, et c'est la propriété du vol
//!
//! `skills/bundled/_shared/dispatch-lib.sh::_parse_disposition` et
//! `::_parse_verdict` restent les lecteurs de production, mot pour mot. Ce
//! module est l'autre moitié d'un **corpus doré** : un répertoire de corps, un
//! fichier d'attendus, et **deux** lecteurs qui le lisent — le bash dans
//! `test-dispatch-lib.sh`, le Rust dans
//! `crates/mika-agent/tests/arch_paraphrase_parity.rs`.
//!
//! C'est la seule fenêtre où la parité est une **preuve**. Au vol suivant les
//! deux fonctions bash deviennent des enveloppes sur une sous-commande `mika`
//! qui appelle [`decide`] : le lecteur bash et le lecteur Rust seront alors le
//! même lecteur, et un lecteur comparé à lui-même est toujours d'accord avec
//! lui-même. D'où l'ordre imposé par le plan : **le commit *pre-switch* se
//! merge d'abord**, sinon la parité du vol suivant est une tautologie.
//!
//! # L'ORDRE DES TIERS EST PORTEUR, et chaque position a sa raison
//!
//! L'ordre est celui du bash — 0, 0b, 1a, 1b, 2 — et aucune des quatre
//! positions n'est stylistique :
//!
//! - **Tier 0 avant tout.** Le moteur retire une disposition qu'il a refusé de
//!   laisser passer et met un marqueur à sa place (mika#2037). Après le tier 1a
//!   le marqueur serait encore honoré, mais après le tier 2 il ne le serait
//!   plus : la paraphrase matche « proceed », « good to go », « plan is clean »
//!   n'importe où dans le texte, donc une réponse dont on a seulement **retiré**
//!   la ligne de disposition rendrait `READY` tirée de son propre corps. Le tier
//!   0 passe en premier pour que le refus du moteur ne soit pas défait en aval.
//! - **Tier 0b avant le tier 1a.** Le refus du moteur est un `ESCALATE` avec une
//!   cause (mika#2338), et le tier 1a grepe le **premier** `Disposition:` du
//!   texte, sans ancre : un `READY` que le modèle a cité en ligne serait lu
//!   avant l'`ESCALATE` que le moteur a ajouté à la fin.
//! - **Tier 1b après le tier 1a.** C'est la tolérance de reprise de session
//!   (mika#1421 v3) : la forme de l'**autre** passe, acceptée faute de mieux.
//!   Elle ne doit jamais devancer la forme canonique de la passe courante.
//! - **Tier 2 en dernier.** C'est le repli, et le seul tier qui rend un
//!   fragment.
//!
//! # Deux populations distinctes sous « aucun jeton »
//!
//! [`decide`] rend `token: None` dans **deux** cas qui ne se lisent pas pareil :
//! le tier 0 (le moteur a retiré la disposition — une absence **décidée**) et le
//! tier `-` (aucun tier n'a matché — un UNPARSED). D'où `Tier::Withheld`
//! distinct de `Tier::None` : deux populations sous un seul nom ne sont pas
//! soustractibles (classe mika#2205), et c'est exactement la différence entre
//! « le moteur a refusé » et « personne n'a rien trouvé ».
//!
//! # Le littéral du marqueur est LU, jamais recopié
//!
//! Le tier 0 lit [`crate::agent_loop::DISPOSITION_WITHHELD_MARKER`]. Une copie
//! du littéral dans ce fichier créerait un **second site de match** pour un
//! jeton de classe B déclaré à un seul endroit, ce que
//! `canonical_tokens::mika2201_every_match_site_is_declared` refuse — et elle
//! rouvrirait la dérive que `test-dispatch-lib.sh` garde depuis mika#2037.

use std::sync::LazyLock;

use regex::Regex;

use crate::arch_paraphrase;

/// Quelle passe d'architecte a produit le texte, donc quelles tables l'interrogent.
///
/// Ré-exporté depuis [`crate::arch_paraphrase`] : c'est le même axe, et deux
/// énumérations jumelles seraient deux vocabulaires à tenir d'accord.
pub use crate::arch_paraphrase::ArchPass;

/// Quel tier a décidé — le vocabulaire FERMÉ de la ligne 2 du canal de sortie.
///
/// `Withheld` et `None` rendent tous deux `token: None` et ne se lisent pas
/// pareil : voir la section « Deux populations distinctes » de l'en-tête.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Tier 0 — le moteur a retiré la disposition (mika#2037). Aucun jeton.
    Withheld,
    /// Tier 0b — ligne d'escalade du moteur (mika#2338). `ESCALATE`.
    EngineEscalation,
    /// Tier 1a — la forme canonique littérale de la passe courante.
    Literal,
    /// Tier 1b — la forme de l'autre passe, tolérance de reprise (mika#1421 v3).
    CarryOver,
    /// Tier 2 — paraphrase ([`crate::arch_paraphrase`]). Le seul tier à fragment.
    Paraphrase,
    /// Aucun tier n'a matché — UNPARSED.
    None,
}

impl Tier {
    /// Le nom du tier tel que le corpus doré l'écrit dans sa sixième colonne.
    ///
    /// `"-"` pour [`Tier::None`], parce que **une colonne vide porte `-`, jamais
    /// rien** : `IFS=$'\t' read` fusionne deux tabulations consécutives et
    /// décale en silence toutes les colonnes suivantes, là où le `split('\t')`
    /// du Rust ne le fait pas.
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Withheld => "0",
            Tier::EngineEscalation => "0b",
            Tier::Literal => "1a",
            Tier::CarryOver => "1b",
            Tier::Paraphrase => "2",
            Tier::None => "-",
        }
    }
}

/// Ce qu'une réponse d'architecte porte : un jeton, le tier qui l'a décidé, et
/// le fragment décisif quand il y en a un.
///
/// `fragment` n'est `Some` **que** pour [`Tier::Paraphrase`], et c'est alors une
/// **tranche de l'entrée**, jamais le motif : le bash écrit sur stderr
/// `… (matched: '$snippet')` où `$snippet` sort de `grep -o`, c'est-à-dire le
/// texte tel qu'il est écrit dans la réponse, casse comprise. Les tiers 0 à 1b
/// n'écrivent pas de fragment — leur prose de stderr diffère par tier et n'est
/// pas un contrat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision<'a> {
    /// `READY` | `ITERATE` | `ESCALATE` | `GROOMED`, ou `None`.
    pub token: Option<&'static str>,
    /// Le tier qui a décidé.
    pub tier: Tier,
    /// Le fragment décisif — tier 2 uniquement, et c'est une tranche de l'entrée.
    pub fragment: Option<&'a str>,
}

/// L'étiquette de protocole de la première passe.
const DISPOSITION_LABEL: &str = "Disposition:";

/// L'étiquette de protocole de la seconde passe.
const VERDICT_LABEL: &str = "Verdict:";

/// Les jetons qui peuvent suivre `Disposition:` — l'ordre du `grep -oE` du bash.
///
/// L'ordre n'arbitre rien ici : les trois ont des premières lettres distinctes,
/// donc deux d'entre eux ne peuvent pas matcher à la même position. Il est celui
/// du bash pour qu'une relecture côte à côte soit possible.
const DISPOSITION_TOKENS: &[&str] = &["READY", "ITERATE", "ESCALATE"];

/// Les jetons qui peuvent suivre `Verdict:` — l'ordre du `grep -oE` du bash.
const VERDICT_TOKENS: &[&str] = &["GROOMED", "ESCALATE"];

/// L'ancre COMPLÈTE de la ligne d'escalade du moteur (mika#2338).
///
/// Transcription de `dispatch-lib.sh::_engine_escalation_line`, motif pour
/// motif. La forme entière est porteuse : le moteur numérote ses constats
/// (`F1:`, `F2:`…), les marque `(BLOCKING)`, les attribue à `[mika-engine]` et
/// nomme la famille `review-anchor:`. Une prose qui cite `[mika-engine]` sans
/// cette forme-là ne doit pas déclencher le tier — c'est le contrôle négatif du
/// corpus.
///
/// `^` ancre en tête de ligne, et ce module interroge le motif **ligne à
/// ligne**, comme `grep -m1 -E`. Écrire `(?m)` et interroger le texte entier
/// serait équivalent ici, mais `[[:space:]]` du moteur de regex Rust contient
/// `\n` — ce que celui de `grep` ne peut pas contenir, puisque `grep` ne voit
/// jamais de `\n` dans une ligne. Le parcours ligne à ligne rend cette
/// différence inatteignable au lieu de la laisser dépendre d'un drapeau.
static ENGINE_ESCALATION_ANCHOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[[:space:]]*F[0-9]+: \(BLOCKING\) \[mika-engine\] review-anchor:")
        .expect("l'ancre d'escalade du moteur doit compiler")
});

/// Le marqueur est-il en TÊTE D'UNE LIGNE du texte ?
///
/// Transcription des deux bras du `case` du bash :
///
/// ```text
/// "Disposition-Withheld: REVIEW-ANCHOR-MISSING"*|*"
/// Disposition-Withheld: REVIEW-ANCHOR-MISSING"*)
/// ```
///
/// soit « le texte commence par le marqueur » **ou** « le texte contient un
/// saut de ligne immédiatement suivi du marqueur ». L'union des deux est
/// exactement « il existe une ligne qui commence par le marqueur », parce que
/// toute ligne commence à l'offset 0 ou juste après un `\n` — d'où la forme
/// ci-dessous. (`str::lines` retire un `\r` de **fin** de ligne ; le marqueur
/// n'en contient pas, donc cela ne peut ni créer ni détruire un préfixe.)
///
/// **En tête de ligne, pas n'importe où dans le texte** : une réponse qui CITE
/// le marqueur au milieu d'une phrase tout en portant un vrai `ITERATE` ne doit
/// pas être supprimée. Seul le moteur l'écrit comme une ligne à lui. C'est le
/// contrôle négatif `tier-shadowed` du corpus.
fn at_line_start(text: &str, marker: &str) -> bool {
    text.lines().any(|line| line.starts_with(marker))
}

/// La première ligne d'escalade du moteur, s'il y en a une.
///
/// `grep -m1 -E '^…'` : la première ligne qui matche, et rien de plus.
fn engine_escalation_line(text: &str) -> Option<&str> {
    text.lines()
        .find(|line| ENGINE_ESCALATION_ANCHOR.is_match(line))
}

/// Le premier jeton qui suit `label` dans le texte, à la manière du bash.
///
/// Transcription de
/// `grep -oE 'Disposition:[[:space:]]*(READY|ITERATE|ESCALATE)' | grep -oE '(READY|…)' | head -1`.
/// Trois propriétés de cette chaîne, et chacune est portée ici :
///
/// - **Par ligne.** `grep` est orienté ligne : un `Disposition:` suivi du jeton
///   à la ligne d'après ne matche pas, parce que `[[:space:]]` ne peut pas être
///   un `\n` dans une ligne. Un `(?s)` ou une normalisation des blancs ferait
///   matcher ce que le bash laisse passer — précisément ce que B1 interdit.
/// - **Première ligne, puis position la plus à gauche sur cette ligne.**
///   `grep -o` rend toutes les occurrences dans l'ordre de lecture et `head -1`
///   retient la première. Donc la position dans le texte décide, et jamais
///   l'ordre de la table.
/// - **Sensible à la casse.** Aucun `-i` sur ces deux `grep` : `disposition:`
///   en minuscules ne matche pas. C'est la tolérance telle qu'elle est, non
///   telle qu'on la voudrait.
///
/// Rend le jeton **canonique** (`&'static str`, celui de la table) et non une
/// tranche de l'entrée : la comparaison est exacte, donc les deux sont le même
/// texte, et les tiers 1a/1b n'écrivent pas de fragment de toute façon.
fn first_labelled_token(text: &str, label: &str, tokens: &[&'static str]) -> Option<&'static str> {
    for line in text.lines() {
        for (at, _) in line.match_indices(label) {
            let rest = &line[at + label.len()..];
            let rest = rest.trim_start_matches(is_posix_space);
            if let Some(token) = tokens.iter().find(|t| rest.starts_with(**t)) {
                return Some(token);
            }
        }
    }
    None
}

/// `[[:space:]]` de POSIX en locale C, et pas l'approximation de Rust.
///
/// `char::is_ascii_whitespace` **omet la tabulation verticale** (`\v`, 0x0B),
/// que `[[:space:]]` inclut. Le corpus ne l'exerce pas — aucun architecte n'écrit
/// de `\v` — donc l'écart ne se verrait sur aucun test et se lirait quand même
/// « portage byte-exact » dans la revue. Les six caractères sont écrits ici,
/// littéralement, parce que c'est la seule forme qui ne dépend pas de ce que la
/// bibliothèque appelle « whitespace ».
///
/// `\n` et `\r` sont dans l'ensemble bien que l'itération soit par ligne : le
/// prédicat transcrit la classe de caractères, pas l'usage qu'on en fait ici.
/// Les retirer serait une seconde décision, prise dans un helper.
fn is_posix_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

/// La tolérance de reprise de session : ce que la forme de l'AUTRE passe vaut.
///
/// Première passe (mika#1421 v3) : `Verdict: GROOMED` → `READY` (la boucle fait
/// alors une seconde passe de confirmation), `Verdict: ESCALATE` → `ESCALATE`.
///
/// Seconde passe (le miroir) : `Disposition: READY` → `GROOMED` — la
/// ratification sur rappel d'un plan déjà ratifié, que le bash documente par
/// l'incident fondateur de 16+ `ESCALATE` fallacieux sur 8+ tickets —
/// et `Disposition: ESCALATE` → `ESCALATE`.
///
/// **`ITERATE` en seconde passe n'a DÉLIBÉRÉMENT pas de bras**, et le `None`
/// qu'il rend ici n'est pas un trou : le bash écrit *« the spec forbids a
/// third-pass, so ITERATE at second-pass falls through to Tier 2 fuzzy rather
/// than being aliased »*. Le repli se fait sur le **premier** jeton trouvé, sans
/// continuer à chercher — un `Disposition: ITERATE` suivi plus bas d'un
/// `Disposition: READY` tombe au tier 2, parce que `head -1` a déjà tranché.
fn carry_over(pass: ArchPass, matched: &str) -> Option<&'static str> {
    match (pass, matched) {
        (ArchPass::First, "GROOMED") => Some("READY"),
        (ArchPass::First, "ESCALATE") => Some("ESCALATE"),
        (ArchPass::Second, "READY") => Some("GROOMED"),
        (ArchPass::Second, "ESCALATE") => Some("ESCALATE"),
        _ => None,
    }
}

/// L'entrée unique : quel jeton une réponse d'architecte porte, et quel tier l'a décidé.
///
/// Les cinq tiers dans l'ordre du bash. Voir l'en-tête du module pour ce que
/// chaque position garantit.
pub fn decide(text: &str, pass: ArchPass) -> Decision<'_> {
    // Tier 0 — le moteur a retiré la disposition (mika#2037). Le littéral est
    // LU chez son propriétaire, jamais recopié ici.
    if at_line_start(text, crate::agent_loop::DISPOSITION_WITHHELD_MARKER) {
        return Decision {
            token: None,
            tier: Tier::Withheld,
            fragment: None,
        };
    }

    // Tier 0b — ligne d'escalade du moteur (mika#2338).
    if engine_escalation_line(text).is_some() {
        return Decision {
            token: Some("ESCALATE"),
            tier: Tier::EngineEscalation,
            fragment: None,
        };
    }

    let (strict_label, strict_tokens, carry_label, carry_tokens) = match pass {
        ArchPass::First => (
            DISPOSITION_LABEL,
            DISPOSITION_TOKENS,
            VERDICT_LABEL,
            VERDICT_TOKENS,
        ),
        ArchPass::Second => (
            VERDICT_LABEL,
            VERDICT_TOKENS,
            DISPOSITION_LABEL,
            DISPOSITION_TOKENS,
        ),
    };

    // Tier 1a — la forme canonique littérale de la passe courante.
    if let Some(token) = first_labelled_token(text, strict_label, strict_tokens) {
        return Decision {
            token: Some(token),
            tier: Tier::Literal,
            fragment: None,
        };
    }

    // Tier 1b — la forme de l'autre passe (mika#1421 v3 et son miroir).
    //
    // Quand le littéral de l'autre passe est là mais que la table de reprise
    // n'a pas de bras pour lui, on tombe au tier 2, et on NE cherche PAS un
    // second jeton plus bas dans le texte — `head -1` a déjà tranché. La
    // chaîne de `let` ci-dessous rend cette lacune muette dans le flot de
    // contrôle, et c'est elle que `tier-1b-iterate-seconde-passe.md` épingle.
    if let Some(matched) = first_labelled_token(text, carry_label, carry_tokens)
        && let Some(token) = carry_over(pass, matched)
    {
        return Decision {
            token: Some(token),
            tier: Tier::CarryOver,
            fragment: None,
        };
    }

    // Tier 2 — paraphrase. Le seul tier qui rend un fragment, et c'est une
    // tranche de l'entrée.
    match arch_paraphrase::paraphrase(text, pass) {
        Some((paraphrase, fragment)) => Decision {
            token: Some(paraphrase.token()),
            tier: Tier::Paraphrase,
            fragment: Some(fragment),
        },
        None => Decision {
            token: None,
            tier: Tier::None,
            fragment: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKER: &str = crate::agent_loop::DISPOSITION_WITHHELD_MARKER;

    /// Tier 0 : le marqueur seul en tête de ligne supprime TOUT, et rend
    /// `Withheld` — pas `None`.
    #[test]
    fn mika2194_le_tier_0_rend_withheld_et_jamais_none() {
        for pass in [ArchPass::First, ArchPass::Second] {
            let text = format!("{MARKER}\n\nDisposition: READY\nShip it.\n");
            let decision = decide(&text, pass);
            assert_eq!(decision.token, None);
            assert_eq!(decision.tier, Tier::Withheld);
            assert_eq!(decision.tier.as_str(), "0");
            assert_eq!(decision.fragment, None);
        }
    }

    /// Tier 0 : le marqueur en tête du texte, sans `\n` devant (premier bras du `case`).
    #[test]
    fn mika2194_le_tier_0_matche_aussi_en_tete_de_texte() {
        assert_eq!(decide(MARKER, ArchPass::First).tier, Tier::Withheld);
    }

    /// Contrôle négatif du tier 0 : le marqueur CITÉ au milieu d'une ligne ne
    /// supprime rien, et la disposition réelle est lue au tier 1a.
    #[test]
    fn mika2194_un_marqueur_cite_au_milieu_dune_ligne_ne_supprime_rien() {
        let text = format!(
            "Le moteur écrit « {MARKER} » quand l'attestation manque.\nDisposition: ITERATE\n"
        );
        let decision = decide(&text, ArchPass::First);
        assert_eq!(decision.token, Some("ITERATE"));
        assert_eq!(decision.tier, Tier::Literal);
    }

    /// Tier 0b : l'escalade du moteur gagne contre un `READY` cité PLUS HAUT.
    #[test]
    fn mika2194_le_tier_0b_gagne_contre_un_ready_cite_avant() {
        let text = "Disposition: READY\n\
                    F1: (BLOCKING) [mika-engine] review-anchor: attestation absente.\n";
        let decision = decide(text, ArchPass::First);
        assert_eq!(decision.token, Some("ESCALATE"));
        assert_eq!(decision.tier, Tier::EngineEscalation);
        assert_eq!(decision.tier.as_str(), "0b");
    }

    /// Contrôle négatif du tier 0b : une prose qui cite `[mika-engine]` sans la
    /// forme complète ne déclenche pas le tier.
    #[test]
    fn mika2194_une_prose_citant_mika_engine_ne_declenche_pas_le_tier_0b() {
        let text = "Le [mika-engine] a signalé un review-anchor: manquant hier.\n\
                    Disposition: ITERATE\n";
        let decision = decide(text, ArchPass::First);
        assert_eq!(decision.tier, Tier::Literal);
        assert_eq!(decision.token, Some("ITERATE"));
    }

    /// Tier 0b : l'ancre tolère une indentation, comme `^[[:space:]]*` du bash.
    #[test]
    fn mika2194_lancre_du_tier_0b_tolere_lindentation() {
        let text = "   F12: (BLOCKING) [mika-engine] review-anchor: rien.\n";
        assert_eq!(decide(text, ArchPass::Second).tier, Tier::EngineEscalation);
    }

    /// Tier 1a : la forme canonique de chaque passe.
    #[test]
    fn mika2194_le_tier_1a_lit_la_forme_canonique_de_sa_passe() {
        let first = decide("Disposition: READY\n", ArchPass::First);
        assert_eq!((first.token, first.tier), (Some("READY"), Tier::Literal));

        let second = decide("Verdict: GROOMED\n", ArchPass::Second);
        assert_eq!(
            (second.token, second.tier),
            (Some("GROOMED"), Tier::Literal)
        );
    }

    /// Tier 1a : la valeur hors vocabulaire ne matche pas, et le tier ne tire pas.
    #[test]
    fn mika2194_une_disposition_hors_vocabulaire_ne_declenche_pas_le_tier_1a() {
        let decision = decide("Disposition: undefined\n", ArchPass::First);
        assert_ne!(decision.tier, Tier::Literal);
        assert_eq!(decision.token, None);
        assert_eq!(decision.tier, Tier::None);
    }

    /// Tier 1a : `grep` est orienté ligne — le jeton à la ligne suivante ne matche pas.
    #[test]
    fn mika2194_le_tier_1a_ne_matche_pas_a_travers_un_saut_de_ligne() {
        assert_ne!(
            decide("Disposition:\nREADY\n", ArchPass::First).tier,
            Tier::Literal
        );
        // Contrôle positif : sur une seule ligne, cela matche.
        assert_eq!(
            decide("Disposition:\tREADY\n", ArchPass::First).tier,
            Tier::Literal
        );
    }

    /// Tier 1a : `[[:space:]]` inclut la tabulation verticale, et Rust ne la
    /// plie pas tout seul.
    ///
    /// `char::is_ascii_whitespace` omet `\v` (0x0B) ; `[[:space:]]` l'inclut.
    /// L'écart est MESURÉ, non supposé : `.pilot-scratch/sonde-tiers/probe-tab-verticale.sh`,
    /// le 2026-11-02, rend `token=[READY]` sur `Disposition:\vREADY` — le bash
    /// le lit. Le corpus doré ne l'exerce pas, et c'est précisément pourquoi ce
    /// test existe ici : sans lui, un portage sur `is_ascii_whitespace` reste
    /// vert sur les 29 lignes et se lit « byte-exact » en revue.
    #[test]
    fn mika2194_le_tier_1a_plie_la_tabulation_verticale_comme_le_bash() {
        let decision = decide("Disposition:\u{0B}READY\n", ArchPass::First);
        assert_eq!(decision.token, Some("READY"));
        assert_eq!(decision.tier, Tier::Literal);
        // Les cinq autres caractères de la classe, sur une seule ligne.
        assert_eq!(
            decide("Disposition: \t\u{0C}READY\n", ArchPass::First).tier,
            Tier::Literal
        );
        // Contrôle négatif : un caractère hors de la classe ne se plie pas.
        assert_ne!(
            decide("Disposition:-READY\n", ArchPass::First).tier,
            Tier::Literal
        );
    }

    /// Tier 1a : la casse compte, faute de `-i` sur le `grep` du bash.
    #[test]
    fn mika2194_le_tier_1a_est_sensible_a_la_casse() {
        assert_ne!(
            decide("disposition: ready\n", ArchPass::First).tier,
            Tier::Literal
        );
    }

    /// Tier 1a : la POSITION décide, jamais l'ordre de la table.
    #[test]
    fn mika2194_le_tier_1a_retient_la_premiere_occurrence_du_texte() {
        let text = "Disposition: ITERATE\nDisposition: ESCALATE\n";
        assert_eq!(decide(text, ArchPass::First).token, Some("ITERATE"));
    }

    /// Tier 1b, première passe : `Verdict: GROOMED` → `READY` (mika#1421 v3).
    #[test]
    fn mika2194_le_tier_1b_de_premiere_passe_mappe_groomed_vers_ready() {
        let decision = decide("Verdict: GROOMED\n", ArchPass::First);
        assert_eq!(decision.token, Some("READY"));
        assert_eq!(decision.tier, Tier::CarryOver);
        assert_eq!(decision.tier.as_str(), "1b");

        let decision = decide("Verdict: ESCALATE\n", ArchPass::First);
        assert_eq!(
            (decision.token, decision.tier),
            (Some("ESCALATE"), Tier::CarryOver)
        );
    }

    /// Tier 1b, seconde passe : `Disposition: READY` → `GROOMED` (le miroir).
    #[test]
    fn mika2194_le_tier_1b_de_seconde_passe_mappe_ready_vers_groomed() {
        let decision = decide("Disposition: READY\n", ArchPass::Second);
        assert_eq!(decision.token, Some("GROOMED"));
        assert_eq!(decision.tier, Tier::CarryOver);

        let decision = decide("Disposition: ESCALATE\n", ArchPass::Second);
        assert_eq!(
            (decision.token, decision.tier),
            (Some("ESCALATE"), Tier::CarryOver)
        );
    }

    /// `ITERATE` en seconde passe TOMBE au tier 2 : pas de bras, et pas de
    /// seconde recherche derrière.
    #[test]
    fn mika2194_un_iterate_en_seconde_passe_tombe_au_tier_2() {
        let decision = decide("Disposition: ITERATE\n", ArchPass::Second);
        assert_ne!(decision.tier, Tier::CarryOver);
        assert_eq!(decision.tier, Tier::None);
        assert_eq!(decision.token, None);

        // `head -1` a déjà tranché : un ESCALATE plus bas ne rattrape pas.
        let decision = decide(
            "Disposition: ITERATE\nDisposition: ESCALATE\n",
            ArchPass::Second,
        );
        assert_ne!(decision.tier, Tier::CarryOver);
    }

    /// Le tier 1a devance le tier 1b, et c'est ce qui fait de 1b une tolérance.
    #[test]
    fn mika2194_le_tier_1a_devance_le_tier_1b() {
        let text = "Verdict: ESCALATE\nDisposition: READY\n";
        let decision = decide(text, ArchPass::First);
        assert_eq!(decision.token, Some("READY"));
        assert_eq!(decision.tier, Tier::Literal);
    }

    /// Tier 2 : le fragment est une TRANCHE de l'entrée, prouvée par son adresse.
    #[test]
    fn mika2194_le_fragment_du_tier_2_est_une_tranche_de_lentree() {
        let text = String::from("The plan is clean, so ship it.");
        let decision = decide(&text, ArchPass::First);
        assert_eq!(decision.token, Some("READY"));
        assert_eq!(decision.tier, Tier::Paraphrase);
        assert_eq!(decision.tier.as_str(), "2");
        let fragment = decision.fragment.expect("le tier 2 rend un fragment");
        let base = text.as_ptr().addr();
        let at = fragment.as_ptr().addr();
        assert!(
            at >= base && at + fragment.len() <= base + text.len(),
            "le fragment doit pointer DANS l'entrée, pas dans un littéral du motif"
        );
    }

    /// Les tiers 0 à 1b n'écrivent AUCUN fragment.
    #[test]
    fn mika2194_seul_le_tier_2_rend_un_fragment() {
        let withheld = format!("{MARKER}\n");
        for text in [
            withheld.as_str(),
            "F1: (BLOCKING) [mika-engine] review-anchor: rien.\n",
            "Disposition: READY\n",
            "Verdict: GROOMED\n",
            "Je relis le plan révisé.\n",
        ] {
            assert_eq!(
                decide(text, ArchPass::First).fragment,
                None,
                "aucun tier hors 2 n'écrit de fragment : {text:?}"
            );
        }
    }

    /// Le cas MODAL en production : une réponse française ne rend rien (B1).
    #[test]
    fn mika2194_une_reponse_francaise_ne_rend_aucun_jeton() {
        let text = "Le plan est net, je le trouve prêt à partir.\n";
        for pass in [ArchPass::First, ArchPass::Second] {
            let decision = decide(text, pass);
            assert_eq!(decision.token, None);
            assert_eq!(decision.tier, Tier::None);
            assert_eq!(decision.tier.as_str(), "-");
        }
    }

    /// Le vocabulaire de la colonne `tier` est FERMÉ, et `-` y est un nom.
    #[test]
    fn mika2194_le_vocabulaire_des_tiers_est_ferme() {
        let all = [
            Tier::Withheld,
            Tier::EngineEscalation,
            Tier::Literal,
            Tier::CarryOver,
            Tier::Paraphrase,
            Tier::None,
        ];
        let names: Vec<&str> = all.iter().map(Tier::as_str).collect();
        assert_eq!(names, ["0", "0b", "1a", "1b", "2", "-"]);
        for name in &names {
            assert!(
                !name.is_empty(),
                "une colonne vide porte « - », jamais rien — sinon `IFS=$'\\t' read` \
                 fusionne deux tabulations et décale tout ce qui suit"
            );
        }
    }
}
