# Plan — mika#2413 : sous dé-parquage multiple, le pilote groom déféré « expire » et le groom passe `failed`

- **Ticket :** senara-solutions/mika#2413
- **Type :** fix (substrat moteur — comptabilité d'un budget de réparation)
- **Date :** 2026-09-20
- **Branche :** `fix/2413/grooms-dispatch-s-en-lot-le-pilote-groom`

---

## Goal Capsule

Sous dé-parquage de N≥3 tickets, le troisième groom voit ses wrappers de
dispatch déféré passer `expired` les uns après les autres, puis son parent
passer `failed`. Ce plan établit le mécanisme réel — **il n'y a pas de TTL, et
rien n'expire par le temps** — et ferme la boucle auto-entretenue qui épuise en
trois tours un budget de réparation prévu pour une tout autre population.

**Ce que le plan rectifie du ticket, et c'est le premier livrable.** Le ticket
pose comme cause probable que « le TTL de la file des pilotes groom déférés est
plus court que le temps d'attente du siège arch ». Deux de ses trois remèdes
proposés (AC2 : « allonger/adapter le TTL déféré à la profondeur de file arch »)
portent donc sur un objet qui n'existe pas :

- `register_deferred_callback` et `rearm_deferred_callback`
  (`skills/executor.rs`) construisent tous deux leur `NewTask` avec
  `timeout_at: None`. Aucun wrapper déféré n'a jamais porté d'échéance.
- Le statut `expired` n'est pas un délai dépassé. C'est le **vocabulaire
  terminal de mika#2169** : `Database::mark_deferred_wrapper_noop`
  (`db/tasks.rs`) écrit `expired` pour dire *« le tour a eu lieu et n'a rien
  dispatché »*. Son doc-comment nomme les trois fins de vie — `delivered` = le
  tour a dispatché ; `expired` + raison = le tour a eu lieu et n'a rien
  dispatché ; `completed` sans successeur = la promotion n'a jamais tiré — et
  justifie le choix de `expired` plutôt que `failed` : `failed` remettrait le
  wrapper dans la file de livraison que `get_undelivered_callback_tasks`
  balaie.

La borne réellement franchie est `MAX_STUCK_REARMS = 2` (`skills/executor.rs`,
mika#2045) : un **budget de réparation par parent**, pas un délai. Le parent
meurt par `RearmOutcome::Unrepairable` →
`TaskDispatcher::rearm_consumed_deferred_wrapper` →
`update_task_failed(parent, "re-armement différé épuisé après 2 tentatives …")`.

### L'arithmétique de la preuve dure, recomposée à la seconde

La preuve du ticket valide ce mécanisme et **réfute** le sien :

| wrapper | instant | lecture |
|---|---|---|
| `5e935a39` | 19:03:54 | 1ʳᵉ consommation stérile → re-arm, `stuck_rearm_count = 1` |
| `e3db2a79` | 19:18:45 | 2ᵉ → re-arm, `stuck_rearm_count = 2` |
| `228efb7b` | 19:19:45 | 3ᵉ → `count >= MAX_STUCK_REARMS` → `Unrepairable` |
| `e7c4e9ad` (parent) | **19:19:45** | `failed`, **la même seconde** |
| `aa0a5d9d` | 19:22:22 | parent déjà terminal → `deferred_wrapper_orphaned_by_terminal_parent` (mika#2169 L2a), **budget non dépensé** |

La coïncidence à la seconde entre la 3ᵉ expiration et la mort du parent est la
signature exacte de la branche `Unrepairable`, qui écrit `record_wrapper_noop`
puis `update_task_failed` dans la même fonction. Aucun délai n'aurait produit
cette simultanéité. Trois consommations contre un budget de deux : le seuil est
franchi à la troisième, exactement comme le code le prescrit.

Le ticket compte « 4 tentatives gaspillées ». La quatrième est postérieure au
décès et relève d'une branche qui ne dépense rien : le gaspillage réel est de
**trois**, et il est intégralement imputable à la comptabilité du budget.

### La cause racine, en une phrase

`MAX_STUCK_REARMS` borne l'hypothèse *« les tours de ce parent ne dispatcheront
jamais »* (mika#2045). Il est dépensé à l'identique par quatre causes de
natures différentes — `noop_completion`, `silent_turn_error`,
`stuck_pending_reaper`, `stale_blocked_dispatch` — dont **la plus fréquente en
régime de dé-parquage n'est pas un défaut mais le fonctionnement nominal de la
file** : un tour qui a correctement appelé son outil, s'est fait refuser en
`global_dispatch_active`, et s'est remis en file.

Ce raisonnement est déjà écrit dans le dépôt, appliqué à une autre condition
transitoire, dans le doc-comment de `RearmOutcome` (`skills/executor.rs`) :

> *« a full deferred-callback queue is not that. It is a transient condition
> that clears on its own, so a task refused for capacity must be left alone and
> retried, not destroyed with repair budget still on it. »*

La contention de slot est de la **même famille** que la saturation de file —
elle est même plus clairement transitoire, puisque le slot se libère
nécessairement. Le raisonnement existait, il n'avait pas été étendu.

### La boucle auto-entretenue, qui est la vraie découverte

La contention n'explique que le **premier** re-arm. Ce qui condamne le parent,
c'est que le re-arm fabrique lui-même la condition qui rend le tour suivant
stérile. Trois mécanismes corrects isolément se composent en épuisement
garanti :

1. **`rearm_deferred_callback` crée un wrapper `pending`** pour que le parent
   reste représenté (mika#2045).
2. **`execute_long_running` court-circuite en `already_deferred`** dès qu'un
   wrapper `pending` existe pour ce parent (intercept mika#1205), et cet
   intercept est placé **avant** `validate_dispatch_readiness`. Donc le tour
   suivant ne tente **aucun** dispatch — y compris quand le slot est entre-temps
   redevenu libre.
3. **R9 (`deferred_dispatch_noop_completion`, mika#1124/#2045)** lit
   `has_non_deferred_active_callback_child(parent)`, dont le SQL porte
   `label NOT LIKE '%:deferred'`. Un wrapper fraîchement créé est invisible à ce
   prédicat : le tour qui s'est correctement remis en file est donc **compté
   comme un no-op**, et re-armé une fois de plus.

Une fois le premier re-arm posé, les deux tours suivants sont stériles **par
construction** et le parent est mort. C'est pourquoi le symptôme apparaît sous
dé-parquage (qui fournit la contention initiale) sans que la contention en soit
la cause d'épuisement.

Deux confusions se composent : **R9 confond « n'a rien fait » et « s'est
correctement remis en file »**, et **le budget confond « ne dispatchera jamais »
et « attend son tour »**.

### Ce que ce plan ne change pas — réponse à la consigne opérateur

Le commentaire du 20/09 demande une halte si le groom révèle un changement de
politique de dispatch. **Il n'y en a pas.** Le cap `groom` reste à 1
(`max_concurrent_for_class`, branche `_ => 1`), la sérialisation sur le siège
arch est inchangée, aucun TTL n'est introduit, aucun retry n'est ajouté (AC4),
aucun paramètre de concurrence n'est touché. Ce qui change est la **comptabilité
d'un budget de réparation** : quelles causes le dépensent, et quand le parent
est déjà représenté.

---

## Product Contract

### Symptôme observable aujourd'hui

Sous dé-parquage de N≥3 tickets vers `ready` :

- le 3ᵉ groom produit 3 à 4 rows `tasks` en `status = 'expired'` sous le même
  parent, en ~20 min ;
- son parent passe `failed` avec `result` = « re-armement différé épuisé après
  2 tentatives (cause=…) — aucun dispatch produit » ;
- l'auto-guérison (`stuck_ready_reconcile`, mika#1824) re-dispatche plus tard et
  le ticket finit GROOMED, au prix d'un cycle complet et d'un `failed`
  transitoire visible dans toutes les surfaces opérateur.

### Comportement visé

Sous dé-parquage de N≥3, aucun groom ne passe `failed` **du seul fait** que son
wrapper déféré a été consommé pendant qu'un autre groom tenait le siège. Le
parent attend son tour, sans dépenser de budget, et le budget mika#2045
continue de borner exactement la population pour laquelle il a été écrit : un
parent dont les tours n'appellent réellement jamais l'outil.

### Population et hors-périmètre

| | dans le périmètre | hors périmètre |
|---|---|---|
| Classe `groom` sous contention | ✅ | |
| Classe `implement` sous contention | ✅ (même code, même boucle) | |
| Cap de concurrence (`MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT`, cap groom) | | ❌ inchangé |
| Sérialisation sur le siège arch (`_arch_ask`, `agent_lock`) | | ❌ inchangée |
| Cause du `body read failed mid-stream` (transport OpenRouter) | | ❌ mika#2391 / mika#2280 |
| Livraison de callback (quarantaine, backoff) | | ❌ mika#2179 |
| `stuck_ready_reconcile` (l'auto-guérison aval) | | ❌ inchangé |

---

## Planning Contract

### D1 — Le discriminant est l'état du parent, jamais la cause

Le réflexe serait d'énumérer les causes et de décider lesquelles dépensent le
budget. Il est **refusé**, pour deux raisons.

D'abord une liste de causes est un format de fil implicite qui dérive : quatre
causes existent aujourd'hui (`noop_completion`, `silent_turn_error`,
`stuck_pending_reaper`, `stale_blocked_dispatch`), une cinquième s'ajouterait
sans que personne ne rouvre cette décision, et elle hériterait du mauvais
défaut.

Ensuite et surtout, la cause n'est pas le bon prédicat. La question que le
re-arm existe pour poser est littéralement : *« le parent a-t-il encore quelque
chose qui le représente dans la file ? »*. Cette question a une réponse exacte,
observable et déjà implémentée : `has_live_deferred_wrapper_child`
(`db/tasks.rs`, mika#2181). Si un wrapper vivant existe, **il n'y a rien à
réparer** — quelle que soit la raison pour laquelle le tour précédent n'a rien
dispatché.

**Décision : `rearm_deferred_callback` rend `RearmOutcome::NotNow` — sans créer
de wrapper et sans incrémenter le compteur — quand le parent porte déjà un
wrapper déféré vivant.** C'est exactement la sémantique documentée de `NotNow` :
*« Refused for a condition that clears by itself — try again next tick »*.

Cette décision ferme les deux confusions d'un seul prédicat :

- plus de **double création** : le wrapper posé par `register_deferred_callback`
  sur le refus `global_dispatch_active` suffit, le re-arm n'en ajoute pas un
  second ;
- plus de **boucle auto-entretenue** : sans wrapper surnuméraire, l'intercept
  mika#1205 cesse de court-circuiter systématiquement le tour suivant, et un
  tour qui trouve le slot libre dispatche réellement.

### D2 — La garde va dans `rearm_deferred_callback`, pas chez ses appelants

Quatre sites appellent la chaîne de re-arm (`TaskDispatcher::rearm_consumed_deferred_wrapper`
× 2, `TaskEngine` stuck-pending reaper, `TaskEngine` stale-blocked L3b). Poser
la garde chez les appelants la ferait diverger en quatre exemplaires — la classe
que `grooming_marker` (mika#2158) a dû fermer une fois dans ce dépôt, et que le
doc-comment de `rearm_deferred_callback` anticipe déjà : *« `rearm_deferred_callback`
owns the "did this turn actually dispatch?" guard, so both this call site and
the reaper's inherit it »*.

La garde va donc **dans la fonction**, à côté de la garde sœur
`has_non_deferred_active_callback_child` qu'elle prolonge, et les quatre
appelants en héritent sans rien changer.

**Ordre imposé :** la nouvelle garde se place **après**
`has_non_deferred_active_callback_child` (le tour a vraiment dispatché : rien à
réparer) et **avant** la lecture de `get_stuck_rearm_count` — sinon un parent
déjà représenté continuerait de faire lire, puis dépenser, un budget qu'il n'a
aucune raison de toucher.

### D3 — Fail-safe : un signal illisible ne dispense jamais de réparer

`has_live_deferred_wrapper_child` peut échouer (erreur DB). La règle maison
— *un signal qu'on ne peut pas lire n'est jamais un terme satisfait* — s'applique
ici avec un sens précis : **une lecture impossible ne doit pas être lue comme
« le parent est représenté »**, sans quoi un hoquet de base condamnerait un
parent au silence permanent (jamais réparé, jamais expiré, plus rien dans la
file).

**Sur erreur, on retombe sur le chemin nominal** (on répare, on dépense). Le
coût est borné et connu : au pire, la boucle d'avant ce correctif, que trois
autres filets couvrent déjà (reaper stuck-pending, L3b, `stuck_ready_reconcile`).
L'asymétrie penche du bon côté : un faux « je répare » coûte un point de budget,
un faux « c'est représenté » coûte un parent qui n'est plus représenté par rien.

### D4 — Le compteur est monotone à vie, et c'est un second défaut

`increment_stuck_rearm_count` est le **seul** écrivain de
`metadata.stuck_rearm_count` : il n'existe aucun `reset_*`. Le compteur ne
redescend jamais, même après un dispatch réel réussi.

Conséquence mesurable et aggravante, qui n'est pas dans le ticket : le parent
d'un groom **devient** le parent de l'implémentation (mika#1614 task-reuse,
`update_task_dispatch_class` fait basculer `groom` → `implement` sur la même
row). Deux contentions subies au grooming condamnent donc l'implémentation
avant qu'elle n'ait commencé.

C'est le miroir exact de mika#2158 (*« un compteur remis à zéro par l'action
qu'il compte ne borne rien »*) : ici, **un compteur que le succès ne remet
jamais à zéro finit par borner autre chose que ce qu'il mesure**.

**Décision : remettre le compteur à zéro sur la preuve qu'un dispatch réel a eu
lieu** — c'est-à-dire à la création d'un enfant callback **non-deferred** pour ce
parent. Le discriminant est sûr parce que c'est précisément l'événement dont
l'absence est bornée : si le parent dispatche vraiment, l'hypothèse « ses tours
ne dispatchent jamais » est réfutée par les faits.

**Ce qui distingue ce reset de celui que mika#2158 a dû retirer** : celui-là
était déclenché par `in_flight_self_dev`, c'est-à-dire par *le fait d'avoir
commencé* — l'action même que le compteur comptait, d'où un compteur inatteignable
(31 re-drives affichant 1). Ici le déclencheur est *le fait d'avoir abouti à un
vrai dispatch*, qui est justement ce que le compteur ne compte pas. La
distinction doit être écrite au site du reset, sinon elle sera relue comme la
régression.

Livré en unité séparée (U3) et **indépendante de U1/U2** : U1 seul ferme le
symptôme du ticket ; U3 ferme la dérive inter-phases que le grooming a trouvée
en chemin.

### D5 — Aucun paramètre nouveau, aucune valeur déplacée

`MAX_STUCK_REARMS` reste à 2. `MAX_PENDING_DEFERRED_CALLBACKS` reste à 10. Le
cap groom reste à 1. Aucune variable d'environnement n'est créée.

Le réflexe « remonter `MAX_STUCK_REARMS` » est **refusé** : il ne ferait
qu'allonger la boucle décrite ci-dessus, en la rendant plus lente à observer, et
il affaiblirait la borne mika#2045 pour la population qu'elle borne
correctement. Le défaut n'est pas que le budget soit trop petit, c'est qu'il
soit dépensé par ce qu'il ne mesure pas.

### Alternatives examinées et refusées

| piste | pourquoi refusée |
|---|---|
| Allonger un TTL déféré | **Objet inexistant** : `timeout_at: None` aux deux sites de création. Le remède n'a pas de site d'application. |
| Adapter un TTL à la profondeur de file arch | Même raison, plus une seconde : la profondeur de file arch n'est pas observable depuis le moteur (la sérialisation arch a lieu *dans* le pilote, en aval du dispatch). |
| Sérialiser la création des déférés à la cadence du siège (AC2, 3ᵉ voie) | Ferait porter au moteur une connaissance de la latence arch qu'il n'a pas, pour un défaut dont la contention n'est que le déclencheur. La file déférée fait déjà son travail : c'est sa comptabilité qui est fausse. |
| Un retry supplémentaire | Explicitement exclu par AC4, et à raison : l'auto-guérison existe déjà. Ce plan **retire** des consommations indues, il n'ajoute pas de tentatives. |
| Remonter `MAX_STUCK_REARMS` | Voir D5. |
| Exclure les `:deferred` du prédicat R9 | Traiterait le symptôme au mauvais étage : R9 est une *détection*, et sa définition (« le parent n'a pas d'enfant réel actif ») est correcte. Ce qui est faux, c'est la conclusion qu'on en tire dans le re-arm. |

---

## Implementation Units

### U1 — La garde « parent déjà représenté » dans `rearm_deferred_callback`

**Fichier :** `crates/mika-agent/src/skills/executor.rs`

Dans `rearm_deferred_callback`, entre la garde
`has_non_deferred_active_callback_child` et la lecture de
`get_stuck_rearm_count`, insérer une garde lisant
`has_live_deferred_wrapper_child(parent_task_id)` :

- `Ok(true)` → `RearmOutcome::NotNow`, **aucun wrapper créé, aucun compteur
  incrémenté**, plus un `info!` nommé (voir U4) ;
- `Ok(false)` → on continue, chemin nominal inchangé ;
- `Err(_)` → `warn!` + on continue sur le chemin nominal (D3 — fail-safe *vers
  la réparation*, l'inverse de la garde qui la précède, et le commentaire au
  site doit dire pourquoi les deux divergent).

Le doc-comment de la fonction doit énoncer le nouvel invariant en une phrase :
*un parent qui porte déjà un wrapper vivant est représenté ; il n'y a rien à
réparer, et donc rien à dépenser.*

**Vérifier avant d'écrire :** la fenêtre de liveness de
`has_live_deferred_wrapper_child` (mika#2181, `pending` **ou** `completed` de
moins de `MIKA_PROMOTED_WRAPPER_LIVENESS_SECS`) est conçue pour le reaper
stuck-pending. Elle est *plus large* que le strict `pending`, ce qui est la
direction sûre ici (un wrapper promu mais pas encore consommé représente bien le
parent). Si l'implémenteur constate qu'elle retient le wrapper **en cours de
consommation** — celui-là même dont on traite la stérilité — la garde doit
l'exclure explicitement par son `id`, sinon aucun re-arm ne serait jamais
possible. **C'est le point de rupture le plus probable de cette unité : le
tester d'abord.**

### U2 — Le conséquence sur `record_wrapper_noop`, à trancher au code

`rearm_consumed_deferred_wrapper` (`task_engine/dispatcher.rs`) n'écrit
aujourd'hui de record terminal sur le wrapper que dans les branches `Rearmed` et
`Unrepairable` ; `NotNow` ne marque rien et laisse le wrapper `completed`.

Avec U1, la branche `NotNow` devient le **chemin nominal sous contention**, et
elle laisserait un wrapper `completed` sans successeur — précisément l'état que
l'indicateur L2b `count_promoted_undelivered_wrappers` (mika#2169) compte comme
« promotion starvation ».

**Décision requise de l'implémenteur, avec les deux branches et leur coût :**

- **écrire le record `expired`** avec une raison distincte (« parent déjà
  représenté ») : le wrapper quitte la file de livraison, l'exclusivité dont
  dépend L2b est préservée, et le mot `expired` garde le sens que mika#2169 lui
  a donné. **Recommandé.**
- **ne rien écrire** : le wrapper reste `completed`, L2b le compte, et
  `deferred_dispatch_promotion_starved` se met à firer sur un régime sain — un
  indicateur qui fire en régime nominal est un indicateur qu'on coupe.

Dans les deux cas, ne **pas** toucher aux branches `Rearmed` / `Unrepairable`.

### U3 — Remise à zéro du compteur sur dispatch réel (indépendante)

**Fichiers :** `crates/mika-agent/src/db/tasks.rs`,
`crates/mika-agent/src/async_db.rs`, + le site de création de l'enfant callback
non-deferred.

Ajouter `reset_stuck_rearm_count(task_id)` (symétrique de
`increment_stuck_rearm_count`, même tolérance `json_valid`), et l'appeler quand
un enfant callback **non-deferred** est créé pour le parent — c'est-à-dire quand
un dispatch réel a effectivement été spawné.

Écrire au site, en toutes lettres, la distinction d'avec mika#2158 (D4) : le
déclencheur est *avoir abouti à un vrai dispatch*, pas *avoir commencé*. Sans
cette phrase, le prochain lecteur retirera le reset en croyant fermer mika#2158.

**Cette unité est optionnelle au sens de la fermeture du ticket** : U1 (+U2)
ferme le symptôme mesuré. U3 ferme la dérive inter-phases groom→implement. Si
U3 est reportée, elle doit être ouverte en ticket de suivi avec le paragraphe D4
recopié — pas simplement omise.

### U4 — Surfaces opérateur

Aucun compteur nouveau, aucune table. Deux ajouts et une précision :

1. `deferred_rearm_skipped_parent_represented` — INFO, champs
   `parent_task_id`, `task_id` (le wrapper consommé), `live_wrapper_id`,
   `cause`, `dispatch_class`. **Régime attendu : non vide sous dé-parquage
   multiple, silencieux hors contention.** C'est la mesure directe que la garde
   mord — et sans elle, un re-arm supprimé se lirait exactement comme un re-arm
   qui n'a jamais eu lieu d'être (classe mika#2205).
2. `deferred_dispatch_rearm_budget_exhausted` porte déjà `cause`. Vérifier
   qu'elle est bien propagée jusqu'à l'audit
   `deferred_dispatch_unrepairable_parent_failed` : après U1, **la distribution
   des causes d'épuisement est le chiffre qui dit si le fix a pris**. Si
   `noop_completion` domine toujours après déploiement, c'est que la garde ne
   mord pas.
3. SQL de lecture, à inscrire dans le corps de PR :
   `SELECT after_value, count(*) FROM audit_events WHERE tool_name = 'deferred_dispatch_unrepairable_parent_failed' GROUP BY 1;`

Si U3 est livrée, ajouter `stuck_rearm_count_reset` (INFO) — un compteur remis à
zéro sans trace est exactement la chose qu'on ne peut pas auditer après coup.

---

## Verification Contract

### V1 — Contrôle positif : trois grooms concurrents (AC3)

Un test qui pose trois parents `groom` en concurrence sur un slot de cap 1, et
vérifie que le troisième :

- ne passe **jamais** `failed` ;
- voit son `stuck_rearm_count` rester à 0 tant qu'il attend ;
- porte à tout instant **exactement un** wrapper déféré vivant (pas deux) ;
- dispatche réellement quand le slot se libère.

### V2 — Contrôle négatif (obligatoire) : le test rougit sans le fix

Le même scénario, avec la garde U1 neutralisée, doit reproduire la trace du
ticket : trois wrappers `expired` et le parent `failed`. **Sans ce contrôle, V1
ne prouve rien** — il passerait aussi contre un prédicat qui ne lit rien.

### V3 — Contrôle de non-régression mika#2045 : le budget borne toujours

Un parent dont les tours sont **réellement** stériles — aucun wrapper vivant,
aucun appel d'outil — doit toujours mourir après `MAX_STUCK_REARMS`
réparations. C'est la population pour laquelle le budget a été écrit, et U1 ne
doit pas l'élargir d'un pouce.

### V4 — Contrôle de non-régression mika#1124 : pas de cascade de no-op

Vérifier qu'aucun chemin ajouté ne promeut en chaîne : la garde U1 **supprime**
une création, elle n'en déclenche aucune. Le garde-fou anti-cascade de
`dispatcher.rs` (`task.label != DEFERRED_DISPATCH_LABEL`) reste intact.

### V5 — Si U3 est livrée : le reset ne rejoue pas mika#2158

Un parent qui *commence* un dispatch (wrapper promu, tour en cours) ne doit
**pas** voir son compteur remis à zéro. Seul l'enfant callback non-deferred
créé le remet. Ce test est ce qui distingue le reset de sa régression.

### V6 — Vérification manuelle post-déploiement, avec sa halte

Au prochain dé-parquage réel de N≥3, lire dans `$MIKA_SPIRIT_LOG_FILE` :

```bash
grep deferred_rearm_skipped_parent_represented "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{parent_task_id, cause, dispatch_class}'
grep deferred_dispatch_unrepairable_parent_failed "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{parent_task_id, cause}'
```

Attendu : le premier non vide, le second vide pour la classe `groom`.

**Halte 1** — le second est non vide avec `cause = "noop_completion"` alors que
le premier est vide : la garde ne mord pas. Ne pas remonter `MAX_STUCK_REARMS` ;
vérifier d'abord la fenêtre de liveness (point de rupture nommé en U1).

**Halte 2** — le premier est non vide et le parent meurt quand même : le budget
est dépensé par une cinquième voie que ce plan n'a pas inventoriée. Lire la
`cause` et établir le site avant de toucher au prédicat.

**Halte 3** — `deferred_dispatch_promotion_starved` (L2b) se met à firer en
régime sain : U2 a été tranchée dans la mauvaise branche. Poser le record
terminal plutôt que couper l'indicateur.

---

## Definition of Done

- La garde U1 est en place dans `rearm_deferred_callback`, avec son invariant
  écrit au doc-comment et sa divergence de fail-safe justifiée au site.
- U2 est tranchée explicitement (record terminal écrit, ou non-écriture
  argumentée), et l'exclusivité dont dépend L2b est préservée dans les deux cas.
- V1 à V4 passent ; V2 est vérifié rouge sans le fix (et le PR le dit).
- Si U3 est livrée : V5 passe et la distinction d'avec mika#2158 est écrite au
  site. Sinon : ticket de suivi ouvert avec D4 recopié.
- Les deux surfaces U4 sont émises et documentées dans le corps de PR avec leur
  régime attendu.
- `cargo test`, `cargo clippy`, `cargo fmt` propres.
- Aucun paramètre de concurrence, aucun TTL, aucun retry n'a été ajouté ou
  déplacé — et le corps de PR l'affirme, en réponse à la consigne opérateur du
  20/09.

## Acceptance criteria

Transcrits du corps du ticket, avec la réponse que le grooming leur apporte.

1. **Localiser le TTL des pilotes groom déférés (`run_claude_pilot:deferred`) et
   la logique de sérialisation sur le siège arch (mika-arch k3, une passe à la
   fois).**
   → **Il n'existe aucun TTL** : `timeout_at: None` dans
   `register_deferred_callback` et `rearm_deferred_callback`. Le statut
   `expired` est le vocabulaire terminal de `mark_deferred_wrapper_noop`
   (mika#2169), pas un délai. La borne franchie est `MAX_STUCK_REARMS = 2`
   (mika#2045). La sérialisation des grooms vient du cap `groom = 1`
   (`max_concurrent_for_class`, branche `_ => 1`, non configurable) ; le siège
   arch sérialise en aval, dans le pilote, et n'intervient pas dans la décision
   de dispatch.

2. **Sous dé-parquage de N≥3 tickets, aucun groom ne doit passer `failed`
   uniquement parce que son pilote déféré a expiré en file : soit allonger le
   TTL, soit re-queue au lieu de failed, soit sérialiser la création des
   déférés.**
   → Satisfait par la **deuxième voie** (« re-queue au lieu de failed »), qui
   est déjà le mécanisme en place : le re-arm. Ce plan cesse de lui faire
   dépenser un budget quand le parent est déjà représenté (U1) et supprime la
   double création de wrappers (U1). Les deux autres voies sont refusées avec
   leur raison (Planning Contract § Alternatives).

3. **Test négatif : simuler 3+ grooms concurrents, vérifier que le 3ᵉ n'expire
   pas avant d'obtenir le siège (ou est re-queué proprement, pas failed).**
   → V1 (contrôle positif) + V2 (contrôle négatif : rouge sans le fix) + V3
   (non-régression mika#2045).

4. **Ne pas masquer par un simple retry : l'auto-guérison existe déjà
   (re-dispatch) ; le but est d'éviter les 4 tentatives gaspillées + le failed
   transitoire.**
   → Aucun retry n'est ajouté. Le correctif **retire** des consommations de
   budget indues et une création de wrapper surnuméraire. Précision au passage :
   les tentatives réellement gaspillées sont **trois**, la quatrième étant
   postérieure au décès du parent et prise en charge par une branche qui ne
   dépense rien (`deferred_wrapper_orphaned_by_terminal_parent`, mika#2169 L2a).

## Sources

Citations **par symbole** (doctrine mika#2397 : un fichier faux avec un symbole
juste se répare par un `grep` ; un numéro de ligne faux rend du code plausible
et sans rapport).

| Symbole | Fichier (2026-09-20) | Rôle dans le diagnostic |
|---|---|---|
| `MAX_STUCK_REARMS` | `skills/executor.rs` | La borne réellement franchie (= 2) |
| `rearm_deferred_callback` | `skills/executor.rs` | Site de la garde U1 |
| `RearmOutcome` | `skills/executor.rs` | Le doc-comment qui raisonne déjà juste, sur une autre condition transitoire |
| `register_deferred_callback` | `skills/executor.rs` | `timeout_at: None` — absence de TTL (1/2) |
| `validate_dispatch_readiness` | `skills/executor.rs` | Branche `global_dispatch_active` → création du 1ᵉʳ wrapper |
| `execute_long_running` | `skills/executor.rs` | Intercept `already_deferred` (mika#1205), placé **avant** la readiness |
| `max_concurrent_for_class` | `skills/executor.rs` | Cap `groom` = 1, non configurable |
| `TaskDispatcher::rearm_consumed_deferred_wrapper` | `task_engine/dispatcher.rs` | Consomme l'issue ; écrit `update_task_failed` sur `Unrepairable` |
| `TaskDispatcher::record_wrapper_noop` | `task_engine/dispatcher.rs` | Site de la décision U2 |
| `TaskEngine::promote_pending_deferred_if_idle` | `task_engine/engine.rs` | Promotion FIFO par classe, vérifie le slot |
| `Database::mark_deferred_wrapper_noop` | `db/tasks.rs` | Écrit `expired` = « le tour n'a rien dispatché » (mika#2169) |
| `Database::has_non_deferred_active_callback_child` | `db/tasks.rs` | `label NOT LIKE '%:deferred'` — le faux positif de R9 |
| `Database::has_live_deferred_wrapper_child` | `db/tasks.rs` | Le prédicat de la garde U1 (mika#2181) |
| `Database::increment_stuck_rearm_count` | `db/tasks.rs` | Seul écrivain du compteur — aucun reset (D4) |
| `Database::count_promoted_undelivered_wrappers` | `db/tasks.rs` | Indicateur L2b dont U2 doit préserver l'exclusivité |
| `Database::update_task_dispatch_class` | `db/tasks.rs` | Le task-reuse groom→implement qui transporte le compteur (D4) |

Tickets de lignée : mika#1011 (file déférée), mika#1070 / mika#1124 / mika#1163
/ mika#1175 (promotion, anti-cascade, prédicats de slot), mika#1205 (intercept
idempotent), mika#1614 (task-reuse), mika#2045 (budget de réparation), mika#2158
(la leçon sur les compteurs), mika#2160 (cap configurable), mika#2169
(vocabulaire terminal des wrappers), mika#2181 (fenêtre de liveness).
