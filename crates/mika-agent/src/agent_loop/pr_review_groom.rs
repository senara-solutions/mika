//! Ce que ce tour-ci **est**, et pourquoi un territoire de handler n'a pas de
//! groom à faire (mika#3004).
//!
//! Ce module porte le raisonnement de mika#3004 et le type qui le tient —
//! [`ProprietesDuTour`] — pour que `agent_loop/mod.rs` n'en garde que le
//! câblage. Il vit à part pour une raison mesurée : `mod.rs` est à 4 Ko du
//! plafond d'un méga-octet de `scripts/check-secrets.sh`, et le premier vol de
//! ce ticket a fait rougir le Secret Scan en y ajoutant ~27 Ko (run CI
//! 41321165649). La prose argumentative est donc **ici**, et chaque site de
//! `mod.rs` n'y renvoie.
//!
//! # L'incident, et ses deux causes
//!
//! Le 2026-10-30 à 17:14:38Z, trace `3b771914-d485-11f1-8e41-923f9687bff8`, un
//! tour de mika-dev déclenché par un webhook `pr_review` sur mika#2968 — dont la
//! PR #2982 était **ouverte** et le plan convergé — s'est vu imposer
//! `run_claude_pilot_groom` comme `required_tools`. Il a pris le re-prompt
//! *« agent responded without calling required tools »*, créé une parente
//! « Groom mika#2968 », et ses deux enfants ont rendu `auto_skipped` /
//! `already_groomed` en trois secondes. Coût : deux tours A2A pour aucun travail.
//!
//! La branche 4 de mika#2757 — *« le tour interdit le dispatch »* — n'a pas
//! mordu, pour **deux** raisons cumulées, et c'est la seconde qui décide de la
//! forme de la correction :
//!
//! - *cause A* — le territoire des handlers qa / ci est **sorti** du domaine
//!   Webhook Fallthrough par conception (`self-dev-webhook-qa` / `-ci` y portent
//!   des dispatchs légitimes), donc `interdit_dispatch` y vaut `false` ;
//! - *cause B* — le texte jugé était celui d'**après** les handlers. Sur ce
//!   tour-là `verdict_handler` avait remplacé `req.text` par un pré-digest
//!   commençant par `<verdict_handler>\n`, si bien que tout prédicat de préfixe
//!   répondait *« ce n'est pas un événement GitHub »* sur un tour qui en était
//!   un.
//!
//! Un vol qui n'aurait traité que la cause A aurait été **vert sur une fixture
//! d'événement brut et rouge en production**. C'est pourquoi
//! `AgentParams::webhook_event_text` voyage depuis `server/handlers.rs`, et
//! pourquoi la fixture porte les **deux** textes à la fois.
//!
//! # Pourquoi pas une lecture ligne à ligne du texte d'après
//!
//! Chercher le préfixe sur *chaque* ligne du pré-digest aurait évité de faire
//! voyager un second texte, et c'est précisément ce qui a été refusé : un tour de
//! fallthrough légitime qui **cite** une ligne d'événement de PR — un corps de
//! ticket, un commentaire d'opérateur — se serait vu désarmer un groom qu'il
//! devait faire. C'est la classe `mask_cited_branch` (mika#2755 / mika#2768) :
//! un prédicat qui lit une citation comme un fait.
//!
//! # Pourquoi la branche 5 ne porte QUE l'outil de groom
//!
//! `run_claude_pilot` reste exigible sur ce même territoire, et ce n'est pas une
//! timidité : `self-dev-webhook-qa` relance un pilote d'implémentation sur un
//! verdict `hold`, `self-dev-webhook-ci` en relance un sur un CI rouge. Élargir
//! la branche à `is_dispatch_tool` fermerait ces deux flux — et tous les tests du
//! moteur resteraient verts. Ce qui est impossible sur ce territoire est le
//! *groom* : un handler y a déjà traité l'événement, et un événement de PR est
//! par construction postérieur au plan.
//!
//! # Pourquoi la cinquième position
//!
//! La position qui compte est celle vis-à-vis de la branche 3 (`NonServi`). Sur
//! un tour `check_suite`, `filter_ci_excluded_tools` (mika#1324) a déjà retiré
//! les deux outils de dispatch du tableau servi : la branche 3 répond donc
//! **première** là, et la branche 5 n'y est jamais atteinte en production. C'est
//! voulu — le ticket demande que l'exigence ne tienne pas, pas qu'elle tombe sous
//! un nom particulier — mais c'est aussi pourquoi on ne peut pas épingler la
//! branche 5 avec une fixture `check_suite` *servie* : cet état-là, la production
//! ne le produit pas, et un test qui le fabrique mesure une **forme** et non un
//! tour. [`tests::mika3004_un_check_suite_repond_sous_non_servi_en_production`]
//! épingle les deux faits dans le même test, et le dit dans ses messages.
//!
//! Vis-à-vis de la branche 4, l'ordre est libre : `interdit_dispatch` et
//! `territoire_handler` ne se recouvrent jamais — le domaine Fallthrough lit le
//! territoire des handlers **en négation** (mika#2517 D1), et
//! `mika2653_les_deux_axes_de_tour_webhook_sont_exclusifs` le tient.
//!
//! # Pourquoi `INFO` et non `WARN`
//!
//! Le désarmement est *nominal* sur ce domaine : il tirera des dizaines de fois
//! par jour pour un comportement correct, et un `WARN` à ce volume est un `WARN`
//! qu'on apprend à ignorer (les « 25+ documented misfires » de mika#1469). Son
//! `reason = "handler_territory_forbids_groom"` lui est propre, donc un
//! `GROUP BY reason` la sépare des quatre autres : deux populations sous un même
//! nom ne sont pas soustractibles (mika#2205). **L'absence de cette ligne ne dit
//! rien d'autre que « le binaire précède le vol »** tant que `make deploy` n'a
//! pas tourné (mika#2340) : l'instrument n'est pas vivant avant.

use super::is_unauthorized_webhook_dispatch;

/// Ce que ce tour-ci est, lu **une fois** pour l'ensemble des exigences
/// (mika#3004).
///
/// # Pourquoi un struct et pas deux booléens de plus
///
/// mika#2757 a établi que le domaine d'un tour est *« une propriété du tour, pas
/// de l'outil »* et a fait entrer `turn_forbids_dispatch` dans
/// [`super::raison_de_desarmement`] en `bool` déjà calculé, pour qu'aucune
/// branche ne soit tentée de le réévaluer par outil. mika#3004 ajoute une
/// **seconde** propriété du même tour, lue sur le même texte. Deux booléens
/// parallèles auraient laissé un appelant les calculer sur deux textes
/// différents — et c'est exactement l'erreur que ce ticket corrige, une couche
/// plus haut (cause B du doc-module).
///
/// Un seul constructeur ([`ProprietesDuTour::depuis`]), des champs privés et pas
/// de `Default` : il n'existe aucune façon d'obtenir une valeur de ce type dont
/// les deux champs ne viennent pas du **même** octet de texte. C'est ce que le
/// type tient, et c'est pourquoi aucun scan de source n'a eu à le tenir.
#[derive(Debug, Clone, Copy)]
pub(super) struct ProprietesDuTour {
    /// Ce tour est dans le domaine Webhook Fallthrough : il interdit à un outil
    /// de dispatch de réussir (mika#2757, branche 4).
    pub(super) interdit_dispatch: bool,
    /// Ce tour est le territoire d'un handler de `server/` — un événement de PR
    /// ou de suite de checks que `self-dev-webhook-qa` / `-ci` possèdent
    /// (mika#3004, branche 5).
    pub(super) territoire_handler: bool,
}

impl ProprietesDuTour {
    /// Les deux propriétés, dérivées du texte qui dit ce que ce tour **est**.
    ///
    /// # Le texte jugé, et pourquoi c'est celui-là
    ///
    /// `webhook_event_text` est ce que le gateway a émis, avant tout handler de
    /// `server/`. Quand il est `Some`, c'est lui qui est jugé — jamais
    /// `turn_message`, qui sur un tour webhook a déjà été réécrit. C'est la
    /// cause B du doc-module, et la cause directe de l'incident.
    ///
    /// # Pourquoi `unwrap_or` et non deux chemins
    ///
    /// Hors du chemin webhook il n'existe pas de texte d'avant qui diffère :
    /// `turn_message` **est** le texte du tour. La retombée préserve donc octet
    /// pour octet le comportement de mika#2757 sur les tours Telegram, CLI, A2A
    /// et de callback — un `None` n'est pas une absence d'information, c'est
    /// l'information que les deux textes coïncident.
    ///
    /// # Aucun littéral de préfixe ici
    ///
    /// Les deux bits sortent des deux prédicats de `webhook_dispatch`, jamais
    /// d'une recopie de leurs préfixes : un seul corps pose la conjonction, deux
    /// questions la lisent de signes opposés
    /// (`mika2517_the_fallthrough_domain_has_a_single_definition`).
    pub(super) fn depuis(webhook_event_text: Option<&str>, turn_message: &str) -> Self {
        let texte = webhook_event_text.unwrap_or(turn_message);
        Self {
            interdit_dispatch: is_unauthorized_webhook_dispatch(texte),
            territoire_handler: crate::webhook_dispatch::is_webhook_pr_event_domain(texte),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{
        TICKET_2750_CLOSED_MSG, make_registry_with_tool, make_resolved_skill_tool, served,
    };
    use super::super::{RaisonDesarmement, filter_available_required_tools, raison_de_desarmement};
    use super::ProprietesDuTour;
    use crate::skills::index::ResolvedSkillTool;
    use std::collections::{HashMap, HashSet};

    /// Le texte **brut** de l'événement webhook de la trace
    /// `3b771914-d485-11f1-8e41-923f9687bff8` (AC3), tel que la passerelle le
    /// compose pour un `pull_request_review` (`mika-gateway/src/github.rs`, bras
    /// `"pull_request_review"`).
    ///
    /// **Ce que la trace atteste** : le dépôt, le ticket mika#2968, la PR #2982
    /// ouverte, le déclencheur `pr_review`, le candidat `review_verdict`. **Ce
    /// qu'elle n'atteste pas** : l'état de la revue, son auteur et le titre de la
    /// PR — remplis ici par les valeurs nominales du flux qa. Aucune assertion
    /// ci-dessous ne dépend d'un de ces trois champs : le prédicat interrogé ne
    /// lit que le **préfixe** de la première ligne.
    const PR_REVIEW_3004_EVENT: &str = "[GitHub] PR review (changes_requested) on \
         senara-solutions/mika#2968 (fix(2968)) by @mika-platform-qa\n\
         https://github.com/senara-solutions/mika/pull/2982";

    /// Le **pré-digest** que `verdict_handler` substitue à `req.text` avant
    /// `run_agent`, sur le modèle exact de ses dix sites de construction.
    ///
    /// **C'est la cause B du vol, et elle tient à une seule propriété** : ce texte
    /// ne commence pas par le préfixe d'un événement de PR — il commence par
    /// `<verdict_handler>`, et la ligne de l'événement y est la **deuxième**. Un
    /// prédicat de préfixe interrogé là-dessus répond donc « hors territoire des
    /// handlers » sur le tour même qu'un handler vient de traiter.
    const PR_REVIEW_3004_PRE_DIGEST: &str = "<verdict_handler>\n\
         [GitHub] PR review (changes_requested) on senara-solutions/mika#2968 by \
         @mika-platform-qa\n\
         VERDICT: block[ac] — un correctif est déjà en vol pour la tâche c4776ac0.\n\n\
         Wait for the active callback to finish before taking further action.\n\
         </verdict_handler>";

    /// Un `check_suite` brut, sur le modèle de `mika-gateway/src/github.rs` bras
    /// `"check_suite"` (trois lignes depuis mika#2917 ; seule la première est
    /// lue par les quatre lecteurs du moteur).
    const CHECK_SUITE_3004_EVENT: &str = "[GitHub] Check suite failure on \
         senara-solutions/mika (branch: fix/2968/relance-de-groom)\n\
         head: 0000000000000000000000000000000000000000\n\
         prs: #2982";

    /// **Le cœur d'AC1 et la première moitié d'AC2.** Le tour `pr_review` de la
    /// fixture n'exige plus `run_claude_pilot_groom`.
    ///
    /// La seconde assertion est ce qui **nomme la cause B** : jugée sur le seul
    /// texte du tour — exactement ce que faisait la production avant ce vol —
    /// l'exigence survit intacte. Les deux appels ne diffèrent que par le texte
    /// d'événement, donc ce test sépare « la cinquième branche existe » de « la
    /// cinquième branche est alimentée par le bon texte ». Sans lui, un vol qui
    /// n'aurait traité que la cause A serait vert ici et rouge en production.
    #[test]
    fn mika3004_un_tour_pr_review_desarme_lexigence_de_groom() {
        let tools = make_registry_with_tool("run_claude_pilot_groom");
        let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
        let required: HashSet<String> = ["run_claude_pilot_groom".to_string()].into();

        let effective = filter_available_required_tools(
            &required,
            &tools,
            &skill_map,
            None,
            &served(&["run_claude_pilot_groom"]),
            PR_REVIEW_3004_PRE_DIGEST,
            Some(PR_REVIEW_3004_EVENT),
        );

        assert!(
            effective.is_empty(),
            "sur un tour `pr_review` d'un ticket dont la PR est ouverte, l'outil de groom \
             ne peut pas être exigé — exigence restée : {effective:?}"
        );

        let juge_sur_le_pre_digest = filter_available_required_tools(
            &required,
            &tools,
            &skill_map,
            None,
            &served(&["run_claude_pilot_groom"]),
            PR_REVIEW_3004_PRE_DIGEST,
            None,
        );

        assert_eq!(
            juge_sur_le_pre_digest, required,
            "contrôle de la cause B : le pré-digest de `verdict_handler` ne porte pas le \
             préfixe de territoire en première ligne, donc un désarmement qui ne lirait que \
             le texte du tour laisserait l'exigence debout — c'est l'état mesuré le \
             2026-10-30 à 17:14:38Z, trace 3b771914"
        );
    }

    /// **Le contrôle négatif de la largeur de la branche.** Sur ce même tour,
    /// `run_claude_pilot` reste exigible.
    ///
    /// Ce n'est pas une précaution : `self-dev-webhook-qa` relance un pilote
    /// d'implémentation sur un verdict `hold`, et `self-dev-webhook-ci` en relance
    /// un sur un CI rouge. Une branche 5 élargie à `is_dispatch_tool` fermerait
    /// ces deux flux — et tous les autres tests du moteur resteraient verts.
    ///
    /// Les deux exigences voyagent **ensemble** dans le même appel : un lot vide
    /// et un lot correctement amputé ne doivent pas rendre les mêmes octets
    /// (classe mika#2205).
    #[test]
    fn mika3004_le_meme_tour_laisse_run_claude_pilot_exigible() {
        let tools = make_registry_with_tool("run_claude_pilot_groom");
        let run_claude_pilot = make_resolved_skill_tool("run_claude_pilot");
        let mut skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
        skill_map.insert("run_claude_pilot".to_string(), &run_claude_pilot);
        let required: HashSet<String> = [
            "run_claude_pilot_groom".to_string(),
            "run_claude_pilot".to_string(),
        ]
        .into();

        let effective = filter_available_required_tools(
            &required,
            &tools,
            &skill_map,
            None,
            &served(&["run_claude_pilot_groom", "run_claude_pilot"]),
            PR_REVIEW_3004_PRE_DIGEST,
            Some(PR_REVIEW_3004_EVENT),
        );

        assert_eq!(
            effective,
            HashSet::from(["run_claude_pilot".to_string()]),
            "seul le groom est impossible sur un territoire de handler ; le dispatch \
             d'implémentation y est un flux légitime (relance sur `hold`, relance sur CI \
             rouge)"
        );
    }

    /// **L'ordre des branches 3 et 5, et l'état que la production ne produit
    /// pas.** Les deux faits sont dans le même test parce que le second n'a de
    /// sens qu'énoncé à côté du premier.
    ///
    /// *Premier fait, celui de la production* : sur un tour `check_suite`,
    /// `filter_ci_excluded_tools` (mika#1324) a déjà retiré les deux outils de
    /// dispatch du tableau servi. La branche **3** répond donc première, et la
    /// branche 5 n'est jamais atteinte là. C'est voulu — AC1 demande que
    /// l'exigence ne tienne pas, pas qu'elle tombe sous un nom particulier.
    ///
    /// *Second fait* : si l'outil arrive quand même *servi*, la branche 5 répond.
    /// Cet état-là, la production ne le produit pas ; ce cas mesure donc une
    /// **forme** et non un tour, et il le dit dans son message d'assertion pour
    /// qu'on ne le lise jamais comme une mesure de la production.
    #[test]
    fn mika3004_un_check_suite_repond_sous_non_servi_en_production() {
        let tools = make_registry_with_tool("run_claude_pilot_groom");
        let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
        let proprietes = ProprietesDuTour::depuis(Some(CHECK_SUITE_3004_EVENT), "peu importe");

        assert_eq!(
            raison_de_desarmement(
                "run_claude_pilot_groom",
                &tools,
                &skill_map,
                None,
                &served(&[]),
                proprietes
            ),
            Some(RaisonDesarmement::NonServi),
            "en PRODUCTION, `filter_ci_excluded_tools` (mika#1324) a déjà retiré l'outil du \
             tableau servi sur un tour `check_suite` : la branche 3 répond première et la \
             branche 5 n'y est jamais atteinte — si cette assertion rougit, c'est que \
             l'exclusion ci a changé, pas que le désarmement est cassé"
        );

        assert_eq!(
            raison_de_desarmement(
                "run_claude_pilot_groom",
                &tools,
                &skill_map,
                None,
                &served(&["run_claude_pilot_groom"]),
                proprietes
            ),
            Some(RaisonDesarmement::TerritoireSansGroom),
            "état que la production ne produit PAS (un `check_suite` ne sert pas les outils \
             de dispatch) : ce cas épingle la forme de la branche 5, pas un tour réel — il \
             est ce qui garantit qu'elle n'est pas morte si l'exclusion ci disparaissait"
        );
    }

    /// **Les deux bits se lisent sur UN texte, et ils sont disjoints.**
    ///
    /// `mika2653_les_deux_axes_de_tour_webhook_sont_exclusifs` tient déjà la
    /// disjonction des deux **prédicats** sur deux matrices. Ce que ce test
    /// ajoute, et qu'aucun test de `webhook_dispatch` ne peut voir, est la
    /// propriété de [`ProprietesDuTour::depuis`] : le texte jugé est **le même**
    /// pour les deux. Deux `bool` parallèles auraient permis de les calculer sur
    /// deux textes différents — un sur l'événement, l'autre sur le tour — et la
    /// disjonction aurait cessé de tenir sans qu'aucun prédicat ne change.
    ///
    /// Le couple choisi le montre par l'absurde : le texte du tour est un
    /// `issues.closed`, du domaine Fallthrough, donc `interdit_dispatch` serait
    /// vrai s'il était lu ; c'est le texte d'événement qui décide, et lui seul.
    #[test]
    fn mika3004_les_deux_proprietes_du_tour_sont_disjointes() {
        let sur_levenement =
            ProprietesDuTour::depuis(Some(PR_REVIEW_3004_EVENT), TICKET_2750_CLOSED_MSG);

        assert!(
            sur_levenement.territoire_handler,
            "le texte d'événement est celui d'une revue de PR : c'est lui qui est jugé"
        );
        assert!(
            !sur_levenement.interdit_dispatch,
            "et le texte du tour — un `issues.closed`, pourtant du domaine Fallthrough — ne \
             l'est pas : un seul texte alimente les deux bits"
        );

        for texte in [
            PR_REVIEW_3004_EVENT,
            CHECK_SUITE_3004_EVENT,
            TICKET_2750_CLOSED_MSG,
            "[GitHub] Issue labeled ready on senara-solutions/mika#1234 — un ticket prêt",
            "classe les notes de la semaine",
        ] {
            let p = ProprietesDuTour::depuis(None, texte);
            assert!(
                !(p.interdit_dispatch && p.territoire_handler),
                "les deux axes sont exclusifs par construction depuis mika#3004 — le domaine \
                 Fallthrough lit le territoire des handlers en négation ; texte : {texte:?}"
            );
        }
    }
}
