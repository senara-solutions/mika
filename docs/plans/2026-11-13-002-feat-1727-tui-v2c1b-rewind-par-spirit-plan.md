# Vol V2c-1b — `/rewind`, `/rewind to` et `/undo` passent par mika-spirit (mika#1727)

> **Branche :** `feat/1727/tui-v2c1b-rewind-par-spirit`
> **Base :** `80cce6bc` (`main` au moment du groom)
> **PR :** `Refs mika#1727`, jamais `Closes` — le ticket se ferme au merge de V4.

La série V2c-1a a posé les trois portes de `rewind_remote.rs` sans appelant. Ce
vol leur donne le leur : dans la TUI, `/undo`, `/rewind <n>` et `/rewind to <id>`
cessent de lire et d'écrire la base en local, et demandent l'ancrage, l'aperçu et
l'exécution à mika-spirit.

---

## 0. Ce qui est mesuré, avant toute conception

Tout ce qui suit est lu sur `80cce6bc`, pas supposé. Le § 0 du plan de V2c-1a
(`git show 6a5d20a3:docs/plans/2026-11-12-001-…`, faits F1 à F12) reste le
matériau ; les faits ci-dessous sont ceux que **ce** vol ajoute ou re-mesure, et
là où main diffère du plan, main fait foi.

**G1 — les trois appels locaux, et où ils sont.** `handle_rewind_impl`
(`crates/mika-cli/src/tui/commands/handlers.rs:1213-1304`) appelle, dans l'ordre,
`rewind::find_recent_exchanges(&app.db, …)` (l. 1228), `rewind::preview_rewind(&app.db, …)`
(l. 1236), puis `rewind::execute_rewind(&app.db, …)` (l. 1253). Entre les deux
derniers, `rewind::format_preview(&preview)` (l. 1248). Après le succès :
`app.messages.clear()`, `app.db.load_recent_messages_filtered(20, app.inbox_mode)`,
remise à zéro de `scroll_offset` et `messages_layout`, puis le texte
« Rewind complete: … ». La garde `app.status != AgentStatus::Idle` est la
**première** instruction. `handle_undo` et `handle_rewind` (l. 1180-1211)
n'analysent que les arguments.

**G2 — les six attentes, et leurs lignes exactes.** `rewind_remote.rs` porte six
`expect(dead_code)` dont la raison nomme V2c-1b : l. 28 (`ResolveMirror`), l. 77
(`ResultMirror`), l. 123 (`RewindRemoteIssue::Blocked::preview`), l. 220
(`resolve`), l. 379 (`preview`), l. 460 (`execute`). Toutes sont des `expect`,
jamais des `allow` : **dès que la production a un appelant, une attente restée en
place devient `unfulfilled_lint_expectation`, c'est-à-dire un rouge de clippy.**
Le câblage et le retrait des attentes ne peuvent donc pas vivre dans deux commits
différents — c'est une contrainte du compilateur, pas une préférence.

**G3 — trois champs de miroir n'auront toujours pas de lecteur après le
câblage.** Mesuré champ par champ contre le rendu de G1 :

| miroir | champ | lecteur après câblage naïf |
|---|---|---|
| `ResolveMirror` | `session_id`, `after_message_id` | la couture |
| `ResolveMirror` | `trace_ids` | **aucun** |
| `ResultMirror` | `messages_deleted`, `reversals_applied`, `tasks_deleted`, `tasks_cancelled`, `warnings` | le rendu de G1 |
| `ResultMirror` | `reversals_skipped`, `reversal_descriptions`, `rewind_trace_id` | **aucun** |

C'est le fait qui décide du § 2.4 : un câblage qui se contente de reproduire le
rendu actuel **ne peut pas** retirer les attentes de `ResolveMirror` et de
`ResultMirror` sans rougir. Le critère 7 (« sans attente insatisfaite, et sans
attente V2c-1b restante ») n'est donc pas un geste de nettoyage en fin de vol :
il force une décision sur ces quatre champs, et le § 2.4 la prend.

**G4 — le factice est privé au module `tests` de `rewind_remote.rs`.**
`fake_spirit` (l. 579), `RequeteRecue` (l. 554, quatre champs privés
`methode`/`chemin`/`autorisation`/`corps`), `Factice` et `servir` vivent tous
dans `#[cfg(test)] mod tests` (l. 547). Sa signature :
`async fn fake_spirit(&[(&str, StatusCode, &str)]) -> (String, Arc<Mutex<Vec<RequeteRecue>>>)`.
Un `fallback` unique sert les trois routes et rend 501 sur un chemin non associé,
« pour qu'une faute de chemin ne se lise jamais comme un `NotFound` ».

**G5 — `pub(crate) mod tests` a trois précédents dans ce dépôt.**
`mika-agent/src/grooming_marker.rs:285`, `mika-agent/src/db.rs:6917`,
`mika-agent/src/server/webhook_prefilter.rs:1968`. La forme est connue du dépôt
et de son `cargo fmt`.

**G6 — `test_app()` existe et donne tout ce qu'il faut.**
`handlers.rs:1351` bâtit une `App<'static>` sur `Database::open_in_memory()`,
crée une session, et pose `agent_name = "mika"`. Elle rend
`(App, UnboundedReceiver<AgentRequest>, TempDir)` — le `TempDir` doit rester
vivant. **Aucun test ne couvre `/rewind` aujourd'hui.**

**G7 — le jeton : la cascade existante retombe sur un jeton que le serveur
refusera.** `dashboard::auth_token()` (`commands/dashboard.rs:32-41`) charge le
`.env` de la home puis lit `MIKA_INTERNAL_TOKEN` **ou** `MIKA_DASHBOARD_TOKEN`.
Les trois routes rewind sont montées sous `auth::require_internal_token`
(`mika-agent/src/server/mod.rs:368-376`) : un jeton de dashboard y prend un 401.
`commands/iterate.rs:72-79` a déjà rencontré ce cas et a tranché dans l'autre
sens — il garde `auth_token()` et commente « c'est le bon refus, et il est
lisible (401) ». Ce vol tranche différemment, et le § 2.1 dit pourquoi.

**G8 — `spirit_url()` ne charge le `.env` que sur une branche.**
`commands/dashboard.rs:18-28` : si `MIKA_SPIRIT_URL` est posé, elle rend
immédiatement **sans** charger le `.env`. Le chargement du `.env` ne peut donc
pas être délégué à `spirit_url()` ; il est explicite chez l'appelant.

**G9 — le balayeur de production est partagé et a un précédent dans ce crate.**
`mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"))`,
puis `.production_of(&scanner.src_root().join(…))` : `remote_ask.rs:1778`,
`ask.rs:1350`, `budget_floor.rs:593`, `agents.rs:1764`, `team_outcome.rs:241`.
Il **masque** les régions `cfg(test)` par des lignes vides au lieu de tronquer
(mika#2398), ce qui est la propriété dont la garde du § 2.5 dépend : le test qui
nomme les trois motifs vit dans le même fichier que la production qu'il refuse.

**G10 — les deux gardes jumelles de ce ticket sont dans ce crate.**
`mika1727_v1_le_tour_de_la_tui_ne_fait_plus_tourner_la_boucle` (vol V1) et
`mika1727_v2a_la_tui_ne_construit_plus_de_moteur_de_taches` (vol V2a, module
`tests` de `commands/chat_turn.rs`), toutes deux « allowlist livrée vide et
pinnée vide ». La garde du § 2.5 est la troisième de la même famille et en prend
la forme.

**G11 — l'egress est déjà couvert.** `docs/egress/egress-manifest.toml:363`
porte `id = "cli-spirit-rewind"`. `handlers.rs` n'ouvre aucun client — le seul
`reqwest::Client::new()` du chemin rewind vit dans `post`
(`rewind_remote.rs:171`), inchangé par ce vol. `docs/egress/` n'est donc pas
touché, et `scripts/verify-egress-manifest.sh --report` est une **vérification**,
pas un livrable.

---

## 1. La forme : une couture, deux fonctions, un seul lecteur d'environnement

`handle_rewind_impl` garde ce qui ne se teste pas (la garde d'état, le
chargement du `.env`, la résolution de l'URL de base) et délègue le reste à une
fonction qui prend ses accès **par argument** — la forme que
`chat_turn::send_turn_to_spirit` a déjà donnée au tour, et que
`rewind_remote::post` a donnée au transport.

Deux fonctions neuves, et la frontière entre elles est dictée par le critère 5 :

```
handle_rewind_impl(app, count, id)        ← garde d'état, .env, spirit_url()
└── rewind_par_spirit(app, base, lire_env, count, id) -> String
    ├── jeton_interne(lire_env) -> Result<String, String>   ← AVANT tout appel
    ├── rewind_remote::resolve(…)   (sauf /rewind to)
    ├── rewind_remote::preview(…)
    └── rewind_remote::execute(…)
```

**Pourquoi la lecture du jeton vit à l'intérieur de `rewind_par_spirit`, et non
chez `handle_rewind_impl`.** Le ticket décrit la couture comme « résout l'URL de
base et le jeton, puis délègue » ; le critère 5 exige, lui, qu'un test montre
*à la fois* le refus nommé **et** qu'« aucune requête » n'atteint le factice,
« sans muter l'environnement du processus ». Les deux ne tiennent ensemble que
si la lecture est dans la fonction que le test appelle. Ce que
`handle_rewind_impl` résout, c'est donc la **source** du jeton
(`&|nom| std::env::var(nom).ok()`, après le `.env`), et la lecture elle-même est
la première instruction de la fonction déléguée. C'est exactement ce que le
ticket demande une ligne plus loin — « la lecture prend sa source en paramètre »
— et c'est la seule lecture des deux phrases qui soit réalisable.

Le type de la source est nommé une fois :

```rust
/// Une source de variables d'environnement. `&dyn` et non un générique : la
/// fonction est `async` et appelée depuis un seul site de production ; un
/// paramètre de type ferait monomorphiser pour rien et se propagerait dans la
/// signature de tout ce qui l'appelle.
type SourceEnv<'a> = &'a dyn Fn(&str) -> Option<String>;
```

---

## 2. Les livrables, un par un

### 2.1 `jeton_interne` — le refus avant l'appel (critère 5)

```rust
fn jeton_interne(lire: SourceEnv<'_>) -> Result<String, String>
```

Lit `MIKA_INTERNAL_TOKEN`, et **rien d'autre**. Sur absence, rend un message qui
nomme la variable et dit quoi faire.

Le commentaire porte le pourquoi que le code ne dit pas — pourquoi le repli de
`dashboard::auth_token()` est écarté ici alors que `iterate.rs:72-79` l'a gardé
là-bas :

> Pas de repli sur `MIKA_DASHBOARD_TOKEN`. Les trois routes rewind sont sous
> `require_internal_token` (`server/mod.rs:368-376`) : ce jeton y prend un 401,
> donc un refus **après** l'appel. `iterate.rs` a tranché l'inverse pour une
> route qui *lance* un dispatch — un aller-retour pour un 401 lisible y coûte un
> appel sans effet. Ici l'appel qui suivrait n'est pas sans effet : c'est
> `resolve` puis `preview` puis `execute`, et le 401 tomberait sur la première
> des trois, laissant à l'opérateur un message qui nomme le transport là où la
> cause est sa configuration. Refuser avant l'appel nomme la cause, et ne touche
> à rien.

### 2.2 `rewind_par_spirit` — la couture (critères 1 à 4)

```rust
async fn rewind_par_spirit(
    app: &mut App<'_>,
    base_url: &str,
    lire_env: SourceEnv<'_>,
    exchange_count: usize,
    after_message_id: Option<i64>,
) -> String
```

Le corps, dans l'ordre :

1. **Le jeton**, par `jeton_interne`. Sur `Err`, le message est rendu tel quel et
   **rien** n'est appelé.
2. **L'ancrage.** `Some(id)` → `(app.session_id.clone(), id)`, sans appel
   (critère 2). Sinon → `rewind_remote::resolve(base_url, &jeton, &app.agent_name,
   &app.session_id, exchange_count)`, et l'on prend `session_id` et
   `after_message_id` du `ResolveMirror`. Un `Err` est rendu par son `Display`,
   qui nomme déjà la cause (401, 404 — « no exchanges to rewind, or unknown
   agent » couvre le cas que `Ok(None)` rendait en local —, 409, hors contrat).
3. **L'aperçu.** `rewind_remote::preview(base_url, &jeton, &app.agent_name,
   &session, ancrage)`.
   - `Err(Blocked { reason, preview })` → la sortie dit le refus et sa raison,
     puis, **si** `preview` est `Some`, `rewind::format_preview` de cet aperçu.
     Rien n'est exécuté (critère 4).
   - `Err(autre)` → rendu par son `Display`.
   - `Ok(apercu)` → `rewind::format_preview(&apercu)`, le texte qui précédera le
     résultat. `format_preview` reste la **seule** mise en forme d'un aperçu —
     c'est tout l'intérêt de la conversion que V2c-1a-2a a payée.
4. **L'origine.** `(session != app.session_id).then(|| app.session_id.clone())`,
   puis `.as_deref()`. Cloné et non emprunté : `app` est repris en `&mut` juste
   après pour le rechargement, et un emprunt immuable qui traverse l'`await`
   d'`execute` ferait porter au lecteur la charge de vérifier qu'il meurt à
   temps.
5. **L'exécution.** `rewind_remote::execute(…, origine)`. Sur `Err`,
   « Rewind failed: {e} ». Sur `Ok(resultat)` : le rechargement de G1 à
   l'identique (`app.messages.clear()`, `load_recent_messages_filtered`,
   `scroll_offset = 0`, `messages_layout` par défaut), puis le rendu du § 2.3.

Le commentaire du rechargement garde ce que G1 disait déjà (la base peut porter
des messages `tool_use` que `app.messages` n'a jamais eus, donc une troncature
par comptage est fausse) et ajoute ce qui est neuf :

> C'est **spirit** qui a écrit cette base, et la TUI qui la relit. Les deux
> processus partagent le fichier ; le déplacement de la session hors du processus
> de la TUI est un vol ultérieur de mika#1727. Jusque-là, le rechargement est
> précisément ce qui rend le rembobinage distant visible ici.

### 2.3 Le rendu du résultat — les huit champs, pas cinq (critère 7)

La sortie reprend « Rewind complete: {messages_deleted} messages deleted,
{reversals_applied} reversals applied. » puis la ligne des tâches et le bloc des
warnings, à l'identique de G1. Trois ajouts, et leur raison est G3 :

- `reversals_skipped` : ajouté à la ligne des inversions quand il est non nul
  (« …, N skipped »).
- `reversal_descriptions` : listé comme les warnings quand la liste est non vide.
- `rewind_trace_id` : une dernière ligne « Trace: {id} » quand il est non vide.

Le commentaire dit pourquoi ce n'est pas de l'ornement :

> Les trois champs n'avaient pas de lecteur, et l'attente `dead_code` de
> `ResultMirror` le disait. Les afficher plutôt que les retirer du miroir est le
> choix que leur contenu impose : `reversals_skipped` et
> `reversal_descriptions` disent **ce que le rembobinage n'a pas su défaire**.
> Un effet mémoire non inversé qu'on tait est exactement le mensonge que
> `reversal_action_from_label` refuse déjà de produire côté aperçu ; le taire
> côté résultat serait le produire une porte plus loin.

### 2.4 `ResolveMirror.trace_ids` — retiré du miroir (critère 7)

Le seul des quatre champs de G3 à n'avoir aucun lecteur naturel : l'ancrage
n'est pas montré à l'opérateur, et c'est le plan de V2c-1a qui l'a écrit. Il
part du miroir, avec son assertion dans
`mika1727_les_miroirs_se_lisent_depuis_la_forme_du_serveur`.

> Le champ n'est pas « supprimé du contrat » : `serde` ignore par défaut ce
> qu'un `Deserialize` ne déclare pas, donc le serveur continue de l'envoyer et
> rien ne casse. Ce qui change, c'est que le miroir ne déclare plus que ce qu'il
> **consomme**. Un champ déclaré sans lecteur est du code mort sous une attente,
> et l'attente est précisément ce que le critère 7 vient retirer. Le jour où un
> appelant veut les traces, il rajoute le champ et son lecteur dans le même
> geste — ce qui est la seule façon de ne jamais retomber ici.

### 2.5 La garde structurelle (critère 6)

Dans le module `tests` de `handlers.rs`, troisième garde de la famille G10 :

```rust
#[test]
fn mika1727_v2c1b_la_tui_ne_rembobine_plus_la_base_en_local() { … }
```

- `ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"))` puis
  `production_of(src_root().join("tui/commands/handlers.rs"))`.
- Trois motifs refusés : `find_recent_exchanges(`, `preview_rewind(`,
  `execute_rewind(`. Un message par motif, qui nomme le remède : passer par la
  porte correspondante de `rewind_remote`, jamais une entrée d'allowlist.
- **Allowlist** : une constante vide, assertie vide (`assert!(….is_empty())`)
  dans la garde elle-même — la disposition du § *Fire-Disposition*.
- **Témoin de lecture** : `assert!(production.contains("async fn handle_rewind_impl("))`,
  avec son commentaire :

  > Sans ce témoin, une garde qui balaie un chemin faux, ou un balayeur qui
  > masquerait le fichier entier, serait verte **par cécité** — l'exact défaut
  > que `ProductionScanner` existe pour refuser (mika#2398). Le témoin vise un
  > nom de la moitié *production* : s'il disparaît, la garde rougit et demande
  > qu'on la re-vise, au lieu de continuer à ne rien regarder.

- La garde ne refuse **pas** `rewind::` ni `format_preview(` : `format_preview`
  reste le formateur légitime, et `app.db` reste lu pour le rechargement. Le scan
  porte sur les trois gestes qui *lisent ou écrivent le rembobinage en local*, et
  sur rien d'autre.

### 2.6 Les attentes et l'en-tête de `rewind_remote.rs` (critère 7)

Les six attentes de G2 tombent dans le commit qui câble (contrainte du
compilateur, G2). L'en-tête du module perd sa phrase « leurs appelants arrivent
avec V2c-1b » et dit l'état atteint : les trois portes ont leur appelant, c'est
`tui/commands/handlers.rs::rewind_par_spirit`, et le `dead_code` qui mordait ici
est désormais celui qui mord chez lui.

Le pilote **mesure** par `cargo clippy -p mika-cli --all-targets` plutôt qu'il ne
suppose : un `expect` encore satisfait sort en `unfulfilled_lint_expectation`
avec sa ligne. Si l'une des six l'est encore après câblage, il le **dit** au lieu
de la garder en silence, et ne met jamais `allow` à la place.

### 2.7 La visibilité du factice

`mod tests` de `rewind_remote.rs` devient `pub(crate) mod tests` (G5), et
`fake_spirit`, `RequeteRecue` et ses quatre champs deviennent `pub(crate)`. Rien
d'autre ne bouge.

> Un second factice dans `handlers.rs` coûterait une cinquantaine de lignes et,
> surtout, ouvrirait une seconde vérité sur ce que « ce que le serveur a vu »
> veut dire — deux témoins de socket qui dérivent, dans un ticket qui existe
> pour retirer des duplications. Le factice est un **instrument de mesure** ;
> il s'emprunte, il ne se recopie pas.

### 2.8 La doc

Un paragraphe dans `crates/mika-cli/CLAUDE.md`, section `## TUI Features`, à la
suite des deux sections jumelles de ce ticket (V1 l. 366, V2a l. 424) :
`### Le rembobinage de la TUI est appliqué par mika-spirit (mika#1727, vol V2c-1b)`.
Il dit les trois portes, la garde structurelle et son allowlist vide, et
l'exigence de `MIKA_INTERNAL_TOKEN` — y compris que `MIKA_DASHBOARD_TOKEN` ne
suffit pas, et que le refus tombe **avant** tout appel.

---

## 3. Ordre des commits

Le pilote commite après chaque garde verte **avec son test**, et mesure
`git diff --numstat origin/main...HEAD` (plan exclu) après chaque commit.

1. **`jeton_interne` et son refus** — § 2.1 + T1 + T2. Autonome : aucune attente
   touchée, aucun appel réseau. ~45 lignes.
2. **La couture** — § 2.2, 2.3, 2.4, 2.6, 2.7 + T3 à T7. C'est le commit
   indivisible du vol : câbler les portes rend les six attentes insatisfaites
   (G2), donc le retrait des attentes, la lecture des champs restants et le
   retrait de `trace_ids` sont dans le **même** commit, ou clippy est rouge.
   ~250 lignes.
3. **La garde structurelle** — § 2.5 + T8. Le pilote montre d'abord son
   **rouge-avant** : la garde posée telle quelle sur le `handlers.rs` de main
   trouve les trois motifs. Puis son vert sur le code du commit 2. ~35 lignes.
4. **La doc** — § 2.8. ~14 lignes.

**Halte sur la borne.** Après chaque commit, si le total dépasse **429**, le
pilote s'arrête : pas de commit suivant, pas de PR. Son dernier message dit
« HALTE borne : N lignes pour 429 », avec le compte par commit. Sur #3077 un
pilote a écrit 510 lignes pour 429 sans rien signaler ; ce silence ne doit pas se
reproduire. Les commentaires comptent dans la borne mais **ne se rabotent pas**
pour la tenir : si le code écrit dépasse, c'est la halte, pas la coupe.

---

## 4. Le jeu de tests

Huit tests. Les cinq premiers visent `rewind_par_spirit` avec `test_app()` (G6)
et le factice emprunté (§ 2.7) ; un helper local bâtit le couple
`(app, base_url, témoin)` une fois.

| # | critère | ce qu'il montre | comment |
|---|---|---|---|
| T1 | 5 | sans `MIKA_INTERNAL_TOKEN`, le refus **nomme la variable** | `jeton_interne(&\|_\| None)` → `Err` contenant `MIKA_INTERNAL_TOKEN` |
| T2 | 5 | `MIKA_DASHBOARD_TOKEN` seul ne suffit pas | source qui ne rend que ce nom → `Err` |
| T3 | 5 | et **le factice ne reçoit aucune requête** | `rewind_par_spirit` avec cette source → témoin `len() == 0` |
| T4 | 1 | `/undo` : `resolve`, `preview`, `execute`, **dans cet ordre**, `agent` dans les trois corps, compteurs en sortie | trois chemins associés, assertions sur `vues[0..3].chemin`, `corps.contains("\"agent\":\"mika\"")`, et sur le texte rendu — dont `reversals_skipped` et `rewind_trace_id` du § 2.3 |
| T5 | 2 | `/rewind to <id>` : **pas** de `resolve`, et le corps d'`execute` n'a **pas** de clé `originating_session_id` | `after_message_id: Some(…)`, `vues.len() == 2`, `!vues[1].corps.contains("originating_session_id")` |
| T6 | 3 | session résolue différente → `originating_session_id` == `app.session_id` | `resolve` sert une autre session ; lu sur la socket |
| T7 | 4 | un 422 de `preview` dit le refus **et** sa raison, et `execute` n'est jamais appelé | `vues.len() == 2`, aucun chemin `execute`, la raison du serveur dans la sortie |
| T8 | 6 | la garde structurelle | § 2.5 |

**Ce que T5 et T6 lisent est le corps sur la socket, pas une valeur de retour.**
C'est la propriété que le critère 2 nomme explicitement (« lu sur la socket ») :
l'absence d'une clé JSON n'est observable nulle part ailleurs — un `Option::None`
côté Rust et une clé `null` sur le fil sont indiscernables depuis l'appelant, et
c'est précisément la distinction que le commentaire d'`execute` déclare
load-bearing (`rewind_remote.rs:481-489`).

**Ce que les tests ne couvrent pas, et pourquoi.** L'analyse d'arguments de
`handle_rewind` / `handle_undo` (G1, l. 1180-1211) ne change pas et reste non
testée ; la couvrir serait du périmètre neuf sous une borne serrée. Les tests
visent `rewind_par_spirit` avec le `after_message_id` que cette analyse produit,
ce qui est l'exact contrat entre les deux.

---

## Fire-Disposition

Ce plan livre **un** détecteur : la garde structurelle du § 2.5 (T8), dont le
chemin de succès est « aucun des trois motifs en production ».

**Disposition retenue : (a) exception nommée en allowlist — livrée vide, et vide
par construction.**

Détail d'implémentation :

- La garde atterrit **armée** (pas de `#[ignore]`, pas de `cfg(skip)`), au
  commit 3, c'est-à-dire **après** le commit 2 qui retire les trois appels. Le
  compte de violations au moment où elle est armée est donc **zéro**, et aucune
  exception n'a de raison d'exister.
- L'allowlist est une constante vide, **assertie vide dans la garde elle-même** :
  c'est la forme auto-nettoyante que cette disposition prend quand il n'y a
  aucune exception à faire expirer. Une entrée ajoutée un jour rougit
  immédiatement, et le message de la garde dit que le remède est de passer par
  les portes de `rewind_remote`, jamais d'élargir la liste.
- Le **témoin de lecture** (§ 2.5) est l'autre moitié de la disposition : une
  garde à allowlist vide est triviale à satisfaire par cécité, et le témoin est
  ce qui rend cette cécité bruyante.
- **Rouge-avant exigé.** Le pilote montre la garde rouge sur le `handlers.rs` de
  main (trois motifs trouvés), puis verte sur le sien. Une garde qu'on n'a jamais
  vue rougir n'a pas été mesurée (critère 6).

C'est la forme exacte des deux gardes jumelles de ce ticket (G10), « allowlist
livrée vide et pinnée vide ».

---

## Definition of Done

- [ ] `/undo`, `/rewind <n>` et `/rewind to <id>` passent par les trois portes de
      `rewind_remote.rs` ; `handlers.rs` n'appelle plus `find_recent_exchanges`,
      `preview_rewind` ni `execute_rewind`.
- [ ] Sans `MIKA_INTERNAL_TOKEN`, le refus nomme la variable **avant** tout appel ;
      `MIKA_DASHBOARD_TOKEN` seul ne suffit pas.
- [ ] Les huit tests du § 4 passent (`cargo test -p mika-cli`).
- [ ] `cargo clippy -p mika-cli --all-targets` est vert, sans `allow(dead_code)`,
      sans attente insatisfaite, sans attente « V2c-1b » restante.
- [ ] `cargo fmt --check` est vert.
- [ ] `bash scripts/verify-egress-manifest.sh --report` est vert, lancé **avant**
      d'ouvrir la PR. S'il est rouge, le pilote s'arrête et le dit.
- [ ] `bash scripts/verify-pipeline.sh origin/main` est vert.
- [ ] Le diff ne touche que `crates/mika-cli/src/tui/commands/handlers.rs`,
      `crates/mika-cli/src/commands/rewind_remote.rs` et
      `crates/mika-cli/CLAUDE.md` (plus ce plan).
- [ ] `git diff --numstat origin/main...HEAD`, plan exclu, est **≤ 429** lignes
      ajoutées. Sinon : halte, et le message du § 3.
- [ ] La PR porte `Refs mika#1727` et aucun `Closes`/`Fixes`/`Resolves`, y
      compris si le gabarit de reprise l'ouvre ; `gh pr view <N> --json
      closingIssuesReferences` rend une liste vide.

---

## Acceptance criteria

Transcrits du corps de mika#1727 (§ *Critères de V2c-1b*).

1. `/undo` appelle `resolve`, puis `preview`, puis `execute`, dans cet ordre, sur
   le factice, avec `agent` égal à `app.agent_name` dans les trois corps. La
   sortie porte les compteurs du `ResultMirror` servi. Un test le montre.
2. `/rewind to <id>` n'appelle pas `resolve`. `preview` et `execute` portent
   `app.session_id` et l'id donné, et le corps d'`execute` n'a **pas** de clé
   `originating_session_id`. Un test le montre, lu sur la socket.
3. Quand `resolve` rend une autre session que celle de la TUI, le corps
   d'`execute` porte `originating_session_id` égal à `app.session_id`. Un test le
   montre.
4. Un 422 de `preview` rend une sortie qui dit le refus et la raison du serveur.
   `execute` n'est jamais appelé. Un test le montre.
5. Sans `MIKA_INTERNAL_TOKEN`, la commande rend un refus qui nomme la variable,
   et le factice ne reçoit **aucune** requête. Un `MIKA_DASHBOARD_TOKEN` seul ne
   suffit pas. Un test le montre sans muter l'environnement du processus.
6. La garde structurelle refuse `find_recent_exchanges(`, `preview_rewind(` et
   `execute_rewind(` dans la moitié production de `handlers.rs`. Son allowlist
   est vide et son témoin de lecture est présent. Le pilote montre son
   rouge-avant sur le code de main, puis son vert.
7. Le pilote lance `cargo clippy -p mika-cli --all-targets`. Le résultat est
   vert, sans `allow(dead_code)`, sans attente insatisfaite, et sans attente
   « V2c-1b » restante dans `rewind_remote.rs`.
8. Seuls `handlers.rs`, `rewind_remote.rs` et `crates/mika-cli/CLAUDE.md`
   changent. Restent intacts : `docs/egress/`, `docs/slash-commands.md`,
   `app.rs`, `chat.rs`, `crates/mika-agent/` et `crates/mika-a2a/`.
9. Le pilote lance `bash scripts/verify-egress-manifest.sh --report` avant
   d'ouvrir la PR. S'il le trouve rouge, il s'arrête et le dit.
10. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
    compris si le gabarit de reprise l'ouvre.

---

## 5. Ce que ce vol ne fait pas

- **Il ne déplace pas la session.** Spirit écrit la base que la TUI relit ; les
  deux processus partagent le fichier. La création de session par spirit est un
  vol ultérieur, mesuré à son tour (`Tracked in: senara-solutions/mika#1727`).
- **Il ne touche pas `/compact`** (`handlers.rs:194-210`), qui est l'autre moitié
  de V2c et a son propre vol.
- **Il ne touche pas `tui/app.rs` ni `commands/chat.rs`**, ni
  `crates/mika-agent/`, ni `crates/mika-a2a/`.
- **Il ne touche pas `docs/egress/`** (G11) ni `docs/slash-commands.md` (sa copie
  synchronisée vit dans `crates/mika-agent/docs/`, hors périmètre).
- **Il ne teste pas l'analyse d'arguments** de `/rewind` (§ 4, dernier
  paragraphe).
- **Il n'ouvre aucun ticket** — gel jusqu'au 01/12. Un manque côté spirit ou un
  reste va en commentaire sur mika#1727 ou dans la PR.

---

## 6. La dette, nommée et non traitée

- **Le rendu du résultat vit en ligne dans `handlers.rs`.** Il n'existe aucun
  `format_result` côté domaine (le plan de V2c-1a le disait déjà : `rewind.rs`
  n'expose que `format_preview`). Le § 2.3 enrichit donc un formateur local, ce
  qui est la bonne décision *à périmètre constant* mais laisse la mise en forme
  d'un résultat sans site unique. Le jour où un second appelant rend un
  `ResultMirror`, c'est ce formatage qu'il faut extraire, pas recopier.
  `Tracked in: senara-solutions/mika#1727`.
- **`MessagePreview.trace_id` reste `None` après conversion** (F9 du matériau).
  Sans effet sur le texte tant que `format_preview` ne le lit pas ; la ligne qui
  rougira est `rewind.rs:739`.
- **Le factice est emprunté au module `tests` d'un autre fichier** (§ 2.7). C'est
  le bon arbitrage sous cette borne, et c'est aussi un couplage de test à
  surveiller : un troisième emprunteur voudra un module de test partagé, pas un
  troisième `pub(crate)`.
- **`mika ask` et la TUI n'ont pas la même cascade de jeton.** `iterate.rs` garde
  le repli, ce vol ne le prend pas (§ 2.1, G7). Les deux conduites sont
  justifiées séparément ; elles ne sont pas unifiées, et une unification devra
  d'abord décider ce que « refus lisible » veut dire par route.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `handlers.rs` — production (§ 2.1 à 2.3 : `jeton_interne`, `rewind_par_spirit`, `handle_rewind_impl` délégué, rendu enrichi, imports) | 132 |
| `handlers.rs` — tests T1 à T7 + helper `(app, factice)` | 144 |
| `handlers.rs` — garde structurelle T8 (§ 2.5) | 32 |
| `rewind_remote.rs` (§ 2.4, 2.6, 2.7 : en-tête, six attentes retirées, `trace_ids`, visibilité du factice, assertion ajustée) | 31 |
| `crates/mika-cli/CLAUDE.md` (§ 2.8) | 14 |

Total estimé : 353 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol, et pourquoi il tient.** 353 contre une borne de 429,
soit 18 % de marge — et cette marge est exactement du bon ordre pour l'erreur
d'estimation mesurée sur les vols précédents de ce ticket (V2b-2b-1 : 241
estimées pour 279 réelles, +16 %). À +16 %, 353 donne ~410 lignes réelles, sous
la borne. À +22 % il la touche : c'est pourquoi la halte du § 3 est mesurée
**après chaque commit** et non en fin de vol, et pourquoi le commit 1 est
l'unité la plus petite qu'on puisse livrer seule.

Le total inclut `CLAUDE.md` — l'unité de la borne de ce ticket est « lignes
ajoutées, code, commentaires, doc et tests compris, plan exclu, comptées par
`git diff --numstat origin/main...HEAD` », et c'est la plus stricte des deux. Le
total hors `CLAUDE.md`, au sens de la règle mika#2636, est de **339**.

Le périmètre n'est pas découpable plus bas sans sortir un critère : les huit
tests sont chacun l'unique preuve d'un critère, et « les tests ne se reportent
jamais à un vol suivant ». La seule coupe possible serait le § 2.3 (le rendu des
trois champs non lus), et elle est interdite par le critère 7 — sans lecteur, les
attentes de `ResultMirror` ne peuvent pas tomber.
