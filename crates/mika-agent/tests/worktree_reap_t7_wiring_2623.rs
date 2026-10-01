//! mika#2623 — le câblage du vecteur T7 vers le bras de purge, observé en
//! entrant par `reap_terminal_worktrees`.
//!
//! La revue multi-agents de PR #2621 (mika#2619) a nommé la lacune que ce
//! fichier ferme : *« aucun test comportemental du câblage du vecteur T7 au site
//! de production »*. `t7_is_needed` et `purge_stale_target_dirs` étaient testés
//! **séparément**, et la mutation qui coupe le câblage — remplacer
//! `&t7_refusals` par `&[]` au site d'appel — laissait `mika2619_v4`, `v5`, `v6`,
//! le scan de co-site et les 143 tests du module **verts**.
//!
//! # Pourquoi un fichier d'intégration, et pas le `mod tests` du module
//!
//! Un fichier sous `tests/` a **son propre binaire**, donc son propre process :
//! le `PATH` muté par le faux `gh` n'atteint pas les ~5700 tests du binaire de
//! la lib. C'est le confinement que `wip_rescue.rs` n'a pas pu obtenir et dont
//! il nomme le coût résiduel en toutes lettres. `#[serial_test::serial]`
//! sérialise ensuite les tests **de ce fichier** entre eux.
//!
//! Prix, nommé : `TargetPurgeStats` et `PURGE_IDLE_DEFAULT_SECS` sont privés,
//! donc l'observation se fait **par l'effet** — un répertoire qui disparaît du
//! disque pendant que son worktree reste, et une ligne d'audit — et la fenêtre
//! d'inactivité est **déclarée** par le test. Les deux sont des gains : pour AC1
//! ce qu'on veut observer est un fait sur le disque, pas un compteur interne ;
//! et une fenêtre déclarée est plus lisible qu'une constante empruntée.
//!
//! # Ce que la lecture du code a déplacé dans le ticket
//!
//! `MIKA_WORKTREE_REAP_MAX_PER_TICK=0` ne met **pas** le budget du faucheur à
//! zéro : `parse_positive_usize` retombe sur le défaut avec un `warn!` — *« le
//! `0` ne désarme pas, c'est le rôle du kill-switch »*, écrit au site. Le seul
//! chemin vers `budget == 0` à l'entrée de `t7_is_needed` est donc **deux
//! dépôts**, le premier épuisant le budget — c'est-à-dire précisément l'état que
//! le doc-comment de `t7_is_needed` nomme comme sa raison d'être. Un test
//! mono-dépôt ne peut pas atteindre AC2.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::worktree_reaper::{
    REAPED_TOOL, REASON_DIRTY, REASON_UNPUSHED_COMMITS, SKIPPED_TOOL, TARGET_PURGED_TOOL,
    collect_live_cwds,
};

// ---------------------------------------------------------------------------
// Contrôles d'environnement — ils nomment la cause, jamais un `return` muet
// ---------------------------------------------------------------------------

/// Un test qui se saute tout seul se lit exactement comme un test qui passe
/// (classe mika#2205). Les deux préconditions de ce fichier échouent donc
/// **franchement**, en nommant ce qui manque.
fn exiger_environnement() {
    assert!(
        Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success()),
        "mika#2623 — `git` est indispensable : ce test monte de vrais worktrees \
         liés. Sans lui le câblage T7 n'est pas observable depuis le site de \
         production."
    );
    assert!(
        !matches!(
            collect_live_cwds(),
            mika_agent::worktree_reaper::LiveCwds::Unavailable
        ),
        "mika#2623 — `/proc` est illisible, donc `collect_live_cwds()` rend \
         `Unavailable`, donc T6 et P3 refusent **tout** sous \
         `process_scan_unreadable` : ni le faucheur ni la purge ne peuvent agir, \
         et l'échec serait imputé au câblage."
    );
}

// ---------------------------------------------------------------------------
// Garde d'environnement
// ---------------------------------------------------------------------------

/// Restaure chaque variable posée, quelle que soit la sortie du test.
///
/// Même forme que le `PathGuard` de `wip_rescue.rs`, et même note de sûreté :
/// `std::env::set_var` est `unsafe` en edition 2024 parce qu'un `getenv`
/// concurrent est une course. Le confinement est le binaire propre **plus**
/// `#[serial_test::serial]` ; la fenêtre est de quelques millisecondes et aucun
/// autre test de ce fichier ne lit ces variables.
struct EnvGuard(Vec<(&'static str, Option<OsString>)>);

impl EnvGuard {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn set(&mut self, key: &'static str, value: &str) {
        self.0.push((key, std::env::var_os(key)));
        // SAFETY: voir la note du type — borné, pas absent.
        unsafe { std::env::set_var(key, value) };
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in self.0.drain(..).rev() {
            // SAFETY: idem.
            unsafe {
                match prev {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

struct PathGuard(Option<OsString>);

impl Drop for PathGuard {
    fn drop(&mut self) {
        // SAFETY: idem.
        unsafe {
            match self.0.take() {
                Some(prev) => std::env::set_var("PATH", prev),
                None => std::env::remove_var("PATH"),
            }
        }
    }
}

/// Met `dir` en **tête** du `PATH` — un `gh` réel installé sur la machine ne
/// doit pas gagner.
fn prepend_to_path(dir: &Path) -> PathGuard {
    let previous = std::env::var_os("PATH");
    let mut next = OsString::from(dir);
    if let Some(prev) = &previous {
        next.push(":");
        next.push(prev);
    }
    // SAFETY: idem.
    unsafe { std::env::set_var("PATH", &next) };
    PathGuard(previous)
}

/// Un faux `gh` qui journalise ses appels et rend `prs_json` sur `pr list`.
///
/// **Son tmpdir est inscrit dans le script** : `run_gh_subprocess` scrubbe tout
/// `MIKA_*` avant l'exec, donc le faux ne peut pas communiquer par une variable
/// préfixée (la note de `wip_rescue.rs`, mot pour mot).
fn install_fake_gh(bin_dir: &Path, prs_json: &str) {
    let d = bin_dir.display();
    let json_path = bin_dir.join("prs.json");
    fs::write(&json_path, prs_json).unwrap();
    let jp = json_path.display();
    let body = format!(
        "#!/bin/sh\n\
         echo \"$*\" >> {d}/calls.log\n\
         if [ \"$1 $2\" = \"pr list\" ]; then cat {jp}; exit 0; fi\n\
         echo '[]'\n\
         exit 0\n"
    );
    let path = bin_dir.join("gh");
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

// ---------------------------------------------------------------------------
// Fabrication des dépôts
// ---------------------------------------------------------------------------

fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} dans {}: {e}", cwd.display()));
    assert!(
        out.status.success(),
        "git {args:?} dans {} a échoué: {}",
        cwd.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_out(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} dans {}: {e}", cwd.display()));
    assert!(out.status.success(), "git {args:?} a échoué");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Un dépôt git jetable dont le remote `origin` fait dériver `owner/repo`.
///
/// `user.email` / `user.name` sont posés **localement** : aucun test n'écrit
/// hors de son `TempDir`, pas même dans la configuration git du poste.
fn fake_repo(root: &Path, name: &str) -> PathBuf {
    let repo = root.join(name);
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--initial-branch=main", "--quiet"]);
    git(&repo, &["config", "user.email", "t@2623.invalid"]);
    git(&repo, &["config", "user.name", "mika#2623"]);
    fs::write(
        repo.join(".gitignore"),
        b"target/\n.pilot-scratch/\n.claude/\n",
    )
    .unwrap();
    fs::write(repo.join("README.md"), b"# fixture mika#2623\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "fixture"]);
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            &format!("https://github.com/senara-solutions/{name}.git"),
        ],
    );
    repo
}

/// Un worktree **géré** : le segment `/.claude/worktrees/` est ce que T1 exige.
///
/// Placé en **frère** du dépôt, comme en production
/// (`mika-platform/.claude/worktrees/<slug>/mika`), plutôt qu'imbriqué dans
/// l'arbre de travail du dépôt lui-même.
fn add_worktree(root: &Path, repo: &Path, slug: &str, branch: &str) -> PathBuf {
    let wt = root
        .join(".claude/worktrees")
        .join(slug)
        .join(repo.file_name().unwrap());
    fs::create_dir_all(wt.parent().unwrap()).unwrap();
    git(
        repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            branch,
            wt.to_str().unwrap(),
        ],
    );
    wt
}

/// Un `target/` plausible, pour que la purge ait quelque chose à retirer.
fn fake_target(wt: &Path) -> PathBuf {
    let target = wt.join("target");
    fs::create_dir_all(target.join("debug/deps")).unwrap();
    fs::write(target.join("debug/deps/libfoo.rlib"), vec![0u8; 4096]).unwrap();
    target
}

/// Vieillit **tout** l'arbre : les répertoires aussi, et après les fichiers
/// qu'ils contiennent — créer un fichier rajeunit son répertoire parent.
///
/// Recopié du `mod tests` in-crate : le helper y est privé et non réutilisable
/// depuis l'extérieur.
fn age_tree(root: &Path, secs: u64) {
    let when = filetime::FileTime::from_system_time(SystemTime::now() - Duration::from_secs(secs));
    let mut dirs = vec![root.to_path_buf()];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let p = entry.path();
            if p.symlink_metadata().unwrap().is_dir() {
                dirs.push(p.clone());
                stack.push(p);
            } else {
                filetime::set_file_mtime(&p, when).unwrap();
            }
        }
    }
    for d in dirs.iter().rev() {
        filetime::set_file_mtime(d, when).unwrap();
    }
}

/// Une PR terminale, close il y a longtemps, sur cette branche.
fn pr_mergee(number: u64, branch: &str) -> String {
    format!(
        r#"{{"number":{number},"state":"MERGED","headRefName":"{branch}",
            "headRefOid":"","closedAt":"2020-01-01T00:00:00Z",
            "url":"https://github.com/senara-solutions/x/pull/{number}"}}"#
    )
}

fn calls(dir: &Path) -> String {
    fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
}

/// La fenêtre d'inactivité déclarée par ces tests. Rien n'est laissé à un
/// défaut : un test qui hérite d'un défaut change de sens le jour où le défaut
/// change.
const IDLE: u64 = 120;

// ===========================================================================
// U1 / AC1 — le câblage, observé en entrant par `reap_terminal_worktrees`
// ===========================================================================

/// **AC1** — un refus T7 atteint le bras de purge depuis le site d'assemblage.
///
/// Trois worktrees, un seul tick :
///
/// | worktree | état | attendu |
/// |---|---|---|
/// | `W1` | fichier non suivi | **conservé** (T7 `dirty`) ; `target/` **purgé** |
/// | `W2` | propre | **fauché** — le contrôle négatif |
/// | `W3` | un commit au-dessus de `origin/<branche>` | **conservé** (T7 `unpushed_commits`) ; `target/` **purgé** |
///
/// `W2` est porteur : sans lui, le test ne distingue pas « le câblage marche »
/// de « rien ne se passe dans ce tick ».
///
/// **Mutation à consigner (V2) :** `&t7_refusals` → `&[]` au site d'appel de
/// `purge_stale_target_dirs`. Elle laisse `v4`, `v5`, `v6`, le scan de co-site et
/// les tests du module **verts** ; seul ce test rougit, sur `W1/target` et
/// `W3/target` qui survivent.
#[tokio::test]
#[serial_test::serial]
async fn mika2623_le_cablage_t7_traverse_le_site_dassemblage() {
    exiger_environnement();

    let tmp = tempfile::tempdir().unwrap();
    // Canonicalisé : `canonical_path_is_managed` canonicalise avant de
    // re-vérifier le segment géré, et `TMPDIR` peut être un lien symbolique.
    let root = tmp.path().canonicalize().unwrap();
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();

    let repo = fake_repo(&root, "mika");

    // W1 — sale (fichier non suivi).
    let w1 = add_worktree(&root, &repo, "fix-2623-dirty", "fix/2623/dirty");
    fs::write(w1.join("DIRTY.txt"), b"non suivi\n").unwrap();
    let t1 = fake_target(&w1);

    // W2 — propre : le contrôle négatif, fauché dans le même tick.
    let w2 = add_worktree(&root, &repo, "fix-2623-clean", "fix/2623/clean");
    let t2 = fake_target(&w2);

    // W3 — un commit local au-dessus de la ref distante.
    //
    // La ref distante est **fabriquée** (`update-ref`) avant le commit local :
    // sans elle `collect_work_state` prend son repli `Clean` sur ref absente et
    // n'atteint jamais la branche `rev-list --count` qu'on veut exercer.
    let w3 = add_worktree(&root, &repo, "fix-2623-ahead", "fix/2623/ahead");
    let base = git_out(&w3, &["rev-parse", "HEAD"]);
    git(
        &repo,
        &[
            "update-ref",
            "refs/remotes/origin/fix/2623/ahead",
            base.as_str(),
        ],
    );
    fs::write(w3.join("LOCAL.txt"), b"commit local\n").unwrap();
    git(&w3, &["add", "LOCAL.txt"]);
    git(&w3, &["commit", "--quiet", "-m", "local"]);
    let t3 = fake_target(&w3);

    for t in [&t1, &t2, &t3] {
        age_tree(t, IDLE + 600);
    }

    install_fake_gh(
        &bin,
        &format!(
            "[{},{},{}]",
            pr_mergee(1, "fix/2623/dirty"),
            pr_mergee(2, "fix/2623/clean"),
            pr_mergee(3, "fix/2623/ahead")
        ),
    );
    let _path = prepend_to_path(&bin);

    let mut env = EnvGuard::new();
    env.set("MIKA_WORKTREE_REAP_REPO_DIRS", repo.to_str().unwrap());
    env.set("MIKA_WORKTREE_REAP_GRACE_SECS", "1");
    env.set("MIKA_WORKTREE_REAP_MAX_PER_TICK", "4");
    env.set("MIKA_WORKTREE_REAP_DISPOSITION", "armed");
    env.set("MIKA_TARGET_PURGE", "1");
    env.set("MIKA_TARGET_PURGE_DISPOSITION", "armed");
    env.set("MIKA_TARGET_PURGE_IDLE_SECS", &IDLE.to_string());
    env.set("MIKA_TARGET_PURGE_MAX_PER_TICK", "4");

    let db = AsyncDatabase::new(Database::open_in_memory().unwrap());
    let session = "session-2623-cablage";
    let _ = mika_agent::worktree_reaper::reap_terminal_worktrees(
        &db,
        "jeton-de-test",
        "trace-2623-cablage",
        session,
    )
    .await;

    let events = db.get_audit_events(session).await.unwrap();

    // **L'ordre des assertions est porteur.** D'abord « le dépôt a été traité »
    // — sans ce pas, un faux `gh` cassé produirait un échec dont le message
    // accuserait le câblage.
    assert!(
        calls(&bin).contains("pr list"),
        "le faux `gh` n'a pas été appelé : le dépôt n'a pas été traité, et rien \
         de ce qui suit ne dit quoi que ce soit du câblage. Appels: {:?}",
        calls(&bin)
    );
    assert!(
        events.iter().any(|e| e.tool_name == SKIPPED_TOOL),
        "aucun refus du faucheur n'a été écrit : T7 n'a pas tourné du tout. \
         Événements: {:?}",
        events.iter().map(|e| &e.tool_name).collect::<Vec<_>>()
    );

    // --- le contrôle négatif : le faucheur marche toujours ---
    assert!(
        !w2.exists(),
        "W2 (propre, PR mergée, hors grâce) doit être fauché — sans ça ce test \
         ne constate pas « rien ne se passe »"
    );
    assert!(
        events
            .iter()
            .any(|e| e.tool_name == REAPED_TOOL && e.target_key.contains("fix-2623-clean")),
        "W2 doit porter sa ligne `worktree_reaped`"
    );

    // --- AC1 : les deux refus T7 ont atteint le bras de purge ---
    for (nom, wt, target, motif) in [
        ("W1", &w1, &t1, REASON_DIRTY),
        ("W3", &w3, &t3, REASON_UNPUSHED_COMMITS),
    ] {
        assert!(
            wt.exists(),
            "{nom} doit être **conservé** par le faucheur (refus T7 {motif})"
        );
        assert!(
            !target.exists(),
            "{nom}/target doit être **purgé** : c'est la seule chose qui atteste \
             que le refus T7 `{motif}` a traversé le site d'assemblage jusqu'au \
             bras de purge. S'il survit, le câblage est coupé."
        );
        let purge = events
            .iter()
            .find(|e| e.tool_name == TARGET_PURGED_TOOL && e.target_key.contains(nom_slug(nom)))
            .unwrap_or_else(|| {
                panic!(
                    "{nom} doit porter une ligne `target_purged`. Événements: {:?}",
                    events
                        .iter()
                        .map(|e| (&e.tool_name, &e.target_key))
                        .collect::<Vec<_>>()
                )
            });
        assert!(
            purge
                .reasoning
                .as_deref()
                .unwrap_or_default()
                .contains(&format!("keep_reason={motif}")),
            "la ligne de {nom} doit porter `keep_reason={motif}` — c'est le motif \
             de conservation du faucheur, rapporté par le bras de purge: {:?}",
            purge.reasoning
        );
    }

    // Le travail n'est jamais touché : seul le dérivé l'est.
    assert!(w1.join("DIRTY.txt").exists(), "W1 garde son arbre sale");
    assert!(w3.join("LOCAL.txt").exists(), "W3 garde son commit local");
}

/// Le slug de worktree correspondant à un nom de cas.
fn nom_slug(nom: &str) -> &'static str {
    match nom {
        "W1" => "fix-2623-dirty",
        "W3" => "fix-2623-ahead",
        other => panic!("cas inconnu: {other}"),
    }
}

// ===========================================================================
// U2 / AC2 — budget faucheur nul, la purge tourne quand même
// ===========================================================================

/// **AC2** — avec le budget du faucheur épuisé par un dépôt antérieur, T7 est
/// **quand même** évalué sur le suivant et son `target/` est purgé.
///
/// # Pourquoi deux dépôts, et pas `MAX_PER_TICK=0`
///
/// `parse_positive_usize` retombe sur le défaut sur `0`, avec un `warn!` — *« le
/// `0` ne désarme pas, c'est le rôle du kill-switch, et l'inverse ferait d'une
/// coquille un désarmement silencieux sur un scan destructif »*. Le seul chemin
/// vers `budget == 0` à l'entrée de `t7_is_needed` est donc un dépôt antérieur
/// qui l'a consommé — très exactement l'état que le doc-comment de ce prédicat
/// nomme comme sa raison d'être.
///
/// # Le sens que le code choisit, et le trou qu'il laisse ouvert
///
/// `t7_is_needed(0, purge > 0, true) == true` : **le bras qui a besoin du calcul
/// le paie**. En contrepartie, la **boucle de disposition** du faucheur reste
/// sous `budget > 0` — ce qui est élargi est le calcul, jamais la disposition.
/// Conséquence, nommée en production (`worktree_reaper.rs`, § *Trou résiduel*) et
/// épinglée ici par `WC` : un candidat `Clean` du second dépôt n'est **ni fauché
/// ni refusé**, donc il n'entre dans aucun vecteur et son répertoire de build
/// échappe à ce tick. Ce n'est pas un défaut — fermer ce cas demanderait de
/// pousser un refus synthétique pour un worktree que rien ne refuse, c'est-à-dire
/// une ligne d'audit fausse.
///
/// **Mutations à consigner (V4) :** `t7_is_needed(budget, 0, purge_cfg.enabled)`
/// au site d'appel — le prédicat reste intact, donc `v5` et le scan de co-site
/// restent verts ; et `should_stop_repo_loop` ramené au seul budget du faucheur.
/// Les deux font survivre `WB/target`.
#[tokio::test]
#[serial_test::serial]
async fn mika2623_budget_faucheur_nul_la_purge_tourne() {
    exiger_environnement();

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();

    // Dépôt A — un worktree propre qui épuise le budget du faucheur.
    let repo_a = fake_repo(&root, "depot-a");
    let wa = add_worktree(&root, &repo_a, "fix-2623-a", "fix/2623/a");
    let ta = fake_target(&wa);

    // Dépôt B — `WB` sale (doit voir T7 malgré le budget nul) et `WC` propre
    // (l'épinglage du trou résiduel).
    let repo_b = fake_repo(&root, "depot-b");
    let wb = add_worktree(&root, &repo_b, "fix-2623-b", "fix/2623/b");
    fs::write(wb.join("DIRTY.txt"), b"non suivi\n").unwrap();
    let tb = fake_target(&wb);
    let wc = add_worktree(&root, &repo_b, "fix-2623-c", "fix/2623/c");
    let tc = fake_target(&wc);

    for t in [&ta, &tb, &tc] {
        age_tree(t, IDLE + 600);
    }

    install_fake_gh(
        &bin,
        &format!(
            "[{},{},{}]",
            pr_mergee(10, "fix/2623/a"),
            pr_mergee(11, "fix/2623/b"),
            pr_mergee(12, "fix/2623/c")
        ),
    );
    let _path = prepend_to_path(&bin);

    let mut env = EnvGuard::new();
    // `parse_repo_dirs` préserve l'ordre : A épuise le budget, B le subit.
    env.set(
        "MIKA_WORKTREE_REAP_REPO_DIRS",
        &format!("{}:{}", repo_a.display(), repo_b.display()),
    );
    env.set("MIKA_WORKTREE_REAP_GRACE_SECS", "1");
    env.set("MIKA_WORKTREE_REAP_MAX_PER_TICK", "1");
    env.set("MIKA_WORKTREE_REAP_DISPOSITION", "armed");
    env.set("MIKA_TARGET_PURGE", "1");
    env.set("MIKA_TARGET_PURGE_DISPOSITION", "armed");
    env.set("MIKA_TARGET_PURGE_IDLE_SECS", &IDLE.to_string());
    env.set("MIKA_TARGET_PURGE_MAX_PER_TICK", "4");

    let db = AsyncDatabase::new(Database::open_in_memory().unwrap());
    let session = "session-2623-budget";
    let _ = mika_agent::worktree_reaper::reap_terminal_worktrees(
        &db,
        "jeton-de-test",
        "trace-2623-budget",
        session,
    )
    .await;

    let events = db.get_audit_events(session).await.unwrap();

    // « Les deux dépôts ont été traités » d'abord.
    let log = calls(&bin);
    assert!(
        log.contains("depot-a") && log.contains("depot-b"),
        "les deux dépôts doivent avoir été interrogés — sinon rien de ce qui \
         suit ne dit quoi que ce soit d'AC2. Appels: {log:?}"
    );

    // Le budget du faucheur a bien été épuisé par A — précondition d'AC2.
    assert!(
        !wa.exists(),
        "WA doit être fauché (budget faucheur = 1) : sans ça le budget n'est pas \
         épuisé à l'entrée du dépôt B et le test n'exerce pas AC2"
    );

    // --- AC2 : T7 a tourné sur B malgré le budget nul ---
    assert!(
        wb.exists(),
        "WB doit être **conservé** — le budget du faucheur est épuisé, donc sa \
         boucle de disposition ne tourne pas, et T7 le refuse de toute façon"
    );
    assert!(
        !tb.exists(),
        "WB/target doit être **purgé** : c'est AC2. Le budget du faucheur est à \
         zéro, mais `t7_is_needed` reste vrai tant que la purge est armée et a du \
         budget — **le bras qui a besoin du calcul le paie**. S'il survit, T7 \
         n'est plus évalué quand le faucheur est à sec."
    );
    let purge = events
        .iter()
        .find(|e| e.tool_name == TARGET_PURGED_TOOL && e.target_key.contains("fix-2623-b"))
        .unwrap_or_else(|| {
            panic!(
                "WB doit porter une ligne `target_purged`. Événements: {:?}",
                events
                    .iter()
                    .map(|e| (&e.tool_name, &e.target_key))
                    .collect::<Vec<_>>()
            )
        });
    assert!(
        purge
            .reasoning
            .as_deref()
            .unwrap_or_default()
            .contains(&format!("keep_reason={REASON_DIRTY}")),
        "la ligne de WB porte le motif T7 : {:?}",
        purge.reasoning
    );

    // --- le trou résiduel, épinglé dans le sens que le code choisit ---
    assert!(
        wc.exists(),
        "WC doit survivre : le budget du faucheur est épuisé, sa disposition ne \
         tourne pas"
    );
    assert!(
        tc.exists(),
        "WC/target doit **survivre** — et ce n'est pas un défaut. `WC` est \
         `Clean`, donc T7 ne le refuse pas ; et le budget du faucheur étant nul, \
         sa boucle de disposition ne le fauche pas non plus. Il n'entre donc dans \
         **aucun** vecteur et échappe à ce tick. Transitoire (≤ 10 min), borné, et \
         fermer ce cas demanderait de pousser un refus synthétique pour un \
         worktree que rien ne refuse — une ligne d'audit fausse."
    );
    assert!(
        !events
            .iter()
            .any(|e| e.tool_name == TARGET_PURGED_TOOL && e.target_key.contains("fix-2623-c")),
        "aucune ligne de purge pour WC"
    );
}
