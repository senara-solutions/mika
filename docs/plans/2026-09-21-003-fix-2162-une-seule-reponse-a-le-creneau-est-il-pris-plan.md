# mika#2162 — « Le créneau est-il pris ? » : une seule réponse, et ce n'est pas le bail

**Ticket :** mika issue#2162
**Type :** fix (unification d'un prédicat dupliqué + observabilité de la rétention)
**Date :** 2026-09-21

---

## Problème

Le ticket mesure un tourniquet réel — huit réveils `deferred dispatch slot freed`
en cinq heures, dont deux suivis à la seconde d'un refus `global_dispatch_active` —
et l'impute à une divergence entre **le bail de créneau** (`dispatch_slot_leases`,
expiré depuis 4 h 12) et **la garde de dispatch**. Il demande, en AC1, que la
question « le créneau est-il libre ? » ait une seule réponse : soit le bail est
renouvelé pendant toute la vie du dispatch, soit le chemin de reprise différée
interroge la même chose que la garde.

**La lecture du code déplace le diagnostic de deux crans, et c'est le premier
livrable de ce plan.** La prémisse causale est fausse : le chemin de reprise
différée n'a jamais lu le bail. Mais la classe que le ticket nomme — deux
mécanismes répondant à une même question par deux mesures différentes — est
**réelle, mesurée, et atteignable** ; elle vit simplement sur un autre axe, et
dans le sens inverse de celui qui est décrit.

### M1 — Aucun chemin de reprise différée ne lit le bail, et ce n'était déjà pas le cas le jour de la mesure

À HEAD, le seul appelant de production de `dispatch_slot_lease_holder` est
`TaskEngine::reap_stale_blocked_dispatch_tasks` (`task_engine/engine.rs:1444`),
le filet L3b de mika#2169 — qui **n'émet jamais** `deferred dispatch slot freed`.

Au jour de la mesure, il n'y en avait aucun :

```
$ git grep -n "dispatch_slot_lease_holder" 9171ef96 -- crates/mika-agent/src/
async_db.rs:1309     pub async fn dispatch_slot_lease_holder(       ← le wrapper
async_db.rs:1315         self.with_db(...)                          ← le wrapper
db.rs:8208           pub fn dispatch_slot_lease_holder(             ← la définition
db.rs:13149/13163/13197                                             ← trois tests
```

`9171ef96` est le HEAD du 2026-09-04, la veille de la première introduction d'un
lecteur :

```
$ git log --format='%h %ad %s' --date=short -S "dispatch_slot_lease_holder" \
      -- crates/mika-agent/src/task_engine/engine.rs \
         crates/mika-agent/src/skills/executor.rs \
         crates/mika-agent/src/task_engine/dispatcher.rs
241763a2 2026-09-05 docs(solutions): deux façons de rester muet … (mika#2169)
```

Un seul commit, **deux jours après** la mesure du 2026-09-03. Le tableau « les
deux lectures » du ticket oppose donc un lecteur (la garde) à une fonction que
personne n'appelait.

### M2 — Le bail expiré pendant le dispatch est le comportement voulu, écrit noir sur blanc

`Database::try_acquire_dispatch_slot` (`db/tasks.rs`, § *On the TTL, and why it
is short*) :

> The lease guards one narrow window: from the moment validation completes to the
> moment the callback row exists. That is a process spawn — seconds. The TTL must
> exceed that, and **must stay far below the duration of a real dispatch**
> (minutes), so that a lease can never outlive the work it guarded and block a
> slot that has legitimately freed. Once the callback row exists, the ordinary
> active-callback check is the durable holder and the lease is redundant;
> **letting it lapse is the intended end of life.**

Les « 98 % de la vie du dispatch » que le ticket compte sont la propriété
recherchée, pas un écart. Le bail et la garde ne répondent pas à la même
question : le bail arbitre **une course** (« puis-je prendre ce créneau à cet
instant, dans la fenêtre où rien de durable ne le prouve ? »), la garde constate
**une occupation** (« une ligne de rappel active existe-t-elle ? »). Renouveler le
bail pendant la session, c'est-à-dire l'option offerte par AC1/AC4, en ferait un
**second** marqueur d'occupation — donc une deuxième réponse à la question que
AC1 veut n'en voir qu'une. Voir D1.

### M3 — La divergence existe, entre deux familles de prédicats, et un doc-comment affirme le contraire

**Cinq** méthodes de `db/tasks.rs` répondent à « le créneau de classe C est-il
occupé ? », en **deux** clauses `WHERE` incompatibles :

| méthode | `action_type='resume_agent'` | `parent_task_id IS NOT NULL` | appelant de production |
|---|---|---|---|
| `has_active_callback_tasks_excluding` | **absent** | **présent** | garde (`cap == 1`, le défaut livré) |
| `count_active_callback_tasks_excluding` | **absent** | **présent** | garde (`cap > 1`) |
| `has_any_active_callback_for_class` | **présent** | **absent** | force-promote (mika#1453) |
| `count_active_callbacks_for_class` | **présent** | **absent** | backstop périodique (`engine.rs:1665`) |
| `has_any_active_callback` | **présent** | **absent** | *aucun* |

Or le doc-comment de `count_active_callbacks_for_class` énonce :

> Counting companion to [`Database::has_any_active_callback_for_class`], **with
> the identical WHERE clause**.

Vrai de son jumeau, faux de la garde qu'elle est censée doubler — et c'est
exactement la forme que mika#2158 a dû défaire une fois (`auto_pull.rs` portait
une regex commentée *« Mirrors GROOMED_VERDICT_RE in skills/executor.rs »* qui
n'a suivi aucun des deux élargissements suivants).

**Les deux termes divergent en sens opposés, et un seul des deux sens est
dangereux :**

- `action_type = 'resume_agent'` **restreint** la population d'occupants vue par
  le backstop. Le backstop voit donc **moins** d'occupants que la garde → il peut
  dire « libre » là où la garde dira « occupé » → **réveil stérile**, la forme
  exacte du symptôme du ticket.
- `parent_task_id IS NOT NULL` restreint la population vue par **la garde**. La
  garde voit donc moins d'occupants que le backstop → elle peut **autoriser un
  second dispatch de la même classe** sur une ligne que le backstop compte.

### M4 — La seconde branche est atteignable aujourd'hui, par `build_mika` et `deploy_mika`

`execute_long_running` dérive le parent du champ que le modèle fournit
(`skills/executor.rs:3335`) :

```rust
let parent_task_id = input
    .get("task_id")
    .and_then(|v| v.as_str())
    .filter(|s| !s.is_empty())
    .map(|s| s.to_string());
```

`build_callback_task` prend donc un `Option<String>`, et trois de ses quatre
appelants de production passent `Some(…)` en dur (`dispatcher.rs:3891`,
`ready_label_handler.rs:1028`, `verdict_handler.rs:893`). Le quatrième est ce
chemin-ci, et il est gouverné par le schéma déclaré de l'outil :

```
$ grep -n required skills/bundled/*/tools.json
dev-pilot/tools.json:    "required": ["skill", "prompt", "task_id"]
dev-groom/tools.json:    "required": ["skill", "prompt", "task_id"]
build-mika/tools.json:   "required": []          ← long_running: true
deploy-mika/tools.json:  "required": []          ← long_running: true
```

`build_mika` et `deploy_mika` sont **`long_running: true`**, tombent en classe
`implement` (`derive_dispatch_class`, `_ => "implement"`), et **n'exigent pas de
`task_id`**. Un de ces dispatchs lancé sans `task_id` écrit une ligne de rappel
`parent_task_id = NULL` qui est **invisible à la garde** et **comptée par le
backstop**. `build_mika` est le chemin de la revue QA (mika#2355) : ce n'est pas
un chemin froid.

Le bail ne rattrape pas cette fenêtre : il expire en 120 s, alors qu'un `build`
ou un `deploy` dure des minutes.

### M5 — Ce qui reste non attribué, et pourquoi il le restera

Le symptôme précis du 2026-09-03 a au moins trois explications que la lecture du
code ne départage pas :

1. **Deux classes distinctes.** Le wrapper promu est de classe `groom` (libre),
   le refus porte sur `implement` (occupée par le pilote de 4 h). Aucune
   divergence : deux créneaux, deux réponses justes.
2. **TOCTOU inhérent.** La promotion écrit `status='completed'` ; le tour
   `DeferredDispatch` qu'elle arme tourne **au tick suivant**. Entre les deux, un
   webhook peut prendre le créneau. La garde est l'arbitre final, et c'est
   correct.
3. **La divergence M3/M4**, si une ligne hors-forme existait ce soir-là.

Les journaux du 2026-09-03 ont tourné et ne sont pas consultables depuis un bac à
sable de dispatch. **Ce plan ferme la classe et instrumente la rétention ; il
n'affirme pas résoudre l'épisode mesuré.** La sonde post-déploiement (V6) est ce
qui permettra de trancher à la prochaine occurrence, avec un compte plutôt qu'une
intuition.

---

## Requirements

- **R1** — La question « le créneau de classe C est-il occupé ? » a **un seul
  site SQL**. Garde et backstop le consomment ; la seule différence admise entre
  eux est le terme d'exclusion, passé en paramètre.
- **R2** — La population d'occupants vue par le backstop est un **sur-ensemble**
  de celle vue par la garde. Un réveil stérile par divergence de prédicat devient
  structurellement impossible.
- **R3** — Une seconde écriture de cette clause `WHERE` hors du site unique fait
  rougir un scan de source. Allowlist livrée vide.
- **R4** — Le bail n'est **pas** renouvelé pendant la session, et la raison est
  écrite à côté de la constante, pas seulement dans ce plan.
- **R5** — Une rétention de promotion (des wrappers en attente, un créneau
  occupé) est **dite** à un niveau collecté. Zéro wrapper en attente ⇒ zéro ligne.
- **R6** — Aucune valeur de réglage ne bouge : ni `DISPATCH_SLOT_LEASE_TTL_SECS`,
  ni `MAX_CONCURRENT_IMPLEMENT_DEFAULT`, ni le cap de `groom`, ni aucune cadence.
- **R7** — Aucune variable d'environnement n'est créée.

---

## Décisions

### D1 — L'option « bail renouvelé » (AC1 branche A, AC4) est **refusée**, et la raison est mesurée

Un battement périodique par le détenteur transformerait le bail d'arbitre de
course en marqueur d'occupation durable. Trois conséquences, chacune rédhibitoire :

- **Elle crée la deuxième réponse que AC1 veut supprimer.** Deux marqueurs
  d'occupation — le bail battu et la ligne de rappel — peuvent diverger, et leur
  divergence serait alors *durable* au lieu d'être bornée à 120 s.
- **Elle a besoin d'un cas « détenteur mort » que AC4 reconnaît lui-même**, et ce
  cas est déjà couvert par trois mécanismes plus fiables qu'un battement : le
  watchdog PID (#959), le reaper de silence (mika#2249/#2277) et le balayage
  phantom (mika#1712). Ajouter un quatrième détecteur de mort de pilote, keyé sur
  l'absence d'un battement, c'est un quatrième prédicat à garder cohérent avec
  les trois autres.
- **Elle contredit une décision écrite** (M2). Le TTL court est ce qui rend le
  fail-closed non bloquant ; un bail battu qui survit à un processus zombie
  bloque sa classe *pour toujours*, ce que le TTL court existe précisément pour
  empêcher.

Branche B retenue : **le chemin de reprise différée interroge la même chose que
la garde** — ce qu'il fait déjà, et que U1 rend structurel.

### D2 — Le prédicat unifié est **celui de la garde**

Arbitrage par le sens du danger (M3) :

- `action_type = 'resume_agent'` est **retiré**. Il restreint la population
  d'occupants dans le sens qui produit un réveil stérile. Population concernée
  aujourd'hui : **vide** (les quatre sites de production posent `RESUME_AGENT`),
  donc le changement est inerte et ferme une mine plutôt qu'un défaut actif.
- `parent_task_id IS NOT NULL` est **conservé**. La garde en a besoin
  structurellement (elle renvoie un `parent_task_id` pour nommer le bloquant, et
  `!= ?1` exige la non-nullité en SQL). Le backstop cesse donc de compter les
  lignes orphelines — dans le même sens que la garde, donc sans créer de réveil
  stérile.
- L'agrégat devient `COUNT(DISTINCT parent_task_id)` ; le `COALESCE(…, id)` du
  backstop devient sans objet une fois la non-nullité garantie.

**Propriété qui en découle, et c'est elle qui vaut R2 :** le backstop ne passe
aucun terme d'exclusion, donc sa population d'occupants est un sur-ensemble strict
de celle de la garde. Un backstop qui voit *plus* d'occupants se retient plus
souvent — jamais l'inverse.

### D3 — La faille de concurrence de M4 n'est **pas** refermée ici

Retirer `parent_task_id IS NOT NULL` de la garde la rendrait cohérente avec le
backstop *dans l'autre sens* et fermerait la fenêtre « deux dispatchs
`implement` simultanés ». C'est **hors périmètre**, pour deux raisons :

- La garde **doit** nommer un bloquant (`BlockingDispatch.parent_task_id`) ; la
  rendre tolérante aux orphelins demande de décider ce qu'elle rapporte alors,
  c'est-à-dire de changer le contrat de son refus.
- Le remède juste est en amont : **déclarer `task_id` requis** dans les schémas
  de `build-mika` et `deploy-mika`, ce qui rend la ligne orpheline
  inconstructible. C'est un changement de contrat d'outil, avec son propre rayon
  d'explosion (tout appel existant sans `task_id` serait refusé), et il mérite sa
  propre mesure.

**Ticket de suivi**, avec pour préalable la mesure V6b (combien de lignes de
rappel orphelines existent en base). Ce plan **nomme** la faille, la mesure, et
n'y touche pas.

### D4 — AC2/AC5 : le TOCTOU n'est pas refermé, et ne peut pas l'être

AC5 demande que `deferred dispatch slot freed` ne soit émis « que quand le créneau
est réellement prenable ». La lecture forte de cette phrase est inatteignable : la
promotion **arme un tour** et ce tour s'exécute au tick suivant ; le monde peut
changer entre les deux, et c'est la raison d'être de la garde. Rendre les deux
atomiques voudrait dire dispatcher depuis la transaction de promotion, ce qui
placerait un spawn de processus dans une transaction SQLite `IMMEDIATE`.

Ce qui **est** livrable, et ce que U1 livre : le prédicat de promotion ne peut
plus dire « libre » là où la garde dit « occupé » **pour une raison
structurelle**. Le résidu — la fenêtre d'un tick — est nommé, et U3 le rend
comptable au lieu de le laisser muet.

### D5 — Le texte `deferred dispatch slot freed` ne change pas

C'est un `tasks.result`, lu par un test (`dispatcher.rs:6170`), cité par un
commentaire (`dispatcher.rs:1113`) et affiché par `mika tasks get`. C'est un
format de fil de facto. Le reformuler pour qu'il « affirme moins » changerait une
surface opérateur pour un gain purement rédactionnel, alors que la propriété
demandée est obtenue par U1. Un renommage serait une rupture à dater, pas une
correction.

### D6 — La rétention de promotion est dite, et seulement quand elle existe

Aujourd'hui, le `continue` sur `class_cap_reached` (`engine.rs:1666`) est **muet**.
« Le backstop s'est retenu parce que le créneau est pris » et « le backstop n'a
rien trouvé à promouvoir » se lisent identiquement — classe mika#2205, et un
`debug!` ne serait pas collecté (mesuré par mika#2131 : zéro occurrence d'un
`debug!` du même module contre 184 d'un `info!` voisin).

Condition d'émission : **au moins un wrapper en attente ET le créneau au cap**.
Zéro wrapper ⇒ zéro ligne, ce qui est le régime courant. Le comptage des wrappers
passe **en tête** de la boucle, ce qui économise au passage une requête dans le
cas nominal. Une ligne INFO par classe par tick pendant une rétention réelle,
soit au plus 60/heure/classe, et c'est de l'information vive au sens de
mika#2329 : ce que l'opérateur veut savoir est que la promotion est retenue
*maintenant*.

---

## Scope Boundaries

**Dans le périmètre :**
- L'unification du prédicat d'occupation de créneau et sa garde structurelle.
- L'observabilité de la rétention de promotion.
- La consignation écrite du refus D1, au site de la constante.

**Hors périmètre, nommé :**
- **N'importe quelle valeur de réglage** (R6) : TTL du bail, cap `implement`, cap
  `groom` (mika#2160), cadences.
- **La faille de concurrence M4/D3** — ticket de suivi, préalable V6b.
- **Le bail lui-même** : `try_acquire_dispatch_slot`, `release_dispatch_slot`, le
  schéma v52, `SlotClaim`. Rien n'y est touché ; AC3 porte sur une non-régression,
  pas sur une modification.
- **`reap_stale_blocked_dispatch_tasks` (L3b, mika#2169)**, seul lecteur de
  production du bail. Sa lecture est **légitime** — il demande « quelqu'un
  revendique-t-il ce créneau à l'instant ? », qui est bien la question à laquelle
  le bail répond. Il n'émet jamais « slot freed ».
- Le balayage phantom (mika#2156) et le cap N (mika#2160), exclus par le ticket.

---

## Implementation Units

### U1 — Un seul site pour « ce créneau est-il occupé ? » *(R1, R2, D2 ; AC1, AC5)*

Dans `crates/mika-agent/src/db/tasks.rs`, une constante privée porte la clause
`WHERE` unique, et les méthodes existantes la consomment via `format!`. Le terme
d'exclusion est optionnel : `Some(parent_id)` pour la garde, `None` pour le
backstop et le force-promote.

- `has_active_callback_tasks_excluding` et `count_active_callback_tasks_excluding`
  (garde) : clause inchangée dans les faits, désormais lue depuis le site unique.
- `count_active_callbacks_for_class` et `has_any_active_callback_for_class`
  (backstop, force-promote) : perdent `action_type = 'resume_agent'`, gagnent
  `parent_task_id IS NOT NULL`, agrégat `COUNT(DISTINCT parent_task_id)`.
- `has_any_active_callback` (aucun appelant de production) : **supprimée**, et non
  alignée. Son doc-comment la déclare « retained as a regression-test baseline » ;
  une cinquième écriture d'un prédicat qu'on vient d'unifier est précisément
  l'endroit où la prochaine divergence s'installerait. Les tests qui s'appuient
  dessus basculent sur la forme scopée par classe.
- Le doc-comment mensonger (« with the identical WHERE clause ») est remplacé par
  l'énoncé de la propriété de sur-ensemble de D2.

### U2 — Le scan de source qui tient U1 *(R3)*

`db::tests::mika2162_le_predicat_doccupation_a_un_seul_site` : refuse toute
occurrence, hors du site unique, des fragments qui composent la clause
(`trigger_type = 'callback'` conjoint à `label NOT LIKE '%:deferred'`) dans
`crates/mika-agent/src`. **Allowlist livrée vide** : quand il tire, on consomme le
site unique, on n'allowliste pas.

Un test comportemental ne peut pas voir cette classe : une copie ne rend aucune
décision fausse **le jour où elle est écrite** — elle diverge des mois plus tard,
en silence, avec toutes les assertions vertes. C'est littéralement ce qui s'est
produit ici, doc-comment « identical » compris.

Le scan doit exclure les fichiers de test via
`crate::source_scan::is_test_source_path` — les tests DB vivent sous `db/tests/`
depuis mika#2321, et la troncature au premier `#[cfg(test)]` ne les couvre pas.

### U3 — La rétention de promotion cesse d'être muette *(R5, D6 ; AC5)*

Dans `TaskEngine::promote_pending_deferred_if_idle` (`engine.rs:1656`), réordonner
la boucle : compter d'abord les wrappers en attente de la classe (`continue`
silencieux si zéro), puis lire l'occupation. Sur `class_cap_reached`, émettre
`deferred_promotion_withheld` (INFO, champs `dispatch_class`, `pending_wrappers`,
`active`, `cap`, `agent_id`).

Pas de ligne `audit_events` : la population est une décision par classe par tick
pendant une rétention, et une ligne durable par tick serait le churn que la
doctrine mika#2131 borne. L'information durable — « un wrapper a été promu » —
existe déjà sous `deferred_dispatch_promoted`.

### U4 — Le refus D1 est écrit au site de la constante *(R4 ; AC4)*

Un paragraphe sur `DISPATCH_SLOT_LEASE_TTL_SECS` (`db.rs:288`) et sur
`dispatch_slot_lease_holder` (`db/tasks.rs:3576`) énonçant : le bail arbitre une
course, pas une occupation ; il n'est **pas** renouvelé pendant la session, et
mika#2162 a refusé de le faire ; la question de l'occupation a un seul lecteur,
celui de U1 ; et le seul appelant légitime de `dispatch_slot_lease_holder` est
celui qui demande « quelqu'un revendique-t-il à l'instant ».

C'est la moitié qui empêche la question d'être rouverte au jugé dans six mois —
la même raison qui a fait écrire M2 au site de `try_acquire_dispatch_slot`.

### U5 — Consignation *(documentation)*

- `crates/mika-agent/CLAUDE.md` § *Dispatcher-source arbitration* : un paragraphe
  sur les deux questions distinctes (course vs occupation), le site unique de U1,
  la propriété de sur-ensemble, et le renvoi vers le suivi D3.
- `CLAUDE.md` racine : la ligne `deferred_promotion_withheld` dans les signaux
  opérateur du voisinage `deferred_dispatch_*`.
- `docs/solutions/best-practices/` : entrée sur la classe « un doc-comment qui
  affirme *identical* n'est pas un test » — troisième occurrence après mika#2158
  et mika#1163.

---

## Verification Contract

| # | Vérification | Forme |
|---|---|---|
| **V1** | Le backstop ne promeut pas pendant qu'un dispatch de la classe est actif, **bail expiré posé explicitement dans la fixture** (TTL = 0) | test d'intégration ; c'est AC2, et le bail expiré y est le **contrôle négatif** qui atteste que le bail ne décide pas |
| **V2** | Contrôle négatif de V1 : le même scénario sans dispatch actif **promeut** | test — sans lui, « ne promeut pas » serait satisfait par un backstop inerte |
| **V3** | Une ligne de rappel `action_type != 'resume_agent'` avec parent est vue **occupante par les deux** | test unitaire DB ; rougit sur le code d'avant U1 |
| **V4** | Une ligne de rappel orpheline est vue **libre par les deux** | test unitaire DB ; rougit sur le code d'avant U1 |
| **V5** | AC3 — deux acquisitions concurrentes dans la même fenêtre de 120 s : une seule obtient le créneau | `tests/dispatcher_contention.rs` **inchangé et vert** ; plus une assertion explicite que U1 ne touche pas `try_acquire_dispatch_slot` |
| **V6a** | U2 tire si l'on réintroduit la clause ailleurs | mutation manuelle vérifiée rouge à la livraison |
| **V6b** | Mesure préalable au suivi D3 : `SELECT COUNT(*) FROM tasks WHERE trigger_type='callback' AND parent_task_id IS NULL;` | relevé à la revue, reporté dans le corps de PR |
| **V7** | `deferred_promotion_withheld` est absent quand aucun wrapper n'attend, présent quand un wrapper attend sur un créneau au cap | test d'intégration sur les deux branches |
| **V8** | `make lint` / `make test` verts | CI |

**Sonde post-déploiement, 7 jours, et ses trois haltes.**

```bash
grep deferred_promotion_withheld "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{dispatch_class, pending_wrappers, active, cap}'
```

- **Régime attendu : non vide pendant une contention, vide hors contention.** Ces
  lignes sont la mesure de la rétention, que rien ne donnait avant.
- **Halte 1 — un réveil `deferred_dispatch_promoted` toujours suivi d'un
  `deferred_dispatch_registered` sur le même parent.** Le TOCTOU de D4 mord plus
  souvent qu'estimé. **Ne pas durcir le prédicat** — il est déjà un sur-ensemble ;
  la cause est la fenêtre d'un tick, et le remède est un autre ticket.
- **Halte 2 — `deferred_promotion_withheld` vide alors que des wrappers stagnent.**
  La rétention ne vient pas du créneau : lire `deferred_dispatch_promotion_starved`
  (L2b, mika#2169) et `has_pending_operator_task_for_class` avant de toucher à
  quoi que ce soit ici.
- **Halte 3 — V6b rend un compte non nul.** La faille M4 est vivante en
  production : ouvrir le suivi D3 **avec ce compte**, et ne pas élargir la garde
  par réflexe.

---

## Definition of Done

- U1–U5 livrés.
- V1–V8 verts ; V6a vérifié rouge par mutation ; V6b relevé et reporté dans le
  corps de PR.
- Aucune valeur de réglage modifiée (R6), aucune variable d'environnement créée
  (R7) — vérifiable au diff.
- `tests/dispatcher_contention.rs` et `db/tests/dispatch_stamp_and_slots.rs`
  inchangés et verts.
- Le corps de PR porte : la rectification M1/M2 en tête, le refus D1 avec sa
  raison, la faille D3 nommée avec son compte V6b et son ticket de suivi, et la
  limite M5 (l'épisode du 2026-09-03 n'est pas attribué).

---

## Acceptance criteria

- **AC1** — Une seule source de vérité pour l'occupation d'un créneau. Soit le
  bail est **renouvelé** pendant toute la vie du dispatch (battement périodique
  par le détenteur), soit le chemin de reprise différée interroge **la même chose
  que la garde** (la présence d'un rappel actif) au lieu du bail.
- **AC2** — Un test couvre le cas mesuré : dispatch en vol depuis plus que le TTL
  du bail → la reprise différée ne se réveille **pas** en annonçant « slot
  freed ».
- **AC3** — Non-régression sur ce que le bail protège vraiment : deux tentatives
  de dispatch **concurrentes** dans la même fenêtre de 120 s continuent d'être
  arbitrées, une seule obtient le créneau.
- **AC4** — Si l'option « bail renouvelé » est retenue, le cas du détenteur mort
  est couvert : un pilote tué ne doit pas laisser un bail perpétuellement
  renouvelé. Le battement s'arrête avec le processus, donc le bail expire — mais
  le test doit le démontrer, pas le supposer.
- **AC5** — Le message `deferred dispatch slot freed` n'est émis que quand le
  créneau est réellement prenable. Un réveil suivi immédiatement d'un
  `global_dispatch_active` est un bug, et le test de l'AC2 doit le rendre
  impossible.

**Correspondance, avec ses réserves :**

| AC | Traitement | Réserve |
|---|---|---|
| AC1 | **Branche B**, par U1 + U2 | Branche A refusée avec sa raison (D1) |
| AC2 | V1, avec V2 en contrôle négatif | — |
| AC3 | V5 — non-régression, rien n'est touché | — |
| AC4 | **Sans objet** puisque la branche A est refusée | Le conditionnel de l'AC est honoré ; le refus est écrit au site de la constante (U4), pas seulement ici |
| AC5 | U1 rend le réveil stérile **par divergence de prédicat** structurellement impossible | **Le TOCTOU d'un tick n'est pas fermé** (D4) et ne peut pas l'être sans mettre un spawn dans une transaction SQLite. Il est nommé, et U3 le rend comptable |

---

## Fire-Disposition

**Le livrable détecteur de ce plan est U2**, un scan de source. Il **ne doit pas
tirer** à la livraison : U1 consomme le site unique partout, donc l'arbre est
conforme par construction. Sa disposition est donc *armé et silencieux*, et V6a
est ce qui prouve qu'il n'est pas silencieux par inertie — la mutation manuelle
le vérifie rouge avant la livraison, faute de quoi un scan inopérant se lirait
exactement comme un arbre propre (classe mika#2205).

**U3 n'est pas un détecteur** : `deferred_promotion_withheld` mesure une rétention
légitime, pas une violation. Son régime attendu est non vide, et aucune de ses
occurrences n'est une panne. Le confondre avec un signal d'alerte conduirait à le
muter — l'erreur que la doctrine mika#2131 borne.

**Aucune garde EndTurn, aucun filet moteur**, et le refus est mesuré : le défaut
est une divergence entre deux requêtes SQL, réparable à sa source. Un filet qui
rattraperait un réveil stérile *après coup* serait un second mécanisme répondant à
la même question — c'est-à-dire la classe que ce ticket existe pour fermer, une
couche plus haut.

---

## Suivi (hors périmètre, nommé)

- **La ligne de rappel orpheline (D3/M4).** `build_mika` et `deploy_mika`
  déclarent `"required": []` et sont `long_running`, donc peuvent écrire une ligne
  de rappel `parent_task_id = NULL` invisible à la garde — ce qui autorise un
  second dispatch `implement` concurrent. Remède probable : déclarer `task_id`
  requis dans les deux schémas. **Préalable : la mesure V6b.**
- **La fenêtre d'un tick entre promotion et dispatch (D4).** Conditionné à la
  Halte 1 de la sonde : si le couple promotion → refus est fréquent, il mérite son
  ticket, dont la piste n'est pas le prédicat mais l'ordonnancement.

---

## Références

- `241763a2` (2026-09-05) — mika#2169, seul commit ayant jamais introduit un
  lecteur de production du bail, deux jours après la mesure du ticket.
- `9171ef96` (2026-09-04) — le HEAD sur lequel M1 est établi.
- `db/tasks.rs` § *On the TTL, and why it is short* — « letting it lapse is the
  intended end of life ».
- mika#1948 — le bail comme arbitre de course (« the claim is the LAST gate »).
- mika#1163 — la première dérive de prédicat asymétrique sur ce même axe.
- mika#2158 — le doc-comment « Mirrors … » qui n'a suivi aucun élargissement ;
  même classe, même remède (lecteur unique + scan de source).
- mika#2205 / mika#2131 — un mécanisme silencieusement inactif se lit comme un
  mécanisme sans travail ; le `debug!` non collecté.
- mika#2321 — `db/tests/` et la prémisse de troncature que U2 doit éviter.
- mika#2160 — le cap N et `class_cap_reached`, hors périmètre.

---

## Revision history

- **2026-09-21** — version initiale (grooming autonome, mika#2162).
