---
title: "Une garde de dispatch webhook doit borner la LIGNÉE de l'événement, pas seulement sa nature — ni l'identité de sa cible"
date: 2026-10-03
category: architecture-patterns
module: skills-executor
problem_type: architecture_pattern
component: tooling
severity: high
applies_when:
  - "Une garde autorise une action coûteuse (spawn de pilote) d'après la NATURE de l'événement déclencheur (préfixe `[GitHub] PR`, `Check suite`)"
  - "On veut lier une autorisation à la cible d'un événement et un flux légitime agit sur une AUTRE cible que celle de l'événement"
  - "On choisit le contrôle positif d'un test de garde alors que certains appelants passent `originating_message = None`"
symptoms:
  - "Un tour mika-dev ouvert par une revue QA sur la PR #2647 a lancé un implement de mika#2646, hors fenêtre ready"
  - "Deux pilotes implement simultanés (cap implement=1 franchi) de 20:03:17Z à ~20:24Z le 2026-10-02"
root_cause: missing_validation
resolution_type: code_fix
tags:
  - webhook
  - dispatch-guard
  - unauthorized-webhook-dispatch
  - lineage
  - milestone-cascade
  - positive-control
  - claude-pilot
---

# Une garde de dispatch webhook doit borner la LIGNÉE de l'événement, pas seulement sa nature — ni l'identité de sa cible

## Context

Le 2026-10-02 (trace `17ba765a-be9c-11f1-94aa-c13d0500c506`), un tour mika-dev
rejoué depuis un webhook `pull_request_review.submitted` sur la PR #2647 a été
re-prompté « required tools », puis a appelé `create_task` (« Implement
mika#2646 ») et `run_claude_pilot` sur cette tâche fraîche. Un vrai pilote est
parti pendant qu'un implement de #2636 volait déjà (mika#2649).

La garde (0) `is_unauthorized_webhook_dispatch` (mika#841 / mika#933) a laissé
passer, et elle faisait ce pour quoi elle était écrite : elle juge la **nature**
de l'événement. Les préfixes `[GitHub] PR ` et `[GitHub] Check suite ` sont hors
du domaine Fallthrough parce que `self-dev-webhook-qa` / `-ci` portent des
dispatchs légitimes sur ces tours. Une fois l'événement classé « territoire
légitime », **plus rien ne liait le dispatch à la PR de l'événement** : un
événement sur #2647 autorisait un pilote pour n'importe quel ticket. Une
allowlist par nature délègue l'autorité sur *toutes* les cibles.

Le doc voisin `docs/solutions/logic-errors/webhook-fallthrough-dispatches-unrelated-backlog-work.md`
(mika#583) traitait les événements **sans** handler. Ce cas-ci est son
complément : l'événement **a** un handler, il est sur l'allowlist, et la fuite
passe par là.

## Guidance

Trois décisions, chacune non évidente depuis le code final.

### 1. La nature d'un événement ne suffit pas : borner aussi à quoi il s'applique

Toute garde qui ouvre une action coûteuse sur critère de nature (préfixe,
type d'événement, canal) doit aussi répondre « *pour quelle cible ?* ». Le
correctif ajoute `evaluate_event_target_binding` dans
`validate_dispatch_readiness` (`crates/mika-agent/src/skills/executor.rs:2326`,
`:2617`), placée **avant** la porte de grooming et les allers-retours GitHub,
et qui ne mord que sur `run_claude_pilot` / `run_claude_pilot_groom`. La cible
est lue par les lecteurs uniques existants (`deadline_verdict::parse_pr_target`
pour les PR, `worktree_reaper::issue_number_from_branch` pour la branche d'un
check-suite) — aucun second parseur de grammaire.

### 2. Borner par LIGNÉE, pas par identité de cible — sinon la cascade M4 casse

Le réflexe « même cible que l'événement, sinon refus » est faux ici. Sur un
`[GitHub] PR closed:`, la cascade de jalon M4 dispatche légitimement **le frère
suivant**, donc un ticket qui n'est *pas* celui de la PR
(`skills/bundled/self-dev-webhook-qa/system_prompt.md:236`). La garde #1218
(`crates/mika-agent/CLAUDE.md`) va même jusqu'à **exiger** cet avancement sur les
tours PR-closed d'un jalon. Une garde par identité aurait refermé l'incident et
arrêté la boucle de jalon.

Ce qui distingue M4 de l'incident n'est pas la cible mais sa **relation** à
l'événement :

| | M4 légitime | l'incident |
|---|---|---|
| la tâche dispatchée | enfant pending préexistant du parent jalon | créée dans le tour même |
| `parent_task_id` | le parent jalon, partagé avec la tâche de la PR | nul |
| metadata | porte la lignée | vide |

D'où quatre termes de lignée (`LineageTerm`, `crates/mika-agent/src/webhook_dispatch.rs`),
le premier qui tient autorise :

- **L1** `reference_url` de la tâche nomme la cible de l'événement ;
- **L2** `claude_pilot.pr_url` de la tâche nomme la PR (lu par `extract_pr_url`, `executor.rs:4485`) ;
- **L3** un **frère** (même parent non nul) satisfait L1/L2 — *c'est M4* ;
- **L4** le **parent** satisfait L1/L2.

Aucun terme ⇒ `webhook_dispatch_target_mismatch`, refus audité sous un nom
unique (`webhook_dispatch_target_binding`, verdict dans `after_value`).

Pas de `gh pr view --json closingIssuesReferences` pour trancher « le ticket
fermé par cette PR » : le lien est déjà porté en base par L2, et un appel réseau
de plus sur un chemin qui spawne un processus était refusé (*la cible est dite,
jamais dérivée*).

### 3. Le contrôle positif doit venir de la population qui traverse VRAIMENT le prédicat

L'AC du ticket voulait comme contrôle positif « le dispatch CI-fix / QA-hold sur
X passe ». Mais ces dispatchs sont **moteur** : `verdict_handler` (et
`iterate_dispatch`) appellent `validate_dispatch_readiness` avec
`originating_message = None` (`crates/mika-agent/src/server/verdict_handler.rs:928-931`), ce qui saute
la garde (0) *et* le nouveau prédicat. Un test sur eux passe parce que le
prédicat **n'est pas interrogé** — il ne prouve rien.

La seule population légitime qui traverse le prédicat sur le chemin LLM est la
cascade M4. Le contrôle positif dur est donc
`mika2649_la_cascade_de_jalon_m4_passe` (L3), et les dispatchs moteur sont
épinglés à part comme *hors population* (`mika2649_un_tour_non_pr_nest_pas_dans_la_population`).

## Why This Matters

- Sans le point 1, toute nouvelle entrée sur l'allowlist par nature élargit
  silencieusement l'autorité à toutes les cibles ; le re-prompt « required
  tools » suffit à faire franchir le pas au modèle dans un tour qui n'avait rien
  à faire.
- Sans le point 2, le correctif « évident » casse un flux prescrit (M4) que rien
  dans `webhook_dispatch.rs` ne montre : il vit dans un prompt de skill et dans
  une garde d'intention voisine.
- Sans le point 3, la suite de tests est verte avec un prédicat qui refuserait
  M4 : le contrôle positif apparent ne traverse jamais le code testé.

## When to Apply

- Ajouter un préfixe ou un type d'événement à une allowlist qui ouvre un spawn,
  une annulation ou une mutation du plan de dispatch.
- Écrire une garde « bornée à la cible » : chercher d'abord les flux légitimes
  qui agissent sur une autre cible (cascades, frères, re-tentatives sous parent)
  — ils fixent le discriminant.
- Choisir un contrôle positif : vérifier que l'appelant passe réellement par le
  prédicat (ici : `originating_message` non nul).

## Examples

Direction du fail-safe, non uniforme à dessein :

- cible d'événement illisible ⇒ `event_unreadable` ⇒ **autorise** (rien à quoi borner) ;
- check-suite sur une branche sans numéro (`main`) ⇒ `no_target_in_event` ⇒ **autorise** ;
- un terme de lignée illisible (metadata vide, JSON non conforme) ⇒ ce terme
  **n'est pas satisfait**. C'est porteur : la tâche de l'incident avait une
  metadata vide, donc « illisible ⇒ autorise » aurait laissé passer l'incident
  (`mika2649_une_metadata_illisible_nest_pas_un_terme_satisfait`) ;
- erreur DB sur la traversée ⇒ `lineage_unreadable` ⇒ **refuse**, aligné sur la
  garde voisine `dispatch_check_failed`.

Hors périmètre, avec raison : le cap implement (AC2) compte des **lignes**
callback `pending/in_progress`, pas des **processus** ; un pilote vif à ligne
terminale lui est invisible. Ce trou n'a pas pu être établi comme cause de
l'incident (base et log hors du bac à sable de dispatch) et reste un suivi ; le
terme de lignée refuse le dispatch mesuré avant que le cap entre en jeu.
`cancel_task` / `create_task` / `update_task_status` ne sont pas bornés :
`ToolContext` ne porte qu'un booléen (`is_webhook_fallthrough_turn`,
`crates/mika-agent/src/tools/mod.rs:178`), et M4 a besoin d'`update_task_status` sur le frère.

## Related

- mika#2649 — le ticket ; PR senara-solutions/mika#2655 (non mergée à l'écriture)
- `docs/plans/2026-10-03-001-fix-2649-webhook-pr-dispatch-lineage-bound-plan.md` — R1–R6, recensement AC4, sondes post-déploiement
- `docs/solutions/logic-errors/webhook-fallthrough-dispatches-unrelated-backlog-work.md` — mika#583, le cas des événements sans handler
- `docs/solutions/architecture-patterns/intent-guard-predicate-sharing-2026-05-14.md` — prédicats partagés entre garde pré-hoc et garde EndTurn sur `[GitHub]`
- `docs/solutions/architecture-patterns/post-hoc-vs-tool-boundary-guard-placement-2026-05-13.md` — pourquoi la garde est pré-hoc
