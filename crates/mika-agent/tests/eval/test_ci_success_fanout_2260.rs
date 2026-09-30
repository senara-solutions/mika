//! L'attribution du fan-out `check_suite.completed(success)` est mesurée, pas
//! déduite (mika#2260).
//!
//! # L'énoncé que ce fichier rend falsifiable
//!
//! Le ticket dit « les **deux** agents entrent encore dans `ci_success_handler` ».
//! C'est une affirmation sur *qui* a atteint un callsite, et le harness eval
//! mono-agent ne peut pas l'exprimer : toutes ses sondes d'audit sont scopées sur
//! l'unique `agent_id` `"mika"`, si bien qu'une assertion de cette forme y est
//! vacuously satisfaite. D'où [`MultiAgentHarness`] (mika#2265), dont le
//! doc-comment nomme ce ticket : *« le mot load-bearing est **attribution** »*.
//!
//! # Les deux patrons, et ce que le second N'EST PAS
//!
//! `multi_agent.rs` documente la raison d'être des deux patrons en une phrase :
//! **#2248 est un *ordre* et #2260 est une *course***. Les deux sous-cas vivent
//! donc dans le même test, chacun sur son propre harness — les comptes d'audit
//! s'accumuleraient sinon, et `1` est une assertion plus lisible que `2`.
//!
//! **Mais le sous-cas (b) n'exerce PAS une course, et c'est à dire plutôt qu'à
//! laisser croire.** `tokio::join!` sonde les deux futurs sur une seule tâche,
//! sans `spawn` ; et à `github_token = None` le futur du dispatcher n'atteint
//! **aucun point `.await`** avant de rendre la main — `parse_check_suite_success`,
//! `db.agent_id()`, `owns_merge_transition` et les deux macros de journal sont
//! synchrones, puis `match github_token { None => return … }`. Le premier sondage
//! le résout donc, il ne se suspend jamais, et aucun entrelacement n'a lieu.
//!
//! Ce que (b) établit réellement : **le verdict est une fonction pure de
//! l'`agent_id`**, donc indépendant de l'ordre d'arrivée par construction. C'est
//! plus faible que « la course se résout bien », et strictement moins que ce que
//! le patron promet. La course authentique — deux évaluateurs suspendus en même
//! temps sur la clé de dedup process-globale — n'est atteignable qu'avec un vrai
//! token et une couture sur `gh`, que mika#1947 a déclarée hors périmètre. Le
//! sous-cas est conservé parce qu'il est bon marché et qu'il épingle
//! l'indépendance à l'ordre ; il n'est pas conservé comme preuve de course.
//!
//! # Ce que ce fichier mesure, et ce qu'il ne mesure pas
//!
//! Sans réseau, `github_token = None`, le seul chemin qui franchit la porte
//! s'arrête sur l'exigence de token — la **première** ligne après elle. Donc :
//!
//! - `ci_success_handler_skipped_not_merge_actor` **discrimine** : une valeur non
//!   nulle pour `mika-qa` et zéro pour `mika-dev` est l'attribution recherchée.
//! - `ci_success_handler_processed` reste à **zéro pour les deux**, et c'est
//!   attendu : nul n'atteint le dedup sans token. Ce compte est asserté comme
//!   **contrôle de non-régression**, jamais comme discriminant — le lire comme
//!   une preuve d'attribution serait une assertion vacue de plus, exactement ce
//!   que ce fichier existe pour retirer. Le vrai contrôle positif de l'entrée de
//!   `mika-dev` est son **verdict** : lui seul porte l'enrichissement de token.
//!
//! Corollaire du même fait, et il vaut d'être écrit (R4 du plan) : puisque le
//! token manque, `try_dedup_check_suite` n'est jamais atteint et la map
//! **globale au processus** de `check_suite_dedup` n'est pas touchée — l'ordre
//! d'exécution des tests est donc sans effet ici. Tout test futur de ce chemin
//! **avec un token réel** doit porter un `(repo, branch)` unique, sous peine de
//! pollution croisée dans le même binaire.

use std::collections::BTreeMap;

use anyhow::Result;

use mika_agent::server::ci_success_handler::try_handle_ci_success;
use mika_agent::server::verdict_handler::VerdictAction;

use super::multi_agent::MultiAgentHarness;

const DISPATCHER: &str = "mika-dev";
const REVIEWER: &str = "mika-qa";

const SKIPPED_EVENT: &str = "ci_success_handler_skipped_not_merge_actor";
const PROCESSED_EVENT: &str = "ci_success_handler_processed";

/// Le texte tel que le gateway le formate.
const CHECK_SUITE_TEXT: &str =
    "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2260/fanout)";

/// Le verdict de `mika-dev` atteste qu'il a franchi la porte : il est le seul à
/// atteindre la ligne suivante, l'exigence de token.
fn assert_dispatcher_entered(action: &VerdictAction, patron: &str) {
    match action {
        VerdictAction::Passthrough {
            enrichment: Some(s),
        } => assert!(
            s.contains("no GitHub token"),
            "{patron} : `{DISPATCHER}` doit franchir la porte et s'arrêter sur l'exigence de \
             token — c'est le contrôle positif de l'attribution. Enrichissement : {s}"
        ),
        other => panic!(
            "{patron} : `{DISPATCHER}` possède la transition de merge, la porte doit le laisser \
             passer. Obtenu : {other:?}"
        ),
    }
}

/// Le relecteur est transparent : `Passthrough` sans enrichissement, pour que son
/// tour voie le texte brut de l'événement.
fn assert_reviewer_transparent(action: &VerdictAction, patron: &str) {
    match action {
        VerdictAction::Passthrough { enrichment: None } => {}
        other => panic!(
            "{patron} : `{REVIEWER}` ne peut pas consommer le signal, il ne doit donc pas \
             l'évaluer — et son tour doit recevoir l'événement brut. Obtenu : {other:?}"
        ),
    }
}

/// L'attribution attendue : une seule valeur non nulle, et c'est le relecteur.
///
/// La carte rendue par `audit_counts_by_agent` porte **toujours** une entrée par
/// agent monté, y compris à zéro — la discrimination se lit donc sur les valeurs,
/// jamais sur la cardinalité.
fn assert_attribution(counts: &BTreeMap<String, i64>, patron: &str) {
    assert_eq!(
        counts.get(REVIEWER).copied(),
        Some(1),
        "{patron} : le refus du relecteur doit être compté sur SA tranche d'audit — \
         c'est l'attribution que le harness mono-agent ne peut pas produire. Carte : {counts:?}"
    );
    assert_eq!(
        counts.get(DISPATCHER).copied(),
        Some(0),
        "{patron} : le dispatcher ne doit JAMAIS être écarté ; sans cette moitié, une porte \
         qui refuserait tout le monde satisferait l'assertion ci-dessus. Carte : {counts:?}"
    );
}

/// T2 (AC2, AC7) — l'attribution du fan-out est mesurable, dans les deux patrons.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mika2260_lattribution_du_fanout_est_mesurable() -> Result<()> {
    // --- (a) Ordre déterministe : le primaire, puis le secondaire. ---
    {
        let h = MultiAgentHarness::builder()
            .agent(DISPATCHER)
            .agent(REVIEWER)
            .build()?;

        for (agent_id, db) in h.agents() {
            let action = try_handle_ci_success(
                CHECK_SUITE_TEXT,
                db,
                None,
                None,
                h.session_id(agent_id),
                "trace-2260-fanout-ordre",
            )
            .await;

            if agent_id == DISPATCHER {
                assert_dispatcher_entered(&action, "ordre");
            } else {
                assert_reviewer_transparent(&action, "ordre");
            }
        }

        assert_attribution(&h.audit_counts_by_agent(SKIPPED_EVENT).await?, "ordre");

        // Contrôle de non-régression, explicitement pas un discriminant : sans
        // token, nul n'atteint le dedup.
        for (agent_id, n) in h.audit_counts_by_agent(PROCESSED_EVENT).await? {
            assert_eq!(
                n, 0,
                "ordre : `{agent_id}` n'a pas de token, il ne peut pas atteindre le marqueur \
                 de dedup"
            );
        }

        // Contrôle positif du montage lui-même : deux bases en mémoire disjointes
        // rendraient toutes les assertions ci-dessus vertes en ne mesurant rien.
        assert!(
            h.cross_read_count(DISPATCHER, REVIEWER).await? > 0,
            "ordre : le dispatcher doit voir la tranche d'audit du relecteur — sinon le montage \
             n'est pas multi-agents sur une base partagée, et l'attribution mesurée est un \
             artefact (voir `multi_agent.rs` § Le montage à NE PAS refaire)"
        );

        h.shutdown();
    }

    // --- (b) Concurrence structurelle : les deux handlers sur la base partagée. ---
    //
    // Ce sous-cas atteste que le verdict est **indépendant de l'ordre**, parce
    // qu'il est une fonction pure de l'`agent_id`. Il n'atteste **pas** qu'une
    // course se résout bien : à `token = None` aucun des deux futurs ne se
    // suspend, donc aucun entrelacement n'a lieu — voir le § *ce que le second
    // N'EST PAS* en tête de fichier, qui nomme la limite et pourquoi la course
    // authentique est hors d'atteinte sans réseau.
    {
        let h = MultiAgentHarness::builder()
            .agent(DISPATCHER)
            .agent(REVIEWER)
            .build()?;

        let (dev_action, qa_action) = tokio::join!(
            try_handle_ci_success(
                CHECK_SUITE_TEXT,
                h.db(DISPATCHER),
                None,
                None,
                h.session_id(DISPATCHER),
                "trace-2260-fanout-course",
            ),
            try_handle_ci_success(
                CHECK_SUITE_TEXT,
                h.db(REVIEWER),
                None,
                None,
                h.session_id(REVIEWER),
                "trace-2260-fanout-course",
            )
        );

        assert_dispatcher_entered(&dev_action, "course");
        assert_reviewer_transparent(&qa_action, "course");

        assert_attribution(&h.audit_counts_by_agent(SKIPPED_EVENT).await?, "course");

        for (agent_id, n) in h.audit_counts_by_agent(PROCESSED_EVENT).await? {
            assert_eq!(
                n, 0,
                "course : `{agent_id}` n'a pas de token, il ne peut pas atteindre le marqueur \
                 de dedup"
            );
        }

        assert!(
            h.cross_read_count(DISPATCHER, REVIEWER).await? > 0,
            "course : le partage de la base doit être effectif"
        );

        h.shutdown();
    }

    Ok(())
}
