//! Le seul site `Command::new("gh")` de la suite `auto_pull` (mika#2713).
//!
//! # Pourquoi ce module existe
//!
//! Le 2026-10-03 puis le 2026-10-14, le ticket **fermé** mika#2636 a reçu
//! 29 commentaires « Auto-pull : promotion refusée pour #2636 » sous l'identité
//! du moteur, plus un label `operator-gated`. Le producteur était un test
//! unitaire : `mika2634_une_fenetre_propre_ne_freine_pas_le_tick` posait
//! `let n = 2636u64;`, construisait une issue avec la fixture `GROOMED_BODY`
//! (dont le callout nomme la branche `feat/123/x`, qui n'existe pas sur
//! `origin`), puis appelait `phase2_reconcile_stuck_ready`. La porte de
//! promotion lisait la timeline du **vrai** #2636, recevait un 404 sur la
//! branche de la fixture, et `refuse_promotion` écrivait.
//!
//! Le `GH_TOKEN="fake-token"` que chaque site posait n'y changeait rien :
//! l'addon d'egress des sandboxes de pilote
//! (`scripts/mika-pilot-github-auth-addon.py`) **retire** l'`Authorization` du
//! client — « the sandbox NEVER holds the GitHub token » — puis **injecte** le
//! jeton lu côté hôte. Tout `cargo test` lancé depuis une sandbox écrit donc
//! sur la forge réelle, sous l'identité du moteur, quel que soit le jeton du
//! test. Le durcissement de cet egress est suivi par mika#2716 ; ici la
//! correction est côté test, et elle tient par construction.
//!
//! # La propriété
//!
//! Sous `#[cfg(test)]`, [`run_gh`] **ne spawne aucun processus**. Elle
//! enregistre l'appel, rend `exit 0` / `stdout = "[]"`, et **panique** sur un
//! [`GhSubject::Issue`] dont le numéro est sous [`FIXTURE_ISSUE_FLOOR`] — un
//! numéro qui pourrait désigner un vrai ticket. La propriété est posée au seul
//! endroit par lequel un numéro peut sortir, donc elle couvre les fixtures
//! futures sans qu'on y pense.
//!
//! `exit 0` / `"[]"` plutôt qu'un échec, et ce choix est mesuré :
//! `auto_pull::gh_compare_branch` ne mappe en `BranchAbsent` que le **404** —
//! tout autre échec devient `Unavailable`, qui **promeut** (fail-open), donc un
//! ENOENT changerait le verdict de la porte au lieu d'être neutre. Et
//! `enact_stuck_ready_skip` n'écrit sa ligne d'audit *que si* le commentaire a
//! été posté, donc `mika2361_the_reentry_comment_is_posted_once_per_abandonment`
//! dépend d'un `gh` qui réussit.

use std::process::Output;

/// À quoi l'appel `gh` s'adresse.
///
/// Le sujet n'est pas décoratif : sous test, c'est lui qui permet au double de
/// refuser un numéro d'issue réel (mika#2713, AC3). En production il n'est pas
/// lu — la couture ne décide rien, elle passe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GhSubject {
    /// Une issue, par son numéro. La variante que le plancher de fixture garde.
    Issue(u64),
    /// Une pull request, par son numéro.
    ///
    /// Aucun des huit sites du périmètre de mika#2713 ne s'adresse à une PR
    /// aujourd'hui ; la variante existe pour que les cinq sites hors périmètre
    /// (mika#2715) n'aient pas à élargir l'énumération au moment de leur
    /// conversion, et elle est exercée par les tests de ce module.
    #[allow(dead_code)]
    Pr(u64),
    /// Le dépôt dans son ensemble : une liste, une comparaison de branches, un
    /// `gh api user`. Rien qu'un numéro de fixture puisse atteindre.
    Repo,
}

/// Le boilerplate commun des huit sites, écrit une fois.
///
/// Rend le même `io::Result<Output>` que `Command::output().await`, pour que
/// chaque site garde son mapping d'erreur **inchangé** : c'est la condition
/// pour que la conversion soit neutre en comportement.
#[cfg(not(test))]
pub(crate) async fn run_gh(
    token: &str,
    _subject: GhSubject,
    args: &[&str],
) -> std::io::Result<Output> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(args);
    cmd.env("GH_TOKEN", token);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);
    cmd.output().await
}

// ---------------------------------------------------------------------------
// Le double — sous `#[cfg(test)]`, rien ne sort de ce processus
// ---------------------------------------------------------------------------

/// Plancher des numéros d'issue qu'une fixture peut adresser (AC3).
///
/// Au-dessus, aucun ticket réel du dépôt : le plus haut à ce jour est dans les
/// 2 700. La bande `990_00x` a déjà un précédent dans `auto_pull.rs`
/// (« Issue numbers well outside the real range »).
#[cfg(test)]
pub(crate) const FIXTURE_ISSUE_FLOOR: u64 = 990_000;

/// Un appel `gh` tel que le double l'a vu.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GhCall {
    pub(crate) subject: GhSubject,
    pub(crate) args: String,
}

#[cfg(test)]
static CALL_LOG: std::sync::LazyLock<std::sync::Mutex<Vec<GhCall>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// `stdout` canné par sujet, pour le test qui a besoin d'une charge fabriquée.
///
/// Keyed par sujet **exprès** : la suite tourne en parallèle dans un seul
/// processus, donc un override global serait lu par les tests voisins. Chaque
/// test posant un override le pose sur son propre numéro de fixture, qui est
/// distinct de ceux des autres (mika#2713 D4).
#[cfg(test)]
static CANNED_STDOUT: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<GhSubjectKey, Vec<u8>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Forme hachable de [`GhSubject`], pour la table d'overrides.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GhSubjectKey(&'static str, u64);

#[cfg(test)]
fn subject_key(subject: GhSubject) -> GhSubjectKey {
    match subject {
        GhSubject::Issue(n) => GhSubjectKey("issue", n),
        GhSubject::Pr(n) => GhSubjectKey("pr", n),
        GhSubject::Repo => GhSubjectKey("repo", 0),
    }
}

/// Le double : enregistre, refuse un vrai numéro, et réussit à vide.
#[cfg(test)]
pub(crate) async fn run_gh(
    _token: &str,
    subject: GhSubject,
    args: &[&str],
) -> std::io::Result<Output> {
    let args = args.join(" ");
    if let GhSubject::Issue(n) = subject {
        assert!(
            n >= FIXTURE_ISSUE_FLOOR,
            "mika#2713 — un test vient d'adresser l'issue #{n}, qui est sous le \
             plancher de fixture {FIXTURE_ISSUE_FLOOR} et peut donc désigner un \
             VRAI ticket du dépôt. C'est exactement la fuite de mika#2713 : 29 \
             commentaires et un label `operator-gated` sur le #2636 fermé. \
             Renumérote la fixture dans la bande {FIXTURE_ISSUE_FLOOR}+ ; il n'y \
             a pas d'allowlist. Args : `gh {args}`"
        );
    }
    CALL_LOG
        .lock()
        .expect("le journal d'appels `gh` doit être lisible")
        .push(GhCall {
            subject,
            args: args.clone(),
        });

    let stdout = CANNED_STDOUT
        .lock()
        .expect("la table d'overrides `gh` doit être lisible")
        .get(&subject_key(subject))
        .cloned()
        .unwrap_or_else(|| b"[]".to_vec());

    Ok(Output {
        status: canned_success(),
        stdout,
        stderr: Vec::new(),
    })
}

/// `exit 0`, sans spawner quoi que ce soit.
#[cfg(test)]
fn canned_success() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(0)
}

/// Les appels `gh` que le double a vus depuis le début du processus.
///
/// Process-global : un test filtre sur **son** numéro de fixture, ce qui le
/// rend indifférent à l'exécution parallèle.
#[cfg(test)]
pub(crate) fn recorded_calls() -> Vec<GhCall> {
    CALL_LOG
        .lock()
        .expect("le journal d'appels `gh` doit être lisible")
        .clone()
}

/// Fabrique le `stdout` que le double rendra pour ce sujet.
#[cfg(test)]
pub(crate) fn set_canned_stdout(subject: GhSubject, stdout: impl Into<Vec<u8>>) {
    CANNED_STDOUT
        .lock()
        .expect("la table d'overrides `gh` doit être lisible")
        .insert(subject_key(subject), stdout.into());
}

/// Le plancher d'AC3, à appeler par un test qui pose un numéro de fixture.
///
/// Le double le vérifie déjà au passage de la couture ; ce helper le vérifie
/// **à la pose**, pour qu'un test dont la fixture n'atteindrait pas l'I/O se
/// nomme quand même de lui-même plutôt que d'attendre un changement futur.
#[cfg(test)]
pub(crate) fn assert_fixture_issue(n: u64) {
    assert!(
        n >= FIXTURE_ISSUE_FLOOR,
        "mika#2713 — la fixture #{n} est sous le plancher {FIXTURE_ISSUE_FLOOR} \
         et peut désigner un vrai ticket du dépôt. Renumérote-la dans la bande \
         {FIXTURE_ISSUE_FLOOR}+."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;

    // -----------------------------------------------------------------------
    // L3 — le scan structurel : qui garde le droit de spawner `gh` ?
    // -----------------------------------------------------------------------

    /// Les sites `Command::new("gh")` que mika#2713 laisse **hors** de sa borne,
    /// par fichier et par compte. Suivi de conversion : **mika#2715**.
    ///
    /// Les cinq fonctions concernées, nommées parce qu'un compte ne se relit
    /// pas :
    ///
    /// - `skills/builtin_handlers.rs` — `gh_read`, `run_gh_pr_view_author_title`,
    ///   `run_gh` (trois sites) ;
    /// - `milestone_manager/reader.rs` — `GhRunner::run` (son propre trait, avec
    ///   son double de test) ;
    /// - `tools/pr_merge_with_gate.rs` — `run_gh_subprocess` (déjà un point de
    ///   passage unique).
    ///
    /// Aucun n'est atteint par la suite `auto_pull`, et aucun n'a jamais fui.
    ///
    /// Le compte, plutôt qu'un numéro de ligne ou un nom de fonction : une ligne
    /// bouge au premier `cargo fmt` et un nom au premier renommage, alors que
    /// « ce fichier porte exactement trois spawns » reste vrai et reste
    /// vérifiable. Convertir **un seul** des trois sites de `builtin_handlers.rs`
    /// fait donc rougir ce test (3 ≠ 2), ce qui est l'effet voulu.
    const GH_SPAWN_ALLOWLIST: &[(&str, usize)] = &[
        ("skills/builtin_handlers.rs", 3),
        ("milestone_manager/reader.rs", 1),
        ("tools/pr_merge_with_gate.rs", 1),
    ];

    /// Le fichier qui porte la couture : son unique spawn **est** la couture.
    const SEAM_FILE: &str = "gh_process.rs";

    /// Combien de sites de spawn `gh` ce contenu de production porte-t-il ?
    ///
    /// Isolé du scan pour que le contrôle de bonne foi ci-dessous puisse
    /// l'appliquer à un contenu planté — sans quoi « le scan ne trouve rien »
    /// et « le scan ne regarde rien » rendraient les mêmes octets.
    fn gh_spawn_sites_in(production: &str) -> Vec<(usize, String)> {
        // Épelé par concaténation : sans quoi la première chose que le scan
        // trouverait serait sa propre définition.
        let needle = ["Command::new(", "\"gh\")"].concat();
        production
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                let t = line.trim_start();
                // Un commentaire *parlant* d'un spawn n'en est pas un, et ce
                // module en porte huit lignes.
                !t.starts_with("//") && !t.starts_with("*") && line.contains(&needle)
            })
            .map(|(n, line)| (n + 1, line.trim().to_string()))
            .collect()
    }

    /// AC2, moitié structurelle — **aucun** autre site ne peut spawner `gh`.
    ///
    /// L'assertion est une **égalité** avec [`GH_SPAWN_ALLOWLIST`], pas une
    /// inclusion : quand mika#2715 convertira un site, retirer son entrée
    /// devient obligatoire, sinon ce test rougit en disant « entrée stale ».
    /// L'allowlist se nettoie donc elle-même.
    #[test]
    fn mika2713_aucun_spawn_gh_hors_la_couture_et_son_allowlist() {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let src_root = scanner.src_root().to_path_buf();

        let mut observed: BTreeMap<String, usize> = BTreeMap::new();
        let mut offender_lines: Vec<String> = Vec::new();

        scanner.for_each(|path: &Path, production: &str| {
            if crate::source_scan::is_test_source_path(path) {
                return;
            }
            let rel = path
                .strip_prefix(&src_root)
                .unwrap_or(path)
                .display()
                .to_string();
            if rel == SEAM_FILE {
                return;
            }
            let sites = gh_spawn_sites_in(production);
            if sites.is_empty() {
                return;
            }
            *observed.entry(rel.clone()).or_default() += sites.len();
            for (line, text) in sites {
                offender_lines.push(format!("{rel}:{line}: {text}"));
            }
        });

        // Non-vacuité : si le scan ne voyait plus rien du tout, l'égalité
        // ci-dessous serait satisfaite par une allowlist vide **et** par un
        // scan cassé. Tant que mika#2715 n'a pas converti les cinq sites, il
        // doit en rester.
        assert!(
            !GH_SPAWN_ALLOWLIST.is_empty(),
            "mika#2713 — l'allowlist est vide : si mika#2715 a bien converti \
             les cinq sites, supprime cette assertion de non-vacuité en même \
             temps. Sinon, le scan ne scanne plus rien."
        );

        let expected: BTreeMap<String, usize> = GH_SPAWN_ALLOWLIST
            .iter()
            .map(|(p, n)| ((*p).to_string(), *n))
            .collect();

        assert_eq!(
            observed,
            expected,
            "mika#2713 — la carte des sites `gh` spawnés a changé.\n\
             Attendu (allowlist, suivi mika#2715) : {expected:?}\n\
             Observé : {observed:?}\n\
             Sites vus :\n  {}\n\n\
             Un site EN TROP : route-le par `gh_process::run_gh` — c'est la \
             couture qui empêche un test d'atteindre la forge réelle (29 \
             commentaires sur le #2636 fermé). N'ajoute pas d'entrée.\n\
             Une entrée EN MOINS : le site a été converti, retire son entrée \
             de `GH_SPAWN_ALLOWLIST`.",
            offender_lines.join("\n  ")
        );
    }

    /// Contrôle de bonne foi (idiome mika#2425) — le scan **mord** vraiment.
    ///
    /// Sans lui, un prédicat qui ne reconnaît plus rien laisserait le test
    /// ci-dessus vert en permanence.
    #[test]
    fn mika2713_le_scan_mord_sur_un_spawn_plante() {
        let planted = "async fn fuite() {\n    \
             let mut cmd = tokio::process::Command::new(\"gh\");\n    \
             cmd.args([\"issue\", \"comment\"]);\n}\n";
        let sites = gh_spawn_sites_in(planted);
        assert_eq!(
            sites.len(),
            1,
            "le prédicat du scan doit reconnaître un spawn planté ; sinon le \
             scan structurel est vert pour la mauvaise raison. Vu : {sites:?}"
        );
        assert_eq!(sites[0].0, 2, "et le signaler à sa ligne : {sites:?}");

        // L'autre moitié de la bonne foi : un commentaire *parlant* d'un spawn
        // n'en est pas un. Ce module en porte huit lignes, et une allowlist
        // gonflée par de la prose serait une allowlist qui ne dit plus rien.
        let prose = "// un `Command::new(\"gh\")` ne doit jamais partir d'un test\n";
        assert!(
            gh_spawn_sites_in(prose).is_empty(),
            "la prose à propos d'un spawn n'est pas un spawn"
        );
    }

    // -----------------------------------------------------------------------
    // L5 — les contrôles comportementaux du double
    // -----------------------------------------------------------------------

    /// Le double enregistre, pour que W2 puisse assert sur l'appel.
    #[tokio::test]
    async fn mika2713_le_double_enregistre_l_appel() {
        let n = FIXTURE_ISSUE_FLOOR + 201;
        let out = run_gh("fake-token", GhSubject::Issue(n), &["issue", "view", "x"])
            .await
            .expect("le double réussit toujours");
        assert!(out.status.success(), "le double rend `exit 0`");

        let mine: Vec<GhCall> = recorded_calls()
            .into_iter()
            .filter(|c| c.subject == GhSubject::Issue(n))
            .collect();
        assert_eq!(
            mine.len(),
            1,
            "un appel, un enregistrement ; filtré sur le numéro de cette \
             fixture car le journal est process-global. Vu : {mine:?}"
        );
        assert_eq!(mine[0].args, "issue view x");
    }

    /// AC3 au passage de la couture — un vrai numéro est **refusé**, en le
    /// nommant. C'est le numéro exact de l'incident.
    #[tokio::test]
    #[should_panic(expected = "2636")]
    async fn mika2713_le_double_refuse_le_numero_de_l_incident() {
        let _ = run_gh(
            "fake-token",
            GhSubject::Issue(2636),
            &["issue", "comment", "2636"],
        )
        .await;
    }

    /// Et le plancher est bien une **borne**, pas une liste : le premier numéro
    /// sous lui est refusé, le premier au-dessus passe.
    #[tokio::test]
    #[should_panic(expected = "989999")]
    async fn mika2713_le_plancher_est_une_borne() {
        let _ = run_gh(
            "fake-token",
            GhSubject::Issue(FIXTURE_ISSUE_FLOOR - 1),
            &["issue", "comment"],
        )
        .await;
    }

    /// AC2, moitié comportementale — **aucun vrai `gh` n'a répondu**.
    ///
    /// L'argument est une sous-commande qui n'existe pas : un vrai `gh`
    /// rendrait un code non nul et un `stderr` non vide. Le double rend
    /// `exit 0`, `stdout = "[]"`, `stderr` vide — donc si quelqu'un remettait
    /// un spawn sous `#[cfg(test)]`, ce test rougirait.
    ///
    /// Pourquoi pas la variante « `PATH` sans `gh` » qu'énonce AC2 : muter
    /// `PATH` mute le processus que ~6 000 tests parallèles partagent, et
    /// `std::env::set_var` est `unsafe` en édition 2024. Le couple
    /// (scan structurel + cet appel impossible) ferme la même classe sans
    /// introduire de course.
    #[tokio::test]
    async fn mika2713_aucun_vrai_gh_ne_repond_sous_test() {
        let out = run_gh(
            "fake-token",
            GhSubject::Repo,
            &["mika-2713-sous-commande-inexistante"],
        )
        .await
        .expect("le double ne peut pas échouer en I/O : il n'y a pas d'I/O");

        assert!(
            out.status.success(),
            "un vrai `gh` aurait refusé cette sous-commande ; un code non nul \
             ici veut dire qu'un processus a été spawné"
        );
        assert_eq!(
            out.stdout, b"[]",
            "le `stdout` est celui du double, pas celui d'un vrai `gh`"
        );
        assert!(
            out.stderr.is_empty(),
            "un vrai `gh` aurait écrit sur stderr : {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Le `stdout` canné est **par sujet**, pour qu'il ne traverse pas les
    /// tests voisins sous exécution parallèle (D4).
    #[tokio::test]
    async fn mika2713_le_stdout_canne_ne_fuit_pas_sur_un_autre_sujet() {
        let mine = FIXTURE_ISSUE_FLOOR + 202;
        let voisin = FIXTURE_ISSUE_FLOOR + 203;
        set_canned_stdout(
            GhSubject::Issue(mine),
            b"[{\"event\":\"labeled\"}]".to_vec(),
        );

        let a = run_gh("t", GhSubject::Issue(mine), &["api", "x"])
            .await
            .expect("double");
        assert_eq!(a.stdout, b"[{\"event\":\"labeled\"}]");

        let b = run_gh("t", GhSubject::Issue(voisin), &["api", "x"])
            .await
            .expect("double");
        assert_eq!(
            b.stdout, b"[]",
            "l'override d'un sujet ne doit pas être lu par un autre"
        );
    }

    /// La variante `Pr` existe pour mika#2715 et n'est pas gardée par le
    /// plancher — une PR n'est pas une issue, et c'est l'issue que l'incident
    /// a touchée. Pinné pour qu'une conversion future ne croie pas l'inverse.
    #[tokio::test]
    async fn mika2713_le_plancher_garde_l_issue_pas_la_pr() {
        let out = run_gh("t", GhSubject::Pr(2636), &["pr", "view", "2636"])
            .await
            .expect("le double réussit");
        assert!(out.status.success());
    }
}
