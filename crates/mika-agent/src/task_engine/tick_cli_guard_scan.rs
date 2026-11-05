//! mika#3042 AC3 — le scan qui refuse, dans `tick()`, un balayage écrivant un
//! état terminal sur une parente `self_dev` **hors** garde `cli_mode`.
//!
//! Pourquoi un scan de source et non un test de comportement : `mika chat`
//! construit son propre [`TaskEngine`](super::TaskEngine) (`cli_mode: true`)
//! contre la base qu'il **partage** avec le démon. Un balayage ajouté demain
//! dans l'échelle de réparation de `tick()` sans garde ne casse aucun test
//! existant — il fauche en production, avec le code d'un TUI ouvert avant le
//! deploy. [`mika3042_le_mode_cli_ne_fauche_pas_une_parente_self_dev`] épingle
//! les balayages d'aujourd'hui ; ce module épingle la **forme**, donc ceux de
//! demain.
//!
//! **Il atterrit armé, vert, et sans mécanisme d'exception.** Pas de liste de
//! dérogations : une dérogation serait une seconde entrée pour un défaut
//! identique, et c'est précisément ce que mika#2309 interdit. Un balayage
//! légitimement hors garde se justifie en déplaçant son prédicat (le cas de
//! `settle_dispatch_parents`, épinglé par
//! [`ac3_settle_dispatch_parents_reste_hors_predicat`]), pas en s'inscrivant
//! quelque part.
//!
//! Le scan porte sur le **texte du corps de `tick()`**, reçu en paramètre
//! plutôt que lu en dur : c'est ce qui rend le contrôle de bonne foi possible
//! ([`ac3_le_meme_scan_rougit_sur_une_copie_sans_la_garde`]).

/// Le texte de production que le scan lit. `include_str!` et non un chemin
/// calculé : le compilateur garantit que le fichier existe et qu'il est celui
/// d'à côté.
const ENGINE_SRC: &str = include_str!("engine.rs");

/// Le texte des requêtes, pour la frontière du prédicat (T5).
const TASKS_SRC: &str = include_str!("../db/tasks.rs");

/// La garde, littéralement telle qu'elle s'écrit aux cinq sites qui la portent
/// (`engine.rs` l. 1033, l. 1202, le bloc #264, et les deux blocs mika#3042).
const CLI_GUARD: &str = "if !self.dispatcher.cli_mode";

/// La signature dont [`tick_body`] extrait le corps.
const TICK_SIGNATURE: &str = "pub async fn tick(&mut self) {";

/// Le terme qui tient `settle_dispatch_parents` hors du prédicat de mika#3042.
const SETTLE_COMPLEMENT_PREDICATE: &str = "AND COALESCE(parent.source, '') != 'self_dev'";

/// Les balayages de `tick()` qui écrivent un état terminal sur une parente
/// `self_dev` — recensement **FERMÉ**.
///
/// **Cinq noms, six sites d'appel :** `reap_orphaned_parent_tasks` est appelé
/// une fois par classe de dispatch depuis mika#2750, et le scan rend une
/// violation **par site**, pas par nom. Un nom balayé deux fois sans garde est
/// deux fauchages, donc deux lignes à lire.
///
/// Chaque entrée nomme la requête qui porte son `parent.source = 'self_dev'` :
/// c'est la donnée qui fait de ce balayage un écrivain terminal de la
/// population que mika#3042 protège, et c'est elle qu'il faut relire avant de
/// retirer une entrée d'ici.
///
/// - `reap_orphaned_parent_tasks` → `find_orphaned_parent_tasks`
///   (`db/tasks.rs:2140`, prédicat l. 2155)
/// - `complete_parent_tasks_on_callback_success` →
///   `find_completable_parent_tasks_on_pr_url` (`db/tasks.rs:2587`, prédicat
///   l. 2601)
/// - `reap_childless_stuck_parent_tasks` → `find_childless_stuck_parent_tasks`
///   (`db/tasks.rs:2767`, prédicat l. 2778)
/// - `reap_orphaned_pending_issue_tasks` → `find_orphaned_pending_issue_tasks`
///   (`db/tasks.rs:2934`, prédicat l. 2955)
/// - `reap_stale_blocked_dispatch_tasks` → `find_stale_blocked_dispatch_tasks`
///   (`db/tasks.rs:3108`, prédicat l. 3128)
///
/// Hors liste, et ce n'est pas un oubli : `settle_dispatch_parents` porte le
/// **complément** de cette population
/// ([`SETTLE_COMPLEMENT_PREDICATE`], `db/tasks.rs:2720`), et
/// `report_promotion_starvation` n'écrit aucun état.
const TERMINAL_PARENT_SWEEPS: &[&str] = &[
    "reap_orphaned_parent_tasks",
    "complete_parent_tasks_on_callback_success",
    "reap_childless_stuck_parent_tasks",
    "reap_orphaned_pending_issue_tasks",
    "reap_stale_blocked_dispatch_tasks",
];

/// Un octet peut-il appartenir à un identifiant Rust ?
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `needle` apparaît-il dans `hay` ? Sur octets, pour que les index du texte
/// blanchi restent utilisables sans découpage de `&str`.
fn window_contains(hay: &[u8], needle: &str) -> bool {
    let n = needle.as_bytes();
    !n.is_empty() && hay.windows(n.len()).any(|w| w == n)
}

/// Remplace par des espaces tout ce qui n'est pas du code : contenu des
/// chaînes (y compris brutes), des littéraux de caractère, et des commentaires
/// de ligne comme de bloc.
///
/// **Même longueur en octets que l'entrée**, un espace par octet remplacé —
/// les commentaires de ce dépôt sont en français, donc multi-octets, et un
/// remplacement caractère-par-espace décalerait tous les index suivants. Les
/// frontières de région sont toutes ASCII (`"`, `/`, `*`, `\n`), donc aucun
/// caractère multi-octet n'est coupé en deux et le résultat reste de l'UTF-8
/// valide.
///
/// Sans ce blanchiment, un comptage naïf d'accolades casserait sur les `{` des
/// `format!` et des `info!`, dont `tick()` et ses voisins sont pleins ; et la
/// recherche de la garde dans le fragment précédant une `{` trouverait le texte
/// d'un commentaire qui *parle* de la garde au lieu de la garde elle-même.
fn blank_noncode(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0usize;

    while i < b.len() {
        // Commentaire de ligne.
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
            continue;
        }

        // Commentaire de bloc, imbricable comme en Rust.
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let mut depth = 1usize;
            out.push(b' ');
            out.push(b' ');
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
                    depth += 1;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                } else if b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/' {
                    depth -= 1;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                } else {
                    out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                    i += 1;
                }
            }
            continue;
        }

        // Chaîne brute `r"…"` / `r#"…"#`. Le `r` doit ouvrir un token, sinon
        // c'est la dernière lettre d'un identifiant (`for`, `char`, …).
        if b[i] == b'r' && (i == 0 || !is_ident_byte(b[i - 1])) {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < b.len() && b[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < b.len() && b[j] == b'"' {
                out.resize(out.len() + (j + 1 - i), b' ');
                i = j + 1;
                while i < b.len() {
                    if b[i] == b'"' {
                        let mut k = i + 1;
                        let mut seen = 0usize;
                        while k < b.len() && b[k] == b'#' && seen < hashes {
                            seen += 1;
                            k += 1;
                        }
                        if seen == hashes {
                            out.resize(out.len() + (k - i), b' ');
                            i = k;
                            break;
                        }
                    }
                    out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                    i += 1;
                }
                continue;
            }
        }

        // Chaîne ordinaire.
        if b[i] == b'"' {
            out.push(b' ');
            i += 1;
            while i < b.len() {
                if b[i] == b'\\' && i + 1 < b.len() {
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                }
                let end = b[i] == b'"';
                out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                i += 1;
                if end {
                    break;
                }
            }
            continue;
        }

        // Littéral de caractère, distingué d'une durée de vie (`'static`,
        // `'a`) : un littéral porte soit une échappée, soit son apostrophe
        // fermante au troisième octet.
        if b[i] == b'\''
            && ((i + 1 < b.len() && b[i + 1] == b'\\') || (i + 2 < b.len() && b[i + 2] == b'\''))
        {
            out.push(b' ');
            i += 1;
            while i < b.len() {
                if b[i] == b'\\' && i + 1 < b.len() {
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                }
                let end = b[i] == b'\'';
                out.push(b' ');
                i += 1;
                if end {
                    break;
                }
            }
            continue;
        }

        out.push(b[i]);
        i += 1;
    }

    String::from_utf8(out)
        .expect("le blanchiment remplace octet par octet, donc l'UTF-8 reste valide")
}

/// Le corps de `pub async fn tick`, blanchi de ses chaînes et de ses
/// commentaires, accolades équilibrées.
///
/// **Rend `None` plutôt qu'un corps vide** quand la signature n'est pas là ou
/// que les accolades ne se referment pas : c'est la moitié fail-closed du
/// dispositif. Un `tick` renommé doit faire **rougir** le scan, pas le faire
/// passer sur un texte vide — un scan qui ne trouve plus sa cible et se tait
/// est la classe mika#2205.
fn tick_body(src: &str) -> Option<String> {
    let blanked = blank_noncode(src);
    let start = blanked.find(TICK_SIGNATURE)? + TICK_SIGNATURE.len();
    let b = blanked.as_bytes();
    let mut depth = 1usize;
    let mut i = start;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(blanked[start..i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Les sites de [`TERMINAL_PARENT_SWEEPS`] appelés hors garde `cli_mode`, dans
/// l'ordre du texte, **un par site d'appel**.
///
/// Tient une pile de garde : à chaque `{` on empile *vrai* si le fragment de
/// code qui précède porte [`CLI_GUARD`], sinon la valeur du parent (une garde
/// vaut pour tout ce qu'elle contient, blocs imbriqués compris) ; à chaque `}`
/// on dépile. Un `self.<nom>(` dont le sommet de pile est *faux* est une
/// violation.
///
/// Le fragment examiné court depuis la dernière `{`, `}` ou `;` : c'est
/// exactement la portée de la condition d'un `if`, et rien de plus.
fn unguarded_sweeps(body: &str) -> Vec<&'static str> {
    let b = body.as_bytes();
    // Le corps de `tick()` lui-même n'est pas gardé.
    let mut stack: Vec<bool> = vec![false];
    let mut fragment_start = 0usize;
    let mut found: Vec<&'static str> = Vec::new();
    let mut i = 0usize;

    while i < b.len() {
        match b[i] {
            b'{' => {
                let guarded = window_contains(&b[fragment_start..i], CLI_GUARD)
                    || stack.last().copied().unwrap_or(false);
                stack.push(guarded);
                fragment_start = i + 1;
            }
            b'}' => {
                stack.pop();
                if stack.is_empty() {
                    stack.push(false);
                }
                fragment_start = i + 1;
            }
            b';' => fragment_start = i + 1,
            _ => {
                if b[i..].starts_with(b"self.") && !stack.last().copied().unwrap_or(false) {
                    let rest = &b[i + 5..];
                    for sweep in TERMINAL_PARENT_SWEEPS {
                        if rest.starts_with(sweep.as_bytes())
                            && rest.get(sweep.len()) == Some(&b'(')
                        {
                            found.push(sweep);
                        }
                    }
                }
            }
        }
        i += 1;
    }

    found
}

/// Le bloc de production, recopié **sans** la ligne de garde : le contrôle de
/// bonne foi d'AC3.
///
/// Il passe par le même [`tick_body`] que le texte réel — donc par tout le
/// dispositif, pas seulement par sa dernière fonction. Il porte aussi
/// `settle_dispatch_parents` et `report_promotion_starvation`, qui ne sont pas
/// au recensement : le contrôle prouve que le scan rend les six sites visés, et
/// pas tout ce qu'il croise.
const TICK_SANS_GARDE: &str = r#"
    pub async fn tick(&mut self) {
        self.heartbeat.tick();
        self.tick_count = self.tick_count.wrapping_add(1);
        if self.tick_count.is_multiple_of(DB_SCAN_INTERVAL_TICKS) {
            self.reap_orphaned_parent_tasks("implement").await;
            self.reap_orphaned_parent_tasks("groom").await;
            self.complete_parent_tasks_on_callback_success().await;
            self.settle_dispatch_parents().await;
            self.reap_childless_stuck_parent_tasks().await;
            self.reap_orphaned_pending_issue_tasks().await;
            self.reap_stale_blocked_dispatch_tasks().await;
            self.report_promotion_starvation().await;
        }
        debug!(fired, "tick fired tasks");
    }
"#;

/// Les six sites du contrôle, dans l'ordre où le scan les rencontre.
const SITES_DU_CONTROLE: [&str; 6] = [
    "reap_orphaned_parent_tasks",
    "reap_orphaned_parent_tasks",
    "complete_parent_tasks_on_callback_success",
    "reap_childless_stuck_parent_tasks",
    "reap_orphaned_pending_issue_tasks",
    "reap_stale_blocked_dispatch_tasks",
];

/// mika#3042 AC3 — le corps réel de `tick()` ne porte aucun balayage terminal
/// de parente `self_dev` hors garde.
#[test]
fn ac3_aucun_balayage_terminal_de_parente_self_dev_hors_garde_cli() {
    let body = tick_body(ENGINE_SRC).expect(
        "la signature `pub async fn tick(&mut self) {` doit être trouvable dans \
         engine.rs, accolades équilibrées — un `tick` renommé rougit ici plutôt \
         que de rendre un corps vide",
    );
    let violations = unguarded_sweeps(&body);
    assert!(
        violations.is_empty(),
        "mika#3042 : ces balayages écrivent un état terminal sur une parente \
         `self_dev` et sont appelés dans `tick()` SANS garde `{CLI_GUARD}` — \
         le moteur de `mika chat` tourne contre la base du démon, avec le code \
         de son propre binaire. Mettre l'appel sous la garde, ou déplacer son \
         prédicat hors de la population `self_dev` (le cas de \
         `settle_dispatch_parents`). Sites trouvés : {violations:?}"
    );
}

/// mika#3042 AC3 — le contrôle de bonne foi : le même scan, sur une copie du
/// même bloc privée de sa ligne de garde, rend les six sites.
///
/// Si ce test passe au vert avec un scan cassé, il rougit. C'est son seul rôle.
#[test]
fn ac3_le_meme_scan_rougit_sur_une_copie_sans_la_garde() {
    let body = tick_body(TICK_SANS_GARDE).expect("le contrôle porte la même signature");
    let violations = unguarded_sweeps(&body);
    assert_eq!(
        violations.as_slice(),
        SITES_DU_CONTROLE.as_slice(),
        "le scan doit rendre les six sites de la copie sans garde — ni moins \
         (il serait aveugle), ni `settle_dispatch_parents` / \
         `report_promotion_starvation`, qui ne sont pas au recensement"
    );
}

/// mika#3042 AC3 — fail-closed, des deux côtés.
///
/// À gauche : un corps introuvable rend `None`, et le test ci-dessus
/// `expect`-e — renommer `tick` fait rougir le scan au lieu de le faire passer
/// sur un corps vide.
///
/// À droite : chaque entrée du recensement apparaît au moins une fois dans le
/// corps réel. Une entrée morte — appel supprimé ou renommé en amont — rougit,
/// au lieu de laisser le scan croire qu'il surveille cinq balayages alors qu'il
/// en surveille quatre. C'est l'assertion auto-nettoyante de la liste close :
/// le geste qui rend une entrée inutile est le geste qui la fait expirer.
#[test]
fn ac3_le_scan_echoue_ferme_plutot_que_de_se_taire() {
    assert!(
        tick_body("async fn autre() { }").is_none(),
        "sans la signature de `tick`, le scan n'a pas de cible : il doit le dire"
    );
    assert!(
        tick_body("pub async fn tick(&mut self) { if x {").is_none(),
        "des accolades qui ne se referment pas ne font pas un corps"
    );

    let body = tick_body(ENGINE_SRC).expect("engine.rs porte `tick`");

    // Les bornes du corps extrait : la dernière ligne de `tick()` est dedans,
    // la fonction suivante est dehors. Sans ces deux ancres, un équilibrage
    // d'accolades qui s'arrête trop tôt rendrait un corps « propre » par
    // troncature (classe mika#2103).
    assert!(
        body.contains("self.detect_empty_pilot_transcripts()"),
        "le corps extrait doit aller jusqu'au bas du bloc de scan DB"
    );
    assert!(
        body.contains("self.fire_task(task)"),
        "le corps extrait doit inclure la phase de tir, qui suit le bloc de scan"
    );
    assert!(
        !body.contains("async fn expire_timed_out_tasks"),
        "le corps extrait doit s'arrêter à la fin de `tick`, pas déborder sur \
         la fonction suivante"
    );

    for sweep in TERMINAL_PARENT_SWEEPS {
        assert!(
            body.contains(&format!("self.{sweep}(")),
            "`{sweep}` est au recensement de TERMINAL_PARENT_SWEEPS mais n'est \
             plus appelé dans `tick()` : retirer l'entrée (le balayage a disparu) \
             ou corriger le nom (il a été renommé). Un recensement qui nomme un \
             site mort surveille moins de sites qu'il ne le dit."
        );
    }
}

/// mika#3042 AC3 — la frontière du prédicat, pour `settle_dispatch_parents`.
///
/// Ce balayage est le seul de l'échelle de réparation qui touche des parentes et
/// vit **hors** garde. Ce qui l'autorise n'est pas une dérogation, c'est son
/// `WHERE` : il porte le **complément** de la population `self_dev`. Si cette
/// clause part de `find_settleable_dispatch_parents`, il devient un écrivain
/// terminal de parentes `self_dev` et doit entrer dans
/// [`TERMINAL_PARENT_SWEEPS`] — ce test rougit alors, et nomme le geste.
#[test]
fn ac3_settle_dispatch_parents_reste_hors_predicat() {
    assert!(
        TASKS_SRC.contains(SETTLE_COMPLEMENT_PREDICATE),
        "`{SETTLE_COMPLEMENT_PREDICATE}` a disparu de db/tasks.rs. C'est ce \
         terme, et lui seul, qui tient `settle_dispatch_parents` hors du \
         prédicat de mika#3042 (« écrit un état terminal sur une parente \
         `self_dev` »). Sans lui, il faut l'ajouter à TERMINAL_PARENT_SWEEPS et \
         le passer sous la garde `{CLI_GUARD}` dans `tick()`."
    );
}
