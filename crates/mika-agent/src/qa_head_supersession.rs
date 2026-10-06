//! Un callback de build QA dont la tête a été remplacée ne paie plus de tour LLM
//! (mika#2671, phase A).
//!
//! # Le défaut que ce module ferme
//!
//! Mesuré le 2026-10-06 sur la PR mika#2659 : le tour de revue lancé par un
//! premier `pull_request.synchronize` démarre `build_mika` à 14:22:20 ; dix
//! secondes plus tard un second `synchronize` remplace la tête. Le build finit,
//! son callback arrive à 14:28:49, et le moteur rejoue un tour LLM complet —
//! 21 appels, 1,03 M de tokens d'entrée — pour poster un verdict sur une tête
//! que GitHub avait remplacée huit minutes plus tôt. Sur l'épisode entier
//! (trois `synchronize` en douze minutes), ≈ 4,5 M de tokens d'entrée.
//!
//! # « Périmé » veut dire : un `synchronize` plus récent a été REÇU
//!
//! Pas « la tête GitHub a bougé ». La différence est ce qui empêche ce module de
//! perdre des revues. Le gateway supprime les `synchronize` sans changement de
//! fichiers (garde no-diff, #886 — un amend de trailer) : avec la vérité GitHub,
//! une tête déplacée par un amend rendrait le build « périmé », le callback serait
//! sauté, et **aucun autre événement** ne déclencherait la revue de la nouvelle
//! tête. Ici, on ne saute un tour que si l'événement qui déclenchera la revue de
//! la tête suivante **a déjà été reçu** : « la dernière tête est toujours revue »
//! (AC4) tient par construction. Corollaire : aucun appel GitHub, ni en
//! production ni à bouchonner en test.
//!
//! # Le registre
//!
//! Une ligne `audit_events` par `synchronize` reçu, écrite par
//! `server::handlers::handle_message` **avant** la bifurcation file / chemin
//! hérité : `tool_name = "qa_pr_sync_observed"`, `target_key = "pr:{repo}#{pr}"`.
//! Pas de table, pas de migration : la lecture passe par
//! `count_recent_audit_events_for_target`, et le registre est **par agent** — les
//! `synchronize` ne sont routés qu'à mika-qa, donc un build de mika-dev ne peut
//! jamais être déclaré périmé.
//!
//! # Fail-safe dans le sens de la revue
//!
//! Garde désarmée, cible PR illisible, registre illisible : le tour tourne, comme
//! avant ce module. La seule issue qui saute un tour est une observation
//! **positive** d'un `synchronize` postérieur au lancement du build.

use crate::server::deadline_verdict::PrTarget;

/// `audit_events.tool_name` d'un `synchronize` reçu. **SOLE WRITER** :
/// [`crate::server::handlers`], via [`sync_observed_key`].
pub const SYNC_OBSERVED_TOOL: &str = "qa_pr_sync_observed";

/// `audit_events.tool_name` et nom d'événement de journal d'un callback de
/// build sauté. **SOLE WRITER** : `task_engine::dispatcher`.
pub const BUILD_CALLBACK_SUPERSEDED_EVENT: &str = "qa_build_callback_superseded";

/// Nom d'événement d'un registre illisible : le tour tourne quand même.
pub const SUPERSESSION_UNREADABLE_EVENT: &str = "qa_build_callback_supersession_unreadable";

/// Kill-switch de la garde. Armée par défaut.
pub const STALE_BUILD_GUARD_ENV: &str = "MIKA_QA_STALE_BUILD_GUARD";

/// La clé du registre pour une PR. Écrivain et lecteur passent tous deux par
/// cette fonction : une grammaire de clé recopiée en deux sites est ce qui a
/// laissé deux lecteurs diverger dans mika#2158.
pub fn sync_observed_key(repo: &str, pr: u64) -> String {
    format!("pr:{repo}#{pr}")
}

/// La garde est-elle armée ? Lecture d'environnement, moitié impure de
/// [`parse_stale_build_guard`].
pub fn stale_build_guard_enabled() -> bool {
    parse_stale_build_guard(std::env::var(STALE_BUILD_GUARD_ENV).ok().as_deref())
}

/// Absent ou vide → armée ; `0`/`false`/`off`/`no` → désarmée ; une valeur non
/// reconnue est **dite** et laisse armée — même forme que
/// `MIKA_QA_CALLBACK_VERDICT_NET`. Une coquille ne doit pas désarmer en silence
/// une garde dont l'absence se lit exactement comme sa présence.
pub fn parse_stale_build_guard(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return true;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "true" | "on" | "yes" => true,
        "0" | "false" | "off" | "no" => false,
        other => {
            tracing::warn!(
                event = "qa_stale_build_guard_invalid",
                value = %format!("\"{other}\""),
                "valeur non reconnue pour {STALE_BUILD_GUARD_ENV} — la garde reste armée"
            );
            true
        }
    }
}

/// Ce que la garde décide d'un callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildCallbackHead {
    /// Le label n'est pas `long_running:build_mika` : hors population.
    NotABuildCallback,
    /// Callback de build sans cible PR lisible (build lancé hors d'un tour
    /// déclenché par un événement PR, ou metadata absente) : le tour tourne.
    NoPrTarget(&'static str),
    /// Aucun `synchronize` reçu depuis le lancement : la tête est courante.
    Current,
    /// Au moins un `synchronize` reçu depuis le lancement : le tour est sauté.
    Superseded { target: PrTarget, later_syncs: i64 },
    /// Le registre n'a pas pu être lu : le tour tourne (fail-safe revue).
    LedgerUnreadable { target: PrTarget },
}

impl BuildCallbackHead {
    /// Le seul état qui saute le tour.
    pub fn skips_turn(&self) -> bool {
        matches!(self, BuildCallbackHead::Superseded { .. })
    }
}

/// La décision, pure : aucun accès base, aucun réseau.
///
/// `read_target` lit la cible PR stampée ; `count_later_syncs` compte les
/// `synchronize` reçus pour cette cible **après** le lancement du build. Les deux
/// sont des paramètres pour que la fonction soit testable sans base, et pour que
/// le registre ne soit consulté que lorsque les termes moins chers ont tenu.
pub async fn decide_build_callback<R, C, Fut>(
    label: &str,
    metadata: Option<&str>,
    read_target: R,
    count_later_syncs: C,
) -> BuildCallbackHead
where
    R: FnOnce(Option<&str>) -> Result<PrTarget, &'static str>,
    C: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<i64>>,
{
    if !crate::qa_build_callback::is_build_callback_label(label) {
        return BuildCallbackHead::NotABuildCallback;
    }
    let target = match read_target(metadata) {
        Ok(t) => t,
        Err(reason) => return BuildCallbackHead::NoPrTarget(reason),
    };
    let key = sync_observed_key(&target.repo, target.pr_number);
    match count_later_syncs(key).await {
        Ok(n) if n > 0 => BuildCallbackHead::Superseded {
            target,
            later_syncs: n,
        },
        Ok(_) => BuildCallbackHead::Current,
        Err(_) => BuildCallbackHead::LedgerUnreadable { target },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qa_build_callback::BUILD_CALLBACK_LABEL;
    use crate::task_engine::dispatcher::read_qa_review_pr_target;

    const META: &str = r#"{"qa_review_pr_target":"senara-solutions/mika#2659"}"#;

    async fn decide(
        label: &str,
        metadata: Option<&str>,
        count: anyhow::Result<i64>,
    ) -> BuildCallbackHead {
        let seen_key = std::sync::Mutex::new(None);
        let out = decide_build_callback(label, metadata, read_qa_review_pr_target, |k| {
            *seen_key.lock().unwrap() = Some(k);
            async move { count }
        })
        .await;
        if let Some(k) = seen_key.lock().unwrap().as_deref() {
            assert_eq!(
                k, "pr:senara-solutions/mika#2659",
                "la clé lue est celle qu'écrit le registre"
            );
        }
        out
    }

    /// AC3 — un `synchronize` reçu après le lancement saute le tour.
    #[tokio::test]
    async fn mika2671_un_sync_posterieur_saute_le_tour() {
        let d = decide(BUILD_CALLBACK_LABEL, Some(META), Ok(1)).await;
        assert!(d.skips_turn(), "{d:?}");
        assert_eq!(
            d,
            BuildCallbackHead::Superseded {
                target: PrTarget {
                    repo: "senara-solutions/mika".into(),
                    pr_number: 2659
                },
                later_syncs: 1
            }
        );
    }

    /// AC2 contrôle négatif — tête courante : comportement inchangé.
    #[tokio::test]
    async fn mika2671_sans_sync_posterieur_le_tour_tourne() {
        let d = decide(BUILD_CALLBACK_LABEL, Some(META), Ok(0)).await;
        assert_eq!(d, BuildCallbackHead::Current);
        assert!(!d.skips_turn());
    }

    /// Fail-safe revue — un registre illisible ne saute jamais un tour.
    #[tokio::test]
    async fn mika2671_registre_illisible_le_tour_tourne() {
        let d = decide(
            BUILD_CALLBACK_LABEL,
            Some(META),
            Err(anyhow::anyhow!("db down")),
        )
        .await;
        assert!(
            matches!(d, BuildCallbackHead::LedgerUnreadable { .. }),
            "{d:?}"
        );
        assert!(!d.skips_turn());
    }

    /// Fail-safe revue — sans cible PR, rien à quoi comparer : le tour tourne,
    /// et le registre n'est même pas consulté.
    #[tokio::test]
    async fn mika2671_sans_cible_le_registre_nest_pas_lu() {
        for meta in [None, Some(""), Some("{}"), Some("pas du json")] {
            let d = decide_build_callback(
                BUILD_CALLBACK_LABEL,
                meta,
                read_qa_review_pr_target,
                |_| async { panic!("le registre ne doit pas être lu sans cible PR") },
            )
            .await;
            assert!(
                matches!(d, BuildCallbackHead::NoPrTarget(_)),
                "{meta:?} → {d:?}"
            );
            assert!(!d.skips_turn());
        }
    }

    /// Portée — un autre flux `long_running` n'est jamais sauté, même avec un
    /// `synchronize` postérieur au registre.
    #[tokio::test]
    async fn mika2671_un_autre_long_running_est_hors_population() {
        for label in [
            "long_running:deploy_mika",
            "long_running:run_claude_pilot",
            "long_running:build_mika_x",
        ] {
            let d = decide(label, Some(META), Ok(5)).await;
            assert_eq!(d, BuildCallbackHead::NotABuildCallback, "{label}");
        }
    }

    #[test]
    fn mika2671_la_cle_du_registre_est_un_format_de_fil() {
        assert_eq!(
            sync_observed_key("senara-solutions/mika", 2659),
            "pr:senara-solutions/mika#2659"
        );
        assert_eq!(SYNC_OBSERVED_TOOL, "qa_pr_sync_observed");
        assert_eq!(
            BUILD_CALLBACK_SUPERSEDED_EVENT,
            "qa_build_callback_superseded"
        );
    }

    #[test]
    fn mika2671_le_kill_switch_a_trois_paliers_et_une_coquille_laisse_arme() {
        assert!(parse_stale_build_guard(None));
        assert!(parse_stale_build_guard(Some("")));
        assert!(parse_stale_build_guard(Some(" 1 ")));
        assert!(
            parse_stale_build_guard(Some("plif")),
            "une coquille ne désarme pas"
        );
        for off in ["0", "false", "OFF", " no "] {
            assert!(!parse_stale_build_guard(Some(off)), "{off}");
        }
    }
}
