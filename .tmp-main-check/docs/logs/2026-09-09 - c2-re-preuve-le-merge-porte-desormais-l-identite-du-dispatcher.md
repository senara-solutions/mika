# 2026-09-09 — C2 re-preuve : le merge porte désormais l'identité du dispatcher

## Ce que cette PR est

Cette PR **est** le véhicule de la re-preuve de C2. Elle est de la même classe que
la PR du premier merge autonome — un journal `docs/logs/`, 100 % mécanique, que le
forge-gate laisse fermer seul. Si elle se ferme sous **`mergedBy = mika-platform-dev`**
(le dispatcher) et non `mika-platform-qa` (le relecteur), la re-preuve est acquise :
l'événement à observer est le `mergedBy` de cette PR une fois mergée.

## Le contexte — C2 acquis, mais la signature était fausse

Le 2026-09-08 à 16:27:56Z, le premier merge sans main humaine a été certifié
autonome : PR **mika#2244** (un journal C2), `human_gate_events=0`,
`merge_tool_calls=1`, review APPROVED. C2, au sens strict, était atteint.

Mais la signature portait un défaut : **`mergedBy = mika-platform-qa`** — le
merge avait été posé sous l'identité du **relecteur**, pas du dispatcher. Cause
mesurée (tool_calls + audit_events) : `ci_success_handler` faisait feu chez les
**deux** agents (mika-dev ET mika-qa), et la course faisait partir le merge sous
le PAT de mika-qa (chaîne PAT-first, classe env-shadow de mika#2218). Juge-et-partie :
qui approuve ne doit pas fermer sa propre décision.

## Le fix — « signal, pas acteur » (mika#2248 → PR#2258)

`ci_success_handler` ne merge plus. Il marque la PR **merge-ready** (un signal),
et **mika-dev seul** consomme ce signal et merge avec **son** token. Le relecteur
(mika-qa) n'atteint plus jamais le chemin de merge — une garde le refuse dans
`pr_merge_with_gate` avant tout appel `gh`. La politique « qui peut merger » vit
désormais dans un module partagé (`mika-common::forge_identity`), source unique
lue par le gateway et l'agent. Séparation structurelle des pouvoirs, pas
conventionnelle.

Le fix touche `pr_merge_with_gate.rs` / `ci_success_handler.rs` — zone
**décision-core**, sous CODEOWNERS @samidarko. Sa PR (#2258) a donc été gatée
humaine par conception, et mergée par Vincent le 2026-09-09 (le gate ne se
modifie pas lui-même en autonome — c'est la borne, pas un bug).

## L'histoire de la livraison — une collision, et sa récupération

Le pilote dispatché sur #2248 a calé (4e SDK-stall du jour, lignée #1901). Le fix
a été porté par un spawn hors-bwrap, qui évite la classe de stall. En cours de
route, la boucle a re-dispatché #2248 (via `stuck_ready_reconcile`, la PR portait
encore `ready`) sur **le même worktree** : le checkout frais du pilote loop a reset
l'arbre et effacé le travail non-commité du spawn. Récupération sans re-spawn :
interruption du tour desyncé, injection d'un correctif, et ré-application **avec
commits incrémentaux** — de sorte qu'un reset futur ne puisse plus tout effacer.
Leçon gravée : un travail non-commité sur un worktree que la boucle peut cibler
n'est protégé que par le commit ; et un ticket doit être `blocked` (pas seulement
`ready` retiré) pour que le feeder ne le re-promeuve pas.

## Ce que la re-preuve certifie

Que la séparation review/merge est désormais **vraie dans la mécanique**, pas
seulement dans l'intention : le dispatcher merge, le relecteur ne peut pas. Le
`mergedBy` de cette PR, une fois mergée en autonome, en est la mesure.
