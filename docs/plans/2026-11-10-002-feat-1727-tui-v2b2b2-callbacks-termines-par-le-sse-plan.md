# mika#1727 — vol V2b-2b-2 : halte sur la borne par vol

> **Verdict du groom : HALTE.** Le périmètre écrit de ce vol mesure
> **370 lignes** estimées hors `docs/`, pour une borne par vol de **279**.
> Le corps du ticket ordonne explicitement la conduite à tenir : « Si le total
> dépasse 279, le groom s'arrête et le dit. **Il ne recoupe pas lui-même** : ce
> vol est déjà le recoupage. » Ce plan est donc la remontée, pas la conception.
>
> Il porte trois choses et rien d'autre : la mesure, livrable par livrable ; la
> raison pour laquelle aucune compression ne rattrape l'écart ; et les coupes
> candidates, **nommées sans être choisies** — le choix appartient à Vincent.

---

## 0 — Ce qui est mesuré, avant toute conception

Deux révisions, lues et non rappelées de mémoire.

**Sur `766a62788` (`main`).** Tout ce que le corps du ticket décrit sous « Ce qui
est sur main » est exact, vérifié site par site :

| affirmation du corps | site vérifié |
|---|---|
| `TaskEventStream::open` / `consume` sont `pub(crate)` | `crates/mika-cli/src/commands/tasks.rs:523` et `:565` |
| `consume` rend la charge brute **et** la trame décodée, filtrée par agent | `tasks.rs:565-595` |
| `fetch_task_detail` et `TaskDetailMirror` sont privés au module, six champs | `tasks.rs:611-632` |
| `fake_spirit` a deux constructeurs, chacun sur son serveur | `tasks.rs:2057` (`spawn`), `:2088` (`spawn_detail`) |
| `poll_callback_tasks` applique le prédicat du poll, réclame, livre | `tui/app.rs:1596-1700` |
| `app_with_response_sender` bâtit une `App` sur une base en mémoire | `tui/app.rs:2158` |
| `TaskEventFrame` a **huit** variantes | `tasks.rs:frame_agent_id`, `match` exhaustif sur les huit |

Un point que le corps ne dit pas et qui pèse sur la mesure :
`app_with_response_sender` **jette** le receveur des `AgentRequest`
(`let (agent_tx, _request_rx) = …`, `tui/app.rs:2179`). Le critère 1 exige
d'observer un `AgentRequest::CallbackResult` et ses quatre champs : le helper
doit donc rendre ce receveur, ce qui touche sa signature et ses **trois**
appelants.

**Sur `refs/pull/3069/head` (matériau).** Les deux commits, lus en diff :

- `23525549c` — le miroir passe à dix champs, `pub(crate)` champ par champ, avec
  ses tests de format. **`+41 −14`, mesuré.** Le jugement du corps tient : il est
  reprenable tel quel.
- `5321ba47d` — `+298` sur trois fichiers, **aucun test**. Deux de ses morceaux
  sortent du périmètre de ce vol et ne sont donc pas comptés : le constructeur
  factice `spawn_stream_and_details` (≈ 55 lignes — le corps interdit un
  constructeur neuf) et le helper `servir` qui le sert (≈ 13).

Reste donc, du matériau, **≈ 230 lignes de code non testé**, auxquelles s'ajoutent
les 41 du miroir : **≈ 271 avant le premier test.** La borne est à 279.

### 0.1 — La calibration de l'estimateur, mesurée sur le vol précédent

Le vol V2b-2b-1 a écrit `Total estimé : 241 lignes`
(`docs/plans/2026-11-10-001-…-plan.md:539`) et a livré, hors `docs/` :

```
$ git diff --numstat dbc6d2b01..766a62788
6    0   crates/mika-cli/src/cli.rs
272  1   crates/mika-cli/src/commands/tasks.rs
1    1   docs/egress/egress-manifest.toml
```

**279 lignes ajoutées pour 241 estimées — une sous-estimation de 16 %**, et un
atterrissage exactement sur la borne. Ce n'est pas une anecdote : c'est le même
fichier, le même style de commentaire, le même auteur de plan. Appliquée aux
370 lignes ci-dessous, cette calibration annonce un réel de l'ordre de **430**.

L'écart à expliquer n'est donc pas « 370 contre 279 » (1,33×) mais, une fois
l'estimateur corrigé de son biais connu, **≈ 1,5×**. C'est ce facteur-là qui rend
la halte robuste : aucune marge d'incertitude plausible ne le referme.

---

## 1 — La mesure, livrable par livrable

Le découpage suit l'ordre de commit que le corps impose — « le pilote commite
après chaque garde verte avec son test ».

### Commit A — le miroir grandit avec son lecteur

Reprise de `23525549c` : **41** (mesuré, pas estimé).

### Commit B — l'abonnement, dans `tasks.rs` et `chat.rs`

| élément | doc | code | total |
|---|---|---|---|
| `terminal_callback_frame` | 7 | 10 | 17 |
| `CallbackFeed` + `spawn_callback_subscription` | 12 | 24 | 36 |
| `TaskDetailMirror::is_resume_callback_for` | 9 | 8 | 17 |
| site d'abonnement dans `chat::run` | 7 | 6 | 13 |
| tests : abonnement filtrant (une séquence SSE multi-trames) | — | 35 | 35 |
| tests : prédicat du poll, table-driven, cinq cas | — | 22 | 22 |

**Sous-total B : 140.**

Deux notes sur ces chiffres, parce qu'ils sont déjà le résultat d'une
compression et non du matériau brut :

- `terminal_callback_frame` ne rend plus un couple `(task_id, original_status)`
  comme dans le matériau, mais le `task_id` seul : `TaskFailed` étant hors de ce
  vol, l'`original_status` vaut toujours `"completed"` et n'a pas à voyager. Les
  dix lignes de code restent dues au `match` exhaustif sur les **huit** variantes,
  que le corps exige sans bras `_`.
- Le matériau pose **trois** champs sur `App` (`task_event_rx`, `spirit_base_url`,
  `spirit_token`). Ce plan les regroupe en un `CallbackFeed` rendu directement par
  `spawn_callback_subscription` : le trio est indissociable — le drainage exige
  les trois ou sort — et le regroupement économise quatre lignes d'initialisation
  en plus de supprimer le `let (Some, Some, Some) = … else` triple.

### Commit C — le drainage, dans `tui/app.rs`

| élément | doc | code | total |
|---|---|---|---|
| champ `callback_feed` + les deux initialisations de constructeur | 7 | 3 | 10 |
| `drain_callback_deliveries` (les quatre gardes) | 13 | 38 | 51 |
| site d'appel au tick, au repos, hors mode équipe | 5 | 3 | 8 |
| `app_with_response_sender` rend le receveur + ses trois appelants | — | 8 | 8 |
| helper d'ensemencement d'une ligne de callback en base | — | 32 | 32 |
| tests : quatre cas (voir § 1.1) | — | 80 | 80 |

**Sous-total C : 189.**

Le helper d'ensemencement coûte 32 lignes et c'est un plancher, pas une
approximation : `NewTask` porte 22 champs sans `Default`, et le seul précédent du
dépôt — `seed_callback_row`, `tasks.rs:1509-1562` — en fait la démonstration sur
exactement la même forme de ligne.

### 1.1 — Le jeu de tests est déjà minimal

Le corps est sans ambiguïté : « Chaque garde arrive avec ses tests, dans le même
commit. Les tests ne se reportent jamais à un vol suivant. » Les cinq critères
exigent huit propriétés distinctes. Voici le groupement le plus serré qui les
couvre toutes, et il tient en **sept** tests :

| propriété exigée | critère | test | où |
|---|---|---|---|
| un `completed` n'est livré qu'une fois | 1 | T3 | `app.rs` |
| rien si le poll a déjà réclamé | 1 | T4 | `app.rs` |
| une autre session n'est ni livrée ni réclamée | 2 | T5 | `app.rs` |
| `callback` / `resume_agent` exigés | 2 | T2 | `tasks.rs` |
| une trame d'un autre agent ne passe pas | 2 | T1 | `tasks.rs` |
| `TaskFailed`, `Unknown`, `OverflowMarker` ignorées sans casser l'abonnement | 3 | T1 | `tasks.rs` |
| détail en erreur HTTP ou illisible : ni livré ni réclamé, ligne encore `completed` | 3 | T6 | `app.rs` |

T1 porte quatre propriétés en une seule séquence SSE, et T6 porte ses deux cas en
un test paramétré : la compression des tests est **déjà appliquée** dans le
chiffre de 137 (57 + 80). Elle ne peut pas aller plus loin sans perdre une
propriété, et le corps interdit de perdre une propriété.

---

## 2 — Pourquoi aucune compression ne rattrape l'écart

Trois leviers existent. Les trois sont déjà tirés, ou ne suffisent pas.

**Les commentaires.** Le corps le dit : dans `crates/` de #3069, 131 lignes
ajoutées sur 338 étaient des commentaires. Le chiffre de 370 ci-dessus en compte
**60** (les colonnes « doc »), soit 16 % contre 39 % au matériau : la coupe est
déjà faite. Les ramener à zéro — ce qui serait un plan refusé en revue, et à
juste titre — laisserait **310**. Toujours au-dessus.

**Le matériau brut.** Il est à 271 (41 + 230) **avant le premier test**. Il ne
reste donc, sous la borne, que 8 lignes de budget pour sept tests. Ce n'est pas
une marge serrée, c'est l'absence de marge.

**Le périmètre.** C'est le seul levier qui reste, et il est précisément celui que
le corps retire au groom.

### 2.1 — La coupe que mika#2205 interdit

La coupe qui viendrait d'abord à l'esprit — livrer l'abonnement dans ce vol, le
drainage dans le suivant — est **interdite**, et le corps la ferme nommément :
« Un abonnement dont personne ne draine le canal, ou un drainage sans livraison,
ne passe pas non plus (mika#2205). » Elle n'est donc pas une coupe candidate, et
ce plan ne la propose pas.

---

## 3 — Les coupes candidates, nommées et non choisies

Elles sont la matière de la décision de Vincent. Aucune n'est retenue ici.

### Coupe 1 — le miroir part seul, et part aujourd'hui

Le commit `23525549c` (41 lignes, relu par MPC, jugé sain) est **mergeable seul**.
Ses lecteurs sont immédiats et déjà sur `main` : les trois formats de
`mika tasks get <id> --spirit` rendent les quatre champs neufs en `json` et
`yaml` dès ce commit — il ne porte donc aucun champ « pour plus tard », et
l'énoncé « chaque champ a son lecteur » survit sans l'abonnement.

Reste à livrer : **329**. La coupe ne suffit pas seule, mais elle sort 41 lignes
du chemin critique et elle est sans risque.

### Coupe 2 — le prédicat se lit sur la base, et non sur le détail HTTP

Au lieu de `fetch_task_detail` puis `is_resume_callback_for`, le drainage
rapproche le `task_id` que le fil annonce de la liste que
`get_undelivered_callback_tasks_for_session` rend déjà — code en place, déjà
testé, et qui porte le prédicat du poll **par construction** plutôt que par
copie.

Gain estimé : −41 (le miroir n'a plus de lecteur hors de `tasks get`, et repart
en coupe 1) −17 (`is_resume_callback_for`) −22 (T2) −26 (T6, le détail n'étant
plus lu) = **≈ 264**, sous la borne.

**Mais elle touche le but du vol**, qui est écrit « se lit par HTTP, se réclame et
se livre » : le fil deviendrait un réveil, et la lecture resterait locale. Elle
supprime aussi au passage la duplication du prédicat que le matériau assume. Ce
n'est pas un arbitrage de taille, c'est un arbitrage d'architecture — et c'est
pourquoi le groom le remonte au lieu de le prendre.

### Coupe 3 — la borne de ce vol est relevée explicitement

La borne de 279 dérive d'un plafond de 150 tours
(`(150 − 27 − 30) × 9 / 3`, site de vérité `_plan_size_per_flight_bound` dans
`skills/bundled/_shared/dispatch-lib.sh`). Le label `loop-substrate` porterait
le plafond à 200 tours, donc la borne à `(200 − 27 − 30) × 9 / 3 = 429`. Le réel
annoncé par la calibration du § 0.1 — **≈ 430** — tombe sur cette valeur, sans
aucune marge.

Nommée pour être complète, et **non recommandée** : atterrir à la borne exacte
est ce que le vol précédent a fait (279 pour 279), et le faire sciemment sur un
vol 1,5× plus gros revient à parier sur un estimateur dont on vient de mesurer
le biais.

---

## 4 — Ce que ce groom n'a pas fait

- **Il n'a pas recoupé le périmètre.** Le corps le lui retire explicitement.
- **Il n'a ouvert aucun ticket**, pas même de suivi : gel des tickets jusqu'au
  01/12. La suite se note `Tracked in: senara-solutions/mika#1727`.
- **Il n'a touché aucun fichier hors `docs/plans/`.** Rien sous
  `.github/workflows/`, `.claude/`, `skills/bundled/`,
  `crates/mika-agent/src/well_known_agents.rs`, `.mika/` ni `docs/egress/`.
- **Il n'a écrit aucun code.** Ce plan ne livre aucun détecteur : le gate
  `## Fire-Disposition` (mika#2306) est **N/A** et n'est donc pas inventé. Il
  redeviendra dû sur le plan qui livrera les sept tests du § 1.1.

---

## Definition of Done

La DoD de **ce plan** est la remontée, non l'implémentation :

1. Le plan est commité sur `feat/1727/tui-v2b2b2-callbacks-termines-par-le-sse`.
2. Il porte la mesure livrable par livrable (§ 1), avec sa part de doc isolée.
3. Il porte la calibration de l'estimateur sur le vol précédent (§ 0.1),
   mesurée et non rappelée.
4. Il porte la raison du non-rattrapage (§ 2) et la coupe que mika#2205
   interdit (§ 2.1).
5. Il porte les coupes candidates avec leur gain estimé, **sans en choisir
   aucune** (§ 3).
6. Il porte sa section `## Taille estimée` au format de mika#2636, avec le total
   réel — y compris quand ce total est ce qui déclenche la halte.

**Ce qu'aucune de ces lignes n'autorise :** écrire une ligne de
`crates/mika-cli/`. Le prochain geste appartient à Vincent.

---

## Acceptance criteria

Transcrits verbatim du corps de mika#1727 (§ « Critères de V2b-2b-2 »). Ils sont
les critères du **vol**, que l'implémentation devra tenir une fois la coupe
tranchée — ils ne sont pas la DoD de ce plan, qui est la halte ci-dessus.

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
le périmètre qui les tient mesure 370 lignes pour une borne de 279. Le § 1.1
établit la correspondance critère → test, afin que la coupe retenue puisse être
jugée sur ce qu'elle ferait perdre.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-cli/src/commands/tasks.rs` — miroir à dix champs (reprise de `23525549c`, mesuré) | 41 |
| `crates/mika-cli/src/commands/tasks.rs` — `terminal_callback_frame` | 17 |
| `crates/mika-cli/src/commands/tasks.rs` — `CallbackFeed` + `spawn_callback_subscription` | 36 |
| `crates/mika-cli/src/commands/tasks.rs` — `is_resume_callback_for` | 17 |
| `crates/mika-cli/src/commands/chat.rs` — site d'abonnement | 13 |
| tests (`crates/mika-cli/src/commands/tasks.rs`) — T1 abonnement, T2 prédicat | 57 |
| `crates/mika-cli/src/tui/app.rs` — champ `callback_feed` + deux inits | 10 |
| `crates/mika-cli/src/tui/app.rs` — `drain_callback_deliveries` | 51 |
| `crates/mika-cli/src/tui/app.rs` — site d'appel au tick | 8 |
| tests (`crates/mika-cli/src/tui/app.rs`) — helper + T3 à T6 | 120 |

Total estimé : 370 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Le total dépasse la borne de 91 lignes, soit 1,33×** — et ≈ 1,5× après
correction du biais d'estimateur mesuré au § 0.1. Conformément au corps du
ticket, le groom **s'arrête et le dit**, et ne recoupe pas lui-même : ce vol est
déjà le recoupage de mika#3069.

La suite se note `Tracked in: senara-solutions/mika#1727`.
