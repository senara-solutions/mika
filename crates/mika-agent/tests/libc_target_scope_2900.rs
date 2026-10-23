//! La déclaration de `libc` doit atteindre les cibles darwin (mika#2900).
//!
//! `crates/mika-agent/Cargo.toml` déclarait `libc` sous
//! `[target.'cfg(target_os = "linux")'.dependencies]` pendant qu'un site du crate
//! l'appelait **hors de tout `#[cfg]`** : le `Drop` de `ProcessGroupKillGuard`
//! (`src/skills/executor.rs`, posé par 70127bbf / mika#2438, 2026-09-22). Sous
//! Linux la table de cible fournit le crate et le code résout ; pour une cible
//! darwin la table ne s'applique pas, `libc` n'entre pas dans le graphe, et les
//! trois chemins d'accès ne résolvent plus :
//!
//! ```text
//! error[E0433]: failed to resolve: use of unresolved module or unlinked crate `libc`
//!     --> crates/mika-agent/src/skills/executor.rs:1430:27
//!     --> crates/mika-agent/src/skills/executor.rs:1444:39
//! error: could not compile `mika-agent` (lib) due to 3 previous errors
//! ```
//!
//! **Pourquoi aucun test existant ne pouvait le voir.** Une table de dépendances
//! par cible est un contrat que **seule une compilation pour cette cible évalue**.
//! Aucun job de `ci.yml` ne compile pour darwin — le job `Check` tourne sur
//! `[self-hosted, Linux, X64, gentux]`, les autres sur `ubuntu-22.04` — donc le
//! premier lecteur du contrat a été un tag de release : le candidat `v1.0.0-rc.1`
//! (run `release.yml` 39961929051, 2026-10-23), dont les deux builds macOS sortent
//! avec le code 101. Le défaut est entré le 2026-09-22 et a été vu le 2026-10-23.
//!
//! Ce test est le substitut hors ligne : il ne compile rien, il lit le manifeste.
//! Il tient la propriété que la matrice de **release** — et non celle de CI —
//! définit l'ensemble des cibles qu'une clé de table doit couvrir.

use std::path::{Path, PathBuf};

/// La clé de table attendue. `cfg(unix)` ne sur-approxime pas : `mika-agent`
/// n'est compilable sur aucune cible non-unix aujourd'hui (`executor.rs` appelle
/// `.process_group(0)` sans `#[cfg]`, et cette méthode vient de
/// `std::os::unix::process::CommandExt`). Il n'existe donc pas de cible où
/// `cfg(unix)` déclarerait `libc` dans le vide.
const EXPECTED_KEY: &str = "cfg(unix)";

/// Le seul site du crate qui appelle `libc` **sans garde de cible**, et donc le
/// seul qui rend [`EXPECTED_KEY`] nécessaire. Les deux autres consommateurs sont
/// bornés et portent un repli explicite : `task_engine/process_liveness.rs`
/// (`libc::kill`, repli `false`) et `worktree_reaper.rs` (`libc::flock`, repli
/// `Unevaluable`).
const UNGUARDED_CALL: &str = "libc::killpg";

fn crate_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read(relative: &str) -> String {
    let path = crate_path(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("lecture de {}: {e}", path.display()))
}

/// Les clés de `[target.*]` dont la section `dependencies` nomme `libc`.
fn target_keys_declaring_libc(manifest: &str) -> Vec<String> {
    let parsed: toml::Table = toml::from_str(manifest).expect("Cargo.toml doit parser");
    let Some(targets) = parsed.get("target").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    targets
        .iter()
        .filter(|(_, table)| {
            table
                .get("dependencies")
                .and_then(toml::Value::as_table)
                .is_some_and(|deps| deps.contains_key("libc"))
        })
        .map(|(key, _)| key.clone())
        .collect()
}

/// A1 — la garde. `libc` est déclaré pour exactement une clé de cible, et c'est
/// `cfg(unix)`.
///
/// La clé est comparée **exactement**, et non évaluée : évaluer
/// `cfg(any(target_os = "linux", target_os = "macos"))` contre les quatre triples
/// de `release.yml` demanderait un mini-évaluateur de cfg dans un test, soit plus
/// de code que ce qu'il garde. Le prix est un faux rouge sur une reformulation
/// délibérée et équivalente ; il est payé par le message ci-dessous, qui dit quoi
/// faire.
#[test]
fn mika2900_libc_est_declare_pour_cfg_unix() {
    let keys = target_keys_declaring_libc(&read("Cargo.toml"));

    assert_eq!(
        keys,
        vec![EXPECTED_KEY.to_string()],
        "La déclaration de `libc` dans crates/mika-agent/Cargo.toml doit être une table de \
         cible unique, de clé `{EXPECTED_KEY}`. Clés trouvées : {keys:?}\n\
         \n\
         mika#2900 : avec la clé `cfg(target_os = \"linux\")`, `libc` n'entre pas dans le graphe \
         de dépendances d'une cible darwin, et `src/skills/executor.rs` — qui appelle \
         `{UNGUARDED_CALL}`, `libc::SIGKILL` et `libc::ESRCH` hors de tout `#[cfg]` — ne résout \
         plus (E0433). `release.yml` construit `aarch64-apple-darwin` et `x86_64-apple-darwin` \
         en plus des deux cibles Linux et ne publie que si les quatre passent : le candidat \
         `v1.0.0-rc.1` (run 39961929051) est sorti avec le code 101 sur les deux builds macOS.\n\
         \n\
         Une table de dépendances par cible n'est évaluée que par une compilation POUR cette \
         cible. C'est donc la matrice de `release.yml`, et non celle de `ci.yml`, qui définit \
         l'ensemble que la clé doit couvrir.\n\
         \n\
         Si une autre clé couvre réellement les quatre cibles — par exemple \
         `cfg(any(target_os = \"linux\", target_os = \"macos\"))` — élargissez l'ensemble accepté \
         par ce test en le justifiant. Ne supprimez pas l'assertion : c'est le seul lecteur \
         hors ligne du contrat, et sans elle le prochain lecteur sera un tag."
    );
}

/// A2 — l'auto-nettoyage. Le site non gardé existe encore.
///
/// A1 n'est nécessaire **que** parce qu'un site appelle `libc` sans garde de
/// cible. Si ce site disparaît, A1 contraint le manifeste pour rien et ce test
/// survit en contrainte morte. Il signale alors sa propre péremption au lieu de
/// rester vert.
#[test]
fn mika2900_le_site_non_garde_qui_rend_cfg_unix_necessaire_existe_encore() {
    let executor = read("src/skills/executor.rs");

    assert!(
        executor.contains(UNGUARDED_CALL),
        "`{UNGUARDED_CALL}` a disparu de crates/mika-agent/src/skills/executor.rs.\n\
         \n\
         C'était le seul appel à `libc` du crate hors de toute garde de cible, et donc la seule \
         raison pour laquelle `libc` doit être déclaré pour `{EXPECTED_KEY}` plutôt que pour \
         `cfg(target_os = \"linux\")` (mika#2900). Les deux autres consommateurs — \
         `task_engine/process_liveness.rs` et `worktree_reaper.rs` — sont bornés par \
         `#[cfg(target_os = \"linux\")]` et portent un repli explicite.\n\
         \n\
         Si le site a été déplacé ou renommé, mettez à jour cette constante. S'il a été borné à \
         Linux ou supprimé, réévaluez mika#2900 avant de retirer ce fichier : borner la garde de \
         groupe de processus laisserait un groupe orphelin sur macOS, la plateforme où le pilote \
         n'a justement pas de superviseur OpenRC."
    );
}
