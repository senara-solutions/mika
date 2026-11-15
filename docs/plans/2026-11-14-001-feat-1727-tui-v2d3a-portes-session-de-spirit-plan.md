# Vol V2d-3a — les portes client vers les routes de session de mika-spirit (mika#1727)

**Ticket :** senara-solutions/mika#1727 — `Refs`, jamais `Closes`. Le ticket se ferme au merge de V4.
**Branche :** `feat/1727/tui-v2d3a-portes-session-de-spirit`, neuve depuis `main`.
**Base mesurée :** `aca358da` (merge de V2d-2, #3085).

Ce vol pose le **côté client** des trois routes que V2d-1a, V2d-1b et V2d-2 ont
servies côté spirit : le transport, le vocabulaire d'erreur, l'entrée d'egress
qui déclare ce site, et **une** porte — celle qui ouvre une session. Aucun
appelant de la TUI ne change ; la bascule est V2d-4.

---

## 0. Ce qui est mesuré

Tout ce qui suit a été lu sur la branche, à `aca358da`. Les numéros de ligne
servent à retrouver le site, pas à être rejoués tels quels.

| fait | site | ce qu'il décide ici |
|---|---|---|
| `post(base_url, token, route, body) -> Result<(u16, String), RewindRemoteIssue>`, client `reqwest::Client::new()` construit **dans** la fonction, statut lu avant le corps | `commands/rewind_remote.rs:163-203` | la forme exacte du `post` de ce vol, **et** la raison du client au site (l. 170-171 : « c'est ce que déclare l'entrée d'egress ») |
| `rewind_url(base, route)` extraite « parce qu'elle a **deux** lecteurs » — `post` ne rend pas l'URL sur son chemin `Ok` | `rewind_remote.rs:152-159` | pourquoi une fonction d'URL existe ici aussi, et pourquoi le `trim_end_matches('/')` ne vit qu'une fois |
| les cinq bras : `200 \| 422 => Ok`, `401 => Denied`, `404 => NotFound`, `409 => Busy`, `_ => Unexpected { status: Some(_) }` ; erreur de transport ⇒ `Unexpected { status: None }` | `rewind_remote.rs:189-202`, `179-183` | la table de ce vol, **moins le 422** (§ 1, D3) |
| `RewindRemoteIssue` : `None` sur `status` dit « aucune réponse reçue », là où un `0` se lirait comme un statut | `rewind_remote.rs:108-109`, `144-147` | la même convention, et la même phrase de `Display` |
| le message de `Denied` nomme `MIKA_INTERNAL_TOKEN` | `rewind_remote.rs:140-143` | le critère 2 côté message, et le test qui le lit |
| `resolve` : `agent` **toujours** envoyé, statut lu **avant** la forme, corps illisible ⇒ `Unexpected` portant statut + URL + texte reçu | `rewind_remote.rs:211-266` | la forme de `ouvrir`, bras pour bras |
| `pub(crate) mod tests` sous `#[cfg(test)]`, avec `fake_spirit(&[(chemin, statut, corps)])` et `RequeteRecue { methode, chemin, autorisation, corps }` | `rewind_remote.rs:524-592` | **le banc est emprunté, pas recopié** (§ 1, D4) |
| un chemin non associé rend **501**, qu'aucun bras de `post` ne traduit — « une faute de chemin ne se lit jamais comme un `NotFound` » | `rewind_remote.rs:566-568` | pourquoi le factice atteste la **visée** du chemin, et pas seulement le statut |
| le banc est déjà emprunté par un second fichier, avec le commentaire qui le dit | `tui/commands/handlers.rs:2464-2468` | le précédent exact de l'emprunt, et sa raison écrite (« un second témoin de socket ouvrirait une seconde vérité ») |
| `POST /api/v1/sessions` ⇒ `200 {session_id: String, canonical: bool}`, 404 agent inconnu, 500 échec d'écriture. Jamais de 409 | `server/sessions.rs:48-54`, `57-118`, `232-237` | les deux champs de `CreatedSessionMirror`, et **le fait que `Busy` n'a pas de producteur sur cette route** (§ 1, D3) |
| `POST /api/v1/sessions/{id}/end` ⇒ `200 {ended, canonical}`, 404, 500 | `server/sessions.rs:121-126`, `131-192` | ce que la ligne `data` de l'egress doit nommer — identifiant de session **dans le chemin** |
| `POST /api/v1/compact` ⇒ `200 {compacted, messages}`, 404, **409**, 500 | `server/compact.rs` | le seul producteur de 409 des trois routes — donc le bras `Busy` est posé pour V2d-3b |
| entrée `cli-spirit-rewind` : `class = "internal"`, `destination_source = "config"`, `logged = false`, `owner = "@samidarko"`, `client_site = call_site = ".../rewind_remote.rs"` | `docs/egress/egress-manifest.toml:362-373` | le gabarit de forme de l'entrée neuve |
| les neuf champs exigés : `id`, `destination`, `destination_source`, `class`, `data`, `logged`, `client_site`, `call_site`, `owner` ; `class ∈ {external, internal}`, `destination_source ∈ {literal, config, skill-declared}`, `logged` booléen | `scripts/lib/egress_manifest_lint.py:568-581`, `645-651` | aucun champ inventé, aucun champ oublié |
| le lint est **bidirectionnel** : D1 code → manifeste (tout fichier de production portant un client doit être couvert), D3 manifeste → code (un `client_site` fantôme rougit) | `egress_manifest_lint.py:18-22`, `700-770` | **l'ordre des commits** (§ 1, D5) |
| le scanner coupe la production au **premier `#[cfg(test)]` suivi d'un `mod X {`** | `egress_manifest_lint.py:54-56`, `295-301` | les tests vont en **fin de fichier**, comme le dit déjà `rewind_remote.rs:515-516` |
| `mika_common::dead_endpoint::DeadEndpoint::reserve()` / `.addr()`, garde à tenir pendant tout l'appel ; réservation **et** refus assertis à la réservation | `mika-common/src/dead_endpoint.rs:83-138` | le point injoignable du critère 4 |
| `mika-common = { workspace = true, features = ["test-utils"] }` est **déjà** en `[dev-dependencies]` de ce crate, et `DeadEndpoint::reserve()` y a déjà trois appelants | `crates/mika-cli/Cargo.toml:67`, `budget_floor.rs:400`, `chat_turn.rs:340` | **aucune dépendance à ajouter** |
| `commands/mod.rs` est une liste plate de `pub mod`, ordre alphabétique, `rewind_remote` l. 23 | `commands/mod.rs:1-32` | la ligne unique à insérer, entre `setup` (l. 24) et `rewind_remote` (l. 23) |

Trois faits qui sont des **absences**, et qu'il faut dire :

- `docs/openapi/mika-spirit.yaml` ne décrit **aucune** des trois routes de
  session, comme il ne décrit aucune route rewind. Ce vol n'y touche pas ; ce
  n'est pas un oubli, c'est l'état du contrat.
- il n'existe **aucun** transport client partagé entre deux familles de routes
  dans ce crate : `tasks.rs`, `rewind_remote.rs`, `iterate.rs` ont chacun le
  leur, et chacun a son entrée d'egress. Le non-partage de § 1 D1 n'est pas une
  exception, c'est la règle du fichier voisin.
- `POST /api/v1/sessions` ne rend **jamais** 409. Le bras `Busy` de ce vol n'a
  donc aucun producteur accessible depuis `ouvrir` — d'où la forme du test du
  critère 2 (§ 1, D3).

---

## 1. La forme, et les six décisions qui la tiennent

Un module, un `post`, un enum, une porte. Ce qui mérite d'être écrit, ce sont
les endroits où la forme évidente est fausse.

### D1 — Le transport n'est **pas** mis en commun avec `rewind_remote.rs`

C'est la décision qui coûte le plus de lignes, et c'est la seule qui soit
exigée par un invariant extérieur au code. Une entrée d'egress déclare un
`client_site` : **le site où le client sortant est construit**. Le lint est
bidirectionnel (§ 0) : il vérifie que tout fichier de production portant un
constructeur de client est couvert par un `client_site`, et qu'aucun
`client_site` ne désigne un fichier qui n'en porte pas.

Un `post` partagé, extrait dans un troisième fichier, produirait l'un des deux
états suivants, tous deux faux :

- **une seule entrée** couvrant le fichier partagé : elle déclarerait alors
  « quelle donnée part » pour **deux familles de routes** à la fois — rewind et
  session — dans une ligne `data` que personne ne peut écrire juste, puisque les
  deux familles n'envoient pas les mêmes champs. C'est la ligne humainement
  relisible que mika#2408 existe pour obtenir, rendue illisible ;
- **deux entrées** pointant le même `client_site` : la direction D1 du lint est
  satisfaite, mais la propriété qu'elle achète est perdue — un lecteur du
  manifeste ne peut plus savoir laquelle des deux descriptions s'applique au
  client qu'il lit, et un changement sur l'une devient invisible dans l'autre.

Le coût du non-partage est d'environ 60 lignes dupliquées. Il est **payé
volontairement**, et un commentaire au site de `post` le dit — pas une
paraphrase du code, mais cette raison-là, qu'on ne lit nulle part dans le
fichier. Le fichier voisin porte déjà la moitié de ce commentaire
(`rewind_remote.rs:170-171`) ; ici il faut l'autre moitié : pourquoi un second
exemplaire plutôt qu'un partage.

### D2 — `post` prend un **chemin**, pas une route

`rewind_remote::post` prend `route: &str` et compose
`/api/v1/rewind/{route}` : les trois routes rewind partagent un préfixe et ne
diffèrent que par un segment terminal. **Ce n'est pas le cas ici** :
`/api/v1/sessions`, `/api/v1/sessions/{id}/end` et `/api/v1/compact` n'ont en
commun que `/api/v1`. Un argument `route` forcerait soit un `format!` à trous
par appelant — donc la composition remonterait chez les portes, et le
`trim_end_matches('/')` avec elle — soit un enum à trois variants dont deux
n'ont pas de producteur dans ce vol.

Donc : `post(base_url, token, chemin, corps)` où `chemin` est le chemin
**complet** (`"/api/v1/sessions"`), et une fonction `session_url(base_url,
chemin)` qui absorbe la barre finale. Elle existe pour la raison déjà écrite sur
`rewind_url` : `post` ne rend pas l'URL sur son chemin `Ok`, donc une porte qui
veut la nommer dans un `Unexpected` doit la recomposer — et deux `format!`
jumeaux dérivent.

Le chemin reste un `&str` et non un type : la socket mesure, pas le typage
(`rewind_remote.rs:161-162`).

### D3 — Quatre bras d'erreur, pas cinq ; et `Busy` se teste sur `post`, non sur `ouvrir`

`post` ne rend **que** 200 sur son chemin `Ok`. Le `200 | 422` de rewind existe
parce que les deux 422 de ce serveur-là n'ont pas la même forme et que seule la
porte sait laquelle attendre ; **aucune** des trois routes de session n'émet de
422. Un bras `422 => Ok` ici serait un chemin sans producteur dans le code du
serveur mesuré — donc du code non testable, sous une attente.

`SessionRemoteIssue` porte donc **quatre** variants : `NotFound { url }`,
`Busy`, `Denied`, `Unexpected { status, url, body }`. Pas de `Blocked` : rien ne
le produit.

Conséquence sur les tests, et c'est le point à ne pas rater. `Busy` **a** un
producteur dans `post` (le bras 409) mais **aucun** accessible depuis `ouvrir` :
`handle_create_session` ne rend jamais 409, c'est `/api/v1/compact` qui le rend
et sa porte est V2d-3b. Le critère 2 est donc mesuré en appelant `post`
**directement** depuis le module de test, sur un factice qui sert 401, 404 et
409 — exactement la conduite de `rewind_remote::tests::un_statut`
(l. 624-633), et pour la même raison : le bras est une propriété du transport,
pas de la porte. Faire passer le 409 par `ouvrir` mesurerait un serveur
imaginaire.

Le message de `NotFound` ne recopie pas celui de rewind. Là-bas le 404 a deux
causes (« aucun échange à rembobiner, ou agent inconnu ») et le message les
nomme toutes deux sans en choisir une. Ici, sur `POST /api/v1/sessions`, le 404
a **une** cause — `resolve_agent` a rendu `None` — mais `post` sert aussi les
deux autres routes, dont la clôture, où le 404 signifie « session inconnue ».
Le message nomme donc les deux causes du **transport** (agent inconnu, ou
session inconnue), pour le motif mika#2331 : en désigner une seule serait une
fausse provenance. Un commentaire le dit au site, parce qu'un relecteur qui
compare au fichier voisin verra un message différent et doit trouver la raison
là où il regarde.

### D4 — Le banc est **emprunté** à `rewind_remote::tests`, pas recopié

`fake_spirit` et `RequeteRecue` sont `pub(crate)` et déjà empruntés par un
second fichier, avec le commentaire qui le justifie
(`handlers.rs:2464-2468`) : un second témoin de socket ouvrirait une seconde
vérité sur « ce que le serveur a vu », dans un ticket qui existe pour retirer
des duplications. Ce vol fait le même geste, par
`use crate::commands::rewind_remote::tests::{RequeteRecue, fake_spirit};`.

Le `fallback` unique du factice est ce qui donne la mesure de **visée** : un
chemin non associé rend 501, qu'aucun bras ne traduit. Un test qui déclare le
factice sur `/api/v1/sessions` et obtient un succès a donc prouvé que la porte a
visé ce chemin-là — là où une route déclarée rendrait un 404 se lisant comme le
cas métier de `NotFound`.

Le non-partage de D1 porte sur le **client de production**, et pas sur le banc :
l'egress ne déclare rien de la face test (le scanner coupe au premier
`#[cfg(test)] mod`), et un second banc n'achèterait aucune propriété.

### D5 — L'entrée d'egress arrive dans le **même commit** que le module

Le lint est bidirectionnel. Un commit qui livre `session_remote.rs` avec son
`reqwest::Client::new()` **sans** l'entrée laisse l'arbre rouge sur la direction
D1 (`uncovered client file`) ; un commit qui livre l'entrée **avant** le fichier
le laisse rouge sur D3 (`phantom client_site`). Les deux sont des états que le
§ 3 interdit : chaque commit est vert.

Donc le commit 1 porte les trois fichiers, et l'ordre de travail à l'intérieur
du commit n'a pas d'importance. C'est le seul endroit où ce vol ne peut pas
découper plus fin, et c'est une contrainte du lint, pas un choix.

### D6 — Les `expect(dead_code)` se **mesurent**, ils ne se devinent pas

Aucun item de ce module n'a d'appelant en production à la fin du vol : la
bascule est V2d-4. Les candidats sont donc `ouvrir`, `CreatedSessionMirror`,
`post`, `session_url`, `SessionRemoteIssue` et ses variants — c'est-à-dire tout
le module.

**Le piège, nommé par le plan de V2c-1a-1 et re-nommé ici.** `rustc` ne signale
`dead_code` que sur la **racine** d'un sous-graphe inatteignable : `ouvrir` est
la racine évidente, et ce qu'elle seule atteint peut être signalé avec elle ou
ne pas l'être du tout. Poser l'attribut sur un item que le lint ne vise pas le
transforme en `unfulfilled_lint_expectation`, un warning. L'attribut rougit donc
dans les deux sens, et c'est exactement sa valeur.

**Conduite, en deux temps.** Le pilote pose
`#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]`
à la naissance de chaque item, dans le commit qui le livre — un commit qui
laisse un `dead_code` nu derrière lui n'est pas vert. Puis, au dernier commit, il
lance `cargo clippy -p mika-cli --all-targets` et **retire** chaque attribut
insatisfait. `--all-targets` compile les deux faces du `cfg_attr` : la cible de
test, où l'attribut n'existe pas et l'item est vivant, et la cible normale, où
il est mort. Il ne substitue **jamais** `allow` à `expect` pour faire taire le
warning : `allow` est muet au câblage, et c'est l'obligation faite à V2d-4 de
retirer l'attribut qui justifie son existence. Le texte de `reason` est fixé par
le corps du ticket et ne varie pas.

---

## 2. Les livrables, un par un

### 2.1 `crates/mika-cli/src/commands/session_remote.rs` — le module neuf

**En-tête de module.** Ce que le module sert — les trois routes de session de
mika-spirit, par ticket et par vol — et les deux pourquoi qu'on ne lit pas dans
le code : D1 (pourquoi ce transport n'est pas celui de `rewind_remote.rs`) et
l'état du vol (une seule porte posée, `clore` et `compacter` en V2d-3b, aucun
appelant avant V2d-4).

**Le miroir.** Un seul, et il porte les deux champs que `CreateSessionResponse`
sérialise :

```rust
#[derive(Debug, Deserialize)]
pub(crate) struct CreatedSessionMirror {
    pub(crate) session_id: String,
    /// Vrai quand l'identifiant rendu est la session canonique de l'agent
    /// (mika#1401) : le client ne la clôt **jamais**. C'est V2d-4 qui lira ce
    /// champ pour décider s'il appelle `clore` en fin de session.
    pub(crate) canonical: bool,
}
```

`Deserialize` seul : `CreateSessionResponse` ne dérive que `Serialize` et vit
dans `crates/mika-agent/`, hors périmètre. Ce miroir **est** le contrat de fil
côté client, sans règle de domaine — la même phrase que `rewind_remote.rs:19-21`,
et la même raison.

**Le vocabulaire d'erreur.** Quatre variants (D3), `Display` à la main, chaque
message nommant l'URL ou la cause :

| variant | message |
|---|---|
| `NotFound { url }` | nomme l'URL **et les deux** causes du transport : agent inconnu, ou session inconnue (D3) |
| `Busy` | « l'agent est occupé : un autre tour tient son verrou » — le producteur est `/api/v1/compact`, câblé en V2d-3b |
| `Denied` | nomme `MIKA_INTERNAL_TOKEN`, comme `rewind_remote.rs:140-143`. Ne porte aucune donnée : le nom de la variable est la moitié utile du message |
| `Unexpected { status: Option<u16>, url, body }` | `Some(s)` ⇒ « mika-spirit a rendu {s} pour {url} : {body} » ; `None` ⇒ « mika-spirit est injoignable à {url} : {body} ». Un commentaire dit que `None` veut dire « aucune réponse reçue », là où un `0` se lirait comme un statut HTTP |

Pas de `Box` sur un variant, et donc pas de `result_large_err` à peser : aucun
variant ne porte de type lourd (le `Box<RewindPreview>` de rewind était la
cause de cette pesée).

**L'URL et le POST.**

```rust
fn session_url(base_url: &str, chemin: &str) -> String {
    format!("{}{chemin}", base_url.trim_end_matches('/'))
}

async fn post(
    base_url: &str,
    token: &str,
    chemin: &str,
    corps: &serde_json::Value,
) -> Result<(u16, String), SessionRemoteIssue> { … }
```

Corps de `post`, bras pour bras : client `reqwest::Client::new()` construit
**dans** la fonction (D1, commenté au site) ; `.header("authorization",
format!("Bearer {token}"))` ; `.json(corps)` ; `.timeout(Duration::from_secs(5))`
— la même borne que le fichier voisin, et le fichier n'est pas dans le
`PERIMETER` de `scripts/check-a2a-timeout-literals.sh` (ce n'est pas un chemin
a2a) ; `send().await.map_err(…)` ⇒ `Unexpected { status: None, url, body:
cause.to_string() }`. Puis **statut lu avant le corps**, avec le commentaire qui
dit pourquoi (un corps d'erreur désérialisé comme un succès est la panne à ne
pas produire), et la table : `200 => Ok((statut, corps))`, `401 => Denied`,
`404 => NotFound { url }`, `409 => Busy`, `_ => Unexpected { status:
Some(statut), url, body: corps }`.

**La porte.**

```rust
pub(crate) async fn ouvrir(
    base_url: &str,
    token: &str,
    agent: &str,
) -> Result<CreatedSessionMirror, SessionRemoteIssue> { … }
```

Trois gestes, et un commentaire sur chacun des deux premiers :

1. `agent` part **toujours**, y compris vide. Le serveur déclare le champ
   `#[serde(default)]` (`server/sessions.rs:41-47`) : un champ absent lui fait
   résoudre *son* agent par défaut, qui n'est pas celui de la TUI. La session
   ouverte serait celle d'un autre agent, et c'est **la session dans laquelle
   V2d-4 fera tourner tous les tours suivants** — l'erreur ne serait pas
   rattrapée plus loin, elle serait commise une fois pour toute la durée de la
   fenêtre. Le champ présent et vide dit au serveur la même chose, mais il le
   dit, et le test du critère 1 le lit sur la socket.
2. `let (statut, corps) = post(base_url, token, "/api/v1/sessions", &envoye).await?;`
3. Statut lu **avant** la forme. La condition est `!= 200` et non `== 422` :
   rien ne garantit que le chemin `Ok` de `post` restera à un seul statut, et
   la même ligne de `resolve` porte déjà ce raisonnement
   (`rewind_remote.rs:251-252`). Puis `serde_json::from_str(&corps)`, dont
   l'`Err` est `Unexpected { status: Some(statut), url: session_url(…), body:
   format!("unreadable created session ({cause}): {corps}") }`.

Un 200 au corps illisible est donc `Unexpected`, **jamais** un succès : le taire
rendrait un `session_id` vide en position de succès, c'est-à-dire exactement le
client que `caller_session_is_adoptable` refuse d'adopter, et à qui
`a2a_create_task` fabriquerait une session neuve à chaque tour. Le commentaire
cite ce chemin, parce que c'est le pourquoi, et qu'il est dans un autre crate.

**Les tests en fin de fichier**, sous `#[cfg(test)] mod tests` — pas par goût :
le scanner d'egress coupe la production au premier `#[cfg(test)]` suivi d'un
`mod X {`, et un item de production placé après en sortirait (la phrase est déjà
écrite en `rewind_remote.rs:515-516`). Le module n'a pas besoin d'être
`pub(crate)` : rien ne l'emprunte dans ce vol.

### 2.2 `crates/mika-cli/src/commands/mod.rs` — une ligne

`pub mod session_remote;` entre `pub mod rewind_remote;` (l. 23) et
`pub mod setup;` (l. 24). L'ordre alphabétique de la liste est tenu.

### 2.3 `docs/egress/egress-manifest.toml` — une entrée

Placée **après** `cli-spirit-rewind` (l. 362-373), sur sa forme, avec les neuf
champs exigés et rien d'autre dans ce fichier :

```toml
[[sink]]
id                 = "cli-spirit-sessions"
destination        = "MIKA_SPIRIT_URL (défaut http://localhost:8080)"
destination_source = "config"
class              = "internal"
data               = "un POST générique vers les trois routes de session de mika-spirit (POST /api/v1/sessions, POST /api/v1/sessions/{id}/end, POST /api/v1/compact) : le corps JSON que l'appelant fournit — le nom de l'agent servi, et pour la clôture l'identifiant de session, qui voyage dans le chemin — et le jeton interne en en-tête Authorization"
logged             = false
client_site        = "crates/mika-cli/src/commands/session_remote.rs"
call_site          = "crates/mika-cli/src/commands/session_remote.rs"
owner              = "@samidarko"
```

La ligne `data` décrit **les trois** routes, et non la seule que ce vol câble :
le `client_site` est le site du client, et c'est ce client-là qui servira les
trois. Une ligne qui n'en nommerait qu'une devrait être réécrite à V2d-3b, et le
lint ne le demanderait pas — donc la description dériverait en silence du code
qu'elle décrit, qui est précisément ce que mika#2408 existe pour empêcher.

`destination_source = "config"` et non `"literal"` : l'URL de base arrive **par
argument** chez les portes, résolue par l'appelant depuis `MIKA_SPIRIT_URL` —
même forme que les deux entrées voisines.

---

## 3. Ordre des commits

Trois commits. Chacun porte son code **et** ses tests ; aucun test n'est
reporté à un commit suivant, ni à un vol suivant.

**Commit 1 — le transport, son vocabulaire, et l'entrée d'egress.**
`session_remote.rs` (en-tête, `SessionRemoteIssue` + `Display`, `session_url`,
`post`), la ligne de `commands/mod.rs`, l'entrée d'egress (D5 : les trois
fichiers ensemble, sinon le lint est rouge dans un sens ou dans l'autre). Tests :
les trois statuts du critère 2, le 500 du critère 3. Couvre les critères 2, 3 et
6.

**Commit 2 — la porte `ouvrir`.** `CreatedSessionMirror`, `ouvrir`. Tests : la
socket du critère 1 (méthode, chemin, `Bearer`, corps portant `agent`) et la
lecture de `session_id` / `canonical`. Couvre le critère 1.

**Commit 3 — les deux absences de réponse.** Aucun code de production neuf ;
deux tests : le point injoignable du critère 4 (`DeadEndpoint::reserve()`) et le
200 au corps illisible du critère 5. Ils arrivent après `ouvrir` parce qu'ils la
traversent tous les deux.

**Après chaque commit** : `git diff --numstat origin/main...HEAD`, plan exclu.
Si le total dépasse **540**, halte — pas de commit suivant, pas de PR, et le
dernier message dit « HALTE borne : N lignes pour 540 » avec le compte par
commit.

**Avant d'ouvrir la PR**, dans cet ordre :

1. `cargo test -p mika-cli`
2. `cargo clippy -p mika-cli --all-targets` — vert, **sans `allow`**. C'est ici
   que se fait le retrait des `expect` insatisfaits (D6).
3. `bash scripts/verify-egress-manifest.sh --report` — critère 6. **Rouge ⇒
   halte et on le dit**, sans retoucher le manifeste à l'aveugle.
4. `bash scripts/verify-pipeline.sh origin/main`

---

## 4. Le jeu de tests

Sept tests, un helper. Le banc est celui de `rewind_remote` (D4).

```rust
/// Un factice qui ne sert qu'un couple sur `chemin`, et l'appel que `post` y
/// fait. L'URL de base revient avec, parce que le critère 3 l'attend dans le
/// message.
async fn un_statut(
    chemin: &str,
    statut: StatusCode,
    corps: &str,
) -> (String, Result<(u16, String), SessionRemoteIssue>) { … }
```

| test | critère | ce qu'il assert |
|---|---|---|
| `mika1727_ouvrir_vise_la_route_des_sessions_avec_son_jeton` | 1 | sur la socket : `methode == "POST"`, `chemin == "/api/v1/sessions"`, `autorisation == Some("Bearer jeton-secret")`, et le corps désérialisé porte `agent == "mika-dev"`. Puis, sur la valeur rendue, `session_id == "s-neuve"` et `canonical == false`. **Les trois faits se lisent sur la socket**, pas sur la valeur passée à `post` : c'est la seule lecture qui mesure ce que le serveur recevra |
| `mika1727_un_401_nomme_la_variable_a_poser` | 2 | `Denied`, et `to_string()` contient `MIKA_INTERNAL_TOKEN`. La variable se lit dans le `Display`, non dans le code : c'est le message que l'opérateur verra |
| `mika1727_un_404_nomme_ses_deux_causes` | 2 | `NotFound`, et le message nomme l'agent inconnu **et** la session inconnue (D3) |
| `mika1727_un_409_est_un_verrou` | 2 | `Busy`. Appelé sur `post` **directement**, par `un_statut`, parce que `POST /api/v1/sessions` n'émet jamais 409 (D3) — le producteur réel est `/api/v1/compact`, câblé en V2d-3b |
| `mika1727_un_statut_hors_contrat_porte_son_statut_et_son_url` | 3 | 500 ⇒ `Unexpected { status: Some(500), .. }`, et le message contient `500` **et** `{base}/api/v1/sessions` |
| `mika1727_un_point_injoignable_na_pas_de_statut` | 4 | `DeadEndpoint::reserve()`, garde tenue pendant tout l'appel ; `ouvrir(&format!("http://{}", dead.addr()), …)` ⇒ `Unexpected { status: None, .. }`, et le message contient l'URL. `status: None` est l'assertion : un `Some(0)` ou un `Some(500)` ici serait un statut inventé |
| `mika1727_un_200_illisible_nest_jamais_un_succes` | 5 | le factice sert 200 avec `{"session_id":"s-1"}` (champ `canonical` manquant) ⇒ `expect_err`, forme `Unexpected { status: Some(200), .. }`, et le message cite le corps reçu. **`expect_err` est la mesure** : un repli sur `canonical: false` rendrait un `Ok` et rougirait ici |

Le corps illisible du dernier test est un **champ manquant**, et non un JSON
syntaxiquement cassé : c'est la forme qu'un écart de version produit réellement
(le serveur retire ou renomme `canonical`), et c'est aussi la seule qui
distingue « le client refuse » de « le client se replie sur un défaut ». Un
`"pas du json"` passerait aussi, mais mesurerait `serde_json` plutôt que la
porte.

Le point injoignable passe par `DeadEndpoint::reserve()` et **jamais** par un
`bind` suivi d'un `drop` (mika#2569) : un port relâché est remis au prochain
`bind(:0)` d'un test parallèle, qui sert alors l'appel censé échouer. Une adresse
non routable n'est pas une alternative non plus — le proxy d'egress du bac à
sable l'intercepte et rend un statut HTTP, donc le test serait vert en CI et
rouge en vol.

---

## 5. Definition of Done

- `commands/session_remote.rs` existe, porte `post`, `session_url`,
  `SessionRemoteIssue` + `Display`, `CreatedSessionMirror` et `ouvrir`.
- `ouvrir` POSTe sur `{base}/api/v1/sessions` avec `Authorization: Bearer <jeton>`
  et un corps portant `agent`, et rend `{session_id, canonical}`.
- `post` rend `Denied` sur 401, `NotFound` sur 404, `Busy` sur 409, et
  `Unexpected { status: Some(_) }` sur tout autre statut ; une erreur de
  transport donne `Unexpected { status: None }`.
- Un 200 au corps illisible est `Unexpected`, jamais un succès.
- Le transport n'est pas partagé avec `rewind_remote.rs`, et un commentaire au
  site dit pourquoi (D1).
- L'entrée `cli-spirit-sessions` existe, après `cli-spirit-rewind`, neuf champs,
  `client_site = call_site = ".../session_remote.rs"`, ligne `data` nommant les
  trois routes.
- `cargo test -p mika-cli` et `cargo clippy -p mika-cli --all-targets` verts,
  **sans `allow`**. Les seuls `expect(dead_code)` portent
  `reason = "câblé par V2d-4, mika#1727"`, et aucun n'est insatisfait.
- `bash scripts/verify-egress-manifest.sh --report` vert ; rouge ⇒ halte
  déclarée. `bash scripts/verify-pipeline.sh origin/main` vert.
- Diff limité à `crates/mika-cli/src/commands/session_remote.rs`,
  `crates/mika-cli/src/commands/mod.rs`, `docs/egress/egress-manifest.toml`
  (+ `docs/plans/` pour ce fichier). **Intacts** : `commands/rewind_remote.rs`,
  `commands/chat.rs`, `commands/chat_turn.rs`, `tui/`, `crates/mika-agent/`,
  `crates/mika-a2a/`, `docs/openapi/`, `docs/egress/README.md`,
  `.github/workflows/`, `.claude/`, `skills/bundled/`, `well_known_agents.rs`,
  `.mika/`.
- Total ajouté ≤ **540** lignes (`git diff --numstat origin/main...HEAD`, plan
  exclu), mesuré après chaque commit.
- PR : `Refs mika#1727`. Aucun `Closes` / `Fixes` / `Resolves`, corps **et**
  messages de commit. `gh pr view <N> --json closingIssuesReferences` rend une
  liste vide — vérifié même, et surtout, si la PR est ouverte par le gabarit de
  reprise (`wip-rescue`), qui écrit `Closes #N` par défaut et a dû être corrigé à
  la main sur #3077 et #3081.
- La PR touche `docs/egress/egress-manifest.toml`, dont CODEOWNERS est
  `@samidarko` : la revue est attendue et n'est pas un blocage du vol.
- Aucun ticket ouvert (gel jusqu'au 01/12). Reste et manques ⇒ commentaire sur
  #1727 ou corps de PR, avec `Tracked in: senara-solutions/mika#1727`.

## 6. Acceptance criteria

Transcrits du corps de #1727, § « Critères de V2d-3a ».

1. `ouvrir` envoie un POST sur `{base}/api/v1/sessions`, avec
   `Authorization: Bearer <jeton>` et un corps qui porte `agent`. Un test lit ces
   trois faits sur la socket du banc `fake_spirit`, et lit `session_id` et
   `canonical` dans la réponse.
2. Sur 401, 404 et 409, `post` rend respectivement `Denied`, `NotFound` et
   `Busy`. Un test par statut.
3. Sur un autre statut (500 par exemple), `post` rend `Unexpected` avec ce
   statut. Un test le montre.
4. Sur un point injoignable (`DeadEndpoint::reserve()`), `ouvrir` rend
   `Unexpected` sans statut. Un test le montre.
5. Sur un 200 au corps illisible, `ouvrir` rend `Unexpected`, jamais un succès.
   Un test le montre.
6. L'entrée `cli-spirit-sessions` existe, avec `client_site` et `call_site` sur
   `session_remote.rs`, et une ligne `data` qui nomme les trois routes.
   `bash scripts/verify-egress-manifest.sh --report` est vert. S'il est rouge, le
   pilote s'arrête et le dit.
7. `cargo clippy -p mika-cli --all-targets` est vert, sans `allow`. Les seuls
   `expect(dead_code)` portent `reason = "câblé par V2d-4, mika#1727"`. Seuls
   `session_remote.rs`, `commands/mod.rs` et `egress-manifest.toml` changent.
   `rewind_remote.rs`, `crates/mika-agent/`, `crates/mika-a2a/`, `tui/` et
   `docs/openapi/` sont intacts.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si elle est ouverte par le gabarit de reprise.

---

## Fire-Disposition

**Le détecteur livré par ce vol est l'`expect(dead_code)` de D6**, et sa
disposition est l'**option (a) — exception nommée en allowlist**, dans sa forme
la plus étroite : l'attribut lui-même.

Les quatre propriétés exigées par mika#1574 :

- **chaque violation reçoit son entrée** — un `#[cfg_attr(not(test),
  expect(dead_code, …))]` par item sans appelant, posé à la naissance de l'item,
  jamais un `#![allow(dead_code)]` de module ;
- **l'entrée nomme la donnée précise** — l'item qu'elle couvre, puisqu'elle est
  posée dessus et sur rien d'autre ;
- **elle référence un ticket de suivi** — `reason = "câblé par V2d-4, mika#1727"`,
  grep-visible par
  `rg 'V2d-4' crates/mika-cli/src/commands/session_remote.rs` ;
- **elle rougit quand elle devient stale** — c'est le choix d'`expect` plutôt
  qu'`allow` : au câblage par V2d-4, l'attribut devient
  `unfulfilled_lint_expectation` et le build le dit. D6 interdit explicitement
  de retomber sur `allow`, qui serait muet au câblage.

**Le risque propre à ce vol, nommé.** L'attribut est posé sur plus d'items que
le lint n'en vise peut-être, parce que `rustc` ne signale que la racine d'un
sous-graphe inatteignable, et chaque attribut de trop est un warning. La conduite
de D6 — poser, mesurer au dernier commit par
`cargo clippy -p mika-cli --all-targets`, retirer l'insatisfait — est donc une
étape de travail à part entière et non une vérification de fin de course.

**Les sept tests du § 4 ne sont pas des détecteurs**, et c'est une évaluation,
pas un oubli : leur chemin de succès est « la porte a rendu `Denied` / un 200
lisible / `Unexpected` sans statut », pas « aucune violation trouvée » sur une
population existante. Ils n'ont donc rien à amnistier. Ce vol ne livre par
ailleurs ni lint, ni garde CI, ni scan structurel, ni garde EndTurn, ni
validateur de schéma — la garde structurelle de la série est V2d-4.

---

## 7. Ce que ce vol ne fait pas, et la dette nommée

**Hors périmètre, par le corps du ticket :**

- Les portes `clore` et `compacter` — V2d-3b, sur le transport de ce vol, sans
  toucher `docs/egress/` (l'entrée de ce vol les déclare déjà).
- La bascule des appelants de la TUI et la garde structurelle — V2d-4.
  **Aucun appelant ne change ici** : `chat.rs`, `chat_turn.rs` et `tui/` ne sont
  pas touchés, et les portes de ce vol n'ont donc aucun consommateur à la fin.
  C'est voulu, et c'est ce que les `expect(dead_code)` disent à voix haute.
- `/compact`, `/rewind`, `/undo` côté TUI, le `TaskEngine`, le provider, les
  skills, le MCP et les équipes — V3 et V4.

`Tracked in: senara-solutions/mika#1727`

**Dette créée par ce vol, et non traitée :**

- **Le transport est dupliqué d'avec `rewind_remote.rs`**, d'environ 60 lignes,
  et la duplication est volontaire (D1). Elle ne se résorbe pas en extrayant un
  module commun : c'est la granularité de l'entrée d'egress qui l'exige. Si
  mika#2408 devait un jour admettre un `client_site` par *famille de routes*
  plutôt que par fichier, la fusion deviendrait possible — et ce serait un
  ticket sur le lint, pas sur ce crate.
- **Le bras `Busy` n'a aucun producteur accessible depuis `ouvrir`**, et son
  test passe par `post`. Il en aura un à V2d-3b, avec la porte `compacter`. Le
  bras n'est pas spéculatif pour autant : `/api/v1/compact` rend bien 409
  aujourd'hui (`server/compact.rs`), et un `post` sans ce bras traduirait ce 409
  en `Unexpected` — un verrou affiché comme un écart de version.
- **`docs/openapi/mika-spirit.yaml` ne décrit aucune des trois routes de
  session**, comme il ne décrit aucune route rewind. L'écart est antérieur à ce
  vol, et son comblement toucherait un fichier exclu (toute modification serait
  copiée dans `crates/mika-agent/docs/openapi/`).
- **Aucune garde structurelle n'empêche encore la TUI d'ouvrir une session en
  local.** `chat.rs:69-115` continue d'écrire sa session dans la base du
  processus de la TUI, et rien ne le refuse. C'est l'objet de V2d-4, et le poser
  ici reviendrait à livrer une garde que le code viole encore — option (b) d'une
  Fire-Disposition dont ce vol n'a pas besoin.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `commands/session_remote.rs` — production (en-tête, miroir, enum + `Display`, `session_url`, `post`, `ouvrir`) | 184 |
| `commands/session_remote.rs` — tests (helper + 7 tests, banc emprunté) | 142 |
| `commands/mod.rs` (`pub mod session_remote;`) | 1 |

Total estimé : 327 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités, et laquelle mord.** La borne par vol (429) porte sur les
lignes **hors `docs/`** : 327, soit une marge de 102 lignes (~24 %). La borne du
ticket (540) porte sur `git diff --numstat origin/main...HEAD` plan exclu, donc
`docs/egress/egress-manifest.toml` inclus : 327 + 11 = **338 pour 540**, soit une
marge de 202 lignes. La plus serrée des deux est celle du vol, et c'est elle qui
est tenue. La borne par PR (1000) ne mord pas.

**Pourquoi un seul vol tient.** Trois raisons mesurées en § 0, et non
optimistes. *Aucun banc n'est écrit* — `fake_spirit` et `RequeteRecue` sont
`pub(crate)` et déjà empruntés par un second fichier (D4), donc les 60 lignes de
factice que ce module aurait pu coûter ne sont pas dépensées. *Aucune dépendance
n'est ajoutée* — `mika-common` avec `test-utils` est déjà en
`[dev-dependencies]` et `DeadEndpoint::reserve()` y a trois appelants.
*Aucune conversion de domaine n'est écrite* — `CreatedSessionMirror` repart tel
quel vers son appelant (V2d-4), là où `rewind_remote` a dû payer
`preview_to_domain` et sa table de graphies, soit ~120 lignes à lui seul.

Le poste le plus lourd est la production (184), et c'est le prix de D1 : le
transport est écrit une seconde fois, délibérément. Les commentaires qui portent
le pourquoi comptent dans la borne et **ne se rabotent pas pour la tenir** — si
le code écrit dépasse 540, le pilote s'arrête selon la halte du § 3 et le dit,
il ne retire pas après coup les commentaires qui portent un pourquoi.

Pas de découpage en vols supplémentaires : la suite est déjà découpée par le
corps du ticket en V2d-3b, V2d-4, V3 et V4.
