//! L'interrupteur d'arrêt à chaud des scans périodiques (mika#2329).
//!
//! # Le défaut que ça ferme
//!
//! `MIKA_DEV_AUTO_PULL=0` est lu **une seule fois**, au démarrage, dans
//! `server::init_agent` : la branche knob-off annule la row récurrente
//! `auto_pull_groomed`, la branche knob-on la crée. Rien ne relit la variable
//! ensuite. Couper le feeder en pleine incidence exigeait donc un redémarrage de
//! mika-spirit — précisément ce qu'on veut le moins faire pendant un P0 (dispatch
//! en vol, worktrees ouverts, session pilote active).
//!
//! # Pourquoi un fichier, et pas la variable d'environnement relue à chaud
//!
//! **La piste évidente ne marche pas, et c'est la rectification centrale du
//! ticket.** `mika-spirit` appelle `mika_common::dotenv::load_dotenv` **une fois**
//! au démarrage ; `dotenvy` lit le fichier et pose les variables dans
//! l'environnement du process. Rien ne surveille le fichier ensuite — aucun
//! watcher, aucun second appel dans tout le crate agent. Or l'environnement d'un
//! process Linux vivant n'est pas mutable de l'extérieur : **éditer `~/.mika/.env`
//! ne change rien à ce que `std::env::var` renverra**, même appelé à chaque tick.
//! Relire l'env au tick aurait déplacé le défaut d'un cran et l'aurait rendu plus
//! difficile à voir, puisque le code aurait alors eu l'air de relire.
//!
//! **Et la maison fige délibérément ses variables.** `MIKA_AGENT_TIER`,
//! `MIKA_DEPLOYMENT`, `MIKA_TELEGRAM_HTML_RENDER` et `MIKA_LOG_LLM_BODIES` sont
//! documentées « not hot-swappable », lues une fois par process et mises en cache ;
//! `TaskDispatcher::tier` porte la justification en commentaire. Rendre **une**
//! variable `MIKA_*` relue à chaud en ferait une exception invisible : deux
//! variables d'apparence strictement identique, l'une relue, l'autre pas, et rien
//! dans le nom, la forme ou le lieu de déclaration ne dirait laquelle. Un
//! opérateur qui apprend sur `MIKA_DEV_AUTO_PULL` que « ça se relit » le
//! transportera sur `MIKA_DEV_WIP_RESCUE`, qui a la forme jumelle et ne se relit
//! pas.
//!
//! D'où : l'interrupteur à chaud est un **objet distinct**, dont la nature hot se
//! lit sur l'objet. Et le geste — `touch` / `rm` — fonctionne depuis n'importe
//! quel shell, sans binaire `mika`, sans DB joignable, sans redémarrage : c'est le
//! plus court disponible pendant un incident.
//!
//! # L'existence vaut STOP ; le contenu n'est jamais lu
//!
//! Un fichier vide est un STOP valide. Lire le contenu créerait une seconde
//! question — « que vaut un contenu invalide ? » — dont la réponse, fail-open ou
//! fail-closed, serait un piège de plus sur un interrupteur d'arrêt.
//!
//! # Fail-open, nommé, et pourquoi il est acceptable *ici* seulement
//!
//! [`Path::exists`] renvoie `false` sur **toute** erreur d'accès (permissions,
//! I/O). Un fichier illisible fait donc tourner la boucle. L'asymétrie penche à
//! première vue du mauvais côté — un faux « pas de STOP » fait tourner le feeder
//! pendant un incident, ce qui est le défaut qu'on ferme. Deux choses le rendent
//! acceptable, et la seconde est porteuse :
//!
//! 1. Le fail-closed n'est pas implémentable proprement : `exists()` ne distingue
//!    pas « absent » de « illisible », et passer par `symlink_metadata()` pour
//!    trancher ferait du cas « répertoire `state/` inexistant » — le cas
//!    **nominal** sur une installation qui n'a jamais posé de STOP — une erreur,
//!    donc un STOP permanent. Le remède serait pire.
//! 2. **L'opérateur constate l'effet au log en ≤ 10 minutes** (`AUTO_PULL_CRON`
//!    vaut `0 */10 * * * *`, et chaque tick court-circuité écrit une ligne INFO).
//!    La boucle de rétroaction est courte et l'erreur auto-détectable : poser le
//!    fichier et ne pas voir la ligne est un signal immédiat et sans ambiguïté.
//!
//! Le point 2 est ce qui autorise le point 1. **Si un jour le lecteur devient
//! faillible d'une manière que l'opérateur ne peut pas constater (DB, réseau),
//! cet arbitrage est à refaire, pas à transporter.**
//!
//! # Portée : `auto_pull`, puis `worktree_reap` (mika#2420)
//!
//! mika#2329 a livré le mécanisme paramétré par nom de scan tout en refusant de
//! l'étendre, faute de besoin mesuré : *« arrêter la revue QA n'est pas la même
//! décision qu'arrêter le feeder »*. **Une opération destructive est précisément
//! ce besoin.** Pendant un incident, on veut arrêter un reaper qui supprime sans
//! redémarrer mika-spirit — le redémarrage étant ce qu'on veut le moins faire
//! avec des dispatches en vol. D'où le second scan, [`WORKTREE_REAP_SCAN`].
//!
//! `MIKA_DEV_WIP_RESCUE` et `MIKA_QA_REVIEW_RECONCILE` ont le même défaut
//! boot-time et **n'ont toujours pas d'interrupteur** : leur arrêt ne détruit
//! rien, et livrer des gestes que personne n'a demandés reste du YAGNI.
//!
//! # [`AUTO_PULL_SCAN`] est le frein de dispatch de la boucle (mika#2498)
//!
//! Son nom dit un scan ; sa **portée** est plus large, et la confondre avec son
//! nom a coûté un incident. Le 2026-09-23, la sentinelle posée à 05:38 a bien
//! court-circuité le tick du feeder — et un implement est parti quand même à
//! 06:11:39Z, parce qu'un groom convergé enchaîne sur son implémentation par
//! l'**auto-fire moteur** ([`crate::task_engine`], mika#1614), qui ne passe par
//! aucun tick. L'opérateur croyait avoir arrêté la boucle ; il n'avait arrêté
//! qu'une de ses deux portes.
//!
//! **Ce n'est pas une décision distincte, donc pas un second fichier.** Le
//! critère de mika#2420 est *« une décision distincte mérite un fichier
//! distinct »* — or les deux routes ont la **même sortie** (un dispatch
//! dev-pilot neuf) atteinte par deux chemins : le feeder promeut `ready` puis
//! dispatche in-process (mika#2470), l'auto-fire dispatche directement. Un
//! opérateur qui arrête l'une et pas l'autre n'a rien arrêté — c'est exactement
//! l'incident. La sentinelle est donc élargie **dans son sens**, pas dupliquée.
//! Scinder aurait garanti que le geste de mémoire musculaire — poser le fichier
//! de l'incident — rende le comportement d'aujourd'hui en ayant l'air d'arrêter.
//!
//! **Le critère pour un futur consommateur**, et il n'est pas « suis-je un
//! scan ? » : *est-ce que je démarre du travail pilote **neuf** ?* Si oui, lire
//! cette sentinelle — quelle que soit la porte d'entrée. Sinon, il faut un nom
//! de scan à soi (le critère de mika#2420). C'est ce qui laisse dehors, à
//! dessein, `verdict_handler` (`block[ac]` / `block[ci]` réparent une PR
//! ouverte : une **continuation**, pas du travail neuf) et un `ready` posé à la
//! main pendant un STOP (savoir si un fichier prime sur le geste que
//! l'opérateur vient de poser est une décision produit, pas substrat).
//!
//! **Le refus est convergent, jamais terminal**, et c'est ce qui rend
//! l'élargissement acceptable : il n'annule pas le groom, ne perd pas le plan
//! (committé et poussé), n'écrit aucune row. À la levée, le réconciliateur
//! stuck-ready re-drive le ticket — et pendant le STOP aucun tick ne tourne,
//! donc **aucun point du budget de re-drive n'est consommé** (mika#2020).
//!
//! # Un seul lecteur
//!
//! Le prédicat vit **ici** et nulle part ailleurs. Précédent explicite :
//! [`crate::grooming_marker`] (mika#2158), né du constat qu'une copie de prédicat
//! avait dérivé pendant des mois en répondant différemment à la même question sans
//! que rien ne casse. [`tests::mika2329_le_chemin_du_fichier_sentinelle_a_un_seul_lecteur`]
//! refuse une seconde occurrence du littéral du chemin sous `crates/mika-agent/src/`.
//! Un test comportemental ne peut pas voir cette classe : une copie ne rendrait
//! aucune décision fausse le jour où elle est écrite.

use std::path::{Path, PathBuf};

/// Répertoire d'état, sous le home **global**.
///
/// Le précédent maison est global, pas per-agent : `dispatch-lib.sh` écrit
/// `$HOME/.mika/state/pilot-gitconfig` et `${MIKA_HOME:-$HOME/.mika}/state/pr-origin-epoch`.
/// Un STOP global doit se poser à un endroit unique, pas une fois par agent.
const STOP_DIR: &str = "state";

/// Le nom de scan d'`auto_pull`, tel qu'il compose le nom de fichier.
///
/// Passé par les appelants plutôt qu'écrit en dur chez eux : c'est ce qui garde
/// le littéral complet du chemin dans ce seul fichier.
pub const AUTO_PULL_SCAN: &str = "auto-pull";

/// Le nom de scan du reaper de worktrees (mika#2420).
///
/// Second usage du mécanisme, et le premier sur un scan **destructif** : c'est
/// le besoin que mika#2329 avait nommé sans le servir. Le fichier est
/// `~/.mika/state/worktree-reap-stop`, et sa sémantique est identique — son
/// existence vaut STOP, son contenu n'est jamais lu.
pub const WORKTREE_REAP_SCAN: &str = "worktree-reap";

/// La variable d'environnement boot-time dont ce module ferme le piège.
const AUTO_PULL_ENV_KNOB: &str = "MIKA_DEV_AUTO_PULL";

/// La valeur de `MIKA_DEV_AUTO_PULL` qui coupe le scan au démarrage.
///
/// Identique au prédicat de `server::init_agent` (`v == "0"`), volontairement :
/// la garde doit avertir sur exactement ce que la branche boot-time aurait lu.
const AUTO_PULL_ENV_KNOB_OFF: &str = "0";

/// `{global_home}/state/{scan}-stop`.
pub fn stop_file_path(global_home: &Path, scan: &str) -> PathBuf {
    global_home.join(STOP_DIR).join(format!("{scan}-stop"))
}

/// Le scan est-il arrêté ?
///
/// **Fail-open** : voir la section du même nom dans la doc de module. Un fichier
/// présent mais illisible rend `false`, donc laisse la boucle tourner, et
/// l'opérateur le constate en ≤ 10 minutes à l'absence de la ligne INFO.
pub fn is_stopped(global_home: &Path, scan: &str) -> bool {
    stop_file_path(global_home, scan).exists()
}

/// Un `MIKA_DEV_AUTO_PULL=0` posé sur disque après le démarrage du process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleKnob {
    /// Le fichier `.env` qui porte la clé — ce que l'opérateur doit éditer.
    pub env_path: PathBuf,
    /// La valeur lue, telle qu'écrite.
    pub value: String,
}

/// Détecte le geste que l'opérateur de mika#2313 a posé : écrire
/// `MIKA_DEV_AUTO_PULL=0` dans un `.env` pendant un STOP P0, en croyant couper la
/// boucle.
///
/// Lit les **fichiers** `.env` (global puis per-agent), jamais l'environnement du
/// process — c'est toute la différence, et c'est pour ça que la fonction existe.
/// Per-agent prioritaire sur le global, dans l'ordre où `Settings::load_for_agent`
/// les compose (mika#2218 : la source per-agent est ajoutée en dernier, elle gagne).
///
/// # Le prédicat est juste par construction, et c'est ce qui le rend sûr
///
/// Si le process avait démarré **avec** le knob, la row récurrente serait annulée
/// et *aucun tick ne tournerait* — le code de la garde ne serait jamais atteint.
/// La garde ne peut donc émettre que dans la population « fichier édité après le
/// boot », qui est exactement celle du ticket. Aucun faux positif n'est
/// atteignable par cette voie.
///
/// Sans cet avertissement, le défaut resterait ouvert dans sa forme la plus
/// dangereuse : **un STOP silencieusement inopérant se lit exactement comme un
/// STOP qui marche.** C'est le symétrique inverse de mika#2205, où un scan
/// silencieusement inactif se lisait comme un scan oisif.
pub fn stale_env_knob(global_home: &Path, agent_home: &Path) -> Option<StaleKnob> {
    // Per-agent d'abord : il gagne dans la cascade, donc c'est lui que
    // l'opérateur doit corriger en premier si les deux portent la clé.
    for home in [agent_home, global_home] {
        let parsed = mika_common::dotenv::parse_dotenv(home);
        if let Some(value) = parsed.get(AUTO_PULL_ENV_KNOB)
            && value == AUTO_PULL_ENV_KNOB_OFF
        {
            return Some(StaleKnob {
                env_path: home.join(".env"),
                value: value.clone(),
            });
        }
    }
    None
}

/// Le nom de la clé, pour que l'appelant la nomme dans son WARN sans la réécrire.
pub fn env_knob_name() -> &'static str {
    AUTO_PULL_ENV_KNOB
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(global_home: &Path, scan: &str, contents: &str) {
        let path = stop_file_path(global_home, scan);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }

    /// T1 — le fichier présent vaut STOP, vide comme non vide (D1).
    #[test]
    fn mika2329_un_fichier_present_meme_vide_vaut_stop() {
        let tmp = tempfile::tempdir().unwrap();

        arm(tmp.path(), AUTO_PULL_SCAN, "");
        assert!(
            is_stopped(tmp.path(), AUTO_PULL_SCAN),
            "un fichier vide est un STOP valide — le contenu n'est jamais lu"
        );

        arm(
            tmp.path(),
            AUTO_PULL_SCAN,
            "peu importe ce qu'il y a dedans",
        );
        assert!(
            is_stopped(tmp.path(), AUTO_PULL_SCAN),
            "le contenu ne doit changer aucune décision"
        );
    }

    /// T2 — l'absence du fichier laisse le chemin nominal intact.
    #[test]
    fn mika2329_un_fichier_absent_ne_coupe_rien() {
        let tmp = tempfile::tempdir().unwrap();
        // Ni le répertoire `state/` ni le fichier n'existent : c'est le cas
        // nominal sur une installation qui n'a jamais posé de STOP, et il ne doit
        // surtout pas être une erreur (voir la section fail-open).
        assert!(!is_stopped(tmp.path(), AUTO_PULL_SCAN));

        // `state/` existe, le fichier non.
        std::fs::create_dir_all(tmp.path().join(STOP_DIR)).unwrap();
        assert!(!is_stopped(tmp.path(), AUTO_PULL_SCAN));
    }

    /// Le chemin est bien `{global_home}/state/{scan}-stop`, et il est paramétré
    /// par le scan (D7 : l'extension aux deux scans jumeaux est une ligne).
    #[test]
    fn mika2329_le_chemin_est_parametre_par_le_scan() {
        let home = Path::new("/home/x/.mika");
        assert_eq!(
            stop_file_path(home, AUTO_PULL_SCAN),
            PathBuf::from("/home/x/.mika/state/auto-pull-stop")
        );
        assert_eq!(
            stop_file_path(home, WORKTREE_REAP_SCAN),
            PathBuf::from("/home/x/.mika/state/worktree-reap-stop")
        );
        assert_eq!(
            stop_file_path(home, "wip-rescue"),
            PathBuf::from("/home/x/.mika/state/wip-rescue-stop")
        );

        // Et les scans ne se coupent pas l'un l'autre : arrêter le reaper ne
        // doit pas arrêter le feeder, et réciproquement (mika#2420 — les deux
        // décisions sont distinctes, c'est tout l'objet du paramétrage).
        let tmp = tempfile::tempdir().unwrap();
        arm(tmp.path(), WORKTREE_REAP_SCAN, "");
        assert!(!is_stopped(tmp.path(), AUTO_PULL_SCAN));
        assert!(is_stopped(tmp.path(), WORKTREE_REAP_SCAN));
    }

    /// T4 — la garde émet quand le `.env` disque contredit le process, sur les
    /// deux homes.
    #[test]
    fn mika2329_la_garde_voit_le_knob_pose_sur_disque() {
        let global = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();

        std::fs::write(global.path().join(".env"), "MIKA_DEV_AUTO_PULL=0\n").unwrap();
        let hit = stale_env_knob(global.path(), agent.path())
            .expect("un knob posé dans le .env global doit être vu");
        assert_eq!(hit.env_path, global.path().join(".env"));
        assert_eq!(hit.value, "0");

        let global2 = tempfile::tempdir().unwrap();
        std::fs::write(agent.path().join(".env"), "MIKA_DEV_AUTO_PULL=0\n").unwrap();
        let hit = stale_env_knob(global2.path(), agent.path())
            .expect("un knob posé dans le .env per-agent doit être vu");
        assert_eq!(hit.env_path, agent.path().join(".env"));
    }

    /// Quand les deux fichiers portent la clé, c'est le per-agent qui est nommé —
    /// il gagne dans la cascade (mika#2218), donc c'est lui qu'il faut corriger.
    #[test]
    fn mika2329_le_per_agent_est_nomme_en_premier() {
        let global = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        std::fs::write(global.path().join(".env"), "MIKA_DEV_AUTO_PULL=0\n").unwrap();
        std::fs::write(agent.path().join(".env"), "MIKA_DEV_AUTO_PULL=0\n").unwrap();

        let hit = stale_env_knob(global.path(), agent.path()).unwrap();
        assert_eq!(
            hit.env_path,
            agent.path().join(".env"),
            "le .env per-agent gagne dans la cascade, c'est donc lui que le WARN doit nommer"
        );
    }

    /// T5 — elle se tait dans tous les autres cas. Un avertissement qui crie en
    /// régime nominal est un avertissement qu'on filtre.
    #[test]
    fn mika2329_la_garde_se_tait_en_regime_nominal() {
        let global = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();

        // Aucun .env.
        assert_eq!(stale_env_knob(global.path(), agent.path()), None);

        // Un .env sans la clé.
        std::fs::write(global.path().join(".env"), "MIKA_ANTHROPIC_API_KEY=x\n").unwrap();
        assert_eq!(stale_env_knob(global.path(), agent.path()), None);

        // La clé, mais à une valeur qui n'est pas celle qui coupe.
        std::fs::write(global.path().join(".env"), "MIKA_DEV_AUTO_PULL=1\n").unwrap();
        assert_eq!(stale_env_knob(global.path(), agent.path()), None);
    }

    /// La garde lit le **fichier**, jamais l'environnement du process. Sans cette
    /// propriété elle réintroduirait E2 à l'envers : elle crierait sur un process
    /// démarré avec le knob (où aucun tick ne tourne) et se tairait sur le geste
    /// qu'elle existe pour voir.
    #[test]
    #[serial_test::serial]
    fn mika2329_la_garde_ne_lit_pas_lenvironnement_du_process() {
        let global = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();

        unsafe { std::env::set_var(AUTO_PULL_ENV_KNOB, AUTO_PULL_ENV_KNOB_OFF) };
        let verdict = stale_env_knob(global.path(), agent.path());
        unsafe { std::env::remove_var(AUTO_PULL_ENV_KNOB) };

        assert_eq!(
            verdict, None,
            "la variable posée dans l'environnement du process n'est pas le geste \
             que cette garde surveille — seul le fichier compte"
        );
    }

    /// T7 — garde structurelle : un seul lecteur du chemin.
    ///
    /// Même forme que `grooming_marker::tests::no_grooming_regex_outside_this_module`,
    /// et pour la même raison : une copie du chemin ne rendrait aucune décision
    /// fausse le jour où elle est écrite, donc aucun test comportemental ne peut
    /// la voir. Elle deviendrait fausse plus tard, en silence, exactement comme la
    /// regex de grooming l'a fait pendant des mois.
    ///
    /// `env!("CARGO_MANIFEST_DIR")` : lecture à l'exécution depuis un chemin
    /// absolu garanti par Cargo — aucun couplage de compilation, aucune dépendance
    /// au `cwd`.
    #[test]
    fn mika2329_le_chemin_du_fichier_sentinelle_a_un_seul_lecteur() {
        // Écrits en deux morceaux pour que la garde ne se dénonce pas elle-même
        // lorsqu'un scan de source la lit. mika#2420 ajoute le second scan : la
        // garde doit couvrir **chaque** nom, sinon le nouveau naît hors
        // protection et la leçon `grooming_marker` se rejoue sur lui.
        let literals: Vec<String> = [AUTO_PULL_SCAN, WORKTREE_REAP_SCAN]
            .iter()
            .map(|scan| format!("{scan}{}", "-stop"))
            .collect();

        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_module = src_root.join("auto_pull_stop.rs");

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
                    if literals.iter().any(|lit| line.contains(lit)) {
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
            "mika#2329 — le chemin du fichier sentinelle vit hors de `auto_pull_stop.rs`. \
             Appelez `auto_pull_stop::is_stopped(global_home, AUTO_PULL_SCAN)` au lieu de \
             recomposer le chemin : une copie répondrait un jour différemment à la même \
             question, en silence.\n{}",
            offenders.join("\n")
        );
    }
}
