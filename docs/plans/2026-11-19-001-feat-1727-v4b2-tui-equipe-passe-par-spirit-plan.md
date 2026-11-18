# Vol V4-b2 — le mode équipe de la TUI passe par spirit (mika#1727)

- **Ticket :** senara-solutions/mika#1727, vol **V4-b2**
- **Branche :** `feat/1727/v4b2-tui-equipe-passe-par-spirit`, neuve depuis `main` (`32bcc6b7`)
- **PR :** `Refs mika#1727`, jamais `Closes` / `Fixes` / `Resolves` — y compris si le
  gabarit de reprise (`wip-rescue`) ouvre la PR : ce gabarit écrit `Closes #N` par
  défaut, celui qui le voit le corrige avant toute revue, puis vérifie que
  `gh pr view <N> --json closingIssuesReferences` rend `[]`.
- **Suite :** `Tracked in: senara-solutions/mika#1727`
- **Label de borne :** `loop-substrate`

## Contexte

V4-a (#3109, `9da388ac`) a livré la route `POST /api/v1/teams/{name}/runs` et son flux
SSE de `TeamRunFrame`. V4-b1 (#3110, `32bcc6b7`) a livré le client de cette route —
`team_runs_remote::lancer_run_equipe` — et y a branché `mika ask --team`.

**Le mode équipe de la TUI fait encore tourner l'équipe dans son propre processus.** Le
worker de `commands::chat::run_team` appelle `mika_agent::teams::run_team`
(`crates/mika-cli/src/commands/chat.rs:1148`) avec les `Settings`, l'App GitHub, la base
d'équipe et le tier/déploiement lus du process CLI. Ce vol le branche sur le client déjà
livré. Après lui, **plus rien dans la moitié production de `mika-cli` n'appelle
`teams::run_team`**, et une garde du crate le tient.

V4 se livre en quatre vols : V4-a (mergé), V4-b1 (mergé), **V4-b2 (ce vol)**, V4-c (le
verrou structurel). **Le ticket se ferme au merge de V4-c, et seulement là.**

### Ce qui est mesuré sur main (`32bcc6b7`)

| fait | site |
|---|---|
| `lancer_run_equipe(base_url, token, team_name, goal, reference_run_id, on_event) -> Result<TeamRun, TeamRunRemoteIssue>`, `pub(crate)`, aucun délai global, statut lu avant le corps, client construit au site de l'appel | `crates/mika-cli/src/commands/team_runs_remote.rs:133` |
| `TeamRunRemoteIssue` : cinq variants `Denied` / `Refus` / `Transport` / `SansTrameFinale` / `RunError`, `Display` **à la main** | `team_runs_remote.rs:46` et `:68` |
| Le nom d'équipe entre dans le chemin par `format!`, **sans validation**, et un commentaire de quatre lignes assume l'écart | `team_runs_remote.rs:141-148` |
| `flux(trames)` → corps SSE joint par `data: {t}\n\n` ; `contre_le_factice(corps)` → `(Vec<TeamEvent>, Result<TeamRun, TeamRunRemoteIssue>)` servi par `rewind_remote::tests::fake_spirit` — les deux **privées au module de test** | `team_runs_remote.rs:241` et `:251` |
| Le worker du mode équipe : `Settings::load`, `GitHubApp::from_settings`, puis par `TeamRequest::Goal` un appel à `mika_agent::teams::run_team` avec `AgentTier::from_env()` et `Deployment::from_env()` | `crates/mika-cli/src/commands/chat.rs:1128-1225`, l'appel en `:1148` |
| La traduction du `TeamRun` : `team_outcome::classify` → `RunFailed` (avec `PARTIAL_OUTPUT_MARKER`) ou `Deliverable` | `chat.rs:1167-1214` |
| Canaux du mode équipe : `mpsc::unbounded_channel::<TeamRequest>()` et `::<TeamEvent>()` | `chat.rs:1115-1116` |
| `jeton_interne(SourceEnv<'_>) -> Result<String, String>` — **seule lecture** de `MIKA_INTERNAL_TOKEN` du crate, et son refus la nomme ; `SourceEnv<'a> = &'a dyn Fn(&str) -> Option<String>` | `crates/mika-cli/src/commands/session_remote.rs:177` et `:147` |
| `spirit_url()` — `MIKA_SPIRIT_URL`, sinon `~/.mika/.env` puis `MIKA_SPIRIT_PORT`, défaut `http://localhost:8080` ; ne peut pas échouer | `crates/mika-cli/src/commands/dashboard.rs:18` |
| La forme canonique « tout ce qui touche le monde arrive par argument » : `ouvrir_session_par_spirit`, `lire_les_skills`, `clore_session_de_sortie` prennent `lire_env: SourceEnv<'_>` | `chat.rs:69`, `:136`, `:165` |
| `main.rs` charge le `.env` de la home **avant** d'appeler `run_team` (branche `--team`), normalise puis valide le nom d'équipe, et vérifie le format UUID de `--run-id` | `crates/mika-cli/src/main.rs:117`, `:114-115`, `:144-151`, appel en `:155` |
| `validate_team_name(&str) -> anyhow::Result<()>` : cinq refus par `bail!` (vide, > 32 caractères, tiret en tête ou en queue, `--` consécutifs, caractère hors `[a-z0-9-]`) | `crates/mika-common/src/team.rs:56` |
| La garde de V4-b1 balaie `ask.rs` **seul**, et son doc nomme `chat.rs` comme l'appel restant | `crates/mika-cli/src/commands/ask.rs:1002-1051` |
| Le gabarit d'une garde **par crate** : `ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"))` + `src_root()` + `for_each`, anti-vacuité par `files()`, allowlist « livrée vide, et le reste » | `crates/mika-cli/src/commands/chat_turn.rs:1294`, `:1346`, `:1368` |
| Les deux seules occurrences de `teams::run_team(` sous `crates/mika-cli/src/` : `chat.rs:1148` (l'appel réel) et `ask.rs:1002` (une ligne `///` de la **moitié test**, donc masquée par `production_of`) | `grep -rn "teams::run_team(" crates/mika-cli/src` |
| `crates/mika-cli/CLAUDE.md` pèse **92 282 octets** pour un plafond de 100 000 | `crates/mika-common/tests/claude_md_size.rs` |

### Trois réutilisations, et c'est elles qui font tenir le vol

1. **Le client existe.** Ce vol n'écrit ni découpeur SSE, ni miroir de types, ni
   transport : `lancer_run_equipe` est livré, attesté par huit tests, et sa seule
   lacune est la validation du nom.
2. **Le banc de test existe.** `flux` et `contre_le_factice` passent `pub(crate)` — deux
   lignes — et les tests du worker servent leurs corps SSE par le même `fake_spirit`.
   Aucun banc neuf.
3. **La traduction existe.** `team_outcome::classify` et les quatre bras qui en sortent
   (`chat.rs:1167-1214`, commentaires compris) **se déplacent sans se récrire**. Ce sont
   des lignes déplacées, pas ajoutées, et c'est pourquoi l'extraction du worker coûte
   le branchement et non la traduction.

---

## Requirements

### R1 — La porte valide le nom avant de bâtir l'URL (défaut 38)

Dans `crates/mika-cli/src/commands/team_runs_remote.rs` :

- Un variant neuf de `TeamRunRemoteIssue` :

  ```rust
  NomInvalide { team_name: String, cause: String },
  ```

  `team_name` **et** `cause` : le critère 1 exige que le refus nomme le nom et la
  cause. Le nom, parce qu'un opérateur qui voit `'../x'` sait immédiatement ce qu'il a
  tapé ; la cause, parce que `validate_team_name` a cinq refus distincts et que « nom
  invalide » tout court renvoie l'opérateur lire le code du juge.

- Son bras de `Display`, dans le même `match` que les cinq autres :

  ```rust
  Self::NomInvalide { team_name, cause } => write!(
      f,
      "'{team_name}' is not a valid team name, so no request was made: {cause}"
  ),
  ```

  « so no request was made » est porteur et n'est pas de l'ornement : ce variant est le
  seul des six qui ne porte **ni URL ni statut**, pour la raison de mika#2331 — rien n'a
  été tenté, et `Transport { url, cause }` se lirait « spirit injoignable » là où aucun
  octet n'est parti. Le `Display` doit donc dire lui-même qu'il n'y a pas de réseau à
  regarder, comme `RunError` dit que le transport n'est pas en cause.

- La validation, **première instruction** de `lancer_run_equipe`, avant le `format!` :

  ```rust
  mika_common::team::validate_team_name(team_name).map_err(|cause| {
      TeamRunRemoteIssue::NomInvalide {
          team_name: team_name.to_string(),
          cause: cause.to_string(),
      }
  })?;
  ```

  `map_err` et non un `?` direct : `validate_team_name` rend `anyhow::Result<()>`, et la
  porte rend `TeamRunRemoteIssue`.

- Le commentaire de `:141-148` est **récrit**, pas supprimé. Il assumait l'écart ; il doit
  maintenant dire pourquoi la porte double un juge qui tourne déjà côté serveur :

  > Le nom va dans le chemin par `format!`, donc un caractère réservé produirait un
  > **autre chemin** — un 404, ou une route voisine — là où la route aurait rendu son
  > 400. Le juge serveur ne peut pas refuser une requête qui ne l'atteint pas. La porte
  > appelle donc le même `team::validate_team_name` avant de composer l'URL : ce n'est
  > pas une seconde politique, c'est le même juge appelé du côté où le nom devient un
  > chemin.

  C'est le pourquoi qu'on ne lit pas dans le code (mika#2201 : le remède est de router
  par le juge, pas de documenter l'écart).

**Ce que R1 ne fait pas :** il ne touche pas `mika_common::team` (fichier exclu), ne
change aucune signature, et ne touche pas `main.rs:114-115` — la validation de `main.rs`
reste, et la porte devient la seconde ligne de défense, celle qui tient pour tout
appelant futur.

### R2 — `flux` et `contre_le_factice` en `pub(crate)` (deux lignes)

Les tests du worker (R5) servent des corps SSE et les relèvent par le même banc. Deux
aides partagées, zéro banc neuf. Changement minimal : `fn flux` → `pub(crate) fn flux`,
et `async fn contre_le_factice` → `pub(crate) async fn contre_le_factice`. Rien d'autre
ne bouge dans ce module de test.

`contre_le_factice` rend `(Vec<TeamEvent>, Result<TeamRun, TeamRunRemoteIssue>)` et
appelle `lancer_run_equipe` elle-même : elle sert donc les tests **du client**. Les tests
du worker (tests 1 à 4) ont besoin du factice et des requêtes relevées, pas de cette
porte-là. Ils appellent donc `rewind_remote::tests::fake_spirit` directement — déjà
`pub(crate)` — et réutilisent **`flux`** pour composer le corps. Seul `flux` est donc
strictement nécessaire ; `contre_le_factice` passe `pub(crate)` dans le même geste
seulement si le test 5 (le refus du nom) y trouve son compte. **Si un seul suffit, un
seul passe :** une promotion de visibilité sans lecteur est une extension d'API gratuite.

### R3 — Le worker d'équipe sort de `run_team` et se branche (défauts 40 et 42)

Dans `crates/mika-cli/src/commands/chat.rs`, une fonction neuve, au-dessus de
`run_team` :

```rust
/// Le worker du mode équipe de la TUI : chaque but part à mika-spirit
/// (mika#1727, vol V4-b2), là où il lançait l'équipe dans ce processus.
///
/// Sortie du `tokio::spawn` pour la raison de `ouvrir_session_par_spirit` : tout
/// ce qui touche le monde arrive **par argument**, donc un test joue le worker
/// entier contre `fake_spirit` sans muter l'environnement. C'est la forme que
/// V3c-2c a posée pour le worker d'agent (adresse et lecteur de jeton par
/// argument du spawn).
///
/// Les arguments sont possédés (`String`, non `&str`) parce que le seul appelant
/// les passe à un `tokio::spawn` qui leur survit.
pub(crate) async fn servir_les_buts_dequipe(
    base_url: String,
    token: String,
    team_name: String,
    reference_run_id: Option<String>,
    mut team_rx: mpsc::UnboundedReceiver<TeamRequest>,
    response_tx: mpsc::UnboundedSender<TeamEvent>,
)
```

Son corps :

- `while let Some(request) = team_rx.recv().await`, `TeamRequest::Quit => break` :
  **inchangés**, la boucle est la même.
- Pour `TeamRequest::Goal(goal)` : le rappel est celui qui existe déjà
  (`chat.rs:1143-1146`, `let tx = response_tx.clone(); move |event| { let _ = tx.send(event); }`),
  **déplacé tel quel** — c'est lui qui pousse chaque `TeamEvent` sur le canal de la TUI
  dans l'ordre reçu (critère 2), et `lancer_run_equipe` l'appelle pour chaque trame
  décodée dans l'ordre reçu (`team_runs_remote.rs:124`). La propriété d'ordre n'est donc
  pas à construire : elle tient par les deux bouts, et le test 2 la mesure.
- L'appel :

  ```rust
  match crate::commands::team_runs_remote::lancer_run_equipe(
      &base_url,
      &token,
      &team_name,
      &goal,
      reference_run_id.as_deref(),
      callback,
  )
  .await
  ```

- `Ok(run)` → le bloc `team_outcome::classify` de `chat.rs:1167-1214` **déplacé sans une
  modification**, commentaires compris (critère 2 : « comme aujourd'hui », critère 6 :
  les messages enregistrés par `tui/app.rs` sont inchangés). Les quatre bras restent
  `Failed` → `RunFailed` avec `PARTIAL_OUTPUT_MARKER`, `Delivered` → `Deliverable`,
  `CompletedWithoutDeliverable` et `Pending { .. }` → `Deliverable(String::new())`.
- `Err(issue)` → `TeamEvent::RunFailed(format!("{issue}"))`. **Un seul bras d'erreur, et
  aucun repli local** (critère 3). Les six variants de `TeamRunRemoteIssue` y passent,
  `RunError` et `SansTrameFinale` comprises : c'est ce qui fait que ni un `run_error` ni
  un flux tronqué ne peuvent produire un `Deliverable`, et donc que le mode équipe ne
  persiste jamais un échec en tour `assistant` durable — le motif écrit sur
  `chat.rs:1168-1178`, qui vaut mot pour mot ici.

**Ce qui part avec l'appel** (critère 6) : `Settings::load` et son bras d'erreur
(`chat.rs:1129-1137`), `GitHubApp::from_settings` (`:1139`), `worker_home`,
`worker_team_db`, `AgentTier::from_env()` (`:1159`) et `Deployment::from_env()` (`:1161`).

**Ce qui reste dans `run_team`** (critère 6, défaut 42) : `team::team_dir`, les deux
`mpsc::unbounded_channel`, `container_db_path` + `create_dir_all` +
`AsyncDatabase::open` — `team_db` garde un lecteur, le résumé du run de référence
(`chat.rs:1228-1264`) —, `App::new_team`, `is_dashboard_running().await` (`:1293`), le
hook de panique, `enable_raw_mode`, la boucle d'écran et la fermeture.

Dans `run_team`, le `tokio::spawn` devient un appel :

```rust
let team_worker_handle = tokio::spawn(servir_les_buts_dequipe(
    base_url,
    token,
    team_name.to_string(),
    run_id.map(|s| s.to_string()),
    team_rx_worker,
    response_tx,
));
```

`team_rx_worker` n'a donc plus besoin d'être `mut` à son `let` — la mutabilité part dans
la signature du worker.

### R4 — `run_team` lit l'adresse et le jeton avant l'écran (défaut 39)

Une fonction neuve dans `chat.rs`, à côté des trois lectures de la même forme :

```rust
/// L'adresse de mika-spirit et le jeton interne du mode équipe, lus **avant**
/// le hook de panique et avant `enable_raw_mode` (mika#1727, vol V4-b2).
///
/// Avant, parce qu'un refus qui attendrait le premier but se produirait
/// terminal rendu, écran ouvert, hook de panique posé : l'opérateur verrait
/// « run failed » dans une TUI là où la cause est une variable absente de son
/// shell. Le `?` de l'appelant est ce qui garantit que l'écran ne s'affiche pas
/// (critère 4) — c'est l'ordre des instructions dans `run_team` qui porte la
/// propriété, et ce test-ci atteste le refus et son texte.
///
/// `lire_env` arrive par argument, comme sur `ouvrir_session_par_spirit` : un
/// lecteur qui résout ses propres accès ne se pilote pas depuis un test.
///
/// `spirit_url()` d'abord, `jeton_interne` ensuite, et l'ordre n'a rien
/// d'accidentel pour qui relit : `spirit_url()` charge `~/.mika/.env` quand
/// `MIKA_SPIRIT_URL` est absente. En production le `.env` est déjà chargé par
/// `main.rs` (branche `--team`) avant l'appel, donc l'ordre ne porte rien
/// aujourd'hui — mais l'inverse créerait une dépendance silencieuse au
/// chargement de l'appelant.
fn acces_du_mode_equipe(lire_env: SourceEnv<'_>) -> Result<(String, String)> {
    let base_url = crate::commands::dashboard::spirit_url();
    // `map_err` et non un `?` direct : la lecture commune rend un `String`,
    // pour la raison écrite sur `jeton_interne`. Le refus nomme
    // `MIKA_INTERNAL_TOKEN` parce que c'est elle qui le nomme.
    let jeton = jeton_interne(lire_env).map_err(anyhow::Error::msg)?;
    Ok((base_url, jeton))
}
```

Et dans `run_team`, **avant** la création des canaux et avant le hook de panique :

```rust
let (base_url, token) = acces_du_mode_equipe(&|cle| std::env::var(cle).ok())?;
```

`SourceEnv` et `jeton_interne` sont déjà importés (`chat.rs:55`). La signature de
`run_team` **ne change pas** (critère 10 : `main.rs` ne change pas).

### R5 — Les sept tests, contre le factice, sans fournisseur réel

Dans le module `tests` de `chat.rs` (tests 1 à 4 et 6) et de `team_runs_remote.rs`
(tests 5 et 7), nommés `mika1727_v4b2_*`, sur le gabarit de
`mika1727_v3c2c_apres_un_setmodel_le_tour_suivant_porte_loverride` (`chat.rs:2061`) :
un `fake_spirit`, l'adresse et le jeton **par argument** du worker, aucune mutation
d'environnement.

1. **`un_but_part_en_post_vers_la_route_avec_le_jeton_et_le_corps`** — un `TeamRequest::Goal`
   envoyé sur le canal du worker produit, chez le factice, **une** requête : méthode
   `POST`, chemin `/api/v1/teams/{nom}/runs`, `authorization: Bearer <jeton>`, et un corps
   dont `goal` est le but. (Critère 2.)
2. **`les_trois_evenements_arrivent_dans_lordre_puis_le_livrable`** — un corps SSE de trois
   `team_event` puis un `run_completed` portant un livrable : le canal de la TUI reçoit
   les trois `TeamEvent` **dans l'ordre émis**, puis `TeamEvent::Deliverable(livrable)`.
   L'ordre est asserté sur la séquence entière, pas par `contains` : un test qui
   vérifierait « les trois sont là » passerait sur un worker qui les rend à l'envers.
   (Critères 2 et 3.)
3. **`un_run_error_donne_un_runfailed_qui_porte_lerreur`** — une trame `run_error` donne
   `TeamEvent::RunFailed` dont le message porte l'erreur du run, et **aucun**
   `Deliverable` sur le canal. (Critère 3.)
4. **`un_flux_ferme_sans_trame_finale_donne_un_runfailed`** — un corps SSE qui s'arrête
   après deux `team_event` donne `TeamEvent::RunFailed`, et **aucun** `Deliverable`.
   (Critère 3.)
5. **`la_porte_refuse_un_nom_invalide_sans_aucune_requete`** — `lancer_run_equipe` avec
   `"../x"` rend `TeamRunRemoteIssue::NomInvalide`, le `Display` porte `../x` et la
   cause, et le factice relève **zéro** requête. L'assertion sur le compte de requêtes
   est la moitié portante : un refus rendu *après* l'appel passerait un test qui ne
   regarderait que le variant. (Critère 1.)
6. **`sans_jeton_la_lecture_qui_precede_lecran_refuse_en_nommant_la_variable`** —
   `acces_du_mode_equipe(&|_| None)` rend un `Err` dont le message contient
   `MIKA_INTERNAL_TOKEN`. Gabarit exact :
   `mika1727_v2d4a_sans_jeton_interne_le_demarrage_refuse_avant_tout_appel`
   (`chat.rs:1427`). (Critère 4.)
7. **`aucune_ligne_de_production_de_mika_cli_nappelle_teams_run_team`** — la garde du
   crate, décrite en R6. (Critère 5.)

### R6 — La garde du crate (défaut 41) et le retrait de celle de V4-b1

La garde vit dans le module `tests` de `crates/mika-cli/src/commands/team_runs_remote.rs`
— **à côté de la porte de remplacement**, suivant le précédent `mika1727_v2a_*`
(`chat_turn.rs:1294`), où la garde vit chez le voisin qui offre le chemin légitime. Trois
tests, le triplet canonique du dépôt :

```rust
/// L'allowlist du vol V4-b2 : vide, et le reste.
const V4B2_ALLOWED: &[&str] = &[];
```

1. **La garde.** `const NEEDLE: &str = concat!("teams::", "run_team", "(");` —
   `concat!` et non le littéral, par-dessus le masquage : la garde vit dans la moitié
   test, donc `production_of` la masque déjà, mais un `concat!` la rend insensible à une
   régression du masquage lui-même, et c'est la forme de la garde de V4-b1.
   `ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"))`, `src_root()`, puis
   `for_each(|path, production| …)` : le chemin relatif, le saut des entrées de
   `V4B2_ALLOWED`, le saut des lignes dont le `trim_start` commence par `//`, et la
   collecte des violations en `fichier:ligne — texte`. Le message d'assertion **nomme le
   remède** : router l'appelant par `team_runs_remote::lancer_run_equipe`, et jamais
   élargir `V4B2_ALLOWED` (mika#2201).
2. **L'anti-vacuité** (mika#2103, mika#2205, mika#2398). `scanner.files()` compte plus de
   dix fichiers, et contient **`commands/chat.rs`** et **`commands/ask.rs`** — les deux
   témoins, par leur chemin avec `commands/` et non par le suffixe nu, parce que
   `Path::ends_with` compare par composants. Ce sont les deux moitiés de la propriété :
   le fichier d'où l'appel part dans ce vol, et celui d'où il est parti en V4-b1. Un
   balayage qui ne les voit pas n'inspecte pas ce qu'il prétend.
3. **Le contre-témoin de masquage.** `production_of` du fichier qui porte la garde ne
   contient pas le nom du test de la garde — preuve que la moitié `#[cfg(test)]` est
   bien masquée, et donc que le vert du point 1 n'est pas le vert d'un scan qui lit
   tout. Et `V4B2_ALLOWED.is_empty()` est asserté (mika#2309 : une allowlist peuplée ne
   refuse plus rien).

**Le retrait de la garde de V4-b1.** `mika1727_v4b1_ask_ne_fait_plus_tourner_lequipe_en_local`
(`ask.rs:1011-1051`) et son doc-comment (`:1002-1010`) partent : la garde du crate les
subsume strictement — elle balaie `ask.rs` **et** tout le reste, avec le même needle et
la même allowlist vide. Les garder serait une garde redondante dont le doc nomme
`chat.rs` comme l'appel restant, c'est-à-dire un commentaire faux. C'est le **seul**
changement d'`ask.rs` dans ce vol (contrainte de la tête). Ces lignes sont des retraits,
et les retraits ne comptent pas dans la borne.

### R7 — La doc (`crates/mika-cli/CLAUDE.md`)

Les deux phrases qui disent que la TUI part en V4-b2 ne le disent plus :

- **l. 181-183** (paragraphe de V4-b1) : « La TUI, elle, part au vol V4-b2 ;
  `commands/chat.rs` appelle donc encore `teams::run_team(` et c'est le régime nominal,
  ce qui est aussi pourquoi la garde textuelle du vol balaie `ask.rs` **seul**. » →
  devient le fait d'après ce vol : plus aucun appel dans la production du crate, et une
  garde **du crate** qui le tient.
- **l. 479-480** (démarrage de V1) : « Le chemin `--team` garde `is_dashboard_running()`
  — son tour ne passe pas par ce bras, et il part au vol V4. » → `is_dashboard_running()`
  reste sur ce chemin (défaut 42), mais la seconde moitié de la phrase est caduque.

Et un paragraphe neuf, qui dit : que le mode équipe de la TUI passe par
`POST /api/v1/teams/{name}/runs`, qu'il exige `MIKA_INTERNAL_TOKEN` **avant** l'écran et
sort en erreur nommant la variable sans l'afficher, qu'il n'a **aucun repli local**
(`Settings::load`, l'App GitHub, `AgentTier::from_env()` et `Deployment::from_env()` ne
sont plus lus par le worker d'équipe), que la porte refuse un nom invalide **sans aucune
requête**, et ce qui reste local (défaut 42 : le résumé du run de référence lu dans la
base du conteneur, la sonde `is_dashboard_running()`, et la persistance des tours par
`tui/app.rs`).

Le fichier reste **sous 100 000 octets** : 92 282 aujourd'hui, et ce vol ajoute une
vingtaine de lignes. `cargo test -p mika-common --test claude_md_size` l'atteste
(critère 9).

---

## Périmètre — ce qui change, et rien d'autre

| fichier | ce qui y change |
|---|---|
| `crates/mika-cli/src/commands/team_runs_remote.rs` | R1 (variant, `Display`, validation, commentaire récrit), R2 (visibilité des aides de test), R6 (la garde du crate et ses deux compagnes) |
| `crates/mika-cli/src/commands/chat.rs` | R3 (le worker sorti et branché), R4 (la lecture avant l'écran), R5 (tests 1 à 4 et 6) |
| `crates/mika-cli/src/commands/ask.rs` | R6, le retrait de la garde de V4-b1 et de son doc-comment — **et rien d'autre** |
| `crates/mika-cli/CLAUDE.md` | R7, deux phrases et un paragraphe |

**Restent intacts :** tout `crates/mika-agent/`, tout `crates/mika-cli/src/tui/`,
`run_team_ask`, `commands/team_outcome.rs`, `main.rs`, `crates/mika-common/`,
`crates/mika-a2a/`, `crates/mika-gateway/`, `docs/egress/`, `dashboard/`.

**Aucun fichier du plan de contrôle :** ni `.github/workflows/`, ni `.claude/`, ni
`skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni `.mika/`, ni
`docs/egress/`, ni aucun chemin de CODEOWNERS.

**`docs/egress/` ne change pas** (critère 7), et ce n'est pas un oubli : l'entrée
`cli-spirit-team-runs` a été déclarée par V4-b1 au site de l'appel, dans
`team_runs_remote.rs`. Ce vol n'ajoute **aucun** site d'appel sortant — il en retire un
(l'appel local à `teams::run_team` n'était pas un egress) et pousse un appelant de plus
vers une porte déjà déclarée. `bash scripts/verify-egress-manifest.sh --report` doit
rester vert, et le pilote le lance.

---

## Ordre de commit

Chaque commit compile et passe ses tests, et **chaque changement arrive avec ses tests,
dans le même commit** — jamais reportés à un vol suivant.

1. **`feat(1727): la porte des runs d'équipe valide le nom avant de bâtir l'URL`** —
   R1 + R2 (la visibilité qu'exige son test) + le test 5. Le commit qui ferme le
   défaut 38, et il est autonome : la porte refuse déjà un nom invalide avant que le
   worker de la TUI y passe.
2. **`feat(1727): le mode équipe de la TUI passe par spirit`** — R3 + R4 + les tests 1 à
   4 et 6 + R6 (la garde du crate et le retrait de celle de V4-b1). La garde **doit**
   être dans ce commit et pas avant : c'est le commit qui retire la dernière violation
   (`chat.rs:1148`), donc celui où la garde atterrit armée et verte, sans allowlist à
   peupler. L'armer plus tôt la livrerait rouge ; plus tard laisserait une fenêtre où
   rien ne tient la propriété.
3. **`docs(1727): la doc de mika-cli dit que le mode équipe passe par spirit`** — R7.

Après **chaque** commit, le pilote mesure, avec cette commande seule et sans rien après
elle :

```
git diff --shortstat origin/main...HEAD -- . ':!docs/plans'
```

---

## Verification Contract

### Le test rougit sans le changement — trois mutations

Dans un **worktree détaché**, sur un commit vert, chacune **seule** ; la PR donne la
sortie de chacune (critère 8) :

1. **Retirer l'appel à `validate_team_name`** dans `lancer_run_equipe` → le test 5
   rougit : la porte compose l'URL avec `../x` et le factice reçoit une requête.
2. **Faire passer le rappel du worker sans pousser les `TeamEvent` sur le canal** (le
   rappel devient un corps vide) → le test 2 rougit : le canal de la TUI ne reçoit plus
   les trois événements avant le livrable.
3. **Traduire un refus du client en `Deliverable(String::new())` au lieu de
   `RunFailed`** dans le bras `Err(issue)` du worker → les tests 3 et 4 rougissent : un
   `run_error` et un flux tronqué produisent un `Deliverable`.

Le choix de ces trois mutations n'est pas décoratif : elles frappent les trois
propriétés que ce vol ajoute et que rien d'autre ne tient — la validation en amont de
l'URL, la transmission ordonnée des événements, et l'étanchéité entre échec et livrable
(celle dont le dommage survit à la session, puisque `tui/app.rs` ne persiste que sur le
bras `Deliverable`).

### Commandes

```
cargo build -p mika-cli --bin mika
cargo test -p mika-cli
cargo clippy --all-targets -p mika-cli
cargo test -p mika-common --test claude_md_size
bash scripts/verify-egress-manifest.sh --report
bash scripts/verify-pipeline.sh origin/main
```

`cargo build` **d'abord**, puis les tests (critère 12). `verify-pipeline.sh` avant
d'ouvrir la PR ; **s'il est rouge, le pilote s'arrête et le dit.**

---

## Definition of Done

- [ ] `lancer_run_equipe` appelle `mika_common::team::validate_team_name` en première
      instruction, et rend `TeamRunRemoteIssue::NomInvalide { team_name, cause }` sans
      aucune requête. Le commentaire de `:141-148` est récrit, pas supprimé.
- [ ] `servir_les_buts_dequipe` existe, reçoit l'adresse, le jeton, le nom d'équipe et le
      run de référence par argument, appelle `lancer_run_equipe`, pousse chaque
      `TeamEvent` sur le canal dans l'ordre reçu, et traduit le `TeamRun` par le bloc
      `team_outcome::classify` déplacé sans modification.
- [ ] Le bras `Err` du worker rend `TeamEvent::RunFailed` pour les six variants de refus.
      Aucun repli local : `Settings::load`, `GitHubApp::from_settings`,
      `AgentTier::from_env()` et `Deployment::from_env()` ne sont plus lus par le worker
      d'équipe.
- [ ] `run_team` appelle `acces_du_mode_equipe` avant les canaux, le hook de panique et
      `enable_raw_mode`. Sa signature n'a pas changé.
- [ ] Le résumé du run de référence, `App::new_team`, `is_dashboard_running()`, la boucle
      d'écran et la fermeture sont inchangés.
- [ ] Les sept tests passent. Aucun ne touche un fournisseur réel ni l'environnement du
      process.
- [ ] La garde du crate vit dans `team_runs_remote.rs`, avec son anti-vacuité (témoins
      `commands/chat.rs` et `commands/ask.rs`), son contre-témoin de masquage, et
      `V4B2_ALLOWED` assertie vide. La garde de V4-b1 et son doc-comment sont retirés
      d'`ask.rs`.
- [ ] `crates/mika-cli/CLAUDE.md` porte le paragraphe neuf et les deux phrases corrigées,
      et reste sous 100 000 octets.
- [ ] Les trois mutations sont jouées, chacune seule, dans un worktree détaché, et la PR
      donne la sortie de chacune.
- [ ] `cargo build -p mika-cli --bin mika`, `cargo test -p mika-cli`,
      `cargo clippy --all-targets -p mika-cli`,
      `cargo test -p mika-common --test claude_md_size`,
      `bash scripts/verify-egress-manifest.sh --report` et
      `bash scripts/verify-pipeline.sh origin/main` sont verts.
- [ ] Aucune attente (`#[ignore]`) ni aucun `allow` neuf.
- [ ] `git diff origin/main...HEAD` ne touche que les quatre fichiers du périmètre, plus
      ce plan.
- [ ] La PR porte `Refs mika#1727` et `Tracked in: senara-solutions/mika#1727`, et aucun
      `Closes` / `Fixes` / `Resolves`, ni dans le corps ni dans un commit.
- [ ] Le corps de PR s'écrit dans le worktree (`pr-body.md` à la racine,
      `--body-file pr-body.md`, puis `rm pr-body.md`), jamais dans `/tmp`, jamais par
      heredoc (mika#2211).

---

## Acceptance criteria

Les treize critères de V4-b2, verbatim de la tête du ticket :

1. La porte appelle `validate_team_name` avant de bâtir l'URL ; un nom invalide rend un
   refus distinct, qui nomme le nom et la cause, sans aucune requête.
2. Le worker d'équipe appelle `lancer_run_equipe`, pousse chaque `TeamEvent` sur le canal
   de la TUI dans l'ordre reçu, et traduit le `TeamRun` par `team_outcome::classify`
   comme aujourd'hui.
3. Un refus du client, un `run_error` ou un flux sans trame finale donnent
   `TeamEvent::RunFailed`, jamais `Deliverable`. Aucun repli local.
4. `run_team` lit l'URL et le jeton avant le hook de panique et l'écran ; sans jeton, il
   sort en erreur qui nomme `MIKA_INTERNAL_TOKEN`, et l'écran ne s'affiche pas.
5. Aucune ligne de la moitié production de `mika-cli` ne contient `teams::run_team(`, et
   une garde du crate le tient, allowlist vide et assertie vide.
6. `Settings::load`, l'App GitHub, `AgentTier::from_env()` et `Deployment::from_env()` ne
   sont plus lus par le worker d'équipe. Le résumé du run de référence et les messages
   enregistrés par `tui/app.rs` sont inchangés.
7. `docs/egress/` ne change pas, et `bash scripts/verify-egress-manifest.sh --report` est
   vert.
8. Les trois mutations du périmètre rougissent chacune un test. La PR donne la sortie de
   chaque mutation, faite seule dans un worktree détaché.
9. `crates/mika-cli/CLAUDE.md` porte le paragraphe et les deux phrases corrigées, et
   `cargo test -p mika-common --test claude_md_size` est vert.
10. `crates/mika-agent/`, `tui/`, `main.rs`, `run_team_ask` et `team_outcome.rs` ne
    changent pas. `git diff origin/main...HEAD` ne touche que les fichiers listés plus
    haut.
11. Aucune attente ni aucun `allow` neuf.
12. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-cli` est vert,
    et `cargo clippy --all-targets -p mika-cli` aussi. Le pilote lance
    `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il le trouve
    rouge, il s'arrête et le dit.
13. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si
    le gabarit de reprise l'ouvre.

---

## Fire-Disposition

Ce vol livre **un** détecteur : la garde du crate de R6 (test 7), dont le chemin de
succès est « aucune violation trouvée ».

**Option (a) — exception nommée en allowlist, livrée avec une allowlist vide et assertie
vide.**

- Le commit qui **arme** la garde (commit 2) est celui qui **retire** la dernière
  violation existante, `chat.rs:1148`. Il n'y a donc rien à excepter : la garde atterrit
  armée, verte, et `V4B2_ALLOWED` est livrée vide.
- Les deux seules occurrences de `teams::run_team(` sous `crates/mika-cli/src/` sont
  mesurées : `chat.rs:1148`, que ce vol supprime, et `ask.rs:1002`, une ligne `///` de la
  moitié `#[cfg(test)]` que `production_of` masque — et qui part de toute façon avec le
  retrait de la garde de V4-b1. Aucune troisième.
- `V4B2_ALLOWED.is_empty()` est **asserté**, pas seulement écrit (mika#2309 : une
  allowlist peuplée ne refuse plus rien, et une liste qu'aucun test ne garde vide se
  peuple par le premier appelant pressé). Précédents dans le même crate :
  `chat_turn.rs:1368` (« `V2A_ALLOWED` est livrée vide, et le reste ») et la garde de
  V3b-2.
- Le doc du test nomme le remède — router le nouvel appelant par
  `team_runs_remote::lancer_run_equipe` — et dit qu'ajouter une entrée d'allowlist n'est
  pas le remède (mika#2201).
- Le détecteur est **couvert par la CI** : il tourne dans `cargo test -p mika-cli`, que
  le job `Check` lance. Une garde que le pipeline n'exécute pas ne refuse rien
  (mika#2578, mika#2752) ; celle-ci est dans la suite unitaire du crate, pas dans un
  script à câbler.

**Option (b) — livrer désarmée : refusée.** Il n'y a rien à désarmer : zéro violation
après le commit 2, et une garde livrée désarmée serait un vert par cécité (mika#2398).

**Option (c) — halte et escalade : sans objet.** Rien n'oblige à s'arrêter ici : le total
estimé tient sous la borne du groom, et aucun critère ne demande de toucher un fichier
exclu.

**Le gel des tickets (jusqu'au 01/12) est respecté :** ce vol n'ouvre aucun ticket, pas
même de suivi ni de sous-ticket. Un manque ou un reste va en commentaire sur mika#1727
ou dans la PR, et la suite — V4-c, le verrou structurel — se note
`Tracked in: senara-solutions/mika#1727`.

---

## Taille estimée

| livrable | lignes ajoutées |
|---|---|
| R1 — variant `NomInvalide`, son bras de `Display`, la validation, le commentaire récrit | 32 |
| R2 — visibilité des aides de test | 2 |
| R3 — `servir_les_buts_dequipe` (signature, doc, boucle, branchement) **hors** bloc `classify` déplacé | 85 |
| R4 — `acces_du_mode_equipe` et son appel dans `run_team` | 18 |
| R5 — tests 1 à 4 et 6, sur mesure (les tests du client de V4-b1 pèsent ~195 lignes, `team_runs_remote.rs:225-419`) | 150 |
| R5 — test 5 (le refus du nom, sans requête) | 26 |
| R6 — la garde du crate, son anti-vacuité, son contre-témoin, l'allowlist assertie vide | 55 |
| R6 — retrait de la garde de V4-b1 (`ask.rs`) | 0 |
| R7 — la doc (`crates/mika-cli/CLAUDE.md`) | 20 |

Total estimé : 388 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Unité.** La ligne ajoutée, plan exclu, code, commentaires, doc et tests compris. Les
retraits ne comptent pas. La commande est, seule et sans rien après elle :
`git diff --shortstat origin/main...HEAD -- . ':!docs/plans'`, et le compte est le nombre
devant `insertions(+)`.

**Borne du groom : 420.** Posée par Vincent pour ce vol, dans la tête du ticket : « Si ce
total dépasse `420`, le groom s'arrête et le dit. Il ne recoupe pas lui-même. » Le total
estimé est **388**, sous la borne du groom (420) et sous la borne par vol (429). Le groom
n'a donc rien à recouper, et n'a rien recoupé.

**Halte du pilote : 590.** Posée par Vincent, et ce plan ne la change pas et n'en ajoute
aucune autre. Après chaque commit, le pilote mesure ; **si le compte dépasse 590, il
s'arrête** : pas de commit suivant, pas de push, pas de PR, et son dernier message dit
« HALTE borne : N lignes ajoutées pour `590` », avec le compte après chaque commit.

**Deux écarts aux indicatifs MPC, nommés :**

- La garde du crate est chiffrée **55** contre un indicatif de 30 à 50. L'indicatif
  compte un test ; le gabarit du dépôt en porte trois — la garde (~30), l'anti-vacuité
  avec ses deux témoins (~15) et l'allowlist assertie vide (~10). Le triplet n'est pas
  négociable : c'est lui qui distingue une garde d'un vert par cécité (mika#2398). Les
  5 lignes d'écart ne se rabotent pas sur les commentaires, qui comptent dans la borne
  mais disent le pourquoi (mika#2201, mika#2309).
- Le bloc `team_outcome::classify` de `chat.rs:1167-1214` — 48 lignes, commentaires
  compris — **se déplace sans se récrire**. `git diff` le comptera en lignes ajoutées sur
  le site d'arrivée et en lignes retirées sur le site de départ, mais il n'est pas chiffré
  ci-dessus parce que le chiffrage sert à borner le travail et qu'il n'y a là aucun
  travail. **Le pilote doit donc s'attendre à un `insertions(+)` mesuré supérieur à 388 —
  de l'ordre de 436 — tout en restant loin de la halte de 590.** C'est dit ici pour qu'un
  écart entre l'estimé et le mesuré ne se lise pas comme un dépassement de périmètre.

---

## Risques et notes de dérive

- **`run_team` n'est pas testable en entier** : elle ouvre l'écran. C'est pourquoi le
  critère 4 se mesure par `acces_du_mode_equipe` (test 6) plus l'ordre des instructions,
  et le critère 2 par `servir_les_buts_dequipe` (tests 1 à 4). La propriété « avant
  l'écran » est portée par le `?` et par la position de l'appel, non par un test — et
  c'est dit dans le doc de la fonction plutôt que caché.
- **Une perte d'événement n'est pas détectable côté client** : la route pousse ses trames
  dans un `mpsc` de 256 en `try_send` et un client lent en perd sans marqueur
  (`team_runs_remote.rs:21-25`). Le mode équipe de la TUI hérite de cette propriété.
  Hors vol, et noté pour qu'un trou dans la suite d'événements ne se lise pas comme un
  défaut de ce branchement.
- **`TeamOutcome::Pending { .. }` → `Deliverable(String::new())` est conservé tel quel**,
  avec son commentaire (`chat.rs:1198-1213`) : la TUI répond « Team completed with no
  deliverable. » à un run suspendu, ce qui est faux. L'asymétrie avec `ask.rs` est
  délibérée et déjà tracée (mika#1940, scope-out (b)). Ce vol **ne la corrige pas** : le
  critère 2 dit « comme aujourd'hui », et un variant de `TeamEvent` en plus toucherait
  `tui/` et `crates/mika-agent/`, tous deux hors périmètre.
- **Si un critère ne tient pas sans toucher un fichier exclu, le groom s'arrête et le
  dit** au lieu d'élargir. Rien dans ce plan n'y oblige : les treize critères tiennent
  dans les quatre fichiers du périmètre.
- **Une halte ne se lève jamais par un amendement du plan, et ce plan ne choisit aucune
  coupe.** Si le pilote doit s'écarter de ce plan sur un point — un fichier, un critère,
  un nom —, il amende ce plan sur la branche dans un commit `docs(plans)` **avant** de
  pousser le code qui s'en écarte, en nommant le point et la raison. Si une halte se
  déclenche, il s'arrête et le dit, même s'il juge qu'elle ne vaut pas ici : c'est à
  Vincent de la lever. Si une revue d'architecte demande de recouper ou d'étendre le vol,
  le groom s'arrête et le dit au lieu de réviser le plan dans ce sens.
- **Le plan qu'exécute le pilote est celui de la branche sur `origin`.** Un pilote qui
  trouve dans son worktree un plan modifié et non commité ne le commite pas : il s'arrête
  et le dit.
- **Si un hook ou un classifieur refuse un geste**, au pilote ou à un sous-agent, il
  s'arrête et le déclare, même si cela lui semble un faux positif. Il ne le contourne
  jamais par un autre outil : pas de heredoc Bash à la place de `Write`, pas de
  reformulation. L'incident mika#2869 du 2026-10-21 en est le contre-exemple.
- **Les brouillons vivent sous `.pilot-scratch/<nom>/` et ne se suppriment jamais**
  (mika#2548) — ni `rm`, ni `rmdir`, et surtout pas `rm -rf`, qui est un refus terminal.
  L'extraction d'une autre révision se fait par
  `git show <ref>:<chemin> > .pilot-scratch/<chemin>`, seule sur sa ligne, sans `--`,
  sans `2>/dev/null`, sans `;` ni `&&`. Aucune fixture dans `/tmp`.
