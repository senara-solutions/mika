---
issue: 2246
type: fix
title: "L'onboarding est collant : check_onboarding dépend d'un seul appel-outil que le modèle doit émettre"
branch: fix/2246/onboarding-collant-user-summary-sentinel
status: groomed
---

> **Grooming (orchestrateur, 2026-09-08).** Cause mesurée par investigation read-only (tenant cloud MikaSenara). Plan par la main (les grooms-pilotes ne finalisent pas leur contrat ; cf #2131/#2238).

# Plan — #2246 : rendre le signal de fin d'onboarding robuste

## Cause racine (mesurée)

`check_onboarding()` (`crates/mika-agent/src/agent_loop/mod.rs:2952`) retourne `true` tant que la core-memory `user_summary` vaut le sentinel par défaut *"No information about the user yet."* (et **fail-open à `true`** si la lecture rend `None`). Tant que `true`, `prompt.rs:1132` injecte `onboarding_prompt()` (`:749`) à **chaque tour** (*"This is your first conversation…"*).

Sur le tenant cloud (glm-5.2), le sentinel est **toujours présent en DB** : le modèle appelle `store_fact` (il connaît « Vincent ») mais **pas — ou pas efficacement — `update_core_memory("user_summary")`**. Donc l'onboarding re-tire indéfiniment, re-demandant le nom à chaque message.

**Le défaut structurel :** faire dépendre l'état « onboardé » d'un **appel-outil que le modèle doit émettre de façon fiable**. Le prompt-only échoue au substrat (cf `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`) — un modèle non-frontier (glm-5.2) ne l'émet pas de façon fiable.

## Fix — un signal déterministe, indépendant du modèle

Introduire un **signal de fin d'onboarding qui ne dépend pas d'un `update_core_memory("user_summary")` émis par le modèle** :

- **D1 — `check_onboarding` élargi (OR de signaux robustes) :** retourner `false` (= onboardé) dès que **l'un** des signaux tient :
  1. `user_summary` != défaut (comportement actuel, conservé) ;
  2. **il existe au moins un person-fact stocké sur l'utilisateur** (le modèle appelle `store_fact` de façon fiable — mesuré : il l'a fait) ;
  3. (optionnel, borne dure) **≥ N tours utilisateur** ont eu lieu dans la conversation — au-delà, on n'est plus « au premier contact » quoi qu'ait fait le modèle.
- **D2 — fail-open corrigé :** `check_onboarding` fail-open actuellement à `true` (onboarding) si la lecture échoue. Pour un utilisateur qui a de l'historique, un échec de lecture ne doit pas le renvoyer à l'onboarding. Décision : si un signal d'historique existe (person-fact ou tours), fail vers `false`.
- **D3 (defense-in-depth, hors périmètre si trop large) :** poser un flag `onboarding_complete` explicite écrit **déterministiquement par le code** (pas par le modèle) à la fin du premier échange substantiel. À évaluer au grooming ; D1+D2 suffisent pour lever le symptôme.

## Acceptance Criteria

- **AC1** — Après un tour où le modèle a appelé `store_fact` sur l'utilisateur MAIS pas `update_core_memory("user_summary")`, `check_onboarding` retourne **`false`** (aujourd'hui : `true`). Test unitaire reproduisant l'état DB mesuré.
- **AC2** — Un utilisateur avec ≥1 person-fact (ou ≥N tours) n'est jamais re-onboardé ; `onboarding_prompt` n'est plus injecté.
- **AC3 (contrôle négatif)** — Un utilisateur réellement neuf (0 fact, 0 tour, user_summary=défaut) reçoit bien l'onboarding **une fois**.
- **AC4** — `check_onboarding` ne dépend plus **uniquement** d'un appel-outil que le modèle doit émettre (signal déterministe ajouté).

## Frères / hors périmètre
- #2245 (historique absent du payload) — défaut RUNTIME distinct, couplé mais actionnable séparément. Réparer #2246 seul ne restaure PAS la continuité tant que #2245 tient.
- #2247 (style glm-5.2) — cluster modèle, séparé.
- Choix du modèle glm-5.2 pour les tenants family — question opérateur (Vincent), hors périmètre code.

## Surface
- `crates/mika-agent/src/agent_loop/mod.rs` (`check_onboarding` :2952) — cœur du fix.
- Lecture : `db.get_core_memory`, l'API person-facts / count de tours (à confirmer à l'impl).
- Test unitaire reproduisant l'état DB mesuré (store_fact sans update_core_memory).
