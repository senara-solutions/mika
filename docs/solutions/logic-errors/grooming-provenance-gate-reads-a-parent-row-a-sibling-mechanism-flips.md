---
title: "Une porte de sûreté qui ne marche qu'avec son bypass activé n'a jamais tourné"
date: 2026-09-11
category: logic-errors
module: skills/executor
problem_type: logic_error
component: tooling
symptoms:
  - "Every autonomous dev-pilot dispatch rejected \"dispatch_grooming_not_verified\" once MIKA_DISPATCH_BYPASS_GROOMING_CHECK was removed from the runtime environment"
  - "has_completed_groom_for_issue required a row with dispatch_class='groom', a terminal status, and reference_url = \"<issue_url>?phase=groom\" that no production groom producer ever wrote"
  - "The engine's own task-reuse auto-fire (try_dispatch_pilot_after_groom_success) flipped the parent row's dispatch_class from groom to implement before the parent ever reached a terminal status, erasing the only row the gate could read"
  - "validate_dispatch_readiness's Err(e) arm was fail-open (warn + allow), so a degraded gate silently let dispatch through instead of refusing it"
  - "The gap was invisible for weeks because an emergency bypass flag masked every rejection"
root_cause: logic_error
resolution_type: code_fix
severity: critical
tags:
  - dispatch-gate
  - grooming-marker
  - autonomous-loop
  - validate-dispatch-readiness
  - callback-proof
  - fail-closed
  - task-reuse
  - loop-substrate
related_components:
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/task_engine/dispatcher.rs
  - crates/mika-agent/src/skills/executor.rs
  - crates/mika-agent/src/task_state/tasks.rs
  - skills/bundled/_shared/dispatch-lib.sh
---
# Une porte de sûreté qui ne marche qu'avec son bypass activé n'a jamais tourné

**Incident fondateur : mika#2287, 2026-09-11.** Le jour où
`MIKA_DISPATCH_BYPASS_GROOMING_CHECK` a quitté l'environnement du spirit, chaque
dispatch autonome `dev-pilot` a été refusé `dispatch_grooming_not_verified`.
Aucun ticket n'avait cessé d'être groomé ; la porte ne pouvait simplement plus
voir aucune preuve — et elle ne l'avait jamais pu.

## Problème

La porte de provenance de grooming (#1620, `has_completed_groom_for_issue`,
`crates/mika-agent/src/db.rs:9541`) exigeait une ligne `tasks` avec
`dispatch_class='groom'`, un statut terminal, et un `reference_url` égal à
`<issue_url>?phase=groom`. Aucun producteur de grooming en production ne peut
laisser une telle ligne :

- le handler structurel `ready`-label (mika#1572) pré-crée le parent groom avec
  l'URL **nue** de l'issue (`crates/mika-agent/src/server/ready_label_handler.rs:438`,
  `reference_url: Some(issue_url.clone())`) ;
- l'auto-fire côté moteur (mika#1614) **bascule** le `dispatch_class` du parent
  groom→implement à l'étape 5c (`crates/mika-agent/src/task_engine/dispatcher.rs:2974-2978`,
  `update_task_dispatch_class(&parent_id, "implement")`) **avant** que le parent
  atteigne un statut terminal — c'est le patron de réutilisation de tâche de
  mika#996, qui évite une collision sur l'index unique `reference_url` ;
- l'enfant callback porte `reference_url: None` (`build_callback_task`,
  `crates/mika-agent/src/skills/executor.rs:2894`).

mika#1614 et mika#1620 ont atterri le **même jour** (2026-06-28). Chacun était
correct seul ; chacun supposait le monde de l'autre. La porte lisait la forme
`?phase=groom` que l'ancien chemin LLM écrivait ; la réutilisation de tâche a
retiré la seule ligne que la porte pouvait voir. Le bypass d'urgence, posé pour
une autre raison, a masqué le mort pendant des semaines : **la seule route qui
dispatchait était le drapeau.**

## Symptômes

- Tout dispatch `dev-pilot` autonome refusé `dispatch_grooming_not_verified`,
  bypass retiré — y compris sur des tickets groomés par la boucle elle-même.
- Le texte de `recovery` du refus recommandait de reposer `ready` : le handler
  redispatchait dev-pilot, qui retombait sur le même refus. Route morte.
- Zéro erreur, zéro log d'échec tant que le bypass était posé ; un `WARN`
  « bypassed via env var » à chaque dispatch, lu comme du bruit.
- Le fix ne pouvait pas passer par la boucle qu'il répare (œuf-et-poule) :
  implémenté hors-bwrap par l'orchestrateur, revue decision-core, bypass
  maintenu retiré.

## Ce qui n'a pas marché

**Réparer le marqueur du corps** (session history) — première hypothèse du
diagnostic : les grooms hors-bwrap de `/mika-groom-ticket` écrivaient
`first-pass (READY)` et non le littéral `second-pass (GROOMED)` que
`check_grooming_markers` attend ; un patch de marqueur a été rédigé pour #2286
et jamais appliqué. Futile : la porte #1620 ne lit pas le corps, elle exige une
ligne structurelle.

**Passer par la voie moteur** (session history) — seconde hypothèse : router le
grooming par le chemin natif de mika#1614 en supposant qu'il satisferait la
porte. Vérifié en code, il échoue pareil — c'est même lui qui bascule la ligne.
La conclusion « les deux mécanismes sont incompatibles par construction » n'a
été fichée (mika#2287) qu'après lecture du prédicat dans `db.rs`, pas depuis la
mémoire.

**Réparer les producteurs** (le handler émettrait une ligne-preuve
`?phase=groom`) — rejeté : contredit la réutilisation mono-`task_id` de
mika#1614 et oblige à réconcilier trois producteurs qui n'ont aucune raison de
partager une forme.

**Lire la ligne parent** — impossible par construction : elle est basculée avant
d'être terminale. Aucune requête sur le parent, si tolérante soit-elle, ne
survit à l'étape 5c.

**Remettre le bypass** comme remède — refusé et maintenu refusé. Un drapeau qui
masque une porte cassée pendant des semaines *est* l'incident, pas sa
solution.

## Solution

Sur la branche `bug/2287/dispatch-gate-1620-preuve-groom`, PR en attente.
Direction (b), ratifiée Vincent + Prime : **réparer la porte, pas les
producteurs**, sous neuf conditions dont : lecture seule, fail-closed sur tout
cas dégradé, un seul point de vérité pour tous les producteurs, test
anti-récursion rouge-avant bloquant, pas de `INSERT` SQL brut dans les tests.

**1. La preuve est la ligne callback, jointe à son parent** (`crates/mika-agent/src/db.rs:9541-9570`) :

```sql
SELECT COUNT(*) FROM tasks child
JOIN tasks parent ON child.parent_task_id = parent.id
WHERE child.agent_id = ?1
  AND child.trigger_type = 'callback'
  AND child.dispatch_class = 'groom'
  AND child.status IN ('completed', 'delivered')
  AND instr(child.result, ?4) > 0
  AND parent.reference_url IN (?2, ?3)
```

Le callback garde sa classe `groom` (dérivée de l'entrée `skill`,
`executor.rs:2899`), atteint `completed` puis `delivered`, et son `result` est
le RESULT de dispatch-lib écrit par `POST /tasks/{id}/complete`
(`crates/mika-agent/src/server/handlers.rs:494`), qui porte `Outcome: PLAN_GROOMED` à la convergence
(`dispatch-lib.sh:6215-6220`). `?2`/`?3` acceptent l'URL nue **et** la forme
héritée `?phase=groom` : une requête, tous les producteurs. `instr` plutôt que
`LIKE` : sensible à la casse.

**2. Un vocabulaire, une constante.** `GROOM_SUCCESS_MARKER = "Outcome:
PLAN_GROOMED"` (`crates/mika-agent/src/task_state/tasks.rs:40`) est lu par la porte **et** par
l'auto-fire (`crates/mika-agent/src/task_engine/dispatcher.rs:2888`, `r.contains(GROOM_SUCCESS_MARKER)`) — le
même marqueur que le moteur faisait déjà confiance pour lancer le pilote. Le
scripteur reste le shell ; le commentaire de la constante le dit.

**3. Le bras `Err` cessait d'être fail-open.** L'appelant
`validate_dispatch_readiness` traitait une erreur DB en `warn` + autoriser.
Extrait en aide pure `groom_provenance_verdict` (`crates/mika-agent/src/skills/executor.rs:1114-1162`) :
`Ok(true)` → passe ; `Ok(false)` → `dispatch_grooming_not_verified` ; `Err` →
`dispatch_check_failed`, refus. Trois tests unitaires sans DB ni token
(`crates/mika-agent/src/skills/executor.rs:8065-8101`), dont un qui vérifie que le texte de `recovery` ne
nomme plus le drapeau (`crates/mika-agent/src/skills/executor.rs:8082`).

**4. Le refus nomme la sortie honnête.** Groomer via `mika ask --agent mika-dev
"groom <ref>"` ; reposer `ready` ne sert à rien tant que les marqueurs sont
présents ; un ticket groomé à la main répond `already_groomed`
(`dispatch-lib.sh:2210`, mika#2012), qui ne frappe **aucune** preuve — retirer
le plan de la branche d'abord.

## Pourquoi ça marche

**Une preuve se lit sur la ligne qui survit au cycle de vie de chaque
producteur.** Le parent appartient à deux mécanismes : le handler qui le crée,
l'auto-fire qui a le droit de le muter. Le callback n'appartient qu'à un seul :
dispatch-lib écrit son `result` une fois, le moteur le fait passer
`completed → delivered`, personne ne le rebascule. C'est la seule ligne dont la
forme est stable d'un bout à l'autre de la chaîne — donc la seule qu'une porte
peut lire sans parier sur l'ordre des autres.

**La forme du test anti-récursion** (`crates/mika-agent/src/task_engine/dispatcher.rs:5734-5764`,
`test_groom_gate_survives_implement_flip`) : construire la paire par l'API
d'écriture de production (`create_groom_callback_pair`, déjà existante), basculer
le parent *comme le moteur le fait*, puis interroger la porte. Rouge sur
l'ancienne porte, vert sur la nouvelle. Une nuance vaut d'être écrite : le test
est **rouge aussi sans la bascule**, parce que l'URL nue et le parent non
terminal suffisent déjà à l'ancien refus. La bascule n'est donc pas ce qui rend
le test rouge — c'est ce qui **rendrait vain un fix qui lirait le parent**. Le
test épingle la raison pour laquelle la seule réparation possible est celle-ci.
Son jumeau `test_groom_gate_refuses_plan_iterate_after_flip` (`:5761`) tient le
négatif : un groom qui a tourné sans converger n'est pas une preuve.

**Deux changements structurels le même jour** ne se voient pas l'un l'autre à
la revue : chacun est testé contre le monde d'avant. Le seul instrument qui les
aurait confrontés est le dispatch réel sans bypass — et c'est précisément ce
que le bypass rendait inutile de tenter.

## Conséquences connues, dites tout haut

- **Demi-vie de 30 jours, sur deux lignes.** `prune_completed_tasks(THIRTY_DAYS_SECS)`
  (`crates/mika-agent/src/task_engine/mod.rs:27-28`) purge parent et callback ;
  `parent_task_id … ON DELETE SET NULL` (`crates/mika-agent/src/db.rs:1422`) casse la jointure dès que
  *l'une* des deux est purgée. La porte est consultée par **chaque** dispatch
  dev-pilot sur l'issue, y compris les relances `block[ac]`/`block[ci]` du
  verdict_handler : une PR en revue plus de 30 jours après son grooming perd ses
  dispatches de correction automatiques. Fail-closed, accepté. Différé : exempter
  les lignes `PLAN_GROOMED` du prune.
- **Les tickets groomés à la main** (`/mika-groom-ticket`) sont désormais refusés
  structurellement — c'est l'intention ratifiée, pas un effet de bord. Leur seule
  sortie : retirer le plan de la branche et re-groomer par la boucle, sans quoi
  `already_groomed` est un cul-de-sac qui ne frappe pas de preuve. Différé à
  Vincent/Prime : `/mika-groom-ticket` doit-il frapper une ligne-preuve ?
- **Les tickets parqués `operator-review`** par la panne ne s'auto-réparent pas ;
  l'opérateur retire le label.
- Résiduel : la jointure borne `child.agent_id`, pas `parent.agent_id` ; la
  preuve n'a ni fraîcheur ni révocation ; le marqueur est apparié par sous-chaîne
  (comme l'auto-fire, un vocabulaire gardé à dessein) ; `dispatch_check_failed`
  est partagé avec les autres bras dégradés.

## Prévention

- **Un bypass sur une porte de sûreté porte une expiration mesurée.** Sans elle,
  il cache la mort de la porte ; le `WARN` à chaque coup devient le bruit de
  fond que personne ne lit. La preuve qu'une porte fonctionne est un dispatch
  réel qui la traverse *sans* le drapeau — c'était la condition n°7 du GO.
- **Avant d'écrire un lecteur, énumérer les écrivains et leur cycle de vie.**
  Pour chaque ligne candidate : qui la crée, qui a le droit de la muter, quand
  elle devient terminale. La ligne-preuve est celle qu'un seul mécanisme écrit.
- **Deux changements structurels le même jour sur la même table : les faire se
  rencontrer.** Un test qui joue le cycle de vie de l'un (`update_task_dispatch_class`)
  puis interroge l'autre (`has_completed_groom_for_issue`).
- **Le bras `Err` d'une porte de sûreté refuse.** Un `warn` + autoriser est
  l'inverse de la porte, sur le seul chemin où elle n'a pas vu sa preuve.
- **Un texte de `recovery` nomme une route qui marche.** Reposer `ready` menait
  au même refus ; le nommer coûtait un tour de boucle et une lecture.
- **Les tests de porte naissent de l'API d'écriture de production**, jamais d'un
  `INSERT` brut : un fixture SQL peut fabriquer exactement la ligne que la
  production ne produit pas — c'est la forme même de ce bug.

## Références

- mika#2287 (cette réparation, PR en attente) ; mika#1620 (la porte) ;
  mika#1614 (réutilisation de tâche, bascule 5c) ; mika#1572 (handler
  structurel, URL nue) ; mika#996 (patron task-reuse) ; mika#2012
  (`already_groomed`) ; #919 (le bypass).
- [verdict-writer-and-gate-must-share-one-vocabulary-2026-08-27](../workflow-issues/verdict-writer-and-gate-must-share-one-vocabulary-2026-08-27.md)
  — la règle n°1 (un vocabulaire déclaré, testé des deux côtés) est ce que
  `GROOM_SUCCESS_MARKER` matérialise.
- [guard-must-read-substance-not-the-shape-its-producer-happens-to-emit-2026-09-04](../architecture-patterns/guard-must-read-substance-not-the-shape-its-producer-happens-to-emit-2026-09-04.md)
  — même classe, un niveau plus bas : ici la forme supposée n'était pas un
  chemin ni un compte, mais une *ligne* qu'un autre mécanisme avait le droit de
  muter.
- [dispatch-gate-strict-groomed-match-2026-05-14](../logic-errors/dispatch-gate-strict-groomed-match-2026-05-14.md)
  (#1108) — même famille de porte.
- [a-guard-that-reads-its-evidence-from-the-claim-cannot-refute-it-2026-08-30](../best-practices/a-guard-that-reads-its-evidence-from-the-claim-cannot-refute-it-2026-08-30.md)
  (mika#2034) — pourquoi les marqueurs du corps ne suffisent pas et pourquoi
  cette porte existe.
- [cli-dispatch-bypasses-grooming-marker-check-2026-05-13](../workflow-issues/cli-dispatch-bypasses-grooming-marker-check-2026-05-13.md)
  — même appelant ; documente encore `MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1`
  comme remède valide : caduc depuis ce ticket, candidat à rafraîchir.
- [dispatch-readiness-guard-long-running-status-validation](../architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md)
  — doc fondateur de `validate_dispatch_readiness` ; son « fail-closed sur
  erreur DB » est le précédent que le bras `Err` réintègre ici.
- [un-booleen-qui-devient-un-seuil-a-plus-de-lecteurs-que-vous-ne-croyez-2026-09-05](../best-practices/un-booleen-qui-devient-un-seuil-a-plus-de-lecteurs-que-vous-ne-croyez-2026-09-05.md)
  — une valeur a plus de lecteurs qu'on croit : deux lecteurs Rust du même
  marqueur, une constante.
- [ready-label-dispatch-requires-grooming-marker-2026-04-30](../workflow-issues/ready-label-dispatch-requires-grooming-marker-2026-04-30.md),
  [cli-dispatch-bypasses-grooming-marker-check-2026-05-13](../workflow-issues/cli-dispatch-bypasses-grooming-marker-check-2026-05-13.md)
  — lignée de la porte.
