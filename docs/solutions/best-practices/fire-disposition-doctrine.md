---
module: dev-groom
tags: [doctrine, scope-bind, detector, fire-disposition, architect-gate]
problem_type: scope-underspecification
category: best-practices
date: 2026-06-28
ticket: mika#1574
---

# Fire-Disposition Doctrine

## Problem

When a dispatch plan includes a detector-class deliverable (test, assertion, lint, invariant, validation), the plan must specify what happens when the detector fires on **existing** data — pre-existing violations that the new detector now surfaces. Without this specification, the implementing pilot either breaks CI (the detector fires and the test suite goes red) or makes an undirected scope decision (silently allowing the violation).

## Founding incident

mika#1326 → mika#1569 → mika#1573: the `verify-bundled-skills` invariant check (mika#1575, the pre-merge structural counterpart to mika#1326 AC2; binary `verify-bundled-skills`, source `crates/mika-agent/src/bin/verify_bundled_skills.rs`) caught a benign cross-skill `gh_read` collision on existing bundled-skill data. The scope-bind said "additions-only, don't mutate existing dispatch paths" but did not name the fire-disposition for when the detector caught existing violations. The pilot's strict interpretation produced a failing test.

## Doctrine

Every plan with a detector-class deliverable MUST include a `## Fire-Disposition` section naming one of three canonical options:

### Option (a): Named allowlist exception (default)

The detector enforces for new cases. Each existing violation gets a grep-visible named exception with:
1. **Specific data name** — the exact entity/path/value triggering the exception, not a blanket allowance
2. **Follow-up tracker reference** — a filed issue to fix the underlying violation
3. **Self-cleaning assertion** — the exception entry itself has a test that fires when the follow-up resolves and the exception becomes stale ("remove this entry")

When the allowlist is structural (a data const + test logic), scope it inside `#[cfg(test)] mod tests` so the production loader cannot consult it at runtime.

### Option (b): Land disabled

The detector lands with `#[ignore]`, `#[cfg(skip)]`, or equivalent, plus a tracked follow-up to enable it. Use only when the existing violation is itself dangerous to leave un-flagged and the detector's CI-red state would mask the danger.

### Option (c): Halt-and-surface

The implementation stops and surfaces to the operator for scoping. Use only when the existing violation's resolution shape is itself the operator-scoping question — the plan cannot pre-decide.

## Structural enforcement

The fire-disposition rule is enforced by the **Fire-Disposition Gate** in `mika-arch-groom-ticket` (first-pass) and `mika-arch-second-review` (second-pass). The gate returns ITERATE (first pass) or ESCALATE (second pass) when a plan with detector deliverables lacks the `## Fire-Disposition` section. This gate is the third architect gate, alongside the Unresolved-Decision Gate (mika#1244) and the Acceptance-Criteria Gate (mika#1559).

## Site de production (mika#2306)

Cette doctrine a décrit la règle et son gate pendant près de trois mois sans jamais dire **qui écrit la section**. Le producteur des plans — `/ce:plan`, plugin tiers du marketplace `compound-engineering` — ignore mika#1574 ; aucune des trois commandes de groom ne prescrivait la section (`grep -n "Fire-Disposition" .claude/commands/*.md` rendait zéro) ; et `_iterate_groom_loop` n'a qu'un seul ITERATE. Un plan neuf livrant un détecteur arrivait donc devant l'architecte sans la section, brûlait l'unique itération sur un motif purement formel, puis ESCALATE au second passage où le gate est sans recours — et la boucle ne dispatchait jamais l'implémentation.

C'est la configuration exacte que le Acceptance-Criteria Gate décrit déjà mot pour mot pour sa section sœur : *« Grooming is the surface we control between the third-party producer and our validator. »* `## Acceptance criteria` a reçu ce traitement (mika#1600/#1627) ; `## Fire-Disposition` ne l'avait jamais reçu.

**Deux sites, dans `skills/bundled/_shared/dispatch-lib.sh` :**

1. **Prescription** — `_FIRE_DISPOSITION_RULE`, injectée dans le `PROMPT` de chaque dispatch `dev-groom` (et d'aucun autre : un pilote d'implémentation n'écrit pas de plan). Elle nomme la section, ses trois options canoniques ci-dessus, et la règle N/A — la conditionnalité fait partie de la prescription, sans quoi un groomeur ajouterait la section à tout plan. La règle vit là plutôt que dans une commande de groom parce que les trois commandes vivent dans `senara-solutions/mika-platform` et sont semées dans le worktree par `_seed_worktree_slash_commands` (mika#1415) : ce `PROMPT` est le seul canal que le dépôt `mika` contrôle. Même raisonnement, au même endroit du fichier, que `_PR_BODY_CONTAINMENT_RULE` (mika#2211).

2. **Rattrapage** — `_fd_retry_if_section_still_missing`, greffé sur la branche de succès de `_launch_revise_pilot`. Le critère de convergence du revise est « le contenu a changé », jamais « le finding a été traité » : un revise qui corrige une virgule sans ajouter la section réclamée était, pour la boucle, indistinguable d'un revise réussi. La garde est une conjonction de deux `grep` — les findings de **première passe** contiennent `Fire-Disposition`, et le plan révisé ne porte toujours pas `^## Fire-Disposition` — et relance le pilote de revise **une seule fois** avec un finding ciblé. Elle ne refuse jamais rien : elle réessaie, puis laisse passer en journalisant, parce qu'un échec dur y aurait déplacé l'ESCALATE d'une porte au lieu de le lever.

**Ce que ces deux sites ne couvrent pas, et qui est nommé plutôt que simulé :** la prescription dans `/mika-groom-plan-only` (étape `5c`, jumelle de la `5b` que mika#1600 a posée pour `## Acceptance criteria`), la même dans `/mika-groom-ticket`, et une consigne à `/mika-revise-plan` pour que la branche ITERATE sache écrire la section qu'on lui réclame. Ces trois fichiers vivent dans `senara-solutions/mika-platform` — **ticket de suivi**. L'ordre de livraison n'est pas contraint : les deux sites ci-dessus sont additifs et fail-safe (l'un ajoute du texte à un prompt, l'autre ajoute une tentative), donc aucun ne peut casser un grooming qui marchait.

**Surfaces opérateur** (`stderr` de `dispatch-lib`, collecté dans le log pilote) :

- `fire_disposition_revise_retried` — la garde a relancé le revise. **Régime attendu : rare.** Chaque ligne est une itération architecte que la boucle n'a pas gaspillée.
- `fire_disposition_still_missing_after_retry` — la seconde tentative n'a pas produit la section. **Régime attendu : zéro.** Une occurrence soutenue dit que le pilote de revise ne sait pas écrire la section, donc que le correctif à faire est le suivi `mika-platform` (`/mika-revise-plan`), **pas** un troisième essai dans `dispatch-lib`.

Les deux sont de l'**observabilité pure** : consommés par l'opérateur et par l'analyse de logs (mika#2205), relus par aucune branche de `dispatch-lib`, sans effet sur le flot de la boucle. Promouvoir l'un d'eux en signal de contrôle serait un changement de contrat, pas un réglage. Et l'absence de la première ligne n'est jamais à elle seule une preuve de succès : elle se lit aussi « aucun groom n'a tourné » — lire le volume de dispatches `dev-groom` sur la même fenêtre avant de conclure.

**Ce qui a été refusé, et pourquoi** : une garde déterministe dans `verify-pipeline.sh`, le pendant de celle que mika#1600 a posée pour `## Acceptance criteria`. Trois motifs. (1) Elle arriverait **après** l'autorité qu'elle contredirait — le grooming n'ouvre pas de PR, donc la CI ne voit le plan qu'à la PR d'implémentation, après un `GROOMED` architecte ; elle ferait rougir une PR dont le plan a déjà été déclaré conforme. Le contraste avec AC est net : AC est inconditionnelle, donc CI et architecte ne peuvent pas diverger. (2) La conditionnalité n'est pas exprimable en bash — « ce plan livre-t-il un détecteur ? » est le jugement sémantique que mika#1574 confie à un LLM, et rendre la section inconditionnelle contredirait frontalement le « gate is N/A » ci-dessus. (3) Elle déplacerait le blocage sans le lever : troquer un ESCALATE architecte contre un rouge CI laisse le ticket immobile, une porte plus loin. Si une telle garde devient souhaitable, sa précondition est une **mesure** — la proportion de plans GROOMED portant un détecteur sans la section — qui n'existe pas aujourd'hui.

## Provenance

- `feedback_scope_bind_must_name_fire_disposition` (orchestrator-CC memory, 2026-06-26)
- mika#1574 (this ticket)
- Mika Prime bearing-read 2026-06-26
- mika#2306 (2026-09-21) — § Site de production : la prescription et le rattrapage
- mika#1600 / mika#1627 — le précédent `## Acceptance criteria` : même producteur tiers, même trou
- mika#2211 — `_PR_BODY_CONTAINMENT_RULE` : le précédent d'une règle injectée par `dispatch-lib` parce que le fichier de commande est hors de portée
