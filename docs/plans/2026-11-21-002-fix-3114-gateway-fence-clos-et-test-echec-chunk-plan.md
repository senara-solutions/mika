# mika#3114 vol 2 — le bloc clôturé plus long que la limite, et le test de l'échec au morceau 2 sur 3

**Disposition de ce plan : HALTE GROOM.** L'estimation du vol, mesurée sur le dépôt,
est de **342 lignes de code** hors `docs/` et hors `CLAUDE.md`, au-dessus de la borne
groom de **300** fixée par Vincent dans la tête du ticket. Le plan **ne propose aucune
coupe** : la tête l'interdit explicitement (« il ne renvoie rien à un vol 3 », « le plan
ne choisit aucune coupe », « une halte ne se lève jamais par un amendement du plan »).
Le détail de l'estimation est au § *Taille estimée* ; la conception ci-dessous est
écrite en entier parce que c'est elle qui **fonde le chiffre**, et pour qu'une levée de
halte par l'opérateur n'ait rien à re-groomer.

Refs senara-solutions/mika#3114, senara-solutions/mika#2983.

## Halte groom

| | |
|---|---|
| Borne groom (tête du ticket, validée par Vincent le 2026-11-21) | 300 lignes estimées |
| Estimation de ce vol | **342 lignes** (hors `docs/`, hors `CLAUDE.md`) |
| Mesure attendue dans l'unité du ticket (`':!docs/plans'`, `CLAUDE.md` inclus) | ~364 lignes |
| Dépassement | +42 lignes estimées (+14 %) |

**Pourquoi l'estimation du ticket (≈ 200) et celle-ci (342) divergent.** L'écart est
presque entièrement dans les tests, et il est fondé sur deux mesures prises dans ce
dépôt, non sur une appréciation :

1. **Les tests de ce module coûtent ~17 lignes pièce**, doc-comment compris : les
   quatre tests du vol 1 occupent `telegram_split.rs:115-184`, soit 70 lignes pour 4
   tests. L'AC1(c) en impose **six** (a–f), chacun avec sa fixture et son assertion de
   non-perte. Le ticket les estimait à 70 lignes au total ; la mesure du vol 1 donne
   ~102, plus un helper d'assertion partagé.
2. **La couche de capture `tracing` coûte ~32 lignes avant le premier test** :
   `SharedBuffer`, son `impl Write` et son `impl MakeWriter` occupent
   `egress_search/tests_e4_no_log.rs:32-63`, et ce module est privé à `egress_search` —
   le modèle est réutilisable par copie, pas par import, sans toucher un crate hors
   périmètre. Le ticket estimait 45 lignes pour le test d'AC3 *couche comprise*.

Les deux postes de code de production (scanner de fences + fermeture/réouverture :
112 ; extraction erreur → statut : 22) sont, eux, dans l'ordre de grandeur annoncé par
le ticket (60 + 15 = 75 contre 134 ici — l'écart vient du scanner, § *Conception C1*,
qui n'était pas chiffré dans la tête).

**Le dépassement n'est pas compressible sans retirer un critère d'acceptation.** Les
208 lignes de tests sont imposées cas par cas par l'AC1(c) (six cas nommés) et l'AC2
(un test avec capture de log, contrôle négatif compris). Resserrer la prose des tests
ou fusionner des cas reviendrait à acheter la borne avec de la couverture — exactement
ce que la tête interdit au pilote (« il ne resserre pas la prose ni les tests pour
repasser sous la borne »).

**Ce que la halte attend de l'opérateur.** Une décision, pas un amendement : lever la
borne pour ce vol, ou retirer explicitement un des deux points de son périmètre. Le
plan n'en choisit aucun.

## Contexte mesuré

Le vol 1 est livré (PR #3115, `ebf7e6e0`, déployée le 2026-11-21 à 10:41Z) et sa sonde
est verte : un texte de 4 827 unités UTF-16 est parti en 2 morceaux, réponse 200, aucun
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
- `telegram_split.rs:105-109` — `cut_is_neutral` compare
  `render_plain(tokenize(gauche)) + render_plain(tokenize(droite))` au baseline.
- `routes.rs:2688-2721` — `send_in_chunks` s'arrête au premier échec, garde
  `delivered`, émet `telegram_send_chunk_failed` avec `chunk_index`, `chunk_count`,
  `delivered`.
- `routes.rs:2889-2906` — la correspondance erreur → statut est **écrite en ligne**
  dans `handle_send` (410 / 429 + `retry-after` / 502), donc non testable telle quelle.
- `crates/mika-gateway/CLAUDE.md` fait 45 717 octets (plafond 100 000,
  `crates/mika-common/tests/claude_md_size.rs`).

### Trois faits de `telegram_markdown.rs` qui décident la conception

Ce vol **ne touche pas** `telegram_markdown.rs`. Il en dépend sur trois points, qui
sont des faits lus, pas des hypothèses.

**F1 — `parse_fence` n'exige pas que l'ouverture soit en début de ligne, et le closer
est le premier run de trois backticks non précédé d'un backtick.**
`telegram_markdown.rs:277-287` : `is_run_of(chars, start, '`', 3)` ouvre, puis
`close = (body_start..len).find(|j| is_run_of(chars, *j, '`', 3))`. Et
`is_run_of:439-444` **refuse un run précédé du marqueur** — donc dans ` ```` ` (quatre
backticks) l'ouverture est au premier et le quatrième appartient au corps. Un fence
sans closer n'est **pas** reconnu : ses backticks restent `Plain` verbatim
(`mika2291_unclosed_fence_stays_verbatim`).

**F2 — la fermeture ajoutée doit être `"\n```"`, et le `\n` n'est pas décoratif.**
`strip_fence_decoration:290-298` jette la première ligne du corps quand elle ne
contient aucun whitespace (l'info-string), **puis** un seul `\n` final
(`strip_suffix('\n')`). Une fermeture ajoutée sans son `\n` ferait manger par ce
`strip_suffix` le `\n` *d'origine* du texte quand la coupe tombe en fin de ligne — le
cas préféré — et la réunion des morceaux perdrait un caractère. Avec `"\n```"`, le `\n`
jeté est celui qu'on vient d'ajouter :

| part gauche | morceau émis | corps lu par `tokenize` |
|---|---|---|
| `a\n` | ` ```rust\na\n ` + `\n``` ` | `a\n` ✓ |
| `a` | ` ```rust\na ` + `\n``` ` | `a` ✓ |

**F3 — la réouverture est `format!("```{label}\n")`, étiquette vide comprise.** La
même règle de F2 jette la première ligne si elle est sans whitespace : `rust` est
jetée comme info-string, et une première ligne **vide** l'est aussi. Donc rouvrir avec
` ``` ` + `\n` préserve le corps exactement, qu'il y ait une étiquette ou non. Et
l'étiquette d'origine se lit par la même règle : première ligne du corps, si elle ne
contient aucun whitespace. Une ouverture de la forme ` ```let x = 1; ` n'a donc **pas**
d'étiquette — tout est code — et rouvre sans étiquette.

Corollaire mesuré du ticket : `parse_fence` jette l'étiquette au rendu HTML
(`SegmentKind::Pre`). L'étiquette répétée sert au **texte source** et au repli brut de
mika#2291, pas au `<pre>`.

## Conception

### C1 — Un scanner de fences local à `telegram_split.rs`

`Segment` ne porte **aucune position source** (fait B du plan du vol 1) et
`telegram_markdown.rs` est hors périmètre : la position d'ouverture d'un fence et son
étiquette ne sont donc obtenables que par un scan local.

```rust
/// Un bloc clôturé *fermé*, tel que `parse_fence` le lit, en octets.
struct FenceSpan {
    body: usize,    // premier octet du corps (juste après les trois backticks d'ouverture)
    close: usize,   // premier octet du run de trois backticks fermant
    label: String,  // l'étiquette de langage telle qu'écrite, `String::new()` si absente
}

fn fence_spans(text: &str) -> Vec<FenceSpan>;
```

Le scan réplique le squelette de `tokenize` **restreint à l'alphabet backtick**, dans
le même ordre de priorité : à chaque position, un run de trois backticks non précédé
d'un backtick ouvre un fence s'il a un closer (F1) ; sinon un backtick isolé consomme
un code en ligne s'il a un closer non adjacent (`parse_inline_code:303-318`), ce qui
décale l'appariement des backticks suivants et doit donc être répliqué ; sinon on
avance d'un octet. Les backticks sont ASCII, donc le scan peut travailler sur
`as_bytes()` sans risque de frontière UTF-8 : aucun octet de continuation ne vaut
`0x60`.

**Le risque nommé de ce scanner est la divergence avec `tokenize`**, et il est gardé
par un test d'accord plutôt que par relecture : `fence_spans(t).len()` doit égaler le
nombre de segments `SegmentKind::Pre` de `tokenize(t)`, sur les cas adverses (backtick
isolé avant un fence, quatre backticks, fence non fermé, fence sans étiquette, deux
fences successifs). C'est le même geste que le contrôle d'anti-vacuité du crate : une
réplique sans test d'accord est une seconde grammaire qui dérive en silence.

### C2 — Le plancher dans un fence est le seul cas que ce vol change

L'aiguillage est écrit pour que **le chemin du vol 1 reste premier**, ce qui rend
l'AC3 (« les tests existants restent verts sans retouche ») vrai par construction et
non par inspection :

1. `hard_boundary(rest, budget)` est extraite de `choose_cut` (même calcul, aucun
   changement de comportement) pour être appelable par l'aiguillage.
2. `choose_cut(rest, budget)` est appelée **inchangée**. Si elle rend autre chose que
   le plancher, le morceau est émis comme aujourd'hui.
3. Le vol 2 n'intervient que si `cut == hard` **et** qu'un `FenceSpan` contient
   `offset + hard` (`span.body <= pos < span.close + 3`). Alors seulement : réserve de
   fermeture, coupe dans le corps, fermeture et réouverture.

Cet ordre a une conséquence voulue : quand une coupe neutre existe **avant** le fence
(prose puis bloc long), elle est préférée et le bloc n'est pas touché à ce tour — c'est
l'AC1(d), gratuite. Et un fence **non fermé** ne produit aucun span (F1), donc
l'AC1(e) est elle aussi le chemin d'aujourd'hui, sans clôture ajoutée.

### C3 — Le budget réserve la place des ajouts avant de choisir la coupe

La boucle de `split_outbound_text` porte deux états de plus : `offset`
(`text.len() - rest.len()`, pour lire les spans) et `reopen: Option<String>`
(l'étiquette à rouvrir, posée par le tour précédent).

```
budget = limit − prefix_units − reopen_units            (reopen_units = 0 si None)
chemin fence : budget_corps = budget − CLOSE_UNITS      (CLOSE_UNITS = 4, "\n```")
```

- `reopen_units = text_len_utf16("```{label}\n")` = `3 + label + 1`.
- Dans le corps, la coupe tombe de préférence au dernier `\n` de
  `rest[span_body_rel..hard_corps]`, sinon au plancher — jamais dans un `char` ni dans
  une paire de substitution, puisque `hard_boundary` accumule `c.len_utf16()`.
- Le morceau émis est `format_outbound_text(agent, &format!("{open}{corps}\n```"))` —
  **le préfixe garde son site de composition unique**, propriété du vol 1 qu'il serait
  facile de perdre ici.
- Sur un fence couvrant `n` morceaux : `n−1` fermetures ajoutées et `n−1` réouvertures.
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

appelée par `handle_send` sur le bras `Some(e)`. **Aucun statut ne change et aucun
`warn!` ne disparaît** : l'extraction existe pour que l'AC2 puisse affirmer « le statut
rendu est celui d'avant le découpage » sur un `TelegramApiError` fabriqué, sans monter
un `AppState` ni un Postgres.

## Non-goals

Transcrits du § *Hors de ce vol* de la tête, sans ajout :

- `telegram_markdown.rs` (recognizer, rendu, repli) et la garde des 50 000 octets.
- La relance de `GatewaySender` et le doublon partiel qu'elle produit.
- La garde de l'outil `send_message` et son message au modèle.
- `mika notify` et son adresse de gateway par défaut — ce défaut attend un mot de
  Vincent pour #2983.
- `.github/workflows/`, `docs/egress/`.
- **CODEOWNERS** : aucun fichier visé n'y figure ; la PR peut se fermer sans geste de
  samidarko au titre du périmètre.

## Verification contract

Tous les tests sont des tests unitaires du crate, sans réseau ni Postgres.

### Dans `telegram_split.rs`

Un helper partagé porte les assertions communes des six cas — limite respectée,
clôtures équilibrées par morceau, réunion sans perte ni réordonnancement :

```rust
fn assert_chunks_wellformed(chunks: &[String], prefix: Option<&str>, limit: usize, input: &str);
```

- **T1 (AC1(c)(a))** — un bloc ` ```rust ` de plus de 4096 unités, préfixe
  `[mika-dev] ` : chaque morceau ≤ 4096 unités clôtures ajoutées comprises ; chaque
  morceau portant une part du bloc l'ouvre en ` ```rust ` ; la réunion, préfixes et
  clôtures ajoutées retirés, rend l'entrée. **Contrôle négatif** : la même fixture
  passée au chemin du plancher (`choose_cut` seul, sans l'aiguillage de C2) donne au
  moins un morceau aux clôtures déséquilibrées — la forme de `origin/main` à
  `ebf7e6e0`, vérifiée et non seulement commentée.
- **T2 (AC1(c)(b))** — un bloc couvrant au moins trois morceaux.
- **T3 (AC1(c)(c))** — un bloc sans étiquette : la réouverture est sans étiquette.
- **T4 (AC1(c)(d))** — un bloc qui tient dans un morceau, prose avant et après : le
  bloc n'est pas coupé, aucune clôture ajoutée.
- **T5 (AC1(c)(e))** — une clôture non fermée de plus de 4096 unités : aucune clôture
  ajoutée, le texte sort au plancher comme aujourd'hui.
- **T6 (AC1(c)(f))** — `tokenize` de chaque morceau lit un segment `Pre` pour sa part
  du bloc : le code est rendu, pas affiché en brut.
- **T7 (accord du scanner, § C1)** — `fence_spans(t).len()` égale le nombre de segments
  `Pre` de `tokenize(t)` sur six entrées adverses.

### Dans `routes.rs`

- **T8 (AC3)** — un envoi faux qui accepte le morceau 1 et refuse le morceau 2, sur un
  texte de trois morceaux : `outcome.delivered.len() == 1`, `outcome.failure` présent,
  l'envoi faux appelé **exactement deux fois** (le morceau 3 n'est jamais tenté), une
  ligne `telegram_send_chunk_failed` capturée portant `chunk_index = 1`,
  `chunk_count = 3`, `delivered = 1`, et **le corps du message absent de toute ligne
  capturée** (mika#2126). Capture par une couche `tracing` de test sur le modèle
  `egress_search/tests_e4_no_log.rs` (`SharedBuffer` + `MakeWriter`,
  `tracing-subscriber` déjà en dev-dépendance). **Contrôle négatif dans le même
  test** : avec un envoi qui accepte tout, le même texte donne trois envois et aucune
  ligne `telegram_send_chunk_failed`.
- **T9 (AC3, statut)** — `status_for_send_failure` rend 410 sur `BotBlocked`, 429 avec
  l'en-tête `retry-after` sur `RateLimited { retry_after: Some(n) }`, 429 sans en-tête
  sur `retry_after: None`, 502 sur le reste.

### Commandes

```
cargo test -p mika-gateway
cargo clippy --all-targets -- -D warnings
bash scripts/verify-pipeline.sh origin/main
```

## Fire-Disposition

Ce plan livre un détecteur : l'assertion de non-fuite du corps de T8 (« le corps du
message n'apparaît dans aucune ligne capturée »), dont le chemin de succès est
« aucune occurrence trouvée ». Les autres tests vérifient un comportement sur une
entrée donnée et ne sont pas des détecteurs.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est vide.**
Le détecteur atterrit **armé**, sans aucune exception, parce qu'il n'existe aucune
violation à excepter : `send_in_chunks` ne journalise aujourd'hui aucun champ portant
le corps (`routes.rs:2704,2708` — `chat_id`, `request_id`, `chunk_index`,
`chunk_count`, `delivered`, `error`), et la mesure se fait au niveau du **rendu** de
la couche `fmt`, pas de la liste des champs. C'est la forme que ce crate utilise
déjà : `SEND_MESSAGE_LITERAL_ALLOWED` « ships empty and stays empty », et la
résolution d'un rougissement est de retirer le champ fautif, jamais d'ajouter une
entrée.

**Anti-vacuité, pour que « rien trouvé » se distingue de « rien regardé »**
(mika#2205) : T8 assure d'abord que la capture est non vide **et** qu'elle contient la
ligne `telegram_send_chunk_failed` attendue, avant d'affirmer l'absence du corps. Sans
cette assertion préalable, un subscriber mal câblé rendrait le test vert en ne
capturant rien.

## Definition of Done

- `split_outbound_text` ferme et rouvre un bloc clôturé plus long que le budget, avec
  la même étiquette, les ajouts comptés dans la limite.
- Un texte sous la limite garde le chemin d'aujourd'hui, au même octet près.
- T1–T9 verts ; les tests existants de `telegram_split.rs`, `routes.rs` et
  `telegram_markdown.rs` verts **sans retouche**.
- `cargo test -p mika-gateway`, `cargo clippy` et les lints du dépôt verts.
- Doc à jour : `crates/mika-gateway/CLAUDE.md` § *Outbound Splitting* (le bloc clôturé
  n'est plus un reste nommé ; la règle de fermeture/réouverture est décrite ; la sonde
  n'a plus de réserve sur « le code lisible » ; le reste nommé sur `GatewaySender` et
  celui de l'outil `send_message` restent) et le module doc de `telegram_split.rs`
  (le cas du bloc clôturé). Le plan du vol 1 n'est pas réécrit.
- PR portant `Refs senara-solutions/mika#2983` et `Refs senara-solutions/mika#3114`,
  **sans** `Closes` / `Fixes` / `Resolves`.

## Acceptance criteria

Transcrits verbatim de la tête du ticket.

1. AC1(c) dans `telegram_split.rs`, testé sur :
   - (a) un bloc ` ```rust ` de plus de 4096 unités : chaque morceau fait au plus 4096
     unités, préfixe et clôtures ajoutées compris ; chaque morceau a ses clôtures
     équilibrées ; chaque morceau qui porte une partie du bloc l'ouvre en ` ```rust ` ;
     la réunion des morceaux, préfixes et clôtures ajoutées retirés, rend l'entrée sans
     perte ni réordonnancement ;
   - (b) un bloc qui couvre au moins trois morceaux ;
   - (c) un bloc sans étiquette : la réouverture est sans étiquette ;
   - (d) un bloc qui tient dans un morceau, précédé et suivi de prose : le bloc n'est
     pas coupé ;
   - (e) une clôture non fermée de plus de 4096 unités : aucune clôture ajoutée ;
   - (f) `tokenize` de chaque morceau lit un segment `Pre` pour sa partie du bloc : le
     code est rendu, pas affiché en brut.

   Contrôle négatif dans le test (a) : sur `origin/main` (ebf7e6e0), le même texte donne
   au moins un morceau aux clôtures déséquilibrées. Le test le dit en commentaire, ou le
   vérifie par une variante qui appelle le chemin du plancher.
2. AC3 en test, comme décrit au point 2 de la tête, contrôle négatif compris.
3. Les tests existants de `telegram_split.rs`, `routes.rs` et `telegram_markdown.rs`
   restent verts sans retouche. Un texte sous la limite garde le chemin d'aujourd'hui,
   au même octet près.
4. La doc dit la vérité :
   - `crates/mika-gateway/CLAUDE.md` § *Outbound Splitting* : le bloc clôturé n'est plus
     un reste nommé, la règle de fermeture et de réouverture est décrite, la sonde après
     deploy n'a plus de réserve sur « le code lisible ». Le reste nommé sur la relance de
     `GatewaySender` reste, et le message de l'outil `send_message` aussi ;
   - le module doc de `telegram_split.rs` décrit le cas du bloc clôturé ;
   - le plan du vol 1 n'est pas réécrit ; le plan du vol 2 est un fichier neuf sous
     `docs/plans/`.

   Le plafond des CLAUDE.md (100 000 octets, `crates/mika-common/tests/claude_md_size.rs`)
   tient : `crates/mika-gateway/CLAUDE.md` fait 45 717 octets aujourd'hui.
5. `cargo test -p mika-gateway`, `cargo clippy` et les lints du dépôt sont verts.
6. La PR porte `Refs senara-solutions/mika#2983` et `Refs senara-solutions/mika#3114`,
   et aucun `Closes`, `Fixes` ni `Resolves` : #3114 se ferme à la main après le deploy
   et la sonde de ce vol.

## Ordre de commit

1. `refactor(3114): extraire la correspondance erreur → statut de handle_send` — C4,
   aucun statut changé, T9.
2. `test(3114): l'échec au morceau 2 sur 3 est observable` — T8 et sa couche de
   capture. Ce commit teste du comportement **déjà livré** au vol 1 : il est vert sans
   aucun changement de production, ce qui est sa propre démonstration.
3. `feat(3114): fermer et rouvrir un bloc clôturé plus long que la limite` — C1 à C3,
   avec T1–T7.
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

**Halte** : si des clôtures apparentes réapparaissent après le deploy, établir d'abord
que le binaire déployé porte le correctif (classe mika#2340) **avant** de toucher au
découpage.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-gateway/src/telegram_split.rs` (scanner `fence_spans`, extraction `hard_boundary`, réserve + fermeture/réouverture, doc module) | 112 |
| `crates/mika-gateway/src/routes.rs` (extraction `status_for_send_failure`) | 22 |
| tests T1–T7 + helper `assert_chunks_wellformed` (`telegram_split.rs`) | 124 |
| tests T8–T9 + couche de capture `tracing` (`routes.rs`) | 84 |

Total estimé : 342 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 300 lignes estimées — borne validée par Vincent le 2026-11-21 pour ce ticket (groom 300, pilote 420) ; la dérivation mika#2693 `(150 − 27 − 30) × 9 / 3 = 279` est écrite pour mémoire et la tête du ticket la déclare non applicable ici

**Le total est au-dessus des deux bornes, et le plan ne découpe pas.** Ce n'est pas une
omission : la tête du ticket prescrit, pour exactement ce cas, une halte groom sans
coupe — « si le plan estime le vol au-dessus de 300 lignes, le groom s'arrête, écrit la
halte et le chiffre, et ne propose aucune coupe », et « il ne renvoie rien à un vol 3 ».
La disposition de ce plan est donc la halte du § *Halte groom*, et la suite appartient à
l'opérateur. Les deux restes du vol 1 demeurent tracés par
`Tracked in: senara-solutions/mika#2983`.

Les lignes comptées incluent les doc-comments Rust, que `git diff --shortstat` compte et
que la densité documentaire de ce dépôt rend non négligeables. `crates/mika-gateway/CLAUDE.md`
est hors compte par la règle mika#2636 ; son édition est estimée à ~22 lignes
supplémentaires au titre de la borne du ticket (`':!docs/plans'`), d'où la mesure
attendue de ~364 lignes.

## Gel des tickets (jusqu'au 01/12)

Ce vol n'ouvre aucun ticket, pas même de suivi. Tout reste part en commentaire sur #2983
ou dans la PR, noté `Tracked in: senara-solutions/mika#2983`.
