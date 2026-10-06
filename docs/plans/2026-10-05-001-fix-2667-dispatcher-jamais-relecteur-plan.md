---
title: "mika#2667 — l'agent dispatcher n'est jamais relecteur"
type: fix
issue: senara-solutions/mika#2667
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# mika#2667 — l'agent dispatcher n'est jamais relecteur

**Ticket :** senara-solutions/mika#2667
**Lié :** mika#2218 / mika#2248 (séparation des rôles à la porte de merge),
mika#2237 (garde verdict↔drapeau, même famille pré-subprocess), mika#2573 (refus
de création de travail sur un tour Fallthrough), mika#2517 (retrait d'outil par
classe de tour), mika#1829 / mika#1853 (porte périmètre).
**`server/verdict_handler.rs` est decision-core (CODEOWNERS) — merge par Vincent.**

## Goal Capsule

- **Objective.** Une PR de la boucle ne peut être mergée qu'au vu d'un verdict
  posé par l'identité de revue. L'identité dispatcher ne peut ni produire ce
  verdict, ni se le reconnaître, ni compenser l'absence de tâche d'un
  `hold[review]` en créant du travail de son propre chef.
- **Means.** Trois gardes indépendantes, une par côté du défaut : écriture
  (KTD1), lecture (KTD3), et tour sans tâche (KTD2).
- **Autorité.** Le corps du ticket mika#2667 (AC1–AC4) prime sur ce plan ; ce
  plan prime sur les intuitions d'implémentation.
- **Stop.** Toute découverte qui exigerait de toucher la cause amont du
  `hold[review]` (vérification de build en revue) s'arrête : hors périmètre.
- **Qui finit.** Le pipeline `/mika` ouvre la PR ; le merge est celui de Vincent.

## Product Contract

### Summary

Fermer la séparation des rôles revue/dispatch des deux côtés : refuser à l'outil
`run_gh` l'écriture d'un verdict ou d'une approbation par le dispatcher, refuser
au handler structurel le merge sur un verdict qui ne vient pas de l'identité de
revue, et faire d'un `hold[review]` sans tâche active un tour qui rend la main à
l'opérateur sans créer de travail.

### Problem Frame

La séparation des rôles (mika#2218, mika#2248) donne la revue à
`mika-platform-qa` et le dispatch à `mika-platform-dev`. Elle n'est tenue que par
le prompt côté écriture et n'est pas vérifiée côté lecture. Un `hold[review]` de
QA arrivé sans tâche active est passé au LLM du dispatcher, qui a fait lui-même la
vérification, posté un `VERDICT: pass` avec approbation, puis appelé la porte de
merge. Seule la porte périmètre l'a arrêté, parce que la PR était decision-core.
Sur une PR MECHANICAL, le bras `pass` de `verdict_handler` ne lit pas l'auteur de
la revue avant `run_gh_merge`. Détail privé côté orchestrateur, référence
`dispatcher-self-approval-2026-10-05`.

### Requirements

- **R1 (AC1).** Quand l'agent courant est le dispatcher, `run_gh` refuse avant
  tout subprocess un `gh pr review` qui porte `--approve` (ou `-a`) ou dont le
  corps contient une ligne `VERDICT:`. Corps de refus sobre, qui nomme la règle
  sans gabarit de contournement. Un `--comment` sans verdict passe ; l'agent de
  revue n'est pas touché.
- **R2 (AC2).** Le passthrough « `hold[review]` sans tâche active » ne se traduit
  ni en création de tâche ni en dispatch dans ce tour ; il rend la main à
  l'opérateur et le dit.
- **R3 (AC3).** Dans le bras `pass`, `run_gh_merge` n'est atteint que si l'auteur
  de la revue est `REVIEWER_FORGE_LOGIN` (lu depuis
  `mika_common::forge_identity`, aucun littéral recopié). Tout autre auteur ⇒
  `Passthrough` nommé et ligne d'audit.
- **R4 (AC4).** La PR nomme l'effet de bord (un état de revue faux a pu être
  écrit dans la mémoire de l'agent dispatcher) et le renvoie à un geste
  opérateur ; aucun nettoyage dans ce diff.

### Scope Boundaries

- Hors périmètre : la cause amont du `hold[review]` (vérification de build
  impossible en revue, dispatch différé perdu) — autre ticket.
- Hors périmètre : le nettoyage de la mémoire de l'agent dispatcher (R4).
- Pas de nouveau champ sur `ToolContext` : les deux mécanismes existants
  (retrait d'outil par message, porte 0 de `validate_dispatch_readiness` par
  `originating_message`) suffisent.
- Ticket PUBLIC : ni trace de session, ni horodatage d'incident, ni mode
  opératoire dans le code, les tests, la PR ou les docs.

## Planning Contract

### Key Technical Decisions

- **KTD1 — AC1 : prédicat pur dans `evidence::guards`, application dans
  `run_gh`.** Même découpage que mika#2237 et mika#2573. Le prédicat
  `detect_review_verdict_act(argv)` rend `Approve`, `VerdictLine` ou
  `UnreadableBody` ; il saute les valeurs des drapeaux à valeur
  (`PR_REVIEW_VALUE_FLAGS`) pour ne pas lire `--approve` dans la prose d'un corps.
  Le corps est lu sur `--body`, `-b`, `--body=` ; `--body-file` / `-F` rend
  `UnreadableBody`, refusé **pour le dispatcher seulement** (fail-closed : un
  corps illisible ne prouve pas l'absence de verdict). La reconnaissance d'une
  ligne `VERDICT:` passe par le lecteur unique existant de la grammaire,
  `server::verdict::verdict_raw_value` (même `VERDICT_RE`), et non par
  `parse_verdict` : une ligne `VERDICT:` non classable reste un verdict posé.
  Rectification à l'implémentation : ce lecteur existait déjà, aucun
  `has_verdict_line` n'est ajouté. Le
  terme d'identité est dans la fonction d'application
  (`forge_identity::is_dispatcher_agent(ctx.db.agent_id())`). Placée juste après
  `validate_qa_review_gh_scope`, avant toute garde qui touche le réseau.
- **KTD2 — AC2 : un marqueur écrit par le handler, lu par les deux portes
  existantes.** `verdict_handler` ouvre l'enrichissement du passthrough par une
  constante `HOLD_REVIEW_NO_TASK_MARKER` ; `webhook_dispatch` porte le prédicat
  unique `is_hold_review_without_task_turn(msg)`. Deux consommateurs :
  `effective_disabled_tools` (agent_loop) retire `FALLTHROUGH_WITHHELD_TOOLS`
  (`create_task`) sur ce tour, et `validate_dispatch_readiness` refuse tout
  dispatch long (couvre `run_claude_pilot` et `run_claude_pilot_groom`, qui
  passent tous deux par cette porte). Le passthrough notifie aussi l'opérateur et
  écrit une ligne d'audit. Le message commence par `[verdict_handler]`, donc reste
  hors du domaine Fallthrough : aucun consommateur de mika#2517 ne bouge.
- **KTD3 — AC3 : condition d'auteur en tête du bras `pass`.** Après la garde
  `state == approved` existante et avant `handle_pass_verdict`, donc avant tout
  appel `gh`. Lecteur unique dans `forge_identity` :
  `is_reviewer_forge_login(login)` (rognage, `@` de tête, casse, suffixe `[bot]`).
  Autre auteur ⇒ `Passthrough` dont l'enrichissement nomme la règle, `warn!`
  `verdict_pass_from_non_reviewer`, ligne d'audit
  `VERDICT_PASS_NON_REVIEWER_AUDIT_TOOL`. Conséquence assumée : un `pass` d'un
  non-relecteur sur une PR decision-core n'atteint plus la notification
  forge-gate — il est refusé plus tôt, pour une raison plus forte.
- **KTD4 — test d'AC3 sans réseau.** Pas de seam `gh` dans le handler. Le test
  passe `github_token = None` : sur le code actuel, l'événement de
  `mika-platform-dev` atteint `handle_pass_verdict` (enrichissement « no GitHub
  token », entrée du chemin de merge) — c'est le rouge observable ; après le
  correctif il rend le passthrough nommé avec sa ligne d'audit. Le contrôle
  positif (`mika-platform-qa`) atteint `handle_pass_verdict`, preuve que la
  condition laisse passer le relecteur. Aucun appel GitHub réel.

### Sequencing

U1 (forge_identity) → U2 (AC1) → U3 (AC3) → U4 (AC2) → U5 (PR).

## Implementation Units

### U1. Lecteur unique du login de revue

- **Goal.** Fournir `is_reviewer_forge_login` (le lecteur de ligne `VERDICT:`
  existe déjà : `verdict_raw_value`).
- **Files.** `crates/mika-common/src/forge_identity.rs`.
- **Tests.** `is_reviewer_forge_login` : vrai sur `mika-platform-qa`,
  `@Mika-Platform-QA`, `mika-platform-qa[bot]` ; faux sur `mika-platform-dev`,
  `mika-qa`, `""`.

### U2. AC1 — le dispatcher ne poste pas de verdict

- **Files.** `crates/mika-agent/src/evidence/guards.rs`,
  `crates/mika-agent/src/skills/builtin_handlers.rs`.
- **Tests.** Prédicat : `--approve`, `-a`, `--comment --body "VERDICT: pass"`,
  `--body=...VERDICT...`, `--body-file x` ; contrôle négatif : `--comment --body
  "LGTM"`, un `--body` qui cite `--approve` dans sa prose sans verdict, un argv
  hors `pr review`. Application via `run_gh` (refus avant subprocess) : refus sur
  `--approve`, refus sur `--comment` + `VERDICT:` pour `mika-dev` ; contrôle
  négatif `--comment` sans verdict non refusé par cette garde ; contrôle négatif
  `mika-qa` non touché par cette garde.

### U3. AC3 — merge seulement sur un verdict du relecteur

- **Files.** `crates/mika-agent/src/server/verdict_handler.rs`.
- **Tests.** `pass` + `approved` de `mika-platform-dev` ⇒ passthrough nommé +
  ligne d'audit, aucun appel au chemin de merge (rouge sur le code actuel, KTD4) ;
  contrôle positif `mika-platform-qa` ⇒ atteint `handle_pass_verdict`. Le scan
  `mika2617_run_gh_merge_has_exactly_three_call_sites` reste vert.

### U4. AC2 — `hold[review]` sans tâche ne crée pas de travail

- **Files.** `crates/mika-agent/src/server/verdict_handler.rs`,
  `crates/mika-agent/src/webhook_dispatch.rs`,
  `crates/mika-agent/src/agent_loop/mod.rs`,
  `crates/mika-agent/src/skills/executor.rs`.
- **Tests.** Le passthrough sans tâche commence par le marqueur et le prédicat
  le reconnaît ; `effective_disabled_tools` retire `create_task` sur ce message ;
  `validate_dispatch_readiness` refuse sur ce message (entrées `dev-pilot` et
  `dev-groom`). Contrôles négatifs : le prédicat est faux sur l'enrichissement
  `block[ac]` et sur un `[GitHub] PR review` brut ; `effective_disabled_tools`
  inchangé sur un message `block[ac]` ; la porte ne refuse pas un
  `originating_message` `block[ac]`.

### U5. PR

- Section « Effet de bord » (R4) dans le corps de PR, sans détail de session.

## Verification Contract

- `cargo test -p mika-common forge_identity`
- `cargo test -p mika-agent` sur les modules touchés (`verdict`, `verdict_handler`,
  `evidence::guards`, `builtin_handlers`, `webhook_dispatch`, `agent_loop`,
  `executor`, `canonical_tokens`, `pr_merge_with_gate`)
- `cargo clippy -p mika-agent -p mika-common --all-targets -- -D warnings`
- `cargo fmt --check`
- Un seul build cargo à la fois ; `cargo clean` en fin de session.

## Definition of Done

- R1–R4 tenus, chaque garde avec contrôle positif et négatif dans les tests.
- Aucun littéral `mika-platform-qa` recopié hors `forge_identity` dans le code
  de production ajouté.
- Aucune trace de session, d'horodatage d'incident ou de mode opératoire dans le
  diff ni la PR.
- Pas de code d'essai abandonné dans le diff.
- PR ouverte avec `Closes #2667`, merge laissé à Vincent.

## Acceptance criteria

- [ ] **AC1. L'identité dispatcher ne peut pas poster de verdict de revue.** Refus pré-subprocess dans `run_gh` (même famille que mika#2237 et mika#2573) de `gh pr review` portant `--approve` ou un corps contenant une ligne `VERDICT:`, quand l'agent courant est le dispatcher. Corps de refus sobre, qui nomme la règle sans donner de gabarit de contournement. Tests : refus sur `--approve`, refus sur `--comment` avec `VERDICT:` ; contrôle négatif : `--comment` sans verdict autorisé ; contrôle négatif : l'agent de revue n'est pas touché.
- [ ] **AC2. Un `hold[review]` sans tâche active ne crée pas de travail.** Le passthrough de `verdict_handler` (« no active in_progress task found ») ne se traduit ni en création de tâche ni en dispatch dans ce tour. Il rend la main à l'opérateur et le dit. Tests : sur ce passthrough, `create_task` et `run_claude_pilot{,_groom}` sont refusés ; contrôle négatif : un `block[ac]` avec une tâche active dispatche comme avant.
- [ ] **AC3. Côté lecture, la porte ne merge que sur un verdict de l'identité de revue.** Dans le bras `pass` de `verdict_handler`, `run_gh_merge` n'est atteint que si l'auteur de la revue est `REVIEWER_FORGE_LOGIN` (`mika_common::forge_identity`, lecteur unique, aucun littéral recopié). Tout autre auteur ⇒ `Passthrough` nommé, ligne d'audit. Tests : `pass` + `approved` de `mika-platform-dev` sur une PR MECHANICAL ⇒ aucun merge (vu rouge sur le code actuel) ; contrôle positif : le même événement de `mika-platform-qa` merge.
- [ ] **AC4. Effet de bord à nommer, pas à corriger ici.** Le tour fautif a écrit dans la mémoire de l'agent dispatcher un état de revue faux. La PR le signale ; le nettoyage est un geste opérateur.

## Assumptions

- `ReviewEvent.reviewer` est le login GitHub de l'auteur de la revue, tel que le
  gateway le formate (`by @<login>`), vérifié dans `mika-gateway/src/github.rs`.
- `run_claude_pilot` et `run_claude_pilot_groom` passent tous deux par
  `validate_dispatch_readiness` avec `ctx.originating_message` — à confirmer au
  premier pas de U4 ; s'il en manquait un, U4 l'ajoute à la porte.
- AC3 « le même événement de `mika-platform-qa` merge » est prouvé jusqu'à
  l'entrée du chemin de merge (`handle_pass_verdict`), sans appel GitHub réel
  (KTD4).
