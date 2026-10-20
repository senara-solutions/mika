//! Le point de passage unique des spawns `gh` (mika#2713, puis mika#2715).
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
//!
//! # Deux entrées, une seule propriété (mika#2715)
//!
//! La couture a **deux** entrées, parce que ses appelants ne lisent pas la même
//! chose, pas parce qu'il y a deux façons de spawner :
//!
//! - [`run_gh`] rend un `io::Result<Output>` — la forme de `output().await`,
//!   celle que lit la suite `auto_pull` ;
//! - [`run_gh_tool`] rend un [`ToolOutput`] — la forme que lisent les handlers
//!   de `skills::builtin_handlers`, et elle délègue la lecture à
//!   `builtin_handlers::spawn_and_collect`.
//!
//! Cette seconde entrée existe parce que la prémisse de mika#2715 (« la couture
//! rend le même `io::Result<Output>` ») est **fausse** pour ces sites : ils ne
//! lisent pas la sortie avec `output()`, qui est non bornée, mais avec
//! `spawn_and_collect`, qui plafonne à `MAX_OUTPUT_LEN`, compte les octets
//! jetés, tique toutes les 30 s (#900) et préfixe `Exit code: N` — convention
//! que `tool_execution::has_non_zero_exit_prefix` relit (mika#2237). Les router
//! par `run_gh` aurait rendu la lecture non bornée : la régression de mika#1746
//! réintroduite pour fermer une classe qui n'a jamais fui.
//!
//! Les deux entrées partagent le **même** double — `run_gh_tool` sous
//! `#[cfg(test)]` délègue à `run_gh` — donc un seul `CALL_LOG`, un seul
//! plancher, une seule table de `stdout` canné, et pas deux doubles libres de
//! divergir.

use crate::tools::ToolOutput;
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
    /// Aucun des huit sites du périmètre de mika#2713 ne s'y adressait ; depuis
    /// mika#2715 elle est construite en production par [`gh_subject_from_argv`]
    /// et par les trois sites convertis de `skills::builtin_handlers`. Elle
    /// n'est **pas** gardée par le plancher de fixture : c'est l'issue que
    /// l'incident a touchée, et une PR n'est pas une issue.
    Pr(u64),
    /// Le dépôt dans son ensemble : une liste, une comparaison de branches, un
    /// `gh api user`. Rien qu'un numéro de fixture puisse atteindre.
    Repo,
}

/// À quoi un `argv` de `gh` s'adresse, lu depuis l'`argv` lui-même (mika#2715).
///
/// `builtin_handlers::run_gh` sert des sous-commandes libres : il n'y a pas de
/// champ d'entrée structuré où lire le sujet. La règle est volontairement
/// courte — première position `issue` ou `pr`, puis le premier positionnel
/// numérique — parce qu'un sujet deviné trop largement **arme** le plancher au
/// mauvais endroit. Deux conséquences assumées :
///
/// - la boucle **s'arrête au premier jeton en `-`**, sinon
///   `gh issue list --milestone 13` donnerait `Issue(13)` et paniquerait sous
///   test alors qu'il ne s'adresse à aucune issue ;
/// - `gh api /repos/o/r/issues/2636/comments` est classé [`GhSubject::Repo`] et
///   **ne réveille pas** le plancher. Armer le plancher sur un chemin d'API
///   demande un parseur d'URL, hors périmètre de ce vol ; la classe que le
///   plancher ferme passe par `issue`/`pr`.
pub(crate) fn gh_subject_from_argv(args: &[String]) -> GhSubject {
    let mut it = args.iter();
    let is_pr = match it.next().map(String::as_str) {
        Some("issue") => false,
        Some("pr") => true,
        _ => return GhSubject::Repo,
    };
    for tok in it {
        if tok.starts_with('-') {
            break;
        }
        // `normalize_pr_identifier` est **le** normaliseur d'identifiant de PR
        // du crate (mika#2237) : un second ici serait la divergence que
        // mika#2158 a mesurée. Sur un jeton qui n'est pas une URL de PR il rend
        // son entrée, donc la boucle continue.
        let candidate = if is_pr {
            crate::skills::builtin_handlers::normalize_pr_identifier(tok)
        } else {
            tok.as_str()
        };
        if let Ok(n) = candidate.parse::<u64>() {
            return if is_pr {
                GhSubject::Pr(n)
            } else {
                GhSubject::Issue(n)
            };
        }
    }
    GhSubject::Repo
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

/// La seconde entrée : le boilerplate des sites qui lisent un [`ToolOutput`].
///
/// Elle construit la `Command` — `GH_PROMPT_DISABLED=1`, scrub des `MIKA_*`,
/// puis `GH_TOKEN` **après** le scrub (#380) — et **délègue** la lecture à
/// `builtin_handlers::spawn_and_collect` plutôt que de la réimplémenter, pour
/// les raisons du § *Deux entrées* du module.
///
/// `repo` est replié en `--repo <repo>` à la fin de l'`argv` quand il est
/// `Some` — la forme qu'avaient les trois sites. `gh_read` le replie déjà
/// par-opération dans `build_gh_read_command` et passe donc `None`.
#[cfg(not(test))]
pub(crate) async fn run_gh_tool(
    token: Option<&str>,
    _subject: GhSubject,
    args: &[String],
    repo: Option<&str>,
) -> ToolOutput {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(args);
    if let Some(repo) = repo {
        cmd.arg("--repo").arg(repo);
    }
    cmd.env("GH_PROMPT_DISABLED", "1");
    crate::skills::executor::scrub_mika_env_vars(&mut cmd);

    // #380 — `scrub_mika_env_vars` retire `GH_TOKEN` (défense en profondeur
    // contre une fuite de `.env`), donc le jeton de plateforme se réinjecte
    // APRÈS le scrub. L'ordre est la propriété, pas un détail de style.
    if let Some(token) = token {
        // substrate-ok: an env() argument handed to a child process — it never
        // enters a tool-result `content`, so no tier reads it.
        cmd.env("GH_TOKEN", token);
    }

    crate::skills::builtin_handlers::spawn_and_collect(cmd, "gh", "Is the GitHub CLI installed?")
        .await
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

/// Échec canné par sujet **et par motif d'`argv`** (mika#2839).
///
/// Le motif, et non le seul sujet : un test qui veut mesurer ce que fait un
/// appelant quand une **écriture** est refusée porte souvent deux appels sur la
/// même issue — lire le corps, puis l'écrire — dont un seul doit échouer. Un
/// échec posé sur le sujet entier rendrait la lecture illisible et le test
/// mesurerait l'autre branche sans le dire.
///
/// Keyed par sujet pour la même raison que [`CANNED_STDOUT`] : la suite tourne
/// en parallèle dans un seul processus (mika#2713 D4).
#[cfg(test)]
static CANNED_FAILURES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<GhSubjectKey, (String, String)>>,
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

    // L'échec canné passe **avant** le `stdout` : un appel refusé ne rend pas
    // de charge, et l'appelant lit son `stderr`.
    let failure = {
        let table = CANNED_FAILURES
            .lock()
            .expect("la table d'échecs cannés `gh` doit être lisible");
        table
            .get(&subject_key(subject))
            .filter(|(needle, _)| args.contains(needle.as_str()))
            .map(|(_, stderr)| stderr.clone())
    };
    if let Some(stderr) = failure {
        return Ok(Output {
            status: canned_failure(),
            stdout: Vec::new(),
            stderr: stderr.into_bytes(),
        });
    }

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

/// Le double de la seconde entrée — **il délègue au premier**.
///
/// Déléguer plutôt que dupliquer : le journal, le plancher et la table
/// d'overrides sont ceux de [`run_gh`], donc le plancher ne peut pas être armé
/// d'un côté et muet de l'autre.
#[cfg(test)]
pub(crate) async fn run_gh_tool(
    token: Option<&str>,
    subject: GhSubject,
    args: &[String],
    repo: Option<&str>,
) -> ToolOutput {
    let mut argv: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Some(repo) = repo {
        argv.push("--repo");
        argv.push(repo);
    }
    let out = run_gh(token.unwrap_or(""), subject, &argv)
        .await
        .expect("le double ne fait aucune I/O : il ne peut pas échouer");
    ToolOutput::success(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `exit 0`, sans spawner quoi que ce soit.
#[cfg(test)]
fn canned_success() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(0)
}

/// `exit 1`, sans spawner quoi que ce soit — la forme qu'un `gh` refusé rend.
///
/// `1 << 8` et non `1` : `from_raw` prend le *wait status* de `waitpid`, dont le
/// code de sortie occupe les bits hauts. `from_raw(1)` décrirait un processus
/// tué par le signal 1, dont `.code()` rend `None`.
#[cfg(test)]
fn canned_failure() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(1 << 8)
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

/// Fabrique l'échec que le double rendra pour ce sujet, sur les appels dont
/// l'`argv` joint contient `argv_needle` (mika#2839).
///
/// Les autres appels du même sujet continuent de réussir : c'est la propriété
/// qui rend testable « la lecture a répondu, l'écriture a été refusée ».
#[cfg(test)]
pub(crate) fn set_canned_failure(
    subject: GhSubject,
    argv_needle: impl Into<String>,
    stderr: impl Into<String>,
) {
    CANNED_FAILURES
        .lock()
        .expect("la table d'échecs cannés `gh` doit être lisible")
        .insert(subject_key(subject), (argv_needle.into(), stderr.into()));
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

    /// Les sites `Command::new("gh")` encore hors de la couture, par fichier et
    /// par compte. Suivi de conversion : **mika#2715, vol 2**.
    ///
    /// Les deux fonctions concernées, nommées parce qu'un compte ne se relit
    /// pas :
    ///
    /// - `milestone_manager/reader.rs` — `GhRunner::run`. Elle a son propre
    ///   trait et huit doubles de test, et `reader.rs:1152` porte une garde
    ///   d'anti-vacuité qui en dépend : le trait **délègue**, il ne disparaît
    ///   pas ;
    /// - `tools/pr_merge_with_gate.rs` — `run_gh_subprocess`, déjà un point de
    ///   passage unique, avec 26 appelants.
    ///
    /// Aucun n'est atteint par la suite `auto_pull`, et aucun n'a jamais fui.
    /// Les trois sites de `skills/builtin_handlers.rs` que mika#2713 laissait
    /// ici sont partis par [`super::run_gh_tool`] (mika#2715, vol 1).
    ///
    /// Le compte, plutôt qu'un numéro de ligne ou un nom de fonction : une ligne
    /// bouge au premier `cargo fmt` et un nom au premier renommage, alors que
    /// « ce fichier porte exactement un spawn » reste vrai et reste vérifiable.
    const GH_SPAWN_ALLOWLIST: &[(&str, usize)] = &[
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

    /// La carte réelle des spawns `gh` du crate, hors couture et hors sources
    /// de test : `(fichier → compte, lignes fautives)`.
    ///
    /// Isolée du test ci-dessous pour que le contrôle négatif de staleness
    /// puisse la comparer à une allowlist volontairement périmée — sinon ce
    /// contrôle porterait sur deux cartes fabriquées, et « l'égalité distingue
    /// une entrée stale » resterait une tautologie.
    fn observed_gh_spawn_map() -> (BTreeMap<String, usize>, Vec<String>) {
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

        (observed, offender_lines)
    }

    /// AC2, moitié structurelle — **aucun** autre site ne peut spawner `gh`.
    ///
    /// L'assertion est une **égalité** avec [`GH_SPAWN_ALLOWLIST`], pas une
    /// inclusion : quand un vol convertit un site, retirer son entrée devient
    /// obligatoire, sinon ce test rougit en disant « entrée stale ».
    /// L'allowlist se nettoie donc elle-même — mika#2715 vol 1 vient d'en faire
    /// l'usage, en retirant l'entrée des trois sites de `builtin_handlers.rs`.
    #[test]
    fn mika2713_aucun_spawn_gh_hors_la_couture_et_son_allowlist() {
        let (observed, offender_lines) = observed_gh_spawn_map();

        // Non-vacuité : si le scan ne voyait plus rien du tout, l'égalité
        // ci-dessous serait satisfaite par une allowlist vide **et** par un
        // scan cassé. Tant que mika#2715 vol 2 n'a pas converti les deux sites
        // restants, il doit en rester.
        assert!(
            !GH_SPAWN_ALLOWLIST.is_empty(),
            "mika#2713 — l'allowlist est vide : si mika#2715 vol 2 a bien \
             converti `reader.rs` et `pr_merge_with_gate.rs`, supprime cette \
             assertion de non-vacuité en même temps. Sinon, le scan ne scanne \
             plus rien."
        );

        let expected: BTreeMap<String, usize> = GH_SPAWN_ALLOWLIST
            .iter()
            .map(|(p, n)| ((*p).to_string(), *n))
            .collect();

        assert_eq!(
            observed,
            expected,
            "mika#2713 — la carte des sites `gh` spawnés a changé.\n\
             Attendu (allowlist, suivi mika#2715 vol 2) : {expected:?}\n\
             Observé : {observed:?}\n\
             Sites vus :\n  {}\n\n\
             Un site EN TROP : route-le par la couture — c'est elle qui \
             empêche un test d'atteindre la forge réelle (29 commentaires sur \
             le #2636 fermé). `gh_process::run_gh` si le site lit un \
             `io::Result<Output>`, `gh_process::run_gh_tool` s'il lit un \
             `ToolOutput` par `spawn_and_collect`. N'ajoute pas d'entrée.\n\
             Une entrée EN MOINS : le site a été converti, retire son entrée \
             de `GH_SPAWN_ALLOWLIST`.",
            offender_lines.join("\n  ")
        );
    }

    /// AC2, contrôle négatif — une entrée **stale** ferait rougir le scan.
    ///
    /// C'est la propriété sur laquelle repose tout ce vol : l'entrée des trois
    /// sites de `builtin_handlers.rs` *devait* partir en même temps que les
    /// spawns, et rien d'autre que l'égalité ne l'exigeait. Le contrôle le
    /// mesure sur la carte **réelle** du scan plutôt que sur deux cartes
    /// fabriquées, et il montre au passage ce qu'une inclusion aurait laissé
    /// passer.
    #[test]
    fn mika2715_une_entree_stale_de_l_allowlist_ferait_rougir_le_scan() {
        let (observed, _) = observed_gh_spawn_map();

        // L'allowlist telle qu'elle était avant ce vol : les deux entrées
        // restantes, plus les trois sites qui viennent d'être convertis.
        let mut stale: BTreeMap<String, usize> = GH_SPAWN_ALLOWLIST
            .iter()
            .map(|(p, n)| ((*p).to_string(), *n))
            .collect();
        stale.insert("skills/builtin_handlers.rs".to_string(), 3);

        assert_ne!(
            observed, stale,
            "mika#2715 — le scan doit distinguer l'allowlist courante de \
             l'allowlist d'avant la conversion. S'il ne le faisait pas, les \
             trois sites de `builtin_handlers.rs` auraient pu rester inscrits \
             après leur conversion, et l'allowlist aurait cessé de dire ce \
             qu'elle prétend dire."
        );
        assert!(
            observed.iter().all(|(k, v)| stale.get(k) == Some(v)),
            "et une **inclusion** aurait été satisfaite par cette allowlist \
             périmée : c'est exactement pourquoi l'assertion du scan est une \
             égalité. Observé : {observed:?}"
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

    /// La variante `Pr` n'est pas gardée par le plancher — une PR n'est pas une
    /// issue, et c'est l'issue que l'incident a touchée. Pinné pour qu'une
    /// conversion future ne croie pas l'inverse.
    #[tokio::test]
    async fn mika2713_le_plancher_garde_l_issue_pas_la_pr() {
        let out = run_gh("t", GhSubject::Pr(2636), &["pr", "view", "2636"])
            .await
            .expect("le double réussit");
        assert!(out.status.success());
    }

    // -----------------------------------------------------------------------
    // mika#2715 vol 1 — la seconde entrée
    // -----------------------------------------------------------------------

    fn argv(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|s| (*s).to_string()).collect()
    }

    /// AC5 — le sujet se lit dans l'`argv`, et il se lit **court**.
    ///
    /// Le quatrième cas est le piège que la règle évite : `--milestone 13`
    /// donnerait `Issue(13)` à une lecture naïve, et le plancher ferait
    /// paniquer un test qui ne s'adresse à aucune issue.
    #[test]
    fn mika2715_le_sujet_se_lit_dans_l_argv() {
        assert_eq!(
            gh_subject_from_argv(&argv(&["issue", "view", "990123"])),
            GhSubject::Issue(990_123)
        );
        assert_eq!(
            gh_subject_from_argv(&argv(&["pr", "view", "https://github.com/o/r/pull/990124"])),
            GhSubject::Pr(990_124),
            "l'identifiant de PR passe par `normalize_pr_identifier`, pas par \
             un second normaliseur"
        );
        assert_eq!(
            gh_subject_from_argv(&argv(&["api", "/repos/o/r/issues/2636/comments"])),
            GhSubject::Repo,
            "un chemin d'API est classé `Repo` : concession assumée de ce vol, \
             le plancher ne s'arme pas sur une URL"
        );
        assert_eq!(
            gh_subject_from_argv(&argv(&["issue", "list", "--milestone", "13"])),
            GhSubject::Repo,
            "la lecture s'arrête au premier jeton en `-` : `13` est un numéro \
             de jalon, pas d'issue"
        );
        assert_eq!(
            gh_subject_from_argv(&argv(&["pr", "list"])),
            GhSubject::Repo
        );
        assert_eq!(gh_subject_from_argv(&[]), GhSubject::Repo);
    }

    /// AC4 — le plancher est armé sur la seconde entrée **aussi**.
    ///
    /// C'est la propriété que la délégation achète : il n'y a qu'un plancher,
    /// donc il ne peut pas être muet d'un côté.
    #[tokio::test]
    #[should_panic(expected = "2636")]
    async fn mika2715_la_seconde_entree_refuse_le_numero_de_l_incident() {
        let _ = run_gh_tool(
            Some("t"),
            GhSubject::Issue(2636),
            &argv(&["issue", "comment", "2636"]),
            None,
        )
        .await;
    }

    /// AC6 — **aucun vrai `gh` n'a répondu** par la seconde entrée non plus.
    ///
    /// Même argument que `mika2713_aucun_vrai_gh_ne_repond_sous_test` : la
    /// sous-commande n'existe pas, donc un vrai `gh` rendrait un code non nul,
    /// que `spawn_and_collect` préfixerait `Exit code:`.
    #[tokio::test]
    async fn mika2715_aucun_vrai_gh_ne_repond_par_la_seconde_entree() {
        let out = run_gh_tool(
            Some("t"),
            GhSubject::Repo,
            &argv(&["mika-2715-sous-commande-inexistante"]),
            Some("senara-solutions/mika"),
        )
        .await;

        assert!(
            !out.is_error,
            "le double ne peut pas échouer : {}",
            out.content
        );
        assert_eq!(
            out.content, "[]",
            "le `content` est celui du double, pas celui d'un vrai `gh`"
        );
        assert!(
            !crate::tool_execution::has_non_zero_exit_prefix(&out.content),
            "mika#2237 — la sortie du double ne doit pas se lire comme un \
             échec : `run_gh` l'utilise pour décider qu'une revue a atterri"
        );

        // Et le `--repo` est bien replié dans l'`argv` vu par la couture.
        let vu = recorded_calls()
            .into_iter()
            .find(|c| c.args.starts_with("mika-2715-sous-commande-inexistante"))
            .expect("la seconde entrée enregistre dans le MÊME journal");
        assert_eq!(
            vu.args,
            "mika-2715-sous-commande-inexistante --repo senara-solutions/mika"
        );
    }

    /// AC7 — le `stdout` canné est keyed par sujet, et les deux entrées lisent
    /// la **même** table. Un test qui pose un override le voit par les deux.
    #[tokio::test]
    async fn mika2715_le_stdout_canne_est_partage_entre_les_deux_entrees() {
        let n = FIXTURE_ISSUE_FLOOR + 204;
        set_canned_stdout(GhSubject::Pr(n), b"{\"isDraft\":true}".to_vec());

        let par_la_seconde = run_gh_tool(
            Some("t"),
            GhSubject::Pr(n),
            &argv(&["pr", "view", &n.to_string()]),
            None,
        )
        .await;
        assert_eq!(par_la_seconde.content, "{\"isDraft\":true}");

        let par_la_premiere = run_gh("t", GhSubject::Pr(n), &["pr", "view"])
            .await
            .expect("double");
        assert_eq!(
            par_la_premiere.stdout, b"{\"isDraft\":true}",
            "une seule table d'overrides, lue par les deux entrées"
        );
    }

    /// AC3 — la moitié production **délègue** la lecture, elle ne la réécrit
    /// pas.
    ///
    /// Elle ne peut pas être exécutée ici (`#[cfg(not(test))]`), donc la
    /// propriété se tient sur la source de ce fichier. Ce qu'on empêche est
    /// précis : une seconde boucle de lecture, qui perdrait la borne
    /// `MAX_OUTPUT_LEN` (mika#1746) ou le préfixe `Exit code:` (mika#2237).
    #[test]
    fn mika2715_la_moitie_production_delegue_a_spawn_and_collect() {
        const SOURCE: &str = include_str!("gh_process.rs");
        let marker = "#[cfg(not(test))]\npub(crate) async fn run_gh_tool(";
        let start = SOURCE
            .find(marker)
            .expect("la moitié production de `run_gh_tool` doit exister ici");
        let body = &SOURCE[start..];
        let end = body
            .find("\n}\n")
            .expect("la moitié production de `run_gh_tool` doit se fermer");
        let body = &body[..end];

        assert!(
            body.contains("spawn_and_collect(cmd,"),
            "la moitié production doit confier la `Command` à \
             `builtin_handlers::spawn_and_collect`. Vu :\n{body}"
        );
        for reimplemente in ["cmd.spawn()", ".output().await", "MAX_OUTPUT_LEN", ".take("] {
            assert!(
                !body.contains(reimplemente),
                "la moitié production réimplémente la lecture (`{reimplemente}`) \
                 au lieu de la déléguer : c'est la borne de mika#1746 qui se \
                 perdrait. Vu :\n{body}"
            );
        }

        // L'ordre de #380 : le jeton se réinjecte APRÈS le scrub qui le retire.
        let scrub = body
            .find("scrub_mika_env_vars")
            .expect("le scrub des `MIKA_*` doit y être");
        let jeton = body
            .find("cmd.env(\"GH_TOKEN\"")
            .expect("la réinjection du jeton doit y être");
        assert!(
            scrub < jeton,
            "#380 — `GH_TOKEN` se pose APRÈS `scrub_mika_env_vars`, qui le \
             retire en défense en profondeur. Inverser l'ordre rend le jeton \
             absent du processus `gh`."
        );
    }
}
