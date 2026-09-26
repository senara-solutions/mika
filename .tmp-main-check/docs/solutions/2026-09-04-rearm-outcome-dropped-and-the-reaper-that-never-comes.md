---
module: mika-agent/task_engine
tags: [task-engine, deferred-dispatch, return-value-discarded, delegation-gap, terminal-state, detector-restraint, state-modeling]
problem_type: bug
category: error-handling
---

# Deux façons de rester muet : jeter une valeur de retour, et déléguer à un faucheur qui ne vous sélectionne pas

## Problème

Le 2026-09-04, trois wrappers de dispatch différé du parent `620ae345` (ticket
mika#2140) se sont terminés `delivered` — le mot le plus affirmatif du
vocabulaire des tâches — avec pour `result` la chaîne `deferred dispatch slot
freed`. Aucun des trois n'avait dispatché quoi que ce soit. Sur la fenêtre
élargie `00:00Z`–`07:00Z`, une seule tâche `long_running:run_claude_pilot`
existe, créée près de six heures après le refus initial.

Le mécanisme de **détection** fonctionnait pourtant parfaitement :
`rearm_consumed_deferred_wrapper` a été appelée trois fois, et
`deferred_dispatch_rearmed` a tiré deux fois. Ce qui manquait n'était ni la
détection ni la réparation, mais l'**enregistrement de l'effet** — et, sur une
seconde branche, la consommation d'une valeur de retour.

## Cause racine

### Défaut A — la valeur de retour jetée à l'endroit exact que sa doc interdit

`rearm_consumed_deferred_wrapper` (`task_engine/dispatcher.rs`) appelait
`rearm_deferred_callback` et terminait par `.await;` — pas par un `match`. Or
`RearmOutcome` existe précisément pour être discriminé, et sa propre
documentation le dit :

> `NotNow` and `Unrepairable` are both refusals, and collapsing them into one
> boolean is the bug this enum exists to prevent.

Ce site d'appel ne les collapsait pas en un booléen. Il les jetait tous les
deux — un mode d'échec que l'auteur de l'enum n'avait pas nommé parce qu'il est
en dessous de celui qu'il craignait.

### Défaut B — la délégation vers un faucheur dont le prédicat vous exclut

Quand le budget de réparation s'épuise, `rearm_deferred_callback` écrit :

```
"repair budget exhausted — leaving the task for the reaper to expire"
```

Le faucheur qu'il nomme est `find_orphaned_pending_issue_tasks`. Sa toute
première clause :

```sql
AND parent.status = 'pending'
```

`620ae345` était `blocked`. **Un parent `blocked` dont le budget s'épuise n'avait
donc aucun chemin qui écrive quoi que ce soit** : le dispatcher déléguait
l'expiration, le faucheur ne le voyait pas, personne n'écrivait. Le message de
log affirmait un transfert de responsabilité qui n'avait pas de destinataire.

Aggravant structurel : le mot `blocked` n'est écrit par aucune ligne du moteur
sur ce chemin. Il vient du LLM via `update_task_status`, prescrit par des
prompts d'escalade. **La couverture de la réparation dépendait donc du mot qu'un
LLM avait choisi** — la classe `feedback_prompt_enforcement_fragile`, un étage
sous celui que mika#2045 avait fermé.

### Défaut C — le re-armement vers un parent déjà terminal

Les trois tours stériles ont tous eu lieu **après** que le balayage phantom eut
terminalisé le parent à `02:04:01Z`. Or `run_claude_pilot` refuse une tâche non
active (`Task is not an active task`) et `failed` est terminal (`Cannot
transition from 'failed' to 'in_progress'`) : le re-armement était
**structurellement impossible avant d'être tenté**. Le budget se consommait dans
le vide, deux fois, sans que rien ne le dise.

## Solution

Quatre changements, dont deux portent la trace mesurée et deux la complètent.

1. **`mark_deferred_wrapper_noop`** — trois fins de vie, trois mots distincts.
   `delivered` = le tour a dispatché ; `expired` + raison = le tour a eu lieu et
   n'a rien dispatché ; `completed` sans suite = la promotion n'a jamais tiré.
   `expired` plutôt que `failed` parce que `failed` appartient au jeu que balaie
   `get_undelivered_callback_tasks` : le marquer ainsi remettrait le wrapper dans
   la file de livraison qu'il vient de quitter.

2. **Garde du parent terminal** — refuser le re-armement, l'écrire, et **ne pas
   consommer le budget**. Un parent illisible n'est *pas* traité comme terminal :
   épuiser un budget est une décision, et la prendre sur un hoquet de base de
   données la dépenserait pour rien.

3. **`match` sur `RearmOutcome`** — `Unrepairable` écrit l'échec *ici*, où la
   cause est connue, au lieu de le déléguer à un balayage qui ne viendra pas.

4. **Balayage discriminé, pas élargi** — pour les parents `blocked` dont le
   wrapper n'a jamais atteint la consommation. Le prédicat est
   `json_extract(result, '$.error') = 'global_dispatch_active'`, la seule valeur
   qu'écrit le refus de fente. Relâcher plutôt la clause du faucheur existant en
   `status IN ('pending','blocked')` aurait balayé les portes opérateur
   délibérées — `blocked` est aussi le mot d'un refus d'auto-merge et d'une
   escalade QA.

## Ce qu'il faut retenir

**Un message de log qui délègue est une affirmation vérifiable.** « leaving the
task for the reaper » nomme un destinataire ; il faut lire le prédicat du
destinataire et vérifier qu'il vous sélectionne. Ici les deux composants étaient
corrects isolément et le trou était entre eux — invisible à toute revue qui lit
un fichier à la fois.

**Un enum de retour ne se protège pas tout seul.** `RearmOutcome` documentait le
mode d'échec « collapser en booléen » et l'a effectivement empêché. Le mode
« jeter les deux » est plus grossier, moins imaginé, et n'était couvert par
rien. `#[must_use]` sur l'enum aurait transformé ce défaut en erreur de
compilation — un candidat de durcissement plus général que ce correctif.

**Ne pas conclure sur un `status` sans lire la transition qui l'a écrit.** Ce
plan s'est trompé **deux fois** avant d'atterrir, chaque fois en lisant un état
à un instant : d'abord un `completed` de transit lu comme terminal (le wrapper
fut livré 2 h 47 plus tard), puis un horodatage de fauchage erroné qui masquait
que les trois tours avaient eu lieu contre un parent **déjà mort**. C'est
`audit_events` — des transitions horodatées — qui a corrigé les deux, pas un
`SELECT status`.

**Restreindre un détecteur à la mesure est parfois le résultat, pas la
timidité.** La première rédaction posait un chien de garde à 300 s sur les
promotions non tirées. La re-mesure a montré que la seule latence observée est
de **2 h 47** : ce chien de garde aurait fauché vingt-deux wrappers vivants vers
`00:53Z` et échoué définitivement des parents servis à `03:35Z`. Une fenêtre
temporelle ne distingue pas la famine d'un vrai orphelin ; il faut un
discriminant d'état, et il n'en existait pas. Le livrable devient donc un
indicateur — avec un drapeau `agent_busy` qui rend le signal interprétable — et
l'action est reportée à un ticket qui la décidera sur la distribution mesurée.

**Une borne temporelle sur un compteur doit s'auto-installer.** Le compteur de
famine ne doit pas compter les rangées antérieures au correctif. Deux remèdes
ont été refusés : une date compilée en dur (le jour où `completed` devient
exclusif est celui où le code *tourne*, pas celui où le plan est écrit) et un
`env!()` au moment du build (deux compilations du même commit produiraient deux
binaires au comportement différent, et le binaire cesserait d'être une fonction
de sa source). Retenu : une ligne `INSERT OR IGNORE` dans `schema_meta` au
premier démarrage — pas de DDL, pas de bump de version de schéma.

**L'ordre de deux écritures faillibles est une propriété de sûreté.** Dans le
balayage filet, remettre le parent à `pending` **avant** de le ré-armer choisit
délibérément le mode d'échec : « `pending` sans wrapper » est exactement la
population que l'échelle mika#2045 possède et répare, tandis que « `blocked`
avec un wrapper vivant » boucle (le wrapper est promu, son tour appelle
`run_claude_pilot`, la garde refuse un parent `blocked`, un nouveau wrapper est
enregistré). Il faut échouer *dans* le filet du voisin, pas à côté.

**Dire ce que le correctif ne fait pas.** Les 80 minutes de silence viennent de
la famine de la file de livraison (un callback en tête de file, `completed` à
`22:03:24Z` et `delivered` seulement à `03:09:16Z` — cinq heures et demie), une
cause hors portée. Ce correctif rend la stérilité **enregistrée, bornée et
visible** ; il ne débouche pas la file. L'écrire explicitement évite de faire
passer pour réparée une boucle qui resterait muette.

## Voir aussi

- `docs/plans/2026-09-04-001-fix-2169-deferred-rearm-green-without-dispatch-plan.md`
- mika#2045 — l'échelle de réparation dont le prédicat `pending` laisse le trou
- mika#1124 / mika#1172 — la détection `deferred_dispatch_noop_completion` qui
  s'arrêtait à l'avertissement
- mika#1948 — le bail `dispatch_slot_leases`, que le balayage filet honore
- mika#2156 — le balayage phantom qui a terminalisé le parent de la trace
