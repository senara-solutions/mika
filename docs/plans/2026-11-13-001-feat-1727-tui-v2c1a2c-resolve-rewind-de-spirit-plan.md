# Vol V2c-1a-2c — la porte `resolve` de mika-spirit (mika#1727)

> **Branche :** `feat/1727/tui-v2c1a2c-resolve-rewind-de-spirit`
> **Base :** `7f6bea00` (`main` au merge de V2c-1a-2b, mika#3078)
> **PR :** `Refs mika#1727`, jamais `Closes` — le ticket se ferme au merge de V4.

Ce vol pose la **dernière des trois portes** sur le transport de V2c-1a-1 :
`resolve` demande à mika-spirit l'ancrage d'un rembobinage de `count` échanges et
rend un `ResolveMirror`, ou une issue lisible. Aucun appelant ne change. La porte
`execute`, livrée par V2c-1a-2b, est le modèle — et là où le plan matériau et le
code de `main` diffèrent, c'est `main` qui fait foi.

---

## 0. Ce qui est mesuré

Les faits F1 à F11 du plan de V2c-1a (`6a5d20a3`) sont repris **sans
re-mesure**, conformément au ticket. Les trois qui portent ce vol sont redits ici
parce qu'une décision du § 2 s'appuie sur chacun. Les quatre mesures neuves —
celles que V2c-1a-2b a changées, ou que le plan matériau n'avait pas — sont
marquées **(neuf)** et ont été lues sur `7f6bea00`.

Les onze citations de ligne de cette section ont été **relues sur `7f6bea00`** :
`main` n'a pas bougé depuis la rédaction, et aucune n'a dérivé.

**F6 — le 404 de `resolve` est ambigu par construction.**
`handle_rewind_resolve` rend 404 dans **deux** cas sans les distinguer : l'agent
est inconnu (`server/rewind.rs:122-131`) ou aucun échange n'a été trouvé dans la
session (`:154-158`, « No exchanges found in session »). Le client ne peut donc
pas transformer ce 404 en une cause unique, et ne doit pas essayer :
`RewindRemoteIssue::NotFound { url }` nomme la route, ce qui est exactement
l'information dont il dispose. Ce fait ne demande **aucun** code à ce vol ; il
dit pourquoi il n'y a rien à ajouter, et il va au § 6 comme dette nommée.

**F8 — un 400 est atteignable, donc un bras par défaut est nécessaire.** Sur
`resolve`, le 400 vient d'un `session_id` vide (`server/rewind.rs:107-113`) ou
d'un `count == 0` (`:114-120`). Le client ne peut donc pas énumérer les statuts
de façon fermée — exigence déjà **satisfaite** par le bras `_ => Unexpected` de
`post` (`rewind_remote.rs:166-206`). Ce vol n'a rien à y ajouter, et c'est ce
fait qui le dit : **pas de garde client sur `count == 0`**, le 400 du serveur
suffit et porte sa raison dans le corps que `Unexpected` transmet.

**F11 — le lint d'egress scanne la construction de client.** Elle vit dans
`post`, en production, dans ce fichier, et l'entrée `cli-spirit-rewind`
(`docs/egress/egress-manifest.toml:363-371`) la couvre déjà — son champ `data`
nomme explicitement les trois routes **et** le « nombre d'échanges » que ce vol
envoie. Ce vol n'ajoute **aucune** construction de client : `docs/egress/` ne
change pas, et le critère 6 tient sans élargir quoi que ce soit.

**F20 (neuf) — `handle_rewind_resolve` ne rend jamais 422.** Les statuts qu'il
émet sont 400 (`server/rewind.rs:107-120`), 404 (`:122-131` et `:154-158`), 409
(`:134-143`), 500 (`:159-163`) et 200 (`:146-153`). Il n'y a **aucun** chemin
vers 422 : contrairement à `preview` (`:235-243`) et à `execute` (`:305-312`),
`resolve` ne décide pas qu'un rembobinage est refusé — il cherche un ancrage.
C'est ce fait qui fonde tout le § 2.1 : un 422 reçu sur cette route est un écart
de version, pas un refus, et le lire comme un refus inventerait une décision que
le serveur n'a pas prise.

**F21 (neuf) — `post` rend pourtant 422 sur son chemin `Ok`.**
`rewind_remote.rs:166-206` : le bras `Ok` couvre 200 **et** 422, parce que ses
deux autres appelants en ont besoin. La porte `resolve` reçoit donc un statut
qu'elle ne produit pas, et le flux du § 2.1 est écrit **pour ce couple** — F20
dit que le cas est anormal, F21 dit qu'il est atteignable depuis ce code. Les
deux ensemble font du test T3 un test de contrat et non une curiosité.

**F22 (neuf) — le précédent de l'attribut est sur ce fichier, un vol plus tôt.**
`ResultMirror` portait `expect(dead_code)` de type mort avant V2c-1a-2b ; depuis
que `execute` le rend, `rewind_remote.rs:65-74` porte un commentaire qui dit la
mesure : « multiple fields are never read », et non « struct is never
constructed », d'où la `reason` actuelle `"champs lus par V2c-1b, mika#1727"`
(`:72`). `ResolveMirror` est dans la position exacte où `ResultMirror` était :
racine morte, `expect(dead_code, reason = "câblé par V2c-1a-2c, mika#1727")`
(`:23`), trois champs qu'aucun site de production ne lit. Le § 2.2 en tire son
attente — **et son obligation de la mesurer plutôt que de la supposer**, parce
qu'un précédent sur un type voisin n'est pas une mesure sur celui-ci.

**F23 (neuf) — le factice et son témoin sont déjà agnostiques des trois routes.**
`fake_spirit` (`rewind_remote.rs:509-…`) monte un `Router::new().fallback(servir)`
(`:483`) et non trois routes déclarées ; `TROIS_ROUTES` (`:529`) nomme déjà
`resolve` et une garde de `main` mesure que les trois chemins sont servis. Un
chemin non associé rend 501, un statut qu'aucun bras de `post` ne traduit, donc
une faute de frappe de chemin ne se lit jamais comme un `NotFound` métier. Ce vol
n'écrit donc **aucune** plomberie de test : il écrit le helper frère de
`par_la_porte_execute` (`:1035`) et ses quatre tests.

---

## 1. La forme : une porte, et rien d'autre

Un seul livrable de production, et un seul commit (§ 3) : **la porte `resolve`**,
avec son attribut, l'ajustement de celui de `ResolveMirror`, et l'en-tête de
module qui dit l'état réel.

**Ce qui ne vit pas ici, et ne changera pas.** Le module ne lit ni
`MIKA_SPIRIT_URL` ni `MIKA_INTERNAL_TOKEN` : l'URL de base et le jeton arrivent
par argument, pour la raison déjà écrite sur `fetch_task_detail` et redite par
l'en-tête (`rewind_remote.rs:3-6`) — un client qui résout ses propres accès ne se
pilote pas depuis un test. C'est V2c-1b qui les résoudra, au site d'appel, et qui
exigera le jeton.

**Pourquoi la porte rend `ResolveMirror` et non un type de domaine.** Parce qu'il
n'y en a pas : les trois champs (`session_id`, `after_message_id`, `trace_ids`)
sont ce que l'appelant de V2c-1b passera tels quels à `preview` puis à `execute`.
Une conversion vers un type CLI serait une recopie de trois champs identiques,
sans lecteur avant V2c-1b. C'est la même décision que V2c-1a-2b a prise pour
`ResultMirror` (son § 1), et pour une raison plus forte encore : ici il n'existe
même pas de formateur d'affichage à préserver, l'ancrage n'étant jamais montré à
l'utilisateur.

**Aucun miroir neuf.** `ResolveMirror` existe déjà depuis V2c-1a-1
(`rewind_remote.rs:21-30`) et sa forme correspond champ pour champ à
`RewindResolveResponse` (`server/rewind.rs:96-101`). Ce vol ne le réécrit pas ; il
le rend atteignable.

---

## 2. Les livrables, un par un

### 2.1 La porte `resolve` (§ 2.5 du matériau)

```rust
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1b, mika#1727"))]
pub(crate) async fn resolve(
    base_url: &str,
    token: &str,
    agent: &str,
    session_id: &str,
    count: usize,
) -> Result<ResolveMirror, RewindRemoteIssue>
```

`count: usize` et non `u32` : c'est le type du champ côté serveur
(`server/rewind.rs:85-86`), et le convertir ici ferait porter au transport une
décision de domaine que personne ne lui a demandée. L'ordre des arguments suit
celui de `preview` et d'`execute` — `base_url`, `token`, `agent`, puis les
paramètres propres à la route — parce que trois portes du même module dont les
signatures divergent s'appellent de travers une fois, et une seule suffit.

**Le corps envoyé.**

```rust
let envoye = serde_json::json!({
    "session_id": session_id,
    "count": count,
    "agent": agent,
});
```

Un seul commentaire à écrire, et rien de plus :

- **`agent` part toujours, y compris vide** — le pourquoi est celui déjà écrit
  sur `preview` (`rewind_remote.rs:310-…`), et le commentaire y **renvoie** sans
  le recopier : deux copies du même pourquoi dérivent, et la copie qui dérive est
  celle qu'on ne relit pas. Sur `resolve` la conséquence d'une omission est plus
  douce que sur `execute` — un ancrage lu sur le fil d'un autre agent, pas un
  rembobinage appliqué dessus — mais c'est **le même ancrage** qui sera ensuite
  passé à `execute` par V2c-1b. L'erreur ne serait pas rattrapée plus loin ; elle
  serait seulement commise plus tôt.

**Pas de `mut`, pas d'insertion conditionnelle.** Les trois clés partent toujours.
`resolve` n'a pas d'équivalent de l'`originating_session_id` d'`execute` : le
serveur ne déclare que trois champs, et `count` a un défaut côté serveur
(`default_count`, `server/rewind.rs:92-94`) que ce client ne doit jamais laisser
jouer — omettre la clé ferait rembobiner **un** échange là où l'appelant en
demandait N, en silence et avec un 200.

**Pas de garde client sur `count == 0`.** F8 : le serveur rend 400 avec sa raison,
`post` en fait un `Unexpected` qui porte statut, URL et corps. Une garde ici
dupliquerait la validation du serveur — deux vérités pour une règle — et masquerait
le jour où la règle change côté serveur. Un commentaire le dit, parce que l'absence
d'une garde ne se lit pas dans le code.

Le flux, et ce que chaque branche rend :

| cas | rendu |
|---|---|
| `post` échoue | remonte tel quel (`?`) — `Denied`, `NotFound` (F6), `Busy`, `Unexpected` (F8) |
| statut ≠ 200 sur le chemin `Ok` — c'est-à-dire 422 (F21) | `Unexpected { status: Some(422), url, body }` |
| 200, corps lisible comme `ResolveMirror` | `Ok(ResolveMirror)` |
| 200, corps illisible | `Unexpected { status: Some(200), url, body }` |

**Le statut se lit avant la forme, et c'est la leçon d'`execute`.** `post` rend
200 **et** 422 sur son chemin `Ok` (F21) ; tenter `ResolveMirror` d'abord ferait
échouer la désérialisation d'un 422 — dont le corps est un `{"error": …}` sans
aucun des trois champs — et le message d'écart de version porterait alors un
`cause` de serde au lieu du statut reçu. La forme est donc :

```rust
let (statut, corps) = post(base_url, token, "resolve", &envoye).await?;
if statut != 200 {
    return Err(RewindRemoteIssue::Unexpected {
        status: Some(statut),
        url: rewind_url(base_url, "resolve"),
        body: corps,
    });
}
serde_json::from_str(&corps).map_err(|cause| RewindRemoteIssue::Unexpected {
    status: Some(statut),
    url: rewind_url(base_url, "resolve"),
    body: format!("unreadable rewind anchor ({cause}): {corps}"),
})
```

Deux décisions à écrire dans le code, chacune avec son pourquoi :

- **Un 422 est `Unexpected`, jamais `Blocked`, et c'est la conduite inverse
  d'`execute` — pour la même règle.** `execute` rend `Blocked` sur un 422 parce
  que son serveur en émet un pour dire « je refuse ce rembobinage, voici
  pourquoi » (`server/rewind.rs:305-312`). `resolve` n'émet aucun 422 (F20) : il
  ne refuse pas un rembobinage, il cherche un ancrage. Rendre `Blocked` ici
  **inventerait une décision que le serveur n'a pas prise** — l'opérateur lirait
  « mika-spirit refuses the rewind » pour un serveur qui n'a rien refusé et irait
  chercher un verrou de rembobinage au lieu d'un écart de version. Et le lire
  comme un succès donnerait un ancrage vide. La règle commune aux deux portes est
  la même qu'au vol précédent : **on ne perd jamais la nature de ce que le
  serveur a dit pour gagner une forme plus familière.** Le commentaire porte F20,
  c'est-à-dire la mesure, pas l'intuition.
- **Le non-200 n'est pas énuméré.** La condition est `statut != 200` et non
  `statut == 422` : `post` ne garantit pas que son chemin `Ok` restera à deux
  statuts, et un troisième ajouté demain tomberait dans un `else` muet qui le
  passerait à `from_str`. Le bras par défaut est la seule forme qui ne vieillit
  pas — c'est déjà l'argument de F8 une couche plus bas.

**L'`Unexpected` du 200 illisible porte ses trois pièces.** Statut, URL (par
`rewind_url(base_url, "resolve")`, `rewind_remote.rs:160-162`) et texte reçu :
les trois choses qu'un écart de version demande. La phrase `"unreadable rewind
anchor"` est le frère de celle d'`execute` (`"unreadable rewind result"`) et s'en
distingue par un mot, parce qu'un diagnostic qui ne dit pas quelle route a dérivé
oblige à relire le code pour l'apprendre.

### 2.2 Les attributs de non-appel (§ 2.6 du matériau)

La porte n'a aucun appelant jusqu'à V2c-1b : elle porte

```rust
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1b, mika#1727"))]
```

— la forme exacte de `preview` (`rewind_remote.rs:309`) et d'`execute` (`:390`).

`ResolveMirror` (`:21-30`) perd l'attente **« câblé par V2c-1a-2c »**, qui devient
fausse à ce merge : ce vol *est* V2c-1a-2c. Deux issues, et c'est la mesure qui
tranche :

| ce que dit `cargo clippy -p mika-cli --all-targets` | geste |
|---|---|
| plus aucun `dead_code` sur `ResolveMirror` | retirer l'attribut entier |
| « multiple fields are never read » (l'issue de F22) | garder l'attribut, `reason = "champs lus par V2c-1b, mika#1727"`, avec le commentaire qui dit la **population** couverte |

L'issue attendue est la seconde, parce que `ResultMirror` l'a prise un vol plus
tôt sur ce fichier, dans la position exacte (F22). **Attendue n'est pas mesurée :**
le pilote pose, lance `cargo clippy -p mika-cli --all-targets`, lit le message, et
écrit la `reason` que le message justifie. S'il garde l'attribut, il écrit aussi
le commentaire qui dit *pourquoi la raison a changé* — le modèle est à
`rewind_remote.rs:65-74`, et ce commentaire est ce qui empêche le prochain
lecteur de croire que l'attente n'a jamais bougé.

**Jamais `allow` à la place.** `allow` est muet au câblage : au moment où V2c-1b
rend la porte vivante, un `expect` devient `unfulfilled_lint_expectation` et le
build le dit, là où un `allow` resterait indéfiniment, vrai hier et faux pour
toujours. C'est cette obligation faite au vol suivant qui justifie l'existence de
l'attribut, et le critère 5 l'exige en toutes lettres.

### 2.3 L'en-tête du module

Le `//!` de `rewind_remote.rs:1-10` dit trois choses qui deviennent fausses à ce
merge : que `resolve` « va à V2c-1a-2c », que `ResolveMirror` « garde un
`expect(dead_code)` de type mort », et que « les appelants des **deux** portes
arrivent avec V2c-1b ». Les trois phrases se corrigent en cinq lignes qui disent
l'état réel : **les trois portes sont posées, leurs appelants arrivent avec
V2c-1b**, et la note sur les deux formes d'attente est recalée sur ce que la
mesure du § 2.2 aura donné. Un en-tête qui décrit un état antérieur est la forme
la moins coûteuse de documentation fausse, et la plus lue.

---

## 3. Ordre des commits

Le ticket demande qu'une garde arrive avec ses tests, dans le même commit. **Ce
vol porte une garde, donc un commit** :

| # | contenu | tests qui atterrissent avec |
|---|---|---|
| 1 | § 2.1 la porte, § 2.2 attributs, § 2.3 en-tête | T1 à T4 |

Le découper serait pire que grossier. Un commit qui livrerait le 200 puis un
second le non-200 laisserait un état intermédiaire où un 422 tombe dans
`from_str::<ResolveMirror>` et sort en `Unexpected` **avec un message de serde au
lieu du statut** : un commit dont le critère 3 est partiellement faux, et dont le
§ 2.1 explique précisément que c'est la faute à ne pas commettre. Les attributs
partent dans le même commit pour la raison symétrique : c'est lui qui rend
l'attente de `ResolveMirror` caduque, et l'ajuster ailleurs laisserait un commit
portant un `unfulfilled_lint_expectation`, c'est-à-dire un commit dont le
critère 5 est faux.

**Après ce commit, le pilote mesure** `git diff --numstat origin/main...HEAD`,
plan exclu. Au-delà de 279 il s'arrête et dit « HALTE borne : N lignes pour 279 »,
avec le compte par commit. Sur #3077 le pilote a écrit 510 lignes pour 429 sans
rien signaler ; ce silence ne doit pas se reproduire, et avec un seul commit la
mesure est la dernière chose qu'il fait avant la PR.

---

## 4. Le jeu de tests

Tous dans le `#[cfg(test)] mod tests` existant (`rewind_remote.rs:459`), **en fin
de fichier** : le scanner d'egress coupe la production au premier `#[cfg(test)]`
et un item de production qui reprendrait après la coupure sortirait de
l'inventaire (F11).

Le factice `fake_spirit` et son témoin `RequeteRecue` sont réutilisés **tels
quels** (F23) : un `fallback` unique, agnostique de route, et un chemin non
associé qui rend 501 plutôt qu'un 404 qui se lirait comme le `NotFound` métier.
C'est eux qui font du corps envoyé une mesure **sur la socket**, exigée par le
critère 2.

Un helper frère est écrit, `par_la_porte_resolve(statut, corps, count)`, sur le
modèle exact de `par_la_porte_execute` (`:1035`). Il rend le couple
*(`Result<ResolveMirror, RewindRemoteIssue>`, témoin des requêtes reçues)* — le
témoin revient avec, parce que T2 lit la socket. **`count` est un paramètre du
helper et non une constante**, précisément parce que le critère 2 demande une
valeur différente de 1 : un helper qui figerait `count` rendrait T2 inécrivable
sans le contourner.

| test | critère | ce qu'il mesure |
|---|---|---|
| T1 | 1 | un 200 à trois champs par la porte : `session_id`, `after_message_id` et `trace_ids` sont lus un à un sur le `ResolveMirror` rendu — `trace_ids` sur son contenu, pas seulement sur sa longueur |
| T2 | 2 | corps vu **sur la socket** avec `count = 3` : `agent`, `session_id` et `count` aux valeurs passées, `count` **à 3 et non à 1** |
| T3 | 3 | 422 `{"error": "…"}` → `Unexpected { status: Some(422), .. }`, et le rendu n'est **jamais** `Blocked` |
| T4 | 4 | 200 au corps illisible → `Unexpected { status: Some(200), .. }`, et le message porte l'URL de la route `resolve` |

**T2 prend `count = 3`, et ce n'est pas un choix esthétique.** Le serveur a un
défaut à 1 (`default_count`), donc une implémentation qui **omettrait** la clé
`count` serait verte sur tout test qui passe `1` : le serveur comblerait le trou
et le corps vu sur la socket ne serait comparé à rien. Avec `3`, l'omission
devient visible — `corps["count"]` est `null` au lieu de `3` — et c'est
exactement la panne que le critère 2 nomme. Le même raisonnement condamne un test
qui lirait `count` « s'il est présent ».

**T1 ne double pas un test de miroir existant.** Les tests de forme de
`ResolveMirror` appellent `serde_json::from_str::<ResolveMirror>` **directement** :
ils mesurent serde. T1 passe par la porte, et mesure que le bras 200 rend
`Ok(miroir)` — chemin qui n'existe pas sur `7f6bea00`. Si la porte régressait vers
`Unexpected`, le test de forme resterait vert ; seul T1 rougirait.

**T3 est le test de contrat du vol, et il est dissymétrique de celui d'`execute`
par construction.** Sur `execute`, le test équivalent exige `Blocked` ; ici il
exige `Unexpected` **et** interdit `Blocked`. Son assertion est donc double : la
forme est `Unexpected`, et le statut qu'elle porte est bien `Some(422)` — un
`Unexpected` sans statut dit « injoignable » (`rewind_remote.rs:112-131`), une
cause différente et un message différent pour l'opérateur. Un test qui ne
regarderait que le discriminant passerait sur une implémentation qui perd le
statut en route.

**T4 n'est pas le symétrique de T3.** Sur un 200, il n'y a rien à préserver :
l'écart de version est la seule information disponible, et la taire reviendrait à
rendre un ancrage vide comme un succès. **Le fixture de T4 prend la forme la plus
discriminante** : aucun des trois champs de `ResolveMirror` ne porte
`#[serde(default)]` (`rewind_remote.rs:21-30`), donc un objet JSON **valide mais
tronqué** échoue tout autant qu'un texte qui n'est pas du JSON. T4 sert donc
`{"session_id":"s-1"}` et non `"pas du json"` : c'est la forme réelle de l'écart
de version — un serveur qui renomme ou retire un champ —, et c'est la seule des
deux qu'une implémentation naïve passerait. Lire le corps en `Value` puis extraire
les trois champs avec des défauts rendrait un `ResolveMirror` à l'ancrage `0`
**en position de succès** sur ce fixture, et serait vert sur l'autre. T4 est ce
qui interdit cette implémentation.

---

## Fire-Disposition

Ce plan livre des détecteurs — quatre tests (mika#2306), et l'attribut du § 2.2,
dont le chemin de succès est « cet item est bien encore mort ».

**Option retenue : (a) atterrissage armé, et pour l'attribut du § 2.2, une
exception nommée, grep-visible et auto-nettoyante.**

Les quatre tests mesurent une surface **entièrement neuve de ce vol** : une porte
qui n'existe pas sur `7f6bea00`. Aucun n'est un scan de source ni un parcours de
corpus, aucun ne peut donc révéler une violation préexistante. Il n'y a **rien à
amnistier**, et aucune allowlist à ouvrir, pas même vide. Les quatre atterrissent
armés et verts dans le commit qui les justifie (§ 3).

Pour l'attribut du § 2.2, l'exception est l'attribut lui-même, et elle porte les
trois propriétés que mika#1574 (a) demande :

- **elle nomme la donnée précise** — un attribut sur la porte, et le cas échéant
  un sur `ResolveMirror`, jamais un `#![allow(dead_code)]` de module ;
- **elle référence un suivi** — `reason = "câblé par V2c-1b, mika#1727"`,
  grep-visible par `rg 'V2c-1b' crates/mika-cli/src/commands/rewind_remote.rs` ;
  et l'attente périmée de `ResolveMirror` (`"câblé par V2c-1a-2c"`) disparaît au
  passage, pour que ce grep ne ramène plus un vol qui vient d'avoir lieu ;
- **elle rougit quand elle devient stale** — c'est le choix d'`expect` plutôt
  qu'`allow` : au câblage par V2c-1b, l'attribut devient
  `unfulfilled_lint_expectation` et le build le dit. Le § 2.2 interdit
  explicitement de retomber sur `allow`, qui serait muet au câblage.

Si un test rougit à l'atterrissage, la conduite est celle de mika#2201 : corriger
le code, **pas** élargir la mesure. Les trois élargissements tentants sont nommés
pour être refusés — ramener le `count` de T2 à `1` « parce que c'est le cas
nominal », ce qui rendrait l'omission de la clé invisible ; remplacer l'assertion
de statut de T3 par un simple `matches!(…, Unexpected { .. })`, qui passerait sur
une implémentation qui perd le 422 en route ; et servir `"pas du json"` dans T4
au lieu de l'objet tronqué, qui laisserait passer exactement l'implémentation à
défauts que ce test existe pour interdire.

Les options (b) et (c) sont écartées. **(b) expédier désarmé** (`#[ignore]`) est
refusé par le ticket lui-même — « chaque garde arrive avec ses tests, dans le même
commit ; les tests ne se reportent jamais à un vol suivant, même si le code
reste ». **(c) halte-et-remontée** n'a pas lieu : le périmètre tient sous la borne
(§ *Taille estimée*), et aucun critère n'exige de toucher un fichier exclu.

---

## Definition of Done

- [ ] La porte `resolve` existe dans
      `crates/mika-cli/src/commands/rewind_remote.rs`, prend
      `(base_url, token, agent, session_id, count)` et rend
      `Result<ResolveMirror, RewindRemoteIssue>`.
- [ ] Son corps envoie **toujours** `agent`, `session_id` et `count`, sans clé
      conditionnelle et sans garde client sur `count == 0` (commentaire de F8).
- [ ] Tout statut autre que 200 sur le chemin `Ok` de `post` rend
      `Unexpected { status: Some(statut), url, body }` — **jamais** `Blocked` —
      avec le commentaire qui porte F20.
- [ ] Son 200 rend le `ResolveMirror` ; un corps illisible rend `Unexpected`
      portant le statut, l'URL de la route `resolve` et le texte reçu.
- [ ] L'attribut `expect(dead_code)` est sur la porte ; celui de `ResolveMirror`
      est retiré **ou** sa `reason` nomme le vrai motif mesuré (§ 2.2), avec son
      commentaire ; l'en-tête `//!` dit que les trois portes sont posées et que
      leurs appelants arrivent avec V2c-1b.
- [ ] Les quatre tests du § 4 passent, contre le factice `fake_spirit` existant,
      et T2 prend un `count` différent de 1.
- [ ] `cargo clippy -p mika-cli --all-targets` vert, **sans** `allow(dead_code)`
      et **sans** `unfulfilled_lint_expectation`.
- [ ] `cargo fmt --check` vert, `cargo test -p mika-cli` vert.
- [ ] `bash scripts/verify-egress-manifest.sh --report` vert, et `docs/egress/`
      **inchangé** (l'entrée `cli-spirit-rewind` couvre déjà ce module).
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] `git diff --name-only origin/main...HEAD` ne contient, parmi les fichiers
      de code, que `crates/mika-cli/src/commands/rewind_remote.rs` — et **aucun**
      de : `crates/mika-cli/src/tui/commands/handlers.rs`,
      `crates/mika-cli/src/tui/app.rs`, `crates/mika-cli/src/commands/chat.rs`,
      quoi que ce soit sous `crates/mika-agent/` ou `crates/mika-a2a/`,
      `docs/egress/`, ni aucun fichier du plan de contrôle
      (`.github/workflows/`, `.claude/`, `skills/bundled/`, `.mika/`,
      `well_known_agents.rs`, CODEOWNERS).
- [ ] `git diff --numstat origin/main...HEAD` : au plus **279** lignes ajoutées,
      plan exclu. Mesuré après le commit ; au-delà, le pilote s'arrête et dit
      « HALTE borne : N lignes pour 279 ».
- [ ] La PR porte `Refs mika#1727`, et ni `Closes`, ni `Fixes`, ni `Resolves`,
      ni dans le corps ni dans un message de commit — y compris si elle est
      ouverte par le gabarit de reprise, qui écrit `Closes #N` par défaut.
      `gh pr view <N> --json closingIssuesReferences` rend une liste vide.
- [ ] Aucun ticket ouvert (gel jusqu'au 01/12) ; tout reste va en commentaire,
      avec `Tracked in: senara-solutions/mika#1727`.

---

## Acceptance criteria

Transcrits du corps de mika#1727, § *Critères de V2c-1a-2c* :

1. Sur un 200, la porte `resolve` rend le `ResolveMirror` servi, avec ses trois
   champs. Un test le montre par la porte.
2. Le corps envoyé par `resolve` porte `agent`, `session_id` et `count`, lus
   **sur la socket** du factice. `count` y vaut la valeur passée, et un test la
   prend différente de 1.
3. Un 422 sur `resolve` rend `Unexpected`, avec le statut 422, et jamais
   `Blocked`. Un test le montre.
4. Un 200 au corps illisible rend `Unexpected`, avec son statut. Un test le
   montre.
5. `cargo clippy -p mika-cli --all-targets` est vert, sans `allow(dead_code)` et
   sans attente insatisfaite.
6. Seul `rewind_remote.rs` change parmi les fichiers de code. `docs/egress/`,
   `handlers.rs`, `app.rs`, `chat.rs`, `crates/mika-agent/` et `crates/mika-a2a/`
   sont intacts.
7. Le pilote lance `bash scripts/verify-egress-manifest.sh --report` avant
   d'ouvrir la PR. S'il le trouve rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si elle est ouverte par le gabarit de reprise.

---

## 5. Ce que ce vol ne fait pas

- **Il ne câble rien.** `handle_rewind_impl`
  (`tui/commands/handlers.rs:1180-1304`) continue d'appeler `mika_agent::rewind`
  sur la base locale. V2c-1b le retourne, et exigera `MIKA_INTERNAL_TOKEN`.
- **Il n'ajoute aucun miroir** : `ResolveMirror` existe depuis V2c-1a-1 et sa
  forme n'est pas touchée.
- **Il ne convertit pas vers un type de domaine** : les trois champs de l'ancrage
  repartent tels quels vers `preview` et `execute` (§ 1).
- **Il ne met aucune garde sur `count`** : F8, et un commentaire le dit.
- **Il ne résout ni URL ni jeton** : aucune lecture d'environnement dans ce
  module (§ 1).
- **Il ne touche pas au serveur.** Aucune ligne sous `crates/mika-agent/`.
- **Il ne touche pas `docs/egress/`** : l'entrée `cli-spirit-rewind` couvre déjà
  ce module et ses trois routes, « nombre d'échanges » compris (F11).
- **Il n'ouvre aucun ticket** (gel jusqu'au 01/12).

---

## 6. La dette, nommée et non traitée

- **`RewindRemoteIssue::NotFound` reste ambigu par construction** (F6) : sur
  `resolve`, un 404 veut dire « agent inconnu » **ou** « aucun échange dans cette
  session », et le serveur ne les distingue pas. Les deux remèdes de l'opérateur
  sont opposés — corriger un nom d'agent contre comprendre qu'il n'y a rien à
  rembobiner —, et le message du client nomme les deux sans choisir, faute de
  pouvoir. Lever l'ambiguïté demande de distinguer les deux statuts côté
  `crates/mika-agent/`, exclu.
- **Les miroirs restent une seconde déclaration du contrat de fil.** Rien ne tient
  `ResolveMirror` en lockstep avec `RewindResolveResponse` : un champ renommé côté
  serveur ne se verrait qu'à l'exécution, dans l'`Unexpected` du § 2.1. Le remède
  propre — un type partagé `Serialize + Deserialize` dans `mika-common`, ou
  `Deserialize` sur les types du serveur — demande de toucher `crates/mika-agent/`,
  exclu.
- **Le défaut de `count` côté serveur est un piège silencieux pour tout futur
  client.** `default_count` rend 1 : un appelant qui omet la clé obtient un 200 et
  un ancrage d'un seul échange. Ce vol s'en protège en envoyant toujours la clé, et
  T2 le mesure — mais la protection est côté client, et le prochain client HTTP de
  cette route devra la réinventer. Retirer le défaut est un geste côté serveur,
  exclu.
- **Les trois portes partagent `post` mais pas leur discipline de statut** :
  `preview` et `execute` lisent un 422 comme un refus, `resolve` comme un écart.
  La dissymétrie est correcte (F20) et documentée dans les trois corps, mais rien
  ne l'empêche de dériver si une quatrième route arrive. Ce vol la nomme ; il ne
  la structure pas.

Les quatre vont en commentaire sur mika#1727 à l'ouverture de la PR.
`Tracked in: senara-solutions/mika#1727`

---

## Taille estimée

Lignes de code ajoutées, commentaires et tests compris, `docs/` exclu, dans
l'unité de `git diff --numstat origin/main...HEAD`.

### Commit 1 — la porte `resolve`

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.1 doc-comment et signature de la porte | 14 |
| § 2.1 le corps envoyé + les deux commentaires (renvoi à `preview`, F8 sur l'absence de garde) | 12 |
| § 2.1 le non-200 et le pourquoi du 422 (F20) | 14 |
| § 2.1 le 200 et son corps illisible | 10 |
| § 2.2 attributs (ajout sur la porte, recalage ou retrait sur `ResolveMirror` ; les retraits ne coûtent aucune ligne ajoutée) | 4 |
| § 2.3 en-tête `//!` du module | 5 |
| helper de test `par_la_porte_resolve` | 18 |
| T1 (les trois champs par la porte) | 20 |
| T2 (le corps sur la socket, `count = 3`) | 20 |
| T3 (422 → `Unexpected { status: Some(422) }`, jamais `Blocked`) | 16 |
| T4 (200 illisible → `Unexpected` avec son statut et l'URL) | 16 |

Sous-total : 149

Total estimé : 149 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 149 contre 279 : 130 lignes de
marge, 47 % de la borne. La marge résiste au pire dépassement mesuré sur ce
ticket — V2c-1a-2a avait estimé 338 pour 510 livrées, soit **+51 %** ; appliqué
ici, 149 estimées donnent ~225 réelles, encore 54 lignes sous la borne. Le vol
immédiatement précédent, V2c-1a-2b, avait estimé 212 pour une borne de 429 et un
périmètre d'une porte **plus** un miroir **plus** six tests ; ce vol-ci est
strictement plus petit sur les trois axes : **une porte, aucun miroir neuf,
quatre tests.**

Quatre raisons structurelles qui rendent cette marge plus sûre qu'un simple
pourcentage :

1. **`ResolveMirror` existe déjà** (`rewind_remote.rs:21-30`). V2c-1a-2b payait
   `ErrorMirror` (12 lignes) ; ce vol ne paie aucun type.
2. **Le factice, son témoin, `TROIS_ROUTES` et `rewind_url` existent déjà** sur
   `main` (F23). Les postes les plus mal bornés d'un vol de client sont déjà
   payés ; ce vol n'écrit que le helper frère de `par_la_porte_execute`.
3. **Un seul bras d'erreur au lieu de deux.** `execute` avait un 422 lisible et
   un 422 illisible, chacun avec son repli nommé et son test ; `resolve` a un
   seul bras non-200, sans repli à composer, donc deux tests de moins.
4. **Aucune conversion de type, aucun fichier neuf, aucune déclaration de module,
   aucune entrée d'egress.**

Les commentaires comptent dans cette estimation et ne se raboteront pas pour
tenir le chiffre : les ~20 lignes de commentaire du § 2.1 portent chacune un
pourquoi non lisible dans le code (F20 et la dissymétrie assumée avec `execute`,
F8 et l'absence délibérée de garde sur `count`, le renvoi au commentaire d'`agent`
de `preview`, le non-200 non énuméré), au niveau de voisinage que ce fichier
impose déjà. Si le code écrit dépasse 279, le pilote s'arrête et le dit,
conformément au ticket — il ne retire pas après coup les commentaires qui portent
un pourquoi.

La suite — le câblage de `/rewind`, `/rewind to` et `/undo` sur les trois portes
(V2c-1b), puis la création de session et `/compact` par spirit, puis V3 et V4 —
est nommée par la ligne ci-dessous, et par elle seule : le gel des tickets jusqu'au
01/12 interdit d'en ouvrir un.

Tracked in: senara-solutions/mika#1727
