---
title: Une garde au point de pose ne couvre pas ce que le producteur a déjà écrit
date: 2026-10-02
category: best-practices
module: crates/mika-agent/src/teams
problem_type: best_practice
component: agent-core
severity: high
applies_when:
  - "Poser une garde fail-closed sur une valeur au point où elle est posée (persistance, notification), et non là où elle est produite"
  - "Écrire « les surfaces aval sont couvertes par construction » dans un doc-comment"
  - "Ajouter une disposition qui demande un tour LLM supplémentaire (re-rédaction, re-prompt) à la fin d'une exécution bornée"
  - "Étiqueter la provenance d'une valeur à un site qui ne sait pas quel bras l'a produite"
related_components:
  - testing_framework
tags:
  - garde
  - point-de-pose
  - fail-closed
  - provenance
  - must-use
  - doctrine-non-transit
  - mika-2633
---

# Une garde au point de pose ne couvre pas ce que le producteur a déjà écrit

## Context

mika#2633 a fermé le canal du livrable d'équipe pour la doctrine non-transit :
la garde `testimony_access_proposal` est posée dans
`TeamEngine::commit_deliverable` (`crates/mika-agent/src/teams/engine.rs`), seul
site qui pose `run.deliverable`, **avant** la notification et la persistance. Le
choix du point de pose, plutôt que du producteur (`deliver()`) ou de la
notification, était juste : quatre sites posent, un seul produit, et la
notification laisse la proposition en base.

Le pilote est mort au plafond de 151 tours après une revue multi-agents à neuf
relecteurs qu'il n'a jamais appliquée. La reprise a appliqué ces constats, et
quatre d'entre eux ont la même forme : **la garde était au bon endroit pour ce
qu'elle voyait, et quatre chemins passaient à côté de ce qu'elle voyait.** Le
doc-comment disait « les surfaces sont couvertes par construction ». Ce n'était
vrai d'aucune des quatre.

## Guidance

Une garde au point de pose ne contrôle que **la valeur qu'on lui passe et ce que
ses appelants font de la valeur qu'elle rend**. Avant de la déclarer complète,
répondre à quatre questions.

1. **Qu'est-ce que le producteur a déjà écrit lui-même, avant la garde ?** Le
   rédacteur reçoit l'instruction « Write the final deliverable to
   `deliverable.md` using `write_workspace` » (`crates/mika-agent/src/teams/prompt.rs`,
   `build_deliverable_context`) et l'exécute **pendant** `deliver()`. Sur un
   refus, ce fichier gardait la proposition : lisible par `read_workspace` dans le
   run, et par tout run lancé avec `reference_run_id`. C'est la classe « atteint
   le prochain run » que le ticket fermait pour `team_runs.deliverable`, rouverte
   d'un cran. Remède : sur un refus, la garde réécrit l'artefact du producteur
   avec le texte engagé (`overwrite_workspace_deliverable`), et seulement s'il
   existe.

2. **La provenance est-elle déclarée par le seul site qui sait quel bras a
   tourné ?** `deliver_phase` étiquetait toujours `Writer`, alors que
   `deliver()` rend le **repli workspace** (#1128) quand le rédacteur dépasse son
   enveloppe. La disposition (c) demandait alors une re-rédaction à l'agent qui
   venait de s'épuiser, en lui affirmant un fait faux sur son propre tour, et
   estampillait `deliverable_source = writer` sur un texte qu'aucun rédacteur
   n'avait écrit (classe mika#2304). Remède : `deliver()` rend
   `(texte, DeliverableSource)`, et une variante `WorkspaceFallback` rejoint le
   bras sans re-rédaction du `match` exhaustif.

3. **« Couvert par construction » est-il imposé au compilateur ?** Les surfaces
   aval (`TeamEvent::Deliverable`, que la TUI pousse dans le chat **et**
   enregistre en base, et `.meta/deliverable.md`) ne sont couvertes que si chaque
   appelant transmet la valeur **rendue**. Un site qui appelle le commit puis
   transmet le texte produit garde le scan de pose vert et tous les tests verts.
   Remède : `#[must_use]` sur `commit_deliverable`, `let _ =` sur le seul rejet
   délibéré, et un test qui pilote `deliver_phase` et lit les deux surfaces.

4. **La disposition dépense-t-elle un budget que l'exécution n'a plus ?** La
   re-rédaction est un tour d'agent complet, avec une enveloppe **neuve**
   (`team_agent_timeout_secs`, 300 s par défaut), tirée à la fin d'un run borné à
   900 s. Coupée par le mur du run, elle ne pose rien : le run persiste `None`, et
   la notification dit « no deliverable produced », l'énoncé faux que la ligne
   neutre existe pour empêcher. Remède : `TeamEngine.run_deadline`, et une
   re-rédaction refusée quand l'enveloppe ne tient pas avant le mur
   (`team_deliverable_rewrite_skipped_no_budget`). C'est la forme de la garde de
   continuation de `run_loop` (mika#848 F3a).

## Why This Matters

Les quatre défauts ont la même signature : **aucune décision n'est fausse le jour
où ils existent.** La garde détecte, la télémétrie tire, les tests du commit sont
verts, et la proposition passe par un chemin que personne n'a énuméré. C'est la
classe mika#2205 (une couverture inerte se lit comme une couverture), appliquée
non plus à un scan mais à une affirmation de doc-comment. La revue l'a trouvée
parce qu'elle lisait les chemins aval, pas la garde.

Le deuxième point coûte aussi le budget et la vérité de la télémétrie. Le
quatrième transforme un refus énoncé en silence menteur.

## When to Apply

- Chaque fois qu'une garde est placée au point de pose plutôt qu'au producteur :
  énumérer les écritures du producteur **antérieures** à la garde, pas seulement
  les lecteurs **postérieurs** de la valeur.
- Chaque fois qu'un doc-comment dit « par construction » : chercher ce qui, dans
  le langage, impose cette construction. S'il n'y a rien, c'est une convention, et
  il faut un `#[must_use]`, un type ou un scan.
- Chaque fois qu'une disposition rappelle un modèle à la fin d'une exécution
  bornée : borner son enveloppe par le temps restant, jamais par une constante.

## Examples

Avant : le site 1 suppose la provenance.

```rust
let produced = self.deliver().await?;
let agent_name = self.resolve_writer_agent();
let deliverable = self
    .commit_deliverable(produced, DeliverableSource::Writer { agent_name })
    .await;
```

Après : la provenance vient de la seule fonction qui sait quel bras a tourné.

```rust
let (produced, source) = self.deliver().await?;
let deliverable = self.commit_deliverable(produced, source).await;
```

Tests qui épinglent chaque point, dans `teams::engine::tests` (vus rouges avant
le correctif ou par mutation) : `mika2633_un_repli_workspace_na_pas_de_redaction`,
`mika2633_le_fichier_livrable_du_workspace_porte_le_texte_engage`,
`mika2633_le_site_1_transmet_le_texte_engage_a_ses_surfaces`,
`mika2633_une_redaction_sans_budget_restant_nest_pas_tentee`.

Limite nommée, non fermée : le mur de 300 s de l'outil `run_team` synchrone n'est
pas connu du moteur. La garde de budget ne couvre que le mur du run.

Voir aussi : `docs/solutions/best-practices/un-plancher-danti-vacuite-peut-se-rembourrer-avec-le-trou-quil-garde.md`
(le même ticket a durci le plancher du scan de canal suivant cette règle).
