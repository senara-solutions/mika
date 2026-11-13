# Vol V2c-1a-2a — l'aperçu rewind de mika-spirit (mika#1727)

> **Branche :** `feat/1727/tui-v2c1a2a-apercu-rewind-de-spirit`
> **Base :** `a40b7547` (`main` au merge de V2c-1a-1, mika#3076)
> **PR :** `Refs mika#1727`, jamais `Closes` — le ticket se ferme au merge de V4.

Ce vol pose la **première porte** sur le transport que V2c-1a-1 a livré : la
conversion de l'aperçu du fil vers la forme du domaine, et la porte `preview`
qui la traverse. Aucun appelant ne change.

---

## 0. Ce qui est mesuré

Les faits F1 à F11 du plan de V2c-1a (`6a5d20a3`) sont repris **sans
re-mesure**, conformément au ticket. Les cinq qui portent ce vol sont redits ici
parce qu'une décision du § 2 s'appuie sur chacun. Les quatre mesures neuves —
celles que V2c-1a-1 a changées, ou que le plan matériau n'avait pas — sont
marquées **(neuf)** et ont été lues sur `a40b7547`.

**F2 — les réponses du serveur sont `Serialize` seulement.**
`RewindPreviewResponse` et ses trois sous-structs
(`crates/mika-agent/src/server/rewind.rs:21-51`) ne dérivent que `Serialize`, et
`crates/mika-agent/` est hors périmètre : les miroirs côté CLI sont **forcés**.
Ils sont déjà sur `main`.

**F3 — `ReversalPreviewResponse.action` est une forme `Debug`.**
`server/rewind.rs:217` écrit `action: format!("{:?}", r.action)`. Les trois
graphies possibles sont donc exactement `"Restore"`, `"Delete"`, `"Skip"`
(`rewind.rs:31-39`).

**F4 — le 422 de `preview` porte le corps d'un 200.** `server/rewind.rs:234-247`
construit la réponse complète, puis choisit le statut sur `resp.blocked`. Le
corps est le **même objet** dans les deux cas — c'est ce qui rend `preview:
Some(..)` atteignable sur un 422, et le critère 3 réalisable.

**F4′ (neuf) — le 200 dont `blocked` est vrai n'est pas produit par ce serveur.**
Le `if resp.blocked` de `server/rewind.rs:234` est exhaustif : tout aperçu bloqué
sort en 422. Le second bras exigé par le critère 3 couvre donc une **incohérence
de serveur**, pas une forme observée. Il reste écrit, et pour une raison qui
n'est pas la symétrie : propager un aperçu `blocked` en `Ok` ferait afficher par
`format_preview` la ligne `BLOCKED: …` **dans la position d'un succès**, c'est-à-dire
un refus rendu comme une opération possible. Le traiter comme un blocage est le
seul choix qui ne mente pas, et il ne dépend d'aucune promesse du serveur.

**F9 — `format_preview` ne lit pas `trace_id`.**
`crates/mika-agent/src/rewind.rs:739-785` lit le *nombre* de messages, puis les
`reversals`, les `tasks` et les `warnings`. `MessagePreview.trace_id`
(`rewind.rs:27`) n'est pas sur le fil et vaudra `None` après conversion : **sans
effet sur le texte**, et c'est ce qui rend le critère 1 atteignable.

**F11 — le lint d'egress scanne la construction de client.** La construction
vit dans `post`, en production, dans ce fichier, et l'entrée
`cli-spirit-rewind` (`docs/egress/egress-manifest.toml:363-371`) la couvre déjà.
Ce vol n'ajoute **aucune** construction de client : `docs/egress/` ne change pas.
Mesuré vert sur `a40b7547` (`bash scripts/verify-egress-manifest.sh --report`,
32 entrées confrontées, aucune violation).

**F12 (neuf) — `RewindPreview` n'est pas `PartialEq`.** `rewind.rs:60-70` dérive
`Debug` seulement, et aucun des quatre types d'aperçu ne dérive `PartialEq`
(seul `ReversalAction` le fait, `rewind.rs:30`). L'égalité du critère 1 n'est
donc pas une égalité structurelle qu'on aurait choisi de ne pas prendre : elle
**ne peut être** que l'égalité du texte rendu. Ce qui tombe bien, puisque le
texte est la surface que l'opérateur lit, et la raison même de convertir vers le
domaine plutôt que de reformater ici.

**F13 (neuf) — le domaine ne produit jamais `blocked` sans raison.** Les trois
sites de blocage (`rewind.rs:189`, `:207`, `:1239`) portent tous un
`block_reason: Some(..)`, et `block_reason: None` n'apparaît nulle part avec
`blocked: true`. Le repli du § 2.3 garde donc le **type** (`Option<String>`),
non une forme observée — et il a son test pour la même raison qu'il existe :
un chemin non mesuré est un chemin qui dérive.

**F14 (neuf) — `post` ne rend pas son URL sur le chemin `Ok`.** Sa signature est
`Result<(u16, String), RewindRemoteIssue>` (`rewind_remote.rs:128-133`) : l'URL
n'est connue de l'appelant que par les bras `Err`. La porte, qui doit bâtir un
`Unexpected { url, .. }` sur un corps illisible, n'y a donc pas accès. Le § 2.1
en tire sa seule retouche au code existant.

---

## 1. La forme : une table, une conversion, une porte

Trois livrables, dans leur ordre de dépendance, et chacun son commit :

1. **`reversal_action_from_label`** — la table des trois graphies de F3, et le
   refus de tout le reste ;
2. **`preview_to_domain`** — l'aperçu du fil reconstruit en `RewindPreview` ;
3. **la porte `preview`** — le POST, la désérialisation, la conversion, et les
   deux chemins de blocage.

**Ce qui ne vit pas ici, et ne changera pas.** Le module ne lit ni
`MIKA_SPIRIT_URL` ni `MIKA_INTERNAL_TOKEN` : l'URL de base et le jeton arrivent
par argument, pour la raison déjà écrite sur `fetch_task_detail` et sur
`TaskEventStream::open` — un client qui résout ses propres accès ne se pilote
pas depuis un test. C'est V2c-1b qui les résoudra, au site d'appel.

**Pourquoi convertir vers le domaine et non formater ici.** `format_preview`
est la seule mise en forme de l'aperçu, et elle est testée chez elle. Un second
formateur côté CLI serait exactement la duplication que mika#1727 existe pour
retirer. La conversion est le prix d'admission : elle coûte ~42 lignes et garde
**une seule vérité d'affichage**. F12 ajoute une seconde raison, plus forte : on
ne saurait pas comparer deux aperçus autrement que par ce texte.

---

## 2. Les livrables, un par un

### 2.1 `rewind_url` — une retouche, pas un livrable

```rust
fn rewind_url(base_url: &str, route: &str) -> String {
    format!("{}/api/v1/rewind/{route}", base_url.trim_end_matches('/'))
}
```

`post` l'appelle au lieu de composer l'URL dans son corps ; la porte l'appelle
pour bâtir ses `Unexpected` (F14). **Un second `format!` dans la porte serait la
seule alternative, et c'est précisément celle à refuser** : deux compositions de
la même URL divergent le jour où le préfixe `/api/v1` change, et la divergence
se verrait dans un *message d'erreur* — l'endroit où personne ne la cherche.

Pourquoi pas élargir `post` à `Ok((u16, String, String))` : ça rendrait l'URL à
l'appelant, mais au prix de retoucher les six tests de transport déjà verts sur
`main`. La fonction extraite ne touche aucun test.

### 2.2 `reversal_action_from_label` (§ 2.4 du matériau)

```rust
fn reversal_action_from_label(label: &str) -> Result<ReversalAction, String>
```

Trois bras — `"Restore"`, `"Delete"`, `"Skip"` de F3 — et un bras d'erreur dont
le message **nomme la valeur reçue** et les trois graphies acceptées.

**Aucun repli sur `Skip`, et c'est la décision centrale de ce vol.** `Skip` veut
dire « rien à inverser » et `format_preview` lui met l'icône `⏭`
(`rewind.rs:758`). Un repli ferait donc afficher « rien à inverser » pour une
variante que le serveur aurait ajoutée — c'est-à-dire **mentirait sur un effet
mémoire non réversible**, en présentant une mutation qu'on ne sait pas défaire
comme une mutation qu'il n'y a pas à défaire. C'est le mensonge le plus coûteux
que ce module puisse produire, et le critère 2 l'interdit nommément. Un repli sur
`Restore` ou `Delete` serait pire encore : il annoncerait un effet.

Pas de `serde` sur `ReversalAction` pour s'en dispenser : ce type est dans
`crates/mika-agent/`, hors périmètre, et un `Deserialize` ajouté là produirait
par défaut le même refus sans le message qui nomme la valeur.

### 2.3 `preview_to_domain` (§ 2.4 du matériau)

```rust
fn preview_to_domain(miroir: PreviewMirror) -> Result<RewindPreview, String>
```

Reconstruit `MessagePreview`, `ReversalPreview`, `TaskPreview`, puis
`RewindPreview`. Trois points que le code ne dit pas et que le commentaire doit
dire :

- **`trace_id: None`, avec le pourquoi de F9** — le fil ne porte pas ce champ, et
  `format_preview` ne le lit pas, donc l'absence est sans effet sur le texte
  *aujourd'hui*. Le commentaire nomme la ligne qui rougira à la relecture si
  `format_preview` se mettait à le lire : c'est la seule divergence que la
  conversion introduit volontairement.
- **Une action inconnue fait échouer l'aperçu entier**, par `?` sur
  `reversal_action_from_label`, et non un reversal silencieusement sauté. Sauter
  rendrait un aperçu *plus court que la réalité* — un opérateur lirait « 2
  changements mémoire » là où il y en a trois, et validerait un rembobinage dont
  une partie ne se verrait jamais.
- **`blocked` et `block_reason` traversent la conversion** et ne sont pas
  consommés par elle : `RewindPreview` les porte (`rewind.rs:67-68`), et
  `format_preview` les lit en premier (`rewind.rs:742-748`). La porte les lit
  **avant** la conversion pour sa propre décision (§ 2.4) ; les laisser passer
  n'est donc pas une redite, c'est ce qui permet au `BLOCKED: …` d'être rendu si
  un appelant formate un aperçu bloqué.

### 2.4 La porte `preview` (§ 2.5 du matériau)

```rust
pub(crate) async fn preview(
    base_url: &str,
    token: &str,
    agent: &str,
    session_id: &str,
    after_message_id: i64,
) -> Result<RewindPreview, RewindRemoteIssue>
```

Corps envoyé : `{"session_id": …, "after_message_id": …, "agent": …}`.

**`agent` part toujours, y compris vide.** Le serveur a `#[serde(default)]`
dessus (`server/rewind.rs:15-16`) : un champ absent lui fait résoudre *son*
agent par défaut (`state.resolve_agent`), qui n'est pas celui de la TUI. Le
rembobinage s'appliquerait alors à la session d'un autre agent — un effet
destructeur sur la mauvaise base, déclenché par une omission. Le commentaire dit
cela, pas « on envoie agent ».

Le flux, et ce que chaque branche rend :

| cas | rendu |
|---|---|
| `post` échoue | remonte tel quel (`?`) — `Denied`, `NotFound`, `Busy`, `Unexpected` |
| corps illisible comme `PreviewMirror` | `Unexpected { status: Some(statut), url, body }` |
| statut 422, **ou** `miroir.blocked` (F4′) | `Blocked { reason, preview }` |
| 200 non bloqué, conversion réussie | `Ok(RewindPreview)` |
| 200 non bloqué, conversion échouée | `Unexpected { status: Some(200), url, body: <message> }` |

Trois décisions à écrire dans le code, chacune avec son pourquoi :

- **Une forme illisible est `Unexpected`, jamais `Blocked`.** Un blocage est une
  décision du serveur ; une forme illisible est une incapacité du client, et
  l'ordinaire de la cause est un écart de version (F2 : rien ne tient les
  miroirs en lockstep). Les confondre afficherait « mika-spirit refuses the
  rewind » là où mika-spirit a dit oui. `Unexpected` est le seul bras qui porte
  à la fois le statut, l'URL et le texte — les trois choses qu'un écart de
  version demande.
- **La raison du blocage est lue avant la conversion, et porte un repli nommé.**
  `block_reason` est un `Option<String>` ; F13 dit que le domaine le remplit
  toujours, mais le type autorise l'absence, et `Display` rendrait alors
  « mika-spirit refuses the rewind: » — une phrase tronquée là où l'opérateur
  attend la cause. Le repli nomme ce qui s'est passé (« blocked without saying
  why »), ce qui est vrai et exploitable, au lieu d'inventer une cause.
- **Sur un blocage, l'aperçu est best-effort (`.ok()`), le blocage ne l'est
  pas.** Si le corps d'un 422 porte une action inconnue, la conversion échoue —
  et rendre `Unexpected` perdrait alors la **raison du refus**, qui est
  l'information que l'opérateur est venu chercher. On rend donc `Blocked` avec
  sa raison et `preview: None`. L'inverse échangerait une information utile
  contre un diagnostic de version.

### 2.5 Les attributs de non-appel (§ 2.6 du matériau)

La porte n'a aucun appelant jusqu'à V2c-1b : elle porte

```rust
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1b, mika#1727"))]
```

Les items qu'elle **atteint** perdent le leur. `rustc` ne signale `dead_code`
que sur la **racine** d'un sous-graphe inatteignable ; un `expect` posé sur un
item que le lint ne vise plus devient `unfulfilled_lint_expectation`, un
warning — c'est-à-dire que l'attribut rougit dans les deux sens, ce qui est toute
sa valeur.

Attente, à **mesurer et non à supposer** : perdent leur attribut `PreviewMirror`,
`MessageMirror`, `ReversalMirror`, `TaskMirror`, `RewindRemoteIssue` et `post`
(tous atteints depuis la porte) ; gardent le leur `ResolveMirror` et
`ResultMirror`, racines mortes indépendantes que ce vol ne touche pas. Les deux
fonctions du § 2.2-2.3 naissent sans attribut sauf dans leur commit propre, où
elles n'ont encore que des tests (§ 3).

**La conduite du pilote :** poser, lancer `cargo clippy -p mika-cli
--all-targets`, **retirer** chaque attente insatisfaite. Jamais `allow` à la
place : `allow` est muet au câblage, et c'est l'obligation faite à V2c-1b de
retirer l'attribut qui justifie son existence.

### 2.6 L'en-tête du module

Le `//!` de `rewind_remote.rs:1-7` annonce « V2c-1a-2 câble les portes et la
conversion ». Il devient faux au merge de ce vol, qui n'en câble qu'une. Deux
lignes à corriger, qui nomment la porte livrée et renvoient `resolve`/`execute`
à V2c-1a-2b. Un en-tête qui décrit un état antérieur est la forme la moins
coûteuse de documentation fausse, et la plus lue.

---

## 3. Ordre des commits

Le pilote commite après chaque garde verte avec son test, et non en fin de vol.

| # | contenu | tests qui atterrissent avec |
|---|---|---|
| 1 | § 2.2 `reversal_action_from_label` | T1, T2 |
| 2 | § 2.3 `preview_to_domain` | T3, T4 |
| 3 | § 2.1 `rewind_url`, § 2.4 la porte, § 2.5 attributs, § 2.6 en-tête | T5 à T13 |

Le commit 3 porte les attributs parce que c'est **lui** qui les rend
insatisfaits : les ajuster ailleurs laisserait un commit intermédiaire avec un
`unfulfilled_lint_expectation`, c'est-à-dire un commit dont le critère 5 est
faux.

---

## 4. Le jeu de tests

Tous dans le `#[cfg(test)] mod tests` existant, **en fin de fichier** : le
scanner d'egress coupe la production au premier `#[cfg(test)]` et un item de
production qui reprendrait après la coupure sortirait de l'inventaire (F11).

Le factice `fake_spirit` (`rewind_remote.rs:222-240`) et son témoin
`RequeteRecue` sont réutilisés tels quels — c'est eux qui font du corps envoyé
une mesure **sur la socket**, exigée par le critère 4. Un helper neuf,
`un_apercu(action)`, rend le couple *(corps JSON servi, `RewindPreview` attendu
construit à la main)* pour une action donnée : c'est ce qui permet à T5-T7 de
tenir en six lignes chacun sans que l'attendu devienne un second formateur.

| test | critère | ce qu'il mesure |
|---|---|---|
| T1 | 2 | les trois graphies de F3 donnent les trois `ReversalAction`, une par une |
| T2 | 2 | `"Rollback"` → `Err`, et le message **nomme la valeur reçue** ; aucune des trois actions n'est rendue |
| T3 | 1 | un aperçu complet converti : messages, reversals, tasks, warnings, `blocked`/`block_reason` tous présents, et `trace_id` à `None` sur chaque message (F9) |
| T4 | 2 | une action inconnue fait échouer l'**aperçu entier** : `Err`, et non un aperçu à un reversal de moins |
| T5 | 1 | `Restore` par la porte : `format_preview(rendu)` **égal** à `format_preview(attendu)` |
| T6 | 1 | idem `Delete` |
| T7 | 1 | idem `Skip` |
| T8 | 4 | le corps vu **sur la socket** porte `agent`, `session_id` et `after_message_id`, aux valeurs passées |
| T9 | 3 | 422 avec `blocked`+`block_reason` → `Blocked`, `reason` **égale** la raison du serveur, `preview` est `Some` |
| T10 | 3 | 200 dont `blocked` est vrai → `Blocked` aussi (F4′) |
| T11 | 2 | action inconnue par la porte → `Unexpected`, et le message ne porte **aucune** des trois icônes `↩`/`🗑`/`⏭` |
| T12 | 3 | 422 dont l'aperçu est illisible → `Blocked` avec sa raison, `preview: None` |
| T13 | 3 | 422 sans `block_reason` → `reason` porte le repli nommé, non une chaîne vide |

**Pourquoi T6 et T7 ne sont pas des doublons de T5 :** les trois actions
prennent trois bras distincts de `reversal_action_from_label` et reçoivent trois
icônes distinctes dans `format_preview` (`rewind.rs:755-759`). Un seul test sur
`Restore` laisserait passer une table de correspondance **permutée** — l'erreur
exactement la plus probable à l'écriture.

**T2 et T11 sont le témoin négatif de T1 et T5-T7, et les deux sont
nécessaires.** T2 mesure la table ; T11 mesure que la porte ne fabrique pas un
succès par-dessus l'échec de la table. Si `reversal_action_from_label` se
repliait sur `Skip`, T2 rougirait sur son `expect_err` **et** T11 rougirait sur
l'icône `⏭` — c'est la seconde moitié du critère 2 (« aucune icône n'est
rendue »), et elle ne se mesure que par la porte.

**T11 assert l'absence des trois icônes, et non de `⏭` seule** : un repli sur
`Restore` ou `Delete` est tout aussi possible à l'écriture, et une assertion sur
une seule icône laisserait passer les deux autres.

---

## Fire-Disposition

Ce plan livre des détecteurs — treize tests (mika#2306), et l'attribut du § 2.5,
dont le chemin de succès est « cet item est bien encore mort ».

**Option retenue : (a) atterrissage armé, et pour l'attribut du § 2.5, une
exception nommée, grep-visible et auto-nettoyante.**

Les treize tests mesurent une surface **entièrement neuve de ce vol** : deux
fonctions et une porte qui n'existent pas sur `a40b7547`. Aucun n'est un scan de
source ni un parcours de corpus, aucun ne peut donc révéler une violation
préexistante. Il n'y a **rien à amnistier**, et aucune allowlist à ouvrir, pas
même vide. Les treize atterrissent armés et verts dans les commits qui les
justifient (§ 3).

Pour l'attribut du § 2.5, l'exception est l'attribut lui-même, et elle porte les
trois propriétés que mika#1574 (a) demande :

- **elle nomme la donnée précise** — un attribut par item, jamais un
  `#![allow(dead_code)]` de module ;
- **elle référence un suivi** — `reason = "câblé par V2c-1b, mika#1727"`,
  grep-visible par `rg 'V2c-1b' crates/mika-cli/src/commands/rewind_remote.rs` ;
- **elle rougit quand elle devient stale** — c'est le choix d'`expect` plutôt
  qu'`allow` : au câblage par V2c-1b, l'attribut devient
  `unfulfilled_lint_expectation` et le build le dit. Le § 2.5 interdit
  explicitement de retomber sur `allow`, qui serait muet au câblage.

Si un test rougit à l'atterrissage, la conduite est celle de mika#2201 :
corriger le code, **pas** élargir la mesure. Les trois élargissements tentants
sont nommés pour être refusés — remplacer l'égalité de texte de T5-T7 par un
`contains` parce que le préfixe suffit, ce qui effacerait la moitié « le même
texte » du critère 1 ; faire de T11 une assertion sur `⏭` seule, qui laisserait
passer les deux autres replis ; et faire de T4 un test sur la présence d'un
`Err` sans vérifier qu'aucun aperçu partiel n'est rendu, ce qui est la seule
moitié qui mesure quelque chose.

Les options (b) et (c) sont écartées. **(b) expédier désarmé** (`#[ignore]`) est
refusé par le ticket lui-même — « chaque garde arrive avec ses tests, dans le
même commit ; les tests ne se reportent jamais à un vol suivant, même si le code
reste ». **(c) halte-et-remontée** n'a pas lieu : le périmètre tient sous la
borne (§ *Taille estimée*), et aucun critère n'exige de toucher un fichier exclu.

---

## Definition of Done

- [ ] `reversal_action_from_label` et `preview_to_domain` existent dans
      `crates/mika-cli/src/commands/rewind_remote.rs`, avec le commentaire de F9
      sur `trace_id: None` et celui du non-repli sur la table.
- [ ] La porte `preview` existe, envoie `agent`, `session_id` et
      `after_message_id`, et rend `Blocked` sur un 422 **comme** sur un 200
      bloqué.
- [ ] Les treize tests du § 4 passent, contre le factice `fake_spirit` existant.
- [ ] `cargo clippy -p mika-cli --all-targets` vert, **sans** `allow(dead_code)`
      et **sans** `unfulfilled_lint_expectation`.
- [ ] `cargo fmt --check` vert, `cargo test -p mika-cli` vert.
- [ ] `bash scripts/verify-egress-manifest.sh --report` vert, et
      `docs/egress/` **inchangé** (l'entrée `cli-spirit-rewind` couvre déjà ce
      module).
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
      plan exclu.
- [ ] La PR porte `Refs mika#1727`, et ni `Closes`, ni `Fixes`, ni `Resolves`,
      ni dans le corps ni dans un message de commit.
- [ ] Aucun ticket ouvert (gel jusqu'au 01/12) ; tout reste va en commentaire,
      avec `Tracked in: senara-solutions/mika#1727`.

---

## Acceptance criteria

Transcrits du corps de mika#1727, § *Critères de V2c-1a-2a* :

1. Pour `Restore`, `Delete` et `Skip`, le `format_preview` de l'aperçu rendu par
   la porte `preview` est **égal** à celui du `RewindPreview` du domaine qu'il
   représente. Chaque action a son test.
2. Une action inconnue rend une erreur dont le message nomme la valeur reçue, et
   aucune icône n'est rendue. Un test le montre, et il rougit si la table se
   replie sur `Skip`.
3. Un 422 sur `preview` rend `Blocked` avec la raison du serveur et
   `preview: Some`. Un 200 dont `blocked` est vrai rend aussi `Blocked`. Chaque
   cas a son test.
4. Le corps envoyé par `preview` porte `agent`, `session_id` et
   `after_message_id`. Un test le lit **sur la socket** du factice.
5. `cargo clippy -p mika-cli --all-targets` est vert, sans `allow(dead_code)` et
   sans attente insatisfaite. Seul `rewind_remote.rs` change parmi les fichiers
   de code. `docs/egress/`, `handlers.rs`, `app.rs`, `chat.rs`,
   `crates/mika-agent/` et `crates/mika-a2a/` sont intacts. Le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il
   le trouve rouge, il s'arrête et le dit. La PR porte `Refs mika#1727` et aucun
   `Closes`, `Fixes` ni `Resolves`.

---

## 5. Ce que ce vol ne fait pas

- **Il ne câble rien.** `handle_rewind_impl` continue d'appeler
  `mika_agent::rewind` sur la base locale. V2c-1b le retourne, et exigera
  `MIKA_INTERNAL_TOKEN`.
- **Il ne livre ni `resolve` ni `execute`**, ni le 422 d'`execute` (forme F5,
  distincte de F4). Ils vont à V2c-1a-2b. `ResolveMirror` et `ResultMirror`
  gardent donc leur attribut.
- **Il ne résout ni URL ni jeton** : aucune lecture d'environnement dans ce
  module (§ 1).
- **Il ne touche pas au serveur.** Aucun `Deserialize` n'est ajouté côté
  `mika-agent` — ce qui aurait économisé les miroirs mais violé le périmètre.
- **Il ne touche pas `docs/egress/`** : l'entrée `cli-spirit-rewind` couvre déjà
  ce module, et ce vol n'ajoute aucune construction de client (F11).
- **Il n'ouvre aucun ticket** (gel jusqu'au 01/12).

---

## 6. La dette, nommée et non traitée

- **Les miroirs restent une seconde déclaration du contrat de fil.** Rien ne les
  tient en lockstep avec `server/rewind.rs` : un champ renommé côté serveur se
  verrait à l'exécution, pas à la compilation. Ce vol ne fait qu'**ajouter un
  lecteur** à cette dette ; il la rend un peu plus visible, puisqu'un corps
  illisible a maintenant un message qui le dit (`Unexpected`). Le remède propre —
  un type partagé `Serialize + Deserialize` dans `mika-common`, ou `Deserialize`
  sur les types du serveur — demande de toucher `crates/mika-agent/`, exclu.
- **La table des graphies est une troisième déclaration de `ReversalAction`.**
  Le serveur sérialise par `format!("{:?}")` (F3), le client re-parse par une
  table : deux conventions non liées autour d'un enum qui n'a ni `Serialize` ni
  `Deserialize`. Un quatrième variant côté domaine passerait la compilation des
  deux côtés et ne rougirait qu'à l'exécution, dans T2. Le remède est le même
  que ci-dessus, et exclu pour la même raison.
- **`RewindRemoteIssue::NotFound` reste ambigu par construction** (F6) : le
  serveur ne distingue pas « agent inconnu » de « rien à rembobiner ».

Les trois vont en commentaire sur mika#1727 à l'ouverture de la PR.
`Tracked in: senara-solutions/mika#1727`

---

## Taille estimée

Lignes de code ajoutées, commentaires et tests compris, `docs/` exclu, dans
l'unité de `git diff --numstat origin/main...HEAD`.

### Commit 1 — la table des graphies

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.2 `reversal_action_from_label` + le commentaire du non-repli | 18 |
| T1, T2 | 24 |

Sous-total : 42

### Commit 2 — la conversion

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.3 `preview_to_domain` + le commentaire de `trace_id: None` (F9) | 42 |
| T3, T4 | 42 |

Sous-total : 84

### Commit 3 — la porte

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.1 `rewind_url` extrait de `post` + son commentaire | 8 |
| § 2.4 la porte `preview` + les trois décisions commentées | 56 |
| § 2.5 attributs (ajout sur la porte ; les retraits ne coûtent aucune ligne ajoutée) | 2 |
| § 2.6 en-tête `//!` du module | 6 |
| helper de test `un_apercu(action)` | 26 |
| T5-T7 (les trois actions par la porte) | 22 |
| T8 (le corps sur la socket) | 20 |
| T9, T10 (422 et 200 bloqué) | 30 |
| T11 (aucune icône) | 16 |
| T12, T13 (422 illisible, 422 sans raison) | 26 |

Sous-total : 212

Total estimé : 338 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 338 contre 429 : 91 lignes de
marge, 21 % de la borne. Le précédent d'estimation le plus proche en forme sur
ce ticket — V2b-2b-1, un client HTTP neuf avec son factice et ses tests — avait
estimé 241 pour 279 livrées, soit **+16 %** ; appliqué ici, 338 estimées donnent
~392 réelles, encore sous la borne avec 37 lignes de marge. Le vol précédent,
V2c-1a-1, a livré un fichier de 416 lignes dont le plan prévoyait 239 pour son
commit 1 seul — mais ce chiffre n'est pas comparable, le fichier portant quatre
commits du même plan.

Deux raisons structurelles qui rendent cette marge plus sûre qu'un simple
pourcentage :

1. **Le factice et son témoin existent déjà** (`rewind_remote.rs:181-240`,
   60 lignes sur `main`). C'était le poste le plus lourd et le plus mal borné de
   V2c-1a-1 ; ce vol n'en paie que le helper `un_apercu`.
2. **Aucun fichier neuf, aucune déclaration de module, aucune entrée d'egress.**
   Les trois postes à surprise d'un vol de client sont tous déjà payés.

Les commentaires comptent dans cette estimation et ne se raboteront pas pour
tenir le chiffre : les ~48 lignes de commentaire des § 2.1 à 2.4 portent chacune
un pourquoi non lisible dans le code (le non-repli, F9, F4′, F14, les trois
décisions de la porte), au niveau du voisinage que ce fichier impose déjà. Si le
code écrit dépasse 429, le pilote s'arrête et le dit, conformément au ticket.

`Tracked in: senara-solutions/mika#1727`
