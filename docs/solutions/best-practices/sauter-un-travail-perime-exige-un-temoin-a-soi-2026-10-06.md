---
title: Sauter un travail périmé exige un témoin à soi, pas seulement un successeur
date: 2026-10-06
category: best-practices
module: mika-agent/server
problem_type: best_practice
component: qa-review
severity: medium
applies_when:
  - On veut sauter un travail parce qu'un travail plus récent de la même clé existe
  - Le registre consulté ne porte pas d'horodatage fiable de l'événement courant
  - Certains événements de la population n'ont jamais été inscrits au registre
tags:
  - debounce
  - audit-events
  - supersession
  - qa-review
  - mika-2671
---

# Sauter un travail périmé exige un témoin à soi, pas seulement un successeur

## Le contexte

mika#2671 phase B2 : avant un tour de revue déclenché par un `synchronize`, le
moteur doit vérifier que la tête visée est encore la tête courante, sinon sortir
sans appel LLM. Le registre `qa_pr_sync_observed` (phases A et B1) porte déjà les
retenues durables de l'anti-rebond, chacune avec l'identité de son événement
(`reasoning = request_id=<uuid>`).

## Le piège

Le prédicat tentant est « une retenue pendante d'une autre identité existe sur
cette clé ⇒ ma tête est périmée ». Il est faux sur une population précise : un
événement qui **n'a jamais été retenu** (anti-rebond désarmé, chemin hérité,
écriture `stage=held` en échec — B1 verse alors l'événement directement en file).
Pour cet événement, une retenue pendante d'une autre identité peut être **plus
ancienne** que lui. Le sauter ferait revoir l'ancienne tête en dernier et jamais
la nouvelle : la perte de revue que tout le ticket existe pour empêcher.

Sans horodatage de l'événement courant, « plus récent que moi » n'a de sens que
si **je suis moi-même dans le registre**. Le successeur seul ne suffit pas : il
faut un témoin à soi pour situer le successeur.

## La règle

> Périmé ⇔ une retenue à soi existe **et** une retenue d'une autre identité a un
> `id` strictement supérieur sur la même clé. Pas de retenue à soi ⇒ aucun
> témoin ⇒ le travail tourne.

Deux propriétés en découlent sans mécanisme supplémentaire :

- la retenue la plus récente d'une clé n'a jamais de successeur, donc **le
  dernier événement n'est jamais sauté** ;
- seules les retenues comptent comme successeurs, parce qu'elles sont durables
  (rejouées jusqu'à leur tour). Une ligne « tour démarré » d'une autre identité
  ne prouve pas que sa tête est plus récente.

## Une seconde question, pas un second registre

La requête (`Database::newer_audit_holds`) lit le même registre, la même clé, le
même marqueur d'identité que la reprise de B1. Rien n'est écrit au registre sur
un tour sauté — il n'a pas démarré, et la retenue sautée n'est déjà plus
pendante. La ligne d'observabilité (`qa_review_head_superseded`) n'est relue par
aucune décision.

## Pourquoi ce n'est pas dans le code seul

Le code dit *quoi* ; il ne dit pas pourquoi la variante « une retenue pendante
d'autrui suffit » — plus simple, réutilisant `list_pending_audit_holds` — a été
écartée. C'est l'événement non retenu qui la réfute, et il n'apparaît dans aucun
test nominal de l'anti-rebond.
