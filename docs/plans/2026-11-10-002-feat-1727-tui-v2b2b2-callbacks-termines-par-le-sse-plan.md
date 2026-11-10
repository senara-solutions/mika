# mika#1727 — vol V2b-2b-2 : halte, sur un plancher remesuré à 326

> **Verdict du groom : HALTE.** Le périmètre de ce vol mesure **326 lignes**
> hors `docs/`, pour une borne par vol de **279**. Le corps du ticket ordonne la
> conduite : « Si le total dépasse 279, le groom s'arrête et le dit. **Il ne
> recoupe pas lui-même** : ce vol est déjà le recoupage. »
>
> **Quatrième passe, et le chiffre a changé.** Les trois passes antérieures
> (15:03Z, 15:29Z, puis la réécriture de ce fichier) ont écrit **395**. Cette
> passe écrit **326**, et le § 1 nomme la cause de l'écart : les trois
> antérieures ont chiffré **l'emplacement que #3069 donne au code**, jamais le
> plancher du but. Déplacer la lecture HTTP et le prédicat derrière une couture
> de `tasks.rs` retire 69 lignes, renforce une contrainte du corps au lieu de
> l'affaiblir, et rend quatre des huit propriétés exigées testables sans base ni
> `App`.
>
> **Ce que l'écart change, et c'est tout l'objet de cette passe.** Il ne suffit
> pas à passer sous 279 — il reste 47 lignes de trop. Mais il refait la table
> des arbitrages du § 5 : **A4 seul suffit désormais**, avec une marge de 24 %
> au-dessus du biais d'estimation de 16 % mesuré au § 0.3, là où la passe 3 le
> jugeait un pari à 8 % et n'admettait que le couple A4 + A1. Le geste qui
> débloque passe donc de « deux gestes, ou un critère perdu » à **un seul geste
> d'opérateur, et aucun critère perdu**.

---

## 0 — Ce qui est mesuré, avant toute conception

### 0.1 — L'état de `main` (`766a62788`)

Tout ce que le corps décrit sous « Ce qui est sur main » est exact, vérifié site
par site cette passe :

| affirmation du corps | site vérifié |
|---|---|
| `TaskEventStream::open` / `consume` sont `pub(crate)` | `crates/mika-cli/src/commands/tasks.rs:523` et `:565` |
| `fetch_task_detail` et `TaskDetailMirror` sont privés au module | `tasks.rs:632` et `:611` |
| le miroir porte **six** champs | `tasks.rs:611-618` |
| le détail échoue explicitement sur un statut non-succès ou un corps illisible | `tasks.rs:643-650` |
| `fake_spirit` a deux constructeurs, chacun sur son serveur | `tasks.rs:2057` (`spawn`), `:2088` (`spawn_detail`) |
| `poll_callback_tasks` réclame par `mark_task_delivered` et livre | `tui/app.rs:1596` |
| il est appelé au tick, au repos, hors mode équipe, toutes les 167 | `tui/app.rs:1123`, `POLL_INTERVAL_TICKS` à `:663` |
| `app_with_response_sender` bâtit une `App` sur une base en mémoire | `tui/app.rs:2160` |
| `TaskEventFrame` a **huit** variantes | `crates/mika-agent/src/server/tasks_stream.rs:102-150` |

**Les six champs du miroir, nommément**, parce que ce point gouverne le § 3 :
`id`, `agent_id`, `label`, `status`, `created_by_session`, `result`. Ni
`trigger_type`, ni `action_type`, ni `created_trace_id`, ni `parent_task_id`. Le
critère 1 exige que le `CallbackResult` porte « `trace_id` et `parent_task_id`
**tirés du détail** », et la garde 2 exige `trigger_type` et `action_type` : **le
miroir à dix champs est obligatoire au but écrit**, pas un confort.

Trois points que le corps ne dit pas et qui pèsent sur la mesure :

- `app_with_response_sender` **jette** le receveur des `AgentRequest`
  (`let (agent_tx, _request_rx) = …`). Le critère 1 exige d'observer un
  `AgentRequest::CallbackResult` : le helper doit rendre ce receveur, ce qui
  touche sa signature et ses **trois** appelants.
- **`poll_callback_tasks` n'a aucun test.** Les deux seules occurrences du nom
  dans `tui/app.rs` sont sa définition (`:1596`) et son appel au tick (`:1123`).
  Aucun helper d'ensemencement d'une ligne de callback n'existe donc côté
  `tui/app.rs` : celui de ce vol part de zéro.
- **`NewTask` ne dérive que `Debug, Clone`** — vérifié à
  `crates/mika-agent/src/task_state/tasks.rs:1049-1050`, aucun `Default`. Lui en
  ajouter un serait modifier `crates/mika-agent/`, que le corps exclut. Le
  littéral de 23 champs est donc irréductible **depuis `mika-cli`**, et c'est ce
  qui fixe le poste le plus lourd des tests (§ 3, 36 lignes).

### 0.2 — Le matériau, remesuré au diff

Les deux commits de #3069 sont lisibles localement (`git cat-file -t` les
confirme), et leurs chiffres sont ceux du corps :

| mesure | commande | résultat |
|---|---|---|
| miroir `23525549c` | `git diff --numstat 766a62788 23525549c` | **`+41 −14`** sur `tasks.rs` |
| matériau `5321ba47d` | `git diff --numstat 23525549c 5321ba47d` | **`+298`** : 19 `chat.rs`, 168 `tasks.rs`, 111 `app.rs` |

Lu ligne par ligne, le matériau de `tasks.rs` se décompose ainsi — et **56 de ses
168 lignes sont hors du périmètre de ce vol**, ce que les passes antérieures
n'avaient pas isolé :

| bloc du matériau | lignes | sort dans ce vol |
|---|---|---|
| `terminal_callback_frame` | 25 | retenu, réduit (`TaskFailed` au bras `None`) |
| `spawn_callback_subscription` | 41 | retenu, refondu en `CallbackFeed` |
| `is_resume_callback_for` | 28 | **absorbé** par la couture du § 2 |
| ajustement du commentaire du miroir | 2 | retenu |
| helper `servir` de `fake_spirit` | 14 | **hors périmètre** (pas de constructeur neuf) |
| `spawn_stream_and_details` | 55 | **hors périmètre** (constructeur neuf, interdit) |

Le matériau porte donc un troisième constructeur de serveur factice que le corps
exclut nommément. Les tests de ce plan s'en passent : T1 se sert de `spawn`, T2
de `spawn_detail`, et le § 3 ne contient aucun poste de constructeur. **Aucun
critère de ce plan n'échoue sans constructeur neuf** — c'est la réponse à la
clause « un constructeur neuf n'entre que si un critère ne tient pas sans lui, et
le plan dit lequel ».

### 0.3 — La calibration de l'estimateur, mesurée sur le vol précédent

Le vol V2b-2b-1 a écrit `Total estimé : 241 lignes` et a livré, hors
`docs/plans/` :

```
$ git diff --numstat dbc6d2b01..766a62788
6    0   crates/mika-cli/src/cli.rs
272  1   crates/mika-cli/src/commands/tasks.rs
1    1   docs/egress/egress-manifest.toml
```

**279 ajoutées pour 241 estimées — une sous-estimation de 16 %**, et un
atterrissage exactement sur la borne. Même fichier, même style, même auteur de
plan : ce biais est la grandeur contre laquelle toute marge de ce plan doit être
jugée, et c'est à ce titre qu'il décide du § 5.

### 0.4 — Egress Manifest Lint, mesuré maintenant

```
$ bash scripts/verify-egress-manifest.sh --report
egress-manifest: 400 fichier(s) .rs de production scanné(s), 31 portant une
construction de client, 22 host(s) distinct(s) rencontré(s), 31 entrée(s)
[[sink]] confrontée(s).
No egress-manifest violations found.
```

**Vert aujourd'hui, et le critère 4 est tenable sans toucher `docs/egress/`** —
sous les deux formes, celle du matériau comme celle du § 2. La raison est
précise, et elle n'est pas celle qu'on devine : le lint confronte les fichiers
qui **construisent un client** et les hosts qu'ils visent, pas les sites
d'appel. `tui/app.rs` en construit déjà un (`:1004`, les boutons du tableau de
bord, déclaré par `cli-spirit-tui`), et le compte est déjà équilibré à 31/31.
Appeler `fetch_task_detail` depuis `tui/app.rs` ne construirait aucun client de
plus : le lint resterait vert.

Ce que la couture du § 2 améliore n'est donc **pas la couleur du lint**, c'est la
véracité du manifeste : `cli-spirit-tasks` déclare
`call_site = "crates/mika-cli/src/commands/tasks.rs"` pour « les deux gestes »,
l'ouverture du flux et la lecture du détail (`docs/egress/egress-manifest.toml:352-359`).
Sous la forme du matériau, le second geste s'appelle depuis `tui/app.rs` et cette
déclaration devient fausse en silence — un manifeste qui ne mord pas. Sous la
couture, elle reste exacte sans être modifiée. C'est un argument de justesse
documentaire, et il est énoncé comme tel.

---

## 1 — L'hypothèse que les trois passes antérieures n'ont pas testée

Les trois passes ont établi un plancher de **210 lignes hors tests** et conclu
qu'aucun découpage ne ramenait le but sous 279. Leur mesure est fidèle — chaque
poste est vérifiable au diff du matériau. Mais elle chiffre **une implémentation
particulière** : celle de #3069, qui place dans `tui/app.rs` la lecture HTTP du
détail et l'application du prédicat, et qui fait donc voyager vers la TUI l'URL
de base, le jeton, et la connaissance du miroir.

Ce qu'aucune des trois n'a demandé : *le plancher change-t-il si la couture
change ?* Il change, et de 69 lignes. Le plancher de 210 n'est pas une propriété
du but écrit ; c'est une propriété de l'emplacement du code.

Trois conséquences se tiennent, et la seconde est la plus importante :

1. **Les trois champs posés sur `App` deviennent un seul**, et ni l'URL ni le
   jeton n'entrent dans `tui/app.rs`.
2. **La contrainte du corps est renforcée, pas contournée.** « Aucun client HTTP
   dans `tui/app.rs`. La TUI appelle ce que `tasks.rs` expose » : le matériau la
   satisfait à la lettre — il appelle bien une fonction de `tasks.rs` — mais
   `tui/app.rs` y détient l'URL, le jeton et le nom de la lecture. Sous la
   couture, la TUI n'a aucun des trois : la contrainte devient une propriété du
   **type**, et non une consigne qu'une relecture doit vérifier.
3. **Quatre des huit propriétés exigées se testent sans base ni `App`** —
   l'économie de test la plus lourde, détaillée au § 3.1.

---

## 2 — La couture, et ce qu'elle déplace

Un type dans `tasks.rs` porte les trois éléments indissociables du fil, et expose
**un seul geste** à la TUI : « donne-moi le prochain candidat recevable ».

```rust
pub(crate) struct CallbackFeed {
    rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    base_url: String,
    token: String,
}
```

- **`CallbackFeed::open_for(agent_id, base_url, token)`** ouvre le flux une
  fois, sans recul ni reconnexion, et pousse sur le canal les `task_id` que
  `terminal_callback_frame` reconnaît. Le canal ne porte **que** le `task_id` :
  `TaskFailed` étant hors de ce vol, l'`original_status` vaut toujours
  `"completed"` et n'a pas à voyager — le couple du matériau tombe.
- **`CallbackFeed::next_candidate(agent_id, session_id) -> Option<TaskDetailMirror>`**
  enchaîne les deux premières gardes : `try_recv`, puis la lecture HTTP du
  détail, puis le prédicat du poll. `None` couvre les quatre refus — canal vide,
  détail en erreur, détail illisible, prédicat faux — et **aucun ne réclame**,
  ce qui est exactement « je passe et je ne réclame pas », la ligne restant
  `completed` pour le poll.

`is_resume_callback_for` disparaît comme méthode publique : ses quatre
conjonctions vivent dans `next_candidate`, qui est son seul appelant, et les
quatre champs qu'elles lisent restent **privés** au module — le miroir conserve
son énoncé « chaque champ a son lecteur, et pas un de plus » sans élargir aucune
visibilité. C'est le poste de 28 lignes du matériau qui s'absorbe.

`tui/app.rs` garde alors les quatre derniers gestes, et eux seuls : réclamer par
`mark_task_delivered`, sauter un `completed` sans résultat, pousser le message
système, envoyer le `CallbackResult` puis passer en `Thinking`. Le `match` sur
`TaskEventFrame` reste exhaustif, sans bras `_`, dans `terminal_callback_frame`.

**Ce que la couture ne fait pas.** Elle ne touche pas `poll_callback_tasks`
(critère 4), n'ajoute aucun constructeur de serveur factice, ne modifie ni
`crates/mika-agent/` ni `crates/mika-a2a/`, et laisse `docs/egress/` intact.
Elle ne supprime pas non plus la duplication des gestes 4 à 6 avec le poll :
cette duplication est assumée pour un vol, V2b-3 retirant le poll et elle avec.

---

## 3 — La mesure, livrable par livrable

L'ordre suit celui des commits que le corps impose — « le pilote commite après
chaque garde verte avec son test ». La colonne « source » dit ce qui est mesuré
et ce qui est estimé, pour que chaque ligne soit contestable séparément.

### Commit A — le miroir grandit avec son lecteur

| livrable | source | lignes |
|---|---|---|
| reprise de `23525549c` telle quelle, tests de format compris | **mesuré** | 41 |

Une version resserrée coûterait ≈ 25 — l'essentiel des 41 est une réécriture de
doc et la mise à jour de l'attente « ni un champ de plus ni un de moins », qui
passe de six à dix paires. Le corps blesse pourtant la reprise telle quelle
(« MPC l'a relu et le juge sain : il peut être repris tel quel »), et un
`cherry-pick` coûte moins de tours qu'une réécriture. **C'est 41 qui est retenu**,
et la différence de 16 lignes est nommée ici pour que l'arbitrage reste ouvert.

### Commit B — l'abonnement (`tasks.rs`, `chat.rs`)

| livrable | source | lignes |
|---|---|---|
| `terminal_callback_frame` — `match` exhaustif sur huit variantes, sans bras `_` | **mesuré 11** + 6 doc | 17 |
| `CallbackFeed` + `open_for` + `next_candidate` | matériau 66 refondu, écrit ligne à ligne | 66 |
| site d'abonnement dans `chat::run` | matériau 19, comprimé | 12 |
| tests — T1 abonnement filtrant, T2 les six refus | estimé | 56 |

**Sous-total B : 151.**

Les 66 lignes de `CallbackFeed` ont été écrites ligne à ligne pour cette mesure,
et non extrapolées : 50 de code (5 de struct, 21 d'`open_for`, 16 de
`next_candidate`, 8 d'`impl` et de signatures) et 16 de doc, soit 24 % — contre
39 % dans `crates/` de #3069 (131 sur 338, chiffre du corps). Le commentaire y
dit le pourquoi qu'on ne lit pas dans le code : pourquoi l'abonnement part de
`chat::run` et non du constructeur, pourquoi le canal ne porte que le `task_id`,
pourquoi aucun des quatre refus ne réclame.

### Commit C — le drainage (`tui/app.rs`)

| livrable | source | lignes |
|---|---|---|
| champ `callback_feed` + les deux initialisations de constructeur | matériau 23, comprimé | 9 |
| `drain_callback_deliveries` (les quatre derniers gestes) | matériau 56, écrit ligne à ligne | 38 |
| site d'appel au tick, au repos, hors mode équipe | **mesuré** | 7 |
| helper d'ensemencement d'une ligne de callback en base | estimé, **plancher** | 36 |
| `app_with_response_sender` rend le receveur + ses trois appelants | estimé | 6 |
| tests — T3 livré une fois et pas deux, T4 déjà réclamé | estimé | 38 |

**Sous-total C : 134.**

Le drainage tombe de 56 à 38 parce que les deux premières gardes sont parties
dans `next_candidate` : il ne reste que le `let Some(detail) = … else { return }`,
le `match` sur `mark_task_delivered`, le saut d'un résultat vide, et la
livraison.

Le helper d'ensemencement coûte 36 lignes et **c'est un plancher mesuré, non une
approximation** : `create_task` prend un `NewTask` de 23 champs sans `Default`
(§ 0.1), ce qui fait 23 lignes pour le seul littéral, plus la signature, l'ouverture
de la base en mémoire, le passage du statut à `completed` et la relecture de
l'identifiant. Le seul précédent du dépôt sur exactement cette forme —
`seed_callback_row`, `tasks.rs:1509-1562` — en fait la démonstration en 53 lignes,
et il est de surcroît `#[cfg(target_os = "linux")]` et paramétré pour un pgid :
il n'est pas réutilisable ici.

### 3.1 — Le jeu de tests est déjà minimal, et c'est la couture qui l'a réduit

Le corps est sans ambiguïté : « Chaque garde arrive avec ses tests, dans le même
commit. Les tests ne se reportent jamais à un vol suivant. » Les cinq critères
exigent huit propriétés distinctes. Voici le groupement le plus serré qui les
couvre, et la colonne « coût » dit ce que la couture a changé :

| propriété exigée | critère | test | où | coût |
|---|---|---|---|---|
| une trame d'un autre agent ne passe pas | 2 | T1 | `tasks.rs` | sans base, sans `App` |
| `TaskFailed`, `Unknown`, `OverflowMarker` ignorées sans casser l'abonnement | 3 | T1 | `tasks.rs` | sans base, sans `App` |
| `callback` / `resume_agent` exigés | 2 | T2 | `tasks.rs` | sans base, sans `App` |
| une autre session n'est ni livrée ni réclamée | 2 | T2 | `tasks.rs` | sans base, sans `App` |
| détail en erreur HTTP : ni livré ni réclamé | 3 | T2 | `tasks.rs` | sans base, sans `App` |
| détail illisible : ni livré ni réclamé | 3 | T2 | `tasks.rs` | sans base, sans `App` |
| un `completed` n'est livré qu'**une** fois | 1 | T3 | `app.rs` | base + `App` |
| rien si le poll a déjà réclamé | 1 | T4 | `app.rs` | base + `App` |

**Six propriétés sur huit quittent `app.rs`.** C'est l'effet principal de la
couture : sous la forme du matériau, « autre session », « pas un callback » et
« détail illisible » exigeaient chacun une `App` et une ligne en base, parce que
le prédicat et la lecture HTTP y vivaient. Sous `next_candidate`, un test les
mesure avec un canal qu'il alimente lui-même et un `spawn_detail` jetable — et
l'unique helper de 36 lignes ne sert plus que **deux** tests au lieu de six.

T1 porte quatre propriétés en une seule séquence SSE de six trames ; T2 porte ses
six refus et son témoin positif en un test paramétré par un corps de détail.
La compression est **déjà appliquée** dans le chiffre de 94 (56 + 38) : elle ne
peut aller plus loin sans perdre une propriété, et le corps l'interdit.

---

## 4 — Le fait dur : le plancher, et ce qui reste pour les tests

Le code structurellement nécessaire au but écrit — un `completed` arrive par le
SSE, se lit par HTTP, se réclame, se livre une fois — hors tout test :

```
 41  miroir à dix champs        (critère 1 : trace_id + parent_task_id du détail)
 17  terminal_callback_frame    (le fil dit quelle tâche, sans bras `_`)
 66  CallbackFeed               (ouverture, canal, lecture HTTP, prédicat)
 12  site d'abonnement          (chat::run, pas run_team)
  9  champ + deux inits
 38  drain_callback_deliveries  (réclamer, sauter un vide, livrer)
  7  site d'appel au tick
---
190  plancher, hors tout test
```

La borne est à 279 : **il reste 89 lignes pour les tests**, dont 36 prises par le
seul helper d'ensemencement et 6 par le receveur à rendre. Soit **47 lignes pour
six tests et huit propriétés**, quand la mesure du § 3 en demande 94.

Les deux leviers restants sont déjà tirés :

- **Les commentaires.** À 24 % du code neuf, contre 39 % au matériau. Les ramener
  à zéro — un plan refusé en revue, et à juste titre — laisserait ≈ **287**.
  Toujours au-dessus, et la marge serait alors négative sous le biais de 16 %.
- **Le rangement des tests.** C'est précisément ce que la couture a déjà fait, et
  à grande échelle : six propriétés sur huit ont quitté `app.rs` (§ 3.1). La
  porte « il suffisait de mieux ranger les tests » n'est pas fermée par un avis,
  elle est **franchie**, et le résultat est les 326 de ce plan au lieu des 395 des
  passes antérieures.

### 4.1 — La coupe que mika#2205 interdit

La coupe qui vient d'abord à l'esprit — l'abonnement dans ce vol, le drainage
dans le suivant — est fermée nommément par le corps : « Un abonnement dont
personne ne draine le canal, ou un drainage sans livraison, ne passe pas non plus
(mika#2205). » Elle n'est pas candidate, et ce plan ne la propose pas.

Son corollaire : puisque le plancher bout-en-bout vaut 190 et que mika#2205 exige
que chaque PR soit utile de bout en bout, **aucun découpage du but ne ramène ce
vol sous 279**. Le seul découpage licite est celui du miroir (A1 ci-dessous),
mergeable seul parce que ses quatre champs neufs ont un lecteur immédiat — les
formats `json` et `yaml` de `mika tasks get <id> --spirit`, déjà sur `main` — et
il laisse 285, encore au-dessus. Le choix qui reste n'est donc pas un choix de
taille.

---

## 5 — Les arbitrages, rechiffrés — et c'est ici que cette passe change la décision

Ils sont la matière de la décision de Vincent. Aucun n'est retenu ici. Les
totaux de la passe 3 sont rappelés pour que l'écart soit lisible.

| voie | total passe 3 | **total cette passe** | borne | marge | ce qu'elle coûte |
|---|---|---|---|---|---|
| **A4** — plafond relevé à 200 tours | 395 | **326** | 429 | **+24 %** | un geste d'opérateur. **Aucun critère.** |
| **A4 + A1** | 354 | **285** (+41 en vol séparé) | 429 | **+34 %** | deux gestes. Aucun critère. |
| **A1** — le miroir part seul | 354 | **285** | 279 | −2 % | ne passe pas |
| **A3** — le détail HTTP sort du chemin | ≈ 230 | ≈ **226** | 279 | +19 % | le critère 3, et une part du critère 1 |

**A4 est la voie que cette passe ouvre.** La borne de 279 dérive d'un plafond de
150 tours (`(150 − 27 − 30) × 9 / 3`, site de vérité
`_plan_size_per_flight_bound` dans `skills/bundled/_shared/dispatch-lib.sh`). Un
plafond de 200 tours donne `(200 − 27 − 30) × 9 / 3 = 429`. À 326, la marge est
de 24 % — **au-dessus du biais de 16 % mesuré au § 0.3**, ce qui distingue cet
arbitrage d'un pari. À 395, la passe 3 n'avait que 8 % et concluait, justement,
qu'A4 seul atterrirait là où le vol précédent a atterri : sur la borne exacte.

Deux chemins mènent à ce plafond, et ils ne se valent pas :

- le label **`loop-substrate`**, qui porte le plafond à 200 via
  `PILOT_LABEL_TURN_CEILINGS`. C'est le chemin nommé par la passe 3. Il a un
  coût que ce plan signale plutôt que de le taire : le label est **sémantique**
  autant que machine, et mika#1727 est un travail de client mince de la TUI, pas
  du substrat de boucle. L'apposer pour son seul effet de bord mélange les deux
  lectures du label ;
- une **surcharge `PILOT_MAX_TURNS`** sur ce dispatch, qui produit le même
  plafond sans rien affirmer sur la nature du ticket. C'est le chemin que la
  mesure recommande, et il est hors de la main du groom.

**A1 ne suffit plus comme complément obligé, mais il reste bon.** Le commit
`23525549c` est mergeable seul, relu, et sort 41 lignes du chemin critique sans
risque. Il porte la marge de 24 % à 34 %. Il est utile, il n'est plus nécessaire.

**A3 reste sous la borne, et reste le seul des quatre à coûter un critère.** Le
but écrit est « se lit par HTTP, se réclame et se livre » ; A3 garde les deux
derniers et abandonne le premier, la ligne locale portant déjà `result`,
`created_trace_id` et `parent_task_id`. Le fil n'apporte alors plus que la
latence. C'est un arbitrage d'architecture, pas de taille — et c'est pourquoi le
groom le remonte au lieu de le prendre. Son chiffre de ≈ 226 est aussi le
moins étayé des quatre : il n'a pas été écrit ligne à ligne comme les 326, et il
conserve le helper de 36 lignes, que le prédicat lu en base rend plus nécessaire
encore.

---

## 6 — Ce que ce groom n'a pas fait

- **Il n'a pas recoupé le périmètre.** Le corps le lui retire explicitement, et
  le § 4.1 montre qu'aucun découpage du but n'existe sous mika#2205.
- **Il n'a pas déclaré un plafond que le ticket ne porte pas.** La section
  `## Taille estimée` écrit `150 (défaut)`, parce que c'est l'état d'aujourd'hui.
  Écrire 200 pour faire tenir le plan serait fabriquer un état de label.
- **Il n'a ouvert aucun ticket**, pas même de suivi : gel des tickets jusqu'au
  01/12. La suite se note `Tracked in: senara-solutions/mika#1727`.
- **Il n'a touché aucun fichier hors `docs/plans/`.** Rien sous
  `.github/workflows/`, `.claude/`, `skills/bundled/`,
  `crates/mika-agent/src/well_known_agents.rs`, `.mika/` ni `docs/egress/`.
  `crates/mika-agent/` et `crates/mika-a2a/` sont intacts. Aucune ligne de
  `crates/mika-cli/` n'a été écrite.
- **Il n'a pas ouvert un second plan concurrent.** Ce fichier est la mise à jour
  des trois passes antérieures, dont il corrige le chiffre et refait la table
  des arbitrages.

---

## 7 — L'issue machine de cette halte

**ESCALATE est l'issue attendue, et c'est la bonne.** mika#2693 est explicite :
« Un total au-dessus de la borne par vol sans découpage en vols […] fait rendre
ITERATE à mika-arch en première passe et ESCALATE en seconde. » Ce plan porte
`Total estimé : 326` pour une borne de 279 et ne porte aucun découpage en vols,
parce que le corps du ticket le lui interdit nommément et que le § 4.1 démontre
qu'aucun n'existe.

Les deux règles ne se contredisent pas, elles se composent : le corps dit au
groom de s'arrêter et de le dire, mika#2693 dit comment une halte voyage.
**ESCALATE *est* le canal par lequel « le groom s'arrête et le dit » atteint
l'opérateur.**

**Ce qu'un cinquième dispatch produirait.** Les trois premières passes ont écrit
395 ; celle-ci écrit 326, en remettant en cause non leur arithmétique mais leur
hypothèse d'emplacement. Cette hypothèse-là est maintenant épuisée : les six
propriétés que la couture pouvait déplacer hors de `app.rs` l'ont été, le
littéral `NewTask` est irréductible depuis `mika-cli` par une propriété vérifiée
du type, et le plancher restant est à 24 % au-dessus de la borne. Un cinquième
groom chercherait une seconde couture dans un espace que celle-ci a vidé.

**Le geste qui débloque, et son auteur.** Vincent relève le plafond de ce
dispatch — par `PILOT_MAX_TURNS` de préférence au label, pour la raison du § 5 —
et le vol part tel qu'il est décrit ici, aucun critère perdu. À défaut, il tranche
A3 et le corps du ticket est amendé pour en retirer le critère 3 et la part du
critère 1 qui exige la lecture HTTP. Les deux voies sont mesurées ; ce plan n'en
choisit aucune.

---

## Fire-Disposition

Ce plan spécifie six détecteurs — les tests T1 à T4 du § 3.1, dont le chemin de
succès est « aucune violation trouvée ». Disposition retenue :
**(c) halte-et-remontée.**

Aucun des six n'atterrit, armé ou désarmé : ce plan n'autorise pas l'écriture
d'une ligne de `crates/mika-cli/`, et la remontée à l'opérateur est son objet
même (§ 7). Il n'y a donc ni exception à nommer en allowlist, ni `#[ignore]` à
poser, ni suivi d'armement à ouvrir — le gel des tickets jusqu'au 01/12
l'interdirait de toute façon, et la suite se note
`Tracked in: senara-solutions/mika#1727`.

Pour mémoire, à l'intention du plan qui livrera effectivement ces six tests une
fois l'arbitrage tranché : leur disposition sera **(a) exception nommée en
allowlist, avec une allowlist vide**. Ils mesurent une surface **neuve** — un
abonnement et un drainage qui n'existent pas sur `main` — donc aucune violation
préexistante n'est à amnistier, et aucune entrée grep-visible n'est due. Un test
qui rougirait à l'atterrissage y signalerait un défaut de l'implémentation, pas
une dette héritée.

---

## Definition of Done

La DoD de **ce plan** est la remontée, non l'implémentation :

1. Le plan est commité sur `feat/1727/tui-v2b2b2-callbacks-termines-par-le-sse`.
2. Il porte la mesure livrable par livrable (§ 3), chaque ligne étiquetée
   « mesuré » ou « estimé » pour être contestable séparément.
3. Il nomme l'hypothèse non testée des trois passes antérieures et l'écart
   qu'elle produit — 395 contre 326 (§ 1), avec la décomposition du matériau qui
   l'établit (§ 0.2), dont les 56 lignes hors périmètre que personne n'avait
   isolées.
4. Il porte le **plancher** du but écrit, hors tests, et le budget de tests qui
   en reste (§ 4) — c'est le fait dur de la halte.
5. Il porte les deux propriétés vérifiées qui rendent deux postes irréductibles :
   les six champs du miroir sur `main` et l'absence de `Default` sur `NewTask`
   (§ 0.1).
6. Il porte la calibration de l'estimateur sur le vol précédent (§ 0.3), mesurée
   et non rappelée, et juge chaque marge du § 5 contre elle.
7. Il porte l'état mesuré d'Egress Manifest Lint (§ 0.4) et dit, sans le
   surestimer, ce que la couture y change — la véracité du manifeste, pas la
   couleur du lint.
8. Il porte la coupe que mika#2205 interdit (§ 4.1) et son corollaire.
9. Il porte les quatre arbitrages avec leur gain rechiffré, **sans en choisir
   aucun** (§ 5), en nommant celui qui suffit désormais seul (A4), le coût
   sémantique du chemin par label, et celui qui coûte un critère (A3).
10. Il porte l'issue machine attendue (§ 7), sa section `## Fire-Disposition` et
    sa section `## Taille estimée` au format de mika#2636, avec le total réel —
    y compris quand ce total est ce qui déclenche la halte.

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
le plancher qui les porte mesure 190 lignes hors tests, pour une borne de 279 dont
42 sont prises par le helper d'ensemencement et le receveur à rendre. Le § 3.1
établit la correspondance critère → test, afin que l'arbitrage retenu puisse être
jugé sur ce qu'il ferait perdre : A4 ne fait rien perdre, A3 coûte le critère 3 et
une part du critère 1. Le critère 4 est tenable sous les deux (§ 0.4, lint mesuré
vert aujourd'hui).

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-cli/src/commands/tasks.rs` — miroir à dix champs (reprise de `23525549c`, mesuré) | 41 |
| `crates/mika-cli/src/commands/tasks.rs` — `terminal_callback_frame` | 17 |
| `crates/mika-cli/src/commands/tasks.rs` — `CallbackFeed` + `open_for` + `next_candidate` | 66 |
| `crates/mika-cli/src/commands/chat.rs` — site d'abonnement | 12 |
| tests (`crates/mika-cli/src/commands/tasks.rs`) — T1 abonnement filtrant, six trames | 22 |
| tests (`crates/mika-cli/src/commands/tasks.rs`) — T2 `candidat` + `corps_candidat` + six refus | 34 |
| `crates/mika-cli/src/tui/app.rs` — champ `callback_feed` + deux inits | 9 |
| `crates/mika-cli/src/tui/app.rs` — `drain_callback_deliveries` | 38 |
| `crates/mika-cli/src/tui/app.rs` — site d'appel au tick | 7 |
| tests (`crates/mika-cli/src/tui/app.rs`) — ensemencement d'une ligne de callback (`NewTask`, 23 champs) | 36 |
| tests (`crates/mika-cli/src/tui/app.rs`) — `app_with_response_sender` rend le receveur + trois appelants | 6 |
| tests (`crates/mika-cli/src/tui/app.rs`) — T3 livré une fois et pas deux, T4 déjà réclamé | 38 |

Total estimé : 326 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Le total dépasse la borne de 47 lignes, soit 1,17×** — contre 1,42× aux trois
passes antérieures. L'écart ne vient pas d'un périmètre revu à la baisse : le but
est le même, les cinq critères sont les mêmes, et c'est l'emplacement du code qui
a changé (§ 1 et § 2). Le dépassement restant n'est pas un excès de périmètre : le
**plancher** du but, hors tout test, vaut 190, ce qui laisse 47 lignes pour six
tests et huit propriétés quand la mesure en demande 94 (§ 4). Conformément au
corps du ticket, le groom **s'arrête et le dit**, et ne recoupe pas lui-même : ce
vol est déjà le recoupage de mika#3069.

Le § 7 dit l'issue machine attendue — ESCALATE, qui est le canal par lequel cette
halte atteint l'opérateur. Ce que cette passe apporte à la décision tient en une
ligne : **A4 seul suffit désormais** (326 pour une borne de 429, soit 24 % de
marge contre un biais mesuré de 16 %), et il ne coûte aucun critère. Le gel des
tickets jusqu'au 01/12 interdisant d'ouvrir un second ticket :

Tracked in: senara-solutions/mika#1727
