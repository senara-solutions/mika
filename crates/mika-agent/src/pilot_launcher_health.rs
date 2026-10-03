//! Le lanceur `claude-pilot` est-il mort sur cet hôte ? (mika#2634, phase B)
//!
//! **Lecteur unique** de cette question. Deux surfaces la posent — la porte de
//! `validate_dispatch_readiness` avant de spawner un pilote, et le filtre de la
//! Phase 2 d'`auto_pull` avant de re-driver un label — et un prédicat écrit deux
//! fois est un prédicat qui peut se contredire. `grooming_marker` a dû graver
//! cette leçon une fois (mika#2158, où la promotion et le routage de dispatch ont
//! répondu différemment à la même question pendant des mois sans que rien ne
//! casse), et `live_pilot` (mika#2279) en est le patron direct : un module, un
//! verdict, deux consommateurs.
//!
//! # Ce que la phase A a laissé ouvert
//!
//! La phase A (U1-U3, PR #2644) a rendu la mort du lanceur **visible** : exit 79
//! estampillé par le pré-flight, `pilot_launcher_dead` en WARN, une ligne
//! `audit_events` sous [`PILOT_LAUNCHER_HEALTH_TOOL`]. Elle n'a rien **borné** :
//! chaque dispatch suivant repartait et mourait de la même façon tant que l'hôte
//! n'était pas réparé. Le 2026-10-02, trois dispatches sont morts sur 2 h 45 et
//! la panne a été trouvée à la main. C'est l'AC2 du ticket, et c'est ce module.
//!
//! # Une FENÊTRE, jamais un compteur persistant (D5)
//!
//! « ≥ 2 occurrences de `dead` dans les N dernières minutes » plutôt qu'un
//! compteur de consécutifs. Trois raisons, dans l'ordre du poids :
//!
//! - **Le frein se lève de lui-même : il n'y a rien à effacer.** Un lanceur
//!   réparé sort de la fenêtre sans qu'aucun geste ne soit posé. C'est la
//!   propriété que mika#2597 écrit pour son hold et que mika#2347 a dû bâtir à la
//!   main faute de l'avoir.
//! - Un compteur persistant demande un site de remise à zéro, et mika#2158 a
//!   mesuré ce que coûte un compteur remis à zéro par l'action qu'il compte :
//!   31 re-drives affichant 1.
//! - Le pire cas est borné et auto-réparant : si l'hôte n'est pas réparé, **un**
//!   dispatch est brûlé par fenêtre au lieu de tous — ce que le ticket demande.
//!
//! # Fail-OPEN sur la lecture, et l'asymétrie l'exige (D6)
//!
//! Un ledger illisible ⇒ **on ne bloque pas**. C'est l'inverse de `wip_rescue`
//! (mika#2199) et de `run_gh pr ready` (mika#2624), et l'inversion est raisonnée :
//! là-bas un faux négatif faisait attendre **une** PR ; ici un faux positif gèle
//! **tous** les dispatches de la flotte. Un faux négatif coûte une fenêtre brûlée
//! — visible (une ligne `pilot_launcher_dead`), borné, rattrapable au tour
//! suivant.
//!
//! [`LauncherHealth::is_braked`] existe pour qu'un appelant ne puisse pas écrire
//! `!matches!(v, Healthy)` et transformer [`LauncherHealth::Unreadable`] en
//! blocage — la seule inversion qui gèlerait la boucle sur une supposition. Même
//! garde, même raison, que `LivePilotVerdict::is_alive`.

use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::skills::executor::{
    PILOT_LAUNCHER_DEAD_VALUE, PILOT_LAUNCHER_HEALTH_TOOL, PILOT_LAUNCHER_RECOVERED_VALUE,
};
use crate::timestamp;
use chrono::Duration;

// ---------------------------------------------------------------------------
// Configuration — trois paliers
// ---------------------------------------------------------------------------

const BRAKE_ENABLED_ENV: &str = "MIKA_PILOT_LAUNCHER_BRAKE";
const BRAKE_WINDOW_ENV: &str = "MIKA_PILOT_LAUNCHER_BRAKE_WINDOW_SECS";
const BRAKE_THRESHOLD_ENV: &str = "MIKA_PILOT_LAUNCHER_BRAKE_THRESHOLD";

/// Une heure, bornée des deux côtés par l'incident mesuré.
///
/// En dessous : les trois morts du 2026-10-02 sont tombées à 03:03Z, 03:05Z et
/// 04:00Z — une fenêtre de trente minutes aurait laissé la troisième repartir à
/// neuf. Au-dessus : la fenêtre cesse de se lever d'elle-même en un délai qu'un
/// opérateur accepte d'attendre après avoir réparé l'hôte, et D5 ne tient que
/// parce que la levée est gratuite.
pub const BRAKE_WINDOW_DEFAULT_SECS: i64 = 3600;

/// La lettre d'AC2 : *« à la **deuxième** occurrence consécutive »*.
///
/// Un seuil de 1 freinerait sur la première mort, c'est-à-dire sur toute panne
/// transitoire du lanceur, et un dispatch brûlé est le prix déjà payé pour
/// *savoir* qu'il y a une panne. Deux est ce que le ticket demande et c'est le
/// premier compte qui distingue un accident d'un hôte cassé.
pub const BRAKE_THRESHOLD_DEFAULT: i64 = 2;

/// Les bornes du frein.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrakeConfig {
    /// Kill-switch, **défaut armé**. `0` désarme sans redéploiement.
    ///
    /// Ce qu'il gate est la **disposition**, jamais la détection : les lignes
    /// `dead` et `recovered` continuent d'être écrites, parce qu'elles *sont* la
    /// mesure (patron mika#2249/#2272 — *la détection est inconditionnelle, seule
    /// la disposition est gardée*).
    pub enabled: bool,
    pub window_secs: i64,
    pub threshold: i64,
}

impl Default for BrakeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_secs: BRAKE_WINDOW_DEFAULT_SECS,
            threshold: BRAKE_THRESHOLD_DEFAULT,
        }
    }
}

/// Trois paliers : absent ou vide → défaut ; illisible, `0` ou négatif → défaut
/// **plus un WARN nommant la valeur entre guillemets**.
///
/// Le `0` **ne désarme pas** — c'est le rôle du kill-switch, et un désarmement
/// par coquille sur un frein de coût est la panne silencieuse que tout ceci
/// ferme. Un seuil à `0` freinerait d'ailleurs *avant la première mort*, donc
/// gèlerait la flotte sur un hôte sain ; une fenêtre à `0` ne couvrirait aucun
/// instant et rendrait le frein inerte. Les deux lectures du `0` sont fausses, et
/// c'est pourquoi aucune n'est honorée.
pub fn parse_positive_i64(raw: Option<&str>, default: i64, env_name: &str) -> i64 {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<i64>() {
            Ok(n) if n > 0 => n,
            _ => {
                warn!(
                    value = %format!("{:?}", v.trim()),
                    default,
                    env = env_name,
                    "pilot_launcher_brake: valeur illisible ou non positive, défaut appliqué"
                );
                default
            }
        },
        _ => default,
    }
}

/// Kill-switch : `0`/`false`/`off`/`no` désarment ; absent, vide ou **non
/// reconnu** laissent armé, avec un WARN nommant la valeur entre guillemets.
///
/// Un désarmement par coquille sur le frein que l'AC2 demande serait exactement
/// la panne silencieuse de l'incident fondateur : la flotte continuerait de
/// brûler ses fenêtres pendant que l'opérateur croit le frein en place
/// (mika#2205). Même table de vérité que `MIKA_TARGET_PURGE` (mika#2497) et que
/// `MIKA_TELEGRAM_HTML_RENDER` (mika#2291).
pub fn parse_brake_enabled(raw: Option<&str>) -> bool {
    match raw.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") => true,
        Some("0" | "false" | "off" | "no") => false,
        Some("1" | "true" | "on" | "yes") => true,
        Some(other) => {
            warn!(
                value = %format!("{other:?}"),
                env = BRAKE_ENABLED_ENV,
                "pilot_launcher_brake: valeur non reconnue — le frein reste armé"
            );
            true
        }
    }
}

/// La configuration en vigueur, lue depuis l'environnement du **process**.
///
/// Lu à chaque résolution plutôt que mis en cache : le coût est trois
/// `std::env::var`, et un cache demanderait un site d'invalidation que rien ici
/// ne justifie.
pub fn brake_config_from_env() -> BrakeConfig {
    BrakeConfig {
        enabled: parse_brake_enabled(std::env::var(BRAKE_ENABLED_ENV).ok().as_deref()),
        window_secs: parse_positive_i64(
            std::env::var(BRAKE_WINDOW_ENV).ok().as_deref(),
            BRAKE_WINDOW_DEFAULT_SECS,
            BRAKE_WINDOW_ENV,
        ),
        threshold: parse_positive_i64(
            std::env::var(BRAKE_THRESHOLD_ENV).ok().as_deref(),
            BRAKE_THRESHOLD_DEFAULT,
            BRAKE_THRESHOLD_ENV,
        ),
    }
}

// ---------------------------------------------------------------------------
// Le verdict
// ---------------------------------------------------------------------------

/// Ce que [`launcher_health`] a pu établir de l'hôte.
///
/// **Trois états, et jamais un `bool`.** Les deux derniers appellent des
/// conduites **opposées** — « le lanceur est cassé, réparez l'hôte » contre « la
/// base ne répond pas, le frein est inerte » — et un booléen ferait lire le
/// second comme le premier. Patron `HoldVerdict` (mika#2597) et
/// `LivePilotVerdict` (mika#2279).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherHealth {
    /// Rien dans la fenêtre, ou moins que le seuil. Comportement d'avant la
    /// phase B, octet pour octet.
    Healthy,
    /// Le seuil est atteint : l'hôte est cassé et chaque dispatch suivant
    /// mourrait de la même façon.
    Braked {
        dead_count: i64,
        /// Le début de la fenêtre interrogée, en ISO 8601 UTC. Porté pour que le
        /// refus puisse *dire* sur quoi il porte : « 2 morts » sans la fenêtre
        /// n'est pas une mesure, c'est un nombre.
        since: String,
    },
    /// La question n'a pas pu être posée. **À lire comme « ne bloquez pas »**,
    /// jamais comme « cassé » — voir le fail-open du module.
    Unreadable { reason: &'static str },
}

/// La lecture du ledger a échoué.
pub const UNREADABLE_DB_ERROR: &str = "db_error";

impl LauncherHealth {
    /// `true` **uniquement** pour [`LauncherHealth::Braked`].
    ///
    /// Existe pour qu'un appelant ne puisse pas écrire `!matches!(v, Healthy)` et
    /// transformer [`LauncherHealth::Unreadable`] en blocage — la seule inversion
    /// qui gèlerait toute la flotte sur une supposition.
    pub fn is_braked(&self) -> bool {
        matches!(self, Self::Braked { .. })
    }
}

/// La décision, **pure** : tout est déjà résolu (plan § U4 point 12).
///
/// `dead_count` vaut `None` quand le ledger n'a pas pu être lu — et `None` n'est
/// **jamais** `0` (mika#2331) : « je n'ai pas pu compter » et « j'ai compté
/// zéro » appellent des conduites différentes, la seconde autorisant le dispatch
/// sur une mesure et la première sur une ignorance qu'il faut nommer.
pub fn classify_launcher_health(
    dead_count: Option<i64>,
    threshold: i64,
    since: &str,
) -> LauncherHealth {
    match dead_count {
        None => LauncherHealth::Unreadable {
            reason: UNREADABLE_DB_ERROR,
        },
        Some(n) if n >= threshold => LauncherHealth::Braked {
            dead_count: n,
            since: since.to_string(),
        },
        Some(_) => LauncherHealth::Healthy,
    }
}

// ---------------------------------------------------------------------------
// Le résolveur
// ---------------------------------------------------------------------------

/// Combien de lanceurs sont morts dans la fenêtre, ou `None` si le ledger est
/// illisible.
///
/// Séparé de la classification pour que la décision reste pure et testable à ses
/// bornes sans base de données, et pour que le site § 16 (`recovered`) puisse
/// poser la même question sans passer par la disposition.
async fn dead_count_in_window(db: &AsyncDatabase, since: &str) -> Option<i64> {
    match db
        .count_recent_audit_events_by_value(
            PILOT_LAUNCHER_HEALTH_TOOL,
            PILOT_LAUNCHER_DEAD_VALUE,
            since,
        )
        .await
    {
        Ok(n) => Some(n),
        Err(e) => {
            warn!(
                event = "pilot_launcher_health_unreadable",
                error = %e,
                "mika#2634: le ledger de santé du lanceur est illisible — le frein est \
                 INERTE pour cette décision (fail-open), et le dispatch procède comme \
                 avant la phase B"
            );
            None
        }
    }
}

/// Le début de la fenêtre, en ISO 8601 UTC.
fn window_start(window_secs: i64) -> String {
    timestamp::now_minus(Duration::seconds(window_secs))
}

/// Le verdict sur l'hôte, pour une **décision de dispatch**.
///
/// Rend [`LauncherHealth::Healthy`] quand le kill-switch est désarmé — mais
/// **après avoir compté**, et en le disant : un frein désarmé qui aurait mordu
/// émet `pilot_launcher_brake_disarmed` (WARN, régime attendu **zéro**). Sans
/// cette ligne, « le frein est désarmé » et « l'hôte va bien » rendraient
/// exactement les mêmes octets, et un opérateur lirait une flotte saine sur un
/// hôte en train de brûler ses fenêtres — la classe mika#2205 appliquée au frein
/// lui-même. Le coût est un `COUNT(*)` indexé par dispatch, c'est-à-dire rien
/// devant le processus et les allers-retours GitHub que ce même dispatch paie.
pub async fn launcher_health(db: &AsyncDatabase) -> LauncherHealth {
    let cfg = brake_config_from_env();
    let since = window_start(cfg.window_secs);
    let verdict = classify_launcher_health(
        dead_count_in_window(db, &since).await,
        cfg.threshold,
        &since,
    );

    if !cfg.enabled {
        if let LauncherHealth::Braked { dead_count, since } = &verdict {
            warn!(
                event = "pilot_launcher_brake_disarmed",
                dead_count,
                threshold = cfg.threshold,
                window_secs = cfg.window_secs,
                since = %since,
                "mika#2634: le frein aurait mordu mais {BRAKE_ENABLED_ENV} le désarme — \
                 le dispatch part sur un hôte dont le lanceur est mort {dead_count} fois \
                 dans la fenêtre"
            );
        }
        return LauncherHealth::Healthy;
    }

    verdict
}

/// La fenêtre porte-t-elle au moins une mort de lanceur ? (plan § U4 point 16)
///
/// `None` = illisible. **Non gaté par le kill-switch**, et c'est la moitié
/// « détection » du patron mika#2249 : la ligne `recovered` est une *mesure* de
/// la réparation de l'hôte, et un opérateur qui a désarmé la disposition pour
/// observer doit continuer de la voir.
pub async fn window_carries_dead_launcher(db: &AsyncDatabase) -> Option<bool> {
    let cfg = brake_config_from_env();
    let since = window_start(cfg.window_secs);
    dead_count_in_window(db, &since).await.map(|n| n > 0)
}

/// Écrire la transition hors de l'état freiné, **et seulement une transition**
/// (plan § U4 point 16).
///
/// Appelé depuis le site de U3 quand le subprocess finit **sans** le code dédié.
/// Ne fait rien quand la fenêtre est propre : une ligne par dispatch sain serait
/// le churn que la doctrine mika#2131 borne, et le volume doit rester **nul en
/// régime sain** pour que `after_value = 'recovered'` se lise comme « une
/// réparation d'hôte » plutôt que comme du bruit.
///
/// Fire-and-forget, comme ses quatre voisines du même site : une mesure ne doit
/// jamais pouvoir casser la livraison qu'elle observe.
pub async fn record_launcher_recovery(db: &AsyncDatabase, task_id: &str) {
    match window_carries_dead_launcher(db).await {
        // Rien à dire : le régime sain est silencieux.
        Some(false) => return,
        // Illisible — on a déjà émis `pilot_launcher_health_unreadable` dans le
        // lecteur. Écrire `recovered` ici affirmerait une transition qu'on n'a
        // pas observée, ce qui est l'inverse de ce que ce ticket défend.
        None => return,
        Some(true) => {}
    }

    let cfg = brake_config_from_env();
    if let Err(e) = db
        .log_audit_event(
            &format!("callback-{task_id}"),
            PILOT_LAUNCHER_HEALTH_TOOL,
            &format!("task:{task_id}"),
            None,
            Some(PILOT_LAUNCHER_RECOVERED_VALUE),
            Some(&format!("window_secs:{}", cfg.window_secs)),
            None,
        )
        .await
    {
        warn!(
            event = "pilot_launcher_recovered_audit_failed",
            task_id = %task_id,
            error = %e,
            "mika#2634: un dispatch a tourné sous un lanceur réparé mais la ligne \
             `recovered` n'est pas partie — le `GROUP BY after_value` sur-compte les \
             morts à partir d'ici"
        );
        return;
    }

    info!(
        event = "pilot_launcher_recovered",
        task_id = %task_id,
        window_secs = cfg.window_secs,
        "mika#2634: le lanceur remarche — ce dispatch a fini sans le code de mort \
         alors que la fenêtre portait au moins une mort"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINCE: &str = "2026-10-03T00:00:00Z";

    // -----------------------------------------------------------------------
    // U6 §22 — `classify_launcher_health` aux trois bornes
    // -----------------------------------------------------------------------

    /// **La borne basse : une seule mort n'est pas un hôte cassé.**
    ///
    /// C'est la lettre d'AC2 (« à la **deuxième** occurrence ») et c'est le
    /// contrôle qui empêche le frein de mordre sur une panne transitoire du
    /// lanceur — population pour laquelle un dispatch brûlé est le prix déjà payé
    /// pour *savoir* qu'il y a une panne.
    #[test]
    fn mika2634_one_dead_launcher_is_not_a_brake() {
        assert_eq!(
            classify_launcher_health(Some(1), BRAKE_THRESHOLD_DEFAULT, SINCE),
            LauncherHealth::Healthy
        );
    }

    /// Zéro mort : le régime nominal, et il doit rester gratuit et silencieux.
    #[test]
    fn mika2634_a_clean_window_is_healthy() {
        assert_eq!(
            classify_launcher_health(Some(0), BRAKE_THRESHOLD_DEFAULT, SINCE),
            LauncherHealth::Healthy
        );
    }

    /// **La borne haute : la deuxième mort freine**, et le verdict porte de quoi
    /// composer un refus qui se lit — le compte ET la fenêtre.
    #[test]
    fn mika2634_two_dead_launchers_engage_the_brake() {
        let verdict = classify_launcher_health(Some(2), BRAKE_THRESHOLD_DEFAULT, SINCE);
        assert_eq!(
            verdict,
            LauncherHealth::Braked {
                dead_count: 2,
                since: SINCE.to_string(),
            }
        );
        assert!(verdict.is_braked());
    }

    /// Le seuil est un paramètre, pas une constante cachée dans le prédicat : un
    /// opérateur qui le relève obtient la décision qu'il a demandée.
    #[test]
    fn mika2634_the_threshold_is_what_decides() {
        assert_eq!(
            classify_launcher_health(Some(2), 3, SINCE),
            LauncherHealth::Healthy
        );
        assert!(classify_launcher_health(Some(3), 3, SINCE).is_braked());
    }

    /// **La troisième borne, et c'est celle qui porte le fail-open.**
    ///
    /// Un ledger illisible n'est pas un hôte cassé : `Unreadable` est un verdict
    /// **distinct** et `is_braked()` est faux pour lui, donc l'appelant procède
    /// exactement comme avant la phase B. Un `bool` aurait fait lire cette panne
    /// de base comme une panne de lanceur et gelé toute la flotte.
    #[test]
    fn mika2634_an_unreadable_ledger_is_not_a_brake() {
        let verdict = classify_launcher_health(None, BRAKE_THRESHOLD_DEFAULT, SINCE);
        assert_eq!(
            verdict,
            LauncherHealth::Unreadable {
                reason: UNREADABLE_DB_ERROR,
            }
        );
        assert!(
            !verdict.is_braked(),
            "INVARIANT VIOLÉ : un signal illisible est devenu un frein — c'est \
             l'inversion qui gèle la flotte sur une supposition (D6)"
        );
    }

    /// `None` n'est jamais `0` : les deux rendent des verdicts différents, ce qui
    /// est la seule façon de garder « je n'ai pas pu compter » distinguable de
    /// « j'ai compté zéro » (mika#2331).
    #[test]
    fn mika2634_an_unreadable_count_is_not_a_zero_count() {
        assert_ne!(
            classify_launcher_health(None, BRAKE_THRESHOLD_DEFAULT, SINCE),
            classify_launcher_health(Some(0), BRAKE_THRESHOLD_DEFAULT, SINCE)
        );
    }

    // -----------------------------------------------------------------------
    // U6 §22 — les trois paliers maison
    // -----------------------------------------------------------------------

    #[test]
    fn mika2634_absent_or_empty_takes_the_default() {
        assert_eq!(parse_positive_i64(None, 3600, "X"), 3600);
        assert_eq!(parse_positive_i64(Some(""), 3600, "X"), 3600);
        assert_eq!(parse_positive_i64(Some("   "), 3600, "X"), 3600);
    }

    #[test]
    fn mika2634_a_positive_integer_is_honoured() {
        assert_eq!(parse_positive_i64(Some("900"), 3600, "X"), 900);
        assert_eq!(parse_positive_i64(Some("  7 "), 3600, "X"), 7);
    }

    /// **`0` ne désarme pas, et ni une fenêtre ni un seuil ne l'accepte.**
    ///
    /// Les deux lectures du `0` sont fausses dans des directions opposées — un
    /// seuil à zéro freine avant la première mort, une fenêtre à zéro ne couvre
    /// aucun instant — donc aucune n'est honorée. Le désarmement est le
    /// kill-switch, pas une coquille.
    #[test]
    fn mika2634_zero_and_negative_take_the_default_and_do_not_disarm() {
        for raw in ["0", "-1", "-3600", "plif", "3.5", "1e3"] {
            assert_eq!(
                parse_positive_i64(Some(raw), 3600, "X"),
                3600,
                "`{raw}` doit retomber sur le défaut, jamais désarmer"
            );
        }
    }

    /// Le kill-switch, et sa pente : **une valeur non reconnue laisse armé**.
    #[test]
    fn mika2634_the_kill_switch_has_three_tiers_and_fails_armed() {
        for raw in [None, Some(""), Some("  ")] {
            assert!(parse_brake_enabled(raw), "absent ou vide ⇒ armé");
        }
        for raw in ["0", "false", "off", "no", "FALSE", " Off "] {
            assert!(!parse_brake_enabled(Some(raw)), "`{raw}` doit désarmer");
        }
        for raw in ["1", "true", "on", "yes", "TRUE"] {
            assert!(parse_brake_enabled(Some(raw)), "`{raw}` doit armer");
        }
        for raw in ["plif", "2", "maybe", "oui"] {
            assert!(
                parse_brake_enabled(Some(raw)),
                "INVARIANT VIOLÉ : `{raw}` a désarmé le frein par coquille — c'est la \
                 panne silencieuse que ce ticket ferme (mika#2205)"
            );
        }
    }

    /// La configuration livrée est celle que le plan nomme (§ U4 point 13).
    #[test]
    fn mika2634_the_shipped_defaults_are_the_plan_values() {
        let cfg = BrakeConfig::default();
        assert!(cfg.enabled, "le frein est livré ARMÉ (Fire-Disposition)");
        assert_eq!(cfg.window_secs, 3600);
        assert_eq!(cfg.threshold, 2, "la lettre d'AC2");
    }

    /// La fenêtre est un instant ISO 8601 UTC à largeur fixe — c'est ce qui rend
    /// la comparaison de chaînes correcte dans le `WHERE created_at > ?`.
    #[test]
    fn mika2634_the_window_start_is_a_fixed_width_utc_instant() {
        let s = window_start(3600);
        assert_eq!(s.len(), 20, "`%Y-%m-%dT%H:%M:%SZ` fait 20 octets : {s}");
        assert!(s.ends_with('Z'), "{s}");
    }
}
