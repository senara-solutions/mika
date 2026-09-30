---
title: "Une garde à fenêtre qui lit un état terminal : assécher l'écrivain, pas exempter le lecteur"
date: 2026-09-30
category: architecture-patterns
module: task-engine
problem_type: architecture_pattern
component: task-engine
severity: high
applies_when:
  - "Une garde refuse une action tant qu'un état terminal (failed, cancelled, expired) récent existe dans une fenêtre de temps"
  - Le système lui-même écrit cet état terminal sur un chemin qui n'est pas un échec (balayage de démarrage, knob, opérateur)
  - On s'apprête à ajouter une n-ième exemption à cette garde
  - "Un statut sert à la fois d'état transitoire d'exécution et d'état de registre"
symptoms:
  - "Une tâche récurrente cesse de tirer après un restart, puis revient seule environ 24 h plus tard"
  - "Le journal porte « mika#1742: refusing to re-register recurring task » sur un label qui n'a jamais échoué"
  - La garde a accumulé plusieurs exemptions par marqueur de metadata, chacune pour un écrivain système différent
root_cause: logic_error
resolution_type: code_fix
related_components:
  - startup_recovery
  - create_recurring_task_if_absent
related_issues:
  - mika#2575
  - mika#1742
  - mika#2271
  - mika#2337
  - mika#2446
tags: [zombie-guard, recurring-task, startup-recovery, terminal-state, grace-window, restart]
---

# Une garde à fenêtre qui lit un état terminal : assécher l'écrivain, pas exempter le lecteur

## Context

La garde anti-zombie de mika#1742 (`create_recurring_task_if_absent`,
`crates/mika-agent/src/db/tasks.rs`) refuse de ré-inscrire une récurrente dont
une ligne du même `(agent_id, label)` est passée à
`status IN ('failed', 'cancelled', 'expired')` dans les
`RECURRING_ZOMBIE_GRACE_HOURS` (24 h). Elle traite l'état terminal comme la
**preuve** d'une mort. Or le système écrit lui-même des états terminaux qui ne
prouvent aucune mort, et chacun est devenu une exemption côté lecteur :

| exemption (lecteur) | écrivain système qui empoisonnait la garde |
|---|---|
| mika#2271 — marqueur `RECURRING_CONFIG_CANCEL_REVERTED_PATH` | un knob ou `identity.toml` qui annule la ligne volontairement |
| mika#2337 — marqueur `RECURRING_UNKNOWN_TRIGGER_PATH`, à usage unique | une mort par `DispatchError::UnknownTrigger` |
| mika#2446 — marqueur `RECURRING_OPERATOR_REARM_PATH` | le geste `mika tasks rearm <label>` |

mika#2575 est le quatrième écrivain de la série : `startup_recovery` passait à
`failed` toute ligne orpheline `in_progress`, y compris une récurrente surprise
**en plein tir** par le restart. Mesuré deux fois : `wip_rescue` (ligne
`ce90ad84`, 2026-09-28) mort ~28 h, et le `heartbeat` de mika-arch (`2b71969e`,
2026-09-25) mort ~24,5 h. La chaîne complète, maillon par maillon, est dans la
section *mika#2575* de `CLAUDE.md` et de `crates/mika-agent/CLAUDE.md` ; elle
n'est pas répétée ici.

## Guidance

**Avant d'ajouter une exemption à une garde qui lit un état terminal, demander
si l'écrivain avait le droit d'écrire cet état.** Une exemption côté lecteur
accepte que l'état faux soit écrit, puis apprend à la garde à l'ignorer — elle
doit donc être marquée, propagée, consommée, et chaque nouvel écrivain système
en réclamera une autre. Côté écrivain, on cesse de fabriquer la population : la
garde reste intacte, sans exemption de plus, et redevient juste parce que ce
qu'elle lit redevient vrai.

Le test qui départage : **l'état écrit décrit-il l'objet que la garde surveille,
ou un autre ?** Pour une récurrente, `in_progress` est un état de *tir*
transitoire — `claim_and_fire_task` le pose, `update_task_rescheduled` repose
`recurring_active` au retour. Le trouver au démarrage dit « un tir a été
interrompu », pas « l'enregistrement est mort ». Écrire `failed` confondait
l'échec d'un tir avec la mort d'un registre, et c'est cette confusion seule qui
armait la garde. Le correctif (`restore_recurring_after_restart`,
`crates/mika-agent/src/task_engine/engine.rs`) ré-arme donc la ligne par le même
primitif que le tir nominal, et ne retombe sur `failed` que si aucun instant
futur n'est calculable — cas où la garde s'arme **légitimement**.

Les trois remèdes refusés sont eux aussi la leçon, parce que chacun paraît
naturel :

- **Réordonner** (balayage avant enregistrement) : la garde lit alors un
  `failed` vieux de quelques millisecondes et refuse **dans le même
  démarrage** — une panne d'un cycle devient une panne immédiate de 24 h.
- **Statut neuf** (`interrupted_by_restart`) : c'est encore une exemption,
  déguisée en valeur de `CHECK`, au prix d'une reconstruction de table SQLite.
- **Prédicat d'horloge** (`updated_at ≈ démarrage`) : échange un prédicat
  d'état contre un proxy temporel borné — une dette datée.

## Why This Matters

- **Une garde à fenêtre transforme un incident ponctuel en panne longue.** Le
  défaut coûte un tir ; la garde le facture 24 h.
- **La durée réelle n'est pas la fenêtre, c'est la fenêtre arrondie au
  prochain événement qui ré-essaie.** La garde ne se réévalue qu'au démarrage :
  le restart n°18 a raté la fin de grâce de 37 s, d'où ~28 h et non 24 h.
- **Le « remède immédiat » intuitif était faux, et seule la mesure l'a
  montré.** « Le restart suivant ré-armera » a été annoncé puis réfuté au
  restart n°13 par le `WARN mika#1742` : une ligne `failed` récente n'est pas
  remplaçable, elle est gardée.
- **La sonde passe après expiration, pas après correctif.** Le 2026-09-30 les
  quatre labels de `mika-dev` étaient `recurring_active` avant tout
  déploiement : une ligne neuve (`358c6e37`) créée parce que la grâce avait
  expiré. Une sonde d'état qui passe sur une garde à fenêtre ne prouve rien
  tant qu'on n'a pas vérifié que la fenêtre n'a pas simplement expiré.
- **Le silence ressemble au succès.** Un scan récurrent mort ne journalise rien
  et se lit comme un scan qui n'a rien trouvé (classe mika#2205).

## When to Apply

- Une garde (refus, veto, disjoncteur) a une fenêtre temporelle et un état
  terminal comme preuve.
- Un chemin de *maintenance* (balayage de démarrage, reaper, knob, CLI
  partageant la base d'un démon) écrit cet état terminal.
- La liste d'exemptions de la garde grandit : à la troisième, énumérer les
  **écrivains** de la population gardée avant d'écrire la quatrième.
- Un même statut sert à la fois de marqueur d'exécution en cours et de
  verdict — le séparer par *sens* (tir vs registre) avant d'en ajouter un.

## Examples

Énumérer les écrivains de la population gardée, plutôt que ses lecteurs :

```bash
# Qui écrit un état que la garde mika#1742 compte comme mort ?
grep -rnE "update_task_status\(.*(FAILED|CANCELLED|EXPIRED)|status = '(failed|cancelled|expired)'" \
  crates/mika-agent/src
```

Chaque site trouvé reçoit une question : décrit-il la mort de
l'**enregistrement** ? Si non, le corriger à l'écriture. Limite restante nommée
par mika#2575 : l'étape 1 de `startup_recovery` (`mark_tasks_expired`) écrit
`expired` sur une récurrente portant un `timeout_at`, population vide pour les
sept récurrentes du démarrage mais non couverte.

Distinguer un refus légitime d'un refus fabriqué, après déploiement :

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'recurring_restart_restore' GROUP BY 1;
-- 'failed_no_cron' explique un refus mika#1742 légitime ;
-- un refus sans cette ligne = un autre écrivain système à trouver.
```
