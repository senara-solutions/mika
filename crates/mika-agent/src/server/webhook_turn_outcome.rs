//! Le second terme du registre des tours webhook : l'ISSUE du tour
//! (mika#2855, vol d'instrument de la classe (g) de mika#2680).
//!
//! mika#2680 a chiffré la classe (g) — `mika-qa` × `check_suite` success sur une
//! branche de PR : 36 tours, 10 687 327 tokens d'entrée, 16,31 % de la fenêtre
//! mesurée — puis l'a réfutée comme filtrable : la porte mika#2260 préserve ce
//! tour exprès, et `qa-review-webhook-success` est l'unique entrée autonome de la
//! revue QA sur CI vert. Seuls ses **arrêts précoces** seraient filtrables, et
//! leur nombre est inconnu parce que le registre ne dit pas ce que le tour a
//! fait. **Ce module n'écarte aucun tour. Il mesure.**
//!
//! # Pourquoi un SECOND terme, et non un champ de plus sur la ligne d'entrée
//!
//! [`super::webhook_prefilter::emit_turn_charged`] est appelée depuis un seul
//! site, dans le bras `Gate::Charged`, **avant** `run_agent`. Son message est
//! littéralement « tour webhook facturé — le tour a lieu » : à cet instant il
//! n'existe aucune issue à nommer. L'issue exige donc une seconde ligne, émise en
//! fin de tour et corrélée à la première par `request_id`.
//!
//! # Pourquoi l'issue est ATTESTÉE par le modèle, jamais déduite de la trace
//!
//! La réserve de Mika Prime demande que le champ nomme **lequel** des quatre
//! arrêts a terminé le tour. La trace d'outils ne peut pas le dire :
//!
//! | issue | ce que la trace d'outils porte |
//! |---|---|
//! | arrêt 1 (pas de PR ouverte) | un `run_gh pr list` |
//! | arrêt 2 (hors périmètre) | un `run_gh pr list` |
//! | arrêt 3 (déjà revue à ce SHA) | un `run_gh pr list` + un `run_gh pr view --json reviews` |
//! | arrêt 4 (verdict déjà posé) | un `run_gh pr list` + un `run_gh pr view --json reviews` |
//!
//! Les arrêts 1 et 2 sont **indiscernables** sur la trace, et 3 et 4 le sont
//! entre eux : la décision vit dans le JSON que le modèle a lu, jamais dans la
//! liste des appels. Une déduction rendrait deux paires fusionnées — deux
//! populations sous un nom, non soustractibles (classe mika#2205). D'où un
//! marqueur déclaratif, [`ATTESTATION_PREFIX`], sur sa propre ligne du texte
//! final.
//!
//! Un **outil** d'attestation était l'autre option, refusée pour trois raisons :
//! il ajoute une surface d'outil à tous les tours de l'agent pour l'usage d'un
//! seul skill ; il faut l'inscrire dans `MIKA_QA_IDENTITY`, donc toucher une
//! allowlist, donc sortir du périmètre « rien ne change hors l'attestation »
//! (AC5) ; et il coûte une étape de boucle par tour — un instrument qui
//! renchérit le tour dont il mesure le coût.
//!
//! # Pourquoi la garde `required_suffix_lines` (#864) n'est PAS armée
//!
//! `[output].required_suffix_lines` existe et re-prompte une fois quand aucune
//! des trois dernières lignes non vides ne matche. Le réflexe du dépôt est de
//! préférer cette garde au « MUST » de prompt. **Ici, non :**
//!
//! - **un re-prompt est un appel LLM supplémentaire sur le tour même que ce vol
//!   mesure.** Un instrument qui peut doubler la grandeur qu'il observe quand le
//!   modèle oublie sa ligne n'est pas un instrument ;
//! - la garde porte sur la dernière ligne, et sur l'issue `review` le texte final
//!   appartient à `qa-review` (verdict, énumération par AC, preuves). Lui imposer
//!   un suffixe de forme serait un second contrat de sortie sur un skill que ce
//!   vol ne touche pas.
//!
//! Le prix de ce refus est nommé, pas passé sous silence : le modèle peut ne rien
//! attester, et c'est exactement ce que [`Outcome::Absent`] compte. L'ordre est :
//! mesurer la désobéissance, puis décider de la contraindre.
//!
//! # Pourquoi la ligne porte `agent_id`, qui semble redondant
//!
//! `agent_id` est déjà sur la ligne d'entrée et le lecteur joint par
//! `request_id` : le champ a l'air d'un doublon. Il n'en est pas un.
//! `scripts/measure-webhook-turn-cost` filtre **chaque ligne du corpus** sur
//! `--agent` (`elif ($agent != "" and (($o.agent_id? // "") != $agent)) then
//! empty`). Une ligne de sortie sans `agent_id` serait écartée par tout appel
//! `--agent mika-qa` — c'est-à-dire par l'appel même que (g′) exige — et la
//! jointure rendrait `outcome: null` partout, sur un agrégat qui aurait l'air
//! complet. Le champ est une **condition de correction du lecteur**.

use std::str::FromStr;

use tracing::info;

/// Nom d'événement `tracing` de l'ISSUE d'un tour webhook (mika#2855).
/// **SEUL ÉCRIVAIN** : [`emit_turn_outcome`].
///
/// Comme son voisin d'entrée [`super::webhook_prefilter::WEBHOOK_TURN_CHARGED_EVENT`],
/// ce n'est délibérément PAS un `tool_name` d'audit : `audit_events` reste
/// réservé aux décisions du moteur, et l'issue d'un tour est un fait de coût.
pub(crate) const WEBHOOK_TURN_OUTCOME_EVENT: &str = "webhook_turn_outcome";

/// Préfixe du marqueur d'attestation, en tête de sa propre ligne du texte final.
///
/// **Source de vérité unique** du préfixe : lue par [`parse_attestation`] ET par
/// le test de parité avec le prompt embarqué. Deux littéraux dériveraient en
/// silence.
pub(crate) const ATTESTATION_PREFIX: &str = "Webhook-Outcome:";

/// Ce que le MODÈLE peut attester : les six sorties du prompt de
/// `qa-review-webhook-success`, dans son ordre, et rien d'autre.
///
/// [`FromStr`] est l'unique porte d'entrée depuis du texte, ce qui rend
/// [`Outcome::Absent`] et [`Outcome::Unreadable`] inatteignables depuis une
/// valeur écrite — propriété du type, pas d'une assertion. Même idiome que
/// `Gate::{Skipped, Charged}` (mika#2680 vol 4) et `Transmission::{Incoming,
/// Outgoing}` (mika#2807).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attested {
    /// § 1 — aucune PR ouverte sur le head SHA.
    StopNoOpenPr,
    /// § 2 — dépôt / auteur / état hors périmètre.
    StopOutOfScope,
    /// § 3 — une revue `mika-platform-qa` porte déjà ce `commit_id`.
    StopAlreadyReviewedSha,
    /// § 4 — un verdict d'autorité est déjà posé sur la tête.
    StopVerdictRecorded,
    /// § 5 — le skill `qa-review` a été appelé.
    Review,
    /// § 6 — la revue n'a pas pu se faire (`send_message` + `blocked`).
    ReviewFailed,
}

impl Attested {
    /// Les six termes. Lue par [`FromStr`] et par le test de parité, donc une
    /// seule énumération à tenir : un variant ajouté sans entrée ici ne se lit
    /// pas, et le test de parité le dit.
    pub(crate) const ALL: [Attested; 6] = [
        Self::StopNoOpenPr,
        Self::StopOutOfScope,
        Self::StopAlreadyReviewedSha,
        Self::StopVerdictRecorded,
        Self::Review,
        Self::ReviewFailed,
    ];

    /// Le terme tel qu'il part sur le fil, et tel que le prompt le demande au
    /// modèle. Un format de fil : épinglé par test.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::StopNoOpenPr => "stop_no_open_pr",
            Self::StopOutOfScope => "stop_out_of_scope",
            Self::StopAlreadyReviewedSha => "stop_already_reviewed_sha",
            Self::StopVerdictRecorded => "stop_verdict_recorded",
            Self::Review => "review",
            Self::ReviewFailed => "review_failed",
        }
    }
}

impl FromStr for Attested {
    type Err = ();

    /// Strict : la valeur est l'un des six termes, littéralement. Aucune
    /// reconnaissance lexicale, aucun rapprochement — le dépôt a déjà refusé
    /// cela sous ce nom (mika#2136 D4). Ce qui n'est pas un terme est
    /// [`Outcome::Unreadable`], jamais un terme deviné.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|a| a.as_str() == s).ok_or(())
    }
}

/// Ce que le MOTEUR met sur le fil. `Absent` et `Unreadable` ne sont pas
/// parsables : aucun texte ne peut les produire, seul le moteur les dérive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Le modèle a attesté l'un des six termes.
    Attested(Attested),
    /// Aucune ligne [`ATTESTATION_PREFIX`] dans le texte final — y compris : pas
    /// de texte du tout (branche `Err` de la boucle). C'est l'exigence AC2 : un
    /// tour qui n'atteste rien ne se lit JAMAIS comme un arrêt deviné.
    Absent,
    /// Une ligne présente, dont la valeur n'est aucun des six termes.
    ///
    /// Distinct d'[`Outcome::Absent`] à dessein : « le modèle n'a rien dit »
    /// appelle un durcissement de prompt, « le modèle a dit quelque chose que le
    /// contrat ne prévoit pas » appelle une correction du contrat ou de l'enum.
    /// Deux lectures, deux suites, non soustractibles sous un nom partagé
    /// (classe mika#2205).
    Unreadable,
}

impl Outcome {
    /// La valeur du champ `outcome` sur le fil. Un format de fil : épinglé par
    /// test.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Attested(attested) => attested.as_str(),
            Self::Absent => "outcome_absent",
            Self::Unreadable => "outcome_unreadable",
        }
    }
}

/// Lit l'attestation du texte final d'un tour. **Pure** : aucune lecture
/// d'état, aucun appel réseau, donc testable sans bouchon.
///
/// Deux écarts délibérés par rapport à ce que le prompt demande :
///
/// - **la dernière occurrence gagne, et non la première.** Un modèle qui
///   raisonne à voix haute peut citer le contrat (« I will emit
///   `Webhook-Outcome: review` once the review completes ») avant de l'honorer.
///   La prose précède l'attestation ; prendre la première occurrence
///   transformerait une intention en fait.
/// - **tolérant sur la position, strict sur la valeur.** Exiger la dernière
///   ligne ferait rendre `outcome_absent` pour une attestation correcte mais mal
///   placée — compter une obéissance comme une désobéissance. Inversement une
///   valeur hors des six n'est jamais devinée : [`Outcome::Unreadable`].
///
/// La comparaison du préfixe est **sensible à la casse** et le `trim` ne porte
/// que sur les blancs de bord : un préfixe est un contrat, pas une intention à
/// reconnaître au lexique.
pub(crate) fn parse_attestation(text: Option<&str>) -> Outcome {
    let Some(text) = text else {
        return Outcome::Absent;
    };
    // `next_back`, et non `last` : la chaîne est bi-directionnelle, donc le
    // dernier élément se prend par la queue sans parcourir tout le texte
    // (`clippy::double_ended_iterator_last`).
    let Some(value) = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix(ATTESTATION_PREFIX))
        .next_back()
    else {
        return Outcome::Absent;
    };
    match Attested::from_str(value.trim()) {
        Ok(attested) => Outcome::Attested(attested),
        Err(()) => Outcome::Unreadable,
    }
}

/// Émet [`WEBHOOK_TURN_OUTCOME_EVENT`] : voici comment CE tour webhook s'est
/// terminé. **SEUL ÉCRIVAIN** de cette ligne.
///
/// `text` est le texte final du tour, ou `None` quand il n'y en a pas — branche
/// `Err` de la boucle, où aucun `AgentOutput` n'existe. Un tour mort n'a rien
/// attesté, et `outcome_absent` le dit exactement.
///
/// Un seul `info!`, trois champs, **pas de bras conditionnel** : aucun champ de
/// cette ligne ne peut manquer, contrairement à `candidate` sur la ligne
/// d'entrée (motif mika#2331). La ligne n'est gatée par rien, comme sa voisine
/// d'entrée et pour la même raison : une fenêtre où la télémétrie a été réduite
/// est précisément celle qu'on ne veut pas perdre.
pub(crate) fn emit_turn_outcome(agent_id: &str, request_id: &str, text: Option<&str>) {
    let outcome = parse_attestation(text).as_str();

    info!(
        target: "mika::otel",
        event = WEBHOOK_TURN_OUTCOME_EVENT,
        agent_id,
        request_id,
        outcome,
        "issue du tour webhook, attestée par le modèle (mika#2855)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le prompt **embarqué**, pas celui de l'arbre : `include_str!` fige les
    /// octets au build, exactement comme `build.rs` les fige dans
    /// `BUNDLED_SKILL_MANIFESTS`. Une édition de l'arbre est invisible aux deux
    /// jusqu'au prochain `make deploy` — la chaîne build → ensemencement →
    /// lecture (motif mika#2340). Même idiome que
    /// `bundled_skills::tests::test_skill_review_prompt_lists_trust_critical_skills`.
    const EMBEDDED_PROMPT: &str =
        include_str!("../../../../skills/bundled/qa-review-webhook-success/system_prompt.md");

    /// Lignes du prompt où [`ATTESTATION_PREFIX`] apparaît SANS être suivi d'une
    /// valeur du contrat. **LIVRÉE VIDE** : le prompt est écrit dans ce même vol,
    /// donc aucune occurrence hors contrat n'existe à l'atterrissage.
    ///
    /// Toute entrée future doit nommer la ligne exacte, référencer son ticket de
    /// suivi, et disparaître quand la ligne disparaît — l'assertion
    /// auto-nettoyante de `mika2855_le_prompt_embarque_et_lenum_ne_divergent_pas`
    /// rougit sur une entrée stale.
    const PROMPT_PARITY_ALLOWLIST: &[&str] = &[];

    /// Le prédicat de parité, sens 2 : les valeurs du prompt que le contrat
    /// refuse. Sorti du test pour que le contrôle de bonne foi l'exerce sur un
    /// prompt synthétique — une allowlist vide sur un détecteur qui ne détecte
    /// rien est indiscernable d'un détecteur inerte.
    ///
    /// La valeur est le plus long préfixe de `[a-z_]` après le marqueur : c'est
    /// l'alphabet des six termes, donc une occurrence sans valeur (`<valeur>`,
    /// une cellule vide) rend `""`, que [`Attested::from_str`] refuse.
    fn attestations_hors_contrat(prompt: &str) -> Vec<String> {
        prompt
            .lines()
            .filter(|line| !PROMPT_PARITY_ALLOWLIST.iter().any(|exc| line.contains(exc)))
            .flat_map(|line| line.split(ATTESTATION_PREFIX).skip(1))
            .map(|rest| {
                rest.trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                    .collect::<String>()
            })
            .filter(|value| Attested::from_str(value).is_err())
            .collect()
    }

    /// AC1 — un rouge par valeur attestable. Les six termes se lisent, et le nom
    /// qui part sur le fil est celui que le prompt demande.
    #[test]
    fn mika2855_les_six_termes_attestes_se_lisent() {
        assert_eq!(WEBHOOK_TURN_OUTCOME_EVENT, "webhook_turn_outcome");
        assert_eq!(ATTESTATION_PREFIX, "Webhook-Outcome:");

        for attested in Attested::ALL {
            let text = format!("Done.\n\n{ATTESTATION_PREFIX} {}\n", attested.as_str());
            assert_eq!(
                parse_attestation(Some(&text)),
                Outcome::Attested(attested),
                "le terme {:?} ne se lit pas depuis sa propre forme de fil",
                attested.as_str()
            );
            assert_eq!(
                parse_attestation(Some(&text)).as_str(),
                attested.as_str(),
                "le terme attesté ne part pas sur le fil sous son propre nom"
            );
        }

        // Les six noms sont distincts : sans cela deux § du prompt tomberaient
        // dans un seul seau de l'agrégat, sans qu'aucune autre assertion ne le
        // voie (classe mika#2205).
        let mut names: Vec<&str> = Attested::ALL.iter().map(|a| a.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            Attested::ALL.len(),
            "deux termes partagent un nom"
        );
    }

    /// **AC2** — un tour qui n'atteste rien rend `outcome_absent`, et pas un
    /// arrêt deviné. Le texte porte ici une trace d'outils qui ressemble à
    /// l'arrêt 1 : une déduction la classerait, l'attestation refuse.
    #[test]
    fn mika2855_un_tour_qui_natteste_rien_rend_absent() {
        let text = "I ran `pr list --state open` and found no matching PR, so there \
                    is nothing to review. Stopping the turn.";

        assert_eq!(parse_attestation(Some(text)), Outcome::Absent);
        assert_eq!(parse_attestation(Some(text)).as_str(), "outcome_absent");
    }

    /// **AC2**, seconde moitié — `None` (branche `Err` de la boucle : aucun
    /// `AgentOutput`, donc aucun texte) rend `Absent`, jamais un arrêt. Un tour
    /// mort n'a rien attesté, et c'est vrai.
    #[test]
    fn mika2855_un_tour_sans_texte_rend_absent() {
        assert_eq!(parse_attestation(None), Outcome::Absent);
        assert_eq!(parse_attestation(Some("")), Outcome::Absent);
        assert_eq!(parse_attestation(Some("   \n\n  ")), Outcome::Absent);
    }

    /// Décision 3 — une valeur hors contrat n'est pas une absence. Les deux faits
    /// sont de signe opposé : l'un appelle un durcissement de prompt, l'autre une
    /// correction du contrat.
    #[test]
    fn mika2855_une_valeur_hors_contrat_nest_pas_une_absence() {
        for hors_contrat in [
            "blah",
            "stop_inexistant",
            "STOP_NO_OPEN_PR",
            "stop_no_open_pr_bis",
            "",
        ] {
            let text = format!("{ATTESTATION_PREFIX} {hors_contrat}");
            assert_eq!(
                parse_attestation(Some(&text)),
                Outcome::Unreadable,
                "{hors_contrat:?} devrait être illisible, pas absent"
            );
        }

        assert_ne!(Outcome::Unreadable.as_str(), Outcome::Absent.as_str());
        assert_eq!(Outcome::Unreadable.as_str(), "outcome_unreadable");

        // Casse du préfixe : un contrat, pas une intention à reconnaître au
        // lexique (mika#2136 D4). Un préfixe mal casé n'est pas une attestation
        // du tout, donc une absence.
        assert_eq!(
            parse_attestation(Some("webhook-outcome: review")),
            Outcome::Absent
        );
    }

    /// Décision 4 — la dernière occurrence gagne. Le modèle cite le contrat avant
    /// de l'honorer ; prendre la première transformerait une intention en fait.
    #[test]
    fn mika2855_la_derniere_occurrence_gagne() {
        let text = format!(
            "I will emit `{ATTESTATION_PREFIX} review` once the review completes.\n\
             The review could not proceed: the plan document is unreachable.\n\
             Escalated to the operator and set the task to blocked.\n\
             {ATTESTATION_PREFIX} review_failed\n"
        );

        assert_eq!(
            parse_attestation(Some(&text)),
            Outcome::Attested(Attested::ReviewFailed)
        );

        // Tolérant sur la position : une attestation correcte mais suivie de
        // prose reste une obéissance.
        let mal_placee = format!("{ATTESTATION_PREFIX} review\n\nDispatched.\n");
        assert_eq!(
            parse_attestation(Some(&mal_placee)),
            Outcome::Attested(Attested::Review)
        );

        // Les blancs de bord ne comptent pas — ni devant le préfixe, ni autour
        // de la valeur.
        let indentee = format!("  \t{ATTESTATION_PREFIX}   review   \n");
        assert_eq!(
            parse_attestation(Some(&indentee)),
            Outcome::Attested(Attested::Review)
        );
    }

    /// La frontière du type — les deux termes dérivés par le moteur ne sont
    /// atteignables depuis AUCUN texte. Si cette assertion tombe, un modèle peut
    /// écrire « je n'ai rien attesté », ce qui est une attestation.
    #[test]
    fn mika2855_absent_nest_pas_parsable() {
        for derive in ["outcome_absent", "outcome_unreadable"] {
            assert!(
                Attested::from_str(derive).is_err(),
                "{derive} est un terme du MOTEUR : aucun texte ne doit pouvoir le produire"
            );
            let text = format!("{ATTESTATION_PREFIX} {derive}");
            assert_eq!(
                parse_attestation(Some(&text)),
                Outcome::Unreadable,
                "un modèle qui écrit {derive} est illisible, pas absent"
            );
        }
    }

    /// Décision 7 — la parité prompt ↔ enum, dans les **deux sens**, sur le
    /// prompt embarqué. C'est la classe que ce dépôt a payée deux fois
    /// (mika#2158 `grooming_marker`, mika#2362 le seuil de retry recopié) : deux
    /// copies qui dérivent en silence, et ici la dérive rendrait
    /// `outcome_unreadable` sur un terme que le prompt demande, avec un agrégat
    /// qui aurait l'air plein.
    ///
    /// La bidirectionnalité est ce qui empêche l'assertion de devenir l'allowlist
    /// que mika#2309 condamne : ajouter un terme au prompt sans l'ajouter à
    /// l'enum rougit, et retirer un terme de l'enum sans le retirer du prompt
    /// rougit aussi.
    #[test]
    fn mika2855_le_prompt_embarque_et_lenum_ne_divergent_pas() {
        // Sens 1 — chacun des six termes apparaît littéralement dans le prompt,
        // précédé du préfixe.
        for attested in Attested::ALL {
            let attendu = format!("{ATTESTATION_PREFIX} {}", attested.as_str());
            assert!(
                EMBEDDED_PROMPT.contains(&attendu),
                "le prompt embarqué de qa-review-webhook-success ne demande pas {attendu:?} — \
                 un terme de l'enum que le modèle ne sait pas attester"
            );
        }

        // Sens 2 — toute occurrence du préfixe dans le prompt porte une valeur
        // que `Attested::from_str` accepte.
        let hors_contrat = attestations_hors_contrat(EMBEDDED_PROMPT);
        assert!(
            hors_contrat.is_empty(),
            "le prompt embarqué demande des valeurs hors contrat : {hors_contrat:?} — \
             les ajouter à Attested, ou corriger le prompt"
        );

        // Une exception qui ne correspond plus à aucune ligne du prompt est une
        // exception stale : elle rougit ici plutôt que de rester à couvrir un cas
        // disparu. C'est ce qui empêche cette allowlist de devenir celle que
        // mika#2309 condamne.
        for exc in PROMPT_PARITY_ALLOWLIST {
            assert!(
                EMBEDDED_PROMPT.contains(exc),
                "exception stale dans PROMPT_PARITY_ALLOWLIST : {exc:?} n'est plus \
                 dans le prompt embarqué — retirer l'entrée"
            );
        }

        // Contrôle de bonne foi — une allowlist vide sur un détecteur qui ne
        // détecte rien est indiscernable d'un détecteur inerte. Le prédicat doit
        // REFUSER un prompt synthétique hors contrat.
        let synthetique =
            format!("## Outcome attestation\n\n`{ATTESTATION_PREFIX} stop_inexistant` — § 7\n");
        assert_eq!(
            attestations_hors_contrat(&synthetique),
            vec!["stop_inexistant".to_string()],
            "le prédicat de parité accepte une valeur hors contrat : il est inerte"
        );
    }
}
