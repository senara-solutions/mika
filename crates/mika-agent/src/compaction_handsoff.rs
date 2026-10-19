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
//!
//! # Vol 2 (mika#2805) — la relecture, et pourquoi elle n'est pas indexée sur la session
//!
//! Le vol 1 s'arrête au témoignage. Le vol 2 ferme le cycle : la déposition
//! écrite avant la condensation est **servie au successeur** après elle, une
//! fois, et le successeur est libre de la contredire.
//!
//! Le déclencheur est *la condensation passée*, pas le premier tour de session
//! (D2 du plan de vol 2) :
//!
//! - **la cérémonie et le rituel sont mutuellement exclusifs par
//!   construction**, et c'est ce qui empêche l'auteur de se relire lui-même. La
//!   cérémonie rend de 43 à 50 messages ; la compaction tire au-dessus de 50 et
//!   ramène le compte à `CONTEXT_WINDOW` = 20. Entre l'écriture et la
//!   condensation, le rituel est donc *structurellement* éteint — aucun second
//!   mécanisme n'est nécessaire pour exclure l'auteur ;
//! - **un agent singleton n'a jamais de second « premier tour de session »**
//!   (`[session].singleton`, mika#1401) : la session canonique accumule tout
//!   l'historique et son `ended_at` reste `NULL`. Un déclencheur indexé sur la
//!   session ne tirerait jamais pour la population qui accumule le plus ;
//! - **servir la déposition pendant que son sujet est encore dans la fenêtre,
//!   c'est servir une copie redondante.**
//!
//! Aucun point d'injection « premier boot » n'est donc écrit : un bloc servi
//! exactement une fois est servi au **premier tour éligible**, et le premier
//! tour éligible *est* le premier boot. Coût nommé : une session qui s'arrête
//! entre 43 et 51 messages sans jamais compacter ne sert jamais sa déposition —
//! rien n'est perdu, son sujet est intact.
//!
//! La marque de relecture est [`crate::read_receipt`], posée par [`resolve`] et
//! par lui seul : « résoudre » et « consommer » sont une seule opération, et lui
//! donner deux implémentations serait lui donner deux réponses. Réarmement
//! opérateur : `rm {agent_home}/handsoffs/.<nom>.read`.

use std::path::Path;

use tracing::{info, warn};

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

/// Le répertoire des dépositions de compaction, relatif à l'agent home.
///
/// **Au pluriel, et l'écart d'une lettre est un invariant** (D5 du vol 2). La
/// compétence bundled `dev-handsoff` écrit `handsoff/<TODAY>[-<slug>].md` — au
/// **singulier** : deux familles d'artefacts, deux répertoires. Un futur éditeur
/// corrigera la « coquille » ; c'est pourquoi le filtre porte aussi sur le
/// suffixe (voir [`SUFFIX`]), et non sur le répertoire seul.
pub const DIR: &str = "handsoffs";

/// Le suffixe qui fait d'un fichier une déposition de compaction.
///
/// Le nom complet est `handsoffs/<YYYY-MM-DD-HHMM>-compaction-handsoff.md`,
/// établi par le bloc de cérémonie du vol 1.
///
/// Conséquence gratuite de ce filtre : un fichier de réponse
/// (`…-compaction-handsoff-response.md`) ne finit pas par ce suffixe, donc n'est
/// pas une déposition, donc **aucune exclusion explicite n'est nécessaire** —
/// là où mika#2807 avait besoin d'un `RESPONSE_INFIX` parce que ses lettres et
/// ses réponses partagent leur suffixe. Ici la forme du nom paie le refus.
pub const SUFFIX: &str = "-compaction-handsoff.md";

/// Une déposition non relue, et le dossier où le successeur peut répondre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingHandsoff {
    /// `handsoffs/<YYYY-MM-DD-HHMM>-compaction-handsoff.md`, relatif à l'agent
    /// home.
    pub path: String,
    /// `handsoffs/<même stem>-response.md` — le dépôt au dossier, si l'instance
    /// choisit d'en faire un (D6 : **rien ne le réclame**).
    ///
    /// Dérivé du nom de la déposition, jamais de l'horloge du tour, qui
    /// donnerait un nom sans lien avec son objet.
    pub response_path: String,
}

/// Ce que le plafond de contexte porte à ce tour, et qui est un **ou exclusif**
/// (D3 du vol 2).
///
/// Les deux bras sont mutuellement exclusifs *par construction* et non par
/// arbitrage : le bras `Handsoff` ne se calcule que lorsque `imminent` vaut
/// `None`, c'est-à-dire lorsque la condensation est passée (voir [`resolve`] et
/// la section « Vol 2 » du doc-module).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ceremony {
    /// La fenêtre approche sa condensation — témoigne (vol 1, mika#1748).
    Imminent { message_count: usize },
    /// La condensation est passée et une déposition non relue attend —
    /// éprouve-la.
    Handsoff(PendingHandsoff),
}

/// Le chemin de réponse d'une déposition, dérivé de son propre nom.
///
/// Opère sur [`str::strip_suffix`], jamais sur des octets :
/// `scripts/check-byte-slices.sh` refuse le découpage d'octets sur `&str`, et un
/// chemin de déposition est composé depuis un nom de fichier sur disque.
pub fn response_path(handsoff_rel: &str) -> String {
    match handsoff_rel.strip_suffix(".md") {
        Some(stem) => format!("{stem}-response.md"),
        None => format!("{handsoff_rel}-response.md"),
    }
}

/// Ce nom est-il celui d'une déposition de compaction ?
///
/// L'inégalité stricte sur la longueur écarte un fichier nommé *exactement*
/// comme le suffixe : il n'aurait pas d'instant, donc pas de place dans le tri.
fn is_handsoff(name: &str) -> bool {
    name.len() > SUFFIX.len() && name.ends_with(SUFFIX)
}

/// Les noms de dépositions présents, du plus récent au plus ancien.
///
/// Un `YYYY-MM-DD-HHMM` à largeur fixe trie chronologiquement : l'ordre
/// lexicographique décroissant **est** l'ordre chronologique inverse, donc
/// aucune `mtime` n'est lue et aucune date n'est parsée (D7). mika#2807 devait
/// lire la `mtime` de son signal parce que son nom ne porte pas d'instant ; ici
/// le nom le porte.
///
/// Rend l'erreur plutôt que de la traiter : l'appelant distingue l'absence
/// nominale du répertoire (le cas de presque tous les agents de la flotte) d'un
/// répertoire réellement illisible, qui se dit.
async fn scan(agent_home: &Path) -> std::io::Result<Vec<String>> {
    let mut entries = tokio::fs::read_dir(agent_home.join(DIR)).await?;
    let mut names: Vec<String> = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_handsoff(&name) {
            names.push(name);
        }
    }
    names.sort_unstable_by(|a, b| b.cmp(a));
    Ok(names)
}

/// La déposition la plus récente que cet agent n'a pas encore relue.
///
/// `None` est le régime nominal. **Ne rend jamais d'erreur** — un `handsoffs/`
/// illisible rend `None` sous WARN `compaction_handsoff_scan_failed` : le rituel
/// est sauté, jamais le tour, même régime que le `compaction_handsoff_count_failed`
/// du vol 1.
///
/// Ne pose **aucun** reçu : la consommation appartient à [`resolve`].
///
/// Dégradation nommée : N dépositions non relues donnent N tours éligibles, la
/// plus récente d'abord, une par tour. Borné, visible, et sa seule cause
/// réaliste est un reçu retiré à la main — c'est-à-dire le geste de réarmement
/// du doc-module, qui demande précisément ça.
pub async fn unread(agent_home: &Path, agent: &str) -> Option<PendingHandsoff> {
    let names = match scan(agent_home).await {
        Ok(names) => names,
        Err(e) => {
            // `NotFound` est le cas de presque tous les agents de la flotte :
            // un `ENOENT` par tour n'est pas un incident et ne se dit pas.
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    target: "mika::otel",
                    event = "compaction_handsoff_scan_failed",
                    agent = %agent,
                    error = %e,
                    "répertoire de dépositions illisible — le rituel est sauté, pas le tour"
                );
            }
            return None;
        }
    };

    for name in names {
        let path = format!("{DIR}/{name}");
        if crate::read_receipt::is_read(agent_home, &path).await {
            continue;
        }
        return Some(PendingHandsoff {
            response_path: response_path(&path),
            path,
        });
    }

    None
}

/// Que porte le plafond de contexte à ce tour — et consomme-le s'il y a lieu.
///
/// Site **unique** d'appel, depuis le corps conversationnel (D8). Deux sites qui
/// posent le reçu seraient deux endroits où l'usage unique peut être mangé.
///
/// - `imminent = Some(n)` ⇒ [`Ceremony::Imminent`], et **aucun reçu n'est
///   touché** : le tour qui témoigne ne se relit pas lui-même ;
/// - sinon, la déposition non relue la plus récente est servie, son reçu est
///   posé, la sonde est émise ;
/// - sinon `None`, et le régime nominal est **muet** — discipline de contrôle
///   négatif du vol 1. L'absence de `compaction_handsoff_served` après un
///   `make deploy` signifie un binaire plus ancien (classe mika#2340), jamais un
///   prédicat muet.
///
/// `agent` ne sert qu'aux sondes : sans lui, un WARN de flotte ne dit pas de
/// quel agent il parle, et le `agent_home` d'un `TempDir` de test ne le dirait
/// pas davantage.
pub async fn resolve(imminent: Option<usize>, agent_home: &Path, agent: &str) -> Option<Ceremony> {
    if let Some(message_count) = imminent {
        return Some(Ceremony::Imminent { message_count });
    }

    let handsoff = unread(agent_home, agent).await?;
    crate::read_receipt::mark_read(agent_home, &handsoff.path).await;
    info!(
        target: "mika::otel",
        event = "compaction_handsoff_served",
        agent = %agent,
        handsoff = %handsoff.path,
        response = %handsoff.response_path,
        "déposition de compaction servie au successeur — reçu posé"
    );
    Some(Ceremony::Handsoff(handsoff))
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

    // ----- Vol 2 (mika#2805) : la relecture -----

    /// Pose une déposition dans `handsoffs/` et rend son chemin relatif.
    async fn poser(home: &Path, name: &str) -> String {
        let dir = home.join(DIR);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join(name), "déposition\n")
            .await
            .unwrap();
        format!("{DIR}/{name}")
    }

    /// Combien d'entrées dans `handsoffs/` — un reçu posé en est une.
    async fn entrees(home: &Path) -> usize {
        let mut entries = tokio::fs::read_dir(home.join(DIR)).await.unwrap();
        let mut n = 0;
        while entries.next_entry().await.unwrap().is_some() {
            n += 1;
        }
        n
    }

    /// V1 — le chemin de réponse est dérivé du nom de la déposition, jamais de
    /// l'horloge du tour.
    #[test]
    fn mika2805_v1_le_chemin_de_reponse_derive_du_nom() {
        assert_eq!(
            response_path("handsoffs/2026-10-19-1240-compaction-handsoff.md"),
            "handsoffs/2026-10-19-1240-compaction-handsoff-response.md"
        );
        assert_eq!(
            response_path("handsoffs/sans-extension"),
            "handsoffs/sans-extension-response.md",
            "sans `.md`, le suffixe s'ajoute — la dérivation ne s'abandonne pas"
        );
    }

    /// V2 — le filtre de suffixe, épinglé dans les **deux** sens (D5).
    #[test]
    fn mika2805_v2_le_filtre_de_suffixe_epingle_les_deux_sens() {
        assert!(is_handsoff("2026-10-19-1240-compaction-handsoff.md"));

        // L'artefact de fin de run de `dev-handsoff` — répertoire `handsoff/`,
        // au singulier, nom sans suffixe. Fusionner les deux répertoires ne le
        // ferait toujours pas servir comme déposition de compaction.
        assert!(!is_handsoff("2026-10-19.md"));
        assert!(!is_handsoff("2026-10-19-mon-slug.md"));

        // Une réponse n'est pas une déposition, et c'est gratuit.
        assert!(!is_handsoff(
            "2026-10-19-1240-compaction-handsoff-response.md"
        ));

        // Le suffixe nu ne porte pas d'instant, donc pas de place dans le tri.
        assert!(!is_handsoff(SUFFIX));
    }

    /// V3 — la plus récente non relue, du plus récent au plus ancien, sans
    /// jamais lire une `mtime`.
    #[tokio::test]
    async fn mika2805_v3_la_plus_recente_non_relue_est_servie() {
        let home = tempfile::TempDir::new().unwrap();
        let h = home.path();
        let vieille = poser(h, "2026-10-17-0900-compaction-handsoff.md").await;
        let moyenne = poser(h, "2026-10-18-1500-compaction-handsoff.md").await;
        let recente = poser(h, "2026-10-19-1240-compaction-handsoff.md").await;

        assert_eq!(unread(h, "mika-dev").await.unwrap().path, recente);

        crate::read_receipt::mark_read(h, &recente).await;
        assert_eq!(
            unread(h, "mika-dev").await.unwrap().path,
            moyenne,
            "la plus récente est relue — la suivante prend son tour"
        );

        crate::read_receipt::mark_read(h, &moyenne).await;
        crate::read_receipt::mark_read(h, &vieille).await;
        assert!(
            unread(h, "mika-dev").await.is_none(),
            "toutes relues ⇒ plus rien à servir"
        );
    }

    /// V3 — `resolve` consomme, et le bras `Imminent` ne touche aucun reçu.
    ///
    /// C'est le gate D2 au niveau unitaire : le tour qui témoigne ne peut pas se
    /// faire servir sa propre déposition.
    #[tokio::test]
    async fn mika2805_v3_resolve_consomme_et_imminent_ne_consomme_rien() {
        let home = tempfile::TempDir::new().unwrap();
        let h = home.path();
        let depot = poser(h, "2026-10-19-1240-compaction-handsoff.md").await;

        assert_eq!(
            resolve(Some(43), h, "mika-dev").await,
            Some(Ceremony::Imminent { message_count: 43 })
        );
        assert!(
            !crate::read_receipt::is_read(h, &depot).await,
            "l'auteur ne se relit pas lui-même : aucun reçu sur le bras Imminent"
        );

        let Some(Ceremony::Handsoff(servie)) = resolve(None, h, "mika-dev").await else {
            panic!("la condensation passée, la déposition non relue est servie");
        };
        assert_eq!(servie.path, depot);
        assert_eq!(
            servie.response_path,
            "handsoffs/2026-10-19-1240-compaction-handsoff-response.md"
        );
        assert!(
            crate::read_receipt::is_read(h, &depot).await,
            "servie ⇒ reçu posé, dans la même opération"
        );

        // AC2 — un handsoff déjà servi ne se re-sert pas au tour suivant.
        assert_eq!(resolve(None, h, "mika-dev").await, None);
    }

    /// V4 — **contrôle négatif** : rien à servir ⇒ `None` et **zéro reçu posé**.
    #[tokio::test]
    async fn mika2805_v4_controle_negatif_aucun_recu_sans_deposition() {
        let home = tempfile::TempDir::new().unwrap();
        let h = home.path();

        // `handsoffs/` absent — le cas de presque tous les agents de la flotte.
        assert!(resolve(None, h, "mika-dev").await.is_none());
        assert!(
            !h.join(DIR).exists(),
            "aucun reçu posé, donc aucun répertoire créé par le rituel"
        );

        // Présent, et peuplé de ce que le filtre refuse.
        poser(h, "2026-10-19.md").await;
        poser(h, "2026-10-19-1240-compaction-handsoff-response.md").await;
        assert!(resolve(None, h, "mika-dev").await.is_none());
        assert_eq!(
            entrees(h).await,
            2,
            "ni reçu ni déposition inventée sur un répertoire sans déposition"
        );
    }

    /// V4 — un `handsoffs/` illisible **se dit**, et nomme l'agent.
    ///
    /// La panne est fabriquée en posant un **fichier** à la place du
    /// répertoire : `read_dir` rend alors `ENOTDIR`, jamais `NotFound`. Un
    /// `chmod 000` serait sans effet sous un uid 0, que la sandbox de dispatch
    /// peut porter ; celui-ci tient pour tout utilisateur.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2805_v4_un_repertoire_illisible_se_dit() {
        let (subscriber, buf) = capturing_subscriber();
        let home = tempfile::TempDir::new().unwrap();
        tokio::fs::write(home.path().join(DIR), "pas un répertoire")
            .await
            .unwrap();

        {
            let _guard = crate::test_utils::test_helpers::install_capturing_subscriber(subscriber);
            assert!(
                resolve(None, home.path(), "mika-dev").await.is_none(),
                "le rituel est sauté, jamais le tour"
            );
        }

        let binding = buf.lock().unwrap();
        let output = String::from_utf8_lossy(&binding);
        assert!(
            output.contains("compaction_handsoff_scan_failed"),
            "le WARN doit porter son nom d'événement : {output}"
        );
        assert!(
            output.contains("mika-dev"),
            "le WARN doit nommer l'agent, sans quoi un log de flotte ne dit pas \
             de qui il parle : {output}"
        );
    }

    /// Le régime nominal est **muet** — contrôle négatif des deux sondes du
    /// vol 2, sans lequel les assertions ci-dessus passeraient sur un log
    /// inconditionnel.
    #[tokio::test]
    #[serial_test::serial]
    async fn mika2805_v4_le_regime_nominal_du_rituel_est_muet() {
        let (subscriber, buf) = capturing_subscriber();
        let home = tempfile::TempDir::new().unwrap();

        {
            let _guard = crate::test_utils::test_helpers::install_capturing_subscriber(subscriber);
            assert!(resolve(None, home.path(), "mika-dev").await.is_none());
            assert!(resolve(Some(43), home.path(), "mika-dev").await.is_some());
        }

        let binding = buf.lock().unwrap();
        let output = String::from_utf8_lossy(&binding);
        assert!(
            !output.contains("compaction_handsoff_scan_failed"),
            "l'absence nominale de `handsoffs/` n'est pas un incident : {output}"
        );
        assert!(
            !output.contains("compaction_handsoff_served"),
            "rien n'a été servi, donc rien ne le dit : {output}"
        );
    }
}
