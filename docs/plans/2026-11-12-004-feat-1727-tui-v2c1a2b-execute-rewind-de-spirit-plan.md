# Vol V2c-1a-2b — la porte `execute` de mika-spirit (mika#1727)

> **Branche :** `feat/1727/tui-v2c1a2b-execute-rewind-de-spirit`
> **Base :** `9540b746` (`main` au merge de V2c-1a-2a, mika#3077)
> **PR :** `Refs mika#1727`, jamais `Closes` — le ticket se ferme au merge de V4.

Ce vol pose la **seconde porte** sur le transport de V2c-1a-1 : `execute`
applique un rembobinage sur mika-spirit et rend son résultat, ou un refus
lisible. Aucun appelant ne change. La porte `preview`, livrée par V2c-1a-2a,
est le modèle — et là où le plan matériau et le code de `main` diffèrent, c'est
`main` qui fait foi.

---

## 0. Ce qui est mesuré

Les faits F1 à F11 du plan de V2c-1a (`6a5d20a3`) sont repris **sans
re-mesure**, conformément au ticket. Les trois qui portent ce vol sont redits
ici parce qu'une décision du § 2 s'appuie sur chacun. Les cinq mesures neuves —
celles que V2c-1a-2a a changées, ou que le plan matériau n'avait pas — sont
marquées **(neuf)** et ont été lues sur `9540b746`.

**F5 — le 422 d'`execute` porte un autre corps que celui de `preview`.**
`server/rewind.rs:305-312` : sur une erreur dont le message contient
`"Rewind blocked"`, le handler rend `(422, {"error": <message>})`. Ce n'est
**pas** l'objet d'aperçu que le 422 de `preview` porte (F4) : même refus, autre
forme. Un client qui ne traiterait que la forme de F4 afficherait « erreur HTTP
422 » là où le serveur a dit *pourquoi* — et c'est le refus le plus fréquent du
rembobinage.

**F8 — un 400 est atteignable, donc un bras par défaut est nécessaire.**
`validate_rewind_request` (`server/rewind.rs:66-…`) rend 400 sur un
`session_id` vide ou de plus de 1000 caractères, et sur un `after_message_id`
négatif. Le client ne peut donc pas énumérer les statuts de façon fermée —
exigence déjà **satisfaite** par le bras `_ => Unexpected` de `post`
(`rewind_remote.rs:139-179`). Ce vol n'a rien à y ajouter, et c'est ce fait qui
le dit.

**F11 — le lint d'egress scanne la construction de client.** Elle vit dans
`post`, en production, dans ce fichier, et l'entrée `cli-spirit-rewind`
(`docs/egress/egress-manifest.toml`) la couvre déjà. Ce vol n'ajoute **aucune**
construction de client : `docs/egress/` ne change pas. Mesuré vert sur
`9540b746` (`bash scripts/verify-egress-manifest.sh --report`, 401 fichiers de
production scannés, 32 entrées `[[sink]]` confrontées, aucune violation).

**F15 (neuf) — ni `ResultMirror` ni `RewindResult` ne dérivent `PartialEq`.**
`rewind_remote.rs:62-72` et `mika-agent/src/rewind.rs:72-83` dérivent `Debug`
seulement. Le critère 1 (« la porte rend le `ResultMirror` servi ») ne peut donc
pas se mesurer par une égalité : il se mesure **champ par champ, sur les huit**.
Ce n'est pas un raccourci de test, c'est la seule forme disponible — et c'est
aussi ce qui la rend utile, puisqu'une égalité globale ne dirait pas *lequel* des
huit a dérivé.

**F16 (neuf) — `rustc` ne signale `dead_code` que sur la racine du sous-graphe,
et c'est mesuré ici, pas supposé.** Sur `9540b746`, `post`,
`reversal_action_from_label` et `preview_to_domain` sont **inatteignables en
production** — leur seul appelant est `preview`, qui porte
`expect(dead_code)` — et **aucune des trois ne porte d'attribut**
(`rewind_remote.rs:139`, `:194`, `:213`). Le lint s'arrête à la porte. Par le
même mécanisme, `ResultMirror` et ses huit champs, atteints depuis `execute`
une fois celle-ci annotée, cessent d'être visés : le § 2.3 en tire son attente,
et `preview` en est le précédent exact, un vol plus tôt, sur le même fichier.

**F17 (neuf) — il n'existe aucun formateur de domaine pour le résultat.**
`mika-agent/src/rewind.rs` n'expose qu'un `format_preview` (`:739`) ; la mise en
forme du résultat est écrite **en ligne chez l'appelant**,
`tui/commands/handlers.rs:1283-1299`. La dissymétrie avec V2c-1a-2a vient donc
de là, et non d'un effort économisé : `preview` convertissait vers le domaine
parce qu'un second formateur côté CLI aurait dupliqué la seule vérité
d'affichage. Ici cette vérité **n'existe pas encore**. Convertir vers
`RewindResult` serait une recopie de huit champs identiques un à un, sans
lecteur avant V2c-1b — et V2c-1b devra de toute façon décider où vit ce
formatage, puisqu'il est aujourd'hui dans un fichier que ce vol n'ouvre pas.

**F18 (neuf) — une clé absente et un `null` ne sont équivalents qu'aujourd'hui.**
`RewindRequest.originating_session_id` est `Option<String>` avec
`#[serde(default)]` (`server/rewind.rs:16-18`) : serde rend `None` dans les deux
cas. Mais le jour où ce champ deviendrait `String` avec `#[serde(default)]` —
la forme que `agent` porte déjà une ligne plus haut — l'absence prendrait le
défaut tandis que le `null` **ferait échouer la désérialisation entière** avec
« invalid type: null ». L'absence est donc strictement plus robuste que le
`null`, et c'est pourquoi le critère 2 demande `body.get(…).is_none()` et non
une valeur vide. Le `null` n'est pas refusé par goût de la rigueur.

**F19 (neuf) — l'agent est résolu avant la validation, donc avant tout.**
`handle_rewind_execute` appelle `state.resolve_agent(&req.agent)` en **première**
instruction (`server/rewind.rs:257-266`) et rend 404 avant même
`validate_rewind_request`. Un `agent` omis ferait donc résoudre au serveur *son*
agent par défaut, et l'application d'un rembobinage — effet destructeur — irait
sur la base d'un autre fil. C'est la raison pour laquelle `agent` part toujours ;
elle est déjà écrite sur `preview` (`rewind_remote.rs:290-296`) et ce vol la
**cite** au lieu de la recopier.

---

## 1. La forme : un miroir d'erreur, une porte

Deux livrables seulement, et un seul commit (§ 3) :

1. **`ErrorMirror`** — la forme du corps de F5, pour que la raison du refus se
   lise par un type et non par une sonde dans un `Value` ;
2. **la porte `execute`** — le POST, le corps à clé conditionnelle, le 200 vers
   `ResultMirror`, et le 422 vers `Blocked`.

**Ce qui ne vit pas ici, et ne changera pas.** Le module ne lit ni
`MIKA_SPIRIT_URL` ni `MIKA_INTERNAL_TOKEN` : l'URL de base et le jeton arrivent
par argument, pour la raison déjà écrite sur `fetch_task_detail` et sur
`TaskEventStream::open` — un client qui résout ses propres accès ne se pilote pas
depuis un test. C'est V2c-1b qui les résoudra, au site d'appel, et qui exigera le
jeton.

**Pourquoi la porte rend `ResultMirror` et non `RewindResult`.** Parce que F17 :
la conversion de `preview` achetait l'unicité d'un formateur, et il n'y a ici
aucun formateur de domaine à préserver. Une conversion champ-pour-champ sur huit
champs identiques coûterait ~12 lignes de production et son test, pour un type
que personne ne lit avant V2c-1b — qui aura, lui, à choisir entre convertir et
déplacer le formatage de `handlers.rs`. Ce choix est le sien ; l'anticiper
d'un vol, c'est le prendre à sa place avec moins d'information.

---

## 2. Les livrables, un par un

### 2.1 `ErrorMirror` — le septième miroir

```rust
#[derive(Debug, Deserialize)]
pub(crate) struct ErrorMirror {
    pub(crate) error: String,
}
```

Il rejoint les six miroirs du haut de fichier, sous le même commentaire de bloc
et pour la même raison : les réponses de `server/rewind.rs` ne dérivent que
`Serialize`, et `crates/mika-agent/` est hors périmètre.

**Pourquoi un type et non `serde_json::Value`.** Lire la raison par
`valeur.get("error").and_then(Value::as_str)` tiendrait en une ligne, et c'est
exactement ce qui la rend fragile : la clé n'est alors écrite nulle part comme un
contrat, le type ne dit rien, et un `error` devenu `message` côté serveur
donnerait silencieusement le repli du § 2.2 — c'est-à-dire un refus affiché comme
« blocked without saying why » là où le serveur a dit pourquoi. Avec le miroir,
la forme attendue est déclarée au même endroit que les six autres, et c'est
`serde_json::from_str` qui échoue, visiblement, dans le bras que T5 mesure.

### 2.2 La porte `execute` (§ 2.5 du matériau)

```rust
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1b, mika#1727"))]
pub(crate) async fn execute(
    base_url: &str,
    token: &str,
    agent: &str,
    session_id: &str,
    after_message_id: i64,
    originating_session_id: Option<&str>,
) -> Result<ResultMirror, RewindRemoteIssue>
```

`Option<&str>` et non `Option<String>` : l'appelant de V2c-1b tiendra son
`session_id` d'origine dans son propre état, et n'a pas à l'allouer pour le
prêter.

**Le corps envoyé, et la clé qui n'est pas là.**

```rust
let mut envoye = serde_json::json!({
    "session_id": session_id,
    "after_message_id": after_message_id,
    "agent": agent,
});
if let Some(origine) = originating_session_id {
    envoye["originating_session_id"] = serde_json::Value::String(origine.to_string());
}
```

Deux commentaires à écrire, et rien de plus :

- **`agent` part toujours, y compris vide** — le pourquoi est celui déjà écrit
  sur `preview` (`rewind_remote.rs:290-296`), et F19 l'aggrave d'un cran :
  ici l'omission n'affiche pas le mauvais aperçu, elle **applique** le
  rembobinage sur la base d'un autre fil. Le commentaire renvoie à celui de
  `preview` sans le recopier — deux copies du même pourquoi dérivent, et la
  copie qui dérive est celle qu'on ne relit pas.
- **Une clé absente, et non un `null`** — le `mut` et l'insertion conditionnelle
  existent parce que `json!` ne sait pas omettre une clé ; le commentaire porte
  F18, c'est-à-dire la raison pour laquelle l'absence est *strictement plus
  robuste* que le `null` et non seulement plus propre.

Le flux, et ce que chaque branche rend :

| cas | rendu |
|---|---|
| `post` échoue | remonte tel quel (`?`) — `Denied`, `NotFound`, `Busy`, `Unexpected` (F8) |
| statut 422, corps lisible comme `ErrorMirror` | `Blocked { reason: <champ error>, preview: None }` |
| statut 422, corps illisible | `Blocked { reason: <repli nommé>, preview: None }` |
| 200, corps lisible comme `ResultMirror` | `Ok(ResultMirror)` |
| 200, corps illisible | `Unexpected { status: Some(200), url, body }` |

**L'ordre des deux lectures compte, et c'est le 422 qui passe en premier.**
`post` rend 200 **et** 422 sur le chemin `Ok` ; tenter `ResultMirror` d'abord
ferait échouer la désérialisation d'un 422 — dont le corps n'a aucun des huit
champs — et le ferait sortir en `Unexpected`. Un refus du serveur se lirait alors
comme un écart de version. Le statut est donc lu avant la forme.

Trois décisions à écrire dans le code, chacune avec son pourquoi :

- **Un 422 est `Blocked`, quoi qu'il arrive à son corps.** C'est l'exigence du
  ticket, et sa raison est dissymétrique : la forme du corps est une information
  *sur le client*, le statut 422 est une décision *du serveur*. Rendre
  `Unexpected` sur un corps illisible ferait afficher « mika-spirit returned 422 »
  là où mika-spirit a refusé et dit pourquoi — l'opérateur chercherait un écart
  de version au lieu de lire un refus. La conduite inverse de `preview`, donc,
  **et pour la même règle** : on ne perd jamais la raison d'un refus pour gagner
  un diagnostic qu'on n'a pas demandé.
- **Le repli nomme ce qui s'est passé, et ne l'invente pas.** Si le corps du 422
  n'est pas un `ErrorMirror`, la raison devient une phrase qui dit *que* le
  serveur a refusé sans raison lisible, et qui porte le corps reçu — le seul
  matériau qu'un diagnostic puisse utiliser. `Display` rend alors
  « mika-spirit refuses the rewind: <repli> », une phrase complète, là où une
  chaîne vide donnerait un deux-points suivi de rien.
- **`preview: None`, toujours, et le champ reste annoté.** Le 422 d'`execute` ne
  porte aucun aperçu (F5) : il n'y a rien à mettre là, et un `Some` fabriqué
  serait une invention. Écrire `None` n'est pas une *lecture* du champ, donc
  l'`expect(dead_code)` qu'il porte (`rewind_remote.rs:91`) reste satisfait — il
  tombera avec V2c-1b, son premier lecteur.

**Le 200 illisible est `Unexpected`, avec ses trois pièces.** Statut, URL
(par `rewind_url(base_url, "execute")`, F14 du vol précédent) et texte reçu : les
trois choses qu'un écart de version demande, et que `Blocked` ne porte pas.

### 2.3 Les attributs de non-appel (§ 2.6 du matériau)

La porte n'a aucun appelant jusqu'à V2c-1b : elle porte

```rust
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1b, mika#1727"))]
```

`ResultMirror` perd le sien (`rewind_remote.rs:61`), parce que la porte l'atteint
et que F16 dit — mesuré, pas supposé — que le lint s'arrête à la racine du
sous-graphe mort. `ErrorMirror` naît donc **sans attribut** : il est atteint par
la même porte.

`ResolveMirror` garde le sien (`rewind_remote.rs:20`), racine morte indépendante
que ce vol ne touche pas — mais sa `reason` dit « câblé par V2c-1a-2 », un vol qui
n'existe plus sous ce nom depuis qu'il s'est scindé. Elle devient
`"câblé par V2c-1a-2c, mika#1727"` : une ligne, et une attente dont le suivi se
grep au bon endroit.

**La conduite du pilote :** poser, lancer `cargo clippy -p mika-cli
--all-targets`, **retirer** chaque attente insatisfaite. Jamais `allow` à la
place : `allow` est muet au câblage, et c'est l'obligation faite à V2c-1b de
retirer l'attribut qui justifie son existence.

**Et si la mesure contredit l'attente.** F16 est mesuré sur trois items de ce
fichier, pas sur celui-ci. Si `cargo clippy` réclamait encore un attribut sur
`ResultMirror` — par exemple en visant ses huit champs, qu'aucun site de
production ne lit — le pilote le **garde**, avec une `reason` qui nomme le vrai
motif (`"champs lus par V2c-1b, mika#1727"`), et le dit dans la PR. Le critère 5
exige « sans `allow(dead_code)` et sans attente insatisfaite » : les deux formes
le tiennent, et c'est la mesure qui tranche, jamais la supposition.

### 2.4 L'en-tête du module

Le `//!` de `rewind_remote.rs:1-9` envoie `resolve` **et** `execute` à V2c-1a-2b,
et dit que `ResolveMirror` et `ResultMirror` « gardent seuls » leur attribut. Les
deux phrases deviennent fausses au merge de ce vol. Trois lignes à corriger, qui
nomment les deux portes livrées et renvoient `resolve` à V2c-1a-2c. Un en-tête
qui décrit un état antérieur est la forme la moins coûteuse de documentation
fausse, et la plus lue.

---

## 3. Ordre des commits

Le ticket demande qu'une garde arrive avec ses tests, dans le même commit. **Ce
vol porte une garde, donc un commit** — et le découper serait pire que grossier :

| # | contenu | tests qui atterrissent avec |
|---|---|---|
| 1 | § 2.1 `ErrorMirror`, § 2.2 la porte, § 2.3 attributs, § 2.4 en-tête | T1 à T6 |

Un commit qui livrerait le 200 puis un second le 422 laisserait un état
intermédiaire où un 422 tombe dans `from_str::<ResultMirror>` et sort en
`Unexpected` : un commit dont le critère 3 est **faux**, et dont le § 2.2
explique précisément que c'est la faute à ne pas commettre. Les attributs
partent dans le même commit pour la raison symétrique : c'est lui qui les rend
insatisfaits, et les ajuster ailleurs laisserait un commit portant un
`unfulfilled_lint_expectation`, c'est-à-dire un commit dont le critère 5 est
faux.

---

## 4. Le jeu de tests

Tous dans le `#[cfg(test)] mod tests` existant, **en fin de fichier** : le
scanner d'egress coupe la production au premier `#[cfg(test)]` et un item de
production qui reprendrait après la coupure sortirait de l'inventaire (F11).

Le factice `fake_spirit` (`rewind_remote.rs:403-421`) et son témoin
`RequeteRecue` (`:364-369`) sont réutilisés **tels quels** : ils sont agnostiques
de route — un `fallback` unique, et un chemin non associé qui rend 501 plutôt
qu'un 404 qui se lirait comme le `NotFound` métier. C'est eux qui font du corps
envoyé une mesure **sur la socket**, exigée par le critère 2.

Le helper `par_la_porte` (`:769-775`), lui, est **spécifique à `preview`** :
il appelle `preview` et rend un `RewindPreview`. Un helper frère est donc écrit,
`par_la_porte_execute(statut, corps, origine)`, qui rend le couple
*(`Result<ResultMirror, RewindRemoteIssue>`, témoin des requêtes reçues)*. Le
témoin revient avec, parce que T2 et T3 lisent la socket.

| test | critère | ce qu'il mesure |
|---|---|---|
| T1 | 1 | un 200 à huit champs par la porte : les **huit** sont lus un à un sur le `ResultMirror` rendu (F15 — il n'y a pas d'égalité disponible, et c'est aussi ce qui dit *lequel* a dérivé) |
| T2 | 2 | corps vu **sur la socket** avec `Some("sess-b")` : `agent`, `session_id` et `after_message_id` aux valeurs passées, **et** `originating_session_id` présent à `"sess-b"` |
| T3 | 2 | corps vu **sur la socket** avec `None` : `body.get("originating_session_id").is_none()` — la clé **absente**, et non `null` ni `""` (F18) ; les trois autres champs toujours là |
| T4 | 3 | 422 `{"error": "Rewind blocked: …"}` → `Blocked`, `reason` **égale** le champ `error` du serveur, `preview` est `None` |
| T5 | 3 | 422 au corps illisible → `Blocked` aussi, `reason` porte le repli nommé, `preview` est `None`, et le rendu n'est **jamais** `Unexpected` |
| T6 | 4 | 200 au corps illisible → `Unexpected { status: Some(200), .. }`, et le message porte l'URL de la route `execute` |

**T2 et T3 sont une paire, et ni l'un ni l'autre ne suffit.** T2 seul passerait
sur une implémentation qui écrit toujours la clé — en `null` quand l'origine est
absente, puisque le serveur l'accepte aujourd'hui (F18) ; T3 seul passerait sur
une implémentation qui ne l'écrit **jamais**, c'est-à-dire sur une porte qui
perd silencieusement le rembobinage inter-sessions. C'est la différence entre
« la clé est conditionnelle » et « la clé est absente », et elle ne se mesure
qu'à deux.

**T5 est le témoin négatif de T4, et le seul qui mesure le critère 3 en
entier.** T4 dit que la raison du serveur arrive intacte ; T5 dit que la porte
ne troque pas le refus contre un diagnostic quand elle ne sait pas lire le
corps. Si le bras 422 se repliait sur `Unexpected`, T4 resterait vert — son
corps est lisible — et seul T5 rougirait. Son assertion est donc double : la
forme est `Blocked`, **et** elle ne porte pas une raison vide.

**T6 n'est pas le symétrique de T5.** Sur un 200, il n'y a aucun refus à
préserver : l'écart de version est la seule information disponible, et la taire
reviendrait à rendre un résultat vide comme un succès. T6 lit le statut *dans*
l'`Unexpected`, parce qu'un `Unexpected` sans statut dit « injoignable »
(`rewind_remote.rs:122`) — une cause différente, et un message différent pour
l'opérateur.

---

## Fire-Disposition

Ce plan livre des détecteurs — six tests (mika#2306), et l'attribut du § 2.3,
dont le chemin de succès est « cet item est bien encore mort ».

**Option retenue : (a) atterrissage armé, et pour l'attribut du § 2.3, une
exception nommée, grep-visible et auto-nettoyante.**

Les six tests mesurent une surface **entièrement neuve de ce vol** : un miroir
et une porte qui n'existent pas sur `9540b746`. Aucun n'est un scan de source ni
un parcours de corpus, aucun ne peut donc révéler une violation préexistante. Il
n'y a **rien à amnistier**, et aucune allowlist à ouvrir, pas même vide. Les six
atterrissent armés et verts dans le commit qui les justifie (§ 3).

Pour l'attribut du § 2.3, l'exception est l'attribut lui-même, et elle porte les
trois propriétés que mika#1574 (a) demande :

- **elle nomme la donnée précise** — un attribut sur la porte, jamais un
  `#![allow(dead_code)]` de module ;
- **elle référence un suivi** — `reason = "câblé par V2c-1b, mika#1727"`,
  grep-visible par `rg 'V2c-1b' crates/mika-cli/src/commands/rewind_remote.rs` ;
  et la `reason` de `ResolveMirror` est recalée sur `V2c-1a-2c` au passage, pour
  que ce grep ne ramène pas un vol qui n'existe plus sous ce nom ;
- **elle rougit quand elle devient stale** — c'est le choix d'`expect` plutôt
  qu'`allow` : au câblage par V2c-1b, l'attribut devient
  `unfulfilled_lint_expectation` et le build le dit. Le § 2.3 interdit
  explicitement de retomber sur `allow`, qui serait muet au câblage.

Si un test rougit à l'atterrissage, la conduite est celle de mika#2201 :
corriger le code, **pas** élargir la mesure. Les trois élargissements tentants
sont nommés pour être refusés — remplacer le `is_none()` de T3 par un test sur
la valeur (`!= ""`), qui accepterait le `null` que F18 condamne ; ne garder que
T4 parmi les deux 422, ce qui laisserait exactement passer le repli sur
`Unexpected` que le critère 3 interdit ; et asserter dans T1 un sous-ensemble
des huit champs « parce que les cinq compteurs suffisent », alors que F15 fait de
l'énumération la seule mesure disponible.

Les options (b) et (c) sont écartées. **(b) expédier désarmé** (`#[ignore]`) est
refusé par le ticket lui-même — « chaque garde arrive avec ses tests, dans le
même commit ; les tests ne se reportent jamais à un vol suivant, même si le code
reste ». **(c) halte-et-remontée** n'a pas lieu : le périmètre tient sous la
borne (§ *Taille estimée*), et aucun critère n'exige de toucher un fichier exclu.

---

## Definition of Done

- [ ] `ErrorMirror` existe dans `crates/mika-cli/src/commands/rewind_remote.rs`,
      au bloc des miroirs, avec le commentaire du pourquoi-pas-`Value`.
- [ ] La porte `execute` existe, envoie `agent`, `session_id` et
      `after_message_id`, et n'envoie `originating_session_id` **que** quand il
      est `Some` — clé absente sinon, avec le commentaire de F18.
- [ ] Son 422 rend `Blocked { reason, preview: None }` dans les **deux** cas
      (corps lisible, corps illisible) et jamais `Unexpected`.
- [ ] Son 200 rend le `ResultMirror`, et un corps illisible rend `Unexpected`
      portant le statut, l'URL de la route `execute` et le texte reçu.
- [ ] L'attribut `expect(dead_code)` est sur la porte ; `ResultMirror` n'en a
      plus (ou en garde un dont la `reason` nomme le vrai motif, § 2.3) ;
      `ResolveMirror` renvoie à V2c-1a-2c ; l'en-tête `//!` dit l'état réel.
- [ ] Les six tests du § 4 passent, contre le factice `fake_spirit` existant.
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
- [ ] `git diff --numstat origin/main...HEAD` : au plus **429** lignes ajoutées,
      plan exclu. Mesuré après le commit ; au-delà, le pilote s'arrête et dit
      « HALTE borne : N lignes pour 429 ».
- [ ] La PR porte `Refs mika#1727`, et ni `Closes`, ni `Fixes`, ni `Resolves`,
      ni dans le corps ni dans un message de commit — y compris si elle est
      ouverte par le gabarit de reprise, qui écrit `Closes #N` par défaut.
      `gh pr view <N> --json closingIssuesReferences` rend une liste vide.
- [ ] Aucun ticket ouvert (gel jusqu'au 01/12) ; tout reste va en commentaire,
      avec `Tracked in: senara-solutions/mika#1727`.

---

## Acceptance criteria

Transcrits du corps de mika#1727, § *Critères de V2c-1a-2b* :

1. Sur un 200, la porte `execute` rend le `ResultMirror` servi. Un test le
   montre par la porte.
2. Le corps envoyé par `execute` porte `agent`, `session_id` et
   `after_message_id`. Avec `Some`, il porte `originating_session_id`. Avec
   `None`, la clé est **absente** :
   `body.get("originating_session_id").is_none()`, et non une valeur vide. Les
   deux cas sont lus **sur la socket** du factice.
3. Un 422 `{"error": "…"}` rend `Blocked` avec la raison du serveur et
   `preview: None`. Un 422 au corps illisible rend aussi `Blocked`, avec un repli
   nommé, et jamais `Unexpected`. Chaque cas a son test.
4. Un 200 au corps illisible rend `Unexpected`, avec son statut. Un test le
   montre.
5. `cargo clippy -p mika-cli --all-targets` est vert, sans `allow(dead_code)` et
   sans attente insatisfaite. Seul `rewind_remote.rs` change parmi les fichiers
   de code. `docs/egress/`, `handlers.rs`, `app.rs`, `chat.rs`,
   `crates/mika-agent/` et `crates/mika-a2a/` sont intacts. Le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il
   le trouve rouge, il s'arrête et le dit. La PR porte `Refs mika#1727` et aucun
   `Closes`, `Fixes` ni `Resolves`, y compris si elle est ouverte par le gabarit
   de reprise.

---

## 5. Ce que ce vol ne fait pas

- **Il ne câble rien.** `handle_rewind_impl`
  (`tui/commands/handlers.rs:1180-1304`) continue d'appeler
  `mika_agent::rewind` sur la base locale, et de formater son résultat en ligne
  (F17). V2c-1b le retourne, et exigera `MIKA_INTERNAL_TOKEN`.
- **Il ne livre pas `resolve`.** `ResolveMirror` garde donc son attribut, dont
  seule la `reason` est recalée sur V2c-1a-2c (§ 2.3).
- **Il ne convertit pas vers `RewindResult`** : F17 dit pourquoi, et V2c-1b aura
  l'information qui manque pour trancher entre convertir et déplacer le
  formatage de `handlers.rs`.
- **Il ne résout ni URL ni jeton** : aucune lecture d'environnement dans ce
  module (§ 1).
- **Il ne touche pas au serveur.** Aucun `Deserialize` n'est ajouté côté
  `mika-agent` — ce qui aurait économisé `ErrorMirror` mais violé le périmètre.
- **Il ne touche pas `docs/egress/`** : l'entrée `cli-spirit-rewind` couvre déjà
  ce module, et ce vol n'ajoute aucune construction de client (F11).
- **Il n'ouvre aucun ticket** (gel jusqu'au 01/12).

---

## 6. La dette, nommée et non traitée

- **Les miroirs restent une seconde déclaration du contrat de fil**, et ce vol en
  ajoute un septième. Rien ne les tient en lockstep avec `server/rewind.rs` : un
  `error` renommé `message` côté serveur ne se verrait qu'à l'exécution, dans le
  repli du § 2.2 — c'est-à-dire sous la forme d'un refus affiché comme « sans
  raison lisible ». Le remède propre — un type partagé `Serialize +
  Deserialize` dans `mika-common`, ou `Deserialize` sur les types du serveur —
  demande de toucher `crates/mika-agent/`, exclu.
- **Les deux 422 du serveur ont deux formes pour un seul sens** (F4 contre F5),
  et le client paie cette dissymétrie en deux bras distincts. Elle vient du
  serveur, qui construit l'aperçu complet pour `preview` et un `{"error": …}` pour
  `execute` ; l'uniformiser est un geste côté `mika-agent/`, exclu.
- **`RewindResult` et `ResultMirror` sont deux déclarations de huit champs
  identiques**, qu'aucun compilateur ne confronte. Un neuvième champ côté
  domaine passerait les deux compilations et ne se verrait qu'à l'exécution, en
  silence — le miroir ignorant simplement la clé inconnue. Même remède, même
  exclusion.
- **`RewindRemoteIssue::NotFound` reste ambigu par construction** (F6) : le
  serveur ne distingue pas « agent inconnu » de « rien à rembobiner », et F19
  montre qu'il résout l'agent en première instruction — donc que la cause la plus
  probable d'un 404 sur `execute` est le nom d'agent, là où le message nomme les
  deux sans choisir.

Les quatre vont en commentaire sur mika#1727 à l'ouverture de la PR.
`Tracked in: senara-solutions/mika#1727`

---

## Taille estimée

Lignes de code ajoutées, commentaires et tests compris, `docs/` exclu, dans
l'unité de `git diff --numstat origin/main...HEAD`.

### Commit 1 — la porte `execute`

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.1 `ErrorMirror` + le commentaire du pourquoi-pas-`Value` | 12 |
| § 2.2 doc-comment et signature de la porte | 16 |
| § 2.2 le corps envoyé + les deux commentaires (renvoi à `preview`, F18) | 20 |
| § 2.2 le 422, son repli nommé et le pourquoi de l'ordre des lectures | 16 |
| § 2.2 le 200 et son corps illisible (F15) | 10 |
| § 2.3 attributs (ajout sur la porte, recalage de la `reason` de `ResolveMirror` ; les retraits ne coûtent aucune ligne ajoutée) | 4 |
| § 2.4 en-tête `//!` du module | 6 |
| helper de test `par_la_porte_execute` | 18 |
| T1 (les huit champs par la porte) | 24 |
| T2 (le corps sur la socket, `Some`) | 20 |
| T3 (le corps sur la socket, clé absente) | 16 |
| T4 (422 `error` → `Blocked`, `preview: None`) | 18 |
| T5 (422 illisible → `Blocked` au repli nommé) | 16 |
| T6 (200 illisible → `Unexpected` avec son statut) | 16 |

Sous-total : 212

Total estimé : 212 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient largement.** 212 contre 429 : 217
lignes de marge, 51 % de la borne. Le précédent d'estimation le plus proche en
forme sur ce ticket — V2b-2b-1, un client HTTP neuf avec son factice et ses
tests — avait estimé 241 pour 279 livrées, soit **+16 %** ; appliqué ici, 212
estimées donnent ~246 réelles, soit encore 183 lignes de marge. Le vol
immédiatement précédent, V2c-1a-2a, avait estimé 338 et le pilote a livré 510
pour une borne de 429 sans le signaler — c'est l'incident que la halte du ticket
existe pour empêcher, et c'est la raison pour laquelle ce plan prend un périmètre
de moitié : **une seule porte, aucune conversion, aucun helper de domaine.**

Trois raisons structurelles qui rendent cette marge plus sûre qu'un simple
pourcentage :

1. **Le factice, son témoin et `rewind_url` existent déjà** sur `main`
   (`rewind_remote.rs:133-135`, `:364-421`). Les trois postes les plus mal bornés
   d'un vol de client sont déjà payés ; ce vol n'écrit que le helper frère de
   `par_la_porte`.
2. **Aucune conversion de type.** C'était le poste le plus lourd de V2c-1a-2a —
   `preview_to_domain` et sa table de graphies, ~60 lignes de production et ~66
   de tests. F17 le retire de ce vol, et pas par économie : il n'a pas de
   lecteur avant V2c-1b.
3. **Aucun fichier neuf, aucune déclaration de module, aucune entrée d'egress.**

Les commentaires comptent dans cette estimation et ne se raboteront pas pour
tenir le chiffre : les ~34 lignes de commentaire des § 2.1 à 2.2 portent chacune
un pourquoi non lisible dans le code (le pourquoi-pas-`Value`, F18, F19, l'ordre
des deux lectures, la dissymétrie assumée avec `preview`), au niveau du
voisinage que ce fichier impose déjà. Si le code écrit dépasse 429, le pilote
s'arrête et le dit, conformément au ticket.

`Tracked in: senara-solutions/mika#1727`
