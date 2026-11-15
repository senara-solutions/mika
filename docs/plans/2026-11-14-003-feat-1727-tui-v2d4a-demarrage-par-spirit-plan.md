# Vol V2d-4a — la TUI ouvre sa session par mika-spirit, ou ne démarre pas

**Ticket :** senara-solutions/mika#1727 (vol V2d-4a)
**Branche :** `feat/1727/tui-v2d4a-demarrage-par-spirit`
**Base mesurée :** `a64a961c` (merge de V2d-3b, #3087)

Le démarrage de `mika` / `mika chat` passe par la porte
`commands/session_remote.rs::ouvrir`. Sans `MIKA_INTERNAL_TOKEN`, ou si la porte
échoue, la TUI **sort en non-zéro avant l'écran**, en nommant la cause. C'est le
choix A de Vincent du 2026-11-14 : depuis V1 chaque tour passe par spirit, donc
une TUI ouverte sans lui n'a aucun mode utile, et la panne doit se dire au moment
où elle compte.

---

## 0. Ce qui est mesuré

| fait | site | ce qu'il décide ici |
|---|---|---|
| la session de démarrage est créée **en local** | `commands/chat.rs:101-107` (`get_or_create_canonical_session`) et `:108-114` (`create_session`) | c'est le bloc que ce vol remplace par la porte |
| un échec de création ne donne qu'un `tracing::warn!` et la TUI continue | `chat.rs:104-106` et `:111-113` | la panne est aujourd'hui **muette** ; elle devient un `Err` qui remonte |
| `spawn_agent_worker` est appelée à trois sites | `chat.rs:543` (démarrage, avec `?`), `:779` (changement d'agent), `:883` (`/restart`) | le `?` du démarrage est **déjà** avant l'écran ; les deux autres ont déjà leur chemin d'erreur (`:828-835`, `:917-925`). **Aucun des trois ne change** (D5) |
| le contrôle de propriété de `--session-id` lit `get_session` | `chat.rs:88-99` | le bras « reprise » est conservé tel quel, et seule la création locale qui le suivait disparaît |
| la sonde de démarrage nomme `MIKA_INTERNAL_TOKEN` comme cause possible d'un `None` | `chat.rs:664-684` | c'est la phrase que le critère 7 rend fausse (D6) |
| la sonde passe par `dashboard::auth_token()`, qui accepte **aussi** `MIKA_DASHBOARD_TOKEN` | `commands/dashboard.rs:37-38` | l'angle mort que ce vol ferme : un shell qui n'a que le jeton du tableau de bord passe la sonde, puis prend un 401 à chaque tour |
| la porte rend `CreatedSessionMirror { session_id, canonical }`, et la route **ne prend aucun identifiant** dans son corps | `session_remote.rs:35-48`, `:233-241` ; `server/sessions.rs:84-106` | `is_singleton_session` vient du fil (critère 3), et une session inconnue ne peut pas être créée à identifiant choisi (critère 5, D4) |
| le modèle de lecture stricte du jeton existe, dans un fichier que ce vol ne touche pas | `tui/commands/handlers.rs:1231` (`jeton_interne`) et son `type SourceEnv<'a>` | le démarrage porte **sa propre** lecture sur la même forme ; la mise en commun est à V2d-4b (D2) |
| le `.env` de la home est chargé **avant** le `match cli.command` | `main.rs:221-234`, le `match` l. 333, les bras `None` l. 335-341 et `Commands::Chat` l. 342-349 | **aucun chargement explicite de `dotenv` n'est à ajouter dans `chat.rs`** — mesuré, et non supposé (D2) |
| les seules occurrences de `create_session(` et `get_or_create_canonical_session(` dans `chat.rs` sont l. 103 et l. 110 | `rg -n 'create_session\(\|get_or_create_canonical_session\(' crates/mika-cli/src/commands/chat.rs` | la garde structurelle de D7 peut être livrée **allowlist vide**, parce que le même commit retire les deux violations |
| `chat.rs` n'a **aucun** module `#[cfg(test)]` | le fichier entier (1253 l.) | il faut en créer un, et il doit être **en toute fin de fichier** : `ProductionScanner` coupe la moitié production au premier `#[cfg(test)]` (D7) |
| `end_session(` apparaît l. 744, 861 et 960 | `chat.rs` | **hors motifs** de la garde : les sorties sont V2d-4b, et les refuser ici rendrait la garde rouge le jour de sa pose |
| le banc est déjà emprunté par le voisin | `session_remote.rs:323` (`use crate::commands::rewind_remote::tests::fake_spirit`), `:325` (`DeadEndpoint`) | les tests de ce vol empruntent le même banc, sans rien y ajouter (`rewind_remote.rs` reste intact) |
| `AsyncDatabase::new(db)`, `agent_id()`, `get_session()` | `mika-agent/src/async_db.rs:49`, `:130`, `:3261` | la fixture de test est une base en mémoire, comme `handlers.rs::test_app()` — pas d'`AppContext` à construire (D3) |

**Les faits qui sont des absences** — ceux qui décident autant que les présences :

- **Personne ne lit `ouvrir` ni les champs de `CreatedSessionMirror` hors test.**
  C'est ce que disent les deux `expect(dead_code)` de `session_remote.rs:36` et
  `:233`. Ce vol leur donne leur premier lecteur de production, donc les deux
  **doivent** tomber dans le commit qui câble, sous peine d'une attente
  insatisfaite (D9).
- **Aucune entrée d'egress ne change.** `chat.rs` appelle la porte ; il ne
  construit aucun client vers les routes de session. L'entrée `cli-spirit-sessions`
  désigne toujours `session_remote.rs`. Si le pilote juge le contraire, il
  s'arrête et le dit (§ 5).
- **Aucun fichier voisin n'est nécessaire.** `commands/mod.rs` porte déjà
  `pub mod session_remote;`, `mika-common` + `test-utils` est déjà en
  `[dev-dependencies]` du crate, et `dashboard::spirit_url()` est déjà appelée
  par `chat.rs` (l. 681).

---

## 1. La forme, et les neuf décisions qui la tiennent

### D1 — La source d'environnement arrive **par argument**, jamais lue au site

```rust
/// La source d'environnement arrive en paramètre : un test de refus qui
/// muterait l'environnement du process serait `unsafe` en édition 2024, et
/// faux sous `cargo test` qui parallélise. La même forme que
/// `tui/commands/handlers.rs:1229`.
type SourceEnv<'a> = &'a dyn Fn(&str) -> Option<String>;
```

C'est ce qui rend les critères 1 et 5 testables **sans environnement** : le test
passe `&|_| None`, ou `&|nom| (nom == "MIKA_DASHBOARD_TOKEN").then(|| "x".into())`.
La production passe `&|nom| std::env::var(nom).ok()`, exactement comme
`handle_rewind_impl`.

**La duplication de `SourceEnv` et de `jeton_interne` avec `handlers.rs` est
délibérée et pré-autorisée par le ticket** : `handlers.rs` ne change pas dans ce
vol, et la mise en commun des deux lectures revient à V2d-4b, qui le touche. La
dette est nommée en § 7.

### D2 — La lecture est stricte, et le refus nomme la variable **et** celle qui ne suffit pas

```rust
fn jeton_interne(lire: SourceEnv<'_>) -> Result<String> {
    lire("MIKA_INTERNAL_TOKEN").ok_or_else(|| {
        anyhow::anyhow!(
            "La TUI exige MIKA_INTERNAL_TOKEN pour ouvrir sa session sur \
             mika-spirit. Pose-la dans ~/.mika/.env — MIKA_DASHBOARD_TOKEN n'est \
             pas accepté par les routes de session, même s'il suffit à la sonde \
             du tableau de bord."
        )
    })
}
```

La seconde phrase n'est pas de la politesse : c'est la **mesure** de
`dashboard.rs:37-38` écrite à l'endroit où un opérateur la lira. Un shell qui
porte le jeton du tableau de bord passe la sonde de démarrage et prend un 401 à
chaque tour — l'angle mort que ce vol ferme, et le seul endroit du code où il se
dit.

**Pas de règle de non-vacuité**, délibérément : le modèle de `handlers.rs` n'en a
pas, et une variable posée à vide est une *configuration fausse*, pas une
absence. Elle part sur le fil et revient en 401, dont le `Display` nomme déjà la
variable (critère 4). Inventer ici une seconde règle de validité ferait diverger
les deux lectures que V2d-4b doit réunir.

**Aucun `dotenv` n'est chargé ici** : `main.rs:221-234` le fait avant le `match`,
pour le bras `None` comme pour `Commands::Chat` (§ 0). Le ticket demandait de le
vérifier avant de conclure ; c'est vérifié, et c'est pourquoi `chat.rs` ne porte
pas la ligne que `handle_rewind_impl` porte.

### D3 — Une fonction qui prend la base, l'URL et la source, et **rien d'autre**

```rust
/// Ouvre — ou reprend — la session de la TUI. La forme est celle de
/// `rewind_par_spirit` : tout ce qui touche le monde arrive par argument, donc
/// un test pilote la source d'environnement, la base et l'URL du banc sans
/// construire d'`AppContext`.
async fn ouvrir_session_par_spirit(
    db: &AsyncDatabase,
    base_url: &str,
    lire_env: SourceEnv<'_>,
    agent: &str,
    session_id: Option<&str>,
    canonical_session_id: Option<&str>,
) -> Result<(String, bool)>  // (session_id, is_singleton_session)
```

Elle prend `&AsyncDatabase` et non `&AppContext` : c'est ce qui rend la fixture
de test une base en mémoire (`Database::open_in_memory()` → `AsyncDatabase::new`),
comme `handlers.rs::test_app()`, au lieu d'un contexte complet avec home,
`Settings` et registre de skills. Le couple rendu est exactement ce que
`spawn_agent_worker` plaçait dans ses deux variables locales, donc le site
d'appel se réduit à une ligne.

### D4 — Deux bras, et le jeton est exigé **dans les deux**

**Sans `--session-id`** (le démarrage nominal) :

1. `jeton_interne(lire_env)?` — le refus tombe **avant toute construction de
   client**. C'est le critère 1 : le banc ne reçoit aucune requête.
2. `session_remote::ouvrir(base_url, &jeton, agent).await` — un POST sur
   `{base}/api/v1/sessions`, `Authorization: Bearer <jeton>`, corps `{"agent": …}`.
3. le couple rendu est `(mirror.session_id, mirror.canonical)`. **`canonical` vient
   du fil**, et non de `resolve_canonical_session_id` : la route rend la session
   canonique de l'agent au lieu d'en créer une (`server/sessions.rs:84-106`), donc
   le serveur est la seule autorité sur ce booléen (critère 3).

**Avec `--session-id`** (la reprise) :

1. `jeton_interne(lire_env)?` — **le même refus, au même rang**. Le jeton n'est pas
   facultatif sur ce chemin : le premier tour part sur le fil de toute façon, et
   un refus qui attendrait le tour nommerait le transport là où la cause est la
   configuration du shell. C'est l'argument de V2c-1b, reconduit.
2. `db.get_session(id).await` :
   - `Ok(Some(s))` dont `s.agent_id != db.agent_id()` → refus, **texte inchangé** ;
   - `Ok(Some(_))` à l'agent → reprise : `(id.to_string(), canonical_session_id == Some(id))`.
     `is_singleton_session` y reste calculé **exactement comme aujourd'hui**, et
     `ouvrir` n'est **pas** appelée ;
   - `Ok(None)` → refus qui **nomme l'identifiant** : la route ne prend aucun
     identifiant dans son corps, donc elle ne sait pas créer une session à
     identifiant choisi. Aucune ligne n'est écrite en base (critère 5) ;
   - `Err(e)` → refus qui nomme l'échec de lecture. **Aujourd'hui cette erreur est
     avalée** (`if let Ok(Some(existing))`, l. 88) et la session est créée en local
     derrière ; demain une base illisible ne doit pas se lire comme « session
     inconnue ». Motif mika#2331 : une fausse provenance coûte plus cher qu'une
     absence.

Le contrôle `--session-id` vide (`chat.rs:71-74`) reste **avant** tout cela, dans
`spawn_agent_worker` : c'est un refus d'argument, pas une question d'ouverture.

### D5 — Les trois sites d'appel ne changent pas, et c'est une mesure

`spawn_agent_worker` rend déjà `Result<…>`. Au démarrage (`chat.rs:543`) l'appel
porte un `?`, et il est **avant** l'installation du hook de panique et du terminal
(l. 688-694) : une erreur y sort en non-zéro sans jamais afficher l'écran.
C'est la seconde moitié du critère 4, et elle est **vraie sur `main`** — ce vol la
démontre, il ne la construit pas. Au changement d'agent (l. 779) et à `/restart`
(l. 883), les chemins d'erreur existants (l. 828-835 et l. 917-925) poussent déjà
le message dans la conversation sans quitter la TUI ; le `Display` de
`SessionRemoteIssue` y arrive tel quel par `anyhow`.

**Rien n'est donc modifié à ces trois sites**, hors la ligne de D3 au premier.

### D6 — Le message de la sonde ne nomme plus le jeton

Le texte actuel (l. 673-683) dit « vérifie `MIKA_INTERNAL_TOKEN` dans ce shell —
la sonde rend la même absence dans les deux cas ». Après ce vol cette phrase est
**fausse** : à la ligne 664, le jeton a été lu (D4, les deux bras), et sur le
chemin nominal spirit a répondu à `ouvrir`. Le message, et le commentaire au-dessus
qui en porte le raisonnement, nomment désormais l'endpoint et le geste du daemon,
et rien d'autre.

**La sonde elle-même ne change pas** : `query_dashboard_status()` est appelée à
l'identique, et `dashboard.rs` reste intact.

**Ce que le nouveau message ne dit plus, et pourquoi c'est juste.** Un `None` peut
encore venir d'un jeton *présent mais refusé* sur le chemin de reprise, où
`ouvrir` n'est pas appelée. Ce n'est pas un silence : ce jeton-là est nommé par le
401 du premier tour, dont le `Display` porte la variable. Le message de la sonde
n'a plus à porter une cause que le chemin nominal a déjà écartée — la nommer
serait la fausse provenance de mika#2331, dans l'autre sens.

### D7 — La garde structurelle, allowlist vide, témoin de lecture, en fin de fichier

```rust
#[test]
fn mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local() {
    const ALLOWED: &[&str] = &[];
    assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
    const MOTIFS: &[(&str, &str)] = &[
        ("create_session(", "session_remote::ouvrir"),
        ("get_or_create_canonical_session(", "session_remote::ouvrir"),
    ];
    let scanner = ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let site = scanner.src_root().join("commands/chat.rs");
    let production = scanner.production_of(&site);
    assert!(
        production.contains("async fn spawn_agent_worker("),
        "la garde n'a pas lu la moitié production de chat.rs"
    );
    // … une boucle par motif : lignes non commentées qui le contiennent, assert vide,
    //   chaque violation nommant la porte qui la remplace.
}
```

Modèle : `mika1727_v2c1b_la_tui_ne_rembobine_plus_la_base_en_local`
(`handlers.rs:2661-2710`), copié dans sa forme et non dans son texte.

Quatre points que la forme impose :

- **L'allowlist est livrée vide et assertie vide.** Elle le peut parce que § 0 le
  mesure : les deux seules violations de `chat.rs` sont l. 103 et l. 110, et le
  même commit les retire. Quand la garde rougit, le remède est de passer par la
  porte, **jamais** d'ajouter une entrée.
- **Le témoin de lecture** (`spawn_agent_worker`) est ce qui distingue « aucune
  violation » de « le scan n'a rien lu » — un chemin faux, un fichier renommé, ou
  un `#[cfg(test)]` remonté par mégarde rendraient la garde verte par vacuité.
- **Le module `tests` va en toute fin de `chat.rs`.** `production_of` coupe au
  premier `#[cfg(test)]` : posé plus haut, il amputerait la moitié production que
  cette garde **et** les deux gardes de crate de `chat_turn.rs`
  (`mika1727_v1_*`, `mika1727_v2a_*`) balaient. La fin de fichier laisse leur
  mesure identique à celle d'aujourd'hui.
- **Les fixtures de test appellent `create_session`** pour poser la session que
  le bras « reprise » relit. C'est légitime et invisible : elles vivent dans la
  moitié test, que `production_of` ne rend pas.

`end_session(` n'est **pas** un motif : les sorties sont V2d-4b, et les refuser
ici rendrait la garde rouge le jour de sa pose.

### D8 — `session_remote.rs` : deux `expect` tombent, et la doc dit le compte **mesuré**

Les `#[cfg_attr(not(test), expect(dead_code, …))]` de `CreatedSessionMirror`
(l. 36) et d'`ouvrir` (l. 233) ont désormais un lecteur de production et doivent
partir. Restent ceux de `clore`, `compacter`, `EndedSessionMirror` et
`CompactedMirror` — **quatre**, pour V2d-4b.

La doc du module (l. 18-27) dit aujourd'hui « **six** — un par porte, un par
miroir ». Elle dira quatre, et redira que ce chiffre se **mesure** sous
`cargo clippy -p mika-cli --all-targets` et ne se devine pas : le retrait de deux
attributs change le graphe d'atteignabilité que `rustc` parcourt, donc un compte
obtenu par soustraction n'est pas une mesure. **Si la passe verte en demande un
nombre différent de quatre, c'est la passe qui a raison et la doc qui s'aligne**,
et le pilote écrit le compte observé.

### D9 — Le câblage et le retrait des `expect` sont dans **le même commit**

Dès que `chat.rs` appelle `ouvrir` et lit les deux champs du miroir, les deux
attributs deviennent `unfulfilled_lint_expectation` : c'est tout l'intérêt
d'`expect` plutôt qu'`allow` (Fire-Disposition de V2d-3a, reconduite). Les séparer
laisserait un commit intermédiaire **rouge sous clippy**, ce que l'ordre de
travail du ticket — « le pilote commite après chaque garde verte » — interdit.
D'où le commit 1 du § 3, qui porte les deux fichiers.

---

## 2. Les livrables, un par un

### 2.1 `chat.rs` — la porte du démarrage

`type SourceEnv<'a>`, `fn jeton_interne`, `async fn ouvrir_session_par_spirit`
(D1-D4), posés **au-dessus** de `spawn_agent_worker`. Dans `spawn_agent_worker`,
le bloc l. 95-114 (le contrôle de propriété, les deux branches de création et
leurs deux `tracing::warn!`) disparaît et devient :

```rust
let (session_id, is_singleton_session) = ouvrir_session_par_spirit(
    &ctx.async_db,
    &crate::commands::dashboard::spirit_url(),
    &|nom| std::env::var(nom).ok(),
    agent_name,
    session_id,
    canonical_session_id.as_deref(),
)
.await?;
```

`identity` et `resolve_canonical_session_id` restent : le bras « reprise » lit
`canonical_session_id`, et `identity` sert plus bas. La variable `reusing_session`
disparaît — le branchement est `session_id.is_some()` à l'intérieur de la
fonction.

### 2.2 `chat.rs` — le message de la sonde (D6)

Le `format!` de l. 674-683 et le commentaire de l. 669-672 sont réécrits. Le
nouveau texte nomme `spirit_url()` et le geste (`mika-spirit`), et ne nomme plus
le jeton. Le commentaire dit **pourquoi** la cause n'est plus ambiguë à ce point
du démarrage — et non ce que le code fait.

### 2.3 `chat.rs` — le module `#[cfg(test)] mod tests`, en fin de fichier

La fixture, les neuf tests de comportement et la garde structurelle (§ 4). Les
imports : `crate::commands::rewind_remote::tests::fake_spirit`,
`mika_common::dead_endpoint::DeadEndpoint`,
`mika_common::source_guard::ProductionScanner`,
`mika_agent::db::Database` / `AsyncDatabase`, `axum::http::StatusCode`. **Rien
n'est ajouté au banc** : `rewind_remote.rs` reste intact, et `RequeteRecue` est lu
sans `Debug`, comme dans `session_remote.rs`.

### 2.4 `session_remote.rs` — les deux `expect` et la doc du module (D8)

Deux lignes retirées, un paragraphe de doc réécrit. **Aucun autre changement** :
ni `post`, ni `lire_json`, ni les tests existants.

### 2.5 `crates/mika-cli/CLAUDE.md` — un paragraphe neuf, et une réconciliation

Un `### Le démarrage de la TUI passe par mika-spirit (mika#1727, vol V2d-4a)` dans
la famille des sections TUI, après celle de V2c-1b. Il dit : ce que le démarrage
exige (`MIKA_INTERNAL_TOKEN`, et que `MIKA_DASHBOARD_TOKEN` ne suffit pas), ce
qu'il refuse (une session inconnue sous `--session-id`), d'où vient
`is_singleton_session` (le fil, pas l'identité locale), et le nom de la garde avec
sa règle — *quand elle rougit, on passe par la porte, on n'ajoute pas d'entrée*.

**Et il réconcilie le § V1.** Celui-ci dit aujourd'hui que la sonde nomme « **les
deux** causes possibles » dont `MIKA_INTERNAL_TOKEN` ; D6 rend la phrase fausse.
Elle est corrigée dans le même commit, en disant ce qui a changé et pourquoi — un
document qui décrit un comportement disparu est pire qu'un document muet.

---

## 3. Ordre des commits

Chaque garde arrive **avec ses tests, dans le même commit**. Après chacun, le
pilote mesure `git diff --numstat origin/main...HEAD`, plan exclu.

**Avant le commit 1**, hors commit : le pilote écrit la garde de D7 dans le
working tree **sans toucher au reste**, lance
`cargo test -p mika-cli mika1727_v2d4a_la_tui_ne_cree_plus`, observe le **rouge**
nommant `chat.rs:103` et `chat.rs:110`, et consigne la sortie dans
`.pilot-scratch/v2d4a/garde-rouge-avant.txt` (mika#2548 — ce répertoire ne se
supprime jamais). C'est le « rouge-avant sur le code de main » du critère 6.

| # | contenu | fichiers | critères |
|---|---|---|---|
| 1 | la porte du démarrage (§ 2.1) **et** le retrait des deux `expect` + la doc du module (§ 2.4, D9), avec les tests T1-T9 | `chat.rs`, `session_remote.rs` | 1, 2, 3, 4, 5, 8 |
| 2 | la garde structurelle (T10), posée verte sur le code du commit 1 | `chat.rs` | 6 |
| 3 | le message de la sonde (§ 2.2) | `chat.rs` | 7 |
| 4 | la doc (§ 2.5) | `crates/mika-cli/CLAUDE.md` | — |

**Halte sur la borne.** Si le total dépasse **540 lignes ajoutées**, le pilote
s'arrête : pas de commit suivant, pas de PR. Son dernier message dit
« HALTE borne : N lignes pour 540 », avec le compte par commit. Les commentaires
qui portent un pourquoi **ne se rabotent pas** pour tenir la borne.

**Avant d'ouvrir la PR** : `cargo test -p mika-cli`,
`cargo clippy -p mika-cli --all-targets` (vert, sans `allow`, sans attente
insatisfaite), `cargo fmt --check`,
`bash scripts/verify-egress-manifest.sh --report`, et
`bash scripts/verify-pipeline.sh origin/main`.

---

## 4. Le jeu de tests

Tous dans le module `tests` en fin de `chat.rs`. La fixture est une base en
mémoire (`Database::open_in_memory()` → `AsyncDatabase::new`), et le banc est
`fake_spirit(&[("/api/v1/sessions", StatusCode::…, corps)])`.

| # | nom | ce qu'il prouve | critère |
|---|---|---|---|
| T1 | `mika1727_v2d4a_sans_jeton_interne_le_demarrage_refuse_avant_tout_appel` | source `&\|_\| None` : le refus nomme `MIKA_INTERNAL_TOKEN`, et **le banc ne reçoit rien** (`vues.len() == 0`) | 1 |
| T2 | `mika1727_v2d4a_un_jeton_de_tableau_de_bord_ne_suffit_pas` | source ne portant que `MIKA_DASHBOARD_TOKEN` : même refus, zéro requête ; **contrôle positif** — la même source plus `MIKA_INTERNAL_TOKEN` ouvre, et le banc voit une requête | 1 |
| T3 | `mika1727_v2d4a_louverture_poste_sur_la_route_des_sessions_avec_son_jeton` | sur la socket : `POST`, `/api/v1/sessions`, `Authorization: Bearer <jeton>`, corps portant `agent` ; et le `session_id` rendu par la TUI est celui du fil | 2 |
| T4 | `mika1727_v2d4a_canonical_vrai_donne_une_session_singleton` | `{"canonical":true}` → `is_singleton_session == true` | 3 |
| T5 | `mika1727_v2d4a_canonical_faux_ne_donne_pas_de_singleton` | `{"canonical":false}` → `is_singleton_session == false` | 3 |
| T6 | `mika1727_v2d4a_un_401_nomme_la_variable_a_poser` | un 401 du banc remonte en `Err` dont le texte porte `MIKA_INTERNAL_TOKEN` | 4 |
| T7 | `mika1727_v2d4a_un_point_injoignable_nomme_lurl` | `DeadEndpoint::reserve()`, garde tenue pendant tout l'appel : l'erreur porte l'URL | 4 |
| T8 | `mika1727_v2d4a_une_session_inconnue_est_refusee_en_nommant_son_identifiant` | base vide, `--session-id "s-absente"` : le refus nomme `s-absente`, `get_session` rend toujours `None` **après**, et le banc ne reçoit rien | 5 |
| T9 | `mika1727_v2d4a_une_session_connue_est_reprise_sans_ouverture` | session de l'agent posée en fixture : reprise rendue telle quelle, `ouvrir` **non appelée** (zéro requête) | 5 |
| T10 | `mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local` | la garde structurelle de D7 : allowlist vide et assertie vide, témoin de lecture, deux motifs | 6 |

**Le contrôle positif de T2 n'est pas décoratif.** Sans lui, un refus qui tomberait
pour n'importe quelle raison — une URL mal bâtie, un banc non armé — rendrait T1 et
T2 verts en mesurant autre chose que la lecture du jeton. C'est la forme de
`mika1727_v2c1b_..._un_jeton_de_tableau_de_bord_ne_suffit_pas`.

**Le compte de requêtes se lit sans `Debug`.** `RequeteRecue` n'en dérive pas, et
le banc du voisin reste intact : les assertions passent par `vues.len()` et les
champs, jamais par un rendu du vecteur.

---

## 5. Definition of Done

- [ ] `cargo test -p mika-cli` vert, T1-T10 compris.
- [ ] `cargo clippy -p mika-cli --all-targets` vert, **sans `allow`** et **sans
      attente insatisfaite**. Les `expect(dead_code)` de `CreatedSessionMirror` et
      d'`ouvrir` ont disparu ; ceux qui restent portent
      `reason = "câblé par V2d-4, mika#1727"`, et la doc du module annonce leur
      compte **mesuré** sous cette passe.
- [ ] `cargo fmt --check` vert.
- [ ] Le rouge-avant de la garde est consigné dans
      `.pilot-scratch/v2d4a/garde-rouge-avant.txt`, puis le vert est montré.
- [ ] `git diff --numstat origin/main...HEAD` (plan exclu) **≤ 540 lignes
      ajoutées**. Sinon : halte, message « HALTE borne : N lignes pour 540 ».
- [ ] `git diff --name-only origin/main...HEAD` ne montre que
      `crates/mika-cli/src/commands/chat.rs`,
      `crates/mika-cli/src/commands/session_remote.rs`,
      `crates/mika-cli/CLAUDE.md` et le plan.
- [ ] `bash scripts/verify-egress-manifest.sh --report` vert. **S'il est rouge, le
      pilote s'arrête et le dit** — il ne touche pas `docs/egress/`.
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] La PR porte `Refs mika#1727`, et **aucun** `Closes`, `Fixes` ni `Resolves`,
      ni dans le corps ni dans un commit — y compris si le gabarit `wip-rescue`
      l'ouvre (il écrit `Closes #N` par défaut : corrigé à la main sur #3077 et
      #3081). Vérifié par `gh pr view <N> --json closingIssuesReferences` rendant
      `[]`.
- [ ] Le corps de la PR porte `Code review: delegated to mika-qa (mika#2694)` et
      les lignes `Tracked in: senara-solutions/mika#1727` du § 7.
- [ ] **Aucun ticket n'est ouvert** (gel jusqu'au 01/12), pas même de suivi.
- [ ] Si un hook ou un classifieur refuse un geste, au pilote ou à un sous-agent :
      **arrêt et déclaration**, même si cela semble un faux positif. Aucun
      contournement par un autre outil (pas de heredoc Bash à la place de `Write`,
      pas de reformulation) — incident mika#2869 du 2026-10-21.

---

## 6. Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#1727, § *Critères de V2d-4a* :

1. Sans `MIKA_INTERNAL_TOKEN`, l'ouverture refuse **avant tout appel**. Le refus
   nomme `MIKA_INTERNAL_TOKEN` et dit que `MIKA_DASHBOARD_TOKEN` ne suffit pas. Un
   test le montre avec une source vide, et un autre avec une source qui ne porte
   que `MIKA_DASHBOARD_TOKEN`. Dans les deux, le banc ne reçoit aucune requête.
2. Avec le jeton, l'ouverture envoie un POST sur `{base}/api/v1/sessions`, avec
   `Authorization: Bearer <jeton>` et un corps qui porte `agent`. Un test lit ces
   trois faits sur la socket du banc, et montre que la session rendue est celle que
   prend la TUI.
3. `canonical: true` donne `is_singleton_session` vrai, et `canonical: false` le
   donne faux. Un test pour chaque cas.
4. Sur un 401, l'erreur nomme `MIKA_INTERNAL_TOKEN`. Sur un point injoignable
   (`DeadEndpoint`), elle nomme l'URL. Un test pour chaque cas. Au démarrage, ces
   erreurs remontent par `spawn_agent_worker` et le `?` de `run`, avant l'écran.
5. Avec `--session-id`, une session inconnue est refusée en nommant l'identifiant,
   et aucune ligne n'est créée en base. Un test le montre.
6. La garde structurelle refuse `create_session(` et
   `get_or_create_canonical_session(` dans la moitié production de
   `commands/chat.rs`. Son allowlist est vide et assertie vide, et son témoin de
   lecture est présent. Le pilote montre son rouge-avant sur le code de main, puis
   son vert.
7. Le message de la sonde de démarrage ne nomme plus `MIKA_INTERNAL_TOKEN` comme
   cause possible d'un `None`.
8. Le pilote lance `cargo clippy -p mika-cli --all-targets`. Le résultat est vert,
   sans `allow`, et sans attente insatisfaite. Les `expect(dead_code)` de
   `CreatedSessionMirror` et d'`ouvrir` ont disparu. Ceux qui restent portent
   `reason = "câblé par V2d-4, mika#1727"`, et la doc du module annonce leur compte
   exact.
9. Seuls `commands/chat.rs`, `commands/session_remote.rs` et
   `crates/mika-cli/CLAUDE.md` changent. Le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le
   trouve rouge, il s'arrête et le dit.
10. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
    compris si le gabarit de reprise l'ouvre.

---

## Fire-Disposition

**Le détecteur livré par ce vol est la garde structurelle de D7** (T10,
`mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local`), et sa disposition est
l'**option (a) — exception nommée en allowlist**, dans sa forme **à zéro entrée** :
`const ALLOWED: &[&str] = &[]`, assertie vide au premier rang du test.

Les quatre propriétés exigées par mika#1574 :

- **chaque violation reçoit son entrée** — la population est vide *parce qu'elle a
  été vidée*, et non parce qu'elle n'a pas été regardée : § 0 mesure que les deux
  seules occurrences de `chat.rs` sont l. 103 et l. 110, et le commit 1 les retire.
  La garde est donc posée verte sur un fichier sans dette ;
- **l'entrée nomme la donnée précise** — à zéro entrée, c'est le **motif** qui
  porte la précision : chaque couple `(motif, porte)` nomme l'appel refusé *et* la
  porte qui le remplace (`session_remote::ouvrir`), donc le message d'échec donne
  le remède sans demander de lire le plan ;
- **elle référence un ticket de suivi** — le nom du test porte `mika1727_v2d4a`,
  grep-visible par `rg 'mika1727_v2d4a' crates/mika-cli/src/commands/chat.rs` ;
- **elle rougit quand elle devient stale** — c'est l'`assert!` sur `ALLOWED` doublé
  du **témoin de lecture** (`spawn_agent_worker`). Une allowlist à zéro entrée ne
  peut pas devenir stale par péremption d'une exception ; le vrai mode de panne est
  la **vacuité silencieuse** (chemin faux, fichier renommé, `#[cfg(test)]` remonté),
  et c'est le témoin qui la transforme en rouge au lieu d'un vert gratuit.

**La règle de remédiation, écrite ici et dans le test.** Quand la garde rougit, le
remède est de router l'appelant par `session_remote::ouvrir` — **jamais** d'ajouter
une ligne à `ALLOWED`. Une allowlist qui cesse d'être vide ici réintroduit
exactement le faux vert que la garde existe pour empêcher (forme mika#2425 /
mika#2457).

**Le second détecteur du vol est l'`expect(dead_code)`**, et sa disposition est
celle de V2d-3a et V2d-3b, **reconduite sans changement** : l'attribut lui-même
posé sur l'item qu'il couvre, `expect` et jamais `allow` — au câblage, il devient
`unfulfilled_lint_expectation` et le build le dit. Ce vol en **retire** deux et
n'en ajoute aucun ; D9 impose que le retrait voyage dans le commit qui câble.

**Les neuf autres tests du § 4 ne sont pas des détecteurs**, et c'est une
évaluation, pas une omission : leur chemin de succès est *« la porte a rendu ceci »*
ou *« la socket a vu cela »*, pas *« aucune violation n'a été trouvée »*. Ils n'ont
donc ni population à disposer ni allowlist à tenir.

---

## 7. Ce que ce vol ne fait pas, et la dette nommée

Noté dans la PR, sous `Tracked in: senara-solutions/mika#1727`. **Aucun ticket
n'est ouvert** : le gel court jusqu'au 01/12, et ce vol n'ouvre ni suivi ni
sous-ticket.

- **`/clear`, les sorties et `/compact` — V2d-4b.** Les trois `end_session(` de
  `chat.rs` (l. 744, 861, 960) et le `/clear` de `handlers.rs:147` restent locaux.
  Ils ne sont pas dans les motifs de la garde, délibérément : les y mettre la
  rendrait rouge le jour de sa pose. V2d-4b les ajoute **avec** leur câblage, et
  câble `clore` et `compacter`, dont les quatre `expect(dead_code)` restants
  tiennent la place.
- **La mise en commun de `jeton_interne` et `SourceEnv` — V2d-4b.** Deux copies
  existent désormais, dans `chat.rs` et dans `handlers.rs:1229-1239`. Le ticket
  l'autorise explicitement, parce que `handlers.rs` **ne change pas** dans ce vol
  et que V2d-4b le touche de toute façon. La dette est bornée à un vol, et sa
  forme est identique des deux côtés, donc la réunion sera un déplacement et non
  une réconciliation.
- **`dashboard::auth_token()` accepte toujours `MIKA_DASHBOARD_TOKEN`.**
  `dashboard.rs` reste intact : ce vol ferme l'angle mort **au démarrage de la
  TUI**, là où il mordait, pas dans la sonde du tableau de bord, dont c'est le
  contrat légitime. Restreindre `auth_token` toucherait un fichier exclu et des
  appelants hors périmètre.
- **`commands/ask.rs` n'est pas la TUI** et ne passe pas par ce chemin.
- **Le `--session-id` sur une session inconnue n'a pas de chemin de création.** La
  route ne prend aucun identifiant dans son corps ; en donner un serait un
  changement de contrat côté serveur, hors de ce vol et hors de la série V2d.
- **La sonde de démarrage reste une sonde.** Elle n'est pas remplacée par
  l'ouverture, qui l'a pourtant précédée : les deux questions diffèrent (*le
  tableau de bord est-il servi* / *la session s'ouvre-t-elle*), et les fondre
  reproduirait l'écrasement que V1 § D4 a défait.

---

## Taille estimée

| livrable | lignes ajoutées |
|---|---|
| `chat.rs` — `SourceEnv` + `jeton_interne` (doc et commentaires compris) | 22 |
| `chat.rs` — `ouvrir_session_par_spirit`, les deux bras (commentaires compris) | 62 |
| `chat.rs` — le site d'appel dans `spawn_agent_worker`, net du bloc retiré | 10 |
| réécriture en ajouts du bloc l. 95-114 re-routé (le `numstat` le compte deux fois) | 20 |
| `chat.rs` — le message de la sonde et son commentaire, réécrits (§ 2.2) | 12 |
| `chat.rs` — le module `tests` : fixture, banc et les neuf tests de comportement | 170 |
| `chat.rs` — la garde structurelle (T10) | 32 |
| `session_remote.rs` — les deux `expect` retirés et la doc du module réécrite | 14 |
| `crates/mika-cli/CLAUDE.md` — le paragraphe neuf et la réconciliation du § V1 | 24 |

Total estimé : 366 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités, et laquelle mord.** La borne du ticket (540) porte sur
`git diff --numstat origin/main...HEAD` plan exclu, **doc et `CLAUDE.md`
compris** : 366, soit une marge de 174 lignes. La borne par vol (429) porte sur
les lignes hors `docs/` et hors `CLAUDE.md` : 342 à cette mesure, mais le total
est **écrit dans l'unité inclusive du ticket**, donc le chiffre que la machine lit
(366) majore celui qu'elle borne, et le tient quand même sous 429 avec 63 lignes
de marge (~15 %). Tenir la plus serrée des deux dans l'unité la plus large est la
seule combinaison qui ne demande à personne de deviner laquelle a été comptée. La
borne par PR (1000) ne mord pas.

**La ligne de réécriture est déclarée exprès**, parce que les deux bornes comptent
des **lignes ajoutées** et non un delta net : les vingt lignes du bloc l. 95-114,
remplacées par l'appel de § 2.1, comptent en ajouts tout en disparaissant du
fichier. L'omettre ferait que la première mesure du pilote dépasserait son
estimation sans qu'aucun travail imprévu n'ait eu lieu — la classe d'erreur que
mika#2693 existe pour supprimer.

**Pourquoi un seul vol tient.** Quatre raisons mesurées en § 0, et non optimistes.
*La porte est déjà là, verte et testée* — `ouvrir`, `post`, `session_url`,
`SessionRemoteIssue` et son `Display` sont livrés par V2d-3a, et ce vol
n'introduit **aucun** statut, aucune route et aucun miroir neuf : il pose un
appelant. *Le banc est déjà emprunté par le voisin* — `fake_spirit`,
`RequeteRecue` et `DeadEndpoint` sont en place et `pub(crate)` depuis V2c-1b, donc
~90 lignes de factice ne sont pas dépensées, et `rewind_remote.rs` reste intact.
*Les trois sites d'appel ne changent pas* (D5) — le `?` du démarrage est déjà
avant l'écran, et les deux chemins d'erreur de la TUI existent : la moitié du
critère 4 est une démonstration, pas un travail. *Aucun fichier voisin et aucune
dépendance* — `commands/mod.rs` porte déjà la ligne, `docs/egress/` ne bouge pas,
`mika-common` + `test-utils` est déjà en `[dev-dependencies]`.

Le poste le plus lourd est les tests (202 pour 126 de production), et c'est
conforme : le ticket exige un test par fait de socket, un test par cas de
`canonical`, un test par erreur, plus le contrôle positif de T2 — et « les tests
ne se reportent jamais à un vol suivant, même si le code reste ». Si le budget ne
tenait pas avec eux, c'est le périmètre qui se réduirait, jamais les tests.

Pas de découpage en vols supplémentaires : la suite est déjà découpée par le corps
du ticket en V2d-4b, V3 et V4.
