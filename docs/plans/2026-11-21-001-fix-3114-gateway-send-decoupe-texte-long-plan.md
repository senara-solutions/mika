# mika#3114 — `/send` découpe un texte trop long pour Telegram au lieu de le perdre

Refs senara-solutions/mika#2983

## Contexte mesuré

Telegram plafonne le champ `text` de `sendMessage` à 4096 unités UTF-16. Au-delà, il
répond 400 « Bad Request: message is too long ». `handle_send`
(`crates/mika-gateway/src/routes.rs:2682`) envoie le texte reçu en un seul
`sendMessage` — son commentaire le dit (`// Send to Telegram (no message splitting —
send as-is)`, ligne 2789) et sa seule garde de longueur est `payload.text.len() >
50_000` **octets** (ligne 2687). Le gateway répond alors 502 et le message est perdu.

Le journal du 2026-11-21 compte **418 envois** perdus depuis le 2026-09-20 (7 025
depuis mai) et **420 lignes** `telegram_html_fallback_failed` portant la même
description : le repli en texte brut de mika#2291 échoue de la même façon, le texte
brut étant lui aussi trop long.

Trois faits de code encadrent le correctif :

1. `send_message_impl` (`telegram.rs:874`) est **l'unique site `sendMessage` du
   crate** ; `TelegramClient` et `CustomerTelegramClient` y convergent tous deux.
2. `mika-common/src/telegram.rs:6` déclare « **Qui découpe :** l'agent » et prête au
   gateway une garde à 4096 qui n'existe pas. `crates/mika-gateway/CLAUDE.md` le
   reconnaît déjà (§ *Known adjacent gap*) et annonce un ticket de suivi jamais
   ouvert.
3. Seul l'outil `send_message` de l'agent (`crates/mika-agent/src/tools/send_message.rs:102`)
   applique `MAX_TEXT_UTF16_UNITS`. `GatewaySender::try_send` et `mika notify` n'ont
   aucune garde — ce sont ces chemins qui perdent les messages.

## Deux faits de code qui décident la conception

Deux propriétés mesurées sur le code existant commandent les deux choix
non-évidents de ce plan. Elles sont établies ici pour que la QA n'ait pas à les
redécouvrir.

### A — `render_plain` ne rallonge jamais, `render_html` peut rallonger

`render_plain` (`telegram_markdown.rs:476`) concatène le texte reconnu, marqueurs
retirés. Toutes les formes de la table de reconnaissance raccourcissent ou
conservent la longueur : `**x**` → `x`, `` `x` `` → `x`, ` ```l\nx\n``` ` → `x`,
`# ` retiré, `* ` → `- ` (égal), et `[label](url)` (= `len+4`) → `label : url`
(= `len+3`). `strip_markdown_around_urls` ne retire que des marqueurs. Donc :

> **un morceau source de ≤ 4096 unités UTF-16 produit toujours un corps brut de
> ≤ 4096 unités.**

C'est ce qui rend le repli de mika#2291 **efficace** après ce correctif alors qu'il
ne l'était pas avant : les 420 `telegram_html_fallback_failed` sont exactement la
population où le brut était lui aussi trop long.

`render_html`, à l'inverse, **rallonge** : `escape_html` porte `&` à `&amp;` (+4) et
les balises ajoutent leurs caractères (`**x**` → `<b>x</b>`, +3).

### B — `tokenize` ne rend aucun offset source

`Segment { kind, text }` (`telegram_markdown.rs:86`) ne porte pas la position du
segment dans le texte d'entrée, et les marqueurs ayant été retirés, cette position
n'est pas reconstructible. Or le ticket exige qu'« une coupe ne tombe jamais au
milieu d'un segment de `telegram_markdown::tokenize` », et met
`telegram_markdown.rs` **hors de ce vol** — on ne peut donc pas y ajouter une
fonction d'offsets.

La sortie est un **prédicat d'invariance** plutôt qu'une table d'offsets : une coupe
à l'indice `k` est *neutre* quand

```
render_plain(tokenize(&s[..k])) + render_plain(tokenize(&s[k..]))
    == render_plain(tokenize(s))
```

Une coupe au milieu de `**gras**` laisse `**gr` à gauche (marqueurs non appariés,
donc verbatim) et `as**` à droite : l'égalité tombe, la coupe est refusée. Une coupe
propre laisse les segments intacts de part et d'autre et l'égalité tient. Le
prédicat **réutilise le recognizer sans le modifier**, et il refuse par construction
toute coupe qui change la reconnaissance — ce qui est exactement l'invariant demandé,
et ce qui donne AC1(e) sans deuxième reconnaissance.

## Découpage en vols (mika#2693)

Le travail complet du ticket mesure environ **375 lignes** hors `docs/`, au-dessus de
la borne par vol de 279. Il part donc en deux vols, le vol courant repassant sous la
borne. La frontière est fonctionnelle, pas arbitraire : **la perte d'abord, le rendu
ensuite.**

### Vol 1 — ce vol : le texte long n'est plus perdu

Le découpage par frontières naturelles, la neutralité de coupe, la boucle d'envoi,
les deux événements, une ligne `outbound_messages` par morceau, l'arrêt au premier
échec, la doc. Couvre AC1(a), AC1(b), AC1(d), AC1(e), AC2, AC4, AC5, AC6, AC7, et le
**comportement** d'AC3.

### Vol 2 — hors ce vol, nommé

`Tracked in: senara-solutions/mika#2983`

- **AC1(c) — le bloc clôturé plus long que la limite.** Clôture du morceau et
  réouverture avec la même étiquette de langage (`fence_open_at`,
  `splits_fence_run`, réserve de clôture dans le budget), et son test.
- **AC3 en test** — l'échec simulé au morceau 2 sur 3, avec la capture de la ligne
  `telegram_send_chunk_failed` et de `delivered = 1`. Le comportement (arrêt, statut,
  événement, `delivered`) est livré dans ce vol ; seule sa vérification par capture de
  log part au vol 2.

**Dégradation nommée et acceptée dans le vol 1.** Un bloc clôturé plus long que la
limite n'a aucun candidat de coupe neutre (le prédicat du § B refuse toute coupe à
l'intérieur d'un fence), le découpage retombe donc sur le plancher de progression et
les backticks se déséquilibrent entre les morceaux. `tokenize` ne reconnaît pas un
fence non fermé — ses backticks restent `Plain` verbatim — donc Telegram accepte
l'envoi (200) et le lecteur voit des ``` apparents. **C'est cosmétique : le message
n'est plus perdu**, ce qui est le défaut mesuré. La sonde après deploy du ticket, qui
demande « le code lisible », ne sera donc pleinement satisfaite qu'au vol 2.

## Requirements

### R1 — Une fonction pure de découpage

Nouveau module `crates/mika-gateway/src/telegram_split.rs`, déclaré par
`mod telegram_split;` dans `main.rs` après `mod telegram_markdown;` (c'est `main.rs`
qui déclare les modules du crate ; `lib.rs` n'expose que `voice`).

```rust
pub(crate) fn split_outbound_text(
    agent_name: Option<&str>,
    text: &str,
    limit: usize,
) -> Vec<String>
```

- **Le préfixe a un seul site de composition.** La fonction appelle
  `routes::format_outbound_text` (rendu `pub(crate)`) pour composer chaque morceau.
  Le cas « tient en un morceau » rend donc **littéralement**
  `vec![format_outbound_text(agent_name, text)]` : l'identité au chemin d'aujourd'hui
  est garantie par construction et non par inspection (AC1(a), AC4).
- **La mesure est `mika_common::telegram::text_len_utf16`**, préfixe compris. Le
  budget d'une tranche vaut `limit − prefix_units`, où `prefix_units` se déduit de
  `text_len_utf16(&full) − text_len_utf16(text)`.
- **Le préfixe est répété sur chaque morceau**, pour que `resolve_reply_agent` route
  une réponse faite à n'importe lequel d'entre eux.
- `limit` est un paramètre et non une constante lue au site, pour que les tests
  puissent exercer le découpage sur des textes courts.

`choose_cut(rest, budget) -> usize` choisit la coupe :

1. Si `text_len_utf16(rest) <= budget`, rendre `rest.len()`.
2. `hard` = le plus grand **indice de frontière de `char`** tel que les unités UTF-16
   accumulées restent `<= budget`. L'itération se fait sur `char_indices()` en
   additionnant `c.len_utf16()` : une coupe ne tombe donc jamais dans un caractère,
   et jamais au milieu d'une paire de substitution, parce qu'un `char` hors BMP
   compte 2 unités d'un seul tenant (AC1(d)).
3. Candidats, par ordre de préférence décroissante dans `rest[..hard]` : dernier
   `\n\n` (fin de paragraphe), dernier `\n` (fin de ligne), dernier `' '` (espace),
   puis `hard`.
4. Rendre le premier candidat non nul qui est **neutre** au sens du § B.
5. **Plancher de progression** : si aucun candidat n'est neutre, rendre `hard`. C'est
   ce qui garantit l'absence de boucle infinie. Un `hard` nul (budget trop petit pour
   le premier `char`) termine la boucle d'appel.

`cut_is_neutral(rest, cut) -> bool` implémente le prédicat du § B. La valeur
`render_plain(tokenize(rest))` est calculée **une fois par tranche**, hors de la
boucle sur les candidats. Le coût total est borné par la garde des 50 000 octets déjà
en place sur `handle_send`.

### R2 — La boucle d'envoi, dans `handle_send`

Dans `routes.rs`, une fonction d'envoi séquentiel portant une **couture de `sender`
injectée** :

```rust
pub(crate) struct ChunkedSendOutcome {
    pub(crate) delivered: Vec<i64>,            // message_ids, dans l'ordre
    pub(crate) failure: Option<TelegramApiError>,
}

pub(crate) async fn send_in_chunks<F, Fut>(
    chat_id: i64,
    request_id: Option<&str>,
    chunks: &[String],
    send: F,
) -> ChunkedSendOutcome
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<i64, TelegramApiError>>,
```

**La couture est le design, pas un surcoût de test.** `api_url`
(`telegram.rs:499`) code en dur `https://api.telegram.org`, et le doc-comment de
`should_fall_back_to_plain` déclare explicitement hors périmètre le fait de rendre
cette URL injectable (« widening the production surface for test-only
observability »). Un `sender` en paramètre donne la testabilité **sans** toucher à
cette décision : `handle_send` le ferme sur `tg_client.send_message(...)`, le test
le ferme sur un enregistreur.

Comportement :

- Envoie les morceaux **dans l'ordre**, un `sendMessage` par morceau via
  `send_message_impl` tel qu'il est — HTML armé, repli brut sur 400, interrupteur
  `MIKA_TELEGRAM_HTML_RENDER`. Rien ne change dans `telegram_markdown.rs`.
- **Au premier échec, l'envoi s'arrête** : le morceau `k+1` n'est jamais tenté.
  `delivered` porte les `message_id` des morceaux livrés, `failure` l'erreur.
- Émet le WARN `telegram_send_chunk_failed` sur l'arm d'échec, avec `chat_id`,
  `request_id`, `chunk_index`, `chunk_count`, `delivered` et `error`. **Le corps du
  message n'est jamais journalisé**, au standard du fichier (mika#2126, restaté par
  mika#2291).

`handle_send` :

- Remplace le `match tg_client.send_message(...)` unique par :
  `split_outbound_text(payload.agent_name.as_deref(), &payload.text,
  mika_common::telegram::MAX_TEXT_UTF16_UNITS)`, puis `send_in_chunks`.
- Quand `chunks.len() > 1`, émet l'INFO `telegram_send_split` avec `chunk_count` et
  la longueur totale en unités UTF-16, **sans le corps**. Un seul morceau n'émet
  rien : l'événement mesure le découpage, et une ligne par envoi nominal serait la
  churn que mika#2131 borne.
- Insère **une ligne `outbound_messages` par morceau livré**, avec le même
  `agent_name` — y compris sur l'arm d'échec, pour les morceaux 1..k−1 : sans elles,
  une réponse à un morceau livré ne routerait pas.
- Mappe `failure` au statut d'aujourd'hui, par les mêmes arms qu'aujourd'hui :
  `BotBlocked` → 410, `RateLimited` → 429 avec `retry-after`, autre → 502. Pas de
  `failure` → 200.
- Le commentaire `// Send to Telegram (no message splitting — send as-is)` disparaît.

**Risque nommé, accepté dans ce vol.** `GatewaySender` refait un envoi complet après
un échec : une erreur au morceau `k` suivie de cette relance renvoie les morceaux
1..k−1 en double. Un doublon visible vaut mieux qu'une perte ; l'événement le rend
comptable, et le corriger toucherait l'agent, hors de ce vol.

### R3 — La doc dit la vérité

- `crates/mika-common/src/telegram.rs` : le bloc « **Qui découpe :** l'agent […]
  envoie tel quel (`send as-is`) » est remplacé par « le gateway découpe », avec le
  renvoi à mika#3114 à côté du renvoi existant à mika#2134.
- `crates/mika-gateway/CLAUDE.md` : le § *Known adjacent gap, deliberately out of
  scope* est remplacé par le comportement de découpage et les deux événements
  (`telegram_send_split`, `telegram_send_chunk_failed`), en nommant la dégradation du
  bloc clôturé et son suivi. Le plafond des 100 000 octets tient largement (43 017
  aujourd'hui).
- Le message de refus de l'outil `send_message` de l'agent (« The gateway does not
  split messages ») **n'est pas touché** : hors de ce vol.

## Non-goals

- La garde de l'outil `send_message` de l'agent et son message au modèle.
- La relance de `GatewaySender` et le doublon partiel.
- Les 15 erreurs réseau du même journal.
- `telegram_markdown.rs` (recognizer, rendu, repli), la garde des 50 000 octets,
  `.github/workflows/`, `docs/egress/`.
- **Mesurer le budget sur le rendu HTML.** La mesure porte sur le morceau **source**,
  préfixe compris, comme le dit « la mesure » du ticket et comme l'exige AC1(b) d'une
  fonction *pure* (qui n'a pas accès à l'état d'armement). Par le § A, un morceau
  source sous la limite a toujours un corps **brut** sous la limite, donc la perte
  disparaît ; un morceau quasi-pleine-limite très décoré peut encore voir son **rendu
  HTML** refusé en 400 et être délivré en brut par le repli de mika#2291. C'est une
  dégradation cosmétique, pas une perte — mais elle écrit une ligne
  `telegram_html_render_fallback`, dont le régime attendu est zéro. À surveiller
  après deploy ; reste tracké sur #2983.
- **CODEOWNERS** : aucun fichier visé n'y figure. La PR peut se fermer sans geste de
  samidarko au titre du périmètre.

## Verification contract

`cargo test -p mika-gateway`, `cargo clippy`, `cargo fmt --check`,
`bash scripts/verify-pipeline.sh origin/main` et les lints du dépôt. Les tests
existants de `routes.rs` et de `telegram_markdown.rs` doivent rester verts **sans
retouche** — c'est la forme opérationnelle d'AC4.

Les nouveaux tests sont des tests de comportement d'une fonctionnalité neuve : leur
chemin de succès est « le découpage a produit ces morceaux », non « aucune violation
trouvée ». Ce plan ne livre donc **aucun détecteur** au sens de mika#1574 — pas de
lint, pas de garde CI, pas de scan de source, pas d'allowlist — et la section
Fire-Disposition de mika#2306 est donc sans objet (gate N/A), raison pour laquelle
ce plan ne la porte pas.

### Tests du vol 1

Dans `telegram_split.rs` (`#[cfg(test)] mod tests`) :

- **T1 (AC1 a)** — un texte sous la limite rend un morceau identique à l'entrée,
  avec `agent_name = None` puis avec un agent nommé (le morceau est alors
  exactement `format_outbound_text`).
- **T2 (AC1 b)** — un texte de 10 000 caractères en paragraphes : chaque morceau,
  préfixe compris, a `text_len_utf16(&chunk) <= 4096` ; la concaténation des
  morceaux, préfixes retirés, **égale l'entrée** sans perte ni réordonnancement.
- **T3 (AC1 d)** — un texte d'emoji hors BMP : aucun morceau ne contient de `char` de
  remplacement, chaque morceau est un `&str` valide, la limite est tenue en UTF-16 —
  et le test **distingue** explicitement les trois comptages, en asservissant le
  nombre de morceaux attendu à `text_len_utf16` et non à `len()` ni à
  `chars().count()`.
- **T4 (AC1 e)** — un `**gras**` à cheval sur la limite : la coupe tombe **avant** le
  `**` ouvrant. Assertion sur la forme (le premier morceau ne finit pas par un
  préfixe de `**gras**`, le second commence par `**`) **et** sur le prédicat du § B
  appliqué à la coupe retenue.

Dans `routes.rs` (`#[cfg(test)] mod tests`) :

- **T5 (AC2)** — le test de bout en bout du défaut. Un texte de plus de 4096
  caractères traverse `split_outbound_text` puis `send_in_chunks` avec un `sender`
  enregistreur (`Arc<Mutex<Vec<String>>>`). Assertions : les envois sont **dans
  l'ordre**, chacun a `text_len_utf16 <= 4096`, et leur texte réuni — préfixes
  retirés — **égale le texte posté**. **Contrôle négatif dans le même test** : le
  même texte passé au `sender` sans découpage produit **un seul** envoi de plus de
  4096 unités, ce qui établit que le texte de fixture est bien dans la population du
  défaut et que l'assertion précédente mesure le découpage.
- **T6 (AC4)** — un texte sous la limite garde le chemin d'aujourd'hui : un seul
  envoi, octet pour octet celui de `format_outbound_text`, et `delivered.len() == 1`
  (donc une seule ligne `outbound_messages`).

## Definition of Done

- [ ] `telegram_split.rs` livré, déclaré dans `main.rs`, et `split_outbound_text`
      appelé par `handle_send`.
- [ ] `send_in_chunks` envoie dans l'ordre, s'arrête au premier échec, rend
      `delivered` et `failure`, et émet `telegram_send_chunk_failed` sans le corps.
- [ ] `telegram_send_split` émis sur un découpage, sans le corps ; rien sur un envoi
      unique.
- [ ] Une ligne `outbound_messages` par morceau livré, même `agent_name`, y compris
      sur l'arm d'échec.
- [ ] Statuts inchangés : 200 / 410 / 429 avec `retry-after` / 502.
- [ ] Le commentaire « no message splitting » a disparu.
- [ ] `mika-common/src/telegram.rs` et le § *Known adjacent gap* de
      `crates/mika-gateway/CLAUDE.md` disent que le gateway découpe.
- [ ] T1–T6 verts ; tests existants de `routes.rs` et `telegram_markdown.rs` verts
      sans retouche.
- [ ] `cargo test -p mika-gateway`, `cargo clippy`, `cargo fmt --check`,
      `scripts/verify-pipeline.sh origin/main` verts. Plafond CLAUDE.md tenu.
- [ ] La PR porte `Refs mika#2983`, aucun `Closes` / `Fixes` / `Resolves`, et la
      ligne `Tracked in: senara-solutions/mika#2983` pour le vol 2.

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#3114. La portée par vol est
donnée au § *Découpage en vols* ci-dessus : AC1(c) et le **test** d'AC3 sont hors du
vol courant et tracés sur #2983.

1. Une fonction pure de découpage, dans `crates/mika-gateway/src/telegram.rs` ou un
   module voisin, rend les morceaux d'un texte. Elle est testée sur :
   - (a) un texte sous la limite : un morceau, identique à l'entrée ;
   - (b) un texte de 10 000 caractères en paragraphes : chaque morceau, préfixe
     compris, fait au plus 4096 unités UTF-16, et la concaténation rend l'entrée sans
     perte ni réordonnancement ;
   - (c) un bloc clôturé plus long que la limite : chaque morceau a des clôtures
     équilibrées et la même étiquette de langage ;
   - (d) un texte d'emoji hors BMP : aucune coupe dans une paire de substitution, et
     la limite est comptée en UTF-16, pas en octets ni en `char` ;
   - (e) un `**gras**` à cheval sur la limite : la coupe tombe avant lui.
2. **Le test de bout en bout du défaut.** Un message de plus de 4096 caractères posté
   sur le chemin de `handle_send` arrive entier. Les envois enregistrés par un faux
   Telegram (wiremock, déjà en dev-dépendance, ou une couture injectée) sont dans
   l'ordre, chacun sous la limite. Leur texte réuni, préfixes et clôtures ajoutées
   retirés, égale le texte posté. Contrôle négatif dans le même test : sans le
   découpage, le même texte part en un seul envoi de plus de 4096 unités.
3. Un échec simulé au morceau 2 sur 3 donne le statut d'erreur, une ligne
   `telegram_send_chunk_failed` avec `delivered = 1`, et aucun envoi du morceau 3.
4. Un texte sous la limite garde le chemin d'aujourd'hui : un envoi, une ligne
   `outbound_messages`. Les tests existants de `routes.rs` et de
   `telegram_markdown.rs` restent verts sans retouche.
5. La doc dit la vérité :
   - `crates/mika-common/src/telegram.rs` : le gateway découpe ;
   - `crates/mika-gateway/CLAUDE.md` : le § *Known adjacent gap* est remplacé par le
     comportement et les deux événements ;
   - le commentaire « no message splitting » de `handle_send` disparaît.
   Le message de refus de l'outil `send_message` (« The gateway does not split
   messages ») n'est pas touché ici : voir *Hors de ce vol*.
6. `cargo test -p mika-gateway`, `cargo clippy` et les lints du dépôt sont verts. Le
   plafond des CLAUDE.md (100 000 octets) tient.
7. La PR porte `Refs mika#2983`, et aucun `Closes`, `Fixes` ni `Resolves`.

## Ordre de commit

1. `feat(gateway): split outbound Telegram text at the UTF-16 limit (mika#3114)` —
   `telegram_split.rs` + `mod` dans `main.rs` + T1–T4.
2. `feat(gateway): send a split message chunk by chunk (mika#3114)` —
   `send_in_chunks`, branchement dans `handle_send`, les deux événements,
   `outbound_messages` par morceau + T5–T6.
3. `docs(gateway): the gateway splits, and says so (mika#3114)` — `mika-common/src/telegram.rs`
   et `crates/mika-gateway/CLAUDE.md`.

Chaque commit laisse `cargo test -p mika-gateway` vert. Le commit 1 seul laisserait
`split_outbound_text` non appelé, donc les commits 1 et 2 vont dans la même PR —
c'est le plancher de ce vol.

## Sonde après deploy

Faire envoyer par un agent un message de plus de 4096 caractères vers le chat de
l'opérateur. Attendu : plusieurs messages dans l'ordre, une ligne
`telegram_send_split`, et aucun `message is too long` à partir de ce deploy. Un bloc
de code reste lisible en contenu mais ses backticks peuvent apparaître — c'est la
dégradation nommée du vol 1, levée au vol 2.

**Halte** : si `message is too long` réapparaît après le deploy, établir d'abord que
le binaire déployé porte le correctif (classe mika#2340) avant de toucher au
découpage.

**Seconde halte** : si `telegram_html_render_fallback` passe au-dessus de 1 % des
envois, c'est l'expansion HTML nommée en *Non-goals* qui mord. Ne pas désarmer le
rendu : mesurer la distribution des longueurs de morceaux et rouvrir la question du
budget sur #2983.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-gateway/src/telegram_split.rs` (découpage, hors fence) | 101 |
| `crates/mika-gateway/src/routes.rs` (`send_in_chunks` + branchement + 2 événements) | 64 |
| tests T1–T4 (`telegram_split.rs`) | 44 |
| tests T5–T6 (`routes.rs`) | 50 |
| `crates/mika-common/src/telegram.rs` (doc-comment) | 8 |

Total estimé : 267 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Le vol courant tient sous la borne parce qu'AC1(c) et le test d'AC3 en sortent —
environ 110 lignes renvoyées au vol 2, nommé ci-dessus et tracé par
`Tracked in: senara-solutions/mika#2983`. Les lignes comptées incluent les
doc-comments Rust, que `git diff --shortstat` compte et que la densité documentaire
de ce dépôt rend non négligeables ; `crates/mika-gateway/CLAUDE.md` est hors compte
par la règle mika#2636, et son édition est estimée à ~25 lignes supplémentaires au
titre de la borne du ticket (`':!docs/plans'`), ce qui laisse la mesure réelle
attendue sous la halte groom de 300.

## Gel des tickets

Ce vol n'ouvre aucun ticket, pas même de suivi. Le reste du travail (vol 2) va en
commentaire sur #2983 et dans le corps de la PR, noté
`Tracked in: senara-solutions/mika#2983`.
