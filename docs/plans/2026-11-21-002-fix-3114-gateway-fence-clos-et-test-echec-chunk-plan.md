# mika#3114 vol 2 — le bloc clôturé plus long que la limite, et le test de l'échec au morceau 2 sur 3

**Disposition de ce plan : HALTE GROOM.** L'estimation du vol, mesurée sur le dépôt, est de
**310 lignes de code** hors `docs/` et hors `CLAUDE.md`, au-dessus de la borne groom de
**300** fixée par Vincent dans la tête du ticket. Le plan **ne propose aucune coupe** : la
tête l'interdit explicitement (« il ne renvoie rien à un vol 3 », « le plan ne choisit
aucune coupe », « une halte ne se lève jamais par un amendement du plan »). Le détail du
chiffre est au § *Taille estimée*.

La conception ci-dessous est écrite **en entier**, et c'est délibéré : c'est elle qui fonde
le chiffre, et une levée de halte par l'opérateur ne doit rien avoir à re-groomer.

Refs senara-solutions/mika#3114, senara-solutions/mika#2983.

## Halte groom

| | |
|---|---|
| Borne groom (tête du ticket, validée par Vincent le 2026-11-21) | 300 lignes estimées |
| Estimation de ce vol | **310 lignes** (hors `docs/`, hors `CLAUDE.md`) |
| Mesure attendue dans l'unité du ticket (`':!docs/plans'`, `CLAUDE.md` inclus) | ~332 lignes |
| Dépassement | +10 lignes estimées (+3 %) |
| Borne pilote (mesure réelle) | 420 — marge réelle |

**Le dépassement est dans les tests, et les tests sont énumérés par la tête.** Les 192 lignes
de tests sont imposées cas par cas : six cas nommés (a–f) par l'AC1(c), et un test avec
capture de log et contrôle négatif par l'AC2. Les deux densités qui les fondent sont des
mesures de ce dépôt, revérifiées à cette passe :

1. **Les tests de ce module coûtent ~17 lignes pièce**, doc-comment compris : les quatre
   tests du vol 1 occupent `telegram_split.rs:115-184`, soit 70 lignes pour 4 tests. Un
   helper d'assertion partagé amortit une part des six cas, pas la totalité.
2. **La couche de capture `tracing` coûte 34 lignes avant le premier test** :
   `SharedBuffer`, son `impl Write` et son `impl MakeWriter` occupent
   `egress_search/tests_e4_no_log.rs:32-65`, et ce module est privé à `egress_search` — le
   modèle est réutilisable par **copie**, pas par import, sans toucher un crate hors
   périmètre.

La tête estimait 70 lignes pour les tests d'AC1(c) et 45 pour celui d'AC3 couche comprise ;
la mesure donne ~108 et ~84. C'est là, et nulle part ailleurs, qu'est l'écart avec
l'estimation de ≈ 200 de la tête.

**Pourquoi le dépassement n'est pas compressible.** Resserrer la prose des tests ou fusionner
des cas nommés achèterait la borne avec de la couverture — exactement ce que la tête interdit
au pilote (« il ne resserre pas la prose ni les tests pour repasser sous la borne »). Du côté
production, la conception retenue est la moins chère des deux instruites : 96 lignes contre
112 pour le scanner de grammaire abandonné (§ *C1*). Il n'y a plus de gras d'un côté, et en
retirer de l'autre coûterait un critère d'acceptation.

**Ce que la halte attend de l'opérateur.** Une décision, pas un amendement : lever la borne
groom pour ce vol, ou retirer explicitement un des deux points de son périmètre. Le plan n'en
choisit aucun. Deux faits utiles à cette décision, et rien de plus : le dépassement est de
10 lignes, et la borne pilote de 420 garde de la marge sur une mesure attendue à ~332.

**Si ce plan est dispatché à un pilote malgré la halte** : le pilote s'arrête au premier tour
et le dit, sans écrire une ligne de code. La levée de la borne est un mot écrit de l'opérateur
sur le ticket ou dans la PR, jamais une lecture du plan.

## Contexte mesuré

Le vol 1 est livré (PR #3115, `ebf7e6e0`, déployée le 2026-11-21 à 10:41Z) et sa sonde est
verte : un texte de 4 827 unités UTF-16 est parti en 2 morceaux, réponse 200, aucun
`message is too long` depuis le redémarrage. Il a laissé deux restes nommés, dans
`crates/mika-gateway/CLAUDE.md` § *Outbound Splitting* et dans
`docs/plans/2026-11-21-001-fix-3114-gateway-send-decoupe-texte-long-plan.md` § *Vol 2*.
Ce sont eux, et eux seuls, que ce vol livre.

État lu dans le code au 2026-11-21 :

- `telegram_split.rs:32-59` — `split_outbound_text` compose chaque morceau par
  `format_outbound_text` et boucle sur `choose_cut`.
- `telegram_split.rs:65-100` — `choose_cut` : frontière dure par accumulation de
  `c.len_utf16()`, puis trois candidats (`\n\n`, `\n`, `' '`) retenus seulement si
  `cut_is_neutral`, sinon le plancher `hard`.
- `telegram_split.rs:105-109` — `cut_is_neutral(rest, cut, baseline)` compare
  `render_plain(tokenize(gauche)) + render_plain(tokenize(droite))` au baseline.
- `routes.rs:2688-2721` — `send_in_chunks` prend un `send` **injecté** (`F: Fn(String) ->
  Fut`), s'arrête au premier échec, garde `delivered`, émet `telegram_send_chunk_failed`
  avec `chunk_index`, `chunk_count`, `delivered`. La couture est déjà là : l'AC3 ne monte ni
  `AppState` ni Postgres.
- `routes.rs:2889-2906` — la correspondance erreur → statut est **écrite en ligne** dans
  `handle_send` (410 / 429 + `retry-after` / 502), donc non testable telle quelle.
- `crates/mika-gateway/CLAUDE.md` fait 45 717 octets (plafond 100 000,
  `crates/mika-common/tests/claude_md_size.rs`).

### Quatre faits de `telegram_markdown.rs` qui décident la conception

Ce vol **ne touche pas** `telegram_markdown.rs`. Il en dépend sur quatre points, relus dans
le code à cette passe — ce sont des faits, pas des hypothèses.

**F1 — un bloc sans closer n'est pas un bloc.** `telegram_markdown.rs:277-287` :
`is_run_of(chars, start, '`', 3)` ouvre, puis
`close = (body_start..len).find(|j| is_run_of(chars, *j, '`', 3))?` — le `?` fait échouer
`parse_fence` sans closer, et les backticks restent `Plain` verbatim
(`mika2291_unclosed_fence_stays_verbatim`). Et `is_run_of:439-444` refuse un run précédé du
marqueur, donc dans ` ```` ` l'ouverture est au premier backtick et le quatrième appartient
au corps.

**F2 — la fermeture ajoutée doit être `"\n```"`, et le `\n` n'est pas décoratif.**
`strip_fence_decoration:290-298` jette la première ligne du corps quand elle ne contient
aucun whitespace (l'info-string), **puis** un seul `\n` final (`strip_suffix('\n')`). Une
fermeture ajoutée sans son `\n` ferait manger par ce `strip_suffix` le `\n` *d'origine* du
texte quand la coupe tombe en fin de ligne — le cas préféré — et la réunion des morceaux
perdrait un caractère. Avec `"\n```"`, le `\n` jeté est celui qu'on vient d'ajouter. Rejoué
à la main sur le code à cette passe :

| part gauche | morceau émis | corps lu par `tokenize` |
|---|---|---|
| `a\n` | ` ```rust\na\n ` + `\n``` ` | `a\n` ✓ |
| `a` | ` ```rust\na ` + `\n``` ` | `a` ✓ |

**F3 — la réouverture est `format!("```{label}\n")`, étiquette vide comprise.** La même règle
jette la première ligne si elle est sans whitespace : `rust` est jetée comme info-string, et
une première ligne **vide** l'est aussi. Rouvrir avec ` ``` ` + `\n` préserve donc le corps
exactement, avec ou sans étiquette. Corollaire mesuré du ticket : `parse_fence` jette
l'étiquette au rendu HTML (`SegmentKind::Pre`) — l'étiquette répétée sert au **texte source**
et au repli brut de mika#2291, pas au `<pre>`.

**F4 — `render_plain` rend le corps d'un `Pre` tel quel.** `telegram_markdown.rs:476-493` :
le `match` n'a de bras spécial que pour `Link`, tout le reste est
`out.push_str(&segment.text)`. Un bloc clôturé rend donc exactement son corps décoré retiré.
**C'est ce fait qui rend la conception C1 possible.**

## Conception

### C1 — Le prédicat de neutralité valide la réparation, il n'y a pas de scanner

Le vol 1 a posé un oracle : une coupe est retenue si
`render_plain(tokenize(gauche)) + render_plain(tokenize(droite))` égale
`render_plain(tokenize(entier))`. Il a été écrit pour juger une **coupe nue**, mais il juge
aussi bien une **coupe réparée** — il compare deux rendus, il ne sait pas d'où viennent les
chaînes. Avec F4, l'arithmétique se ferme :

```
entier   = ```rust\nA\nB\n```            baseline = "A\nB"
gauche   = ```rust\nA\n  +  \n```        Pre → "A\n"
droite   = ```rust\n     +  B\n```       Pre → "B"
                                         somme  = "A\nB"  ✓
```

**Conséquence : la conception n'a pas besoin de savoir où commence le bloc.** Elle a besoin
d'un *candidat* de réparation, que l'oracle existant accepte ou refuse. Ce qui remplace un
scanner de grammaire est donc une **devinette bon marché** :

```rust
/// L'étiquette du bloc dans lequel `head` se termine, *si* `head` s'y termine.
/// Devinette : le dernier run de trois backticks de `head`, puis sa première ligne
/// si elle est sans whitespace (F3). Une devinette fausse est **refusée par
/// l'oracle**, jamais livrée — voir [`repair_is_neutral`].
fn fence_label_before(head: &str) -> Option<String>;
```

et l'oracle est généralisé d'un cran, `cut_is_neutral` devenant son appelant à une ligne :

```rust
fn repair_is_neutral(left: &str, right: &str, baseline: &str) -> bool;
```

**Pourquoi c'est strictement meilleur qu'un scanner.** Une conception à scanner devait être
*correcte* : une grammaire répliquée qui dérive de `tokenize` livre des clôtures
déséquilibrées en silence, et il lui fallait un test d'accord dédié pour s'en garder. Ici,
une devinette fausse retombe sur le plancher, c'est-à-dire **sur le comportement
d'aujourd'hui**. Les trois cas adverses se traitent sans aucun cas particulier :

| entrée adverse | ce qui se passe | pourquoi c'est correct |
|---|---|---|
| clôture **non fermée** de plus de 4096 unités | `gauche + "\n```"` *ferme* un bloc que le baseline lit verbatim (F1) : la somme des rendus diffère | oracle refuse → plancher, **aucune clôture ajoutée** — AC1(e) sans code dédié |
| ` ```` ` (quatre backticks) | `rfind("```")` rend l'index 1, `parse_fence` ouvre à 0 : la devinette est fausse | oracle refuse → plancher |
| backtick isolé avant un bloc | la devinette ne lit que les runs de trois, l'appariement peut différer | oracle refuse → plancher |

Le risque résiduel n'est plus « une clôture déséquilibrée part chez le lecteur » mais « une
réparation légitime est refusée et on retombe sur le comportement d'aujourd'hui ». C'est le
bon sens de l'erreur, et T1 est le test qui prouve que le cas nominal, lui, passe.

### C2 — Le chemin du vol 1 reste premier

L'aiguillage est écrit pour que l'AC3 (« les tests existants restent verts sans retouche »)
soit vrai **par construction** et non par inspection :

1. `hard_boundary(rest, budget)` est extraite de `choose_cut` (même calcul, aucun changement
   de comportement) pour être appelable par l'aiguillage.
2. `choose_cut(rest, budget)` est appelée **inchangée**. Si elle rend autre chose que le
   plancher, le morceau est émis comme aujourd'hui.
3. Le vol 2 n'intervient que si `cut == hard` **et** que `fence_label_before` rend un
   candidat **et** que `repair_is_neutral` accepte la réparation. Sinon : plancher, à l'octet
   d'aujourd'hui.

Cet ordre a une conséquence voulue : quand une coupe neutre existe **avant** le bloc (prose
puis bloc long), elle est préférée et le bloc n'est pas touché à ce tour — c'est l'AC1(d),
gratuite.

### C3 — Le budget réserve la place des ajouts avant de choisir la coupe

La boucle de `split_outbound_text` porte un état de plus : `reopen: Option<String>`
(l'étiquette à rouvrir, posée par le tour précédent). Le texte de travail du tour est
`reopen_prefix + rest` — **et c'est sur lui que le baseline est calculé**, sans quoi le tour 2
d'un bloc à trois morceaux comparerait un `Pre` à de la prose et refuserait toujours. Rejoué
à la main à cette passe : baseline du tour 2 = `tokenize("```rust\nB2B3\n```")` → `"B2B3"`, et
les deux moitiés réparées somment à `"B2" + "B3"` ✓.

```
budget       = limit − prefix_units − reopen_units       (reopen_units = 0 si None)
budget_corps = budget − CLOSE_UNITS                      (CLOSE_UNITS = 4, "\n```")
```

- `reopen_units = text_len_utf16("```{label}\n")` = `3 + label + 1`.
- Dans le corps, la coupe tombe de préférence au dernier `\n` sous `budget_corps`, sinon au
  plancher — jamais dans un `char` ni dans une paire de substitution, puisque `hard_boundary`
  accumule `c.len_utf16()`.
- Le morceau émis est `format_outbound_text(agent, &format!("{open}{corps}\n```"))` — **le
  préfixe garde son site de composition unique**, propriété du vol 1 qu'il serait facile de
  perdre ici.
- Sur un bloc couvrant `n` morceaux : `n−1` fermetures ajoutées et `n−1` réouvertures.
  L'AC1(b) (trois morceaux) marche par récurrence, sans cas particulier.

### C4 — La correspondance erreur → statut devient une fonction privée

`routes.rs:2889-2906` mélange le `match` sur `outcome.failure`, deux `warn!` et la
construction du `HeaderMap` du 429. Extraction en

```rust
fn status_for_send_failure(
    chat_id: i64,
    request_id: Option<&str>,
    error: TelegramApiError,
) -> Response;
```

appelée par `handle_send` sur le bras `Some(e)`. **Aucun statut ne change et aucun `warn!` ne
disparaît** : l'extraction existe pour que l'AC2 puisse affirmer « le statut rendu est celui
d'avant le découpage » sur un `TelegramApiError` fabriqué.

## Non-goals

Transcrits du § *Hors de ce vol* de la tête, sans ajout :

- `telegram_markdown.rs` (recognizer, rendu, repli) et la garde des 50 000 octets.
- La relance de `GatewaySender` et le doublon partiel qu'elle produit.
- La garde de l'outil `send_message` et son message au modèle.
- `mika notify` et son adresse de gateway par défaut — ce défaut attend un mot de Vincent
  pour #2983.
- `.github/workflows/`, `docs/egress/`.
- **CODEOWNERS** : aucun fichier visé n'y figure ; la PR peut se fermer sans geste de
  samidarko au titre du périmètre.

## Verification contract

Tous les tests sont des tests unitaires du crate, sans réseau ni Postgres.

### Dans `telegram_split.rs`

Un helper partagé porte les assertions communes des six cas — limite respectée, clôtures
équilibrées par morceau, réunion sans perte ni réordonnancement :

```rust
fn assert_chunks_wellformed(chunks: &[String], prefix: Option<&str>, limit: usize, input: &str);
```

- **T1 (AC1(c)(a))** — un bloc ` ```rust ` de plus de 4096 unités, préfixe `[mika-dev] ` :
  chaque morceau ≤ 4096 unités clôtures ajoutées comprises ; chaque morceau portant une part
  du bloc l'ouvre en ` ```rust ` ; la réunion, préfixes et clôtures ajoutées retirés, rend
  l'entrée. **Contrôle négatif** : la même fixture passée au chemin du plancher
  (`choose_cut` seul, sans l'aiguillage de C2) donne au moins un morceau aux clôtures
  déséquilibrées — la forme de `origin/main` à `ebf7e6e0`, vérifiée et non seulement
  commentée.
- **T2 (AC1(c)(b))** — un bloc couvrant au moins trois morceaux.
- **T3 (AC1(c)(c))** — un bloc sans étiquette : la réouverture est sans étiquette.
- **T4 (AC1(c)(d))** — un bloc qui tient dans un morceau, prose avant et après : le bloc
  n'est pas coupé, aucune clôture ajoutée.
- **T5 (AC1(c)(e))** — une clôture non fermée de plus de 4096 unités : aucune clôture
  ajoutée, le texte sort au plancher comme aujourd'hui. **C'est le test de l'oracle de C1** :
  il ne vérifie pas un cas particulier du code, il vérifie que la réparation illégitime est
  refusée.
- **T6 (AC1(c)(f))** — `tokenize` de chaque morceau lit un segment `Pre` pour sa part du
  bloc : le code est rendu, pas affiché en brut.

Il n'y a **pas** de test d'accord de grammaire : la conception C1 ne réplique aucune
grammaire, et T5 garde la propriété qui en tenait lieu.

### Dans `routes.rs`

- **T8 (AC3)** — un envoi faux qui accepte le morceau 1 et refuse le morceau 2, sur un texte
  de trois morceaux : `outcome.delivered.len() == 1`, `outcome.failure` présent, l'envoi faux
  appelé **exactement deux fois** (le morceau 3 n'est jamais tenté), une ligne
  `telegram_send_chunk_failed` capturée portant `chunk_index = 1`, `chunk_count = 3`,
  `delivered = 1`, et **le corps du message absent de toute ligne capturée** (mika#2126).
  Capture par une couche `tracing` de test sur le modèle `egress_search/tests_e4_no_log.rs`
  (`SharedBuffer` + `MakeWriter`, `tracing-subscriber` déjà en dev-dépendance). **Contrôle
  négatif dans le même test** : avec un envoi qui accepte tout, le même texte donne trois
  envois et aucune ligne `telegram_send_chunk_failed`.
- **T9 (AC3, statut)** — `status_for_send_failure` rend 410 sur `BotBlocked`, 429 avec
  l'en-tête `retry-after` sur `RateLimited { retry_after: Some(n) }`, 429 sans en-tête sur
  `retry_after: None`, 502 sur le reste.

### Commandes

```
cargo test -p mika-gateway
cargo clippy --all-targets -- -D warnings
bash scripts/verify-pipeline.sh origin/main
```

## Fire-Disposition

Ce plan livre un détecteur : l'assertion de non-fuite du corps de T8 (« le corps du message
n'apparaît dans aucune ligne capturée »), dont le chemin de succès est « aucune occurrence
trouvée ». Les autres tests vérifient un comportement sur une entrée donnée et ne sont pas
des détecteurs.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est vide.** Le détecteur
atterrit **armé**, sans aucune exception, parce qu'il n'existe aucune violation à excepter :
`send_in_chunks` ne journalise aujourd'hui aucun champ portant le corps (`routes.rs:2704,2708`
— `chat_id`, `request_id`, `chunk_index`, `chunk_count`, `delivered`, `error`), et la mesure se
fait au niveau du **rendu** de la couche `fmt`, pas de la liste des champs. C'est la forme que
ce crate utilise déjà : `SEND_MESSAGE_LITERAL_ALLOWED` « ships empty and stays empty », et la
résolution d'un rougissement est de retirer le champ fautif, jamais d'ajouter une entrée.

**Anti-vacuité, pour que « rien trouvé » se distingue de « rien regardé »** (mika#2205) : T8
assure d'abord que la capture est non vide **et** qu'elle contient la ligne
`telegram_send_chunk_failed` attendue, avant d'affirmer l'absence du corps. Sans cette
assertion préalable, un subscriber mal câblé rendrait le test vert en ne capturant rien.

## Definition of Done

- `split_outbound_text` ferme et rouvre un bloc clôturé plus long que le budget, avec la même
  étiquette, les ajouts comptés dans la limite.
- Un texte sous la limite garde le chemin d'aujourd'hui, au même octet près.
- T1–T6, T8, T9 verts ; les tests existants de `telegram_split.rs`, `routes.rs` et
  `telegram_markdown.rs` verts **sans retouche**.
- `cargo test -p mika-gateway`, `cargo clippy` et les lints du dépôt verts.
- Doc à jour : `crates/mika-gateway/CLAUDE.md` § *Outbound Splitting* (le bloc clôturé n'est
  plus un reste nommé ; la règle de fermeture/réouverture est décrite ; la sonde n'a plus de
  réserve sur « le code lisible » ; le reste nommé sur `GatewaySender` et celui de l'outil
  `send_message` restent) et le module doc de `telegram_split.rs` (le cas du bloc clôturé, et
  le fait que l'oracle valide la réparation). Le plan du vol 1 n'est pas réécrit.
- PR portant `Refs senara-solutions/mika#2983` et `Refs senara-solutions/mika#3114`, **sans**
  `Closes` / `Fixes` / `Resolves`.

## Acceptance criteria

Transcrits verbatim de la tête du ticket.

1. AC1(c) dans `telegram_split.rs`, testé sur :
   - (a) un bloc ` ```rust ` de plus de 4096 unités : chaque morceau fait au plus 4096
     unités, préfixe et clôtures ajoutées compris ; chaque morceau a ses clôtures
     équilibrées ; chaque morceau qui porte une partie du bloc l'ouvre en ` ```rust ` ; la
     réunion des morceaux, préfixes et clôtures ajoutées retirés, rend l'entrée sans perte ni
     réordonnancement ;
   - (b) un bloc qui couvre au moins trois morceaux ;
   - (c) un bloc sans étiquette : la réouverture est sans étiquette ;
   - (d) un bloc qui tient dans un morceau, précédé et suivi de prose : le bloc n'est pas
     coupé ;
   - (e) une clôture non fermée de plus de 4096 unités : aucune clôture ajoutée ;
   - (f) `tokenize` de chaque morceau lit un segment `Pre` pour sa partie du bloc : le code
     est rendu, pas affiché en brut.

   Contrôle négatif dans le test (a) : sur `origin/main` (ebf7e6e0), le même texte donne au
   moins un morceau aux clôtures déséquilibrées. Le test le dit en commentaire, ou le vérifie
   par une variante qui appelle le chemin du plancher.
2. AC3 en test, comme décrit au point 2 de la tête, contrôle négatif compris.
3. Les tests existants de `telegram_split.rs`, `routes.rs` et `telegram_markdown.rs` restent
   verts sans retouche. Un texte sous la limite garde le chemin d'aujourd'hui, au même octet
   près.
4. La doc dit la vérité :
   - `crates/mika-gateway/CLAUDE.md` § *Outbound Splitting* : le bloc clôturé n'est plus un
     reste nommé, la règle de fermeture et de réouverture est décrite, la sonde après deploy
     n'a plus de réserve sur « le code lisible ». Le reste nommé sur la relance de
     `GatewaySender` reste, et le message de l'outil `send_message` aussi ;
   - le module doc de `telegram_split.rs` décrit le cas du bloc clôturé ;
   - le plan du vol 1 n'est pas réécrit ; le plan du vol 2 est un fichier neuf sous
     `docs/plans/`.

   Le plafond des CLAUDE.md (100 000 octets, `crates/mika-common/tests/claude_md_size.rs`)
   tient : `crates/mika-gateway/CLAUDE.md` fait 45 717 octets aujourd'hui.
5. `cargo test -p mika-gateway`, `cargo clippy` et les lints du dépôt sont verts.
6. La PR porte `Refs senara-solutions/mika#2983` et `Refs senara-solutions/mika#3114`, et
   aucun `Closes`, `Fixes` ni `Resolves` : #3114 se ferme à la main après le deploy et la
   sonde de ce vol.

## Ordre de commit

1. `refactor(3114): extraire la correspondance erreur → statut de handle_send` — C4, aucun
   statut changé, T9.
2. `test(3114): l'échec au morceau 2 sur 3 est observable` — T8 et sa couche de capture. Ce
   commit teste du comportement **déjà livré** au vol 1 : il est vert sans aucun changement de
   production, ce qui est sa propre démonstration.
3. `feat(3114): fermer et rouvrir un bloc clôturé plus long que la limite` — C1 à C3, avec
   T1–T6.
4. `docs(3114): le bloc clôturé n'est plus un reste nommé` — `CLAUDE.md` § *Outbound
   Splitting* et le module doc.

Plancher : les commits 1 et 2 sont mergeables seuls et sans risque (aucun changement de
comportement). Le commit 3 est le vol.

## Sonde après deploy

Envoyer vers le chat de l'opérateur un message de plus de 4096 caractères dont un bloc
` ```rust ` dépasse à lui seul 4096 unités. Attendu :

- plusieurs messages dans l'ordre ;
- le code rendu en bloc dans chaque message, sans clôture apparente ;
- une ligne `telegram_send_split` ;
- aucun `message is too long` et aucun `telegram_send_chunk_failed` depuis ce deploy.

**Halte** : si des clôtures apparentes réapparaissent après le deploy, établir d'abord que le
binaire déployé porte le correctif (classe mika#2340) **avant** de toucher au découpage.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-gateway/src/telegram_split.rs` (`repair_is_neutral` + `cut_is_neutral` appelant, `fence_label_before`, extraction `hard_boundary`, réserve + fermeture/réouverture, doc module) | 96 |
| `crates/mika-gateway/src/routes.rs` (extraction `status_for_send_failure`) | 22 |
| tests T1–T6 + helper `assert_chunks_wellformed` (`telegram_split.rs`) | 108 |
| tests T8–T9 + couche de capture `tracing` (`routes.rs`) | 84 |

Total estimé : 310 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 300 lignes estimées — (150 − 27 − 30) × 9 / 3 = 279, surchargée à 300 par Vincent dans la tête du ticket le 2026-11-21

Tracked in: senara-solutions/mika#2983

**Sur les deux nombres de cette ligne.** La dérivation mika#2693 donne 279 ; la tête du ticket
déclare ce calcul non applicable à ce vol et fixe les bornes à la main (groom 300, pilote 420).
**Les deux sont sous l'estimation de 310 : la halte ne dépend pas du choix entre eux**, et la
plus serrée mord de toute façon.

**Le total est au-dessus des deux bornes, et le plan ne découpe pas.** Ce n'est pas une
omission : la tête du ticket prescrit, pour exactement ce cas, une halte groom sans coupe —
« si le plan estime le vol au-dessus de 300 lignes, le groom s'arrête, écrit la halte et le
chiffre, et ne propose aucune coupe », et « il ne renvoie rien à un vol 3 ». La disposition de
ce plan est donc la halte du § *Halte groom*, la suite appartient à l'opérateur, et les deux
restes du vol 1 demeurent tracés par la ligne `Tracked in:` ci-dessus.

Les lignes comptées incluent les doc-comments Rust, que `git diff --shortstat` compte et que la
densité documentaire de ce dépôt rend non négligeables. `crates/mika-gateway/CLAUDE.md` est hors
compte par la règle mika#2636 ; son édition est estimée à ~22 lignes supplémentaires au titre de
la borne du ticket (`':!docs/plans'`), d'où la mesure attendue de ~332 lignes.

## Gel des tickets (jusqu'au 01/12)

Ce vol n'ouvre aucun ticket, pas même de suivi. Tout reste part en commentaire sur #2983 ou
dans la PR, sous la ligne `Tracked in:` du § *Taille estimée*.
