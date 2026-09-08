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
//! leurs deux appelants, à leur place.
//!
//! Elles ne divergent plus sur la forme du chemin : mika#2120 a rendu `auto_pull`
//! permissif au segment de dépôt optionnel (`mika/docs/plans/…`), que `executor` acceptait
//! déjà. Elles restent volontairement asymétriques sur la rigueur — `auto_pull` ancre ses
//! trois prédicats en début de ligne et lit hors des blocs clôturés, `executor` se contente
//! d'une sous-chaîne. L'écart va dans le sens sûr : le lecteur strict est celui qui promeut.
//!
//! **Ce module n'a pas absorbé cette moitié-là, et ce n'est pas un oubli.** `auto_pull`
//! décide d'une promotion, `executor` d'un routage : les deux lisent le même callout mais
//! n'engagent pas la même dépense, et un prédicat commun leur imposerait la rigueur du plus
//! strict ou la tolérance du plus lâche sans que personne ait tranché lequel. Le marqueur de
//! verdict est ici parce que les deux appelants en veulent la **même** lecture ; les
//! conditions `Branch`/`Plan` restent chez eux parce qu'ils n'en veulent pas la même.
//!
//! # Le discriminateur a changé de nature
//!
//! Avant mika#2158 il était **lexical** : le mot `second-pass` devait précéder `GROOMED`.
//! Il est désormais **positionnel** : la ligne de callout `Grooming history` est le contexte,
//! et le **dernier marqueur d'état** qu'elle contient est l'état. Cet ancrage sur le préfixe
//! de ligne `^> - **Grooming history:**` est ce qui préserve la distinction callout/prose —
//! « le ticket a été GROOMED hier » écrit en corps de texte ne rend toujours rien.
//!
//! Les marqueurs d'état sont les deux tokens de verdict — `GROOMED` et `ESCALATE` — **plus**
//! le `READY` de première passe lorsqu'aucune passe ultérieure n'est annoncée. `ITERATE`
//! n'en est jamais un. **Il n'y a plus d'exception hors-ordre** : tous les marqueurs sont
//! rangés dans le même ordre, et le dernier fait foi.
//!
//! # Pourquoi `READY` est entré dans l'ordre (mika#2188)
//!
//! Jusqu'à mika#2188, le `READY` de première passe était traité par un **repli** —
//! atteignable seulement quand le callout ne portait aucun token de verdict. Dès qu'un
//! `GROOMED` ou un `ESCALATE` apparaissait n'importe où, le repli devenait inatteignable.
//!
//! Ce détail a rendu indispatchable un chemin **prescrit** par `/mika-groom-ticket` :
//!
//! ```text
//! /ce:plan → checkpoint Phase 2.5 (ESCALATE-divergence, résolu par l'opérateur)
//!         → réconciliation → mika-arch first-pass (READY)
//! ```
//!
//! `VERDICT_TOKEN_RE` matche `ESCALATE` **à l'intérieur** de `ESCALATE-divergence` — le tiret
//! satisfait la frontière de mot finale. Le dernier *token de verdict* restait donc
//! `ESCALATE`, et le repli `READY` n'était jamais lu. Un ticket dont l'escalade avait été
//! résolue par l'opérateur, puis approuvé par l'architecte, était lu comme escaladé. Mesuré
//! sur **mika-cloud#205** le 2026-09-05 : `verdict=Escalated` — et non `Absent`, ce qui est
//! la signature qui distingue cette cause de toutes les autres.
//!
//! ## La forme retenue, et celle qui a été écartée
//!
//! **Retenue — `READY` devient un marqueur positionnel.** Une seule liste ordonnée, le
//! dernier marqueur fait foi. Elle *supprime* un concept : le module retrouve une règle au
//! lieu de deux qui se marchaient dessus.
//!
//! **Écartée — reconnaître le motif « escalade résolue »** (`ESCALATE…` suivi d'une passe
//! aboutie, traité comme neutralisant l'escalade). Trois raisons :
//!
//! 1. Elle répare le symptôme, pas la cause : la cause est qu'une règle de repli cohabitait
//!    avec un discriminateur qui se déclare positionnel.
//! 2. Elle ne voit pas le cas symétrique. `first-pass (READY) → revue opérateur (ESCALATE)`
//!    doit rendre `Escalated` ; sous la forme retenue c'est automatique, sous le motif il
//!    faudrait une seconde règle. Le cas est épinglé par
//!    [`tests::first_pass_ready_then_later_escalate_is_escalated`].
//! 3. Définir « une passe aboutie » revient à reconstruire la liste de marqueurs pour un
//!    usage local et unique — la forme retenue, plus le coût de nommer le motif.
//!
//! **Écartée aussi — resserrer `VERDICT_TOKEN_RE`** pour que `ESCALATE-divergence` ne matche
//! plus : elle ferait dépendre le verdict de l'orthographe d'un mot composé plutôt que de la
//! chronologie.
//!
//! ## Ce que la règle positionnelle garantit — et ce qu'elle ne garantit pas
//!
//! Elle est **purement chronologique**. Le dernier marqueur gagne, quelle que soit la prose
//! autour de lui : `(ESCALATE-divergence, NON résolu) → first-pass (READY)` rend `Groomed`.
//! Le prédicat ne lit pas le mot « résolu » et ne peut pas le lire — ce qui atteste la
//! résolution est **la passe architecte postérieure elle-même**, pas l'adjectif.
//!
//! Une escalade qui doit rester lisible comme escalade est donc une escalade qu'aucun
//! marqueur abouti ne suit. C'est ce qu'épinglent
//! [`tests::escalate_without_later_groomed_is_escalated`] et
//! [`tests::first_pass_ready_then_later_escalate_is_escalated`], et c'est tout ce que la
//! forme retenue promet. Prétendre qu'elle distingue une escalade résolue d'une escalade
//! ouverte serait lui prêter une lecture sémantique qu'elle n'a pas.

use std::sync::LazyLock;

use regex::Regex;

/// Le verdict de grooming lu dans le callout `Grooming history` d'un corps d'issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroomingVerdict {
    /// Le grooming a abouti : le ticket peut partir en implémentation.
    Groomed,
    /// Le dernier marqueur d'état est un `ESCALATE` — le ticket est dans les mains de
    /// l'opérateur, pas prêt.
    ///
    /// Depuis mika#2188, ce qui lève une escalade n'est plus seulement un `GROOMED`
    /// postérieur : un `READY` de première passe postérieur la lève aussi. C'est la
    /// **position** du dernier marqueur qui décide, pas sa nature.
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
///
/// **Le groupe 1 est le token `READY` lui-même**, et non la phrase qui l'introduit : c'est sa
/// position qui entre dans l'ordre des marqueurs d'état (voir [`grooming_verdict`]). Ancrer
/// sur `first-pass` donnerait le même verdict sur les corps connus, mais ferait dépendre
/// l'ordonnancement de la longueur du préfixe de phrase — indéfendable dès qu'une forme
/// nouvelle apparaît.
///
/// **La casse du token est significative ; celle de la phrase qui l'introduit ne l'est pas.**
/// `(?i)` ne couvre que l'alternance de préfixe — `First-pass`, `Première passe` sont des
/// variations de rédaction légitimes. `READY` reste sensible à la casse, exactement comme
/// [`VERDICT_TOKEN_RE`], et pour la même raison : c'est un token produit par le pipeline, pas
/// un mot de prose. Depuis mika#2188 ce `READY` peut **surclasser** un `ESCALATE` qui le
/// précède ; lui laisser une discipline plus lâche qu'aux tokens qu'il surclasse ferait de
/// `first-pass (ready ?)` écrit en passant un ordre de dispatch.
static FIRST_PASS_READY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i:first-pass|première passe|premiere passe)\s*\(\s*(READY)")
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
    let lines: Vec<&str> = CALLOUT_LINE_RE
        .captures_iter(issue_body)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

/// Le verdict de grooming porté par le corps d'issue.
///
/// # La règle
///
/// 1. Pas de ligne de callout `Grooming history` → [`GroomingVerdict::Absent`].
/// 2. Le **dernier marqueur d'état** du callout fait foi. Les marqueurs sont les tokens
///    `GROOMED` et `ESCALATE[DS]?`, **plus** le `READY` de première passe lorsqu'aucune
///    marque de passe ultérieure n'est annoncée. `… (ESCALATE …) → … (GROOMED …)` rend
///    `Groomed` ; `… (GROOMED) → … (ESCALATE)` rend `Escalated` ;
///    `… (ESCALATE-divergence, résolu…) → … first-pass (READY)` rend `Groomed`. L'ordre
///    compte dans tous les sens.
/// 3. Aucun marqueur → `Absent`.
///
/// Il n'y a **pas** de quatrième règle, et aucun repli : c'est le point de mika#2188. Une
/// première passe `READY` vaut verdict parce que `/mika-groom-ticket` phase 3 étape 10
/// prescrit de sauter à la phase 5 sans seconde passe quand le plan est sain du premier
/// coup — en exiger une reviendrait à punir le grooming qui s'est bien passé. Mais elle le
/// vaut **à sa position**, comme les autres, et non par une branche atteinte seulement en
/// l'absence de tout token de verdict.
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
/// Depuis mika#2188 ce désarmement gouverne la **participation** de `READY` à l'ordre des
/// marqueurs, et non plus un repli. Sa justification vaut mot pour mot ; seul son point
/// d'application a bougé. `LATER_PASS_RE` s'évalue sur le texte concaténé entier : un second
/// callout portant `second-pass` désarme donc le `READY` du premier. C'est le comportement
/// d'avant, préservé volontairement — le changer ouvrirait une question de périmètre que
/// mika#2188 ne couvre pas.
///
/// `ITERATE` seul reste `Absent` pour la raison symétrique : une première passe qui itère
/// **prescrit** une seconde passe, qui n'a pas eu lieu.
pub fn grooming_verdict(issue_body: &str) -> GroomingVerdict {
    let Some(text) = callout_text(issue_body) else {
        return GroomingVerdict::Absent;
    };

    // Les marqueurs d'état, dans l'ordre du document : (offset, est_groomé).
    let mut markers: Vec<(usize, bool)> = VERDICT_TOKEN_RE
        .find_iter(&text)
        .map(|m| (m.start(), m.as_str().starts_with("GROOMED")))
        .collect();

    // Une première passe `READY` est un marqueur d'état — mais seulement si aucune passe
    // ultérieure n'est annoncée. Ce désarmement est la règle AC1 de mika#2158, inchangée :
    // il gouverne désormais la *participation* de `READY` à l'ordre, là où il gouvernait
    // un repli.
    if !LATER_PASS_RE.is_match(&text) {
        markers.extend(
            FIRST_PASS_READY_RE
                .captures_iter(&text)
                .filter_map(|c| c.get(1))
                .map(|m| (m.start(), true)),
        );
    }

    // Trier sur l'offset seul suffit : `VERDICT_TOKEN_RE` ne matche ni `READY` ni ses
    // préfixes, donc deux marqueurs ne peuvent pas partager une position. Un `then`
    // départageur laisserait croire à une ambiguïté qui n'existe pas.
    markers.sort_by_key(|(offset, _)| *offset);

    match markers.last() {
        Some((_, true)) => GroomingVerdict::Groomed,
        Some((_, false)) => GroomingVerdict::Escalated,
        None => GroomingVerdict::Absent,
    }
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

    /// mika#2188 — le chemin nominal Phase 2.5 : une escalade de réconciliation résolue par
    /// l'opérateur, suivie d'un first-pass READY. Callout relevé sur mika-cloud#205 le
    /// 2026-09-05, verbatim.
    ///
    /// Avant le correctif ce corps rendait `Escalated` — et non `Absent` : le prédicat ne
    /// disait pas « je ne sais pas lire », il disait « ce ticket est escaladé ». C'est la
    /// signature qui distingue cette cause de toutes les autres.
    #[test]
    fn ac1_escalate_divergence_resolved_then_first_pass_ready_is_groomed() {
        let history = "/ce:plan → checkpoint Phase 2.5 (ESCALATE-divergence, résolu par \
                       l'opérateur) → réconciliation → mika-arch first-pass (READY)";
        assert_eq!(
            grooming_verdict(&body_with(history)),
            GroomingVerdict::Groomed
        );
        assert!(has_groomed_verdict(&body_with(history)));
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

    /// mika#2188 — l'ordre compte aussi quand `READY` précède l'escalade. Une escalade
    /// POSTÉRIEURE à une passe aboutie reste une escalade. C'est le cas que la forme (a)
    /// rend automatique et que la forme (b) — « reconnaître le motif escalade-résolue » —
    /// aurait manqué : voir §2 du plan.
    #[test]
    fn first_pass_ready_then_later_escalate_is_escalated() {
        assert_eq!(
            grooming_verdict(&body_with(
                "mika-arch first-pass (READY) → revue de périmètre opérateur (ESCALATE)"
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

    /// mika#2188 — `READY` surclasse désormais un `ESCALATE` qui le précède. Il porte donc
    /// la même autorité qu'un token de verdict, et doit porter la même discipline lexicale :
    /// la casse compte. Sans cela, `first-pass (ready ?)` écrit en passant dans une phrase
    /// française vaudrait ordre de dispatch sur un ticket escaladé.
    ///
    /// La casse du **préfixe** reste libre — `Première passe` est une variation de rédaction,
    /// pas un token.
    #[test]
    fn lowercase_ready_is_not_a_state_marker() {
        assert_eq!(
            grooming_verdict(&body_with(
                "checkpoint Phase 2.5 (ESCALATE-divergence) → l'opérateur veut refaire la \
                 first-pass (ready ?)"
            )),
            GroomingVerdict::Escalated
        );
        assert_eq!(
            grooming_verdict(&body_with("Première passe (READY) — rien à redire")),
            GroomingVerdict::Groomed
        );
    }

    /// **Limite connue, épinglée — ce n'est pas une régression.**
    ///
    /// `LATER_PASS_RE` s'évalue sur le texte **concaténé** de tous les callouts. Un premier
    /// callout portant `second-pass` désarme donc le `READY` d'un second callout empilé, et
    /// le correctif de mika#2188 est inerte sur cette forme : le verdict reste `Escalated`.
    ///
    /// C'est le comportement d'avant mika#2188, préservé volontairement (§3.2 et §6 du plan) :
    /// le re-grooming qui empile un second callout (mika#2012) est une population distincte,
    /// et changer la portée de la porte ouvrirait un périmètre que le ticket ne couvre pas.
    ///
    /// Ce test existe parce qu'une prose disant « préservé volontairement » ne se distingue
    /// pas d'une régression pour qui lit le module six mois plus tard. Si cette limite doit
    /// tomber un jour, c'est **ce test** qu'il faudra retirer sciemment — pas un verdict
    /// surprenant qu'il faudra re-diagnostiquer.
    #[test]
    fn stacked_callouts_later_pass_gate_is_global_known_limit() {
        let body = "## Description\n\n\
                    > - **Grooming history:** first-pass (ITERATE) → second-pass (ESCALATE)\n\
                    > - **Grooming history:** mika-arch first-pass (READY)\n";
        assert_eq!(
            grooming_verdict(body),
            GroomingVerdict::Escalated,
            "limite connue mika#2188 : la porte LATER_PASS_RE est globale au texte concaténé"
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
    /// # Réserve honnête, à lire avant de conclure que les deux prédicats sont un seul
    ///
    /// mika#2120 a fermé la divergence sur la forme du chemin : `auto_pull` accepte
    /// désormais le préfixe de dépôt, que `executor` acceptait déjà. Les deux ne sont pas
    /// pour autant identiques, et ne doivent pas l'être — `auto_pull` est **ancré** et lit
    /// hors des blocs de code, `executor` reste une sous-chaîne. C'est un écart dans le
    /// sens sûr : le lecteur strict est celui qui promeut. Le resserrer côté `executor`
    /// serait un autre ticket, pas une harmonisation.
    ///
    /// Ce que ce croisement atteste reste donc borné aux six corps figés, qui écrivent
    /// tous la forme nue. Le jeu qui mesure l'axe du chemin vit à côté, en
    /// `crates/mika-agent/tests/fixtures/plan_callout_bodies/`.
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

    /// Les deux prédicats s'accordent désormais sur la forme préfixée par le dépôt —
    /// c'est ce que mika#2120 a rendu. Le test qui **épinglait leur désaccord** sur cette
    /// forme (`mika2120_divergence_is_still_open_and_this_test_pins_it`) a été supprimé
    /// dans le même commit que le correctif, comme sa propre documentation le prescrivait.
    ///
    /// Ce qui reste vrai et ce qui a changé : le croisement ci-dessus garde sa portée
    /// bornée aux six corps figés, mais l'accord qu'il constate n'est plus le fruit d'un
    /// jeu qui évite la forme litigieuse. L'axe du chemin a son propre jeu de mesure, en
    /// `crates/mika-agent/tests/fixtures/plan_callout_bodies/`.
    #[test]
    fn mika2120_repo_prefixed_plan_callout_is_read_by_both_predicates() {
        let repo_prefixed = "## Description\n\n\
             > - **Branch:** `fix/2120/x`\n\
             > - **Plan:** `mika/docs/plans/2026-09-03-001-fix-2120-x-plan.md` (committed @ abc)\n\
             > - **Grooming history:** mika-arch second-pass (GROOMED)\n";

        assert!(
            crate::auto_pull::is_groomed(repo_prefixed),
            "auto_pull accepte le segment de dépôt optionnel depuis mika#2120"
        );
        assert!(
            crate::skills::executor::check_grooming_markers(repo_prefixed).is_empty(),
            "executor l'acceptait déjà"
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
