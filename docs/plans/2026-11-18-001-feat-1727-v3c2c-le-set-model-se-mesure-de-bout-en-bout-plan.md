# mika#1727 — vol V3c-2c : le `SetModel` se mesure de bout en bout

- **Ticket** : senara-solutions/mika#1727 — *TUI as thin HTTP client of mika-spirit*
- **Vol** : V3c-2c (le dernier de V3c-2 ; vient ensuite V4, au merge duquel le ticket se ferme)
- **Branche** : `feat/1727/v3c2c-le-set-model-se-mesure-de-bout-en-bout`
- **Base mesurée** : `main` @ `633e4116`
- **Label** : `loop-substrate` (plafond de tours : 200)
- **Borne du groom** : 220 lignes ajoutées, posée par Vincent pour ce vol (corps du ticket,
  § *Borne de ce vol*). **Halte du pilote** : 310 lignes ajoutées.
- **Unité de toutes les bornes** : la ligne **ajoutée**, colonne 1 de
  `git diff --numstat origin/main...HEAD -- . ':!docs/plans' | awk '{a+=$1} END{print a}'` —
  code, commentaires, doc et tests ; les retraits ne comptent pas.
- **La PR porte `Refs mika#1727`**, jamais `Closes`, `Fixes` ni `Resolves` — ni dans le corps,
  ni dans un commit. Le ticket se ferme au merge de V4.

---

## 0. Pourquoi ce vol tient en un seul vol

Tout ce qui voyage existe déjà. Depuis V3b-1, `mika.model_override` part sur le fil, le
bras `SetModel` pose l'override par `override_de_set_model`, et spirit résout l'alias puis
refuse fermement ce qu'il ne sait pas servir (mika#2304). Rien de cela ne change ici.

Ce qui manque est **l'observabilité d'une affectation**. Le critère « après un `SetModel`,
le tour suivant porte l'override » s'est fermé en V3b-1 par composition : table de cas de
la fonction pure, puis sortie de cette fonction donnée à `send_turn_to_endpoint`. Le plan
de V3b-1 a nommé le maillon restant É1 — « le maillon qui reste hors test est **une**
affectation de champ dans le worker, visible en revue » —, et la cause qu'il donnait est
mesurable : le corps de `spawn_agent_worker` lit le monde au lieu de le recevoir, donc
aucun test ne peut l'appeler sans poser `MIKA_SPIRIT_URL` — `unsafe` en édition 2024, et
faux sous un `cargo test` qui parallélise.

Le vol retire cette lecture. Deux arguments de plus sur une fonction, l'adresse gardée par
le worker, un paramètre de plus sur trois fonctions de `chat_turn.rs`, et le corps entier
du worker devient atteignable. Le reste du volume est **un** test, livré dans le même
commit que le changement qui le rend possible.

L'estimation est de **174 lignes** (§ *Taille estimée*), sous la borne du groom de 220 et
sous la borne de 429 de l'unité de l'architecte, **tests compris**. Il n'y a donc aucune
coupe à demander, et ce plan ne s'arrête pas. Il n'en pratique aucune non plus : une coupe
demande le mot de Vincent.

---

## 1. Ce qui est mesuré sur main (@ `633e4116`)

Les mesures du corps du ticket ont été rejouées site par site. **Les deux lectures
concordent sur chaque ligne citée** — ce § ne corrige aucun chiffre de la tête.

### `crates/mika-cli/src/commands/chat.rs` (2090 lignes)

| site | ligne | mesure |
|---|---|---|
| `ouvrir_session_par_spirit` | 69 | **le précédent de forme** : `(db, base_url: &str, lire_env: SourceEnv<'_>, agent, session_id, canonical)`. Sa doc : « tout ce qui touche le monde arrive par argument, donc un test pilote la source d'environnement, la base et l'URL du banc sans construire le contexte du démarrage » |
| `lire_les_skills` | 139 | même forme : `(base_url, lire_env, agent)`. Rien n'y est fatal : le retour est un `EtatSkills`, pas un `Result` remonté par `?` |
| `clore_session_de_sortie` | 164 | même forme encore, troisième précédent du fichier |
| `spawn_agent_worker` | 214 | `(ctx: DbContext, agent_name, session_id, model_override: Option<String>)`, rend un 8-uplet. **Le préfixe littéral `async fn spawn_agent_worker(` est le témoin de deux gardes** (l. 1929, l. 2044) |
| lecture de l'adresse — session | 248-249 | `&crate::commands::dashboard::spirit_url()` et `&\|nom\| std::env::var(nom).ok()`, en argument de l'ouverture |
| lecture de l'adresse — skills | 268-269 | les deux mêmes expressions, en argument de la lecture du registre |
| état du worker | 306-313 | `model`, `let mut worker_override = model_override;`, `worker_session`, `worker_agent_name = agent_name.to_string()` |
| bras `Message` | 347 | `chat_turn::send_turn_to_spirit(&text, &worker_agent_name, &worker_session, false, worker_override.as_deref())` |
| bras `CallbackResult` | 418 | `chat_turn::livrer_tour_de_callback(&worker_db, &worker_agent_name, &worker_session, &framing, &task_id, &original_status, worker_override.as_deref())` |
| bras `SetModel` | 442 | `worker_override = chat_turn::override_de_set_model(&model);` — **l'affectation qu'aucun test ne voit** |
| superviseur | 462-470 | `supervise(handle, shutdown_initiated, …)` ; un `Ok(())` sans `shutdown_initiated` est rapporté « worker exited before session closed » |
| appelants | 614, 837, 950 | `run` au démarrage (avec `--model`), la bascule d'agent et `/restart` (tous deux `None`) |
| `mod tests` | 1333 | `fake_spirit` **déjà importé** (l. 1337-1338), `ROUTE_SESSIONS` (1346), `CORPS_OUVERT` (1347), `base_en_memoire()` (1360), `jeton_pose()` (1365) |
| gardes | 1929, 2044 | scans par fichier, allowlists livrées vides et asserties vides, témoin `async fn spawn_agent_worker(` et contre-témoin `fn mika1727_v2d4a_un_point_injoignable_nomme_lurl` |

### `crates/mika-cli/src/commands/chat_turn.rs` (1342 lignes)

| site | ligne | mesure |
|---|---|---|
| `spirit_turn_endpoint` | 42 | `fn (agent_name) -> String`, corps `format!("{}/a2a/{}", spirit_url(), agent_name)` — **le troisième lecteur de l'adresse** |
| `send_turn_to_spirit` | 79 | 5 paramètres ; résout l'endpoint l. 86 puis délègue |
| `send_turn_to_endpoint` | 98 | endpoint déjà résolu. Sa doc dit pourquoi le jumeau existe : « les tests visent un serveur factice, et l'alternative serait de muter `MIKA_SPIRIT_URL` » |
| `livrer_tour_de_callback` | 181 | 7 paramètres ; résout l'endpoint l. 190 puis délègue |
| `livrer_tour_de_callback_sur_endpoint` | 211 | endpoint déjà résolu ; sur échec, remet la tâche à son statut d'origine |
| doc de module | 1-30 | énonce la prémisse que ce vol renverse : le bras du worker « exige une base ouverte, une identité sur disque et une session ouverte sur spirit. Rien de ce bras n'est atteignable depuis un test » |
| test de référence | 995 | `mika1727_v3b1_loverride_de_set_model_se_lit_et_se_tait` ; son commentaire l. 1021 nomme « le maillon que la table seule ne mesure pas » |
| fixtures | 390, 418 | `spawn_mock` (axum, route déclarée `/a2a/{agent}`, capture `last_body`), `ok_task_with_text` |
| gardes | 1134, 1205, 1237, 1296 | scans de crate à aiguilles (`agent::run_agent(`, `AgentParams`, `TaskEngine::new(`, …), `V1_ALLOWED` et `V2A_ALLOWED` vides et asserties vides, anti-vacuité sur les deux fichiers du vol |

**Aucune garde du crate ne prend un paramètre, une arité ou un lecteur de `spirit_url()`
pour aiguille.** Les quatre scans portent sur des gestes (`agent::run_agent(`,
`TaskEngine::new(`, `make_llm_provider(`, `AppContext`, …). Le critère 5 tient donc sans
retoucher un motif, un témoin ni une allowlist : c'est mesuré, pas espéré.

### Le factice, et ce qu'il sait faire

`rewind_remote::tests::fake_spirit` (`rewind_remote.rs` l. 580) est `pub(crate)` depuis
V2c-1b, et `chat.rs` l'emprunte déjà. Sa forme, mesurée :

- il prend une **table de chemins bruts** `&[(&str, StatusCode, &str)]` et sert tout par un
  `fallback` unique (`servir`, l. 552) ; un chemin non associé rend **501**, qu'aucun bras
  métier ne traduit en `NotFound` ;
- il relève `RequeteRecue { methode, chemin, autorisation, corps, requete }` (l. 535) dans
  un `Arc<Mutex<Vec<_>>>` qu'il rend avec l'URL de base ;
- il n'associe **que** le chemin : la chaîne de requête part dans `requete`, donc
  `/api/v1/skills?agent=mika-dev` se sert sous la clé `/api/v1/skills`.

### Autour, hors périmètre, mesuré pour décider

- `DbContext` (`init.rs` l. 11) a ses **quatre champs `pub`** — `settings`, `async_db`,
  `home_dir`, `global_home` — et un `Drop` qui ferme la base. Un test le construit donc
  sans qu'une ligne de production soit ajoutée pour lui.
- `Settings::test_defaults()` est atteignable : `mika-common` est en dev-dependency avec
  `features = ["test-utils"]` (`Cargo.toml`), et `commands/tasks.rs` l. 2287 s'en sert
  déjà. `tempfile` est en dev-dependency aussi.
- `Identity` a un `serde(default)` sur chacun de ses onze champs, et
  `SessionIdentityConfig::singleton` vaut `false` par défaut (`prompt.rs` l. 589). Un
  `identity.toml` **vide** parse donc vers les défauts, et
  `resolve_canonical_session_id` rend `None`.
- `skills_remote::lire` (l. 241) vise `GET {base}/api/v1/skills?agent=<nom>`.
- `session_remote::ouvrir` vise `POST {base}/api/v1/sessions` ; `SourceEnv<'a>` est
  `&'a dyn Fn(&str) -> Option<String>` (`session_remote.rs` l. 147), et `jeton_interne`
  exige `MIKA_INTERNAL_TOKEN`.
- `crates/mika-cli/CLAUDE.md` pèse **88 862 octets** pour un plafond de 100 000
  (`mika-common/tests/claude_md_size.rs`). La section de V3b-2 commence l. **722** ; la
  suivante (`### La route des skills est la source de la TUI …`) l. 760.

### Écarts entre la tête du ticket et la mesure

**É-A — le lecteur d'environnement du spawn ne gouverne pas le jeton du tour.**
`remote_ask::send_message_to_agent` lit `std::env::var("MIKA_INTERNAL_TOKEN")` dans le
processus (`remote_ask.rs` l. 512), et la tête met explicitement ce jeton **hors du vol**
(« `remote_ask.rs` et le jeton du tour (défaut 21) »). Le lecteur passé au spawn gouverne
donc l'ouverture de session et la lecture des skills, pas l'autorisation du tour. **É1
n'assertit rien sur l'en-tête `authorization` du chemin `/a2a/<agent>`** : il lit des
corps, ce que le critère 3 demande mot pour mot. Prétendre le contraire serait une
provenance fausse (mika#2331).

**É-B — chaque envoi est précédé d'un GET que le factice ne sert pas.**
`send_message_to_agent` appelle `budget_floor::attested_envelope_secs` (l. 516), qui vise
`GET {scheme}://{authority}/api/v1/agents/<agent>/budget` dès que l'URL est un `/a2a/<nom>`
en loopback (`budget_floor.rs` l. 129 et l. 55) — ce que l'adresse du factice est. Non
associé dans la table, il rend 501, le plancher reste **non attesté**, et c'est la bonne
lecture. Conséquence pour le test, et seule qui compte : **le dernier élément du journal
n'est pas le tour**, donc les assertions filtrent sur le chemin au lieu de prendre
`last()`. Le vol ne sert pas cette route : un corps de budget inventé par un factice
serait un fait que personne n'a attesté.

**É-C — le critère 1 porte sur la fonction, pas sur le fichier.**
« Elle ne lit plus ni `spirit_url()` ni `std::env::var` » vise `spawn_agent_worker`. Après
ce vol, `chat.rs` garde cinq autres lecteurs de `spirit_url()` — `CallbackFeed::open_for`
(l. 642), l. 715, les deux `clore_session_de_sortie` (l. 789, l. 919) et le chemin des
équipes (l. 1028) —, **tous hors de cette fonction**, plus les trois appelants qui
deviennent lecteurs par ce vol. Une QA qui compte les `spirit_url()` du fichier en trouvera
huit et ne doit pas y lire une dérive.

**É-D — les jumeaux à endpoint résolu restent, et c'est délibéré.**
`send_turn_to_endpoint` et `livrer_tour_de_callback_sur_endpoint` gardent leur raison
d'être : treize sites de test les visent déjà. Ce vol ne les retire pas ; il rend
**leurs appelants publics** testables à leur tour, par la même technique et un niveau plus
haut. Leurs docs ne mentent donc pas après le vol, et ne sont pas réécrites.

---

## 2. Décisions

**D1 — la forme exacte de la nouvelle signature.**

```rust
async fn spawn_agent_worker(
    ctx: DbContext,
    base_url: &str,
    lire_env: SourceEnv<'_>,
    agent_name: &str,
    session_id: Option<&str>,
    model_override: Option<String>,
) -> Result<( /* 8-uplet inchangé */ )>
```

Les deux arguments du monde arrivent **en positions 2 et 3**, et pas à la fin : c'est
l'ordre d'`ouvrir_session_par_spirit`, de `lire_les_skills` et de
`clore_session_de_sortie`, les trois précédents du même fichier, et un lecteur des trois
appelants lit la paire comme une unité. `&str` + `SourceEnv<'_>` plutôt qu'un type neuf :
le crate a déjà ce vocabulaire à six sites de production, et une structure que seuls les
tests construiraient serait une abstraction sans second lecteur. **Le préfixe littéral
`async fn spawn_agent_worker(` ne change pas** : deux gardes le prennent pour témoin
(l. 1929, l. 2044), et le 8-uplet de retour ne bouge pas non plus.

**D2 — l'adresse vit dans le worker sous `worker_base`.**

```rust
let worker_base = base_url.to_string();
```

Posée à côté de `worker_agent_name` (l. 313) et pour la même raison : la tâche
`tokio::spawn` est `'static`, un `&str` ne peut pas y entrer. Le commentaire dira ce qu'on
ne lit pas dans le code — le worker garde l'adresse **reçue au spawn**, donc un respawn
(bascule d'agent, `/restart`) parle au spirit que son appelant a nommé, et non à celui que
l'environnement aurait entre-temps désigné.

**D3 — `spirit_turn_endpoint` prend l'adresse de base.**

```rust
fn spirit_turn_endpoint(base_url: &str, agent_name: &str) -> String {
    format!("{base_url}/a2a/{agent_name}")
}
```

**Pas de `trim_end_matches('/')`**, et c'est une décision, pas un oubli : le `format!`
d'aujourd'hui n'en a pas, un `MIKA_SPIRIT_URL` à barre finale produit donc déjà
`…//a2a/<agent>` sur main, et normaliser ici serait un changement de comportement que ce
vol ne mesure pas — avec une branche que rien ne couvrirait. `skills_remote::lire` dit
dans sa doc pourquoi elle coupe là où elle a deux lecteurs à tenir d'accord ; ici il n'y
en a qu'un. **Le vol déplace l'adresse, il ne la normalise pas.**

**D4 — les deux envois publics prennent l'adresse en deuxième position.**

`send_turn_to_spirit(text, base_url, agent_name, session_id, callback_turn,
model_override)` et `livrer_tour_de_callback(db, base_url, agent_name, session_id, cadrage,
task_id, statut_dorigine, model_override)` : l'adresse se colle au nom d'agent, parce que
c'est la paire dont `spirit_turn_endpoint` fait un endpoint. La doc de chacune des trois
fonctions touchées dit **le pourquoi** : l'adresse arrive par argument pour que le corps du
worker soit atteignable depuis un test, là où la lire dans le processus demandait de muter
`MIKA_SPIRIT_URL` — `unsafe` en édition 2024, et faux sous un `cargo test` parallèle. La
doc de module de `chat_turn.rs` perd la phrase que ce vol rend fausse (« Rien de ce bras
n'est atteignable depuis un test ») et nomme le test qui la remplace.

**D5 — les trois appelants de `run` passent l'adresse et la source.**

Aux l. 614, 837 et 950, chacun ajoute `&crate::commands::dashboard::spirit_url()` et
`&|nom| std::env::var(nom).ok()` — les deux expressions littérales qui partent des
l. 248-249 et 268-269. Elles ne sont ni hissées dans une variable partagée, ni résolues une
fois pour `run` : les trois sites sont des démarrages distincts, et un seul `spirit_url()`
mémorisé au premier ferait qu'un `/restart` parlerait à l'adresse d'avant. **Un seul
commentaire pour les trois**, au premier site, parce que la raison est la même et qu'un
pourquoi répété trois fois est un pourquoi que personne ne relit.

**D6 — le `DbContext` de test se construit sur place.**

```rust
/// Un contexte que le test possède en entier : base en mémoire comme
/// `base_en_memoire`, réglages canoniques, et un home temporaire qui porte un
/// `identity.toml` vide.
fn contexte_de_test(home: &TempDir) -> DbContext { /* … */ }
```

Les quatre champs de `DbContext` sont `pub` : rien n'est ajouté à la production pour ce
test. Le `TempDir` est **tenu par l'appelant** et non par la fixture, pour que son
répertoire survive à l'appel. L'`identity.toml` est écrit **vide** plutôt qu'absent : un
fichier absent route `load_identity` par son chemin fail-closed, qui journalise un
incident et pose une posture que ce test ne veut pas exercer ; un fichier vide parse vers
les défauts, `singleton` y vaut `false`, et l'ouverture prend donc la branche `POST` —
celle qu'É1 mesure. Deux lignes pour rester sur le cas ordinaire.

**D7 — ce que le factice sert, et sous quelles clés.**

Trois entrées, dont une déjà écrite dans ce module :

| clé | rôle | corps |
|---|---|---|
| `ROUTE_SESSIONS` = `/api/v1/sessions` | l'ouverture de session (existe l. 1346) | `CORPS_OUVERT` (existe l. 1347) |
| `ROUTE_SKILLS` = `/api/v1/skills` | le registre, lu à chaque spawn | un miroir vide : `{"agent":"mika-dev","skills":[],"skipped":[],"warnings":[]}` |
| `ROUTE_A2A` = `/a2a/mika-dev` | les tours et les callbacks | un `Task` JSON-RPC complété, de la forme d'`ok_task_with_text` |

`ROUTE_A2A` est écrite **en entier**, et non composée depuis le nom d'agent : c'est
l'argument déjà porté par `ROUTE_FIN` l. 1350-1353 — composer ici rejouerait le `format!`
de la porte, et une faute dans celui-ci passerait inaperçue des deux côtés. Le registre est
servi **vide** parce qu'aucune assertion d'É1 ne le lit ; le servir est ce qui évite qu'un
501 se glisse dans le journal au milieu des tours.

**D8 — chaque assertion lit le journal filtré sur le chemin du tour.**

```rust
/// Le corps du n-ième POST relevé sur `ROUTE_A2A`, désérialisé.
fn corps_du_tour(recues: &Arc<Mutex<Vec<RequeteRecue>>>, rang: usize) -> Value
```

Filtré, et non `last()` : chaque envoi est précédé d'un GET sur
`/api/v1/agents/mika-dev/budget` (É-B), donc la queue du journal n'est pas le tour. Le
rang est explicite parce que l'ordre **est** la propriété mesurée : un helper qui rendrait
« le dernier » laisserait passer un override appliqué un tour trop tard.

**D9 — les cinq points vivent dans une seule fonction de test.**

`mika1727_v3c2c_apres_un_setmodel_le_tour_suivant_porte_loverride`, dans le `mod tests` de
`chat.rs` (l. 1333), là où `fake_spirit`, `base_en_memoire` et `jeton_pose` sont déjà
importés. Un test par point voudrait cinq workers et cinq factices, et **l'ordre cesserait
d'être mesuré** — or c'est lui la propriété : un même `worker_override` traversant quatre
tours. C'est la forme de `mika1727_v3a2a_la_cle_de_callback_voyage_et_son_absence_aussi`
(deux moitiés dans une fonction) et celle du test de référence de V3b-1.

L'ordre, et ce que chaque point lit :

1. worker lancé avec `Some("groq/kimi-k2")` → `Message` → le corps du tour 0 porte
   `MODEL_OVERRIDE_KEY == "groq/kimi-k2"` ;
2. `SetModel { model: "anthropic/claude-sonnet-4-6" }` puis `Message` → le tour 1 porte la
   **nouvelle** valeur ;
3. `SetModel { model: "   " }` puis `Message` → le tour 2 **ne porte pas la clé**
   (`metadata.get(MODEL_OVERRIDE_KEY).is_none()`), moitié négative sans laquelle un
   `Some(String::new())` resterait vert ;
4. `CallbackResult { … }` → le tour 3 porte `CALLBACK_TURN_KEY == true` **et** rien sous
   `MODEL_OVERRIDE_KEY`, puisque le point 3 a effacé l'override — l'override du callback
   suit celui du worker, ce qui est exactement la propriété ; un quatrième `SetModel` avant
   le callback mesurerait deux choses à la fois ;
5. `Quit` → le worker s'arrête.

Aucun `sleep` nulle part : le worker traite ses requêtes en série, donc la réponse reçue
sur `agent_rx` après un `Message` atteste que le `SetModel` qui le précède est déjà
appliqué. C'est la synchronisation, et elle est structurelle.

**D10 — le point 5 pose `shutdown_initiated` avant `Quit`.**

Comme la boucle de chat le fait sur ses sorties attendues : sans cela,
`supervision::supervise` lit l'`Ok(())` du worker comme « worker exited before session
closed » et pousse un `WorkerCrashed`, et le test assertirait sur un faux incident. Le test
pose donc `worker.shutdown_initiated.store(true, Ordering::Release)`, envoie `Quit`, attend
`&mut worker.supervisor_handle` — `worker` reste entier, donc son `_ctx` ferme la base en
fin de test —, puis vérifie que `agent_rx.recv().await` rend `None` : les deux émetteurs
sont partis, le worker et son superviseur avec.

**D11 — `fake_spirit` est emprunté, et `RequeteRecue` aussi.**

L'import existe (l. 1337-1338) et son commentaire dit déjà pourquoi : « un second témoin
de socket ouvrirait une seconde vérité sur ce que le serveur reçoit, libre d'en diverger ».
Ce vol n'ajoute qu'un `use` pour nommer `RequeteRecue` dans la signature de D8 — le type
est `pub(crate)` depuis l. 535. **Aucune ligne n'est recopiée depuis `rewind_remote.rs`.**

---

## Amendements du pilote (2026-11-18)

Deux points sur lesquels le code s'écarte des décisions ci-dessus. Ils sont écrits **avant**
le commit du code qui s'en écarte, comme la tête de mika#1727 l'exige. Aucun des deux ne
lève une halte ni ne change un critère.

**A1 — D4 : `livrer_tour_de_callback` prend la tâche en paire nommée, et non en deux
arguments.**

La forme écrite en D4 — `livrer_tour_de_callback(db, base_url, agent_name, session_id,
cadrage, task_id, statut_dorigine, model_override)` — fait **huit** arguments, et déclenche
`clippy::too_many_arguments` (8/7) à `chat_turn.rs:206`. La CI lance
`cargo clippy --all-targets --all-features -- -D warnings` (`.github/workflows/ci.yml:76`),
donc le warning rougit la PR, et le **critère 9 interdit tout `allow` neuf** : il n'y avait
pas d'issue sans toucher à la liste.

Les deux arguments retirés de la liste voyagent dans un type de ce fichier :

```rust
pub(crate) struct TacheDeCallback<'a> {
    pub task_id: &'a str,
    pub statut_dorigine: &'a str,
}
```

Ce sont les deux qui ne se lisent **qu'ensemble**, et qu'au seul bras d'erreur de
`livrer_tour_de_callback_sur_endpoint` : l'identifiant seul ne dit pas à quel statut rendre
la ligne, le statut seul ne nomme aucune ligne. Les autres candidats au regroupement
auraient été pires : `base_url` + `agent_name` sont la paire dont D4 fait tout son argument
de position, et un type sur elle aurait dû traverser `send_turn_to_spirit` aussi, pour un
seuil que cette fonction-là ne frôle pas.

**Ce que l'amendement ne change pas** : l'adresse reste en **deuxième position**, le
pourquoi reste dans la doc, `spirit_turn_endpoint` (D3) est intacte, et
`livrer_tour_de_callback_sur_endpoint` passe de 7 à 6 arguments — donc les deux livraisons
ont de la marge sous le seuil. Les trois sites de test de `chat_turn.rs` qui appellent la
variante `_sur_endpoint` nomment la paire au lieu de deux positions, et leurs assertions ne
bougent pas.

**A2 — D7 : le corps du registre des skills est emprunté, pas écrit ici.**

D7 prescrit pour `ROUTE_SKILLS` « un miroir vide :
`{"agent":"mika-dev","skills":[],"skipped":[],"warnings":[]}` ». Le code emprunte
`skills_remote::tests::CORPS` (`pub(crate)`, `skills_remote.rs` l. 327) à la place, pour la
raison que porte la doc de cette constante : c'est l'unique corps de fil de
`/api/v1/skills` du crate, et elle interdit explicitement qu'un appelant en écrive un
réduit — « un second corps de fil divergerait du premier sans qu'aucun test ne le voie ».
Écrire le corps de D7 aurait été exactement le geste que le critère 6 refuse pour
`fake_spirit`, un fichier plus loin.

Au passage : **`ROUTE_SKILLS` existe déjà** dans le `mod tests` de `chat.rs` (l. 1820), donc
ce vol n'en ajoute pas la constante — contrairement à ce que la ligne de D7 laissait
attendre. `ROUTE_SESSIONS`, `CORPS_OUVERT` et `ROUTE_A2A` sont inchangées, `ROUTE_A2A`
toujours écrite en entier.

---

## Fire-Disposition

Ce vol livre un détecteur (le test É1), donc cette section est obligatoire (mika#2306,
mika#1574).

**Option retenue : (a) exception nommée — livrée armée, et sans aucune exception.**

- **Violations préexistantes : zéro.** É1 est **vert à l'arrivée**. Il ne découvre pas un
  défaut : il rend observable un comportement déjà juste, que la composition de V3b-1
  tenait à une affectation près. Le commit qui l'arme est celui qui le rend possible, et
  rien n'a à être excepté.
- **Aucune allowlist n'est créée.** É1 est un test de comportement, pas un scan de source :
  il n'a ni motif, ni liste, ni fichier à excepter. Les quatre allowlists du crate
  (`V1_ALLOWED`, `V2A_ALLOWED` et les deux `ALLOWED` de `chat.rs`) restent vides, asserties
  vides, et **ne sont pas touchées**.
- **Anti-vacuité : les deux mutations du critère 4, et elles seules.** Un test qui appelle
  tout le worker contre un factice peut rester vert par cécité — un corps lu sur le
  mauvais chemin, un rang qui glisse. La preuve n'est donc pas une assertion de plus mais
  **la démonstration qu'il rougit** : affectation du bras `SetModel` retirée → rouge sur
  les points 2 à 4 ; `None` à la place de `worker_override` dans le bras `Message` → rouge
  sur les points 1 et 2. Chaque mutation est faite **seule**, sur un commit vert, dans un
  worktree détaché, et la PR donne la sortie des deux. C'est le contre-témoin de ce vol,
  au sens de mika#2103, #2205 et #2398.
- **Ce qui arrive quand É1 tire.** Il nomme une régression de comportement : l'override
  cesse d'atteindre le fil. Le remède est le code, jamais une retouche du test — et jamais
  un `#[ignore]`.
- **Option (b) — livrer désarmé : refusée.** Il n'y a rien à désarmer : zéro violation, et
  un test de comportement désarmé ne mesure rien du tout.
- **Option (c) — halte et escalade : sans objet.** Rien n'oblige à s'arrêter ici : le total
  estimé tient sous la borne du groom, et aucun critère ne demande de toucher un fichier
  exclu.

---

## 3. Ordre de commit

Deux commits, par livrable, comme la tête le demande : « la signature et le test É1, puis
la doc ». Chacun compile et passe ses tests. **Les tests ne se reportent jamais à un vol
suivant.**

### C1 — l'adresse arrive par argument, et É1 la mesure (critères 1 à 6, 9)

`crates/mika-cli/src/commands/chat.rs` et `crates/mika-cli/src/commands/chat_turn.rs`, dans
le même commit : la signature sans son test serait un changement sans mesure, et le test
sans la signature ne compilerait pas.

1. D3, D4 : `spirit_turn_endpoint`, `send_turn_to_spirit`, `livrer_tour_de_callback` et
   leurs docs ; la doc de module perd la phrase que le vol rend fausse.
2. D1, D2 : la signature de `spawn_agent_worker`, sa doc, les l. 248-249 et 268-269
   devenues `base_url` / `lire_env`, `worker_base`, et les deux bras d'envoi.
3. D5 : les trois appelants.
4. D6 à D11 : la fixture, les constantes du factice, le lecteur du journal, et É1.

`cargo build -p mika-cli --bin mika` **d'abord** (critère 10), puis
`cargo test -p mika-cli` et `cargo clippy --all-targets -p mika-cli`. ≈ 162 lignes.

**Après C1, avant la PR : les deux mutations du critère 4.** Dans un worktree détaché —
`git worktree add --detach .pilot-scratch/mutation-e1 HEAD`, sous un chemin déjà exclu par
`info/exclude` —, une mutation à la fois, `cargo test -p mika-cli
mika1727_v3c2c` après chacune, sortie capturée sous `.pilot-scratch/`. La branche du vol ne
porte **jamais** une mutation. Le scratch ne se supprime pas : ni `rm`, ni `rmdir`, et
`rm -rf` est terminal (mika#2548).

### C2 — la doc dit qu'É1 est mesuré (critère 7)

`crates/mika-cli/CLAUDE.md`, **section de V3b-2 uniquement** (l. 722, jusqu'à la suivante
l. 760). Trois faits, pas un de plus : É1 est mesuré, par quel test (son nom), et l'adresse
de spirit arrive au worker **par le spawn**. Le fichier passe de 88 862 octets à moins de
89 500, pour un plafond de 100 000 ; `cargo test -p mika-common --test claude_md_size` le
mesure. **Aucune autre section ne change** — vérifiable par
`git diff origin/main...HEAD -- crates/mika-cli/CLAUDE.md`. ≈ 12 lignes.

### Fin de pipeline

Dans cet ordre, avant d'ouvrir la PR :

1. `cargo fmt`
2. `cargo build -p mika-cli --bin mika` — vert
3. `cargo clippy --all-targets -p mika-cli` — vert
4. `cargo test -p mika-cli` — vert
5. `cargo test -p mika-common --test claude_md_size` — vert
6. `bash scripts/verify-pipeline.sh origin/main`

**Si le dernier est rouge, le pilote s'arrête et le dit** (critère 10). Il ne contourne pas,
il ne commente pas la garde.

Vérifications de périmètre, à la main, avant la PR :

- `git diff --stat origin/main...HEAD` ne nomme que `commands/chat.rs`,
  `commands/chat_turn.rs`, `crates/mika-cli/CLAUDE.md` et ce plan.
- `git diff origin/main...HEAD -- crates/mika-cli/src/remote_ask.rs crates/mika-cli/src/tui/`
  est **vide**, à l'octet (critère 8) ; de même pour `crates/mika-agent/`,
  `crates/mika-common/`, `crates/mika-a2a/`, `crates/mika-gateway/`, `docs/egress/` et
  `dashboard/`.
- Aucun `#[allow(` ni `#[expect(` neuf dans le diff (critère 9).
- **Aucun fichier du plan de contrôle** : ni `.github/workflows/`, ni `.claude/`, ni
  `skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni `.mika/`, ni
  `docs/egress/`, ni aucun chemin de CODEOWNERS.

**Vol lean (mika#2694)** : pas de `ce-simplify-code`, pas de `ce-code-review`. Le corps de
PR porte `Code review: delegated to mika-qa (mika#2694)`, `Refs mika#1727`, la sortie des
deux mutations du critère 4, et `Tracked in: senara-solutions/mika#1727` pour V4. Il
s'écrit dans `pr-body.md` **sous le worktree**, jamais dans `/tmp`, jamais par heredoc
(mika#2211).

**Gel des tickets jusqu'au 01/12** : ce vol n'ouvre **aucun** ticket, pas même de suivi ni
de sous-ticket. Un manque ou un reste va en commentaire sur mika#1727 ou dans la PR.

### Halte du pilote sur la borne

Après **chaque** commit, mesurer :

```
git diff --numstat origin/main...HEAD -- . ':!docs/plans' | awk '{a+=$1} END{print a}'
```

- **Au-dessus de 310 : arrêt.** Pas de commit suivant, pas de push, pas de PR.
- Le dernier message dit alors, mot pour mot :
  **« HALTE borne : N lignes ajoutées pour `310` »**, suivi du compte après chaque commit.

Cette halte est posée par Vincent, et ce plan ne la change pas. **Une halte ne se lève
jamais par un amendement du plan**, et ce plan n'ajoute aucune halte à celles de la tête.

Les commentaires comptent dans la borne et **ne se rabotent pas** pour la tenir : un
commentaire porte un pourquoi qu'on ne lit pas dans le code, pas une paraphrase. Si le
code dépasse, la règle ci-dessus arbitre.

**Si un hook ou un classifieur refuse un geste**, au pilote ou à un sous-agent, le pilote
**s'arrête et le déclare**, même si le refus paraît un faux positif. Il ne le contourne
jamais par un autre outil : pas de heredoc Bash à la place de `Write`, pas de
reformulation (contre-exemple mika#2869, 2026-10-21).

### Si le code s'écarte de ce plan

La QA juge contre ce plan, pas contre le corps du ticket. Si le pilote doit s'en écarter sur
un point — un fichier, un critère, un nom — **il amende ce plan sur la branche dans un
commit `docs(plans)` avant de pousser le code qui s'en écarte**, en nommant le point et la
raison. Un plan qui dit une chose pendant que le code en fait une autre est bloqué par la
QA, même quand le code est juste.

---

## 4. Definition of Done

- [ ] `spawn_agent_worker` prend `base_url: &str` et `lire_env: SourceEnv<'_>` en positions
      2 et 3, garde le préfixe littéral `async fn spawn_agent_worker(` et son 8-uplet, et
      ne contient plus ni `spirit_url()` ni `std::env::var`.
- [ ] Le worker garde l'adresse sous `worker_base` ; les bras `Message` et `CallbackResult`
      la passent à leur envoi.
- [ ] `spirit_turn_endpoint`, `send_turn_to_spirit` et `livrer_tour_de_callback` prennent
      l'adresse de base ; la doc de chacune dit pourquoi elle arrive par argument.
- [ ] La doc de module de `chat_turn.rs` ne dit plus que le bras du worker est hors
      d'atteinte d'un test, et nomme É1.
- [ ] Les trois appelants de `run` (l. 614, 837, 950) passent `spirit_url()` et
      `std::env::var`, avec un commentaire au premier site.
- [ ] `mika1727_v3c2c_apres_un_setmodel_le_tour_suivant_porte_loverride` vit dans le
      `mod tests` de `chat.rs`, appelle `spawn_agent_worker` en entier contre `fake_spirit`,
      sans `unsafe` et sans muter l'environnement du processus, et montre les cinq points
      dans l'ordre de D9 sur les corps relevés par le factice.
- [ ] Les deux mutations du critère 4 sont faites séparément dans un worktree détaché, et
      leur sortie est dans la PR.
- [ ] `fake_spirit` est emprunté ; aucune ligne de factice n'est recopiée.
- [ ] Les gardes de `chat.rs` et de `chat_turn.rs` sont vertes, motifs, témoins et
      allowlists inchangés.
- [ ] La section de V3b-2 de `crates/mika-cli/CLAUDE.md` dit qu'É1 est mesuré, par quel
      test, et que l'adresse vient du spawn ; aucune autre section ne change ; le fichier
      reste sous 100 000 octets.
- [ ] `crates/mika-cli/src/remote_ask.rs` et `crates/mika-cli/src/tui/` sont inchangés à
      l'octet ; aucun fichier du plan de contrôle n'est touché.
- [ ] Aucune attente ni aucun `allow` neuf.
- [ ] `cargo build -p mika-cli --bin mika`, puis `cargo test -p mika-cli`,
      `cargo test -p mika-common --test claude_md_size` et
      `cargo clippy --all-targets -p mika-cli` verts ;
      `bash scripts/verify-pipeline.sh origin/main` lancé et vert.
- [ ] Au plus 310 lignes ajoutées, plan exclu.
- [ ] La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves` — corps et
      commits. Si le gabarit de reprise l'ouvre avec un `Closes`, il est corrigé **avant
      toute revue**, puis `gh pr view <N> --json closingIssuesReferences` rend `[]`.
- [ ] Aucun ticket ouvert par ce vol (gel jusqu'au 01/12) ; le reste est noté
      `Tracked in: senara-solutions/mika#1727`.

---

## 5. Acceptance criteria

Les onze critères de V3c-2c, transcrits du corps du ticket.

1. `spawn_agent_worker` reçoit l'adresse de spirit et le lecteur d'environnement en
   arguments. Elle ne lit plus ni `spirit_url()` ni `std::env::var`. Ses trois appelants
   passent `spirit_url()` et `std::env::var`.
2. Les deux envois du worker prennent l'adresse reçue par le spawn, et ne relisent plus
   `spirit_url()`.
3. Un test appelle `spawn_agent_worker` contre `fake_spirit`, sans `unsafe`, sans muter
   l'environnement du processus, et montre les cinq points du périmètre sur les corps
   relevés par le factice.
4. Ce test rougit quand l'affectation du bras `SetModel` est retirée, et quand le bras
   `Message` ne passe plus `worker_override`. La PR donne la sortie de chaque mutation,
   faite seule dans un worktree détaché.
5. Les gardes existantes de `chat.rs` et de `chat_turn.rs` restent vertes, sans retouche de
   leurs motifs, de leurs témoins ni de leurs allowlists.
6. `fake_spirit` est emprunté, jamais recopié.
7. Dans `crates/mika-cli/CLAUDE.md`, la section de V3b-2 dit qu'É1 est mesuré, par quel
   test, et que l'adresse de spirit vient du spawn. Aucune autre section ne change.
8. `remote_ask.rs` et `tui/` ne changent pas. `git diff origin/main...HEAD` ne touche que
   les fichiers listés plus haut.
9. Aucune attente ni aucun `allow` neuf.
10. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-cli` et
    `cargo test -p mika-common --test claude_md_size` sont verts.
    `cargo clippy --all-targets -p mika-cli` est vert. Le pilote lance
    `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il le trouve
    rouge, il s'arrête et le dit.
11. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si le
    gabarit de reprise l'ouvre.

**Comment le critère 3 est tenu là où V3b-1 ne pouvait pas le tenir** : voir § 0 et É-A.
Le critère porte sur les **corps** relevés par le factice, et non sur l'autorisation du
tour — que `remote_ask` lit encore dans le processus, hors de ce vol.

---

## Taille estimée

| livrable | lignes estimées |
|---|---|
| `chat.rs` — signature de `spawn_agent_worker`, sa doc, les deux lectures devenues des arguments (C1) | 16 |
| `chat.rs` — `worker_base` et les deux bras d'envoi (C1) | 6 |
| `chat.rs` — les trois appelants de `run` et leur commentaire (C1) | 10 |
| `chat_turn.rs` — `spirit_turn_endpoint`, les deux envois publics, leurs docs et la doc de module (C1) | 20 |
| test É1 — fixtures : `contexte_de_test`, constantes du factice, `corps_du_tour`, imports (C1) | 40 |
| test É1 — les cinq points dans une fonction, doc comprise (C1) | 70 |
| `crates/mika-cli/CLAUDE.md` — la section de V3b-2 dit qu'É1 est mesuré (C2) | 12 |

Total estimé : 174 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités diffèrent de 12 lignes ici.** L'unité de l'architecte exclut `docs/` et
les `CLAUDE.md`, donc elle compte **162** ; celle des bornes de ce vol
(`git diff --numstat`, plan exclu) compte les 174, `CLAUDE.md` inclus. **174 pour 220, et
162 pour 429 : les deux bornes tiennent, tests compris.** Le total estimé ne dépasse pas la
borne du groom, donc ce plan ne s'arrête pas et ne demande aucune coupe — et il n'en
pratique aucune de lui-même : une coupe demande le mot de Vincent.

Marge : 46 lignes avant la borne du groom, 136 avant la halte du pilote. Après C1 (≈ 162),
la halte de 310 est loin ; c'est le § 3 qui arbitre, pas cette estimation.

Tracked in: senara-solutions/mika#1727
