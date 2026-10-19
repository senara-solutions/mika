//! Détection du plafond de contexte, pour que la cérémonie précède la
//! condensation (mika#1748, vol 1).
//!
//! La compaction est une transition de vaisseau : la fenêtre se condense, les
//! *bearings* survivent (mémoire persistée, fichiers, courrier), le résidu
//! meurt. mp#187 a livré la forme du rituel pour le changement de **modèle** ;
//! ce module en porte le déclencheur pour la condensation du **contexte**.
//!
//! Il ne contient que le prédicat et son réglage. Le rendu vit dans
//! [`crate::prompt::build_system_prompt`], l'alimentation dans
//! `agent_loop::load_agent_context` — site unique, parce que c'est l'entonnoir
//! des trois boucles.
//!
//! # D2 — le seuil se mesure en messages, pas en tokens
//!
//! AC1 du ticket dit « reads context usage ; threshold (default 85 %) ».
//! **mika-agent n'instrumente pas un ratio d'occupation de fenêtre.** La
//! grandeur sur laquelle le substrat décide réellement de condenser est
//! `db.count_messages()` comparée à [`COMPACTION_THRESHOLD`]. Le seuil de
//! cérémonie s'exprime donc en pourcentage de ce même compteur : 85 % de 50 →
//! **43 messages**.
//!
//! Un pourcentage de tokens serait un *second* instrument, mesurant une autre
//! grandeur que celle qui déclenche l'événement qu'il prétend précéder : il
//! pourrait rester sous son seuil au moment où la compaction tire, ou tirer
//! trois fois sans qu'elle approche. Un détecteur désaligné de son déclencheur
//! est un détecteur qui mentira le jour où la réponse compte.
//!
//! # D3 — ce qui meurt ici n'est pas ce qui meurt dans Claude Code
//!
//! L'observation fondatrice (2026-07-07, 100 % → 12 %) porte sur une session
//! Claude Code, où la compaction remplace **tout** l'historique par un
//! sommaire. `maybe_compact` préserve les 20 derniers messages et ne résume que
//! le reste. Deux conséquences :
//!
//! 1. La cérémonie ne risque pas d'être avalée par la compaction qui la suit —
//!    elle atterrit dans la fenêtre préservée. Le pré-seuil n'est pas là pour
//!    la sauver d'elle-même.
//! 2. Ce qui meurt, ce sont les messages `1..N−20` : le résidu porté par la
//!    tête de conversation. C'est pour lui que la cérémonie existe.
//!
//! # D4 — le prédicat est évalué en début de tour, la compaction en fin de tour
//!
//! `load_agent_context` tourne avant l'appel LLM, `maybe_compact` après. Le
//! message utilisateur du tour est **déjà persisté** quand le prédicat compte
//! (`run_agent_with_deadline` l'enregistre avant d'entrer dans la boucle), donc
//! le décalage jusqu'à `maybe_compact` est de **≥ 1** : la réponse de
//! l'assistant, plus chaque message intermédiaire que le tour produit. Il est
//! assumé et borné : le prédicat franchit à 43, la compaction tire au-dessus de
//! 50 — 7 messages de marge. Le prédicat n'est pas déplacé en fin de tour,
//! parce qu'un avertissement posté après le tour arriverait après la
//! condensation qu'il annonce.

use tracing::warn;

use crate::compaction::COMPACTION_THRESHOLD;

/// Pourcentage de [`COMPACTION_THRESHOLD`] au-delà duquel la cérémonie est
/// proposée. 85 % de 50 → 43 messages (AC1).
pub const DEFAULT_THRESHOLD_PCT: u32 = 85;

/// Variable d'environnement qui règle le seuil, en pourcentage.
pub const THRESHOLD_PCT_ENV: &str = "MIKA_COMPACTION_HANDSOFF_THRESHOLD_PCT";

/// Seuil en force, lu depuis l'environnement.
///
/// Précédent de forme : [`crate::db::dispatch_slot_lease_ttl_secs`] — un
/// réglage à un seul lecteur se lit à son point d'usage, sans traverser
/// `Settings`. `load_agent_context` n'en reçoit pas, et en faire passer un
/// créerait deux lecteurs d'un même réglage.
pub fn threshold_pct() -> u32 {
    resolve_threshold_pct(std::env::var(THRESHOLD_PCT_ENV).ok().as_deref())
}

/// Le cœur pur de [`threshold_pct`] : aucune lecture d'environnement, donc
/// testable sans `unsafe { set_var }` et sans sérialisation de la suite.
///
/// Toute valeur hors `1..=100` retombe sur [`DEFAULT_THRESHOLD_PCT`] **en le
/// disant** — un réglage refusé qui se tait est indistinguable d'un réglage
/// appliqué (mika#2293). `0` est refusé comme toute autre valeur hors borne :
/// il n'y a pas de sentinelle de désarmement sur cet axe à ce vol, et le
/// laisser signifier « à chaque tour » serait un troisième comportement que
/// personne n'a demandé.
pub(crate) fn resolve_threshold_pct(raw: Option<&str>) -> u32 {
    let Some(raw) = raw else {
        return DEFAULT_THRESHOLD_PCT;
    };

    match raw.trim().parse::<u32>() {
        Ok(pct) if (1..=100).contains(&pct) => pct,
        _ => {
            warn!(
                target: "mika::otel",
                event = "compaction_handsoff_threshold_invalid",
                received = raw,
                default_pct = DEFAULT_THRESHOLD_PCT,
                env = THRESHOLD_PCT_ENV,
                "seuil de handsoff de compaction hors 1..=100 — le défaut s'applique"
            );
            DEFAULT_THRESHOLD_PCT
        }
    }
}

/// La cérémonie est-elle due pour une conversation de `message_count` messages ?
///
/// Comparaison en entiers, **multipliée plutôt que divisée** : pas d'arrondi
/// flottant sur un seuil, et pas de division entière qui ferait franchir à 42
/// ou à 44 selon le sens de la troncature.
///
/// À `pct = 85` et [`COMPACTION_THRESHOLD`] `= 50`, le franchissement est
/// exactement **43** : `43 × 100 = 4300 ≥ 85 × 50 = 4250`, là où
/// `42 × 100 = 4200` ne l'est pas.
pub fn ceremony_due(message_count: usize, pct: u32) -> bool {
    message_count * 100 >= pct as usize * COMPACTION_THRESHOLD
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    /// Un abonné qui écrit dans un tampon mémoire, pour lire le WARN plutôt
    /// que de le supposer.
    fn capturing_subscriber() -> (impl tracing::Subscriber + Send + Sync, Arc<Mutex<Vec<u8>>>) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = buf.clone();

        let layer = fmt::layer()
            .with_writer(move || WriterHandle(writer.clone()))
            .with_ansi(false)
            .with_target(true);

        (tracing_subscriber::registry().with(layer), buf)
    }

    struct WriterHandle(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for WriterHandle {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// V1 — sous le seuil, au seuil exact, au-dessus.
    ///
    /// Le franchissement à 43 est écrit en littéral, pas recalculé depuis les
    /// constantes : un test qui recalcule le seuil reste vert quand
    /// l'arithmétique bouge, puisqu'il bouge avec elle.
    #[test]
    fn mika1748_ceremony_due_franchit_a_quarante_trois() {
        assert!(!ceremony_due(0, DEFAULT_THRESHOLD_PCT));
        assert!(
            !ceremony_due(42, DEFAULT_THRESHOLD_PCT),
            "42 est sous 85 % de 50"
        );
        assert!(
            ceremony_due(43, DEFAULT_THRESHOLD_PCT),
            "43 est le franchissement"
        );
        assert!(ceremony_due(44, DEFAULT_THRESHOLD_PCT));
        assert!(
            ceremony_due(51, DEFAULT_THRESHOLD_PCT),
            "au-delà du seuil de compaction, a fortiori"
        );
    }

    /// V1 — un `pct` non-défaut déplace le franchissement, et c'est tout ce
    /// qu'il fait.
    #[test]
    fn mika1748_un_pct_configure_deplace_le_franchissement() {
        // 50 % de 50 → 25.
        assert!(!ceremony_due(24, 50));
        assert!(ceremony_due(25, 50));

        // 100 % de 50 → 50 : la cérémonie tombe au tour où la compaction
        // devient possible. Borne haute admise, pas recommandée.
        assert!(!ceremony_due(49, 100));
        assert!(ceremony_due(50, 100));
    }

    /// V2 — absence de réglage → le défaut.
    #[test]
    fn mika1748_sans_reglage_le_defaut_sapplique() {
        assert_eq!(resolve_threshold_pct(None), DEFAULT_THRESHOLD_PCT);
    }

    /// V2 — une valeur dans les bornes est honorée, espaces compris.
    #[test]
    fn mika1748_une_valeur_dans_les_bornes_est_honoree() {
        assert_eq!(resolve_threshold_pct(Some("60")), 60);
        assert_eq!(resolve_threshold_pct(Some(" 60 ")), 60);
        assert_eq!(resolve_threshold_pct(Some("1")), 1);
        assert_eq!(resolve_threshold_pct(Some("100")), 100);
    }

    /// V2 — toute valeur hors borne retombe sur le défaut.
    ///
    /// `0` est dans cette liste par décision : pas de sentinelle de
    /// désarmement sur cet axe à ce vol.
    ///
    /// `#[serial_test::serial]` parce que ce test atteint le callsite du WARN
    /// que le test suivant capture : la sérialisation des **voisins** est la
    /// moitié portante du dispositif (mika#2646).
    #[test]
    #[serial_test::serial]
    fn mika1748_une_valeur_hors_borne_retombe_sur_le_defaut() {
        for raw in ["0", "101", "-1", "", "quatre-vingt-cinq", "85.5", "85%"] {
            assert_eq!(
                resolve_threshold_pct(Some(raw)),
                DEFAULT_THRESHOLD_PCT,
                "« {raw} » doit retomber sur le défaut"
            );
        }
    }

    /// V2 — le refus **se dit**, et nomme la valeur reçue.
    ///
    /// Sans cette assertion, un seuil mal orthographié dans un `.env` produit
    /// un comportement par défaut indistinguable d'un réglage appliqué : *un
    /// réglage qu'on ne peut pas observer n'est pas un réglage, c'est un
    /// espoir* (mika#2293).
    #[test]
    #[serial_test::serial]
    fn mika1748_un_seuil_refuse_se_dit_et_nomme_la_valeur() {
        let (subscriber, buf) = capturing_subscriber();

        {
            let _guard = crate::test_utils::test_helpers::install_capturing_subscriber(subscriber);
            assert_eq!(resolve_threshold_pct(Some("101")), DEFAULT_THRESHOLD_PCT);
        }

        let binding = buf.lock().unwrap();
        let output = String::from_utf8_lossy(&binding);
        assert!(
            output.contains("compaction_handsoff_threshold_invalid"),
            "le WARN doit porter son nom d'événement : {output}"
        );
        assert!(
            output.contains("101"),
            "le WARN doit nommer la valeur reçue, sans quoi l'opérateur ne sait pas \
             quoi corriger : {output}"
        );
        assert!(
            output.contains(THRESHOLD_PCT_ENV),
            "le WARN doit nommer la variable, sans quoi il ne dit pas où corriger : {output}"
        );
    }

    /// Le défaut n'émet rien. Un avertissement qui crie en régime nominal est
    /// un avertissement qu'on filtre — et c'est le contrôle négatif du test
    /// ci-dessus, sans lequel il passerait sur un WARN inconditionnel.
    #[test]
    #[serial_test::serial]
    fn mika1748_le_regime_nominal_est_muet() {
        let (subscriber, buf) = capturing_subscriber();

        {
            let _guard = crate::test_utils::test_helpers::install_capturing_subscriber(subscriber);
            assert_eq!(resolve_threshold_pct(None), DEFAULT_THRESHOLD_PCT);
            assert_eq!(resolve_threshold_pct(Some("85")), 85);
        }

        let binding = buf.lock().unwrap();
        let output = String::from_utf8_lossy(&binding);
        assert!(
            !output.contains("compaction_handsoff_threshold_invalid"),
            "aucun avertissement sur un réglage valide : {output}"
        );
    }
}
