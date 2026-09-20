//! Le puits hors-ligne : **un** résolveur de chemin, et son lecteur (mika#2267).
//!
//! **LECTURE SEULE.** Ce module compose un `PathBuf` et lit un répertoire. Il
//! n'écrit rien, n'appelle pas `gh`, n'ajoute aucune autorité d'écriture.
//!
//! ## Le défaut que ça ferme
//!
//! Avant mika#2267, `write_offline_sink` (`cadence.rs`) déposait un `.md`
//! horodaté dans un répertoire **que rien ne listait, rien n'exposait et aucune
//! procédure ne nommait** : recherche exhaustive sur l'arbre, `offline_sink` /
//! `OFFLINE_SINK` n'apparaissait qu'à l'écriture, à la config et dans la prose.
//! Zéro commande, zéro outil, zéro route, zéro consommateur. Du point de vue du
//! lecteur humain — celui qui doit rendre le verdict de fidélité — **« le canal
//! est cassé » et « le canal est un puits » produisent exactement les mêmes
//! octets : aucun.** C'est la classe que la maison a déjà dû nommer (mika#2205 :
//! un scan silencieusement inactif se lit exactement comme un scan qui n'a rien
//! trouvé à faire).
//!
//! ## Pourquoi UN résolveur, et pourquoi un module à lui
//!
//! Le chemin était composé en ligne dans `manager_config_from_env`. Un lecteur
//! CLI qui le recomposerait de son côté pourrait diverger de l'écrivain — et
//! **un lecteur qui regarde ailleurs que l'écrivain est précisément le défaut
//! qu'on ferme**, reproduit une couche plus haut. L'écrivain (`cadence`, via la
//! config que `spawn` assemble) et le lecteur (`mika milestone reports`) passent
//! tous deux par [`resolve_offline_sink_dir`].
//!
//! Le module existe pour que la garde structurelle
//! [`tests::mika2267_sink_dir_resolution_has_a_single_reader`] ait une cible
//! nette : **aucun autre fichier `.rs` de production** ne peut porter le nom de
//! la variable d'environnement ni recomposer `…/sink`. L'allowlist est livrée
//! **vide** : quand la garde tire, on retire le second site, on ne l'allowliste
//! pas.

use std::path::{Path, PathBuf};

/// Nom de la variable d'environnement qui déplace le puits hors-ligne.
///
/// **Unique site de ce littéral dans l'arbre de production** — tout autre
/// lecteur passe par cette constante ou par [`resolve_offline_sink_dir`].
pub const ENV_OFFLINE_SINK_DIR: &str = "MIKA_MANAGER_OFFLINE_SINK_DIR";

/// Racine de repli des répertoires d'état quand ni la variable ni `HOME` ne
/// sont posées (dernier recours ; ne devrait pas être atteint en production).
const FALLBACK_STATE_ROOT: &str = "/tmp/mika-manager";

/// Par quelle porte le chemin du puits a été décidé.
///
/// Rapporté sur `manager_delivery_resolved` et par `mika milestone reports`.
/// Motif `llm_budget_resolved` (mika#2293) : *un réglage qu'on ne peut pas
/// observer n'est pas un réglage, c'est un espoir.* Les deux valeurs appellent
/// des remèdes opposés — `Env` dit « le chemin est celui que vous avez posé,
/// cherchez ailleurs », `Default` dit « rien ne l'a déplacé ».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkDirSource {
    /// La variable d'environnement était posée et non vide.
    Env,
    /// Aucune variable : `$HOME/.mika/manager/sink`, ou le repli `/tmp`.
    Default,
}

impl SinkDirSource {
    /// Le mot posé sur le fil (journal, JSON de la CLI). Format de fil.
    pub fn as_str(self) -> &'static str {
        match self {
            SinkDirSource::Env => "env",
            SinkDirSource::Default => "default",
        }
    }
}

/// **LE** résolveur du répertoire du puits hors-ligne.
///
/// L'écrivain et le lecteur passent tous deux par ici, donc « un lecteur qui
/// regarde ailleurs que l'écrivain » n'est pas exprimable.
///
/// Convention maison : une variable posée à la chaîne vide vaut « non posée ».
pub fn resolve_offline_sink_dir() -> (PathBuf, SinkDirSource) {
    match std::env::var(ENV_OFFLINE_SINK_DIR) {
        Ok(raw) if !raw.trim().is_empty() => (PathBuf::from(raw.trim()), SinkDirSource::Env),
        _ => (default_state_root().join("sink"), SinkDirSource::Default),
    }
}

/// Racine de repli partagée par le puits et les checkpoints.
///
/// `pub(super)` : `spawn::manager_config_from_env` s'en sert encore pour le
/// répertoire de checkpoints, qui n'est pas le sujet de mika#2267 et garde sa
/// composition en ligne.
pub(super) fn default_state_root() -> PathBuf {
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        return PathBuf::from(home.trim()).join(".mika").join("manager");
    }
    PathBuf::from(FALLBACK_STATE_ROOT)
}

/// Un rapport trouvé dans le puits.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SinkEntry {
    /// Nom du fichier, tel qu'il est sur le disque.
    pub file_name: String,
    /// Chemin absolu — ce qu'un opérateur peut `cat`.
    pub path: PathBuf,
    /// Le slug de milestone porté par le nom de fichier, quand il est lisible.
    pub milestone_slug: Option<String>,
    /// L'horodatage porté par le nom de fichier (forme `write_offline_sink`,
    /// c'est-à-dire un RFC 3339 dont les `:` ont été remplacés par des `-`).
    /// `None` quand le nom ne suit pas la convention.
    pub timestamp: Option<String>,
    /// Taille du fichier en octets.
    pub size_bytes: u64,
}

/// Ce que le puits a répondu.
///
/// **Trois états, et la distinction est la propriété porteuse du ticket.**
/// Un répertoire absent et un répertoire vide disent des choses opposées —
/// « la cadence n'a jamais écrit ICI » contre « la cadence a écrit ici et le
/// puits a été vidé » — et les rendre par une liste vide dans les deux cas
/// reproduirait à la surface du lecteur le silence qu'il existe pour lever.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkListing {
    /// Le répertoire n'existe pas (ou n'est pas lisible comme répertoire).
    DirAbsent { dir: PathBuf, source: SinkDirSource },
    /// Le répertoire existe et ne porte aucun rapport.
    Empty { dir: PathBuf, source: SinkDirSource },
    /// Le répertoire porte des rapports, du plus récent au plus ancien.
    Entries {
        dir: PathBuf,
        source: SinkDirSource,
        entries: Vec<SinkEntry>,
    },
}

impl SinkListing {
    /// Le chemin consulté — **toujours nommé**, y compris sur le chemin
    /// nominal. C'est ce qui rend lisible en une ligne le cas où la CLI
    /// (lancée par l'opérateur) et le démon (lancé par le service, sous un
    /// autre `HOME`) ne résolvent pas le même répertoire.
    pub fn dir(&self) -> &Path {
        match self {
            SinkListing::DirAbsent { dir, .. }
            | SinkListing::Empty { dir, .. }
            | SinkListing::Entries { dir, .. } => dir,
        }
    }

    /// Par quelle porte ce chemin a été décidé.
    pub fn source(&self) -> SinkDirSource {
        match self {
            SinkListing::DirAbsent { source, .. }
            | SinkListing::Empty { source, .. }
            | SinkListing::Entries { source, .. } => *source,
        }
    }

    /// Les rapports, du plus récent au plus ancien. Vide pour les deux autres
    /// états — un appelant qui n'a besoin que du contenu peut ignorer la
    /// distinction, un appelant qui doit la rendre lit la variante.
    pub fn entries(&self) -> &[SinkEntry] {
        match self {
            SinkListing::Entries { entries, .. } => entries,
            _ => &[],
        }
    }
}

/// Lister les rapports du puits, du plus récent au plus ancien.
///
/// `milestone_slug` restreint à une milestone (le slug de
/// [`crate::milestone_manager::types::MilestoneRef::slug`]) ; `None` = toutes.
///
/// **Le tri est lexicographique sur l'horodatage du nom de fichier**, et c'est
/// correct pour la même raison que partout ailleurs dans ce dépôt : l'ordre
/// lexicographique d'un ISO 8601 à largeur fixe est son ordre chronologique.
/// La substitution `:` → `-` que `write_offline_sink` applique est uniforme,
/// donc elle préserve l'ordre relatif entre deux noms de même forme. Un nom
/// qui ne suit pas la convention n'a pas d'horodatage lisible : il est trié
/// **après** les autres (clé vide) plutôt que d'être écarté — un fichier qu'on
/// ne sait pas dater reste un fichier que l'opérateur doit voir.
pub fn list_sink_reports(milestone_slug: Option<&str>) -> SinkListing {
    let (dir, source) = resolve_offline_sink_dir();
    list_sink_reports_in(&dir, source, milestone_slug)
}

/// Comme [`list_sink_reports`], sur un répertoire donné.
///
/// Séparé pour que les tests n'aient pas à muter l'environnement du process
/// (`std::env::set_var` est `unsafe` en édition 2024 et globalement partagé
/// entre tests parallèles).
pub fn list_sink_reports_in(
    dir: &Path,
    source: SinkDirSource,
    milestone_slug: Option<&str>,
) -> SinkListing {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => {
            return SinkListing::DirAbsent {
                dir: dir.to_path_buf(),
                source,
            };
        }
    };

    let mut entries: Vec<SinkEntry> = Vec::new();
    for item in read.flatten() {
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let stem = &file_name[..file_name.len() - 3]; // strip ".md"
        let (slug, timestamp) = split_stem(stem);

        if let Some(wanted) = milestone_slug {
            // Un nom illisible n'appartient à aucune milestone : il ne peut pas
            // satisfaire un filtre nominatif.
            if slug.as_deref() != Some(wanted) {
                continue;
            }
        }

        let size_bytes = item.metadata().map(|m| m.len()).unwrap_or(0);
        entries.push(SinkEntry {
            file_name: file_name.to_string(),
            path,
            milestone_slug: slug,
            timestamp,
            size_bytes,
        });
    }

    // Décroissant sur l'horodatage ; le nom de fichier départage à horodatage
    // égal pour que l'ordre soit total et reproductible.
    entries.sort_by(|a, b| {
        let ka = a.timestamp.as_deref().unwrap_or("");
        let kb = b.timestamp.as_deref().unwrap_or("");
        kb.cmp(ka).then_with(|| b.file_name.cmp(&a.file_name))
    });

    if entries.is_empty() {
        SinkListing::Empty {
            dir: dir.to_path_buf(),
            source,
        }
    } else {
        SinkListing::Entries {
            dir: dir.to_path_buf(),
            source,
            entries,
        }
    }
}

/// Couper `<slug>-<horodatage>` produit par `write_offline_sink`.
///
/// Le slug est `owner-repo-number` (il porte des tirets et des chiffres), donc
/// on ne peut pas couper sur le dernier `-`. L'horodatage, lui, commence
/// toujours par `-YYYY-MM-DDT` : on cherche cette forme, ce qui est un ancrage
/// que le slug ne peut pas imiter (un slug ne contient pas de `T` précédé de
/// huit chiffres et deux tirets).
fn split_stem(stem: &str) -> (Option<String>, Option<String>) {
    let bytes = stem.as_bytes();
    for (i, _) in stem.match_indices('-') {
        // On veut `-` puis `YYYY-MM-DDT` = 11 octets.
        let rest = &bytes[i + 1..];
        if rest.len() < 11 {
            break;
        }
        let looks_like_date = rest[0..4].iter().all(u8::is_ascii_digit)
            && rest[4] == b'-'
            && rest[5..7].iter().all(u8::is_ascii_digit)
            && rest[7] == b'-'
            && rest[8..10].iter().all(u8::is_ascii_digit)
            && rest[10] == b'T';
        if looks_like_date {
            let slug = &stem[..i];
            let ts = &stem[i + 1..];
            if slug.is_empty() {
                return (None, Some(ts.to_string()));
            }
            return (Some(slug.to_string()), Some(ts.to_string()));
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ---- C1 : le résolveur ------------------------------------------------

    #[test]
    fn the_default_lives_under_the_manager_state_root() {
        // Pas de mutation d'environnement : on éprouve la composition, qui est
        // la moitié que `resolve_offline_sink_dir` ajoute au-dessus du `var`.
        let root = default_state_root();
        assert!(
            root.ends_with("manager"),
            "racine inattendue: {}",
            root.display()
        );
        assert!(root.join("sink").ends_with("sink"));
    }

    #[test]
    fn the_two_sources_render_as_a_wire_format() {
        assert_eq!(SinkDirSource::Env.as_str(), "env");
        assert_eq!(SinkDirSource::Default.as_str(), "default");
    }

    // ---- C2 : le lecteur --------------------------------------------------

    fn write(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).unwrap();
    }

    /// **T1** — les entrées sortent du plus récent au plus ancien.
    #[test]
    fn mika2267_reports_lists_sink_entries_most_recent_first() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "senara-solutions-mika-1799-2026-08-21T12-00-00+00-00.md",
            "midi",
        );
        write(
            tmp.path(),
            "senara-solutions-mika-1799-2026-08-22T09-30-00+00-00.md",
            "lendemain",
        );
        write(
            tmp.path(),
            "senara-solutions-mika-1799-2026-08-20T23-59-59+00-00.md",
            "veille",
        );
        // Bruit : un fichier non-markdown ne doit pas entrer.
        write(tmp.path(), "notes.txt", "x");

        let listing = list_sink_reports_in(tmp.path(), SinkDirSource::Default, None);
        let names: Vec<&str> = listing
            .entries()
            .iter()
            .map(|e| e.file_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "senara-solutions-mika-1799-2026-08-22T09-30-00+00-00.md",
                "senara-solutions-mika-1799-2026-08-21T12-00-00+00-00.md",
                "senara-solutions-mika-1799-2026-08-20T23-59-59+00-00.md",
            ]
        );
        assert_eq!(
            listing.entries()[0].milestone_slug.as_deref(),
            Some("senara-solutions-mika-1799")
        );
        assert_eq!(
            listing.entries()[0].timestamp.as_deref(),
            Some("2026-08-22T09-30-00+00-00")
        );
    }

    /// **T2 — le test porteur.** Un répertoire absent et un répertoire vide ne
    /// se lisent pas pareil.
    ///
    /// Contrôle négatif intégré : l'assertion finale compare les deux
    /// variantes. Si un jour les deux branches sont fondues en une liste vide,
    /// ce test rougit — c'est sa seule raison d'exister.
    #[test]
    fn mika2267_reports_absent_dir_is_distinguishable_from_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let absent = tmp.path().join("il-nexiste-pas");
        let empty = tmp.path().join("vide");
        fs::create_dir_all(&empty).unwrap();

        let a = list_sink_reports_in(&absent, SinkDirSource::Default, None);
        let e = list_sink_reports_in(&empty, SinkDirSource::Env, None);

        assert!(matches!(a, SinkListing::DirAbsent { .. }), "{a:?}");
        assert!(matches!(e, SinkListing::Empty { .. }), "{e:?}");
        assert_ne!(
            std::mem::discriminant(&a),
            std::mem::discriminant(&e),
            "un puits absent et un puits vide doivent rester deux états distincts"
        );

        // Et les deux nomment le chemin consulté, y compris l'absent.
        assert_eq!(a.dir(), absent.as_path());
        assert_eq!(e.dir(), empty.as_path());
        assert_eq!(a.source(), SinkDirSource::Default);
        assert_eq!(e.source(), SinkDirSource::Env);
    }

    #[test]
    fn mika2267_target_filter_keeps_only_that_milestone() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "senara-solutions-mika-1799-2026-08-21T12-00-00+00-00.md",
            "a",
        );
        write(
            tmp.path(),
            "senara-solutions-mika-1800-2026-08-22T12-00-00+00-00.md",
            "b",
        );

        let listing = list_sink_reports_in(
            tmp.path(),
            SinkDirSource::Default,
            Some("senara-solutions-mika-1799"),
        );
        assert_eq!(listing.entries().len(), 1);
        assert_eq!(
            listing.entries()[0].milestone_slug.as_deref(),
            Some("senara-solutions-mika-1799")
        );

        // Filtre qui ne matche rien : le puits existe, il est « vide » POUR
        // cette milestone — et non « absent ».
        let none = list_sink_reports_in(tmp.path(), SinkDirSource::Default, Some("autre-repo-1"));
        assert!(matches!(none, SinkListing::Empty { .. }), "{none:?}");
    }

    /// Un nom hors convention reste visible (il est simplement non daté), et
    /// il est trié après les noms datés.
    #[test]
    fn mika2267_an_unconventional_name_is_listed_last_not_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "rapport-a-la-main.md", "x");
        write(
            tmp.path(),
            "senara-solutions-mika-1799-2026-08-21T12-00-00+00-00.md",
            "y",
        );

        let listing = list_sink_reports_in(tmp.path(), SinkDirSource::Default, None);
        assert_eq!(listing.entries().len(), 2);
        assert_eq!(
            listing.entries()[1].file_name,
            "rapport-a-la-main.md",
            "un fichier qu'on ne sait pas dater reste un fichier que l'opérateur doit voir"
        );
        assert!(listing.entries()[1].timestamp.is_none());
    }

    #[test]
    fn split_stem_anchors_on_the_date_not_on_the_last_dash() {
        assert_eq!(
            split_stem("senara-solutions-mika-1799-2026-08-21T12-00-00+00-00"),
            (
                Some("senara-solutions-mika-1799".to_string()),
                Some("2026-08-21T12-00-00+00-00".to_string())
            )
        );
        // Un slug seul n'a pas d'horodatage.
        assert_eq!(split_stem("senara-solutions-mika-1799"), (None, None));
        // Un nom arbitraire non plus.
        assert_eq!(split_stem("rapport-a-la-main"), (None, None));
    }

    // ---- T3 / T7 : les gardes structurelles -------------------------------

    fn crates_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/mika-agent a un parent")
            .to_path_buf()
    }

    /// La moitié « production » d'un fichier source.
    ///
    /// `None` quand le fichier **entier** est du code de test — la leçon de
    /// mika#2321 : un module de test extrait ne porte aucun littéral
    /// `#[cfg(test)]`, donc la troncature seule le scanne comme de la
    /// production.
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

    fn scanned_roots() -> Vec<PathBuf> {
        let roots = vec![
            crates_dir().join("mika-agent").join("src"),
            crates_dir().join("mika-cli").join("src"),
        ];
        for r in &roots {
            assert!(
                r.is_dir(),
                "racine de scan introuvable: {} — la garde ne couvre plus ce qu'elle prétend couvrir",
                r.display()
            );
        }
        roots
    }

    /// **T3** — le chemin du puits a un seul site de résolution.
    ///
    /// Deux aiguilles, et chacune ferme une moitié : le **littéral** du nom de
    /// la variable (un second lecteur qui la relirait de son côté) et la
    /// **recomposition** `…/sink` (un second lecteur qui refabriquerait le
    /// chemin par défaut). Les deux sont des façons de regarder ailleurs que
    /// l'écrivain.
    ///
    /// **Allowlist livrée vide** : ce module est le site de définition, pas une
    /// exemption. Quand la garde tire, on retire le second site.
    ///
    /// Un test comportemental ne peut pas attraper cette classe : un second
    /// résolveur ne rendrait aucune décision fausse le jour où il est écrit —
    /// il divergerait plus tard, en silence, et toutes les assertions
    /// resteraient vertes pendant que l'opérateur reperdrait ses rapports.
    #[test]
    fn mika2267_sink_dir_resolution_has_a_single_reader() {
        const NEEDLES: &[&str] = &[
            // Le nom de la variable, écrit à la main plutôt que lu par la
            // constante. Concaténé pour que ce test ne se dénonce pas lui-même.
            concat!("MIKA_MANAGER", "_OFFLINE_SINK_DIR"),
            // La recomposition du chemin par défaut.
            concat!(".join(", "\"sink\")"),
        ];
        // Le seul fichier autorisé à porter ces aiguilles : celui-ci.
        let owner = crates_dir()
            .join("mika-agent")
            .join("src")
            .join("milestone_manager")
            .join("sink_dir.rs");
        assert!(owner.is_file(), "{} introuvable", owner.display());

        let mut files = Vec::new();
        for root in scanned_roots() {
            walk_rs(&root, &mut files);
        }
        assert!(
            files.len() > 50,
            "scan suspicieusement court: {}",
            files.len()
        );

        let mut violations: Vec<String> = Vec::new();
        for path in files {
            if path == owner {
                continue;
            }
            let Ok(src) = fs::read_to_string(&path) else {
                continue;
            };
            let Some(prod) = production_half(&path, &src) else {
                continue;
            };
            for needle in NEEDLES {
                if prod.contains(needle) {
                    violations.push(format!("{}: {needle}", path.display()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "second site de résolution du puits hors-ligne — retirez-le, ne l'allowlistez pas \
             (mika#2267 C1) :\n  {}",
            violations.join("\n  ")
        );
    }

    /// **T7** — toute variable `MIKA_MANAGER_*` que le code lit est déclarée
    /// dans `.env.example`.
    ///
    /// Ferme durablement la classe que mika#2267 a trouvée : aucune variable
    /// `MIKA_MANAGER_*` n'était déclarée nulle part dans le dépôt, donc aucune
    /// revue n'avait jamais pu constater leur absence — ni leur mauvaise
    /// orthographe dans la prose (`MIKA_MANAGER_SINK_DIR`, qui n'existe pas).
    /// Corriger l'occurrence sans poser la garde laisserait la classe ouverte.
    #[test]
    fn mika2267_every_manager_env_const_is_declared_in_env_example() {
        let manager_dir = crates_dir()
            .join("mika-agent")
            .join("src")
            .join("milestone_manager");
        let mut files = Vec::new();
        walk_rs(&manager_dir, &mut files);

        // `pub const ENV_… : &str = "MIKA_…";`
        //
        // Scan the WHOLE file, not `production_half`: `spawn.rs` carries a
        // `#[cfg(test)]` helper above its const block, so truncating at the
        // first occurrence would hide nine of the ten declarations — and a
        // guard that finds nothing passes. Only test *files* are skipped.
        let mut declared: Vec<String> = Vec::new();
        for path in &files {
            if crate::source_scan::is_test_source_path(path) {
                continue;
            }
            let Ok(src) = fs::read_to_string(path) else {
                continue;
            };
            for line in src.lines() {
                let line = line.trim();
                if !line.starts_with("pub const ENV_") {
                    continue;
                }
                if let Some(open) = line.find('"')
                    && let Some(close) = line[open + 1..].find('"')
                {
                    let value = &line[open + 1..open + 1 + close];
                    if value.starts_with("MIKA_") {
                        declared.push(value.to_string());
                    }
                }
            }
        }
        declared.sort();
        declared.dedup();
        assert!(
            declared.len() >= 10,
            "le scan n'a trouvé que {declared:?} — il ne lit plus les constantes qu'il prétend lire"
        );

        let env_example = crates_dir()
            .parent()
            .expect("crates/ a un parent")
            .join(".env.example");
        let example = fs::read_to_string(&env_example)
            .unwrap_or_else(|e| panic!("{} illisible: {e}", env_example.display()));

        let missing: Vec<&String> = declared
            .iter()
            .filter(|v| !example.contains(v.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "variables lues par le code et absentes de {} — un canal sans déclaration versionnée \
             est un canal qu'aucune revue ne peut voir (mika#2267 C5) : {missing:?}",
            env_example.display()
        );
    }
}
