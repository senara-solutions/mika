# mika#1727 — vol V2b-2b-2 : halte sur la borne par vol

> **Verdict du groom : HALTE.** Le périmètre écrit de ce vol mesure
> **395 lignes** hors `docs/`, pour une borne par vol de **279**.
> Le corps du ticket ordonne la conduite à tenir : « Si le total dépasse 279, le
> groom s'arrête et le dit. **Il ne recoupe pas lui-même** : ce vol est déjà le
> recoupage. » Ce plan est donc la remontée, pas la conception.
>
> Il porte quatre choses et rien d'autre : la mesure, livrable par livrable ; le
> **plancher** du but, qui est le fait dur de cette halte ; la raison pour
> laquelle aucune compression des tests ne rattrape l'écart ; et les arbitrages
> candidats, **chiffrés et nommés sans être choisis** — le choix appartient à
> Vincent.

---

## 0 — Ce qui est mesuré, avant toute conception

Deux révisions, lues et non rappelées de mémoire.

**Sur `766a62788` (`main`).** Tout ce que le corps du ticket décrit sous « Ce qui
est sur main » est exact, vérifié site par site :

| affirmation du corps | site vérifié |
|---|---|
| `TaskEventStream::open` / `consume` sont `pub(crate)` | `crates/mika-cli/src/commands/tasks.rs:523` et `:565` |
| `fetch_task_detail` et `TaskDetailMirror` sont privés au module, six champs | `tasks.rs:611-618` et `:632` |
| le détail échoue explicitement sur un statut non-succès ou un corps illisible | `tasks.rs:643-650` |
| `fake_spirit` a deux constructeurs, chacun sur son serveur | `tasks.rs:2057` (`spawn`), `:2088` (`spawn_detail`) |
| `poll_callback_tasks` réclame par `mark_task_delivered` et livre | `tui/app.rs:1596`, `:1627`, `:1646` |
| `app_with_response_sender` bâtit une `App` sur une base en mémoire | `tui/app.rs:2158` |
| `TaskEventFrame` a **huit** variantes | les huit, énumérées au `match` du matériau |

**Les six champs du miroir, nommément** — ce point gouverne tout le § 3 :
`id`, `agent_id`, `label`, `status`, `created_by_session`, `result`
(`tasks.rs:611-618`). Ni `trigger_type`, ni `action_type`, ni `created_trace_id`,
ni `parent_task_id`. Le critère 1 exige que le `CallbackResult` porte
« `trace_id` et `parent_task_id` **tirés du détail** » : **le miroir à dix champs
est donc obligatoire au but écrit**, et non un confort.

Deux points que le corps ne dit pas et qui pèsent sur la mesure :

- `app_with_response_sender` **jette** le receveur des `AgentRequest`
  (`let (agent_tx, _request_rx) = …`, `tui/app.rs:2179`). Le critère 1 exige
  d'observer un `AgentRequest::CallbackResult` et ses six champs : le helper doit
  rendre ce receveur, ce qui touche sa signature et ses appelants.
- **`poll_callback_tasks` n'a aucun test** (`grep` sur `tui/app.rs` : quatre
  occurrences, toutes de production). Il n'existe donc, côté `tui/app.rs`, aucun
  helper d'ensemencement d'une ligne de callback à réutiliser. Celui de ce vol
  part de zéro, et c'est le poste F1 ci-dessous.

**Sur `refs/pull/3069/head` (matériau), lu au diff et non estimé :**

- `23525549c` — le miroir passe à dix champs, `pub(crate)` champ par champ, avec
  ses tests de format. **`+41 −14`, mesuré** (`git diff --numstat`). Le jugement
  du corps tient : reprenable tel quel.
- `5321ba47d` — **`+298` sur trois fichiers** (`chat.rs` 19, `tasks.rs` 181,
  `tui/app.rs` 111), **aucun test**. Deux de ses morceaux sortent du périmètre de
  ce vol et ne sont pas comptés : le constructeur factice
  `spawn_stream_and_details` (55 lignes — le corps interdit un constructeur neuf)
  et le helper `servir` qui le sert (13). Un troisième allège : tout ce qui fait
  voyager l'`original_status` et le repli `FAILED_TASK_FALLBACK` sert les
  `failed`, que ce vol exclut.

### 0.1 — La calibration de l'estimateur, mesurée sur le vol précédent

Le vol V2b-2b-1 a écrit `Total estimé : 241 lignes`
(`docs/plans/2026-11-10-001-…-plan.md:539`) et a livré, hors `docs/plans/` :

```
$ git diff --numstat dbc6d2b01..766a62788
6    0   crates/mika-cli/src/cli.rs
272  1   crates/mika-cli/src/commands/tasks.rs
1    1   docs/egress/egress-manifest.toml
```

**279 lignes ajoutées pour 241 estimées — une sous-estimation de 16 %**, et un
atterrissage exactement sur la borne. Ce n'est pas une anecdote : même fichier,
même style de commentaire, même auteur de plan. La mesure ci-dessous est moins
exposée à ce biais — 41 de ses lignes sont mesurées au commit et ~165 sont lues
au diff du matériau — mais ses 185 lignes de tests, elles, n'ont jamais été
écrites et portent le biais entier.

---

## 1 — La mesure, livrable par livrable

Le découpage suit l'ordre de commit que le corps impose — « le pilote commite
après chaque garde verte avec son test ». La colonne « source » dit ce qui est
mesuré et ce qui est estimé, pour que chaque ligne soit contestable séparément.

### Commit A — le miroir grandit avec son lecteur

| livrable | source | lignes |
|---|---|---|
| reprise de `23525549c`, tests de format compris | **mesuré** | 41 |

### Commit B — l'abonnement (`tasks.rs`, `chat.rs`)

| livrable | source | lignes |
|---|---|---|
| `terminal_callback_frame` — `match` exhaustif sur huit variantes, sans bras `_` | matériau 25, comprimé | 20 |
| `CallbackFeed` + `spawn_callback_subscription` | matériau 40, + struct, doc comprimée | 43 |
| `TaskDetailMirror::is_resume_callback_for` | matériau 26, doc comprimée | 18 |
| commentaire du miroir ajusté à son nouveau lecteur | **mesuré** | 2 |
| site d'abonnement dans `chat::run` | matériau 19, comprimé | 14 |
| tests — T1 abonnement filtrant (une séquence SSE multi-trames), T2 prédicat (cinq cas) | estimé | 57 |

**Sous-total B : 154.**

Trois notes, parce que ces chiffres sont déjà le résultat d'une compression et
non du matériau brut :

- `terminal_callback_frame` ne rend plus le couple `(task_id, original_status)`
  du matériau mais le `task_id` seul : `TaskFailed` étant hors de ce vol,
  l'`original_status` vaut toujours `"completed"` et n'a pas à voyager. Les
  12 lignes de code restent dues au `match` exhaustif sur les huit variantes, que
  le corps exige sans bras `_`.
- Le matériau pose **trois** champs sur `App` (`task_event_rx`,
  `spirit_base_url`, `spirit_token`). Ce plan les regroupe en un `CallbackFeed` :
  le trio est indissociable — le drainage exige les trois ou sort — et le
  regroupement économise quatre lignes d'initialisation et supprime le
  `let (Some, Some, Some) = … else` triple. Le gain net est de 5 lignes : la
  struct coûte ce que les champs épars coûtaient.
- Les commentaires sont déjà coupés : 60 lignes de doc sur les 395, soit 15 %,
  contre 39 % dans `crates/` de #3069 (131 sur 338, chiffre du corps).

### Commit C — le drainage (`tui/app.rs`)

| livrable | source | lignes |
|---|---|---|
| champ `callback_feed` + les deux initialisations de constructeur | matériau 23, comprimé | 7 |
| `drain_callback_deliveries` (les quatre gardes) | matériau 78, `failed` retiré, doc comprimée | 57 |
| site d'appel au tick, au repos, hors mode équipe | **mesuré** au matériau | 8 |
| helper d'ensemencement d'une ligne de callback en base | estimé, **plancher** | 30 |
| `app_with_response_sender` rend le receveur + ses appelants | estimé | 8 |
| tests — T3 à T6 (voir § 1.1) | estimé | 90 |

**Sous-total C : 200.**

Le helper d'ensemencement coûte 30 lignes et c'est un plancher, pas une
approximation : `create_task` prend un `NewTask` de **22 champs sans `Default`**
(`crates/mika-agent/src/db/tasks.rs:99`), et le seul précédent du dépôt sur
exactement cette forme de ligne — `seed_callback_row`, `tasks.rs:1509-1562` —
en fait la démonstration en 53 lignes, dont 23 pour le seul littéral.

### 1.1 — Le jeu de tests est déjà minimal

Le corps est sans ambiguïté : « Chaque garde arrive avec ses tests, dans le même
commit. Les tests ne se reportent jamais à un vol suivant. » Les cinq critères
exigent huit propriétés distinctes. Voici le groupement le plus serré qui les
couvre toutes, et il tient en **six** tests :

| propriété exigée | critère | test | où |
|---|---|---|---|
| une trame d'un autre agent ne passe pas | 2 | T1 | `tasks.rs` |
| `TaskFailed`, `Unknown`, `OverflowMarker` ignorées sans casser l'abonnement | 3 | T1 | `tasks.rs` |
| `callback` / `resume_agent` exigés | 2 | T2 | `tasks.rs` |
| un `completed` n'est livré qu'une fois | 1 | T3 | `app.rs` |
| rien si le poll a déjà réclamé | 1 | T4 | `app.rs` |
| une autre session n'est ni livrée ni réclamée | 2 | T5 | `app.rs` |
| détail en erreur HTTP ou illisible : ni livré ni réclamé, ligne encore `completed` | 3 | T6 | `app.rs` |

T1 porte quatre propriétés en une seule séquence SSE, et T6 porte ses deux cas en
un test paramétré : la compression des tests est **déjà appliquée** dans le
chiffre de 147 (57 + 90). Elle ne peut aller plus loin sans perdre une propriété,
et le corps interdit de perdre une propriété.

Aucun constructeur neuf de serveur factice n'entre : T1 se sert de `spawn`, T3 à
T6 de `spawn_detail` avec un canal bâti par le test. C'est ce que le corps
demande, et le plan n'a pas de critère qui échoue sans un constructeur neuf.

---

## 2 — Le fait dur : le plancher du but dépasse déjà, avant les tests

C'est ici que cette halte se distingue d'un simple dépassement de périmètre.

Le code **structurellement nécessaire au but écrit** — un `completed` arrive par
le SSE, se lit par HTTP, se réclame, se livre une fois — est la somme des postes
hors tests :

```
41  miroir à dix champs        (critère 1 : trace_id + parent_task_id du détail)
20  terminal_callback_frame    (le fil dit quelle tâche)
43  CallbackFeed + abonnement  (une ouverture, un canal)
18  is_resume_callback_for     (le prédicat du poll, garde 2)
 2  commentaire du miroir
14  site d'abonnement          (chat::run, pas run_team)
 7  champ + inits
57  drain_callback_deliveries  (les quatre gardes)
 8  site d'appel au tick
--
210  plancher, hors tout test
```

La borne est à 279. **Il reste 69 lignes pour l'ensemble des tests** — dont 30
sont prises par le seul helper d'ensemencement, qui est un plancher mesuré. Soit
**39 lignes pour six tests et huit propriétés**, environ 6 lignes par test, sans
compter le receveur à rendre.

Ce n'est pas une marge serrée, c'est l'absence de marge. Et le corps ferme la
seule sortie qui resterait : « Si le budget ne tient pas avec les tests, c'est le
périmètre qui se réduit, **jamais les tests**. »

Les deux autres leviers sont déjà tirés :

- **Les commentaires.** Déjà à 15 % contre 39 % au matériau. Les ramener à zéro —
  ce qui serait un plan refusé en revue, et à juste titre — laisserait **335**.
  Toujours au-dessus.
- **Le rangement des tests.** Migrer vers `tasks.rs` tout ce qui se teste sans
  base (T2, T6, et le prédicat de T5) économise le setup `App` de trois tests,
  ≈ −25, et coûte une fonction d'enveloppe, ≈ +8. Net **−17**, à 378. La porte
  « il suffisait de mieux ranger les tests » est donc fermée par la mesure, et
  non par un avis.

### 2.1 — La coupe que mika#2205 interdit

La coupe qui vient d'abord à l'esprit — l'abonnement dans ce vol, le drainage
dans le suivant — est **interdite**, et le corps la ferme nommément : « Un
abonnement dont personne ne draine le canal, ou un drainage sans livraison, ne
passe pas non plus (mika#2205). » Elle n'est pas une coupe candidate, et ce plan
ne la propose pas.

Son corollaire, qui est le résultat de ce groom : puisque le plancher
bout-en-bout vaut 210 et que mika#2205 exige que chaque PR soit utile de bout en
bout, **aucun découpage en vols ne ramène ce but sous 279**. Un découpage en deux
PR dont la première serait le miroir seul (41) laisse 354 à la seconde. Le choix
qui reste n'est donc pas un choix de taille : c'est l'un des trois arbitrages
ci-dessous.

---

## 3 — Les arbitrages candidats, chiffrés et non choisis

Ils sont la matière de la décision de Vincent. Aucun n'est retenu ici.

### A1 — Le miroir part seul, et part aujourd'hui

Le commit `23525549c` (`+41 −14`, relu par MPC, jugé sain) est **mergeable
seul**, et ses lecteurs sont immédiats et déjà sur `main` : les trois formats de
`mika tasks get <id> --spirit` rendent les quatre champs neufs en `json` et
`yaml` dès ce commit. Il ne porte aucun champ « pour plus tard » — l'énoncé
« chaque champ a son lecteur » survit sans l'abonnement, et mika#2205 est
satisfait.

Reste à livrer : **354**. L'arbitrage ne suffit pas seul, mais il sort 41 lignes
du chemin critique, sans risque, et il est le seul des trois qui soit
livrable immédiatement.

### A2 — Le prédicat se lit sur la base, et non sur le détail HTTP

Au lieu de `is_resume_callback_for` sur le détail, le drainage rapproche le
`task_id` que le fil annonce de la liste que
`get_undelivered_callback_tasks_for_session` rend déjà — code en place, déjà
testé, et qui porte le prédicat du poll **par construction** plutôt que par
copie.

Gain : −18 (`is_resume_callback_for`) −22 (T2) = **355**.

**Correction d'un chiffrage antérieur.** La version précédente de ce plan
créditait cet arbitrage de −41 de plus, en supposant que le miroir sortait avec
le prédicat, et l'annonçait à ≈ 264 — sous la borne. **C'est faux, et le § 0 en
donne la raison :** les six champs du miroir sur `main` ne comprennent ni
`created_trace_id` ni `parent_task_id`, que le critère 1 exige « tirés du
détail ». Le miroir à dix champs reste donc obligatoire tant que le critère 1
tient, et cet arbitrage **ne passe pas** sous la borne.

### A3 — Le détail HTTP sort du chemin : le fil devient un réveil

C'est le seul arbitrage mesuré **sous la borne**, et il a un prix explicite.

La ligne locale porte déjà tout ce que la livraison lit — `result`, `label`,
`created_trace_id`, `parent_task_id` — et le drainage doit de toute façon la
toucher pour `mark_task_delivered`. Si le détail HTTP sort, le fil n'apporte plus
que la **latence** : le réveil au tick au lieu des ~5 s du poll.

| poste | lignes |
|---|---|
| `terminal_callback_frame` | 20 |
| `CallbackFeed` + abonnement | 43 |
| site d'abonnement `chat.rs` | 14 |
| champ + inits | 7 |
| drainage sans lecture ni prédicat (réutilise la liste du poll) | 35 |
| site d'appel au tick | 8 |
| tests — T1 abonnement, puis livré-une-fois, déjà-réclamé, autre-session | 103 |

**Total ≈ 230**, miroir **non** compris : A3 ne le requiert plus du tout, et les
quatre champs neufs ne valent alors que pour `mika tasks get --spirit`, c'est-à-dire
par A1 et pour lui seul.

**Son prix :** le but écrit du vol est « se lit par HTTP, se réclame et se livre ».
A3 garde « se réclame et se livre » et abandonne « se lit par HTTP ». Il fait
tomber les critères 1 (partiellement) et 3, et il supprime au passage la
duplication du prédicat que le matériau assume. **Ce n'est pas un arbitrage de
taille, c'est un arbitrage d'architecture** — et c'est pourquoi le groom le
remonte au lieu de le prendre.

### A4 — La borne de ce vol est relevée explicitement

La borne de 279 dérive d'un plafond de 150 tours (`(150 − 27 − 30) × 9 / 3`, site
de vérité `_plan_size_per_flight_bound` dans
`skills/bundled/_shared/dispatch-lib.sh`). Le label `loop-substrate` porterait le
plafond à 200 tours, donc la borne à `(200 − 27 − 30) × 9 / 3 = 429`.

Avec A1 appliqué (le miroir en PR séparée), le vol tombe à **354** pour une borne
de 429 : une marge de 75 lignes, soit 21 %. C'est **au-dessus** du biais de 16 %
mesuré au § 0.1, et c'est ce qui distingue cet arbitrage d'un pari.

Sans A1, le vol est à 395 pour 429 : une marge de 8 %, **sous** le biais mesuré.
Le couple **A4 + A1** est donc la seule combinaison dont la mesure soutienne la
marge ; A4 seul atterrirait là où le vol précédent a atterri — à la borne exacte.

---

## 4 — Ce que ce groom n'a pas fait

- **Il n'a pas recoupé le périmètre.** Le corps le lui retire explicitement.
- **Il n'a ouvert aucun ticket**, pas même de suivi : gel des tickets jusqu'au
  01/12. La suite se note `Tracked in: senara-solutions/mika#1727`.
- **Il n'a touché aucun fichier hors `docs/plans/`.** Rien sous
  `.github/workflows/`, `.claude/`, `skills/bundled/`,
  `crates/mika-agent/src/well_known_agents.rs`, `.mika/` ni `docs/egress/`.
  `crates/mika-agent/` et `crates/mika-a2a/` sont intacts.
- **Il n'a écrit aucun code.** Ce plan ne livre **aucun détecteur** : le gate
  `## Fire-Disposition` (mika#2306) est **N/A** et n'est donc pas inventé. Il
  redeviendra dû sur le plan qui livrera les six tests du § 1.1.
- **Il n'a pas ouvert un second plan concurrent.** Ce fichier est la mise à jour
  du plan commité à 15:03Z, dont la mesure portait sur un corps antérieur à sa
  révision de 15:20Z. Son verdict est confirmé par une mesure indépendante ; deux
  de ses chiffrages sont corrigés (§ A2, et 395 au lieu de 370).

---

## Definition of Done

La DoD de **ce plan** est la remontée, non l'implémentation :

1. Le plan est commité sur `feat/1727/tui-v2b2b2-callbacks-termines-par-le-sse`.
2. Il porte la mesure livrable par livrable (§ 1), chaque ligne étiquetée
   « mesuré » ou « estimé » pour être contestable séparément.
3. Il porte le **plancher** du but écrit, hors tests, et le budget de tests qui
   en reste (§ 2) — c'est le fait dur de la halte.
4. Il porte la calibration de l'estimateur sur le vol précédent (§ 0.1), mesurée
   et non rappelée.
5. Il porte la coupe que mika#2205 interdit (§ 2.1) et son corollaire : aucun
   découpage en vols ne ramène ce but sous la borne.
6. Il porte les quatre arbitrages candidats avec leur gain mesuré, **sans en
   choisir aucun** (§ 3), en nommant celui qui passe (A3) et le couple dont la
   marge est soutenue par la mesure (A4 + A1).
7. Il porte sa section `## Taille estimée` au format de mika#2636, avec le total
   réel — y compris quand ce total est ce qui déclenche la halte.

**Ce qu'aucune de ces lignes n'autorise :** écrire une ligne de
`crates/mika-cli/`. Le prochain geste appartient à Vincent.

---

## Acceptance criteria

Transcrits verbatim du corps de mika#1727 (§ « Critères de V2b-2b-2 »). Ils sont
les critères du **vol**, que l'implémentation devra tenir une fois l'arbitrage
tranché — ils ne sont pas la DoD de ce plan, qui est la halte ci-dessus.

1. Un callback `completed` de la conversation en cours atteint la TUI en
   `AgentRequest::CallbackResult` **une seule fois**. Il porte
   `original_status = "completed"`, le résultat lu par
   `GET /api/v1/tasks/{task_id}`, et `trace_id` et `parent_task_id` tirés du
   détail. Un test rougit si le même candidat est livré deux fois. Un autre
   rougit s'il est livré alors que le poll l'a déjà réclamé.
2. Rien n'est livré ni réclamé dans trois cas : une tâche d'une **autre
   session** du même agent, une tâche qui n'est pas `callback` / `resume_agent`,
   ou une trame d'un **autre agent**. Chacun des trois a son test.
3. Une trame `TaskFailed`, `Unknown` ou `OverflowMarker` ne mène à aucune
   livraison et ne casse pas l'abonnement : un `TaskCompleted` qui la suit
   arrive encore. Un détail en erreur HTTP ou illisible n'est ni livré ni
   réclamé, et la ligne reste `completed` pour le poll. Chaque cas a son test.
4. `poll_callback_tasks` n'est pas modifié. Egress Manifest Lint est vert **avec
   `docs/egress/` intact** : le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il
   le trouve rouge, il s'arrête et le dit, au lieu de toucher le manifeste.
5. La PR porte `Refs mika#1727` et aucun `Closes`, `Fixes` ni `Resolves`.

**État de ces critères sous ce plan : aucun n'est tenu, et aucun ne peut l'être** —
le plancher qui les porte mesure 210 lignes hors tests, pour une borne de 279 dont
30 sont prises par le seul helper d'ensemencement. Le § 1.1 établit la
correspondance critère → test, afin que l'arbitrage retenu puisse être jugé sur ce
qu'il ferait perdre : A3 est le seul qui passe, et il coûte le critère 3 et une
part du critère 1.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-cli/src/commands/tasks.rs` — miroir à dix champs (reprise de `23525549c`, mesuré) | 41 |
| `crates/mika-cli/src/commands/tasks.rs` — `terminal_callback_frame` | 20 |
| `crates/mika-cli/src/commands/tasks.rs` — `CallbackFeed` + `spawn_callback_subscription` | 43 |
| `crates/mika-cli/src/commands/tasks.rs` — `is_resume_callback_for` | 18 |
| `crates/mika-cli/src/commands/tasks.rs` — commentaire du miroir ajusté | 2 |
| `crates/mika-cli/src/commands/chat.rs` — site d'abonnement | 14 |
| tests (`crates/mika-cli/src/commands/tasks.rs`) — T1 abonnement, T2 prédicat | 57 |
| `crates/mika-cli/src/tui/app.rs` — champ `callback_feed` + deux inits | 7 |
| `crates/mika-cli/src/tui/app.rs` — `drain_callback_deliveries` | 57 |
| `crates/mika-cli/src/tui/app.rs` — site d'appel au tick | 8 |
| tests (`crates/mika-cli/src/tui/app.rs`) — helper d'ensemencement | 30 |
| tests (`crates/mika-cli/src/tui/app.rs`) — receveur rendu + appelants | 8 |
| tests (`crates/mika-cli/src/tui/app.rs`) — T3 à T6 | 90 |

Total estimé : 395 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Le total dépasse la borne de 116 lignes, soit 1,42×.** Et le dépassement ne
vient pas d'un périmètre trop large : le **plancher** du but écrit, hors tout
test, vaut déjà 210, ce qui laisse 39 lignes pour six tests une fois le helper
d'ensemencement payé (§ 2). Conformément au corps du ticket, le groom **s'arrête
et le dit**, et ne recoupe pas lui-même : ce vol est déjà le recoupage de
mika#3069.

La suite — l'arbitrage tranché, puis le vol qu'il permet — reste portée par le
ticket, le gel des tickets jusqu'au 01/12 interdisant d'en ouvrir un second :

Tracked in: senara-solutions/mika#1727
