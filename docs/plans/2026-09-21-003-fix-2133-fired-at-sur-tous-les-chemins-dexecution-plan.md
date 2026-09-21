# mika#2133 — `fired_at` estampillé sur tous les chemins d'exécution

- **Ticket** : senara-solutions/mika#2133
- **Type** : fix (observabilité)
- **Priorité** : p2-normal
- **Base mesurée** : HEAD `c8520787`

---

## Problème

### La mesure du ticket est datée, et la rectifier est le premier livrable

Le ticket a été ouvert le 2026-09-01 sur cette mesure :

| `trigger_type` | total | avec `fired_at` |
|---|---|---|
| `recurring` | 64 | 56 |
| `time` | 6 | 2 |
| `manual` | 1236 | 17 |
| `callback` | 1946 | **0** |
| `a2a` | 542 | **0** |

Depuis, **deux tickets voisins ont fermé la majeure partie du défaut**, sans que
mika#2133 soit refermé ni mis à jour. Lecture du code à `c8520787` :

- **mika#2263 défaut (b)** — `Database::set_task_process_id`
  (`db/tasks.rs:2931`) stampe `fired_at` quand un `process_id` est enregistré et
  que la ligne n'en porte pas. Son unique appelant de production est
  `skills/executor.rs:3582`, juste après le spawn, et il écrit sur la **ligne
  callback enfant**. Donc **tout callback portant un pilote est estampillé
  depuis ce correctif** — c'est-à-dire l'essentiel de la population `callback`.
- **mika#2335 F2a** — `Database::mark_parent_dispatched` (`db/tasks.rs:838`)
  est l'écrivain unique de la transition de dispatch d'un parent, stamp compris.
  Les **trois** chemins de dispatch de production l'appellent
  (`skills::executor::execute_long_running` — l'original #525 —,
  `server::ready_label_handler`, `server::verdict_handler`). Donc la population
  `manual` des lignes de tracking est estampillée.
- **AC4 est déjà tenu pour ces chemins**, et il l'est par une garde :
  `db::tests::mika2335_no_production_dispatch_transitions_a_parent_without_stamping`,
  scan de source **à allowlist vide**, dont la disposition d'un quatrième site
  est écrite comme halt-and-surface.
- Un test eval existe : `tests/eval/test_dispatch_fired_at_stamped.rs`.

**Ce qu'il reste à faire n'est donc pas ce que le ticket décrit.** Le travail
n'est ni d'écrire un writer, ni de « trouver où les chemins callback et a2a font
démarrer une tâche » en général : c'est de fermer les **deux poches résiduelles**
que les deux correctifs précédents n'ont pas traversées, et de trancher par écrit
le second symptôme du commentaire 1.

### Poche 1 — `a2a` : aucun site n'écrit `fired_at`, littéralement

```
$ grep -c fired_at crates/mika-agent/src/a2a_db.rs
0
```

Une tâche `a2a` est créée `pending` (`a2a_db.rs:215`), puis passée à
`in_progress` par `a2a_update_task_state(&task_id, "working")`
(`a2a_db.rs:255`), dont l'`UPDATE` écrit `status`, `updated_at` et
`completed_at` — et rien d'autre.

Deux appelants de production, tous deux au démarrage réel du tour :

- `server/a2a.rs:1154` — branche synchrone de `message/send` ;
- `server/a2a.rs:1627` — `run_a2a_stream_turn`, la branche `message/stream`.

La branche `returnImmediately` ne les traverse pas : elle crée la ligne et la
rend en `submitted` sans jamais exécuter de tour. **C'est le contrôle négatif
naturel d'AC2/AC3 en production** — cette population doit rester NULL.

Le `status` de ce chemin est correct (`in_progress` est bien posé) ; seule
l'estampille manque. C'est le cœur résiduel d'AC1.

### Poche 2 — `callback` sans PID

Une ligne `callback` a **deux phases**, et le ticket ne parle que de la
première :

1. **Travail** — la ligne est créée `pending`, un pilote est spawné et son PID y
   est enregistré (`set_task_process_id`, estampille depuis mika#2263), le
   pilote travaille, un écrivain externe pose le résultat et la passe
   `completed`.
2. **Livraison** — `get_undelivered_callback_tasks` (`db/tasks.rs:1625`)
   sélectionne les lignes `completed`/`failed` non livrées,
   `dispatch_resume_agent` (`dispatcher.rs:828`) lance `run_silent_agent`, puis
   `mark_task_delivered` la passe `delivered`.

Une ligne callback **sans** pilote — le wrapper différé, un rendez-vous de build
— ne traverse jamais `set_task_process_id`. Le seul instant où le moteur
travaille sous elle est son **tour de livraison**, qui est du travail réel : un
tour LLM prenant le verrou d'agent pour jusqu'à `AGENT_TOTAL_TIMEOUT_SECS`.
Aujourd'hui, rien ne le marque.

**La taille de cette population n'a pas pu être mesurée** : la base de
production n'est pas lisible depuis le bac à sable de dispatch
(`no matching policy rule -- denied by default`). C'est la sonde AC5
post-déploiement qui la révélera, et le plan est écrit pour que sa lecture soit
possible (§ Verification Contract, sonde S2).

### Le second symptôme du commentaire 1, et pourquoi il n'est pas de cette famille

Le commentaire 1 mesure un second effet de la même transition absente : une
ligne callback portant un pilote vivant reste `status = 'pending'`, donc
« un garde-tableau qui compte `status='in_progress'` affiche zéro pendant qu'un
pilote travaille ».

Le fait est exact et il est toujours vrai à `c8520787`. Mais **le remède n'est
pas du même ordre que celui de `fired_at`**, et la décision D6 ci-dessous
l'établit sur une mesure plutôt que sur une préférence : poser `in_progress` sur
ces lignes rouvre un périmètre qu'un ticket voisin a explicitement borné par
écrit. Le commentaire 1 offre lui-même les deux branches acceptables — « un état
intermédiaire … **ou une raison écrite quelque part expliquant pourquoi ce
chemin n'en a pas besoin** — mais pas le silence actuel ». Ce plan livre la
seconde, de façon durable, et ouvre le suivi pour la première.

---

## Requirements

- **R1** — Une tâche `a2a` dont un tour démarre porte `fired_at`. Les deux
  chemins de production (`message/send` synchrone, `message/stream`) sont
  couverts par un seul site de code.
- **R2** — Une tâche `callback` dont le tour de livraison démarre porte
  `fired_at`, y compris lorsqu'aucun pilote ne lui a jamais été attaché.
- **R3** — Une tâche jamais tirée garde `fired_at` NULL. Aucune estampille n'est
  posée à la création, ni sur la branche `returnImmediately`, ni sur une
  transition terminale.
- **R4** — Une estampille existante n'est jamais réécrite sur les chemins
  one-shot. La sémantique d'écrasement de `claim_and_fire_task` pour les
  `recurring` est préservée telle quelle (D4).
- **R5** — Le fragment SQL d'estampillage existe à **un seul endroit**, et un
  cinquième écrivain ne peut pas apparaître en silence.
- **R6** — Aucune ligne historique n'est rétro-estampillée : aucune migration de
  données, aucun DDL.
- **R7** — La raison pour laquelle le chemin callback ne porte pas d'état
  intermédiaire est écrite là où l'opérateur la lit, et un suivi la porte.

---

## Décisions

### D1 — Le stamp a2a vit au point de passage unique, pas chez les deux appelants

`a2a_update_task_state` est traversé par les deux chemins qui démarrent un tour
a2a. Le stamp y est posé, pas dans `server/a2a.rs`.

La doctrine est déjà écrite dans ce dépôt, sur le site jumeau
(`set_task_process_id`) : *« Le stamp vit ici, au point de passage unique que
tout chemin de spawn traverse, plutôt que dans chaque appelant — un site ne peut
pas dériver d'un autre comme le fait une convention par appelant. »*

Et le mode de panne qu'elle prévient est **mesuré dans ce dépôt, sur ce défaut
précis** : les trois chemins de dispatch parent sont nés par recopie l'un de
l'autre — leurs commentaires le disent en toutes lettres (« mirrors
`execute_long_running`'s #525 transition ») — et c'est exactement ce qui a
produit le défaut que mika#2335 a dû fermer. Deux sites a2a aujourd'hui, un
troisième arrivera de la même façon.

### D2 — Le prédicat porte sur le statut interne, jamais sur la chaîne de protocole

Le stamp est conditionné à `internal_status == "in_progress"`, pas à
`a2a_state == "working"`.

Deux raisons. `a2a_state_to_task_status` (`a2a_db.rs:64`) porte un bras
`_ => "pending"` qui absorbe toute valeur inconnue : un prédicat écrit sur la
chaîne A2A diverge de la table à la première valeur de protocole ajoutée en
amont. Et le statut interne **est** la définition de « démarrée » partout
ailleurs dans cette table — `claim_and_fire_task`, `mark_parent_dispatched` et
les trois compteurs de concurrence la lisent tous ainsi. Un second vocabulaire
créerait deux définitions de « déclenchée », précisément ce qu'AC4 interdit.

### D3 — NULL-only, jamais un écrasement, sur tous les sites livrés ici

Clause `fired_at = CASE WHEN fired_at IS NULL THEN strftime(…) ELSE fired_at END`.

C'est la forme déjà en vigueur aux deux sites existants, et la raison y est déjà
écrite : *« les reapers mesurent l'âge d'un dispatch depuis ce champ, et un
re-stamp remettrait cet âge à zéro sous eux »*. Les reapers concernés sont
nommés et vivants — `MIKA_PILOT_STALL_REAP_AGE_SECONDS` (mika#2249/#2277),
le balayage phantom (mika#1712/#2156), le watchdog #959 —, et `db/tasks.rs:1266`
est une requête qui trie littéralement sur `fired_at ASC` avec un
`fired_at < ?2`.

Le besoin n'est pas théorique sur les deux chemins livrés : rien n'interdit un
second `a2a_update_task_state(…, "working")` sur la même ligne, et le tour de
livraison d'un callback est **explicitement ré-essayé** par le mécanisme de
backoff de mika#2179 (`MIKA_CALLBACK_DELIVERY_MAX_ATTEMPTS`). Sans la clause, un
callback en quarantaine verrait son `fired_at` avancer d'une heure à chaque
tentative — une estampille qui a l'air d'une mesure et qui suit l'horloge du
réessai.

### D4 — `claim_and_fire_task` n'est PAS migré vers NULL-only, et c'est une décision

Il écrit `fired_at = strftime(…)` sans condition : chaque tir écrase. Pour une
tâche `recurring`, c'est **le sens voulu** — l'estampille y dit « dernier tir »,
pas « premier tir ». C'est la seule population qui fonctionnait avant ce ticket
(56/64), et la lire comme des premiers tirs la rendrait fausse.

Les deux sémantiques coexistent par nature du déclencheur : pour une tâche
one-shot, premier et dernier tir sont le même instant, donc la distinction est
sans objet ; pour une récurrente, seul le dernier a un sens. Uniformiser vers
NULL-only figerait chaque récurrente sur son tir inaugural et casserait la seule
lecture qui marchait — un correctif d'observabilité qui détruit la mesure
existante.

Cette asymétrie est donc **déclarée** dans le registre d'U4 avec cette raison,
pas exemptée en passant.

### D5 — AC4 est tenu par un fragment unique et une garde, non par une fonction unique

AC4 demande « une seule primitive d'estampillage » et précise sa lettre :
« ne pas dupliquer le `UPDATE` ».

La fusion en une seule fonction n'est pas disponible, et mika#2335 a déjà refusé
cette voie par écrit en l'instruisant : `update_manual_task_status` sert aussi
`rewind.rs`, *« qui restaure un statut antérieur et ne doit rien stamper — un
rewind n'est pas un dispatch »*. Les écrivains diffèrent par leur garde
(`trigger_type = 'manual'`, `status IN (…)`), par ce qu'ils écrivent d'autre
(`process_id`, `status`) et par leur atomicité requise. Les réduire à un appel
commun signifierait soit relâcher ces gardes, soit passer le stamp dans un
second aller-retour — et un second aller-retour peut échouer entre les deux
écritures, laissant exactement la ligne `in_progress` sans `fired_at` que ce
ticket ferme.

Ce que ce plan livre à la place, et qui tient la lettre **et** l'esprit d'AC4 :

- **un fragment SQL unique**, `FIRED_AT_STAMP_IF_NULL`, constante de
  `db/tasks.rs`, interpolé par les quatre sites NULL-only — donc une seule
  définition textuelle de l'acte d'estampiller ;
- **une garde de source** (U4) qui refuse toute autre écriture littérale de
  `fired_at =` en production, avec un registre nommé des écrivains légitimes.

C'est le patron exact que mika#2335 a posé pour la moitié parent — un écrivain
nommé plus un scan de source —, et qu'un test comportemental ne peut pas
remplacer : la régression qu'il attrape n'est pas une décision fausse, c'est un
cinquième chemin muet qui laisse toutes les assertions existantes vertes.

### D6 — Le second symptôme (`status`) n'est pas livré ici ; la raison est écrite et le suivi ouvert

**Le blast radius, mesuré.** `get_active_callback_tasks_with_pid`
(`db/tasks.rs:2976`), le watchdog #959, filtre :

```sql
WHERE trigger_type = 'callback' AND status = 'in_progress' AND process_id IS NOT NULL
```

Sa population est **vide par construction aujourd'hui** : les lignes callback
portent leur PID en `pending`. Poser `in_progress` sur le chemin de travail y
ferait entrer d'un coup **toute** la population des pilotes vivants — et ce
watchdog **marque la tâche `failed`** quand le processus meurt.

**Ce périmètre a déjà été rencontré et borné par écrit.** mika#2272 a élargi la
population du reaper voisin à `status IN ('pending','in_progress')` et a
délibérément laissé celle-ci étroite, en l'instruisant : *« le watchdog callback
#959 garde délibérément la requête `in_progress` plus étroite — élargir cette
population-là change qui marque une tâche `failed` quand un processus meurt, ce
qui court contre le moniteur de spawn ; blast radius distinct, ticket
distinct. »*

Poser `in_progress` ici serait donc rouvrir en passant un périmètre qu'un ticket
voisin a explicitement fermé — un changement de comportement moteur habillé en
observabilité, sur un ticket p2 dont la priorité dit « ne casse pas la boucle ».
Les deux compteurs de concurrence (`count_active_callback_tasks_excluding`,
`count_active_callbacks_for_class`) lisent déjà `IN ('pending','in_progress')`
et seraient insensibles ; le watchdog ne l'est pas, et c'est lui qui tue.

**Ce que l'opérateur a gagné entre-temps, et qui change la forme du besoin.**
Le commentaire 1 demande qu'un callback en travail soit observable « sans avoir
à corréler avec `ps` ». mika#2335 a répondu à ce besoin par une autre voie :
`mika tasks <id>` affiche une ligne `Dispatch pilot:` alimentée par
`PilotLiveness` (`mika-cli/src/commands/tasks.rs:473`) — PID vérifié par
`process_start_time` et mtime du log du pilote —, et son doc-comment tranche la
question de fond : *« `fired_at` est maintenant estampillé (F2a), mais le signal
de vie faisant foi n'a jamais été la ligne : ce sont le PID et le mtime du log.
Ce type est ce signal, lu là où l'opérateur regarde plutôt que laissé comme un
geste dont il doit se souvenir. »* La corrélation manuelle avec `ps` est donc
déjà retirée de la charge de l'opérateur, par un signal plus juste que le statut.

**Ce que ce plan livre donc** (U6) : le fait écrit là où on le lit — un
doc-comment sur `get_active_callback_tasks_with_pid` nommant sa population vide
et le coût de l'élargir, plus une entrée `CLAUDE.md` — et un suivi nommé. C'est
la seconde branche que le commentaire 1 déclare acceptable ; le silence actuel,
qu'il refuse, cesse.

### D7 — Sémantique unique de `fired_at`, en une phrase qui couvre les quatre chemins

> `fired_at` est **le premier instant où le moteur a commencé à travailler sous
> cette ligne** — et pour une récurrente, le dernier tir (D4).

Concrètement : pour un dispatch, le spawn ; pour un tour a2a, la transition
`working` ; pour un callback sans pilote, le début de son tour de livraison ;
pour l'ordonnanceur, la réclamation.

Cette phrase est ce qui rend acceptable qu'une ligne callback avec pilote porte
l'instant du spawn pendant qu'une ligne sans pilote porte celui de sa
livraison : ce ne sont pas deux définitions, c'est une définition appliquée à
deux natures de travail. La clause NULL-only (D3) est ce qui la rend vraie —
sur une ligne portant un pilote, la livraison arrive après et ne réécrit rien.

**Ce que la phrase ne dit pas, délibérément** : une ligne callback `completed` en
attente de livraison porte déjà l'estampille de son pilote, et rien ne distingue
« en file » de « en cours de livraison » sur ce champ. Cette mesure-là existe
ailleurs et mika#2179 l'a livrée à sa place : `audit_events` avec
`tool_name = 'callback_delivered'`, dont l'`after_value` porte le `wait_secs`.
Dupliquer ici cette latence sur une colonne qui répond à une autre question
créerait deux mesures divergentes de la même attente.

---

## Scope Boundaries

**Dans le périmètre**

- Le stamp au point de passage a2a et au point de passage de livraison callback.
- Le fragment SQL partagé et la migration des deux sites NULL-only existants
  vers lui.
- La garde de source AC4 et son registre d'écrivains.
- La raison écrite de D6 et le suivi qui la porte.

**Hors périmètre**

- **Poser `in_progress` sur la ligne callback en travail** (D6) — suivi nommé.
- **Le seuil du test de file figée** — hors périmètre par le ticket.
- **mika#2132** — hors périmètre par le ticket.
- **La sémantique de `completed_at` / `updated_at`** — hors périmètre par le
  ticket.
- **Toute migration de données** — AC6 l'interdit. La colonne existe
  (`task_state/tasks.rs:104`, schéma courant), donc aucun DDL n'est requis
  non plus.
- **Modifier `claim_and_fire_task`** (D4).
- **Élargir `get_active_callback_tasks_with_pid`** (D6) : ce plan documente sa
  population, il ne la change pas.

---

## Implementation Units

### U1 — Le fragment SQL unique

`crates/mika-agent/src/db/tasks.rs` — introduire

```rust
/// L'acte d'estampiller, écrit une fois (mika#2133 AC4).
pub(crate) const FIRED_AT_STAMP_IF_NULL: &str = "fired_at = CASE \
     WHEN fired_at IS NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
     ELSE fired_at END";
```

et l'interpoler dans `mark_parent_dispatched` et `set_task_process_id`, qui en
portent aujourd'hui chacun une copie littérale. `set_task_process_id` conserve
sa condition supplémentaire `?1 IS NOT NULL` — un effacement de PID ne stampe
pas, propriété épinglée par un test existant qui doit rester vert.

Les doc-comments existants de ces deux méthodes ne sont pas réécrits : ils
portent la mesure de leurs incidents fondateurs (2026-09-09, 2026-09-15) et
cette réécriture n'est pas la leur. Une ligne y renvoie au fragment.

### U2 — Le stamp a2a

`crates/mika-agent/src/a2a_db.rs::a2a_update_task_state` — ajouter le fragment
au même `UPDATE`, conditionné au statut interne (D2) :

```rust
let sql = if internal_status == "in_progress" {
    format!("UPDATE tasks SET status = ?1, updated_at = ?2, completed_at = ?3, {} WHERE …",
            crate::db::tasks::FIRED_AT_STAMP_IF_NULL)
} else {
    "UPDATE tasks SET status = ?1, updated_at = ?2, completed_at = ?3 WHERE …".to_string()
};
```

Un seul aller-retour, donc la transition et le stamp restent un seul acte —
la propriété que le site jumeau formule ainsi : *« la transition et le stamp sont
un seul acte, donc ils sont un seul écrivain. »* Aucun appelant de
`server/a2a.rs` n'est modifié.

Le doc-comment de la méthode gagne la phrase de D7 et nomme la branche
`returnImmediately` comme population légitimement NULL.

### U3 — Le stamp du tour de livraison callback

`crates/mika-agent/src/db/tasks.rs` — un écrivain dédié :

```rust
/// Estampille `fired_at` sans toucher au statut (mika#2133 R2).
pub fn stamp_task_fired_at_if_null(&self, id: &str, agent_id: &str) -> Result<()>
```

Il ne touche **pas** au statut : c'est ce qui le distingue de
`mark_parent_dispatched` et ce qui le met hors du périmètre de D6.

`crates/mika-agent/src/task_engine/dispatcher.rs::dispatch_resume_agent`
(ligne 828) est le point de passage — il sert les callbacks **et** les
reminders, et `is_callback` y est calculé en première ligne. Appel conditionné à
`is_callback`, **avant** `run_silent_agent`, en `warn!`-et-continue : comme les
trois sites de dispatch existants, le stamp est de l'observabilité et ne doit
jamais faire échouer un tour.

Les reminders sont exclus : ils passent par `fire_task` → `claim_and_fire_task`
et sont déjà estampillés, avec la sémantique d'écrasement de D4 qu'un appel ici
contredirait.

### U4 — La garde AC4 et son registre

`crates/mika-agent/src/db/tests/` — voisine de la garde mika#2335, dont elle
reprend la mécanique (`mika_common::source_guard::ProductionScanner`, qui lit la
frontière production/test) et le prédicat sur source décommentée
(`dispatch_stamp_violations`, `db/tests/mod.rs:80`, sert de modèle).

Prédicat : tout fichier de production portant une écriture littérale de
`fired_at =` est une violation, **sauf** s'il figure au registre.

Registre livré, chaque entrée nommant son site et sa raison :

| site | forme | raison |
|---|---|---|
| `db/tasks.rs::FIRED_AT_STAMP_IF_NULL` | la constante | l'unique définition |
| `db/tasks.rs::claim_and_fire_task` | écrasement | D4 — « dernier tir » des récurrentes |

Les trois sites qui **interpolent** la constante n'ont pas besoin d'entrée : ils
ne portent plus d'écriture littérale, et c'est la propriété que la garde mesure.

Deux tests, comme chez mika#2335 :

- la garde elle-même, verte à l'atterrissage, avec l'assertion
  `scanned > 0` qui interdit qu'un chemin cassé la rende vacuous ;
- son **contrôle négatif**, qui lui présente une source fautive et vérifie
  qu'elle rougit — sans lui, une réparation de classification trop large la
  laisserait verte en ayant cessé de regarder (le mode de panne que mika#2321 et
  mika#2398 ont mesuré à 14 043 lignes de production perdues).

Disposition d'un futur site : **halt-and-surface**, mot pour mot comme mika#2335
— ni entrée de registre posée en passant, ni `#[ignore]`. La question « ce site
démarre-t-il un travail ou non ? » ne se pré-tranche pas ici.

**Ce que la garde ne couvre pas, et c'est écrit ici plutôt que découvert :
une absence d'appel.** U4 scanne des **écritures** littérales de `fired_at =` ;
elle ne voit pas un chemin qui *aurait dû* estampiller et ne le fait pas. La
population exposée est nommable précisément, parce qu'U3 est branché sur
`is_callback` dans `dispatch_resume_agent` — et que **la branche `else` de ce
site est un fourre-tout** (`Reminder path`), vérifié à `c8520787` : tout
`trigger_type` qui n'est pas `callback` y tombe. Aujourd'hui c'est correct, les
reminders étant déjà estampillés en amont par `claim_and_fire_task` (D4).

> **Si un nouveau `trigger_type` traverse `dispatch_resume_agent` sans passer
> par `claim_and_fire_task`, il faut décider s'il doit être estampillé par
> `stamp_task_fired_at_if_null` — la garde U4 ne couvre pas cette absence.**

Et le coût de l'omission n'est pas seulement une colonne vide de plus. C'est la
classe d'échec que mika#2263 a mesurée sur ce champ exact : *« Toute
sonde/faucheur qui filtre `fired_at IS NULL` … voit ces rows comme
non-dispatchées et NE les balaie PAS — le zombie est invisible à cette classe de
faucheur. »* Une population non estampillée n'est pas seulement invisible aux
sondes S1–S4 : **elle est invisible au filet, parce que le filet ne sait pas
qu'elle existe.** Un scan capable de voir cette absence demanderait une analyse
de flot que cette famille de gardes n'a pas (même limite que le coût lexical
nommé au § Fire-Disposition) ; ce qui est disponible, et qui est livré, est que
la question soit posée par écrit au site où le prochain éditeur la rencontrera.

### U5 — Tests comportementaux

`crates/mika-agent/tests/eval/test_dispatch_fired_at_stamped.rs`, en extension
du fichier existant plutôt qu'en nouveau fichier — c'est déjà le lieu déclaré de
cet invariant, et un second fichier en ferait deux.

- **T1 (AC1, a2a)** — `a2a_create_task` puis `a2a_update_task_state("working")` :
  `fired_at` non NULL et `status = 'in_progress'`.
- **T2 (AC3, contrôle négatif a2a)** — `a2a_create_task` **seul**, aucune
  transition : `fired_at` NULL. C'est la forme exacte de la branche
  `returnImmediately`.
- **T3 (AC1, callback sans PID)** — ligne callback sans `process_id`,
  `stamp_task_fired_at_if_null` : `fired_at` posé, `status` **inchangé**. La
  seconde moitié de l'assertion est celle qui tient D6 : elle rougit si
  quelqu'un ajoute une transition de statut à cet écrivain.
- **T4 (AC3, contrôle négatif callback)** — ligne callback créée et non
  dispatchée : `fired_at` NULL.
- **T5 (R4/D3)** — une ligne déjà estampillée traversant une seconde fois U2
  puis U3 conserve son estampille d'origine, à la valeur près.
- **T6 (AC6)** — aucune migration : la suite de migrations existante reste à sa
  version courante et aucun test de migration n'est ajouté. Vérifié par revue,
  pas par assertion — un test qui affirme qu'un DDL n'existe pas mesure sa
  propre absence.
- **T7 (D4, contrôle négatif de D4)** — `claim_and_fire_task` appelé deux fois
  sur une récurrente **avance** `fired_at`. Ce test dit que l'asymétrie est
  voulue : il rougit le jour où quelqu'un « harmonise » les quatre sites.

Rouge-avant : T1, T2 (non — T2 est vert avant, c'est un contrôle négatif), T3 et
T5 sont rouges sur `c8520787`. Recette d'injection pour la garde U4 : retirer
l'interpolation de la constante à l'un des sites et y remettre le littéral — la
garde nomme le site.

### U6 — La raison écrite (D6, R7)

**Quatre** écritures, aucune n'étant un changement de comportement :

1. **Doc-comment sur `get_active_callback_tasks_with_pid`** (`db/tasks.rs:2976`)
   — sa population est vide par construction, le PID vivant sur une ligne
   `pending` ; élargir le filtre fait entrer tous les pilotes vivants dans un
   watchdog qui marque `failed` ; renvoi à mika#2272 et au suivi.
2. **Entrée `CLAUDE.md`**, au voisinage des signaux de tâches : la phrase de D7,
   le tableau des quatre chemins et de leur écrivain, et la requête AC5 comme
   sonde opérateur.
3. **Le corps de mika#2133 lui-même** — la quatrième écriture, et celle sans
   laquelle R7 n'est pas tenu. Les deux premières vivent dans le code et dans
   `CLAUDE.md` ; **le lecteur du ticket ne lit ni l'un ni l'autre**, et c'est
   très précisément sur le ticket que le commentaire 1 refuse « le silence
   actuel ». Deux gestes, détaillés au Definition of Done : un encadré daté
   rectifiant la mesure du 2026-09-01, et un commentaire d'avis d'édition
   nommant la branche AC7 retenue. Convention de rectification des corps de
   tickets : mika#2169 / mika#2158, appliquée sur mika#2162.
4. **Le suivi nommé** dans le corps de PR et au § Suivi ci-dessous.

---

## Verification Contract

### Statique

- `cargo build` et `cargo clippy` propres.
- `cargo test -p mika-agent` — dont la garde U4 et son contrôle négatif, et la
  garde mika#2335 existante, qui doit **rester verte** : U1 la traverse en
  changeant la forme des deux sites qu'elle surveille.
- `cargo test -p mika-agent --test eval test_dispatch_fired_at_stamped`.
- `make verify-bundled-skills` et `scripts/verify-pipeline.sh` inchangés.

### Sondes post-déploiement, et leurs haltes

**S1 — AC5, non-vacuité, sur une fenêtre postérieure au déploiement.** Une
mesure d'avant ne prouve rien, et l'AC le dit :

```sql
SELECT trigger_type, COUNT(*), SUM(fired_at IS NOT NULL)
  FROM tasks
 WHERE created_at > '<instant du déploiement>'
 GROUP BY 1;
```

Attendu : `a2a` quitte le zéro. `callback` est **déjà** non nul depuis
mika#2263 — le lire comme une preuve de ce correctif serait attribuer à ce
travail ce qu'un autre a livré, et c'est la lecture que S2 existe pour éviter.

**Halte S1** — si `a2a` reste à zéro alors que des tours a2a ont tourné :
**ne pas élargir le prédicat par réflexe**. Établir d'abord que le binaire servi
porte le correctif (classe mika#2340 — `~/.mika/skills/.manifest-writer`, et la
version du démon). Un `a2a` à zéro sur un binaire antérieur est le résultat
attendu, pas un défaut du prédicat.

**S2 — attribution : la poche 2 est-elle non vide ?** La population « callback
sans PID » n'a pas pu être mesurée avant livraison (base inaccessible depuis le
bac à sable). Elle se lit ainsi :

```sql
SELECT COUNT(*) FROM tasks
 WHERE trigger_type = 'callback' AND process_id IS NULL
   AND fired_at IS NOT NULL AND created_at > '<instant du déploiement>';
```

Zéro **n'est pas une panne** : il signifie que tout callback porte un pilote et
que U3 est un filet sans population. C'est un résultat, à écrire comme tel — et
c'est alors U3 qui mérite d'être réinterrogé, pas le prédicat élargi.

**S3 — contrôle négatif en production, AC2/AC3.** Sur la même fenêtre, les
tâches a2a créées par `returnImmediately` et jamais exécutées doivent porter
`fired_at` NULL :

```sql
SELECT COUNT(*) FROM tasks
 WHERE trigger_type = 'a2a' AND status = 'pending' AND fired_at IS NOT NULL
   AND created_at > '<instant du déploiement>';
```

Attendu : **zéro**. Toute ligne ici est une estampille posée sur une tâche
jamais démarrée — c'est-à-dire le défaut d'AC3 réalisé, la colonne devenue
pleine et toujours aussi muette. **Halte** : désarmer par revert avant
diagnostic.

**S4 — AC6, non-régression historique.** Le compte de lignes estampillées
créées **avant** le déploiement ne doit pas bouger d'un tick :

```sql
SELECT COUNT(*) FROM tasks
 WHERE fired_at IS NOT NULL AND created_at < '<instant du déploiement>';
```

Relever cette valeur **avant** le déploiement. Une variation prouve une
rétro-estampille, qu'aucune ligne de ce plan ne produit — donc un écrivain
inattendu.

### Ce que ce travail n'achète pas

Aucun compteur, aucun événement de journal nouveau. Le défaut est une colonne
vide, et une colonne vide ne s'émet pas. Les seuls instruments sont les quatre
requêtes ci-dessus, et **leur silence ne prouve rien tant que personne ne les
exécute**. Le champ devient lisible ; il ne devient pas surveillé.

---

## Definition of Done

- U1 à U6 livrés, chacun avec ses tests.
- `cargo build`, `cargo clippy`, `cargo test -p mika-agent` verts.
- La garde mika#2335 existante verte après la réécriture d'U1.
- La garde U4 verte, et son contrôle négatif rouge sur source fautive.
- Aucune migration de schéma, aucun DDL, aucun `UPDATE` de données historiques
  dans le diff.
- Le corps de PR relève S1 à S4 comme sondes à exécuter après déploiement, avec
  la valeur de référence de S4 relevée avant.
- Le suivi D6 est ouvert et nommé dans le corps de PR.
- **Le corps de mika#2133 est rectifié** par un encadré daté reflétant la mesure
  à `c8520787` — poches `manual` et `callback`-avec-pilote closes respectivement
  par mika#2335 et mika#2263, restent `a2a` et `callback`-sans-pilote —, la
  mesure d'origine du 2026-09-01 étant **conservée et datée** plutôt que
  remplacée : c'est elle qui explique pourquoi le ticket a été ouvert.
  Convention de rectification des corps de tickets : mika#2169 / mika#2158,
  appliquée sur mika#2162.
- **Un commentaire d'avis d'édition est posté sur mika#2133**, nommant la
  branche AC7 retenue — seconde branche, la raison écrite (U6), sur le blast
  radius mesuré en D6 — et le suivi qui porte la première.
  **Pourquoi ce geste est au DoD et pas seulement dans U6** : le commentaire 1
  n'accepte que deux issues, « un état intermédiaire … **ou** une raison écrite
  quelque part … mais pas le silence actuel ». Une raison qui ne vit que dans un
  doc-comment (`db/tasks.rs:2976`) et dans `CLAUDE.md` n'atteint pas le lecteur
  du ticket, pour qui le silence demeure entier. Sans ces deux gestes, **R7
  n'est pas tenu et AC7 n'est pas livré** — un futur lecteur de mika#2133 y
  trouverait une mesure périmée de quatre mois et aucune trace de la décision.
- **Si l'un des deux gestes n'est pas exécutable** par l'implémenteur (portée
  outillée, jeton), il est **nommé comme tel dans le corps de PR** avec le
  contenu à poser, à destination de l'orchestrateur — jamais silencieusement
  omis, le silence sur le ticket étant exactement ce que ce DoD ferme.

---

## Acceptance criteria

Transcrits du ticket, avec leur disposition dans ce plan.

- **AC1** — Toute tâche qui démarre porte `fired_at`, quel que soit son
  `trigger_type`. Vérifié sur `callback` et `a2a`, les deux chemins aujourd'hui
  à zéro.
  → U2 (a2a), U3 (callback sans pilote), T1/T3, sonde S1. Pour `callback` avec
  pilote, déjà clos par mika#2263 — le plan le mesure plutôt que de le
  reproduire.
- **AC2** — Une tâche jamais tirée garde `fired_at` **NULL**. La distinction
  « en attente » / « en travail » redevient lisible.
  → D3 (NULL-only), aucune écriture à la création, T2/T4, sonde S3.
- **AC3** — **Contrôle négatif obligatoire** : un test crée une tâche et ne la
  déclenche pas ; `fired_at` doit être **NULL**.
  → T2 (a2a, forme exacte de `returnImmediately`) et T4 (callback). Doublés en
  production par la sonde S3.
- **AC4** — Une seule primitive d'estampillage. Réutiliser `claim_and_fire_task`
  ou extraire ce qu'il faut ; ne pas dupliquer le `UPDATE`.
  → D5 : fragment unique `FIRED_AT_STAMP_IF_NULL` (U1) plus la garde de source
  U4 et son registre. L'asymétrie de `claim_and_fire_task` est déclarée avec sa
  raison (D4) et épinglée par T7, non exemptée.
- **AC5** — Preuve de non-vacuité : mesurer
  `SELECT trigger_type, COUNT(*), SUM(fired_at IS NOT NULL) FROM tasks GROUP BY 1`
  sur une fenêtre postérieure.
  → Sonde S1, avec sa halte. La base n'étant pas lisible depuis le bac à sable de
  dispatch, cette mesure est structurellement post-déploiement et le corps de PR
  la porte.
- **AC6** — Les tâches **historiques** ne sont pas rétro-estampillées.
  → Aucune migration, aucun DDL, aucun `UPDATE` de données dans le diff ; sonde
  S4 en non-régression, avec sa valeur de référence relevée avant déploiement.
- **AC7** (suggéré au commentaire 1) — Qu'une tâche `callback` en cours
  d'exécution soit observable par une requête sur `status` seule, « **ou** une
  raison écrite quelque part expliquant pourquoi ce chemin n'en a pas besoin —
  mais pas le silence actuel ».
  → **Seconde branche**, livrée par U6, sur la mesure de D6 : le watchdog #959
  marque `failed`, sa population est vide par construction, et mika#2272 a borné
  ce périmètre par écrit. Le suivi porte la première branche. Le signal de vie
  faisant foi est par ailleurs déjà rendu à l'opérateur par `PilotLiveness`
  (mika#2335), ce qui retire du besoin la corrélation manuelle avec `ps` que le
  commentaire nomme.
  **« Quelque part » a un lieu, et c'est le ticket.** Cet AC n'est pas tenu par
  le doc-comment et l'entrée `CLAUDE.md` seuls : ils sont lus par qui lit le
  code, pas par qui lit mika#2133. L'AC7 est livré quand les quatre écritures
  d'U6 le sont, **y compris la rectification du corps et le commentaire d'avis
  d'édition** portés au Definition of Done. C'est la condition qui rend vraie la
  phrase « le silence actuel cesse ».

---

## Fire-Disposition

Ce plan livre **un détecteur** : la garde de source U4, dont le chemin de succès
est « aucune violation trouvée ».

**Option retenue : (a) — registre nommé, grep-visible.**

Le détecteur atterrit **armé et vert**. Le registre ne contient aucune violation
à corriger plus tard : il déclare les deux sites qui écrivent légitimement
`fired_at` en littéral, chacun avec sa raison lisible sur place.

| entrée | raison | ticket |
|---|---|---|
| `db/tasks.rs::FIRED_AT_STAMP_IF_NULL` | l'unique définition de l'acte | mika#2133 D5 |
| `db/tasks.rs::claim_and_fire_task` | écrasement voulu — « dernier tir » | mika#2133 D4 |

C'est une **déclaration, pas une exemption** — la distinction que mika#2201 a dû
écrire pour le lint de jetons canoniques (« on déclare, on n'allowliste pas »).
Aucune entrée ne masque un défaut à réparer ; chacune nomme un écrivain dont
l'existence est une décision de ce plan.

**Assertion auto-nettoyante.** Le registre est comparé **dans les deux sens** :
une entrée dont le site ne porte plus d'écriture littérale de `fired_at` fait
rougir la garde, au même titre qu'un site non déclaré. Sans cette moitié, la
suppression de `claim_and_fire_task` laisserait une entrée morte qui exempterait
silencieusement un futur homonyme. Patron :
`scripts/check-dispatch-seats-declared.sh` (mika#2092) et
`mika2201_every_declared_symbol_still_exists`.

**Contrôle négatif livré avec la garde.** Une réparation de classification trop
large rendrait le détecteur vacuous sans rien casser — vert parce qu'il ne
regarde plus rien. Le test compagnon lui présente une source fautive et vérifie
qu'il rougit, et la garde porte `assert!(scanned > 0)`. C'est la moitié que
mika#2321 et mika#2398 ont mesurée : 14 043 lignes de production sorties du
champ d'une garde restée verte.

**Disposition d'un cinquième site : halte-et-remontée**, mot pour mot comme la
garde sœur de mika#2335. Pas d'entrée posée en passant, pas de `#[ignore]`. La
question ne se pré-tranche pas ici : ce site démarre-t-il un travail — il
interpole alors la constante et il était un nouveau visage du défaut — ou non —
et c'est la garde qu'il faut affiner ?

**Coût assumé, nommé.** La portée du prédicat est lexicale sur `fired_at =`. Un
futur site qui construirait son `UPDATE` par concaténation dynamique passerait
dessous. C'est la même limite que la garde mika#2335 documente pour elle-même
(« portée lexicale sur le littéral `"in_progress"`, et c'est un coût assumé ») :
un scan sémantique demanderait une analyse de flot que cette famille de gardes
n'a pas. La limite est écrite sur la garde, pas découverte plus tard.

**Seconde limite, de nature différente et plus coûteuse : la garde voit des
écritures, jamais des absences.** Un chemin qui aurait dû appeler
`stamp_task_fired_at_if_null` et ne l'appelle pas la laisse verte. Le cas
concret — un nouveau `trigger_type` traversant `dispatch_resume_agent`, dont la
branche `else` est un fourre-tout — est nommé en toutes lettres au § U4, avec la
décision qu'il appelle et le mode de panne de mika#2263 qu'il rejoue.

---

## Suivi (hors périmètre, nommé)

- **`mika` — un état intermédiaire pour la ligne callback en travail (D6, AC7
  première branche).** Préalable écrit : décider qui marque `failed` quand un
  pilote meurt, une fois la population du watchdog #959 non vide — c'est la
  question que mika#2272 a nommée et renvoyée (« ce qui court contre le moniteur
  de spawn »). Une mesure préalable utile : le nombre de lignes callback
  portant un PID vivant à un instant donné, que ce plan ne peut pas relever
  depuis le bac à sable. **Ticket à ouvrir.**
- **`mika` — la latence de file de livraison sur `fired_at`.** Délibérément non
  livrée (D7) : la mesure existe sous
  `audit_events.tool_name = 'callback_delivered'` (mika#2179). À rouvrir
  seulement si une mesure montre que cette surface ne suffit pas — et alors
  comme une question de surface, pas comme une seconde écriture du même fait.
- **`mika` — refermer mika#2133 si les sondes S1/S2 montrent les deux poches
  closes.** Le ticket restant ouvert sur une mesure du 2026-09-01 est ce qui a
  fait re-découvrir ici un défaut aux trois quarts fermé ; la sonde est aussi ce
  qui autorise à le clore avec un compte plutôt qu'avec une impression.

---

## Références

**Code lu à `c8520787`**

- `crates/mika-agent/src/db/tasks.rs:638` — `claim_and_fire_task` (D4)
- `crates/mika-agent/src/db/tasks.rs:838` — `mark_parent_dispatched` (mika#2335)
- `crates/mika-agent/src/db/tasks.rs:2931` — `set_task_process_id` (mika#2263)
- `crates/mika-agent/src/db/tasks.rs:1625` — `get_undelivered_callback_tasks`
- `crates/mika-agent/src/db/tasks.rs:2976` — `get_active_callback_tasks_with_pid` (D6)
- `crates/mika-agent/src/a2a_db.rs:64` — `a2a_state_to_task_status` (D2)
- `crates/mika-agent/src/a2a_db.rs:255` — `a2a_update_task_state` (U2)
- `crates/mika-agent/src/server/a2a.rs:1154`, `:1627` — les deux appelants `working`
- `crates/mika-agent/src/task_engine/dispatcher.rs:828` — `dispatch_resume_agent` (U3)
- `crates/mika-agent/src/skills/executor.rs:3582` — le seul appelant de production de `set_task_process_id`
- `crates/mika-cli/src/commands/tasks.rs:473` — `PilotLiveness` (D6)
- `crates/mika-agent/src/db/tests/dispatch_stamp_and_slots.rs` — la garde sœur
- `crates/mika-agent/src/db/tests/mod.rs:80` — `dispatch_stamp_violations`, modèle d'U4
- `crates/mika-agent/tests/eval/test_dispatch_fired_at_stamped.rs` — le fichier étendu par U5

**Tickets**

- mika#2133 (ce ticket) · mika#2263 défaut (b) · mika#2335 F2a · mika#2272 ·
  mika#2179 · mika#959 · mika#525 · mika#2321 / mika#2398 (frontière
  production/test) · mika#2201 (« on déclare, on n'allowliste pas ») ·
  mika#1574 (les trois dispositions) · mika#2092 (comparaison à double sens)

---

## Revision history

- **v1** (2026-09-21) — plan initial. Rectifie la mesure du ticket sur le code à
  `c8520787` : les poches `manual` et `callback`-avec-pilote sont closes par
  mika#2335 et mika#2263 ; restent `a2a` et `callback`-sans-pilote. Tranche le
  second symptôme du commentaire 1 par la branche « raison écrite » que ce
  commentaire déclare acceptable, sur le blast radius mesuré du watchdog #959.
- **v2** (2026-09-21) — révision sur la première passe architecte (ITERATE).
  **F1 (BLOCKING) adressée** : le Definition of Done exige désormais les deux
  gestes sur le ticket — encadré daté rectifiant la mesure du 2026-09-01
  (conservée, non remplacée) et commentaire d'avis d'édition nommant la branche
  AC7 retenue —, avec la convention citée (mika#2169 / mika#2158, appliquée sur
  mika#2162) et une clause de remontée dans le corps de PR si l'un des deux
  n'est pas exécutable par l'implémenteur. U6 passe de trois à quatre écritures
  et l'AC7 dit maintenant que « quelque part » a un lieu : le ticket, un
  doc-comment et `CLAUDE.md` n'atteignant pas son lecteur.
  **F2 (sharpening) adressée** : § U4 nomme la limite « la garde voit des
  écritures, jamais des absences », avec la question à trancher si un nouveau
  `trigger_type` traverse `dispatch_resume_agent` sans passer par
  `claim_and_fire_task`, et la citation de mika#2263 sur le zombie invisible au
  faucheur. Le § Fire-Disposition y renvoie en seconde limite. Précision
  vérifiée en chemin à `c8520787`, qui rend la limite plus aiguë que la finding
  ne la formulait : la branche `else` de `dispatch_resume_agent` est un
  fourre-tout (`Reminder path`), donc un nouveau `trigger_type` y tomberait
  silencieusement.
  Aucune AC affaiblie ; aucune décision D1–D7 modifiée ; aucun périmètre élargi.
