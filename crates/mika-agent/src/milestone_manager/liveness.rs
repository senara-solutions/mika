//! Battement de vivacité vers le registre cm — un POST par tick réussi (mika#1990).
//!
//! **LECTURE seule.** Ce module n'ajoute aucune autorité d'écriture : son seul
//! effet de bord sortant est un `POST` minimal sur un endpoint de liveness
//! déclaré, de la même famille que le POST de rapport de `cadence.rs`.
//!
//! ## Le défaut que ça ferme
//!
//! `mika-manager` poll toutes les 5 min et ne POSTe **rien** tant que le cycle
//! ne délivre pas ; la delivery est hybride (`state_changed || heartbeat_fired`,
//! le second sur un plancher de 6 h). Entre deux battements légitimes le
//! registre cm ne reçoit aucun signe de vie, la freshness passe RED, et le
//! nudge-scanner crie au loup alors que la cadence tourne parfaitement.
//!
//! ## Pourquoi un canal DISTINCT du POST de rapport
//!
//! `DeliveryBody` est un **format de fil que cm consomme** et ne porte aucun
//! champ `reason`. Y ajouter un champ serait une rupture gratuite d'un canal qui
//! fonctionne, pour un besoin — la *freshness* — qui ne se lit pas dans un
//! rapport de 30 Ko posté quatre fois par jour. D'où : endpoint distinct, corps
//! minimal, un POST par tick, et le `reason` qui dit *pourquoi ce battement a eu
//! lieu*.
//!
//! ## L'URL est DÉCLARÉE, jamais dérivée (mika#1990 R2)
//!
//! L'endpoint vit dans `control-monitor`, **hors de ce workspace** : aucune ligne
//! écrite ici ne peut le faire exister ni prouver qu'il répond. Le livrable est
//! donc l'émetteur seul, et son URL vient d'une variable à elle —
//! [`ENV_LIVENESS_URL`] — jamais d'une composition depuis `delivery_url`
//! (exemple posé : `…/api/v1/messages/dispatch`, aucun rapport de forme avec
//! `…/api/v1/agents/<id>/heartbeat`) ni depuis `health_url`, dont l'entité n'est
//! pas la nôtre et dont la sémantique est **inverse** (on y *lit* la santé de
//! l'exécuteur ; ici on *écrit* la nôtre). Doctrine maison appliquée, pas
//! inventée : mika#2249 (« the worktree path is declared, never derived ») et
//! mika#2368 (« la cible PR est dite, jamais dérivée »).
//!
//! ## Où vit la constante, et pourquoi pas dans `spawn.rs`
//!
//! `ENV_LIVENESS_URL` est déclarée **ici**, avec le canal qui la nomme, et
//! ré-exportée par `spawn.rs` — précédent exact et même raison que
//! `ENV_OFFLINE_SINK_DIR` (mika#2267 C1) : la garde structurelle qui tient
//! l'écrivain unique a besoin d'un fichier propriétaire unique. Le test T7
//! `mika2267_every_manager_env_const_is_declared_in_env_example` scanne tout le
//! répertoire, donc la déclaration reste couverte où qu'elle vive.

use super::cadence::{ManagerConfig, url_is_routable};
use super::types::{CycleOutcome, MilestoneRef, Severity};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{info, warn};

/// Variable d'environnement portant l'URL du battement.
///
/// **Le nom est `LIVENESS` et non `HEARTBEAT`, et c'est une décision (mika#1990
/// R4/D5).** `MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS` existe déjà et désigne
/// **autre chose** — le plancher de delivery 6 h. Nommer celle-ci
/// `MIKA_MANAGER_HEARTBEAT_URL` poserait côte à côte, dans le même
/// `EnvironmentFile` :
///
/// ```text
/// MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS=21600
/// MIKA_MANAGER_HEARTBEAT_URL=https://cm.example.com/api/v1/agents/mika-manager/heartbeat
/// ```
///
/// dont tout opérateur conclurait « le heartbeat vers cette URL bat toutes les
/// 6 h » — très exactement la croyance fausse que ce ticket existe pour tuer. Le
/// nom retenu sépare deux concepts que le vocabulaire confond, et la confusion
/// de ce vocabulaire *est* le défaut. L'endpoint nommé par le ticket
/// (`/api/v1/agents/mika-manager/heartbeat`) reste intact — c'est la *variable*
/// qui change de nom, pas la route.
pub const ENV_LIVENESS_URL: &str = "MIKA_MANAGER_LIVENESS_URL";

/// L'entité dont ce battement atteste la vivacité.
pub const LIVENESS_ENTITY: &str = "mika-manager";

/// Préfixe du motif d'un tick qui n'a rien délivré.
///
/// **Format de fil (mika#1990 D8).** Les valeurs atterrissent dans le registre
/// cm et un opérateur en fera des `GROUP BY` : deux orthographes d'un même motif
/// couperaient une population en deux sans le dire. Figé par
/// `mika1990_le_vocabulaire_du_reason_est_un_format_de_fil`.
pub const REASON_POLL_PREFIX: &str = "poll:";

/// Préfixe du motif d'un tick qui a délivré un rapport. Voir
/// [`REASON_POLL_PREFIX`] pour le contrat de format de fil.
pub const REASON_DELIVERY_PREFIX: &str = "delivery:";

/// Borne d'un battement, appliquée **deux fois** : par le client `reqwest` de
/// [`HttpLivenessSink`] et par le `tokio::time::timeout` de
/// [`LivenessEmitter::beat`].
///
/// **Pourquoi la borne vit dans `beat` et pas seulement dans le rail.** Même
/// raison que `verify_gh_auth` (mika#1975 D1) et que le filet de mika#2342 : le
/// budget devient une propriété de la *fonction*, porte son propre test, et un
/// futur second sink l'hérite au lieu d'avoir à s'en souvenir. Un sink qui ne
/// rendrait jamais la main retarderait sinon le tick suivant sans borne.
///
/// **Une seule valeur, et la course entre les deux est bénigne.** Les deux
/// bornes valent 5 s, donc le rail coupe généralement le premier (son erreur
/// est alors *classée*) et le filet ne mord que sur un rail défaillant. Quand le
/// filet gagne la course, la classe rendue est `Unreachable` au lieu de la
/// classe qu'aurait posée le rail : les deux mènent au même WARN de transition
/// et à la même conduite, donc l'écart ne change aucune décision. Un second
/// nombre différent serait une valeur de plus à relire (mika#2189).
///
/// **5 s, alignés sur `probe_executor_health`** — même nature, une sonde de
/// liveness bornée court — soit au pire **1,7 %** d'un `poll_interval` de 300 s.
pub const LIVENESS_TIMEOUT: Duration = Duration::from_secs(5);

/// Corps du battement. Minimal à dessein : la freshness est un **instant**, pas
/// un rapport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivenessBody {
    /// Toujours [`LIVENESS_ENTITY`].
    pub entity: String,
    /// `"poll:<n>"` ou `"delivery:<severity>"` — voir [`liveness_reason`].
    pub reason: String,
    /// ISO 8601 UTC.
    pub generated_at: String,
    pub milestone_ref: MilestoneRef,
}

/// Classe d'échec d'un battement.
///
/// **Dérivée du STATUT, jamais d'une sous-chaîne du message** (règle mika#2179,
/// et même forme que `DeliveryFailureKind` un fichier plus loin). Les trois
/// valeurs nomment trois remèdes opérateur distincts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessFailureClass {
    /// 401 ou 403 — le registre a répondu et refusé le credential. Le remède est
    /// le `delivery_token` (D6), pas ce code.
    CredentialRefused,
    /// Le registre n'a pas répondu : connexion refusée, DNS, TLS, ou la borne
    /// [`LIVENESS_TIMEOUT`] a mordu.
    Unreachable,
    /// Tout le reste — un 404 (l'endpoint cm n'existe pas encore), un 500, un
    /// corps illisible. Délibérément **pas** un signal d'authentification.
    Other,
}

impl LivenessFailureClass {
    /// **Format de fil** — atterrit dans `class=` sur `manager_liveness_failed`
    /// et un opérateur `grep` ces valeurs. Figé par test.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CredentialRefused => "credential_refused",
            Self::Unreachable => "unreachable",
            Self::Other => "other",
        }
    }
}

/// Un battement qui n'est pas arrivé, et qui se souvient de laquelle des trois
/// classes il était.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivenessFailure {
    pub class: LivenessFailureClass,
    pub message: String,
}

/// Frontière de transport du battement — HTTP en production, en mémoire en test.
///
/// Motif exact de `ReportDeliverer` / `HttpReportDeliverer` (`cadence.rs`), dont
/// les doubles de test sont déjà écrits sur cette forme.
///
/// La méthode s'appelle `deliver_beat` et non `beat` pour rester distinguable de
/// [`LivenessEmitter::beat`] : la garde
/// `mika1990_le_battement_a_un_seul_ecrivain` compte les sites d'appel au sink,
/// et un nom partagé avec l'émetteur rendrait son prédicat inapplicable.
#[async_trait::async_trait]
pub trait LivenessSink: Send + Sync {
    /// Poster un battement. `token` est l'auth bearer optionnelle.
    async fn deliver_beat(
        &self,
        url: &str,
        token: Option<&str>,
        body: &LivenessBody,
    ) -> Result<(), LivenessFailure>;
}

/// Sink de production — POST JSON bearer-authentifié, même discipline
/// wrapper-only (`reqwest` du workspace, aucune dépendance neuve) que
/// `HttpReportDeliverer`.
pub struct HttpLivenessSink {
    client: reqwest::Client,
}

impl HttpLivenessSink {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(LIVENESS_TIMEOUT)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

impl Default for HttpLivenessSink {
    fn default() -> Self {
        Self::new()
    }
}

/// Combien de caractères du corps d'erreur atteignent le journal.
///
/// Un 404 dit « l'endpoint cm n'existe pas encore » et un 500 dit autre chose :
/// l'extrait est ce qui les sépare. Borné comme `stderr_head` l'est un fichier
/// plus loin, parce qu'un corps d'erreur n'a pas de taille garantie.
const LIVENESS_ERROR_BODY_MAX_CHARS: usize = 200;

#[async_trait::async_trait]
impl LivenessSink for HttpLivenessSink {
    async fn deliver_beat(
        &self,
        url: &str,
        token: Option<&str>,
        body: &LivenessBody,
    ) -> Result<(), LivenessFailure> {
        // `client.post(url)` et non une construction par `Method::POST` ni par
        // chaîne : `no_dispatch_test.rs` interdit le littéral `"POST"` dans tout
        // fichier du module (contrat LECTURE-SEULE), et les deux autres formes
        // le porteraient.
        let mut req = self.client.post(url).json(body);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let res = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let class = if e.is_connect() || e.is_timeout() {
                    LivenessFailureClass::Unreachable
                } else {
                    LivenessFailureClass::Other
                };
                return Err(LivenessFailure {
                    class,
                    message: format!("battement non parti: {e}"),
                });
            }
        };
        if !res.status().is_success() {
            let status = res.status();
            let class = match status.as_u16() {
                401 | 403 => LivenessFailureClass::CredentialRefused,
                _ => LivenessFailureClass::Other,
            };
            let text: String = res
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(LIVENESS_ERROR_BODY_MAX_CHARS)
                .collect();
            return Err(LivenessFailure {
                class,
                message: format!("battement refusé: {status} — {text}"),
            });
        }
        Ok(())
    }
}

/// Le nom de fil d'une sévérité, tel que `serde` le rend.
///
/// `match` exhaustif **sans bras `_`** (motif maison : `AuthClass::as_str`) plus
/// `mika1990_la_severite_du_motif_suit_serde`, qui compare ces trois valeurs au
/// rendu réel de `serde`. Le `match` seul pourrait diverger de l'attribut
/// `rename_all` ; le test rend la divergence impossible sans rougir.
fn severity_wire_name(severity: &Severity) -> &'static str {
    match severity {
        Severity::Healthy => "healthy",
        Severity::Attention => "attention",
        Severity::Blocked => "blocked",
    }
}

/// Le **site unique** de composition du motif (mika#1990 D8).
///
/// `"poll:<n>"` quand le cycle n'a rien délivré, `"delivery:<severity>"` quand
/// il a délivré. Un seul battement par tick, jamais deux : la freshness est un
/// instant, pas un compte, et deux battements simultanés ne diraient rien de
/// plus qu'un seul tout en dédoublant la population que cm voit.
pub fn liveness_reason(tick: u64, delivered: bool, severity: &Severity) -> String {
    if delivered {
        format!("{REASON_DELIVERY_PREFIX}{}", severity_wire_name(severity))
    } else {
        format!("{REASON_POLL_PREFIX}{tick}")
    }
}

/// Ce qu'un battement observé oblige à dire — ou rien.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionLine {
    /// Le canal vient de tomber : un WARN, **une seule fois** par transition.
    Failed {
        consecutive_failures: u32,
        class: LivenessFailureClass,
        message: String,
    },
    /// Le canal vient de se rétablir : un INFO, une seule fois.
    Recovered {
        /// Combien d'échecs consécutifs ce rétablissement referme.
        failures_before: u32,
    },
}

/// L'émetteur : le compteur de ticks et l'état de transition du canal.
///
/// **L'état de transition est en mémoire, perdu au redémarrage à dessein** — un
/// process neuf re-photographie ce qu'il trouve (motif `auto_pull_stop`,
/// mika#2329). Il n'y a rien à ressusciter, donc rien qui puisse refuser de
/// ressusciter.
#[derive(Debug, Default)]
pub struct LivenessEmitter {
    tick: u64,
    failing: bool,
    consecutive_failures: u32,
}

impl LivenessEmitter {
    /// Avancer le compteur d'**itérations de boucle**, en tête d'itération et
    /// avant le cycle (mika#1990 D3).
    ///
    /// Le compteur est incrémenté par le **site d'appel**, pas par [`Self::beat`] :
    /// sinon un cycle en erreur, qui ne bat pas, n'incrémenterait pas et les
    /// trous disparaîtraient. Conséquence voulue : une suite `poll:5` → `poll:9`
    /// dit « quatre cycles ont échoué », information qu'un compteur de
    /// *battements émis* (`poll:5` → `poll:6`) effacerait.
    ///
    /// Repart à `1` au démarrage du process, donc `poll:1` est le marqueur d'un
    /// redémarrage — utile, pas un défaut.
    pub fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.saturating_add(1);
        self.tick
    }

    /// Le tick courant. `0` avant le premier [`Self::next_tick`].
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Enregistrer l'issue d'un battement et dire s'il faut écrire une ligne.
    ///
    /// Fonction pure d'état, testable sans réseau : un WARN sur la **première**
    /// panne, rien sur les suivantes, un INFO au rétablissement (mika#1990 D7).
    pub fn observe(&mut self, outcome: Result<(), LivenessFailure>) -> Option<TransitionLine> {
        match outcome {
            Ok(()) => {
                let failures_before = self.consecutive_failures;
                self.consecutive_failures = 0;
                if self.failing {
                    self.failing = false;
                    Some(TransitionLine::Recovered { failures_before })
                } else {
                    None
                }
            }
            Err(failure) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                if self.failing {
                    None
                } else {
                    self.failing = true;
                    Some(TransitionLine::Failed {
                        consecutive_failures: self.consecutive_failures,
                        class: failure.class,
                        message: failure.message,
                    })
                }
            }
        }
    }

    /// Composer le motif, poster le battement, journaliser la transition.
    ///
    /// **Best-effort et jamais bloquant pour le cycle (AC1).** Aucune erreur
    /// n'est propagée : la signature ne rend rien, donc un appelant n'a aucun
    /// moyen de laisser un battement raté casser sa boucle.
    ///
    /// **URL absente ⇒ rien n'est posté et rien n'échoue (D4)** : zéro POST,
    /// zéro erreur, zéro ligne par tick. C'est aussi le rollback — retirer la
    /// variable désarme le canal sans redéploiement.
    ///
    /// Appelé **après** le cycle, donc il ne retarde pas ce cycle-ci ; il peut
    /// retarder le suivant, borné par [`LIVENESS_TIMEOUT`], et
    /// `tokio::time::interval` rattrape un tick retardé
    /// (`MissedTickBehavior::Burst` par défaut).
    ///
    /// **Aucun battement n'est rattrapé, et c'est le bon arbitrage** : pas de
    /// file, pas de réessai. Réessayer un « je suis vivant » périmé est au mieux
    /// inutile, au pire un mensonge daté ; le suivant arrive dans 5 min.
    pub async fn beat(
        &mut self,
        cfg: &ManagerConfig,
        sink: &dyn LivenessSink,
        outcome: &CycleOutcome,
    ) {
        let url = cfg.liveness_url.as_deref();
        if !url_is_routable(url) {
            return;
        }
        // `url_is_routable` vient de rendre `true`, donc `Some`.
        let Some(url) = url else { return };

        let body = LivenessBody {
            entity: LIVENESS_ENTITY.to_string(),
            reason: liveness_reason(self.tick, outcome.delivered, &outcome.severity),
            generated_at: crate::timestamp::now(),
            milestone_ref: cfg.target.clone(),
        };

        // Le token est celui de la delivery : même destinataire (cm), même
        // credential (D6). Les échecs du battement n'alimentent PAS le ledger
        // auth-boundary `manager_to_delivery` — cette population compte les
        // échecs de *livraison de rapport*, de l'ordre de 4 tentatives/jour, et
        // y verser 288 tentatives/jour changerait complètement ce que la requête
        // opérateur mesure. L'information de classe n'est pas perdue : elle est
        // portée par le WARN de transition ci-dessous.
        let token = cfg.delivery_token.as_deref();

        let outcome = match tokio::time::timeout(
            LIVENESS_TIMEOUT,
            sink.deliver_beat(url, token, &body),
        )
        .await
        {
            Ok(result) => result,
            Err(_elapsed) => Err(LivenessFailure {
                class: LivenessFailureClass::Unreachable,
                message: format!(
                    "le battement n'a pas rendu la main en {}s",
                    LIVENESS_TIMEOUT.as_secs()
                ),
            }),
        };

        // Un battement réussi n'écrit RIEN (AC4). À 288 battements/jour et par
        // milestone, une ligne par battement serait exactement le churn que la
        // doctrine mika#2131 borne.
        match self.observe(outcome) {
            Some(TransitionLine::Failed {
                consecutive_failures,
                class,
                message,
            }) => {
                warn!(
                    target: "mika::milestone_manager",
                    event = "manager_liveness_failed",
                    milestone = %cfg.target.as_display(),
                    class = class.as_str(),
                    consecutive_failures,
                    reason = %body.reason,
                    error = %message,
                    "battement de liveness perdu — la freshness cm va passer RED"
                );
            }
            Some(TransitionLine::Recovered { failures_before }) => {
                info!(
                    target: "mika::milestone_manager",
                    event = "manager_liveness_recovered",
                    milestone = %cfg.target.as_display(),
                    failures_before,
                    reason = %body.reason,
                    "battement de liveness rétabli"
                );
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::sink_dir::SinkDirSource;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    // ---- doubles et fixtures ----------------------------------------------

    fn milestone() -> MilestoneRef {
        MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 35,
        }
    }

    fn cfg_with_url(url: Option<&str>) -> ManagerConfig {
        ManagerConfig {
            target: milestone(),
            github_token: None,
            heartbeat_interval: chrono::Duration::seconds(21_600),
            poll_interval: chrono::Duration::seconds(300),
            silence_threshold_days: 3,
            delivery_url: None,
            delivery_token: Some("tok".into()),
            escalation_url: None,
            health_url: None,
            checkpoint_dir: PathBuf::from("/tmp/mika-1990-checkpoints"),
            offline_sink_dir: PathBuf::from("/tmp/mika-1990-sink"),
            sink_dir_source: SinkDirSource::Default,
            liveness_url: url.map(|u| u.to_string()),
        }
    }

    fn outcome(delivered: bool, severity: Severity) -> CycleOutcome {
        CycleOutcome {
            milestone_ref: milestone(),
            delivered,
            escalated: false,
            state_changed: delivered,
            heartbeat_fired: false,
            severity,
            generated_at: crate::timestamp::now(),
            auth_boundary: None,
            auth_attempted: false,
        }
    }

    /// Sink en mémoire : enregistre chaque battement, rend ce qu'on lui dit.
    #[derive(Default)]
    struct RecordingSink {
        beats: Arc<Mutex<Vec<LivenessBody>>>,
        fail_with: Option<LivenessFailure>,
        /// Combien de temps le sink « pend » avant de répondre.
        hang_for: Option<Duration>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self::default()
        }

        fn failing(class: LivenessFailureClass) -> Self {
            Self {
                fail_with: Some(LivenessFailure {
                    class,
                    message: "refusé par le double".into(),
                }),
                ..Self::default()
            }
        }

        fn hanging(for_: Duration) -> Self {
            Self {
                hang_for: Some(for_),
                ..Self::default()
            }
        }

        fn reasons(&self) -> Vec<String> {
            self.beats
                .lock()
                .expect("mutex du double")
                .iter()
                .map(|b| b.reason.clone())
                .collect()
        }

        fn count(&self) -> usize {
            self.beats.lock().expect("mutex du double").len()
        }
    }

    #[async_trait::async_trait]
    impl LivenessSink for RecordingSink {
        async fn deliver_beat(
            &self,
            _url: &str,
            _token: Option<&str>,
            body: &LivenessBody,
        ) -> Result<(), LivenessFailure> {
            if let Some(d) = self.hang_for {
                tokio::time::sleep(d).await;
            }
            self.beats
                .lock()
                .expect("mutex du double")
                .push(body.clone());
            match &self.fail_with {
                Some(f) => Err(f.clone()),
                None => Ok(()),
            }
        }
    }

    // ---- V1 — AC3 : N ticks poll + 1 delivery ------------------------------

    /// **V1 / AC3** — une séquence de N+1 ticks dont seul le dernier délivre
    /// produit exactement `poll:1` … `poll:N` puis un unique
    /// `delivery:<severity>`.
    #[tokio::test]
    async fn mika1990_un_cycle_simule_emet_n_ticks_poll_puis_une_delivery() {
        let cfg = cfg_with_url(Some(
            "https://cm.example.test/api/v1/agents/mika-manager/heartbeat",
        ));
        let sink = RecordingSink::new();
        let mut emitter = LivenessEmitter::default();

        const N: u64 = 4;
        for _ in 0..N {
            emitter.next_tick();
            emitter
                .beat(&cfg, &sink, &outcome(false, Severity::Healthy))
                .await;
        }
        emitter.next_tick();
        emitter
            .beat(&cfg, &sink, &outcome(true, Severity::Attention))
            .await;

        assert_eq!(
            sink.reasons(),
            vec![
                "poll:1".to_string(),
                "poll:2".to_string(),
                "poll:3".to_string(),
                "poll:4".to_string(),
                "delivery:attention".to_string(),
            ],
            "AC3 : N battements poll suivis d'un seul battement de delivery"
        );
    }

    /// Le corps du battement porte l'entité et le milestone, et rien de plus —
    /// la freshness est un instant.
    #[tokio::test]
    async fn mika1990_le_corps_du_battement_est_minimal_et_nomme_lentite() {
        let cfg = cfg_with_url(Some("https://cm.example.test/beat"));
        let sink = RecordingSink::new();
        let mut emitter = LivenessEmitter::default();
        emitter.next_tick();
        emitter
            .beat(&cfg, &sink, &outcome(false, Severity::Healthy))
            .await;

        let beats = sink.beats.lock().expect("mutex du double");
        let body = beats.first().expect("un battement");
        assert_eq!(body.entity, LIVENESS_ENTITY);
        assert_eq!(body.milestone_ref, milestone());
        assert!(
            !body.generated_at.is_empty(),
            "l'instant du battement est ce que la freshness lit"
        );
    }

    // ---- V2 — contrôle négatif : URL absente ------------------------------

    /// **V2 / AC5** — URL absente ⇒ **zéro** appel au sink, **zéro** erreur.
    /// C'est aussi le rollback de D4 : retirer la variable désarme le canal.
    #[tokio::test]
    async fn mika1990_sans_url_declaree_aucun_battement_nest_poste() {
        let cfg = cfg_with_url(None);
        let sink = RecordingSink::new();
        let mut emitter = LivenessEmitter::default();

        for _ in 0..3 {
            emitter.next_tick();
            emitter
                .beat(&cfg, &sink, &outcome(false, Severity::Healthy))
                .await;
        }
        emitter.next_tick();
        emitter
            .beat(&cfg, &sink, &outcome(true, Severity::Blocked))
            .await;

        assert_eq!(
            sink.count(),
            0,
            "canal désarmé : aucun POST, quel que soit le tick"
        );
    }

    /// Une URL posée mais **vide** est aussi un canal désarmé — `read_string_env`
    /// écarte déjà les vides venant de l'env, mais un `ManagerConfig` construit
    /// à la main peut en porter une.
    #[tokio::test]
    async fn mika1990_une_url_vide_est_un_canal_desarme() {
        let cfg = cfg_with_url(Some(""));
        let sink = RecordingSink::new();
        let mut emitter = LivenessEmitter::default();
        emitter.next_tick();
        emitter
            .beat(&cfg, &sink, &outcome(false, Severity::Healthy))
            .await;
        assert_eq!(sink.count(), 0);
    }

    // ---- V3 — AC1 : jamais bloquant --------------------------------------

    /// **V3 / AC1** — un sink qui rend `Err` n'empêche pas les battements
    /// suivants et ne propage rien : la signature de `beat` ne rend rien.
    #[tokio::test]
    async fn mika1990_un_sink_en_echec_ne_casse_pas_la_boucle() {
        let cfg = cfg_with_url(Some("https://cm.example.test/beat"));
        let sink = RecordingSink::failing(LivenessFailureClass::Other);
        let mut emitter = LivenessEmitter::default();

        for _ in 0..3 {
            emitter.next_tick();
            emitter
                .beat(&cfg, &sink, &outcome(false, Severity::Healthy))
                .await;
        }

        assert_eq!(
            sink.count(),
            3,
            "les trois battements ont été tentés — l'échec du premier n'a rien arrêté"
        );
    }

    /// **V3 / AC1 (second volet)** — un sink qui pend **au-delà** de la borne
    /// rend la main quand même, et l'échec est classé `unreachable`.
    ///
    /// `tokio::time::pause` : l'horloge est virtuelle, donc le test ne dort pas
    /// 6 s réelles.
    #[tokio::test(start_paused = true)]
    async fn mika1990_un_sink_qui_pend_est_borne_par_le_timeout() {
        let cfg = cfg_with_url(Some("https://cm.example.test/beat"));
        let sink = RecordingSink::hanging(LIVENESS_TIMEOUT + Duration::from_secs(60));
        let mut emitter = LivenessEmitter::default();

        emitter.next_tick();
        // Rend la main : sans la borne de `beat`, ce `await` ne finirait jamais.
        emitter
            .beat(&cfg, &sink, &outcome(false, Severity::Healthy))
            .await;

        assert_eq!(
            sink.count(),
            0,
            "le sink n'a jamais enregistré : la borne a coupé avant qu'il réponde"
        );
        // Et l'état de l'émetteur porte la panne, pas un succès.
        assert_eq!(emitter.consecutive_failures, 1);
        assert!(emitter.failing);
    }

    // ---- V4 — AC2 : le motif est un format de fil -------------------------

    /// **V4 / AC2 / D8** — les deux motifs sont distincts et leurs formes
    /// exactes sont figées.
    #[test]
    fn mika1990_le_vocabulaire_du_reason_est_un_format_de_fil() {
        assert_eq!(liveness_reason(7, false, &Severity::Healthy), "poll:7");
        assert_eq!(liveness_reason(1, false, &Severity::Blocked), "poll:1");
        assert_eq!(
            liveness_reason(42, true, &Severity::Healthy),
            "delivery:healthy"
        );
        assert_eq!(
            liveness_reason(42, true, &Severity::Attention),
            "delivery:attention"
        );
        assert_eq!(
            liveness_reason(42, true, &Severity::Blocked),
            "delivery:blocked"
        );

        // Les deux préfixes ne peuvent pas se confondre : un `GROUP BY` sur le
        // préfixe sépare les deux populations.
        assert_ne!(REASON_POLL_PREFIX, REASON_DELIVERY_PREFIX);
        assert!(!liveness_reason(1, false, &Severity::Healthy).starts_with(REASON_DELIVERY_PREFIX));
        assert!(!liveness_reason(1, true, &Severity::Healthy).starts_with(REASON_POLL_PREFIX));

        // La classe d'échec est un format de fil elle aussi (champ `class=`).
        assert_eq!(
            LivenessFailureClass::CredentialRefused.as_str(),
            "credential_refused"
        );
        assert_eq!(LivenessFailureClass::Unreachable.as_str(), "unreachable");
        assert_eq!(LivenessFailureClass::Other.as_str(), "other");
    }

    /// La sévérité du motif suit **le rendu réel de `serde`**, pas une seconde
    /// table écrite à la main. Sans ce test, le `match` de `severity_wire_name`
    /// pourrait diverger de `#[serde(rename_all = "snake_case")]` en silence, et
    /// cm verrait deux orthographes de la même sévérité.
    #[test]
    fn mika1990_la_severite_du_motif_suit_serde() {
        for sev in [Severity::Healthy, Severity::Attention, Severity::Blocked] {
            let serde_form = serde_json::to_string(&sev).expect("Severity sérialisable");
            let serde_form = serde_form.trim_matches('"');
            assert_eq!(
                severity_wire_name(&sev),
                serde_form,
                "le nom de fil de {sev:?} a divergé de serde"
            );
        }
    }

    // ---- V5 — D3 : un cycle en erreur crée un trou -------------------------

    /// **V5 / D3 / AC6** — un cycle en erreur ne bat pas, et le trou dans la
    /// suite de `<n>` est conservé comme information : `poll:5` puis `poll:7`,
    /// jamais `poll:5` puis `poll:6`.
    ///
    /// Le site d'appel incrémente en tête d'itération ; seul le bras `Ok`
    /// appelle `beat`. Ici on rejoue cette forme.
    #[tokio::test]
    async fn mika1990_un_cycle_en_erreur_laisse_un_trou_dans_la_suite() {
        let cfg = cfg_with_url(Some("https://cm.example.test/beat"));
        let sink = RecordingSink::new();
        let mut emitter = LivenessEmitter::default();

        // ticks 1..=4 : Ok — ils battent.
        for _ in 1..=4 {
            emitter.next_tick();
            emitter
                .beat(&cfg, &sink, &outcome(false, Severity::Healthy))
                .await;
        }
        // ticks 5 et 6 : le cycle échoue — le compteur avance, rien ne bat.
        emitter.next_tick();
        emitter.next_tick();
        // tick 7 : Ok de nouveau.
        emitter.next_tick();
        emitter
            .beat(&cfg, &sink, &outcome(false, Severity::Healthy))
            .await;

        let reasons = sink.reasons();
        assert_eq!(
            reasons.last().map(String::as_str),
            Some("poll:7"),
            "après deux cycles en erreur le motif saute de poll:4 à poll:7 — le trou EST l'information"
        );
        assert!(
            !reasons.contains(&"poll:5".to_string()) && !reasons.contains(&"poll:6".to_string()),
            "un cycle en erreur ne bat pas (D2) : {reasons:?}"
        );
    }

    /// Le compteur repart à `1` sur un émetteur neuf — `poll:1` est le marqueur
    /// d'un redémarrage, pas un bug.
    #[test]
    fn mika1990_le_compteur_repart_a_un_au_demarrage() {
        let mut emitter = LivenessEmitter::default();
        assert_eq!(emitter.tick(), 0, "avant le premier tick");
        assert_eq!(emitter.next_tick(), 1);
        assert_eq!(emitter.next_tick(), 2);
    }

    // ---- V6 — D7 : le journal est discret et parle aux transitions ---------

    /// **V6 / D7 / AC4** — un WARN sur la **première** panne, rien sur les
    /// suivantes, un INFO au rétablissement. Un battement réussi hors
    /// transition n'écrit rien.
    #[test]
    fn mika1990_le_journal_ne_parle_quaux_transitions() {
        let mut e = LivenessEmitter::default();

        // sain → rien
        assert_eq!(e.observe(Ok(())), None, "un battement réussi n'écrit rien");

        // sain → cassé : un WARN, portant la classe et le compte
        let first = e.observe(Err(LivenessFailure {
            class: LivenessFailureClass::Unreachable,
            message: "connexion refusée".into(),
        }));
        assert_eq!(
            first,
            Some(TransitionLine::Failed {
                consecutive_failures: 1,
                class: LivenessFailureClass::Unreachable,
                message: "connexion refusée".into(),
            })
        );

        // cassé → cassé : rien (sinon la ligne serait un flot, pas un signal)
        assert_eq!(
            e.observe(Err(LivenessFailure {
                class: LivenessFailureClass::Unreachable,
                message: "connexion refusée".into(),
            })),
            None
        );
        assert_eq!(
            e.observe(Err(LivenessFailure {
                class: LivenessFailureClass::Other,
                message: "500".into(),
            })),
            None,
            "même un changement de classe ne rouvre pas la transition"
        );

        // cassé → sain : un INFO, nommant combien d'échecs il referme
        assert_eq!(
            e.observe(Ok(())),
            Some(TransitionLine::Recovered { failures_before: 3 })
        );

        // et on est revenu au régime silencieux
        assert_eq!(e.observe(Ok(())), None);
    }

    /// Le WARN porte la classe **du statut**, jamais une sous-chaîne du message
    /// (règle mika#2179) : la classe traverse `observe` intacte.
    #[test]
    fn mika1990_la_classe_dechec_traverse_observe_intacte() {
        for class in [
            LivenessFailureClass::CredentialRefused,
            LivenessFailureClass::Unreachable,
            LivenessFailureClass::Other,
        ] {
            let mut e = LivenessEmitter::default();
            let line = e.observe(Err(LivenessFailure {
                class,
                message: "peu importe le texte".into(),
            }));
            match line {
                Some(TransitionLine::Failed { class: got, .. }) => assert_eq!(got, class),
                other => panic!("attendu une transition Failed, obtenu {other:?}"),
            }
        }
    }

    // ---- V7 — Fire-Disposition : les deux scans ---------------------------

    /// Sites de production autorisés à composer un motif ou à appeler le sink,
    /// hors du propriétaire.
    ///
    /// **Livrée vide, et c'est la disposition (doctrine mika#2201).** Le code est
    /// neuf : il n'existe aucune violation préexistante à exempter. Quand le scan
    /// tire, **on retire le second site — on ne l'allowliste pas.**
    const LIVENESS_BEAT_SITES_ALLOWED: &[&str] = &[];

    fn crates_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/mika-agent a un parent")
            .to_path_buf()
    }

    /// La moitié « production » d'un fichier source. `None` quand le fichier
    /// **entier** est du code de test — la leçon de mika#2321 : un module de test
    /// extrait ne porte aucun littéral `#[cfg(test)]`, donc la troncature seule
    /// le scannerait comme de la production.
    fn production_half(path: &Path, src: &str) -> Option<String> {
        if crate::source_scan::is_test_source_path(path) {
            return None;
        }
        Some(match src.find("#[cfg(test)]") {
            Some(i) => src[..i].to_string(),
            None => src.to_string(),
        })
    }

    fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(read) = fs::read_dir(dir) else { return };
        for item in read.flatten() {
            let p = item.path();
            if p.is_dir() {
                walk_rs(&p, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(p);
            }
        }
    }

    /// Les trois cardinalités du battement, comptées sur la moitié production
    /// d'une source.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct BeatSites {
        /// Sites de composition du motif — la définition de `liveness_reason`
        /// n'en est pas un, d'où le filtre sur `fn liveness_reason(`.
        reason: usize,
        /// Sites d'appel au sink (`.deliver_beat(`).
        sink: usize,
        /// Sites de câblage — l'appel `<emitter>.beat(…)` depuis une boucle.
        /// `.deliver_beat(` ne matche pas (le caractère qui précède `beat(` y est
        /// `_`, pas `.`), ni la définition `pub async fn beat(`.
        wiring: usize,
    }

    /// Le prédicat, extrait pour que le **contrôle de bonne foi** ci-dessous
    /// atteste exactement celui que le scan applique.
    fn count_beat_sites(prod: &str) -> BeatSites {
        let mut sites = BeatSites::default();
        for line in prod.lines() {
            // Les commentaires *décrivent* le canal ; ils ne l'écrivent pas
            // (classe mika#2050 : la prose d'une session à propos d'un signal a
            // déjà été lue une fois comme une émission de ce signal).
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains("liveness_reason(") && !line.contains("fn liveness_reason(") {
                sites.reason += 1;
            }
            if line.contains(".deliver_beat(") {
                sites.sink += 1;
            }
            if line.contains(".beat(") {
                sites.wiring += 1;
            }
        }
        sites
    }

    /// **V7 / Fire-Disposition** — le battement a un seul écrivain.
    ///
    /// **Pourquoi un scan et pas un test de comportement :** un second écrivain
    /// de battement ne rend **aucune décision fausse** le jour où il est écrit —
    /// le cycle continue de tourner, chaque assertion reste verte, et seule la
    /// freshness devient incomptable (deux suites de `<n>` entrelacées chez cm).
    /// Invisible à tout test comportemental. C'est très exactement la classe que
    /// `grooming_marker` (mika#2158) a dû fermer dans ce dépôt.
    #[test]
    fn mika1990_le_battement_a_un_seul_ecrivain() {
        let src_root = crates_dir().join("mika-agent").join("src");
        assert!(
            src_root.is_dir(),
            "racine de scan introuvable: {} — la garde ne couvre plus ce qu'elle prétend couvrir",
            src_root.display()
        );
        let mut files = Vec::new();
        walk_rs(&src_root, &mut files);
        assert!(
            files.len() > 100,
            "scan suspicieusement court: {} fichiers",
            files.len()
        );

        let mut total = BeatSites::default();
        let mut sites: Vec<String> = Vec::new();
        // Assertion d'anti-vacuité : le nom que le scan cherche doit exister
        // quelque part dans l'arbre. Un scan visant un nom mort vérifie zéro
        // chose et se lit exactement comme un arbre propre (classe mika#2205).
        let mut name_is_written_somewhere = false;

        for path in &files {
            let Ok(src) = fs::read_to_string(path) else {
                continue;
            };
            if src.contains("liveness_reason") {
                name_is_written_somewhere = true;
            }
            let Some(prod) = production_half(path, &src) else {
                continue;
            };
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if LIVENESS_BEAT_SITES_ALLOWED.contains(&file_name.as_str()) {
                continue;
            }
            let found = count_beat_sites(&prod);
            if found != BeatSites::default() {
                sites.push(format!("{}: {found:?}", path.display()));
            }
            total.reason += found.reason;
            total.sink += found.sink;
            total.wiring += found.wiring;
        }

        assert!(
            name_is_written_somewhere,
            "le scan cherche un nom que l'arbre n'écrit nulle part — il ne vérifie rien"
        );
        // Cardinalité, contre le rétrécissement silencieux : sans elle un
        // prédicat devenu trop étroit passerait en ne regardant rien
        // (mika#2496 U3).
        let seen = sites.join("\n  ");
        assert_eq!(
            total.reason, 1,
            "le motif doit être composé à UN SEUL site de production — retirez le second, \
             ne l'allowlistez pas (mika#2201). Sites vus :\n  {seen}"
        );
        assert_eq!(
            total.sink, 1,
            "le sink doit être appelé depuis UN SEUL site de production. Sites vus :\n  {seen}"
        );
        assert_eq!(
            total.wiring, 1,
            "l'émetteur doit être câblé à UNE SEULE boucle : deux câblages produiraient deux \
             suites de `<n>` entrelacées chez cm, et la freshness deviendrait incomptable — \
             ce qu'aucun test de comportement ne peut voir. Sites vus :\n  {seen}"
        );
    }

    /// **Contrôle de bonne foi du scan** — sans lui, « le scan regarde » et « le
    /// scan est inerte » se lisent pareil (classe mika#2205).
    #[test]
    fn mika1990_le_scan_du_battement_voit_un_second_site() {
        let propre = r#"
            pub fn liveness_reason(tick: u64) -> String { format!("poll:{tick}") }
            pub async fn beat(&mut self) {
                let r = liveness_reason(self.tick);
                sink.deliver_beat(url, token, &body).await
            }
            fn boucle() { liveness.beat(&cfg, sink, &outcome).await; }
        "#;
        assert_eq!(
            count_beat_sites(propre),
            BeatSites {
                reason: 1,
                sink: 1,
                wiring: 1
            },
            "ni `fn liveness_reason(` ni `fn beat(` ne sont des sites d'appel"
        );

        let fautif = r#"
            pub async fn beat(&mut self) {
                let r = liveness_reason(self.tick);
                sink.deliver_beat(url, token, &body).await
            }
            pub async fn un_second_ecrivain(&mut self) {
                let r = liveness_reason(99);
                other_sink.deliver_beat(url, None, &body).await
            }
            fn boucle_a() { liveness.beat(&cfg, sink, &a).await; }
            fn boucle_b() { autre_emetteur.beat(&cfg, sink, &b).await; }
        "#;
        assert_eq!(
            count_beat_sites(fautif),
            BeatSites {
                reason: 2,
                sink: 2,
                wiring: 2
            },
            "le scan doit VOIR un second écrivain et un second câblage — sinon il est inerte"
        );

        let en_prose = r#"
            // Ce commentaire parle de liveness_reason( et de .deliver_beat( sans les appeler.
            /// Idem en doc-comment : liveness_reason(, .deliver_beat( et .beat(.
        "#;
        assert_eq!(
            count_beat_sites(en_prose),
            BeatSites::default(),
            "la prose décrit le canal, elle ne l'écrit pas (classe mika#2050)"
        );
    }

    /// L'allowlist est livrée **vide** et doit le rester : une allowlist née
    /// vide est un endroit où déposer la prochaine infraction (mika#2323).
    #[test]
    fn mika1990_lallowlist_du_battement_est_vide() {
        assert!(
            LIVENESS_BEAT_SITES_ALLOWED.is_empty(),
            "quand le scan tire, on retire le second site — on n'ajoute pas d'entrée"
        );
    }

    /// La variable est déclarée sous le nom que la doc annonce, et **pas** sous
    /// un nom qui la ferait confondre avec le plancher de delivery 6 h (R4).
    #[test]
    fn mika1990_le_nom_de_la_variable_ne_dit_pas_heartbeat() {
        assert_eq!(ENV_LIVENESS_URL, "MIKA_MANAGER_LIVENESS_URL");
        assert!(
            !ENV_LIVENESS_URL.contains("HEARTBEAT"),
            "MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS désigne le plancher 6 h : un nom en \
             HEARTBEAT ferait lire « ce battement bat toutes les 6 h », la croyance fausse \
             que ce ticket existe pour tuer"
        );
    }
}
