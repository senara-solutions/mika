//! Anti-rebond des `pull_request.synchronize` (mika#2671, phase B1).
//!
//! # Le défaut que ce module ferme
//!
//! La coalescence de [`super::webhook_queue_v2`] (clé `pr_sync:{repo}:{pr}`) ne
//! fusionne que des événements **encore en file**. La file de mika-qa est presque
//! toujours vide, donc chaque `synchronize` part aussitôt et paie une revue
//! complète : trois `synchronize` en douze minutes sur mika#2659 ont coûté trois
//! revues et deux callbacks de build, ≈ 4,5 M de tokens d'entrée, pour une seule
//! tête utile.
//!
//! # Fenêtre fixe, dernier gagnant
//!
//! Un `synchronize` pour une PR sans fenêtre ouverte **ouvre** une fenêtre de W
//! secondes ; tout `synchronize` de la même PR reçu pendant la fenêtre
//! **remplace** l'événement retenu. À l'échéance, le dernier retenu est versé
//! dans la file v2 comme s'il venait d'arriver. La fenêtre n'est pas réarmée à
//! chaque événement : un débit continu de pushes ne retarde pas la revue au-delà
//! de W.
//!
//! Seul `PullRequestSync` est retenu ([`debounce_key`]) : `opened`,
//! `ready_for_review`, `review_requested` sont classés `Other` par
//! `classify_event` et ne peuvent pas être retardés (AC4).
//!
//! # Durabilité
//!
//! La table en mémoire disparaît au redémarrage, et un redémarrage pendant la
//! fenêtre perdrait la revue de la dernière tête — un risque que la fenêtre
//! **crée**. Chaque retenue est donc inscrite au registre de phase A
//! (`qa_pr_sync_observed`, même clé, `after_value = "stage=held"`, texte de
//! l'événement dans `before_value`) par `server::handlers`, et
//! `handlers::recover_held_syncs` remet en fenêtre, au démarrage du worker de
//! drain, toute retenue qu'aucun tour n'a suivie.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::types::MessageRequest;
use super::webhook_queue_v2::{EnqueueResult, WebhookEventKind, WebhookQueue, classify_event};

/// Kill-switch. Armé par défaut ; `0`/`false`/`off`/`no` désarme.
pub const DEBOUNCE_SWITCH_ENV: &str = "MIKA_QA_SYNC_DEBOUNCE";

/// Durée de la fenêtre, en secondes.
pub const DEBOUNCE_WINDOW_ENV: &str = "MIKA_QA_SYNC_DEBOUNCE_SECS";

/// Fenêtre par défaut : 5 min (corps de mika#2671).
pub const DEFAULT_WINDOW_SECS: u64 = 300;

/// Au-delà d'une heure, une fenêtre retarde la revue plus qu'elle n'économise :
/// la valeur est traitée comme une coquille.
pub const MAX_WINDOW_SECS: u64 = 3600;

/// `audit_events.after_value` d'une retenue au registre `qa_pr_sync_observed`.
/// Les lignes de tour démarré (phase A) gardent `after_value` NULL.
pub const HELD_STAGE: &str = "stage=held";

/// Horizon de reprise : une retenue plus vieille n'est pas rejouée.
pub const RECOVERY_HORIZON_SECS: i64 = 86_400;

/// Identité d'un événement au registre, portée par `reasoning` sur la ligne de
/// retenue ET sur la ligne de tour démarré. Le texte du gateway ne suffit pas :
/// il est identique pour tous les `synchronize` d'une même PR (titre, branche,
/// URL), donc seul le `request_id` (un UUID par livraison) dit QUEL événement un
/// tour a consommé. Une seule écriture du format, ici.
pub fn request_marker(request_id: &str) -> String {
    format!("request_id={request_id}")
}

/// Rend `request_id` depuis [`request_marker`], `None` sur toute autre forme.
pub fn parse_request_marker(reasoning: &str) -> Option<&str> {
    reasoning
        .strip_prefix("request_id=")
        .filter(|id| !id.is_empty())
}

/// Fenêtre en vigueur, ou `None` si l'anti-rebond est désarmé.
pub fn window_from_env() -> Option<Duration> {
    let armed = crate::qa_head_supersession::parse_switch(
        DEBOUNCE_SWITCH_ENV,
        std::env::var(DEBOUNCE_SWITCH_ENV).ok().as_deref(),
    );
    armed.then(|| {
        Duration::from_secs(parse_window_secs(
            std::env::var(DEBOUNCE_WINDOW_ENV).ok().as_deref(),
        ))
    })
}

/// Trois paliers maison : absent ou vide → défaut ; illisible, `0`, négatif ou
/// au-delà de [`MAX_WINDOW_SECS`] → défaut **et** WARN nommant la valeur. Le `0`
/// ne désarme pas : c'est le rôle du kill-switch.
pub fn parse_window_secs(raw: Option<&str>) -> u64 {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return DEFAULT_WINDOW_SECS;
    };
    match raw.parse::<u64>() {
        Ok(n) if (1..=MAX_WINDOW_SECS).contains(&n) => n,
        _ => {
            tracing::warn!(
                event = "qa_sync_debounce_window_invalid",
                value = %format!("\"{raw}\""),
                default = DEFAULT_WINDOW_SECS,
                "valeur illisible pour {DEBOUNCE_WINDOW_ENV} — fenêtre par défaut"
            );
            DEFAULT_WINDOW_SECS
        }
    }
}

/// La clé de fenêtre d'un événement : `Some` pour un `synchronize` seulement,
/// et c'est la clé du registre de phase A (une seule grammaire, mika#2158).
pub fn debounce_key(text: &str) -> Option<String> {
    match classify_event(text) {
        WebhookEventKind::PullRequestSync { repo, pr } => {
            Some(crate::qa_head_supersession::sync_observed_key(&repo, pr))
        }
        _ => None,
    }
}

/// Issue d'une retenue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// Aucune fenêtre ouverte pour cette PR : celle-ci l'ouvre.
    Opened,
    /// Une fenêtre était ouverte : l'événement retenu est remplacé.
    Replaced,
}

impl Hold {
    pub fn label(self) -> &'static str {
        match self {
            Hold::Opened => "opened",
            Hold::Replaced => "replaced",
        }
    }
}

/// Les `synchronize` retenus d'un agent, un par PR. Verrou synchrone : aucune
/// section critique ne traverse un `.await`.
#[derive(Default)]
pub struct SyncDebounce {
    held: Mutex<HashMap<String, MessageRequest>>,
    /// Clés dont un événement a quitté la file et attend ou exécute son tour —
    /// ni retenues, ni en file, ni encore « démarrées » au registre. Lu par le
    /// balayage des pendants pour ne pas rejouer un événement vivant.
    in_flight: Mutex<HashMap<String, usize>>,
}

impl SyncDebounce {
    /// Retient `req` sous `key`, en remplaçant l'éventuel retenu.
    pub fn hold(&self, key: String, req: MessageRequest) -> Hold {
        let mut held = self.held.lock().unwrap_or_else(|p| p.into_inner());
        match held.insert(key, req) {
            Some(_) => Hold::Replaced,
            None => Hold::Opened,
        }
    }

    /// Ferme la fenêtre de `key` et rend le dernier retenu.
    pub fn take(&self, key: &str) -> Option<MessageRequest> {
        self.held
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key)
    }

    /// Retient `req` seulement si aucune retenue n'existe pour `key`. Chemin de
    /// la reprise : un rejeu ne doit jamais remplacer une tête retenue en direct,
    /// qui est par construction plus récente.
    pub fn hold_if_absent(&self, key: String, req: MessageRequest) -> bool {
        let mut held = self.held.lock().unwrap_or_else(|p| p.into_inner());
        if held.contains_key(&key) {
            return false;
        }
        held.insert(key, req);
        true
    }

    pub fn is_held(&self, key: &str) -> bool {
        self.held
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(key)
    }

    /// Marque `key` en vol (sortie de file, tour pas encore démarré ou en cours).
    pub fn enter_flight(&self, key: &str) {
        *self
            .in_flight
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(key.to_string())
            .or_default() += 1;
    }

    pub fn leave_flight(&self, key: &str) {
        let mut f = self.in_flight.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(n) = f.get_mut(key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                f.remove(key);
            }
        }
    }

    pub fn is_in_flight(&self, key: &str) -> bool {
        self.in_flight
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(key)
    }

    #[cfg(test)]
    pub fn held_count(&self) -> usize {
        self.held.lock().unwrap().len()
    }
}

/// Retient `req` et, si la fenêtre s'ouvre, lance la tâche d'échéance qui versera
/// le dernier retenu dans `queue` après `window`. Aucun accès base : l'écriture
/// durable est faite par l'appelant (SOLE WRITER du registre, `server::handlers`).
pub fn admit(
    debounce: &Arc<SyncDebounce>,
    queue: &Arc<WebhookQueue>,
    key: String,
    req: MessageRequest,
    window: Duration,
) -> Hold {
    let outcome = debounce.hold(key.clone(), req);
    if outcome == Hold::Opened {
        let debounce = Arc::clone(debounce);
        let queue = Arc::clone(queue);
        // L'échéance est fixée à l'ouverture, pas au premier poll de la tâche :
        // un runtime chargé ne doit pas allonger la fenêtre.
        let deadline = tokio::time::Instant::now() + window;
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            close_window(&debounce, &queue, &key).await;
        });
    }
    outcome
}

/// Variante de reprise de [`admit`] : n'ouvre une fenêtre que si aucune
/// retenue n'existe pour `key` (voir [`SyncDebounce::hold_if_absent`]).
pub fn admit_recovered(
    debounce: &Arc<SyncDebounce>,
    queue: &Arc<WebhookQueue>,
    key: String,
    req: MessageRequest,
    window: Duration,
) -> bool {
    if !debounce.hold_if_absent(key.clone(), req) {
        return false;
    }
    let debounce = Arc::clone(debounce);
    let queue = Arc::clone(queue);
    let deadline = tokio::time::Instant::now() + window;
    tokio::spawn(async move {
        tokio::time::sleep_until(deadline).await;
        close_window(&debounce, &queue, &key).await;
    });
    true
}

/// Échéance : le dernier retenu part dans la file v2. Rien à verser si la fenêtre
/// a déjà été fermée (course impossible aujourd'hui, une seule tâche par clé).
async fn close_window(debounce: &SyncDebounce, queue: &WebhookQueue, key: &str) {
    let Some(req) = debounce.take(key) else {
        return;
    };
    let result = queue.enqueue(req).await;
    if result == EnqueueResult::Dropped {
        tracing::warn!(
            event = "qa_sync_debounce_released_into_full_queue",
            target = %key,
            "file v2 pleine à l'échéance — l'événement le plus ancien a été évincé"
        );
    } else {
        tracing::info!(
            event = "qa_sync_debounce_released",
            target = %key,
            "fenêtre d'anti-rebond close — la dernière tête part en revue"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: Duration = Duration::from_secs(300);

    fn sync(pr: u64, n: u32) -> MessageRequest {
        MessageRequest {
            text: format!(
                "[GitHub] PR synchronize: senara-solutions/mika#{pr} \u{2014} fix: x (branch: b)\nhead {n}"
            ),
            chat_id: None,
            channel: "github".into(),
            request_id: format!("r-{pr}-{n}"),
            agent: "mika-qa".into(),
            images: None,
        }
    }

    fn setup() -> (Arc<SyncDebounce>, Arc<WebhookQueue>) {
        (
            Arc::new(SyncDebounce::default()),
            Arc::new(WebhookQueue::new(64, Duration::from_millis(100))),
        )
    }

    fn admit_sync(d: &Arc<SyncDebounce>, q: &Arc<WebhookQueue>, req: MessageRequest) -> Hold {
        let key = debounce_key(&req.text).expect("un synchronize a une clé");
        admit(d, q, key, req, W)
    }

    /// Dépile avec une borne : une fenêtre qui ne se ferme jamais fait ÉCHOUER
    /// le test au lieu de le bloquer.
    async fn pop(q: &WebhookQueue) -> MessageRequest {
        tokio::time::timeout(Duration::from_secs(1), q.dequeue())
            .await
            .expect("rien en file : la fenêtre ne s'est pas fermée")
            .unwrap()
            .request
    }

    /// Laisse tourner les tâches réveillées par une avance d'horloge.
    async fn settle() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    /// AC1 — trois `synchronize` dans la fenêtre ⇒ un seul élément en file, la
    /// dernière tête.
    #[tokio::test(start_paused = true)]
    async fn mika2671_trois_sync_dans_la_fenetre_un_seul_tour_sur_la_derniere_tete() {
        let (d, q) = setup();
        assert_eq!(admit_sync(&d, &q, sync(2659, 1)), Hold::Opened);
        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(admit_sync(&d, &q, sync(2659, 2)), Hold::Replaced);
        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(admit_sync(&d, &q, sync(2659, 3)), Hold::Replaced);
        settle().await;
        assert_eq!(q.depth().await, 0, "rien ne part avant l'échéance");

        tokio::time::advance(Duration::from_secs(181)).await;
        settle().await;
        assert_eq!(q.depth().await, 1);
        let item = q.dequeue().await.unwrap();
        assert!(
            item.request.text.ends_with("head 3"),
            "{}",
            item.request.text
        );
        assert_eq!(d.held_count(), 0, "la fenêtre est close");
    }

    /// AC1 contrôle négatif — deux `synchronize` séparés de plus que la fenêtre ⇒
    /// deux tours, dans l'ordre.
    #[tokio::test(start_paused = true)]
    async fn mika2671_deux_sync_separes_de_plus_que_la_fenetre_deux_tours() {
        // Le drain consomme le premier avant l'arrivée du second, comme en
        // production : sans ce dequeue, la coalescence de la file v2 fusionnerait
        // les deux et le test lirait 1 pour une raison qui n'est pas la fenêtre.
        let (d, q) = setup();
        assert_eq!(admit_sync(&d, &q, sync(2659, 1)), Hold::Opened);
        tokio::time::advance(W + Duration::from_secs(1)).await;
        settle().await;
        let first = pop(&q).await;
        assert!(first.text.ends_with("head 1"));
        assert_eq!(admit_sync(&d, &q, sync(2659, 2)), Hold::Opened);
        tokio::time::advance(W + Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(q.depth().await, 1);
        assert!(pop(&q).await.text.ends_with("head 2"));
    }

    /// Une fenêtre par PR : deux PR dans la même fenêtre sont deux tours.
    #[tokio::test(start_paused = true)]
    async fn mika2671_deux_pr_dans_la_meme_fenetre_deux_tours() {
        let (d, q) = setup();
        admit_sync(&d, &q, sync(2659, 1));
        admit_sync(&d, &q, sync(2660, 1));
        tokio::time::advance(W + Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(q.depth().await, 2);
    }

    /// AC4 — seul `synchronize` est retenu ; contrôle négatif : les trois actions
    /// qui demandent une revue n'ont pas de clé, donc ne sont jamais retardées.
    #[test]
    fn mika2671_seul_synchronize_est_retenu() {
        assert_eq!(
            debounce_key(&sync(2659, 1).text).as_deref(),
            Some("pr:senara-solutions/mika#2659")
        );
        for text in [
            "[GitHub] PR opened: senara-solutions/mika#2659 \u{2014} fix: x (branch: b)\nu",
            "[GitHub] PR ready_for_review: senara-solutions/mika#2659 \u{2014} fix: x (branch: b)\nu",
            "[GitHub] PR review_requested: senara-solutions/mika#2659 \u{2014} fix: x (branch: b)\nu",
            "[GitHub] PR closed: senara-solutions/mika#2659 \u{2014} fix: x (branch: b)\nMerged: true",
            "[GitHub] PR review (approved) on senara-solutions/mika#2659 (fix: x) by @mika-platform-qa\nu\n\nVERDICT: pass",
            "[GitHub] Check suite success on senara-solutions/mika (branch: b)",
            "salut Mika",
        ] {
            assert_eq!(debounce_key(text), None, "{text}");
        }
    }

    /// La reprise ne remplace jamais une retenue en direct (plus récente).
    #[tokio::test(start_paused = true)]
    async fn mika2671_la_reprise_ne_remplace_pas_une_retenue_vivante() {
        let (d, q) = setup();
        admit_sync(&d, &q, sync(2659, 9));
        let key = debounce_key(&sync(2659, 1).text).unwrap();
        assert!(!admit_recovered(&d, &q, key.clone(), sync(2659, 1), W));
        // Contrôle : sans retenue, la reprise ouvre bien une fenêtre.
        let other = debounce_key(&sync(2660, 1).text).unwrap();
        assert!(admit_recovered(&d, &q, other, sync(2660, 1), W));
        tokio::time::advance(W + Duration::from_secs(1)).await;
        settle().await;
        let mut heads = vec![];
        while q.depth().await > 0 {
            heads.push(pop(&q).await.text);
        }
        assert!(heads.iter().any(|t| t.ends_with("head 9")), "{heads:?}");
        assert!(
            !heads
                .iter()
                .any(|t| t.contains("#2659") && t.ends_with("head 1"))
        );
    }

    #[test]
    fn mika2671_le_marqueur_de_requete_est_un_format_de_fil() {
        assert_eq!(request_marker("abc"), "request_id=abc");
        assert_eq!(parse_request_marker("request_id=abc"), Some("abc"));
        assert_eq!(parse_request_marker("request_id="), None);
        assert_eq!(parse_request_marker("tour de revue démarré"), None);
    }

    #[test]
    fn mika2671_le_vol_se_compte() {
        let d = SyncDebounce::default();
        d.enter_flight("k");
        d.enter_flight("k");
        d.leave_flight("k");
        assert!(d.is_in_flight("k"));
        d.leave_flight("k");
        assert!(!d.is_in_flight("k"));
    }

    #[test]
    fn mika2671_la_fenetre_a_trois_paliers_et_une_borne() {
        assert_eq!(parse_window_secs(None), 300);
        assert_eq!(parse_window_secs(Some("")), 300);
        assert_eq!(parse_window_secs(Some(" 120 ")), 120);
        assert_eq!(parse_window_secs(Some("3600")), 3600);
        for bad in ["0", "-5", "abc", "3601", "1.5"] {
            assert_eq!(parse_window_secs(Some(bad)), 300, "{bad}");
        }
    }
}
