---
title: "fix(webhooks): pré-filtre déterministe, phase 2 — pr_review non actionnable sans tâche (mika#2675 AC1(c))"
type: fix
status: active
date: 2026-10-07
issue: 2675
---

# Pré-filtre webhook, phase 2 — classe (c)

## Pourquoi

Mesure fondatrice de mika#2675 (18 h, `llm_calls` × `audit_events`) : 23
sessions, 1,72 M tokens d'entrée, sur des `pr_review` sans tâche active dont le
verdict ne mène nulle part. `verdict_handler.rs` le sait déjà en code — il
journalise « no active task found for PR — passing through to LLM », notifie
l'opérateur d'un `hold[review]` sans tâche (mika#2667 AC2), refuse le `pass`
d'une identité qui n'est pas le relecteur (mika#2667 AC3), traite le verdict
illisible en `hold` par défaut — puis rend quand même la main au modèle, qui
conclut « stale — no action ».

La phase 1 (#2676, 1bb2eaa7) a posé la porte `webhook_prefilter::skip_turn`
entre le dernier handler et `agent::run_agent`, avec la règle « texte intact ».
Cette règle exclut la classe (c) par construction : le `verdict_handler`
enrichit ou remplace TOUJOURS le texte sur ces chemins. La phase 2 étend le même
module ; elle ne crée pas de second mécanisme.

## Décisions

1. **Même porte, même placement.** Le `verdict_handler` garde tous ses effets
   (notification, audit, disjoncteur PR). Le filtre ne retire que le tour LLM.
2. **« Texte intact » relu pour (c) : intact APRÈS le verdict_handler.** On
   capture `req.text` juste après le `verdict_handler` ; pour (c), un handler
   ULTÉRIEUR qui toucherait le texte garde le tour. Le `verdict_handler`, lui,
   a déjà dit au modèle ce qu'il avait à dire, et ce n'est rien d'actionnable.
3. **« Sans tâche active » = `find_active_task_by_pr_url` rend `None`**
   (tout statut non terminal compte comme actif). Plus large que
   `find_task_for_verdict` (qui n'accepte que `in_progress`) : un faux
   « actif » coûte un tour, un faux « inactif » perdrait un événement. La
   recherche a lieu APRÈS les handlers, donc une tâche qu'un handler aurait
   créée est vue.
4. **Une revue sans ligne `VERDICT:` n'est pas un verdict.** Elle n'entre pas
   dans la classe (c) : c'est la forme d'une consigne écrite en revue, et
   l'identité GitHub de l'opérateur est partagée (AC2, « un commentaire n'est
   jamais filtré »). Seule une ligne `VERDICT:` PRÉSENTE ouvre la classe.
5. **Les quatre termes de « non actionnable »**, dans cet ordre, chacun
   suffisant :
   - `hold[review]` — déjà suivi : `hold_review_without_task` a notifié
     l'opérateur et écrit sa ligne d'audit. Un `hold[x]` inconnu n'est suivi
     par personne : il garde son tour.
   - auteur ≠ relecteur QA (`forge_identity::is_reviewer_forge_login`).
   - ligne `VERDICT:` présente mais illisible (`Verdict::Missing`).
   - PR `CLOSED` ou `MERGED` côté forge — consulté seulement si aucun des trois
     premiers termes ne tient, pour ne payer `gh` qu'au besoin.
6. **AC3 côté forge.** Pas de jeton, `gh` en échec, état inconnu : le tour a
   lieu. `OPEN` : le tour a lieu.
7. **Couture d'état injectée.** Un trait `PrefilterState` (tâche par branche,
   tâche par PR, état de PR) remplace la fermeture unique de la phase 1 ; la
   production l'implémente sur `AsyncDatabase` + `gh pr view --json state`,
   les tests sur un bouchon. Aucun appel GitHub réel en test.
8. **Audit** : une classe par terme (`verdict_hold_tracked`,
   `verdict_non_reviewer`, `verdict_unreadable`, `verdict_pr_closed`), cible
   `pr:<repo>#<n>`. AC5 compte par terme.

## Estimation

Code + tests ≈ 380 lignes (module ≈ 120, tests unitaires ≈ 150, `handlers.rs`
≈ 20, tests d'intégration ≈ 90). Marge ×2,5 : ≈ 950 < 1000 — pas de découpage.
Docs (plan, compound) en sus.

## Fichiers

- `crates/mika-agent/src/server/webhook_prefilter.rs` — variante, candidat,
  décision, trait d'état, état de forge.
- `crates/mika-agent/src/server/handlers.rs` — capture du texte après le
  `verdict_handler`, jeton passé à la couture.
- `crates/mika-agent/src/server/mod.rs` — tests d'intégration sur
  `run_agent_for_message` (compteur du LLM bouchonné).
- `verdict_handler.rs` : **inchangé**. Il reste le point d'entrée lu ; son
  comportement ne bouge pas.

## Definition of Done

- Classe (c) écartée, terme par terme, avec ligne d'audit et zéro appel LLM.
- Mutations terme par terme vues rouges, consignées dans le corps de la PR.
- `cargo test -p mika-agent` et `cargo test -p mika-common mika2398` verts.
- PR `Refs #2675` (la phase 3 reste), mention CODEOWNERS / merge opérateur.

## Acceptance criteria

- [ ] AC1(c) — un `pr_review` sans tâche active pour la PR, dont le verdict
  n'est pas actionnable (`hold[review]` déjà suivi, auteur ≠ relecteur QA,
  ligne VERDICT illisible, PR fermée ou mergée), ne produit aucun tour LLM et
  écrit une ligne d'audit nommée (classe et cible).
- [ ] AC2 — un `VERDICT: pass` du relecteur QA sur une PR avec tâche active
  passe toujours ; un `block[ac]` du relecteur sur une PR ouverte sans tâche
  passe ; une revue sans ligne VERDICT passe ; un commentaire n'est jamais
  filtré.
- [ ] AC3 — état de tâche illisible ou état de PR illisible (pas de jeton,
  `gh` en échec, valeur inconnue) : le tour a lieu.
- [ ] AC4 — `MIKA_WEBHOOK_PREFILTER` désarmé : rien n'est filtré.
- [ ] Chaque terme a sa mutation vue rouge.

## Hors périmètre

Classe (b) (phase 3). Router vers un seul agent.
