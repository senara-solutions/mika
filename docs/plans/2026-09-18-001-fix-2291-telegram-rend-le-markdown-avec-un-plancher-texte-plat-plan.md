---
issue: 2291
type: fix
---

# fix(mika#2291) — Telegram rend le markdown, avec un plancher texte-plat : un reconnaisseur, deux rendus, et un repli qui fait du pire cas le comportement d'aujourd'hui

## Symptôme mesuré (2026-09-11, tenant cloud d'Al)

Les réponses Telegram du tenant montrent le markdown **brut** : `**gras**` arrive
littéralement au lieu d'être rendu. Le ticket le classe p2 cosmétique, chemin démo
compagne, et le relie à mika#2247 (fuites de style). L'impact réel est celui-là :
une sortie non professionnelle vue par un invité.

## Ce que le code établit — et qui précise le corps du ticket

Le corps dit « le gateway n'applique pas le rendu Telegram (MarkdownV2 / HTML) **ou**
ne strippe pas ». Les deux moitiés sont vraies, mais la première n'est pas un oubli :
c'est une décision datée, motivée, et écrite dans le code avec sa condition de levée.

### F1 — `parse_mode` est absent **par décision**, pas par omission

`crates/mika-gateway/src/telegram.rs:311-335`, doc-comment de `SendMessagePayload` :

> **No `parse_mode` — and that is a decision, not an omission (mika#2126).**
> […] MarkdownV2 requires escaping `_ * [ ] ( ) ~ \` > # + - = | { } . !` throughout
> the *entire* text […] A single unescaped character makes the Telegram API reject the
> **whole message** with a 400. We would then have traded a broken link for an
> **absent message** — a clear regression […]
> **Changing this field means owning the escaping of every outbound message, including
> agent-authored text we do not control. Take it back through grooming, not here.**

**Ce plan est ce grooming.** La dernière phrase est une précondition procédurale, pas
une interdiction : elle demande que la levée soit argumentée. L'argument de mika#2126
est exact et reste vrai tel qu'énoncé — il porte sur `parse_mode` **sans repli**. Ce
plan ne le contredit pas, il en supprime la prémisse (voir Décision 2).

### F2 — Il existe déjà une transformation markdown, de périmètre *URL* seulement

`strip_markdown_around_urls` (`telegram.rs:430-435`) et ses quatre auxiliaires
(`rewrite_markdown_links`, `parse_markdown_link`, `strip_border_decoration`,
`strip_token_borders`, `telegram.rs:438-580`) réparent le cas
`**https://…/mika**` → lien vers `…/mika**` → 404. Son doc-comment pose l'ancrage :
« **The perimeter is *URLs*, not markdown rendering.** » Elle sort tôt quand le texte
ne porte aucun `http://` / `https://`.

C'est pourquoi `**important**` dans une phrase sans URL traverse intact : le ticket
mika#2291 n'est pas une régression de mika#2126, c'est le **complément** de son
périmètre, laissé ouvert et nommé.

### F3 — Point d'émission unique

`send_message_impl` (`telegram.rs:584-664`) est le **seul** appel `sendMessage` du
crate. `TelegramClient` et `CustomerTelegramClient` y convergent tous les deux ; les
~12 sites `let _ = tg.send_message(...)` en héritent. C'est la propriété que mika#2126
a construite délibérément (« every present and future caller inherits the cleaning
here ») et ce plan s'y branche sans la déplacer.

### F4 — Un test gelé s'oppose frontalement au ticket, et c'est intentionnel

`telegram.rs:1884` :

```rust
#[test]
fn test_strip_markdown_bold_text_without_url_unchanged() {
    // Out of scope on purpose: the perimeter is URLs, not markdown rendering (AC3).
    let input = "C'est **important** de le savoir.";
    assert_eq!(strip_markdown_around_urls(input), input);
}
```

Ce test pose exactement le symptôme de mika#2291 comme comportement attendu — **de la
fonction qu'il nomme**. Il reste vert dans ce plan (Décision 4) : le périmètre qui
s'élargit est celui du *pipeline*, pas celui de cette fonction.

### F5 — La contrainte qui décide de l'architecture : la préservation octet-pour-octet

`telegram.rs:1837-1840` ouvre une série de contrôles négatifs :

> **Negatives (AC3): a healthy message passes byte-for-byte unchanged** — A fix that
> rewrites correct URLs has repaired nothing: it has added a second way to break them.
> Each of these asserts `out == input`, not merely "looks ok".

Dont `test_strip_markdown_preserves_exact_whitespace_and_newlines` (`telegram.rs:1893`),
qui gèle `"Ligne un\n\n  Va voir  https://example.com/a\tfin"` **avec ses espaces
doubles et sa tabulation**. Cette contrainte est ce qui écarte un parser CommonMark —
voir Décision 3.

### F6 — Aucun parser markdown dans le workspace

Recherche sur tous les `Cargo.toml` et `Cargo.lock` : ni `pulldown-cmark`, ni `comrak`,
ni `markdown`. Le seul hit est `clap-markdown` (génération de doc CLI, inutilisable) et
`react-markdown` côté dashboard. Un parser serait une **nouvelle dépendance workspace**
(+ passage `deny.toml`).

### F7 — Un 400 aujourd'hui perd le message côté utilisateur

`send_message_impl:649-663` mappe 400 → `TelegramApiError::BadRequest { message }`.
`handle_send` (`routes.rs:2333-2336`) le traduit en **502 au corps vide** — la
`description` de Telegram (`"Bad Request: can't parse entities: …"`) est journalisée
mais **non propagée**. Côté agent, `GatewayMessageSender::send`
(`messaging.rs:194-224`) réessaie une fois à 2 s — ce qui, sur un 400 déterministe,
reproduit l'échec — puis `save_failed_send`, puis `flush_failed_sends` rejoue jusqu'au
`Drop` par âge. Rien n'est avalé côté journal ; **du point de vue de l'utilisateur
Telegram, la perte est totale et silencieuse.**

C'est exactement le mode de panne que mika#2126 refusait d'ouvrir. Toute la conception
ci-dessous est organisée autour de son élimination.

---

## Les décisions

### Décision 1 — Rendre, pas seulement stripper

Le ticket sanctionne les deux (« rendre […] **ou** le stripper proprement »). Ce plan
rend, pour une raison mesurable au symptôme : l'impact déclaré est *« sortie non
professionnelle vue par un invité »*, et un texte plat où l'agent avait mis de
l'emphase reste une sortie qui dit « cet assistant ne sait pas mettre en forme ». Le
strip ferme le symptôme littéral (`**` visibles) en laissant une version atténuée du
même défaut.

Mais le strip **n'est pas écarté** : il devient le plancher (Décision 2). Il est
construit d'abord, testé pour lui-même, et il est ce que le système fait quand le rendu
échoue ou qu'on le désarme.

### Décision 2 — HTML, jamais MarkdownV2 — et un repli qui supprime la prémisse de mika#2126

**HTML plutôt que MarkdownV2**, pour une raison de surface d'échappement : MarkdownV2
exige d'échapper dix-huit caractères sur **tout** le texte, URLs comprises ; le mode
HTML de Telegram n'en exige que trois — `<`, `>`, `&` — et uniquement hors balises.
La surface d'erreur est plus petite d'un ordre de grandeur, et elle est *locale* (on
échappe le contenu textuel, on émet les balises soi-même) au lieu d'être *globale*.

**Le repli est ce qui rend la décision tenable.** Sur un 400 alors que `parse_mode`
était posé, `send_message_impl` **réessaie une fois, sans `parse_mode`, avec le texte
plat**. Conséquence, qui est le cœur du plan :

> Le pire cas du chemin HTML est **exactement** le comportement d'aujourd'hui, moins
> les marqueurs bruts. Un message ne peut plus être perdu à cause d'un rendu.

L'objection de mika#2126 — *« we would have traded a broken link for an absent
message »* — portait sur `parse_mode` **nu**. Avec le repli, l'échange n'existe plus :
on troque un marqueur brut contre un marqueur brut. Ce plan ne réfute pas mika#2126,
il en retire la prémisse.

**Le déclencheur du repli est le status 400, jamais une sous-chaîne de la
`description`.** Le dépôt a déjà dû trancher cette classe (voir mika#2179 : les classes
d'erreur viennent de la *variante* `LlmError` via `downcast_ref`, « never from a
substring match on the rendered message »). Un 400 alors qu'on a posé `parse_mode` est
*par définition* un cas où retirer `parse_mode` ne peut pas nuire : si la cause était
ailleurs (chat introuvable, texte trop long), la seconde tentative échoue à
l'identique et on rend la seconde erreur — coût : un appel API, aucune dégradation.

### Décision 3 — Reconnaisseur ciblé écrit à la main, **pas** de parser CommonMark

F5 décide. La série de contrôles négatifs gèle une préservation **octet-pour-octet**,
espaces doubles et tabulation compris. Un aller-retour CommonMark (parse → AST →
rendu) ne peut pas la tenir : il normalise l'espacement, réinterprète les indentations
de quatre espaces en blocs de code, `1. ` en liste ordonnée, `---` en séparateur — il
*sur-interprète* de la prose de conversation. Ce n'est pas une préférence de style :
c'est une propriété testée que la dépendance ferait rougir.

S'y ajoute F6 (nouvelle dépendance workspace + `deny.toml`) et le précédent de la
maison : mika#2126 a écrit ses cinq fonctions à la main, « infallible by construction.
No `Result`, no `unwrap`, no panic, no raw byte indexing », et elles portent 25 tests.

**Critère de révision, écrit pour ne pas être redécouvert :** si un jour la sortie doit
porter des tableaux, des listes imbriquées ou des blocs de citation structurés, le
reconnaisseur ciblé n'est plus le bon outil et un parser devient justifiable — mais il
faudra alors renégocier explicitement la contrainte de préservation octet-pour-octet,
car elle et lui sont incompatibles.

### Décision 4 — Un reconnaisseur, deux rendus

La propriété structurelle qui fait tenir l'ensemble : **une seule reconnaissance, deux
émissions**.

```
tokenize(text) -> Vec<Segment>        // reconnaissance, une fois
    render_html(&segments)  -> String // mode armé
    render_plain(&segments) -> String // plancher, repli, et mode désarmé
```

`Segment` est **plat**, non imbriqué : `{ kind, text }` avec
`kind ∈ {Plain, Bold, Italic, Code, Pre, Strike, Link{url}}`. Telegram accepte
l'imbrication ; la prose d'agent n'en produit pratiquement pas, et le plat garde le
transformateur total et testable.

Trois conséquences, toutes désirables :

1. **Le désarmement et le repli produisent le même octet.** Le kill-switch n'est pas un
   chemin de code parallèle non testé : c'est le chemin du repli, exercé par ses propres
   tests. Un rollback en production emprunte un chemin qui tourne déjà.
2. **AC3 est structurel.** Si `tokenize` rend un unique `Plain` couvrant toute l'entrée,
   `render_plain` restitue l'entrée octet-pour-octet — par construction, pas par
   coïncidence de tests. La préservation de F5 devient une propriété du type.
3. **Les deux rendus ne peuvent pas diverger sur ce qu'ils reconnaissent**, seulement
   sur ce qu'ils émettent. C'est la forme que le dépôt a dû imposer deux fois par des
   gardes structurelles (mika#2158 « un seul lecteur du verdict de grooming »,
   mika#2363 « un seul prédicat ») ; ici elle est obtenue par la signature.

**`strip_markdown_around_urls` n'est ni retirée ni modifiée.** `render_plain` passe par
elle en second temps (voir Implémentation §3). Ses 25 tests restent verts et gardent
leur sens : ils épinglent l'auxiliaire *URL*, et une régression du nouveau
reconnaisseur ne peut pas rouvrir mika#2126 **sur le chemin plat** — le mode armé, lui,
ne la traverse pas et tient le défaut fondateur par le reconnaisseur seul (voir la
portée exacte du filet en Brique 2, et R5). Le test F4 reste vert **et reçoit un
commentaire** disant ce qu'il épingle désormais (la fonction, pas le pipeline), avec un
test frère au niveau pipeline assertant que `**important**` **change** maintenant. Un
test gelé qu'on laisse vert sans dire qu'il a changé de portée est un test qui ment
plus tard.

### Décision 5 — Kill-switch global, lu une fois, non hot-swappable

`MIKA_TELEGRAM_HTML_RENDER`, défaut **armé**. Analyse trois paliers de la maison :
absent/vide → défaut ; illisible → défaut + WARN nommant la valeur **entre guillemets**
(mika#2220 : sans les guillemets, une espace parasite est invisible) ; `0` / `false` /
`off` / `no` (insensible à la casse) → désarmé.

**Armé par défaut**, contrairement à la prudence réflexe, et pour une raison mesurée
ailleurs : mika#2272 a dû constater qu'un détecteur livré désarmé derrière une
condition de bascule produit un compteur qui ne peut *structurellement* pas bouger —
« zéro était l'absence de mesure, pas la présence de prudence ». Ici la prudence est
payée par le repli (Décision 2), qui est un mécanisme et non une intention, et par
l'événement de repli qui rend le taux d'échec comptable dès le premier jour.

**Mécanisme : `OnceLock<bool>` dans le module, initialisé une fois depuis `Settings`
dans `main.rs`, défaut armé s'il n'est jamais initialisé** (binaires de test, sites de
construction de `github.rs`). Contrat identique à `MIKA_AGENT_TIER` et
`MIKA_DEPLOYMENT` : lu une fois par process, non hot-swappable, à poser dans
l'EnvironmentFile / ConfigMap **avant** le démarrage.

**Le coût est nommé :** c'est un global, et un global est une odeur. L'alternative — un
champ sur les deux structs client — se paie à 7 sites de construction non-test
(`main.rs:106`, `orchestrator_inbox.rs:525`, `routes.rs:476,572,1106,2252,2277`) et,
surtout, **ne donne pas un vrai kill-switch** : les sites « fire-and-forget » du chemin
entrant (`routes.rs:659,674,732,806,1709,…`) garderaient le défaut et échapperaient au
désarmement. Un interrupteur qui n'éteint qu'une partie du circuit n'est pas un
interrupteur. Le global est acheté pour cette propriété-là, et pour aucune autre.

---

## Le correctif

### Brique 1 — `crates/mika-gateway/src/telegram_markdown.rs` (module neuf, privé au binaire)

Module neuf plutôt qu'ajout à `telegram.rs` (déjà ~1930 lignes) : il obtient son propre
`#[cfg(test)] mod tests`, et reste privé au binaire — **`lib.rs` n'est pas touché**, ce
que son propre commentaire exige (« Do NOT add unrelated modules here », mika#1796).
Les tests unitaires du binaire tournent sous `cargo test -p mika-gateway`.

**`tokenize(text: &str) -> Vec<Segment>`** — infaillible par construction, au même
standard que mika#2126 : pas de `Result`, pas d'`unwrap`, pas de panic, aucune
indexation d'octets brute (tout le découpage se fait sur `Vec<char>`).

Reconnu, et rien d'autre :

| Forme | Segment | Condition |
|---|---|---|
| `**gras**` / `__gras__` | `Bold` | run apparié, contenu non vide |
| `*ital*` | `Italic` | run apparié, contenu non vide |
| `_ital_` | `Italic` | run apparié **et** non intra-mot (voir ci-dessous) |
| `~~barré~~` | `Strike` | run apparié |
| `` `code` `` | `Code` | run apparié |
| ` ```…``` ` | `Pre` | clôture présente ; l'info-string éventuelle est retirée |
| `[label](url)` | `Link` | délégué à la grammaire déjà validée de `parse_markdown_link` |
| `# ` … `###### ` en début de ligne | `Plain` sans le préfixe | 1 à 6 `#` suivis d'une espace |
| `* ` / `+ ` en début de ligne | `Plain` avec `- ` | puce, normalisée |

**La garde intra-mot sur `_` est porteuse, pas cosmétique.** Sans elle,
`mon_fichier_test` devient `monfichiertest` : une corruption de données utilisateur
livrée par un correctif cosmétique. Règle retenue, celle de CommonMark : `_` n'est
délimiteur d'emphase que lorsqu'il n'est pas bordé d'alphanumériques des deux côtés.
Contrôle négatif obligatoire (§Contrat de vérification, test N7).

Tout ce qui n'est pas dans ce tableau — `> citation`, `1. ` liste ordonnée, `---`,
indentation de quatre espaces, tableaux — **reste `Plain` intact**. C'est l'application
directe de la doctrine AC3 de mika#2126 : un correctif qui réécrit un message sain n'a
rien réparé, il a ajouté une seconde façon de le casser.

**`render_plain(&[Segment]) -> String`** — concatène les `text`. `Link` rend
`label : url` (forme exacte de `rewrite_markdown_links`, préservée à dessein : elle est
gelée par les tests de mika#2126 et le pipeline la produit déjà aujourd'hui ; une
seconde écriture du même fait serait une divergence en attente). Label vide ou identique
à l'URL → l'URL nue, là encore comme aujourd'hui.

**`render_html(&[Segment]) -> String`** — échappe `<`, `>`, `&` dans **chaque** `text`
(y compris `Code` et `Pre` — Telegram l'exige), puis enrobe : `<b>`, `<i>`, `<code>`,
`<pre>`, `<s>`, `<a href="…">`. L'URL d'un `Link` est échappée de la même façon dans
l'attribut.

> **Un message sain n'est pas inchangé en mode HTML s'il contient `&`, `<` ou `>`** —
> il devient `&amp;`, `&lt;`, `&gt;`. Ce n'est pas une violation d'AC3 : le client
> Telegram restitue le caractère d'origine, donc **ce que l'utilisateur voit** est
> inchangé. AC3 s'applique au rendu perçu, et le contrôle octet-pour-octet reste posé,
> lui, sur `render_plain`.

### Brique 2 — repli dans `send_message_impl` (`telegram.rs:584`)

```
segments = tokenize(text)

if html_render_enabled() {
    body = render_html(&segments)
    match post(body, parse_mode = Some("HTML")) {
        Ok(id) => return Ok(id),
        Err(BadRequest { message }) => {
            warn!(… event = "telegram_html_render_fallback", description = %message, …)
            // fall through
        }
        Err(other) => return Err(other),
    }
}

// plancher : mode désarmé ET chemin de repli — même octet
body = strip_markdown_around_urls(&render_plain(&segments))
post(body, parse_mode = None)
```

Trois propriétés à tenir, chacune testée :

- **Le repli ne tire que sur 400.** 401 / 403 / 429 / 5xx remontent tels quels : les
  rejouer sans `parse_mode` doublerait un appel voué à échouer et, sur 429,
  aggraverait la limitation.
- **Un seul réessai.** Le second envoi n'a pas de repli ; son erreur est rendue. Le
  réessai à 2 s de `GatewayMessageSender` (`messaging.rs:194-224`) reste en amont et
  n'est pas touché.
- **`strip_markdown_around_urls` est conservée en second passage — et seulement sur le
  chemin plat.** Après `render_plain`, les marqueurs reconnus ont disparu, donc elle ne
  trouve presque jamais rien à faire (elle sort tôt sans schéma d'URL). Elle reste pour
  le résidu que son propre doc-comment nomme — `_texte https://url_`, décoration non
  appariée au niveau du token — que le reconnaisseur laisse volontairement `Plain`.
  Coût : un passage sur une chaîne.

  > **Portée exacte du filet, écrite parce qu'elle est plus étroite qu'elle n'en a
  > l'air.** Le bénéfice « une régression du reconnaisseur ne peut pas rouvrir
  > mika#2126 » vaut pour le **plancher**, pas pour le mode armé : la branche HTML
  > poste `render_html(&segments)` sans repasser par `strip_markdown_around_urls`.
  > Or le mode armé est le **défaut**. En mode HTML, ce qui tient le défaut fondateur
  > n'est pas le filet, c'est le reconnaisseur lui-même : une décoration appariée
  > autour d'une URL devient une balise, donc la borne du lien est la balise et non un
  > `*` collé à l'URL (R4) ; une décoration non appariée reste `Plain`, donc l'URL
  > traverse intacte (N3). **Ces deux propriétés sont ce qui remplace le filet sur le
  > chemin armé, et R5 les contrôle pour elles-mêmes.**
  >
  > Faire tourner `strip_markdown_around_urls` **aussi** avant `render_html` serait le
  > réflexe symétrique, et il est écarté : elle réécrit `[label](url)` en
  > `label : url`, ce qui détruirait le `Link` que `render_html` doit rendre en
  > `<a href>`. Les deux transformations se recouvrent au lieu de se composer — d'où
  > un seul reconnaisseur en amont (Décision 4) et le filet en aval du seul rendu qui
  > le tolère.

Le `debug!` de métriques existant (`telegram.rs:598-610`) est conservé tel quel.

### Brique 3 — `parse_mode` sur le payload

`SendMessagePayload` gagne `#[serde(skip_serializing_if = "Option::is_none")] parse_mode: Option<&'static str>`.
`&'static str` plutôt que `String` : les seules valeurs possibles sont `"HTML"` et
l'absence, et le type interdit d'en inventer une troisième au site d'appel.

**Le doc-comment de mika#2126 n'est pas supprimé : il est réécrit.** Son argument reste
valide et doit rester lisible — ce qui change est que sa prémisse (`parse_mode` nu) ne
décrit plus le code. La réécriture dit : pourquoi HTML et pas MarkdownV2, pourquoi le
repli supprime l'échange dénoncé, et où est le kill-switch. Effacer le raisonnement
rendrait la prochaine tentative de passage à MarkdownV2 aussi coûteuse qu'avant.

### Brique 4 — réglage + documentation

- `crates/mika-gateway/src/settings.rs` : champ `telegram_html_render: bool`, défaut
  `true`, analyse trois paliers.
- `crates/mika-gateway/src/main.rs` : initialisation unique du `OnceLock` après
  `Settings::load`.
- `.env.example` : la variable, son défaut, son sens.
- `crates/mika-gateway/CLAUDE.md` : **c'est la dette que ce ticket paie au passage.**
  Ce fichier est aujourd'hui muet sur l'absence de `parse_mode`, sur
  `strip_markdown_around_urls` et sur mika#2126 — ces trois décisions ne vivent que
  dans des doc-comments, ce qui est probablement la raison pour laquelle « pas de
  parse_mode » n'était pas découvrable sans lire le code, et donc pour laquelle ce
  ticket a été ouvert comme un oubli. Une sous-section « Rendu du texte sortant » : le
  mode HTML, le repli, le kill-switch, et le renvoi aux doc-comments.
- `CLAUDE.md` racine, § variables d'environnement : la variable et son contrat
  read-once.

---

## Implémentation, par fichier

| # | Fichier | Nature |
|---|---|---|
| 1 | `crates/mika-gateway/src/telegram_markdown.rs` | **neuf** — `Segment`, `tokenize`, `render_plain`, `render_html`, `html_render_enabled()`, `init_html_render()`, `#[cfg(test)] mod tests` |
| 2 | `crates/mika-gateway/src/main.rs` | `mod telegram_markdown;` + initialisation du `OnceLock` |
| 3 | `crates/mika-gateway/src/telegram.rs` | `SendMessagePayload.parse_mode` ; doc-comment réécrit ; `send_message_impl` : tokenize + deux rendus + repli ; **`strip_markdown_around_urls` et ses quatre auxiliaires inchangés** ; commentaire de portée sur le test F4 + test frère pipeline |
| 4 | `crates/mika-gateway/src/settings.rs` | champ `telegram_html_render` |
| 5 | `.env.example` | la variable |
| 6 | `crates/mika-gateway/CLAUDE.md` | § « Rendu du texte sortant » |
| 7 | `CLAUDE.md` (racine) | la variable dans la liste |

**Aucune nouvelle dépendance** (Décision 3) — donc aucun passage `deny.toml`.
**`lib.rs` n'est pas touché** (mika#1796).

---

## Contrat de vérification

Tous les tests sont unitaires et purs, dans `telegram_markdown.rs` sauf mention. Aucun
mock HTTP : `api_url` code en dur `https://api.telegram.org` et le rendre injectable
serait un élargissement hors sujet (voir Hors périmètre).

### Positifs — reconnaissance et rendu

| # | Entrée | `render_plain` | `render_html` |
|---|---|---|---|
| P1 | `C'est **important** de le savoir.` | `C'est important de le savoir.` | `C'est <b>important</b> de le savoir.` |
| P2 | `un *mot* ital` | `un mot ital` | `un <i>mot</i> ital` |
| P3 | `` code `foo()` ici `` | `code foo() ici` | `code <code>foo()</code> ici` |
| P4 | `~~annulé~~` | `annulé` | `<s>annulé</s>` |
| P5 | `[le dépôt](https://example.com/a)` | `le dépôt : https://example.com/a` | `<a href="https://example.com/a">le dépôt</a>` |
| P6 | `# Titre\ncorps` | `Titre\ncorps` | `Titre\ncorps` |
| P7 | `* premier\n* second` | `- premier\n- second` | `- premier\n- second` |
| P8 | ```` ```rs\nlet x = 1;\n``` ```` | `let x = 1;` | `<pre>let x = 1;</pre>` |

**P9 — le cas fondateur du ticket** (fixture gelée, nommée
`mika2291_reported_case_raw_bold_is_no_longer_visible`) : l'entrée porte `**gras**` ; le
rendu HTML ne contient **aucun** `*` et le rendu plat non plus. C'est l'assertion qui
décrit le symptôme mesuré et qui rougira si le correctif est défait.

### Négatifs — AC3, octet-pour-octet sur `render_plain`

Chacun asserte `render_plain(tokenize(x)) == x`, pas « a l'air correct ».

| # | Entrée | Pourquoi |
|---|---|---|
| N1 | `Bonjour Sonia 🌸` | message sain, multi-octets |
| N2 | `Ligne un\n\n  Va voir  https://example.com/a\tfin` | **espaces doubles + tabulation préservés** — le test qui écarte le parser CommonMark (F5) |
| N3 | `https://example.com/path_with_underscore_` | `_` final non apparié, légal en URL (KTD3 de mika#2126) |
| N4 | `2 * 3 * 4 = 24` | `*` arithmétique : runs non appariés au sens de l'emphase |
| N5 | `> une citation` | non reconnu à dessein |
| N6 | `1. premier\n2. second` | liste ordonnée non reconnue à dessein |
| N7 | `mon_fichier_test.rs` | **garde intra-mot** — la corruption que la Décision 3 nomme |
| N8 | `` ` `` seul, `**` seul, `[label](` tronqué | runs non clos → intacts, aucun panic |
| N9 | `""` | vide |
| N10 | `Éh 🌸 https://example.com/été — ça va ?` | aucune indexation d'octets brute (KTD6) |
| N11 | `[x](https://example.com/a b)` | **grammaire de lien refusée** — `parse_markdown_link` rend `None` sur une URL portant une espace (gelé par `test_strip_markdown_link_with_space_in_url_unchanged`, `telegram.rs:1909`). `tokenize` doit alors laisser le texte `Plain`, **dans les deux rendus** : un `<a href>` posé sur une forme que la grammaire a refusée serait un lien que le reconnaisseur aurait inventé. |

**N12 — le contrôle porte sur ce qui part, pas sur une fonction intermédiaire.** Les dix
premiers négatifs asserted `render_plain(tokenize(x)) == x`, mais le chemin plat émet
`strip_markdown_around_urls(render_plain(tokenize(x)))`. Tant que le contrôle s'arrête
au milieu, AC3 est vraie d'une valeur que l'utilisateur ne reçoit jamais. N12 rejoue
donc **N1–N11 à travers la composition complète** et asserte la même égalité
octet-pour-octet. Le coût est d'une boucle ; le bénéfice est que la seule composition
capable de réécrire un message sain — deux transformateurs dont chacun préserve, mis
bout à bout — cesse d'être un angle mort. Son échec serait d'ailleurs un signal précis :
il ne dirait pas « un des deux est faux », il dirait « ils se recouvrent », ce qui est
la question ouverte de la Brique 2.

### Échappement HTML

| # | Assertion |
|---|---|
| H1 | `a < b & c > d` → `a &lt; b &amp; c &gt; d`, aucune balise |
| H2 | `` `if (a<b) {}` `` → `<code>if (a&lt;b) {}</code>` — échappement **à l'intérieur** de `code` |
| H3 | `[a&b](https://x/?q=1&r=2)` → `&` échappé dans l'attribut `href` **et** dans le label |
| H4 | `<b>déjà</b>` écrit par l'agent → `&lt;b&gt;déjà&lt;/b&gt;` (littéral, pas du gras) — injection de balise impossible |

**H4 est une propriété de sécurité, pas de cosmétique.** Sans elle, un texte
d'utilisateur relayé par l'agent pourrait poser des entités Telegram arbitraires.

### Propriétés structurelles

| # | Assertion |
|---|---|
| S1 | **Identité du plancher** : pour un corpus de ~20 entrées, la sortie du mode désarmé et celle du chemin de repli sont **octet-identiques**. C'est ce qui fait du rollback un chemin déjà testé (Décision 4.1). |
| S2 | `tokenize` ne perd ni ne duplique aucun caractère non-marqueur : la concaténation des `text` des segments, pour une entrée sans lien, est l'entrée privée de ses seuls marqueurs reconnus. |
| S3 | Aucun `unwrap`, `expect`, `panic!` ni indexation `&text[i..j]` dans `telegram_markdown.rs` — scan de source, au modèle de `mika2131_exclusion_skips_never_return_to_an_uncollected_debug`. Un test comportemental ne peut pas voir cette classe : la régression ne rendrait pas une sortie fausse, elle rendrait un envoi **panicable**. |

### Réglage

| # | Assertion |
|---|---|
| C1 | absent / vide → armé |
| C2 | `0`, `false`, `FALSE`, `off`, `no` → désarmé |
| C3 | `plif` → armé + WARN nommant `"plif"` **entre guillemets** |
| C4 | jamais initialisé (binaire de test) → armé, sans panic |

### Non-régression mika#2126 — `telegram.rs`

| # | Assertion |
|---|---|
| R1 | Les **25 tests existants** de `strip_markdown_around_urls` restent verts, inchangés. Population nommée pour être recomptable : la famille `test_strip_markdown_*` de `telegram.rs`, aujourd'hui lignes 1758–1922 (`grep -c "fn test_strip_markdown" crates/mika-gateway/src/telegram.rs` → 25). Un chiffre qu'on ne peut pas recompter est une AC qu'on ne peut pas vérifier. |
| R2 | `test_strip_markdown_bold_text_without_url_unchanged` (F4) reste vert et reçoit un commentaire disant ce qu'il épingle désormais : la fonction, pas le pipeline. |
| R3 | Test frère neuf, au niveau pipeline : `C'est **important** de le savoir.` **change** maintenant. R2 et R3 côte à côte sont la trace lisible du déplacement de périmètre. |
| R4 | `**https://example.com/a**` : en mode HTML le lien cliqué est `https://example.com/a` (le gras devient `<b>`, la borne du lien est la balise) ; en mode plat, idem via le second passage. Le défaut fondateur de mika#2126 est clos **dans les deux modes**. |
| R5 | **Le mode armé tient mika#2126 sans le filet.** Le corpus URL de mika#2126 — le cas fondateur, les bornes appariées, la décoration non appariée, l'`_` final légal en URL, le lien à URL espacée (N11) — est rejoué **contre `render_html`**, en assertant que l'URL cliquable reste exactement l'URL d'origine. C'est le contrôle que R1 ne peut pas donner : R1 vérifie que l'auxiliaire *URL* est intact, or le chemin par défaut ne l'appelle pas. Sans R5, la non-régression de mika#2126 ne serait mesurée que sur le chemin de repli — celui qui, en régime nominal, ne tourne jamais. |

---

## Surfaces opérateur

Journal `$MIKA_GATEWAY_LOG_FILE` (ou stdout) :

- **`telegram_html_render_fallback`** (WARN) — champs `chat_id`, `description`
  (la `description` de Telegram, F7, qui n'était jusqu'ici visible nulle part
  d'exploitable), `len_html`, `len_plain`. **Régime attendu : zéro ligne.** Toute
  occurrence est un message que le rendu HTML a cassé et que le repli a sauvé — donc à
  la fois une preuve que le filet fonctionne et un cas de reconnaisseur à corriger. Le
  corps du message n'est **jamais** journalisé (donnée utilisateur), au même standard
  que le `debug!` de mika#2126.
- **`telegram_html_fallback_failed`** (WARN) — le second envoi, sans `parse_mode`, a
  échoué lui aussi. **Régime attendu : zéro ligne.** Cette population est celle où
  l'utilisateur perd effectivement le message ; sans cet événement, elle serait
  indistinguable d'un 502 ordinaire.
- **`telegram_html_render_disabled`** (INFO, une fois au démarrage, seulement si
  désarmé) — sinon le désarmement est invisible et un opérateur lisant « zéro repli »
  conclurait que le rendu marche alors qu'il ne tourne pas. Le silence d'un détecteur
  désarmé ressemble trait pour trait au silence d'un détecteur sain (doctrine
  mika#2205).

Pas d'`audit_events` : la table est côté agent (SQLite) et côté gateway
(`audit_events.rs`, Postgres, périmètre webhook). Un événement par envoi y serait du
churn — le journal suffit pour une population attendue vide.

---

## Sonde post-déploiement, avec sa halte

1. **Le symptôme, directement.** Sur un tenant cloud, demander une réponse contenant de
   l'emphase et vérifier dans Telegram que le gras **s'affiche** et qu'aucun `*` n'est
   visible. C'est la sonde qui répond au ticket ; les autres mesurent le coût.
2. **Taux de repli, 48 h.** `grep telegram_html_render_fallback` — attendu **zéro**.
   Quelques lignes isolées : lire la `description`, corriger le reconnaisseur, ne pas
   désarmer (le filet fait son travail). **Halte à un taux soutenu (> 1 % des envois) :
   désarmer par `MIKA_TELEGRAM_HTML_RENDER=0` et corriger le reconnaisseur avant de
   réarmer** — un filet qui porte le trafic nominal n'est plus un filet, et il masque
   le signal qui permettrait de voir la panne (doctrine mika#2334).
3. **Perte réelle, 48 h.** `grep telegram_html_fallback_failed` — attendu **zéro**.
   Toute ligne est un utilisateur qui n'a rien reçu : traiter avant le point 2.
4. **Contrôle négatif.** Un message sans aucun markdown (accusé de réception, message
   système de `routes.rs`) doit arriver **inchangé**. S'il change, le reconnaisseur
   sur-interprète et c'est la classe AC3 que mika#2126 a payée une fois.
5. **Halte explicite.** Si le markdown brut **réapparaît** alors que
   `telegram_html_render_fallback` est vide : ne pas élargir le reconnaisseur. Cela
   signifie que le texte est parti par un chemin qui ne traverse pas
   `send_message_impl` — et établir lequel passe avant toute correction. F3 dit qu'il
   n'y en a pas aujourd'hui ; un tel constat serait donc d'abord une information sur
   l'architecture, pas sur le rendu.

---

## Hors périmètre, délibérément

- **La garde miroir 4096 au gateway.** `mika-common/src/telegram.rs:3-4` affirme que
  « `mika-gateway` s'en sert pour refuser le texte tel qu'il partira, préfixe compris »
  et le plan mika#2134 la prévoyait — **elle n'existe pas** : `handle_send` ne contrôle
  que `text.len() > 50_000` octets (`routes.rs:2193`). Un texte de 4090 caractères plus
  le préfixe `[mika-dev] ` part et prend un 400. C'est un défaut réel, trouvé en
  chemin, **sans rapport avec le rendu** : moitié inachevée de mika#2134.
  **Ticket de suivi à ouvrir.** Ce plan n'en dépend pas — et le repli le couvre même
  s'il se manifeste, puisqu'un 400 pour longueur échoue identiquement au second envoi
  et rend la seconde erreur.
  > Note sur la longueur : la doc Bot API compte `text` « 1-4096 characters **after
  > entities parsing** », donc les balises HTML ne comptent pas et le mode HTML
  > n'ajoute aucun risque de longueur. **À vérifier à l'implémentation contre la doc
  > courante** — et la conception est robuste si la réponse est l'inverse : un
  > dépassement donnerait un 400, donc le repli, donc le texte plat, qui est **toujours
  > plus court que l'entrée brute**.
- **La propagation de la `description` du 400 jusqu'à l'agent.** `handle_send` rend un
  502 au corps vide (F7). L'améliorer changerait un contrat HTTP et le nouvel événement
  WARN rend déjà la `description` lisible côté gateway, là où on diagnostique.
- **Rendre `api_url` injectable pour un `MockServer`.** Ce serait le seul moyen de
  tester le repli de bout en bout sur HTTP. Élargissement de surface pour de
  l'observabilité de test seule ; le repli est testable en découplant la décision
  (« 400 + parse_mode posé → rejouer en plat ») de l'appel, et c'est ce que fait S1.
  **Ticket de suivi possible** si le repli devient une population non vide.
- **Le découpage des messages > 4096.** Reste à l'agent (`send_message.rs:57-67`), par
  la décision mika#2134 : le gateway insère une ligne `outbound_messages` par envoi et
  le routage des réponses dépend de cette relation 1:1.
- **Demander à l'agent de ne pas écrire de markdown.** Écarté sans hésitation :
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` (mika#2120 :
  neuf récurrences sous application par prompt contre zéro quand c'était écrit à la
  main). Le markdown est le registre par défaut d'un LLM ; le refuser par consigne est
  une garantie qui dure le temps de la mémoire de celui qui écrit la consigne.
- **MarkdownV2.** Écarté par la Décision 2 et rien dans ce plan ne le rapproche.
- **mika#2247 (fuites de style)**, que le ticket relie : parenté de symptôme, pas de
  cause. Rien ici ne le referme.
- **Les autres canaux.** Telegram est la seule sortie utilisateur du gateway ; `voice/`
  est une surface de types sans transport (mika#1796).

---

## Risques

| # | Risque | Portée | Atténuation |
|---|---|---|---|
| 1 | Le reconnaisseur **sur-interprète** et abîme un message sain | La classe qu'AC3 de mika#2126 existe pour fermer | Tableau de reconnaissance fermé (Décision 4) ; 11 contrôles négatifs octet-pour-octet, rejoués à travers la composition réellement émise (N12) ; S2 |
| 2 | Corruption d'identifiants par `_` intra-mot | `mon_fichier_test` → `monfichiertest` | Garde CommonMark intra-mot, test N7 dédié |
| 3 | Un 400 dû au rendu perd le message | Le mode de panne que mika#2126 refusait | **Le repli (Décision 2)** — pire cas = comportement d'aujourd'hui ; deux événements WARN ; kill-switch |
| 4 | Injection de balise par du texte relayé | Sécurité | Échappement systématique de `< > &`, test H4 |
| 5 | Divergence entre les deux rendus | Le désarmement emprunterait un chemin non testé | Un seul reconnaisseur (Décision 4) ; S1 asserte l'identité repli/désarmé |
| 6 | Régression de mika#2126 | Liens cassés, déjà payés une fois | `strip_markdown_around_urls` inchangée + conservée en second passage (chemin plat) ; **R5 pour le chemin HTML, qui ne la traverse pas** ; R1–R4 |
| 7 | Le `OnceLock` global | Odeur architecturale | Assumée et nommée (Décision 5) ; achetée contre la propriété « le kill-switch éteint tout le circuit » |
| 8 | La sémantique de longueur 4096 est autre que supposée | Un long message en HTML prendrait un 400 | Le repli couvre ; vérification explicite à l'implémentation ; le texte plat est toujours plus court que l'entrée |

---

## Fire-Disposition

Exigée par la porte mika#1574
(`docs/solutions/best-practices/fire-disposition-doctrine.md`) : ce plan introduit des
livrables de classe détecteur ; la doctrine demande ce que fait l'implémentation quand
le détecteur tire sur les données **existantes** — les violations préexistantes, non le
code neuf.

| # | Détecteur | Population existante | Option |
|---|---|---|---|
| D1 | Chemin de repli 400 → texte plat (Brique 2) | **Non vide** — tout message sortant après déploiement | **(a) Auto-remédiation silencieuse**, détail ci-dessous |
| D2 | Garde structurelle S3 (`unwrap` / `panic!` / indexation d'octets dans `telegram_markdown.rs`) | **Vide par construction** — le fichier n'existe pas avant ce ticket | Sans objet, zéro exception, **pas d'allowlist** |
| D3 | Contrôles négatifs N1–N12, positifs P1–P9, échappement H1–H4, non-régression R5 | **Vide** — les fixtures sont écrites par ce ticket | Sans objet, zéro exception |
| D4 | WARN de valeur illisible du kill-switch (C3) | **Vide** — la variable n'existe pas avant ce ticket | Sans objet |

### D1 — option (a), et pourquoi le silence est ici le bon choix

Le repli **remédie sans escalader** : il réémet en texte plat et l'utilisateur reçoit
son message. C'est délibérément le seul détecteur de ce plan qui ne halte pas, pour une
raison qui est l'objet même du ticket : **halter sur un défaut de rendu, ce serait
choisir l'absence de message plutôt que le message mal formé** — exactement l'échange
que mika#2126 a refusé et que la Décision 2 supprime.

« Silencieuse » qualifie le chemin utilisateur, **pas** la surface opérateur : chaque
tir écrit `telegram_html_render_fallback` avec la `description` de Telegram, et la
sonde post-déploiement n°2 porte la halte — au-delà de 1 % d'envois, on désarme et on
corrige. La remédiation est donc silencieuse **pour l'utilisateur** et comptée **pour
l'opérateur**, ce qui est la seule répartition compatible avec un défaut p2 cosmétique
sur le chemin démo.

---

## Note zone

Zone **gateway / sortie Telegram**. Deux tickets antérieurs y ont posé des décisions
que ce plan touche, et aucun des deux n'est contredit :

- **mika#2126** — `strip_markdown_around_urls`, absence de `parse_mode`. Ses cinq
  fonctions et ses 25 tests sont **inchangés**. Sa décision est levée par la procédure
  qu'elle prescrivait elle-même (« take it back through grooming »), et son argument
  reste écrit dans le code réécrit.
- **mika#2134** — limite 4096, découpage côté agent. Inchangé. Sa moitié inachevée (la
  garde miroir) est nommée hors périmètre avec un ticket de suivi.

Dette documentaire payée au passage : `crates/mika-gateway/CLAUDE.md` était muet sur les
trois décisions de rendu, qui ne vivaient que dans des doc-comments — vraisemblablement
la raison pour laquelle mika#2291 a été ouvert en lisant « le gateway n'applique pas le
rendu » comme un oubli.

---

## Definition of Done

1. `crates/mika-gateway/src/telegram_markdown.rs` existe : `Segment`, `tokenize`,
   `render_plain`, `render_html`, lecture du kill-switch.
2. `SendMessagePayload` porte `parse_mode: Option<&'static str>`, omis à la
   sérialisation quand `None` ; son doc-comment est réécrit (argument mika#2126
   conservé, prémisse corrigée, kill-switch nommé).
3. `send_message_impl` tokenize une fois, rend en HTML si armé, **replie sur 400** vers
   le texte plat sans `parse_mode`, et ne replie que sur 400.
4. `strip_markdown_around_urls` et ses quatre auxiliaires sont **inchangés** et
   conservés en second passage du rendu plat ; leurs 25 tests sont verts.
5. `MIKA_TELEGRAM_HTML_RENDER` : `Settings`, analyse trois paliers, défaut armé, lu une
   fois au démarrage, documenté dans `.env.example` et les deux `CLAUDE.md`.
6. Les trois événements opérateur sont émis, sans jamais journaliser le corps d'un
   message.
7. Tous les tests du contrat de vérification passent : P1–P9, N1–N12, H1–H4, S1–S3,
   C1–C4, R1–R5.
8. `cargo build`, `cargo test`, `cargo clippy` (zéro warning), `cargo fmt --check`
   verts sur le workspace.
9. Aucune dépendance ajoutée ; `crates/mika-gateway/src/lib.rs` inchangé.
10. `make verify-bundled-skills` et `scripts/verify-pipeline.sh` verts.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés du symptôme mesuré, du « Fix » proposé, et du contrat de
vérification.

- **AC1 — Le markdown n'arrive plus brut.** Un message d'agent contenant `**gras**`,
  `*ital*`, `` `code` ``, `~~barré~~` ou `[label](url)` arrive dans Telegram **rendu**
  (mode armé) ou **propre, sans marqueur visible** (mode désarmé). Dans aucun des deux
  modes un `*`, `_`, `~` ou `` ` `` de décoration reconnue n'est visible par
  l'utilisateur. Épinglé par P1–P9, dont la fixture gelée du cas fondateur (P9).

- **AC2 — Aucun message ne peut être perdu à cause du rendu.** Sur un 400 alors que
  `parse_mode` était posé, le gateway réémet une fois sans `parse_mode` avec le texte
  plat. Le repli ne tire que sur 400 ; 401 / 403 / 429 / 5xx remontent inchangés ; le
  second envoi n'a pas de repli. Épinglé par les tests du chemin de repli et par S1.

- **AC3 — Un message sain traverse sans être réécrit.** `render_plain(tokenize(x)) == x`
  octet-pour-octet pour les onze entrées N1–N11 — espaces doubles, tabulations,
  multi-octets, `_` intra-mot, runs non clos et grammaire de lien refusée comprises —
  **et la même égalité tient à travers la composition réellement émise**
  `strip_markdown_around_urls(render_plain(tokenize(x)))` (N12). En mode HTML, seul
  l'échappement de `<`, `>`, `&` diffère, et le rendu **perçu** est identique.

- **AC4 — Aucune balise ne peut être injectée.** Tout `<`, `>`, `&` du texte est
  échappé, y compris dans `code`, `pre` et l'attribut `href`. `<b>déjà</b>` écrit par
  l'agent arrive **littéral**. Épinglé par H1–H4.

- **AC5 — Le rendu est désarmable sans redéploiement.** `MIKA_TELEGRAM_HTML_RENDER=0`
  restitue un chemin **octet-identique** à celui du repli. Analyse trois paliers ;
  absent → armé ; illisible → armé + WARN nommant la valeur entre guillemets ; lu une
  fois par process. Épinglé par C1–C4 et S1.

- **AC6 — mika#2126 n'est pas rouvert.** Les cinq fonctions et les 25 tests de
  `strip_markdown_around_urls` sont inchangés et verts ; elle est conservée en second
  passage du rendu plat. `**https://example.com/a**` donne un lien cliqué
  `https://example.com/a` **dans les deux modes**. Le test gelé de F4 reste vert et
  porte un commentaire disant ce qu'il épingle désormais, avec un test frère au niveau
  pipeline assertant le changement de comportement. **Le contrôle porte sur les deux
  modes séparément** : R1–R4 sur le chemin plat, qui traverse l'auxiliaire, et R5 sur
  le chemin HTML, qui ne la traverse pas et doit tenir le défaut fondateur par le
  reconnaisseur seul. Épinglé par R1–R5.

- **AC7 — Le rendu est infaillible par construction.** Aucun `unwrap`, `expect`,
  `panic!` ni indexation d'octets brute dans `telegram_markdown.rs`, asserté par un
  scan de source (S3) et non par un test comportemental — la régression ne rendrait pas
  une sortie fausse, elle rendrait un envoi panicable.

- **AC8 — L'état du rendu est observable.** `telegram_html_render_fallback` (WARN, avec
  la `description` de Telegram), `telegram_html_fallback_failed` (WARN) et
  `telegram_html_render_disabled` (INFO au démarrage, seulement si désarmé) sont émis.
  Aucun corps de message n'est journalisé. Régime attendu des deux WARN : **zéro
  ligne**.

- **AC9 — La décision est découvrable sans lire le code.**
  `crates/mika-gateway/CLAUDE.md` porte une section « Rendu du texte sortant » (mode
  HTML, repli, kill-switch, renvoi aux doc-comments) ; la variable est dans
  `.env.example` et dans le `CLAUDE.md` racine.

- **AC10 — Aucun élargissement collatéral.** Zéro dépendance ajoutée, `lib.rs`
  inchangé, `handle_send` inchangé, découpage 4096 toujours côté agent. Les deux
  défauts préexistants trouvés en chemin (garde miroir 4096 absente, `description` du
  400 non propagée) sont nommés hors périmètre avec ticket de suivi, et non corrigés
  ici.

---

## Revision history

| Date | Auteur | Changement |
|---|---|---|
| 2026-09-18 | dev-groom (mika#2291) | Re-groom idempotent. Les sept faits porteurs sont re-vérifiés exacts contre le code (25 tests `test_strip_markdown_*` lignes 1758–1922 ; `sendMessage` unique à `telegram.rs:619` ; `SendMessagePayload` à 332 ; `strip_markdown_around_urls` et ses quatre auxiliaires à 430/438/467/519/543 ; test gelé F4 à 1884 ; `routes.rs:2193` ne contrôle que `50_000` ; les 7 sites de construction non-test). **Une affirmation du plan était plus large que ce qu'il livre et a été corrigée** : « une régression du reconnaisseur ne peut pas rouvrir mika#2126 » ne vaut que pour le **chemin plat**, puisque la branche HTML — le mode par **défaut** — poste `render_html` sans repasser par `strip_markdown_around_urls`. La Brique 2 nomme désormais la portée exacte du filet, dit pourquoi la symétrie est écartée (l'auxiliaire réécrit `[label](url)` en `label : url` et détruirait le `Link` que `render_html` doit rendre en `<a href>`), et **R5** rejoue le corpus URL de mika#2126 contre `render_html` — le contrôle que R1 ne peut structurellement pas donner. Deux trous de contrat comblés au passage : **N11** (grammaire de lien refusée, `[x](https://example.com/a b)`, gelée par `telegram.rs:1909` — un `<a href>` posé sur une forme que `parse_markdown_link` a refusée serait un lien inventé) et **N12** (AC3 était posée sur `render_plain` seul alors que le chemin plat émet `strip_markdown_around_urls(render_plain(tokenize(x)))` — le contrôle s'arrêtait à une valeur que l'utilisateur ne reçoit jamais, et la seule composition capable de réécrire un message sain restait un angle mort). |
| 2026-09-18 | dev-groom (mika#2291) | Re-groom idempotent. Recomptage de la population mika#2126 contre le code : **25** tests `test_strip_markdown_*` (lignes 1758–1922), pas 30 — le chiffre apparaissait six fois, dont dans la Definition of Done et dans AC6, où il rendait le critère invérifiable. R1 nomme désormais la commande qui le recompte. Les six autres faits porteurs sont re-vérifiés exacts contre le code : F1 (doc-comment `parse_mode` à `telegram.rs:312`), F3 (site `sendMessage` unique à `telegram.rs:619`), F4 (test gelé à `telegram.rs:1884`), F6 (aucun parser markdown au workspace), F7 (`handle_send` rend un 502 au corps vide par sa branche `Err(e)`, les branches `BadRequest` de `routes.rs:908/917` étant sur `download_image`, chemin distinct), et les 7 sites de construction non-test de la Décision 5. La garde miroir 4096 est bien absente (`routes.rs:2193` ne contrôle que `50_000`). |
| 2026-09-18 | dev-groom (mika#2291) | Plan initial. Trois faits du code déplacent le corps du ticket : `parse_mode` est une décision datée avec sa condition de levée (F1), une transformation markdown de périmètre URL existe déjà (F2), et un test gelé pose le symptôme comme attendu (F4). Retenu : HTML plutôt que MarkdownV2 (surface d'échappement de 3 caractères contre 18), avec un repli sur 400 qui fait du pire cas le comportement d'aujourd'hui — ce qui supprime la prémisse de mika#2126 au lieu de la contredire. Parser CommonMark écarté sur une contrainte testée (préservation octet-pour-octet des espaces, F5), pas sur une préférence. |
