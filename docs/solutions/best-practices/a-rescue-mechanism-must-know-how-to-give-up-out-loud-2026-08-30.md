---
module: auto_pull
tags: [loop-substrate, circuit-breaker, refusal, observability, counter-semantics, dispatch]
problem_type: silent-failure
category: best-practices
---

# Un mécanisme de sauvetage doit savoir abandonner — et le dire

## Problem

`mika#1901` a reçu le label `ready` **16 fois en ~19 h**, un toggle toutes les 70–90 minutes, par le reconciler stuck-ready de l'auto-pull (mika#1824). Zéro plan produit, zéro PR ouverte, branche identique à `main`. Chaque tour consommait le créneau `groom` qu'un ticket réellement groomable aurait pris.

Le seul moyen de s'en apercevoir a été de compter les événements du ticket à la main :

```bash
gh api "repos/senara-solutions/mika/issues/1901/events?per_page=100" \
  --jq '[.[] | select(.event=="labeled" and .label.name=="ready")] | length'
→ 16
```

Rien dans les logs ne disait « j'ai renoncé », parce que le mécanisme n'avait pas la notion de renoncer. Il réessayait, indéfiniment, et chaque tentative réussissait techniquement.

## Root cause

Trois défauts distincts, dont chacun aurait pu être écrit sans les deux autres.

### 1. Un compteur qui compte la mauvaise chose

Le circuit breaker existait (`CIRCUIT_BREAKER_THRESHOLD = 3` sur `auto_pull_stats.failure_count`) et faisait exactement son travail — qui n'était pas celui-là. `failure_count` signifie « l'appel `gh` a échoué ». Chaque rescue de #1901 **réussissait** côté API, donc `reset_auto_pull_failure` remettait le compteur à zéro à chaque tour.

Le piège est fin : le compteur d'échecs se remet à zéro sur exactement l'événement qu'un compteur de tentatives doit incrémenter. Les deux sémantiques sont opposées **au même point du code** :

```rust
// auto_pull.rs, boucle de rescue Phase 2
db.reset_auto_pull_failure(DEFAULT_REPO, n).await;   // ← efface la mémoire
db.increment_auto_pull_redrive(DEFAULT_REPO, n).await; // ← la garde (mika#2020)
```

**Généralisation.** Avant de réutiliser un compteur existant pour une nouvelle garde, demande : *sur quel événement est-il remis à zéro, et est-ce le même événement que je veux compter ?* Si oui, une colonne dédiée n'est pas du zèle — c'est la seule implémentation correcte. Le test qui verrouille ça (`test_auto_pull_redrive_survives_failure_counter_reset`) vaut plus que les autres réunis : il prouve que le reset de l'un ne touche pas l'autre.

### 2. Un seuil d'alarme formulé globalement, aveugle à la concentration

La documentation du mécanisme énonçait son propre garde-fou (`CLAUDE.md`) :

> *Steady-state expectation: ≤5 rescues/day; >20/day indicates the dispatch-layer primary fix is still needed.*

Seize reprises sur **un seul ticket** en 19 h ne déclenchent pas un seuil de 20/jour tous tickets confondus. Le seuil était bon et l'angle mort était sa granularité : une alarme agrégée ne voit jamais une boucle locale.

**Généralisation.** Quand une alarme est agrégée, se demander à quoi ressemble sa pire violation *par entité*. Si un seul acteur peut consommer indéfiniment la ressource sans franchir le seuil global, il manque un compteur par entité.

### 3. La forme d'un artefact confondue avec son appartenance

`is_groomed()` vérifiait la présence de la sous-chaîne `> - **Plan:** \`docs/plans/` — la *forme* du callout — jamais à qui le plan appartient. Le corps de mika#1887 en porte la trace : son callout désignait `docs/plans/2026-08-21-002-fix-1933-…-plan.md`. Un vrai fichier, un vrai plan, l'intention d'un **autre ticket**. Le pilote l'a ouvert, l'a lu, et n'avait aucun moyen de savoir.

C'est la classe la plus chère, et elle ordonne la sévérité des refus :

> **Un ticket sans plan est moins dangereux qu'un ticket avec le mauvais plan.** Le premier produit une boucle stérile — coûteuse, visible en creux, réparable par un groom. Le second produit du travail confiant sur une intention devinée : une PR qui compile, qui passe la revue, et qui répond à une question que personne n'a posée.

## Solution

### Deux refus, deux vitesses

| Situation | Geste | Pourquoi |
|---|---|---|
| Plan attribué à une autre issue | Abandon **immédiat**, zéro re-drive | `is_groomed()` répond `true`, donc dev-groom saute le grooming et dispatche droit à l'implémentation du mauvais plan. Le re-drive est *activement nuisible*. |
| Toute autre impasse, dont l'absence totale de callout | Re-drive jusqu'à `MIKA_AUTO_PULL_MAX_REDRIVES` (défaut 3), puis abandon | `ready` sur un ticket non groomé est l'état d'entrée **nominal** du pipeline : c'est ce label qui déclenche dev-groom. Le re-drive est la voie qui lui redonne sa chance. |

Le piège évité ici mérite d'être nommé : la tentation était de refuser tout ticket non groomé en Phase 2, par symétrie avec Phase 0 et Phase 1. Ç'aurait cassé le flux normal. **Un filtre correct à un endroit du pipeline peut être une régression à un autre** — la même condition n'a pas le même sens avant et après le déclencheur.

### Fail-open sur l'ambiguïté, fail-closed sur la contradiction

Le nom canonique d'un plan est `<YYYY-MM-DD>-<seq>-<type>-<issue>-<slug>-plan.md`, et le créneau d'issue est lu **ancré à cette position**, jamais cherché librement :

```rust
Regex::new(r"^\d{4}-\d{2}-\d{2}-\d{3,4}-[a-z]+-(\d+)-")
```

Trois verdicts : `Owned`, `OwnedByOther(n)` — le seul qui refuse — et `Unattributable` pour les noms historiques sans créneau. On n'accuse que sur une preuve positive de mauvaise attribution.

L'ancrage est le point. mika#2038 a documenté le dégât inverse : un glob permissif sur `*-2026-*` matchait `rustsec-2026-0097` et envoyait un pilote sur un plan d'avril. Un motif non ancré ici commettrait la faute symétrique — refuser un ticket parce qu'un nombre de son slug ressemble à un numéro d'issue.

Vérification empirique sur les 776 plans réels de `docs/plans/` : **265 attribués correctement, 511 en fail-open, zéro faux positif.**

Le fail-open n'est pas de la timidité, c'est une contrainte de cohérence : `_find_issue_plan` de dev-groom accepte aussi un plan via un marqueur `**Issue:**` dans les 20 premières lignes du contenu, que l'auto-pull ne peut pas lire sans une I/O par ticket et par tick. Une garde plus stricte que le consommateur refuserait du travail légitime.

### Un abandon est un geste, pas une absence de geste

```
appliquer `operator-review`  ← en premier, et son échec AVORTE l'abandon
retirer `ready`
commenter le ticket : le ticket, la raison, le remède
estamper `redrive_abandoned_at`
warn! + log_audit_event
```

**L'ordre est load-bearing, et la revue l'a corrigé.** La première version retirait `ready` d'abord. Si `operator-review` échouait ensuite, le ticket se retrouvait sans aucun des deux labels mais avec l'estampille — exactement l'état que Phase 2 lit comme le geste de remise en jeu de l'opérateur. Le budget repartait à zéro, et avec lui une boucle de N re-drives. Poser le label d'abord rend l'échec **convergent** : rien n'a bougé, le compteur est toujours au-dessus du budget, le tick suivant réessaie.

**Généralisation.** Dans une séquence de gestes externes non transactionnelle, ordonner par *ce qui arrête la chose*, pas par *ce qui la nettoie*. Puis parcourir chaque échec partiel et demander : cet état intermédiaire est-il lu ailleurs comme un état signifiant ?

Le commentaire GitHub est le seul canal qui atteint un humain sans grep. Un `debug!` avec `reason=…` n'est pas un refus — c'est un refus que personne ne lit. Le libellé est testé (`reason()` / `remedy()` sont des fonctions pures) précisément parce qu'**un refus dont le message n'est pas testé se dégrade en silence à la première réécriture**.

### La remise en jeu est le geste que l'opérateur fait déjà

Pas un nouveau label, pas une commande : retirer `operator-review`. Le tick suivant voit l'estampille sans le label, remet le compteur à zéro, et le ticket redevient éligible. La colonne `redrive_abandoned_at` existe uniquement pour séparer deux états que l'absence du label ne distingue pas — « le budget vient d'être épuisé, le label n'est pas encore posé » et « le budget a été épuisé plus tôt, l'opérateur a depuis retiré le label ».

### Une exclusion structurelle doit l'être partout

La revue a trouvé la fuite : `is_feeder_excluded` (`blocked`/`operator-review`) était honoré par Phase 0 mais **pas** par Phase 1. L'asymétrie était inoffensive avant ; elle est devenue load-bearing dès que l'abandon a reposé sur ce label. Un ticket groomé qu'on venait de confier à l'opérateur pouvait recevoir `ready` au tick creux suivant, et le webhook le re-dispatchait.

**Généralisation.** Quand une correction fait reposer une garantie sur un signal existant, greper **tous** les points qui devraient l'honorer. Une asymétrie tolérable devient un trou dès qu'on s'appuie dessus. Voir `feedback_structural_gate_audit_grep_all_callsites`.

## Verification

```bash
cargo test -p mika-agent auto_pull        # 87 tests, dont les 7 de plan_ownership
cargo test -p mika-agent schemas_converge # convergence v1 / incrémental après v49→v50
cargo clippy --workspace --all-targets -- -D warnings
```

Signaux opérateur après déploiement :

| Grep | Sens |
|---|---|
| `auto_pull_redrive_abandoned` | Un ticket a été rendu à l'opérateur. `reason=plan_owned_by_other_issue` ou `redrive_budget_exhausted`. |
| `auto_pull_plan_ownership_mismatch` | Phase 0/1 a écarté un candidat dont le plan appartient à une autre issue. |
| `auto_pull_redrive_reentry` | Un opérateur a remis un ticket abandonné en jeu. |

Un pic de `auto_pull_plan_ownership_mismatch` sur plusieurs tickets pointe vers le producteur de callouts (dispatch-lib), pas vers cette garde.

## Related

- mika#1824 — le reconciler stuck-ready que cette garde borne.
- mika#1363 — le circuit breaker dont la sémantique est délibérément préservée.
- mika#2038 / `docs/plans/2026-08-29-002-fix-2038-…-plan.md` — même classe côté dispatch-lib : réfutation d'en-tête au palier 1. Ce travail en est le pendant côté auto-pull.
- mika#1563 — `identical_diff_circuit_breaker` : même intention (casser une boucle qui ne produit rien), autre surface.
- `feedback_dev_groom_find_issue_plan_filename_slot` — le contrat de `_find_issue_plan` avec lequel la garde reste cohérente.
- `feedback_prompt_enforcement_fragile` — pourquoi la borne est structurelle (compteur + label) et non une consigne.
