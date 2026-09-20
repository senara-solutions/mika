//! Un trigger enregistré a un destinataire, et une mort par trigger inconnu ne
//! gèle plus la récurrence — mika#2337.
//!
//! **Rectification du diagnostic, portée par ce fichier.** Le ticket localise la
//! racine en `dispatcher.rs` (« le bras de match `"qa_review_reconcile"` est
//! absent ») et demande de l'ajouter. Ce bras existe, et il a été introduit par
//! `1340936e` — le commit de la PR que le ticket accuse de l'avoir oublié. Le
//! fix primaire tel qu'énoncé est un no-op ; `mika2334_le_scan_est_route_dans_le_dispatcher`
//! (livré par ce même commit) est vert depuis le début et l'était pendant toute
//! la panne.
//!
//! Ce qui tenait réellement #2334 inert est ailleurs : la mort en `failed` a armé
//! la garde zombie mika#1742 pour 24 h, et son unique porte de sortie
//! (`revert_config_cancel_recurring_task`) ne cible que `status = 'cancelled'`.
//! **Tout redémarrage pendant cette fenêtre — y compris avec le binaire correct —
//! refusait de ré-enregistrer la récurrence.** Le remède naturel était
//! précisément ce que l'état neutralisait.
//!
//! Deux gardes ici, et une troisième famille dans `db::tests` (`mika2337_*`) :
//!
//! - **V3.1 — garde de classe.** Chaque trigger *enregistré* a un bras dans
//!   `dispatch_run_skill`, pour **tout** trigger et pas seulement celui du jour.
//!   Le prédicat s'ancre sur l'appelant — l'argument `action_config` des appels
//!   à `task_engine::ensure_recurring_task` — et non sur la forme textuelle
//!   `{"trigger":"X"}`, qui a trois faux membres dans l'arbre dont un
//!   (`engine.rs`, helper `#[cfg(test)]` d'`action_type = "resume_agent"`) ferait
//!   fire la garde au land.
//! - **V3.2 — sonde de tir.** La récurrence `qa_review_reconcile` est tirée par
//!   le moteur et ne finit pas `failed` sur le motif du catch-all. Son contrôle
//!   négatif — un trigger volontairement inconnu — fait le chemin inverse et
//!   atteste au passage AC3 (variante dédiée, WARN nommé, ligne `audit_events`,
//!   marqueur posé sur la ligne).
//!
//! **Portée honnête (AC9).** Ces gardes ferment la divergence **intra-binaire** —
//! celle que le ticket croit avoir observée. **Aucune n'aurait attrapé l'incident
//! du 2026-09-16**, qui est un décalage entre le code mergé et le code en
//! exécution : les deux littéraux étaient cohérents dans le source. Rendre
//! lisible au démarrage la version réellement en exécution appartient au suivi.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, RECURRING_UNKNOWN_TRIGGER_PATH};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::skills::SkillRegistry;
use mika_agent::task_engine::dispatcher::TaskDispatcher;
use mika_agent::task_engine::engine::TaskEngine;
use mika_agent::tools::default_tools;

const AGENT_ID: &str = "mika";

/// Épinglé à la compilation : la garde de classe lit aussi l'arbre au runtime
/// (pour voir un septième site d'enregistrement où qu'il soit), mais le bloc
/// `match` vient d'une source qui ne peut pas être une copie périmée.
const DISPATCHER_SRC: &str = include_str!("../../src/task_engine/dispatcher.rs");

// ───────────────────────── V3.1 — garde de classe ─────────────────────────

/// Les arguments de l'appel dont la parenthèse ouvrante est à `open`.
///
/// Un `split(',')` naïf couperait à l'intérieur de `r#"{"trigger":"x"}"#` — qui
/// ne contient pas de virgule aujourd'hui, ce qui est exactement le genre de
/// coïncidence sur laquelle une garde ne doit pas reposer.
fn call_arguments(src: &str, open: usize) -> Option<Vec<String>> {
    let bytes = src.as_bytes();
    debug_assert_eq!(bytes[open], b'(');
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut i = open;

    while i < bytes.len() {
        let c = bytes[i] as char;

        // Chaîne brute `r#..#"…"#..#` — le nombre de `#` doit se correspondre.
        if c == 'r' {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < bytes.len() && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                let closing = format!("\"{}", "#".repeat(hashes));
                let end = src[j + 1..].find(&closing)? + j + 1;
                current.push_str(&src[i..end + closing.len()]);
                i = end + closing.len();
                continue;
            }
        }

        match c {
            '"' => {
                let mut j = i + 1;
                while j < bytes.len() {
                    if bytes[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if bytes[j] == b'"' {
                        break;
                    }
                    j += 1;
                }
                current.push_str(&src[i..=j.min(bytes.len() - 1)]);
                i = j + 1;
                continue;
            }
            '/' if bytes.get(i + 1) == Some(&b'/') => {
                i = src[i..].find('\n').map(|n| i + n).unwrap_or(bytes.len());
                continue;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    if !current.trim().is_empty() {
                        args.push(current.trim().to_string());
                    }
                    return Some(args);
                }
            }
            ',' if depth == 1 => {
                args.push(current.trim().to_string());
                current.clear();
                i += 1;
                continue;
            }
            _ => {}
        }

        if depth > 0 && !(depth == 1 && i == open) {
            current.push(c);
        }
        i += c.len_utf8();
    }
    None
}

/// Le contenu d'un littéral de chaîne Rust (`"…"` ou `r#"…"#`), ou `None` si
/// l'argument n'en est pas un.
fn string_literal_content(arg: &str) -> Option<&str> {
    let arg = arg.trim();
    if let Some(rest) = arg.strip_prefix('r') {
        let hashes = rest.len() - rest.trim_start_matches('#').len();
        let open = format!("{}\"", "#".repeat(hashes));
        let close = format!("\"{}", "#".repeat(hashes));
        return rest.strip_prefix(&open)?.strip_suffix(&close);
    }
    arg.strip_prefix('"')?.strip_suffix('"')
}

/// Tout fichier `.rs` sous `crates/mika-agent/src/`.
fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("lire src/") {
        let path = entry.expect("entrée de src/").path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Les triggers `run_skill` enregistrés en production : `trigger → site`.
///
/// La population est celle des **appels** à `task_engine::ensure_recurring_task`,
/// seul enregistreur de récurrences `run_skill` (il pose `action_type::RUN_SKILL`
/// lui-même). Le scan balaie tout l'arbre plutôt que le seul `server/mod.rs`,
/// pour qu'un septième site posé ailleurs entre dans la population au lieu d'y
/// échapper.
///
/// La frontière production/test est lue par [`mika_common::source_guard`]
/// (mika#2398). Le prédicat local qu'elle remplace — `find("\n#[cfg(test)]\nmod ")`
/// — était déjà le plus étroit des treize, et sa précision sur le helper de
/// niveau module est conservée par la clause de forme d'item ; ce qu'il ne
/// voyait pas, c'est un fichier **intégralement** de test, qui ne porte aucun
/// marqueur parce que l'attribut est sur la déclaration `mod` chez le parent.
fn registered_triggers() -> BTreeMap<String, String> {
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let src_dir = scanner.src_root().to_path_buf();

    let mut found = BTreeMap::new();
    for path in scanner.files() {
        let production = scanner.production_of(&path);
        let src = production.as_str();
        let rel = path
            .strip_prefix(&src_dir)
            .unwrap_or(&path)
            .display()
            .to_string();

        let mut from = 0usize;
        while let Some(rel_idx) = src[from..].find("ensure_recurring_task(") {
            let idx = from + rel_idx;
            from = idx + "ensure_recurring_task(".len();

            // La définition elle-même n'est pas un enregistrement.
            let line_start = src[..idx].rfind('\n').map(|n| n + 1).unwrap_or(0);
            if src[line_start..idx].contains("fn ") {
                continue;
            }

            let open = idx + "ensure_recurring_task".len();
            let args = call_arguments(src, open)
                .unwrap_or_else(|| panic!("{rel}: appel à ensure_recurring_task non refermé"));
            assert_eq!(
                args.len(),
                4,
                "{rel}: ensure_recurring_task prend 4 arguments ; la garde lit le \
                 4e (`action_config`). Si la signature a changé, c'est ici qu'il \
                 faut la suivre — pas dans une exemption."
            );

            let literal = string_literal_content(&args[3]).unwrap_or_else(|| {
                panic!(
                    "{rel}: l'`action_config` de cet enregistrement n'est pas un \
                     littéral ({}), donc la garde ne peut pas lire le trigger qu'il \
                     déclare. Halte : une récurrence dont le destinataire est \
                     illisible est exactement l'incident mika#2337, et sa résolution \
                     est une décision de périmètre, pas un geste de poseur.",
                    args[3]
                )
            });

            let config: serde_json::Value = serde_json::from_str(literal)
                .unwrap_or_else(|e| panic!("{rel}: `action_config` n'est pas du JSON: {e}"));

            // `{"skill_name": …}` est routé par nom de skill, pas par trigger :
            // il n'a légitimement aucun bras dans le `match`.
            if config.get("skill_name").is_some() && config.get("trigger").is_none() {
                continue;
            }

            let trigger = config
                .get("trigger")
                .and_then(|t| t.as_str())
                .unwrap_or_else(|| {
                    panic!(
                        "{rel}: `action_config` ne déclare ni `trigger` ni \
                         `skill_name` — `dispatch_run_skill` ne saura pas le router"
                    )
                });
            found.insert(trigger.to_string(), rel.clone());
        }
    }
    found
}

/// Les bras `"X" =>` du `match trigger_name` de `dispatch_run_skill`.
fn match_arm_triggers() -> Vec<String> {
    let start = DISPATCHER_SRC
        .find("match trigger_name {")
        .expect("le match sur trigger_name doit exister dans dispatch_run_skill");
    let body_start = start + "match trigger_name {".len();
    let bytes = DISPATCHER_SRC.as_bytes();
    let mut depth = 1usize;
    let mut i = body_start;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    let body = &DISPATCHER_SRC[body_start..i];

    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix('"')?;
            let (name, tail) = rest.split_once('"')?;
            tail.trim_start()
                .starts_with("=>")
                .then(|| name.to_string())
        })
        .collect()
}

/// **AC7 — la garde de classe.** Tout trigger enregistré a son bras, et
/// l'échec nomme le trigger fautif.
///
/// Elle **subsume** `mika2334_le_scan_est_route_dans_le_dispatcher` pour la
/// moitié « routage » : cette dernière garde reste utile pour l'assertion sur
/// l'appel effectif au scan, qu'une garde générique ne peut pas voir.
#[test]
fn mika2337_tout_trigger_enregistre_a_un_bras_dans_le_dispatcher() {
    let registered = registered_triggers();
    let arms = match_arm_triggers();

    let orphans: Vec<String> = registered
        .iter()
        .filter(|(trigger, _)| !arms.contains(trigger))
        .map(|(trigger, site)| format!("  - `{trigger}` (enregistré en {site})"))
        .collect();

    assert!(
        orphans.is_empty(),
        "trigger(s) enregistré(s) sans bras correspondant dans \
         `dispatch_run_skill` — la récurrence tirera dans le catch-all, mourra \
         une fois (le ré-enfilement n'existe que dans le bras `Ok`) et se taira :\n{}\n\
         Ajoutez le bras, ou n'enregistrez pas la récurrence.",
        orphans.join("\n")
    );
}

/// Contrôle négatif de la garde ci-dessus : une garde qui ne trouverait aucun
/// site passerait toujours. La population comptée dans l'arbre à `9f28342b` est
/// de six enregistrements, tous dans `server/mod.rs`.
#[test]
fn mika2337_la_garde_de_classe_a_une_population_non_vide() {
    let registered = registered_triggers();

    assert!(
        registered.len() >= 6,
        "population attendue ≥ 6 enregistrements (heartbeat, reflection, \
         auto_pull_groomed, wip_rescue, qa_review_reconcile, curator_review) — \
         trouvé {} : {:?}. Une garde dont la population s'est vidée est verte \
         pour la mauvaise raison.",
        registered.len(),
        registered.keys().collect::<Vec<_>>()
    );
    assert!(
        registered.contains_key("qa_review_reconcile"),
        "le trigger de l'incident doit faire partie de la population gardée"
    );
    assert!(
        !match_arm_triggers().is_empty(),
        "les bras du match doivent être lisibles — sinon la garde compare à vide"
    );
}

/// Les trois littéraux `{"trigger":…}` qui ne sont **pas** des enregistrements
/// restent hors de la population. Deux d'entre eux passaient par coïncidence le
/// prédicat textuel écarté (`auto_pull_groomed` et `heartbeat` ont un bras) ; le
/// troisième l'aurait fait fire au land.
#[test]
fn mika2337_les_faux_membres_restent_hors_de_la_population() {
    let sites: Vec<String> = registered_triggers().into_values().collect();

    for faux in ["task_engine/engine.rs", "research/mechanism_analyzer.rs"] {
        assert!(
            !sites.iter().any(|s| s.replace('\\', "/") == faux),
            "{faux} ne pose aucune récurrence : il ne doit pas entrer dans la \
             population. Le prédicat s'ancre sur l'appelant, pas sur la forme \
             textuelle du littéral."
        );
    }
    assert!(
        !match_arm_triggers().contains(&"callback".to_string()),
        "`{{\"trigger\":\"callback\"}}` (helper de test, action_type = \
         resume_agent) n'est pas un run_skill et n'a légitimement aucun bras"
    );
}

// ───────────────────────── V3.2 — sonde de tir ─────────────────────────

struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// Dispatcher hermétique. `github_token` et `github_app` sont **tous deux**
/// `None` : `resolve_periodic_scan_token` rend alors `None` et
/// `dispatch_qa_review_reconcile` retourne `Ok(())` avant tout appel à `gh`.
/// La sonde s'arrête donc à la **résolution**, ce qui est exactement sa portée —
/// l'exécution du scan appelle le réseau et n'a pas sa place dans un test
/// hermétique.
fn test_dispatcher(db: AsyncDatabase) -> Arc<TaskDispatcher> {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let mut settings = mika_common::config::Settings::load(tmp.path()).expect("load settings");
    // `Settings::load` lit l'environnement : un PAT présent sur la machine de
    // l'opérateur ferait sortir la sonde sur le réseau.
    settings.github_token = None;
    Arc::new(TaskDispatcher {
        db,
        tier: mika_common::home::AgentTier::Default,
        deployment: mika_common::home::Deployment::Unknown,
        llm: mika_common::llm::dummy_provider(),
        tools: Arc::new(default_tools()),
        skills: Arc::new(SkillRegistry::empty()),
        message_sender: Some(Arc::new(NoopSender)),
        home_dir: PathBuf::from("/tmp"),
        // mika#2329 — home global inexistant : aucun STOP n'y est armé, chemin nominal.
        global_home_dir: PathBuf::from("/tmp/mika-test-global-home-absent"),
        embedding_client: None,
        brave_api_key: None,
        gateway_url: None,
        internal_token: None,
        github_token: None,
        github_app: None,
        skills_dirty: Arc::new(AtomicBool::new(false)),
        agent_lock: None,
        cli_mode: true,
        settings,
        pr_reviews_posted: None,
        auto_pull_stop_armed: AtomicBool::new(false),
        worktree_reap_stop_armed: AtomicBool::new(false),
        proactive_budget_reported: std::sync::Mutex::new(None),
    })
}

/// Enregistre la récurrence par le **vrai** chemin de production
/// (`ensure_recurring_task`), puis la fait tirer par le moteur — `tick()` →
/// `fire_task` → `dispatch`. Rend l'id de la ligne récurrente.
///
/// Le cron est à la seconde parce que le heap **recalcule** l'échéance d'une
/// récurrente depuis son `cron_expr` (`enqueue_queued_task`) et ne lit jamais la
/// colonne `next_fire_at` : antidater la colonne ne ferait rien. La boucle
/// `tick()` attend donc l'échéance réelle.
///
/// `settled` est le prédicat d'arrêt, et c'est lui qui empêche la sonde d'être
/// verte sans avoir rien tiré : la boucle échoue bruyamment si le moteur n'a
/// jamais fait feu, au lieu de laisser une assertion « le statut n'est pas
/// `failed` » passer trivialement.
async fn fire_recurring(
    db: &AsyncDatabase,
    label: &str,
    action_config: &str,
    settled: impl Fn(&mika_agent::db::Task) -> bool,
) -> String {
    mika_agent::task_engine::ensure_recurring_task(db, label, "* * * * * *", action_config).await;

    let id = db
        .get_tasks_by_status(vec!["recurring_active".to_string()])
        .await
        .expect("lire les récurrences")
        .into_iter()
        .find(|t| t.label == label)
        .map(|t| t.id)
        .expect("la récurrence doit être enregistrée");

    let dispatcher = test_dispatcher(db.clone());
    let mut engine = TaskEngine::new(db.clone(), dispatcher);
    engine.startup_recovery().await.expect("startup recovery");

    for _ in 0..120 {
        engine.tick().await;
        let task = db
            .get_task(&id)
            .await
            .expect("relire la tâche")
            .expect("la tâche existe");
        if settled(&task) {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "le moteur n'a jamais fait feu sur `{label}` en 6 s — la sonde ne \
         mesurerait rien et ses assertions passeraient pour la mauvaise raison"
    );
}

/// **AC8 — la sonde de tir demandée par le ticket.** La récurrence
/// `qa_review_reconcile` est tirée par le moteur et ne finit pas `failed` sur le
/// motif du catch-all. Sans réseau.
///
/// Elle est verte au land — le bras existe depuis `1340936e` (AC1) — et c'est
/// précisément pour cela que son contrôle négatif ci-dessous est obligatoire :
/// une sonde qui ne tirerait rien serait verte pour la même raison.
#[tokio::test]
async fn mika2337_la_recurrence_qa_review_reconcile_ne_tombe_pas_dans_le_catch_all() {
    let db = test_db();
    // Le témoin du tir réussi est le **ré-enfilement** : `ensure_recurring_task`
    // laisse `next_fire_at` NULL, et seul le bras `Ok` de `fire_task` le
    // renseigne (`update_task_rescheduled`). Un dispatch tombé dans le catch-all
    // ne replanifie rien — F4 — donc ce prédicat ne peut pas être satisfait par
    // la panne qu'il mesure.
    let id = fire_recurring(
        &db,
        "qa_review_reconcile",
        r#"{"trigger":"qa_review_reconcile"}"#,
        |t| t.next_fire_at.is_some() || t.status == "failed",
    )
    .await;

    let task = db.get_task(&id).await.unwrap().unwrap();
    assert!(
        task.next_fire_at.is_some(),
        "le tir doit avoir réussi et replanifié la récurrence ; résultat: {:?}",
        task.result
    );
    assert_ne!(
        task.status, "failed",
        "la récurrence ne doit pas mourir au dispatch ; résultat: {:?}",
        task.result
    );
    assert!(
        !task
            .result
            .as_deref()
            .unwrap_or_default()
            .contains("unknown run_skill trigger"),
        "le trigger doit résoudre vers `dispatch_qa_review_reconcile`, pas vers \
         le catch-all ; résultat: {:?}",
        task.result
    );
    assert_eq!(
        db.count_audit_events_by_tool_name("recurring_unknown_trigger")
            .await
            .unwrap(),
        0,
        "un trigger routé ne doit produire aucune ligne d'audit de cette classe"
    );
}

/// **AC3 — le contrôle négatif, et la moitié comportementale du hotfix.**
///
/// Un trigger volontairement inconnu, tiré par le même moteur : la tâche meurt
/// (l'état terminal est correct), mais la mort est désormais **nommée**
/// (`audit_events`), **typée** (variante dédiée, pas un `anyhow!` noyé dans
/// `task dispatch failed`) et **marquée** — et c'est ce marqueur qui empêche le
/// veto mika#1742 de survivre à la mort, ce que les jumeaux `db::tests::mika2337_*`
/// attestent côté ré-inscription.
#[tokio::test]
async fn mika2337_un_trigger_inconnu_meurt_nomme_audite_et_marque() {
    let db = test_db();
    let id = fire_recurring(&db, "zorglub_scan", r#"{"trigger":"zorglub"}"#, |t| {
        t.status == "failed"
    })
    .await;

    let task = db.get_task(&id).await.unwrap().unwrap();
    assert_eq!(
        task.status, "failed",
        "un trigger inconnu doit rester une mort : c'est sa *conséquence* sur la \
         ré-inscription que mika#2337 corrige, pas l'état terminal"
    );
    assert!(
        task.result
            .as_deref()
            .unwrap_or_default()
            .contains("unknown run_skill trigger: zorglub"),
        "le message rendu doit rester celui que les journaux portent depuis \
         toujours ; résultat: {:?}",
        task.result
    );

    assert_eq!(
        db.count_audit_events_by_tool_name("recurring_unknown_trigger")
            .await
            .unwrap(),
        1,
        "la classe doit être audible en SQL — sans quoi elle n'est lisible que \
         par grep sur des giga-octets de journal"
    );

    // Lu en JSON plutôt que par `get_task_metadata_field`, qui rend une chaîne :
    // le marqueur est l'entier `1`, parce que la garde compare avec `= 1` et que
    // SQLite ordonne INTEGER avant TEXT (`'1' = 1` est faux).
    let metadata: serde_json::Value =
        serde_json::from_str(task.metadata.as_deref().unwrap_or("{}"))
            .expect("metadata JSON valide");
    assert_eq!(
        metadata.get("unknown_trigger_death"),
        Some(&serde_json::json!(1)),
        "la ligne morte doit porter `{RECURRING_UNKNOWN_TRIGGER_PATH}` en entier — \
         c'est lui qui fait que le redémarrage suivant ré-inscrit au lieu \
         d'attendre 24 h ; metadata: {:?}",
        task.metadata
    );
}
