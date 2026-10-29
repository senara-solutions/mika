//! Garde structurelle : aucune troisième liste de dépôts sur le chemin de
//! l'évaluateur (mika#2921, AC3).
//!
//! Le périmètre jugé par l'évaluateur est
//! [`crate::webhook_dispatch::DISPATCHABLE_REPOS`], réutilisée et non retapée :
//! le dépôt porte déjà deux listes qui ne disent pas la même chose (celle-ci et
//! `INTERNAL_REPOS` dans `mika-gateway`), et une troisième serait la divergence
//! programmée que ce dépôt a déjà payée deux fois (mika#2092,
//! `DISPATCHABLE_REPOS` ↔ `labels.yml`). Le plan l'a rendue structurelle : le
//! trait `FlightSource` ne prend plus de périmètre, et **un seul fichier** lit
//! la liste, `flight_source.rs`, hors de ce module. Cette garde épingle un état
//! que le code rend déjà vrai.
//!
//! # Ce qu'elle refuse, et où
//!
//! Sur la moitié production de `src/milestone_manager/**` et de
//! `src/flight_source.rs`, commentaires retirés :
//!
//! - **un littéral de dépôt**, `senara-solutions/`, partout, y compris dans le
//!   lecteur unique : la liste vit dans `webhook_dispatch.rs` et nulle part
//!   ailleurs ;
//! - **dans le module du manager**, toute mention d'un identifiant `_REPOS` :
//!   le module ne déclare pas de liste, n'en importe pas, et n'en lit pas —
//!   lire `DISPATCHABLE_REPOS` depuis `cadence.rs` ferait un second lecteur ;
//! - **dans le lecteur unique**, un `_REPOS` sur une ligne de déclaration ou
//!   d'import (`use`, `const`, `static`). Les lectures de la liste importée y
//!   sont son travail ; une seconde liste ou un second import ne le sont pas.
//!
//! **L'import de `DISPATCHABLE_REPOS` est exempté par forme, pas par fichier.**
//! La ligne acceptée est exactement [`IMPORT_DU_LECTEUR_UNIQUE`] ; un import
//! groupé, un autre chemin ou un littéral écrit dans le même fichier restent
//! refusés. Et l'exemption expire avec son site : si l'import disparaît ou
//! change de forme, `mika2921_lexemption_par_forme_couvre_un_seul_import_reel`
//! rougit plutôt que de laisser une dérogation sans objet (motif mika#2309).
//!
//! # La classification test, et sa prémisse
//!
//! Le code de test est écarté par [`crate::source_scan::production_files`] :
//! par le **chemin** ([`crate::source_scan::is_test_source_path`]), puis par la
//! troncature au `#[cfg(test)] mod tests` inline. La troncature est aveugle à
//! un module de test **extrait** (prémisse mika#2321) — et ce module en porte
//! deux, `no_dispatch_test.rs` et ce fichier, dont aucun n'a un chemin de test.
//! Ils sont donc écartés par la **déclaration** qui les rend tests :
//! `#[cfg(test)] mod <nom>;` dans `mod.rs`, lue dans le fichier plutôt que
//! recopiée ici. Si cette garde rougit après un déplacement de code de test,
//! **réparer la classification — jamais élargir l'allowlist, jamais élargir le
//! motif.**

use std::path::Path;

use crate::source_scan::{production_files, strip_comment_lines};

/// Le préfixe de tout dépôt sous contrôle : un littéral qui le porte est un
/// élément de liste de dépôts, quelle que soit la forme de la liste.
const MOTIF_LITTERAL: &str = "senara-solutions/";

/// Le suffixe que portent les deux listes du dépôt, `DISPATCHABLE_REPOS` et
/// `INTERNAL_REPOS`, et qu'une troisième porterait par imitation.
const MOTIF_LISTE: &str = "_REPOS";

/// Le seul fichier du chemin de l'évaluateur qui lit la liste.
const LECTEUR_UNIQUE: &str = "flight_source.rs";

/// La forme exacte de l'import exempté — une ligne, pas un fichier.
const IMPORT_DU_LECTEUR_UNIQUE: &str = "use crate::webhook_dispatch::DISPATCHABLE_REPOS;";

/// Exceptions nommées, par **(fichier, motif, raison, ticket de suivi)** —
/// jamais par fichier seul, pour qu'une exception ne couvre pas le littéral
/// suivant sans que personne le décide.
///
/// **Livrée vide**, et `mika2921_lallowlist_est_livree_vide` le tient : le jour
/// où une entrée y entre, ce test rougit et pose la question — « pourquoi une
/// liste de dépôts dans ce module ? » — au lieu de la laisser passer comme une
/// donnée de configuration. Si l'entrée est légitime, c'est cette assertion
/// qu'on modifie, délibérément, avec son ticket.
const REPO_LITERAL_ALLOWED: &[(&str, &str, &str, &str)] = &[];

/// Ce qu'un fichier a le droit de savoir des dépôts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Portee {
    /// Un fichier de `milestone_manager/` : rien.
    Module,
    /// `flight_source.rs` : lire la liste importée, sans en déclarer une autre.
    LecteurUnique,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Violation {
    fichier: String,
    motif: &'static str,
    ligne: String,
}

/// Le résultat d'un scan, et de quoi distinguer « rien trouvé » de « rien
/// regardé » (classe mika#2205).
#[derive(Debug, Default)]
struct Scan {
    violations: Vec<Violation>,
    lignes_scannees: usize,
    imports_exemptes: usize,
}

/// Le cœur de la garde, sur une source **littérale** : c'est ce qui permet au
/// contrôle de bonne foi de lui donner une liste fabriquée et de vérifier
/// qu'il la voit. `production` est déjà la moitié production du fichier.
fn scanner(
    fichier: &str,
    portee: Portee,
    production: &str,
    allowed: &[(&str, &str, &str, &str)],
) -> Scan {
    let mut scan = Scan::default();
    for ligne in strip_comment_lines(production).lines() {
        let t = ligne.trim();
        if t.is_empty() {
            continue;
        }
        scan.lignes_scannees += 1;
        if portee == Portee::LecteurUnique && t == IMPORT_DU_LECTEUR_UNIQUE {
            scan.imports_exemptes += 1;
            continue;
        }
        let mut motifs = Vec::new();
        if t.contains(MOTIF_LITTERAL) {
            motifs.push(MOTIF_LITTERAL);
        }
        if t.contains(MOTIF_LISTE) {
            let refuse = match portee {
                Portee::Module => true,
                Portee::LecteurUnique => est_declaration_ou_import(t),
            };
            if refuse {
                motifs.push(MOTIF_LISTE);
            }
        }
        for motif in motifs {
            if allowed
                .iter()
                .any(|(f, m, _, _)| *f == fichier && *m == motif)
            {
                continue;
            }
            scan.violations.push(Violation {
                fichier: fichier.to_string(),
                motif,
                ligne: t.to_string(),
            });
        }
    }
    scan
}

/// Une ligne qui déclare ou importe un item, plutôt qu'elle ne le lit.
fn est_declaration_ou_import(t: &str) -> bool {
    let sans_visibilite = t
        .strip_prefix("pub(crate) ")
        .or_else(|| t.strip_prefix("pub(super) "))
        .or_else(|| t.strip_prefix("pub "))
        .unwrap_or(t);
    ["use ", "const ", "static "]
        .iter()
        .any(|p| sans_visibilite.starts_with(p))
}

/// Les modules de test **extraits** déclarés par `mod_rs` : un
/// `#[cfg(test)]` immédiatement suivi de `mod <nom>;`. Lus dans le fichier qui
/// les déclare, parce que c'est cette déclaration — et non leur chemin — qui en
/// fait du code de test.
///
/// `pub(super)` depuis mika#2958 : le `mod tests` de `mod.rs` a besoin de la
/// **même** classification pour sa moitié « un seul déclarant de
/// l'interrupteur », et deux lectures de « ce fichier est-il du code de test ? »
/// divergeraient le jour où un troisième garde extrait arrive (doctrine du
/// lecteur unique, mika#2201).
pub(super) fn modules_de_test_extraits(mod_rs: &str) -> Vec<String> {
    let lignes: Vec<&str> = mod_rs.lines().map(str::trim).collect();
    lignes
        .windows(2)
        .filter(|w| w[0] == "#[cfg(test)]")
        .filter_map(|w| {
            let nom = w[1].strip_prefix("mod ")?.strip_suffix(';')?;
            Some(format!("{nom}.rs"))
        })
        .collect()
}

/// Le périmètre réel, chaque fichier avec sa portée et sa moitié production.
fn perimetre_reel() -> Vec<(String, Portee, String)> {
    let mod_rs = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/milestone_manager/mod.rs"),
    )
    .expect("milestone_manager/mod.rs doit être lisible");
    let extraits = modules_de_test_extraits(&mod_rs);

    production_files()
        .into_iter()
        .filter_map(|(chemin, production)| {
            if chemin == LECTEUR_UNIQUE {
                return Some((chemin, Portee::LecteurUnique, production));
            }
            let nom = chemin.strip_prefix("milestone_manager/")?;
            if extraits.iter().any(|e| e == nom) {
                return None;
            }
            Some((chemin, Portee::Module, production))
        })
        .collect()
}

fn scanner_le_perimetre_reel() -> (Vec<(String, Scan)>, Vec<Violation>) {
    let scans: Vec<(String, Scan)> = perimetre_reel()
        .into_iter()
        .map(|(chemin, portee, production)| {
            let nom = chemin.rsplit('/').next().unwrap_or(&chemin).to_string();
            let scan = scanner(&nom, portee, &production, REPO_LITERAL_ALLOWED);
            (chemin, scan)
        })
        .collect();
    let violations = scans
        .iter()
        .flat_map(|(_, s)| s.violations.iter().cloned())
        .collect();
    (scans, violations)
}

/// **AC3 — aucune troisième liste de dépôts.** Le module du manager ne nomme
/// aucun dépôt et aucune liste, et le lecteur unique n'en déclare pas une
/// seconde.
#[test]
fn mika2921_aucune_troisieme_liste_de_depots_sur_le_chemin_de_levaluateur() {
    let (_, violations) = scanner_le_perimetre_reel();
    assert!(
        violations.is_empty(),
        "une liste de dépôts est apparue sur le chemin de l'évaluateur :\n{}\n\n\
         Le périmètre est `DISPATCHABLE_REPOS`, lue dans `flight_source.rs` et nulle part \
         ailleurs. Un dépôt de plus est une ligne dans cette liste — qui ouvre aussi la porte \
         de dispatch, donc un geste d'opérateur. Si la violation vient d'un code de test \
         déplacé : réparer la classification, jamais élargir l'allowlist ni le motif.",
        violations
            .iter()
            .map(|v| format!("  {} — `{}` : {}", v.fichier, v.motif, v.ligne))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// L'allowlist est livrée **vide**, et ce test est ce qui force la question le
/// jour où elle ne l'est plus.
#[test]
fn mika2921_lallowlist_est_livree_vide() {
    assert!(
        REPO_LITERAL_ALLOWED.is_empty(),
        "une exception est entrée dans REPO_LITERAL_ALLOWED : {REPO_LITERAL_ALLOWED:?}. \
         Pourquoi une liste de dépôts dans ce module ? Si c'est légitime, modifiez cette \
         assertion délibérément, avec le ticket qui le décide."
    );
}

/// **Le garde-fou anti-vacuité.** Le scan a regardé l'arbre réel : assez de
/// fichiers et de lignes, le lecteur unique parmi eux, et les deux modules de
/// test extraits classés comme tels. Une racine qui dérive, une classification
/// trop large, et le scan rend zéro violation pour la mauvaise raison — « zéro
/// trouvé » et « rien regardé » rendraient les mêmes octets (mika#2205).
///
/// Les planchers sont posés sous la mesure du 2026-10-26 (16 fichiers, environ
/// 4 300 lignes non vides hors commentaires), avec la marge d'un refactor
/// ordinaire ; un module réduit d'un tiers est un événement à regarder.
#[test]
fn mika2921_le_scan_a_regarde_larbre_reel() {
    let (scans, _) = scanner_le_perimetre_reel();
    let fichiers = scans.len();
    let lignes: usize = scans.iter().map(|(_, s)| s.lignes_scannees).sum();
    assert!(
        fichiers >= 12 && lignes >= 3_000,
        "le scan n'a regardé que {fichiers} fichiers et {lignes} lignes — la racine ou la \
         classification a dérivé : {:?}",
        scans.iter().map(|(c, _)| c).collect::<Vec<_>>()
    );
    assert!(
        scans.iter().any(|(c, _)| c == LECTEUR_UNIQUE),
        "le lecteur unique n'est pas dans le périmètre scanné"
    );
    for module_de_test in ["no_dispatch_test.rs", "no_repo_list_test.rs"] {
        assert!(
            !scans
                .iter()
                .any(|(c, _)| c.ends_with(&format!("/{module_de_test}"))),
            "{module_de_test} est un module de test extrait et a été scanné comme production"
        );
    }

    let mod_rs = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/milestone_manager/mod.rs"),
    )
    .expect("milestone_manager/mod.rs doit être lisible");
    let extraits = modules_de_test_extraits(&mod_rs);
    assert!(
        extraits.contains(&"no_dispatch_test.rs".to_string())
            && extraits.contains(&"no_repo_list_test.rs".to_string()),
        "la lecture des déclarations `#[cfg(test)] mod …;` n'a pas trouvé les deux gardes : \
         {extraits:?}"
    );
}

/// **L'exemption par forme expire avec son site.** L'import exempté est vu
/// **une fois**, dans le lecteur unique : s'il disparaît ou change de forme, la
/// dérogation n'a plus d'objet et doit être retirée, pas laissée comme un trou
/// (motif mika#2309).
#[test]
fn mika2921_lexemption_par_forme_couvre_un_seul_import_reel() {
    let (scans, _) = scanner_le_perimetre_reel();
    let par_fichier: Vec<(&str, usize)> = scans
        .iter()
        .filter(|(_, s)| s.imports_exemptes > 0)
        .map(|(c, s)| (c.as_str(), s.imports_exemptes))
        .collect();
    assert_eq!(
        par_fichier,
        vec![(LECTEUR_UNIQUE, 1)],
        "l'import `{IMPORT_DU_LECTEUR_UNIQUE}` doit exister exactement une fois, dans \
         {LECTEUR_UNIQUE}"
    );
}

/// **Le contrôle de bonne foi.** Sur des sources fabriquées, le scan voit
/// chacune des formes qu'une troisième liste prendrait — et laisse passer ce
/// qui n'en est pas une. Un détecteur qui démarre vert n'a jamais montré qu'il
/// regarde.
#[test]
fn mika2921_le_scan_rougit_sur_une_troisieme_liste() {
    let motifs = |portee, source: &str| -> Vec<&'static str> {
        scanner("cadence.rs", portee, source, &[])
            .violations
            .into_iter()
            .map(|v| v.motif)
            .collect()
    };

    // Dans le module : un littéral, une liste déclarée, une lecture de la liste
    // existante — trois refus.
    assert_eq!(
        motifs(
            Portee::Module,
            "const DEPOTS: &[&str] = &[\"senara-solutions/mika\"];"
        ),
        vec![MOTIF_LITTERAL]
    );
    assert_eq!(
        motifs(
            Portee::Module,
            "pub(crate) const MANAGER_REPOS: &[&str] = &[];"
        ),
        vec![MOTIF_LISTE]
    );
    assert_eq!(
        motifs(
            Portee::Module,
            "let repos = crate::webhook_dispatch::DISPATCHABLE_REPOS.to_vec();"
        ),
        vec![MOTIF_LISTE],
        "un second lecteur de la liste, dans le module, est refusé"
    );

    // Dans le lecteur unique : l'import exact et ses lectures passent…
    let sain = format!(
        "{IMPORT_DU_LECTEUR_UNIQUE}\nfn controlled_repos() -> Vec<String> {{\n    \
         DISPATCHABLE_REPOS.iter().map(|r| r.to_string()).collect()\n}}"
    );
    let scan = scanner(LECTEUR_UNIQUE, Portee::LecteurUnique, &sain, &[]);
    assert!(scan.violations.is_empty(), "{:?}", scan.violations);
    assert_eq!(scan.imports_exemptes, 1);

    // …mais l'exemption est une forme, pas un fichier.
    assert_eq!(
        motifs(
            Portee::LecteurUnique,
            "use crate::webhook_dispatch::{DISPATCHABLE_REPOS, READY_LABEL};"
        ),
        vec![MOTIF_LISTE],
        "un import groupé n'est pas la forme exemptée"
    );
    assert_eq!(
        motifs(Portee::LecteurUnique, "static AUTRES_REPOS: &[&str] = &[];"),
        vec![MOTIF_LISTE]
    );
    assert_eq!(
        motifs(
            Portee::LecteurUnique,
            "let r = \"https://github.com/senara-solutions/wizzard\";"
        ),
        vec![MOTIF_LITTERAL],
        "un littéral dans le lecteur unique reste refusé"
    );

    // Ce qui n'est pas une liste : la prose.
    assert!(
        motifs(
            Portee::Module,
            "/// le périmètre est `DISPATCHABLE_REPOS` (senara-solutions/mika…)"
        )
        .is_empty(),
        "un commentaire qui nomme la liste n'est pas une liste"
    );
}

/// Une exception, quand elle existera, porte sur un couple `(fichier, motif)` :
/// elle ne couvre ni l'autre motif du même fichier, ni le même motif ailleurs.
#[test]
fn mika2921_une_exception_ne_couvre_que_son_fichier_et_son_motif() {
    let exceptions = [(
        "reporter.rs",
        MOTIF_LITTERAL,
        "exemple fabriqué",
        "mika#0000",
    )];
    let source = "const A: &str = \"senara-solutions/mika\";\nconst B_REPOS: &[&str] = &[];";

    let ici = scanner("reporter.rs", Portee::Module, source, &exceptions);
    assert_eq!(
        ici.violations.iter().map(|v| v.motif).collect::<Vec<_>>(),
        vec![MOTIF_LISTE],
        "l'exception couvre le littéral, pas la liste déclarée du même fichier"
    );

    let ailleurs = scanner("cadence.rs", Portee::Module, source, &exceptions);
    assert_eq!(
        ailleurs.violations.len(),
        2,
        "l'exception de reporter.rs ne couvre rien dans cadence.rs : {:?}",
        ailleurs.violations
    );
}
