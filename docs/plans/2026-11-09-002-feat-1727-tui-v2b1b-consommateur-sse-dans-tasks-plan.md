# mika#1727 — vol V2b-1b : le consommateur SSE des tâches devient réutilisable, sans quitter `tasks.rs`

- **Ticket :** senara-solutions/mika#1727
- **Branche :** `feat/1727/tui-v2b1b-consommateur-sse-dans-tasks` (neuve, depuis `main`)
- **Base :** `b78f62291` (merge de V2b-1 part serveur, mika#3064)
- **PR :** `Refs mika#1727` — **jamais `Closes`**. Le ticket se ferme au merge de V4.
- **Matériau mesuré :** `refs/pull/3062/head`, commits `334b80278` (le consommateur, dans un
  fichier neuf) et `1f24523a7` (le serveur factice `spawn_fake_spirit`).

---

## 0 — Ce qui est mesuré sur `b78f62291`, avant toute conception

Trois des quatre critères de ce vol portent sur du code dont **aucun test n'existe**. Le quatrième
porte sur un lint. Les quatre mesures qui décident du plan :

1. **`crates/mika-cli/src/commands/tasks.rs` fait 1 377 lignes**, et son dernier item de production
   est `task_to_json` (l. 834). Le bloc `#[cfg(test)] mod tests {` commence l. 859-860 et court
   jusqu'à la fin du fichier (l. 1377). Il n'y a **aucune production après la coupe**.
2. **`stream_task_events` (l. 409-471)** résout l'URL de base (`dashboard::spirit_url`) et le jeton
   (`dashboard::auth_token`) **à l'intérieur** d'elle-même, construit son client l. 420
   (`reqwest::Client::new()`), tient le tampon, découpe, et écrit sur `std::io::stdout()`. Son seul
   appelant est la l. 398 (`mika tasks stream`, `cli.rs:1105`).
3. **`find_event_separator` (l. 481-494)** est correcte et documentée, et `git grep` sur
   `crates/mika-cli` ne la trouve qu'aux l. 453 et 481 : **aucun test ne l'appelle**.
4. **`axum` et `futures-util` sont en `[dev-dependencies]`** de `mika-cli`
   (`Cargo.toml:68,71`), donc disponibles sous `cfg(test)` et nulle part ailleurs. Le modèle de
   serveur factice du crate est `axum::serve` sur `TcpListener::bind("127.0.0.1:0")` dans un
   `tokio::spawn` (`commands/chat_turn.rs:245-253`).

Et la mesure qui gouverne le critère 4 : **`dashboard::auth_token()` lit l'environnement et échoue
sans lui** (`commands/dashboard.rs:32-42` — `MIKA_INTERNAL_TOKEN`, puis `MIKA_DASHBOARD_TOKEN`,
puis `.context(…)`). `spirit_url()` lit `MIKA_SPIRIT_URL` (l. 18-28). Les tests de
`commands/agents.rs` **mutent** `MIKA_SPIRIT_URL`. C'est cette mesure, et non une préférence de
style, qui impose que l'URL et le jeton voyagent **par argument** : un test qui les laisserait se
résoudre serait tantôt rouge sur un poste sans jeton, tantôt perturbé par le voisin.

### 0.1 — Les deux bornes de ce vol ne sont pas dans la même unité, et c'est la plus serrée qui mord

Le corps borne ce vol à **450 lignes ajoutées**, « code et tests compris, plan exclu, comptées comme
`git diff --numstat origin/main...HEAD` », et précise : « Cette borne est en lignes livrées : on n'y
applique aucun facteur. Le plan écrit son `Total estimé :` dans cette même unité. »

La règle mika#2693 borne le même vol à **279 lignes estimées**, un nombre qui porte déjà un facteur
3 dans sa dérivation (`(150 − 27 − 30) × 9 / 3`).

Ce plan ne se sert pas de cette différence d'unité. Il écrit son total en lignes **ajoutées**, comme
le corps l'exige, et le soumet **tel quel** à la borne de 279 — sans invoquer le facteur en sa
faveur. C'est la lecture conservatrice, et c'est elle qui a dicté les trois retraits du § 0.2 : le
périmètre nommé par le corps, pris en entier depuis le matériau, pèse ~307 lignes ajoutées et
passerait 450 sans passer 279.

### 0.2 — Les trois choses que ce vol ne reprend pas du matériau, et pourquoi

`334b80278` livrait le consommateur dans un fichier neuf `commands/task_events.rs` (241 lignes
ajoutées). Trois de ses morceaux ne viennent pas ici, et le § 0.1 du plan V2b-1 les avait déjà
nommés :

| morceau du matériau | lignes | disposition ici |
|---|---|---|
| le **fichier neuf** `commands/task_events.rs` | — | **non** : c'est lui qui a rendu #3062 rouge au lint egress (D1 sur `task_events.rs:85`, D3 sur `cli-spirit-tasks`). Le consommateur est promu **dans `tasks.rs` même** (§ 0.3) |
| `frame_agent_id` + son test, et le paramètre `agent_filter` de `consume` | ~35 | **non** : ils ne servent qu'au consommateur **filtré** de V2b-2. Livrés ici, ils seraient un filtre sans appelant filtrant (mika#2205) |
| le décodage de la charge en `TaskEventFrame` | ~12 | **non** : le seul appelant de ce vol relaie des octets. Décoder sans lecteur du décodé est la même faute une couche plus bas (§ D2) |
| `stream_url` exposée + son test | ~17 | **non** : l'URL se construit dans `open`, qui est le seul endroit qui en a besoin |

`Tracked in: senara-solutions/mika#1727` — le filtre par agent, le décodage en `TaskEventFrame` et
la variante du serveur factice dont le flux ne se ferme jamais partent en **V2b-2**, avec leur
appelant. Aucun ticket n'est ouvert (gel jusqu'au 01/12).

### 0.3 — Le critère 4 est tenu par construction, et la construction se démontre sur le scanner

Le critère 4 demande Egress Manifest Lint vert **sans toucher `docs/egress/`**. Trois faits le
rendent vrai par forme, pas par vigilance :

- **D1 (code → manifeste).** `CLIENT_CONSTRUCTION_RE`
  (`scripts/lib/egress_manifest_lint.py:148-152`) inventorie les fichiers qui portent
  `Client::new()`. Ce vol garde cette construction dans `tasks.rs`, dans `TaskEventStream::open` —
  et l'entrée `cli-spirit-tasks` (`docs/egress/egress-manifest.toml:351-360`) déclare déjà
  `client_site = "crates/mika-cli/src/commands/tasks.rs"`. Aucun fichier neuf, donc aucun fichier
  non déclaré.
- **D3 (manifeste → code).** Le `client_site` déclaré continue de couvrir un fichier de
  l'inventaire D1, puisque le client n'en sort pas. C'est très exactement la D3 qui rougissait sur
  #3062 : là, le client avait déménagé et la déclaration était devenue fantôme.
- **D2 (littéraux de host).** Le serveur factice lie `"127.0.0.1:0"`, couvert par
  `LOOPBACK_V4_RE` (l. 252), et sert sur `format!("http://{addr}")`, une forme dont
  `normalize_host` rend `None` (gabarit sans point résiduel, l. 196-214). Aucun host à déclarer.

**Où le bloc de test se place, et pourquoi ce n'est pas un détail.** Le scanner coupe le fichier au
**premier** `#[cfg(test)]` suivi d'un `mod X {` en bloc, et `INLINE_TEST_MOD_RE` (l. 274) reconnaît
`pub(crate) mod` aussi bien que `mod`. Il ne reprend pas après la fermeture du bloc : c'est le trou
assumé que le cas N15 de `scripts/test-verify-egress-manifest.sh` remesure à chaque run. Le module
`fake_spirit` se place donc **après** le `mod tests` existant (l. 859), en fin de fichier, là où la
coupe a déjà eu lieu : son effet sur le scanner est exactement nul, et N15 ne voit aucune
production nouvelle derrière la coupe. Le placer avant déplacerait la coupe de 859 à ~500 et
rendrait les l. 506-858 invisibles au lint — sans conséquence aujourd'hui (elles ne portent aucun
sink), ce qui est précisément ce qui en ferait un piège pour le prochain qui y ajoute un appel.

Le pilote lance quand même `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR,
comme le critère l'exige. **S'il le trouve rouge, il s'arrête et le dit** : corriger le manifeste
est le geste de Vincent (CODEOWNERS `@samidarko`), et un lint supposé vert est la classe de faute
que mika#2205 nomme.

### 0.4 — Ce que ce vol ne touche pas

`crates/mika-cli/src/tui/app.rs` et `crates/mika-agent/` ne sont pas modifiés. Aucun fichier du
plan de contrôle : `.github/workflows/`, `.claude/`, `skills/bundled/`,
`crates/mika-agent/src/well_known_agents.rs`, `.mika/`, `docs/egress/`. La commande
`mika tasks stream` garde sa signature CLI, sa sortie et ses deux lignes de stderr.

---

## 1 — Les points de conception

### D1 — Ouvrir et lire sont deux gestes, parce que les deux appelants les distinguent

`TaskEventStream::open(base_url: &str, token: &str)` puis `consume(self, on_payload)`, et non un
seul appel. Trois raisons, dont deux sont déjà visibles :

1. `mika tasks stream` imprime « connected » **entre** les deux — un fil connecté mais inactif ne
   doit pas se lire comme un fil pendu. Ce comportement existe aujourd'hui (l. 437) et ce vol le
   conserve ; il ne serait pas exprimable avec un appel unique.
2. L'abonnement de la TUI (V2b-2) recule sur un échec d'**ouverture** et reconnecte simplement sur
   une **fin** de flux. Les deux cas sont le même `Err` si l'ouverture est dans la boucle.
3. **L'URL et le jeton sont des paramètres, pas des défauts résolus dedans** (§ 0, mesure finale).
   La résolution reste dans `stream_task_events`, qui n'est pas testée et n'a rien à l'être : elle
   appelle deux fonctions publiques de `dashboard` et passe leur résultat.

### D2 — Le consommateur rend la charge `data:` telle qu'elle est arrivée, et rien de plus

`consume` prend `on_payload: impl FnMut(&str)` et lui donne la charge brute. Il ne désérialise pas.

C'est le point où ce plan s'écarte du matériau, et le doc-comment du matériau donne lui-même la
raison : « `on_event` receives the `data:` payload **as it came off the wire** […] so a caller that
only relays bytes (`mika tasks stream`) does not depend on this binary knowing every `event`
value ». Le matériau décodait **en plus**, parce qu'il filtrait par agent. Sans filtre, le décodage
n'a aucun lecteur — et un décodage sans lecteur est la faute que ce vol corrige une couche plus
haut : #3062 a livré un fil, puis un consommateur, chacun avant son lecteur.

Conséquence mesurable : `tasks.rs` ne gagne **aucune** dépendance à
`mika_agent::server::tasks_stream::TaskEventFrame`. V2b-2 l'ajoutera avec le filtre qui la lit.

### D3 — `drain_events` sort du corps de la boucle, parce que c'est la seule façon d'attester le recollage sans dépendre d'un cadencement

Le critère 1 exige qu'« un [test rougisse] si un événement est coupé entre deux paquets ». Cette
propriété est celle du **tampon**, pas de la socket : un événement arrive coupé, la première
itération n'en voit qu'un fragment, la seconde complète. Deux façons de la mesurer :

- **par la socket** : faire émettre le serveur factice en deux morceaux séparés d'un `sleep`. Le
  test devient tributaire d'un cadencement, et si une couche coalesce les deux morceaux, il passe
  **sans rien mesurer** — vert pour la mauvaise raison ;
- **par la fonction** : `fn drain_events(buffer: &mut Vec<u8>, on_payload: &mut impl FnMut(&str))`,
  pure, appelée deux fois avec les deux moitiés choisies à la main. Le test rougit si la charge
  rendue est un fragment, et il rougit de la même façon sur toutes les machines.

La seconde, donc. `consume` devient `extend_from_slice` + `drain_events` dans une boucle de trois
lignes ; le corps de `drain_events` est celui des l. 450-468 d'aujourd'hui, déplacé sans changement
de logique — `find_event_separator` comprise, qui n'est **pas** modifiée (§ D4).

### D4 — `find_event_separator` ne bouge pas d'une ligne, et c'est ce qui donne son sens au critère 3

Le critère 3 demande qu'elle soit testée sur ses trois séparateurs. Elle est correcte aujourd'hui,
son doc-comment nomme la propriété porteuse (l'ordre d'essai : `\r\n\r\n` avant `\n\n`, sans quoi un
`\r\n\r\n` serait lu comme le `\n\n` à l'offset+1 et laisserait un `\r` en tête de l'événement
suivant), et **rien ne la mesure**. Les trois tests de `334b80278` sont repris tels quels.

L'ordre des commits (§ 4) les fait atterrir **avant** le refactor. Ce n'est pas une préférence :
« la découpe est préservée » est, sans eux, une affirmation sur un diff — exactement ce que
`334b80278` a affirmé sans test.

### D5 — Le serveur factice sert un corps **fini**, et V2b-2 ajoutera la variante ouverte

`#[cfg(test)] pub(crate) mod fake_spirit` expose
`async fn spawn(payload: String) -> String` (l'URL de base) et `fn sse_payload(&[&str]) -> String`
(l'encodeur : une charge par `data: …\n\n`).

La réponse porte `content-type: text/event-stream` et un corps complet, puis se ferme — `consume`
rend alors `Ok(())` et le test assert la **séquence entière** sans timeout. Le factice de
`1f24523a7` chaînait un `futures_util::stream::pending()` pour que le flux ne finisse jamais, parce
que l'abonnement de la TUI reconnecte sur une fermeture propre et rejouerait les trames. Ce vol n'a
pas d'abonnement : la variante ouverte arrive en V2b-2, avec le code qui en a besoin. Le dire ici
vaut mieux que de livrer un drapeau `keep_open` dont un seul côté a un appelant.

Le module se place **après** le `mod tests` existant pour la raison du § 0.3, et il est
`pub(crate)` pour que les tests de `tui/app.rs` le prennent en V2b-2 : `#[cfg(test)]` le limite à
la compilation de test, où `axum` et `futures-util` sont disponibles (§ 0, mesure 4).

### D6 — Le critère 2 se tient par un puits paramétré, pas par une capture de stdout

« Une ligne par charge `data:`, rien d'autre sur stdout » n'est mesurable qu'en comparant **tout**
le flux de sortie à une chaîne attendue. `std::io::stdout()` n'est pas capturable depuis un test
Rust sans sous-processus. Donc :

```rust
async fn stream_task_events_into<W: Write>(base: &str, token: &str, out: &mut W) -> Result<()>
```

`stream_task_events(url_override)` résout l'URL et le jeton, imprime sa ligne de connexion sur
**stderr**, et appelle `stream_task_events_into` avec `&mut std::io::stdout()`. Le test l'appelle
avec un `Vec<u8>` et assert `String::from_utf8(buf) == "{j1}\n{j2}\n{j3}\n"` — une **égalité**, pas
un `contains` : c'est le `==` qui tient « rien d'autre », et un `contains` l'aurait laissé
passer. Les deux `eprintln!` restent sur stderr, donc absents du tampon — ce qui est la mesure, et
non la redite, de « rien d'autre sur stdout ».

`&mut std::io::stdout()` plutôt qu'un `StdoutLock` tenu à travers un `.await` : `Stdout` est `Send`
et verrouille par écriture, ce qui est le comportement d'aujourd'hui (le lock est pris par ligne,
l. 464-466).

---

## 2 — La forme livrée

Dans `crates/mika-cli/src/commands/tasks.rs`, production :

```rust
pub(crate) struct TaskEventStream { resp: reqwest::Response }

impl TaskEventStream {
    pub(crate) async fn open(base_url: &str, token: &str) -> Result<Self>;
    pub(crate) async fn consume(self, on_payload: impl FnMut(&str)) -> Result<()>;
}

fn drain_events(buffer: &mut Vec<u8>, on_payload: &mut impl FnMut(&str));
fn find_event_separator(buf: &[u8]) -> Option<(usize, usize)>;   // INCHANGÉE

async fn stream_task_events(url_override: Option<String>) -> Result<()>;            // l. 398
async fn stream_task_events_into<W: Write>(base: &str, token: &str, out: &mut W) -> Result<()>;
```

`open` conserve mot pour mot le refus explicite d'aujourd'hui sur un statut non-succès (corps
inclus, l. 429-437) : un flux qui ne rend rien en silence est indistinguable d'un agent sans tâche.

---

## 3 — Les tests

Les tests des critères 1 et 3 et le test de `drain_events` vivent dans le `mod tests` existant
(l. 860) ; `fake_spirit` est son frère, après lui.

| test | critère | ce qu'il atteste |
|---|---|---|
| `mika1727_v2b1b_les_trois_separateurs_sont_reconnus` | 3 | `\n\n`, `\r\n\r\n`, `\r\r`, et `None` sur un `\n` seul |
| `mika1727_v2b1b_un_crlfcrlf_nest_pas_lu_comme_un_lflf_decale` | 3 | l'ordre d'essai : `(1, 4)` sur `b"a\r\n\r\nb"`, et le reste du tampon est `b"b"` |
| `mika1727_v2b1b_le_premier_separateur_gagne` | 3 | `(1, 2)` sur `b"a\n\nb\n\nc"` |
| `mika1727_v2b1b_un_evenement_coupe_en_deux_paquets_est_rendu_entier_une_fois` | 1 | `drain_events` sur deux moitiés d'un même JSON : **rien** après la première, la charge **entière** après la seconde (§ D3) |
| `mika1727_v2b1b_le_consommateur_rend_ses_trames_dans_lordre` | 1 | `open` + `consume` contre `fake_spirit` : le `Vec<String>` collecté est **égal** à la séquence servie — ni perte, ni doublon, ni réordonnancement |
| `mika1727_v2b1b_mika_tasks_stream_ecrit_une_ligne_par_charge_et_rien_dautre` | 2 | `stream_task_events_into` sur un `Vec<u8>` : égalité exacte avec `"{j1}\n{j2}\n{j3}\n"` (§ D6) |

**Ce qui rougit.** Perdre une trame, en doubler une, ou les réordonner fait rougir le cinquième ;
rendre un fragment au lieu d'attendre la fin de l'événement fait rougir le quatrième ; écrire quoi
que ce soit d'autre sur le puits — un en-tête, un préfixe, une ligne de connexion mal dirigée vers
stdout — fait rougir le sixième ; abîmer l'ordre d'essai des séparateurs fait rougir le second.

**Aucune variable d'environnement n'est touchée.** L'URL et le jeton voyagent par argument (§ D1),
donc ces tests ne peuvent ni être perturbés par les mutateurs de `MIKA_SPIRIT_URL` de
`commands/agents.rs`, ni les perturber — et aucun `serial_test` n'est nécessaire.

**Ce qu'ils ne couvrent pas** : l'émission des trames depuis `AsyncDatabase`, mesurée par les trois
tests de V2b-1 (`async_db.rs`, mika#3064), et le filtre par agent, qui n'existe pas encore (§ 0.2).

---

## 4 — Fire-Disposition

Ce vol livre six détecteurs — six tests dont le chemin de succès est « aucune violation trouvée ».

**Option (a) — exception nommée en allowlist, avec une allowlist vide.** Les six atterrissent
**armés**, sans `#[ignore]` et sans entrée d'exception, parce qu'il n'existe aucune violation
existante à exempter : `find_event_separator` est correcte sur `b78f62291` (ses trois tests sont
repris de `334b80278`, où ils étaient verts), la découpe d'événements est celle d'aujourd'hui
déplacée sans changement de logique, et la sortie de `mika tasks stream` est celle qu'on épingle.
Rien à nommer, rien à référencer en suivi, donc aucune assertion auto-nettoyante à écrire : la
propriété « l'allowlist est vide » est ici le résultat, pas une dette. En inventer une entrée
produirait l'allowlist de complaisance que mika#2309 condamne.

**Si le pilote les trouve rouges avant le refactor, c'est (c) — halte-et-remontée.** Les trois
tests de `find_event_separator` atterrissent au commit 1, **avant** que le consommateur bouge
(§ D4). S'ils rougissent là, la découpe sur `main` ne fait pas ce que son doc-comment annonce : ce
n'est pas une exception à déclarer, c'est un défaut à remonter en commentaire sur mika#1727 avant
de promouvoir quoi que ce soit. Jamais un `#[ignore]` posé sur la mesure qui vient de contredire la
doc.

---

## 5 — Ordre de commit

1. **`test(1727): les trois séparateurs SSE de tasks.rs sont attestés`**
   Les trois tests de `find_event_separator` dans le `mod tests` existant. **Aucune ligne de
   production.** D'abord, parce que c'est la mesure de ce que le commit 2 prétend préserver (§ D4) :
   écrits après, ils n'attesteraient que l'état d'arrivée.
2. **`refactor(1727): le consommateur SSE des tâches devient réutilisable`**
   `TaskEventStream` (`open`, `consume`), `drain_events`, `stream_task_events_into`, et
   `stream_task_events` réduite à son rôle d'appelant — plus le test de recollage de `drain_events`,
   qui est déterministe et n'a besoin d'aucune socket. `find_event_separator` **inchangée**.
3. **`test(1727): le consommateur et la sortie de mika tasks stream sont tenus contre un serveur factice`**
   `#[cfg(test)] pub(crate) mod fake_spirit` après le `mod tests`, et les deux tests bout-en-bout
   des critères 1 et 2.

Plancher de rebase : `b78f62291`. Aucun conflit attendu : le vol ne touche qu'un fichier, et ni V1,
ni V2a, ni V2b-1 n'y ont écrit.

---

## 6 — Definition of Done

- [ ] `TaskEventStream::open` prend l'URL de base et le jeton en **paramètres** ; aucune résolution
      d'environnement sous elle (§ D1).
- [ ] `consume` rend la charge `data:` brute ; `tasks.rs` ne gagne aucune dépendance à
      `TaskEventFrame` (§ D2).
- [ ] `drain_events` est une fonction libre, testée directement sur un événement coupé en deux
      (§ D3).
- [ ] `find_event_separator` est **inchangée** (`git diff` ne la montre pas) et porte trois tests.
- [ ] `mika tasks stream` écrit une ligne par charge sur stdout et **rien d'autre** ; les deux
      lignes de connexion restent sur stderr. Le test assert une **égalité** (§ D6).
- [ ] `#[cfg(test)] pub(crate) mod fake_spirit` existe dans `tasks.rs`, **après** le `mod tests`
      (§ 0.3), et suit le modèle `axum::serve` / `TcpListener::bind("127.0.0.1:0")`.
- [ ] Aucun fichier neuf. Le `reqwest::Client::new()` reste dans `tasks.rs`, et nulle part ailleurs.
- [ ] Aucune variable d'environnement touchée par un test ; aucun `serial_test` ajouté.
- [ ] `cargo test -p mika-cli` vert ; `cargo test` complet vert.
- [ ] `cargo clippy --all-targets` et `cargo fmt --check` verts.
- [ ] `bash scripts/verify-egress-manifest.sh --report` vert, **sans aucune modification de
      `docs/egress/`** (§ 0.3). S'il est rouge : arrêt et remontée, pas de correction du manifeste.
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] `git diff --numstat origin/main...HEAD` (plan exclu) **≤ 450 lignes ajoutées**.
- [ ] `crates/mika-cli/src/tui/app.rs` et `crates/mika-agent/` **non modifiés** (§ 0.4).
- [ ] Aucun fichier du plan de contrôle touché.
- [ ] PR : `Refs mika#1727` (jamais `Closes`), la ligne
      `Tracked in: senara-solutions/mika#1727` du § 0.2, et
      `Code review: delegated to mika-qa (mika#2694)`.
- [ ] Aucun ticket ouvert, pas même de suivi (gel jusqu'au 01/12).

---

## Acceptance criteria

Les quatre critères sont ceux du corps de mika#1727 § « Critères de V2b-1b », transcrits tels
quels, avec leur disposition dans ce vol.

- **AC1 — Le consommateur s'ouvre sur une URL et un jeton donnés, contre le serveur factice, et
  rend ses trames dans l'ordre. Un test rougit si une trame est perdue ou doublée, et un autre si
  un événement est coupé entre deux paquets.**
  → **Tenu.** `TaskEventStream::open(base_url, token)` (§ D1) ;
  `mika1727_v2b1b_le_consommateur_rend_ses_trames_dans_lordre` pour l'ordre, la perte et le
  doublon (égalité de séquence) ; `…_un_evenement_coupe_en_deux_paquets_est_rendu_entier_une_fois`
  pour le recollage, mesuré sur `drain_events` et non sur un cadencement (§ D3).
- **AC2 — `mika tasks stream` garde sa sortie : une ligne par charge `data:`, rien d'autre sur
  stdout. Un test le tient contre le serveur factice.**
  → **Tenu.** `stream_task_events_into` écrit dans un puits paramétré, et le test compare le flux
  **entier** à la chaîne attendue (§ D6). Les deux lignes de stderr sont conservées.
- **AC3 — `find_event_separator` est testée sur ses trois séparateurs (`\r\n\r\n`, `\n\n`,
  `\r\r`).**
  → **Tenu.** Trois tests (§ 3), au commit 1, avant tout refactor (§ D4). La fonction elle-même
  n'est pas modifiée.
- **AC4 — Egress Manifest Lint est vert sans aucune modification de `docs/egress/`. Le pilote lance
  `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le trouve rouge, il
  s'arrête et le dit.**
  → **Tenu par construction** (§ 0.3) : aucun fichier neuf, le client reste au `client_site`
  déclaré, le factice n'introduit aucun host à déclarer, et le bloc de test se place là où la coupe
  du scanner a déjà eu lieu. Le pilote lance quand même la commande, et s'arrête si elle rougit.

En plus des critères du corps, parce que ce sont les deux propriétés dont dépend V2b-2 :

- **AC5 — Le serveur SSE factice est réutilisable depuis un autre module du crate** :
  `#[cfg(test)] pub(crate) mod fake_spirit` dans `tasks.rs`, après le `mod tests` (§ D5, § 0.3).
- **AC6 — Rien n'est livré sans appelant.** Pas de filtre par agent, pas de décodage en
  `TaskEventFrame`, pas de `stream_url` exposée, pas de drapeau `keep_open` sur le factice
  (§ 0.2, § D2, § D5).

### Correspondance AC → livrable

| AC | livrable | détecteur |
|---|---|---|
| AC1 | `open` / `consume` / `drain_events` + `fake_spirit` | 2 tests (§ 3) |
| AC2 | `stream_task_events_into` + `stream_task_events` appelant | 1 test (égalité exacte) |
| AC3 | aucun changement de production | 3 tests |
| AC4 | aucun (construction) | `scripts/verify-egress-manifest.sh --report` |
| AC5 | `#[cfg(test)] pub(crate) mod fake_spirit` | ses deux appelants de ce vol |
| AC6 | — (ce qui n'est pas écrit) | relecture du diff |

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `TaskEventStream` — `open` + `consume` + doc (`crates/mika-cli/src/commands/tasks.rs`) | 55 |
| `drain_events` + doc (corps déplacé depuis `stream_task_events`) | 30 |
| `stream_task_events_into` + `stream_task_events` réécrite + doc | 19 |
| serveur SSE factice `#[cfg(test)] pub(crate) mod fake_spirit` + encodeur `sse_payload` | 40 |
| tests des critères 1 et 2 (ordre, recollage, sortie exacte) + imports | 80 |
| tests du critère 3 (`find_event_separator`, trois séparateurs + ordre d'essai) | 28 |

Total estimé : 252 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**L'unité, et pourquoi le nombre n'est pas assoupli.** 252 est une prévision de lignes **ajoutées**
au sens `git diff --numstat origin/main...HEAD`, plan exclu — l'unité que le corps impose. Ce plan
la compare **telle quelle** aux deux bornes, sans invoquer le facteur 3 que la dérivation de 279
porte déjà : 252 ≤ 279 et 252 ≤ 450. La marge sur la plus serrée est de 27 lignes, et c'est elle
qui a décidé le périmètre.

**Provenance des six postes : mesurés sur le matériau, pas estimés de zéro.** `334b80278` ajoutait
241 lignes dans son fichier neuf. En retirer ce qui ne vient pas ici — doc de module d'un fichier
qui n'existe pas (−10 net), `frame_agent_id` et son test (−35), `stream_url` et son test (−17),
`find_event_separator` déjà sur `main` (−22), le décodage de frame (−12) — et y ajouter
`drain_events` extrait (+10 nets) et `stream_task_events_into` (+9) donne les 104 lignes des trois
premiers postes. Le factice de `1f24523a7` pesait ~76 lignes avec sa route de détail et sa carte de
hits ; sans elles, 40. Les tests des critères 1 et 2 sont neufs : #3062 ne tenait le critère 2 par
aucun test.

**Pourquoi un seul vol tient.** Le vol touche **un seul fichier**, n'en crée aucun, ne change
aucune route, aucune authentification et aucune signature publique hors de `tasks.rs`. Ses trois
commits sont séquencés par la mesure (§ 5) mais chacun compile et passe seul, et le périmètre entier
du V2b-1b nommé par le corps — pris en entier depuis le matériau, soit ~307 lignes — a été ramené à
252 par les quatre retraits du § 0.2, dont chacun est un morceau qui n'aurait eu **aucun appelant**
avant V2b-2.

`Tracked in: senara-solutions/mika#1727` — le filtre par agent (`frame_agent_id`, `agent_filter`),
le décodage en `TaskEventFrame`, et la variante du factice dont le flux ne se ferme jamais partent
en V2b-2. Aucun ticket n'est ouvert (gel jusqu'au 01/12) : la suite se note sur mika#1727.
