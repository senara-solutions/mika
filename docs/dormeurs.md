# Registre des dormeurs

Un **dormeur** est un travail réellement dû dont la condition d'exécution n'est pas remplie
aujourd'hui. Il ne se fait pas maintenant, mais il ne disparaît pas : il vit ici, avec sa
**condition de réveil datée et vérifiable**.

## Pourquoi ce fichier existe

La politique ratifiée le 2026-09-01 dit qu'un travail encore dû ne se ferme pas — fermer
fabrique un zéro qui cache, et le compte doit dire ce qui reste à faire. Elle dit aussi que
**le zéro se mesure sur les actionnables**.

Le 2026-09-03, l'opérateur a tranché que la visibilité **change de support** : le registre
versionné remplace le ticket ouvert. Rien ne se perd — au contraire, les conditions de réveil
sont désormais lisibles en un seul endroit au lieu d'être dispersées dans onze corps de
tickets — et le compte d'issues cesse de mélanger « ce qui reste à faire » avec « ce qui
attend le monde extérieur ».

**Ce fichier n'est donc pas un cimetière. C'est la file d'attente, rendue lisible.**

## Contrat d'une entrée

Une condition de réveil est valable quand un lecteur peut dire, **sans contexte**, si elle
est remplie. « quand `git rev-list --count A..B` rend 0 », « quand mika#2141 est mergé »,
« le 2026-09-10 » : oui. « plus tard », « si ça revient », « quand on aura le temps » : non —
ces trois-là sont des fermetures déguisées en attentes.

**Réveil.** Quand la condition est remplie, rouvrir le ticket GitHub cité (il conserve tout
son historique) et retirer la ligne d'ici.

## Registre

| ticket | sujet | condition de réveil |
|---|---|---|
| [#1403](https://github.com/senara-solutions/mika/issues/1403) | monitoring filtré événementiel pour les agents orchestrateurs | `git rev-list --count origin/feat/1403/gateway-agent-core-event-driven-filtered..origin/main` rend **0** |
| [#1619](https://github.com/senara-solutions/mika/issues/1619) | build + push ECR de l'image `mika-agent` au merge sur `main` | les **trois** ensemble : `gh secret list --repo senara-solutions/mika` contient `ECR_PUSH_ROLE_ARN` et le rôle IAM existe (mika-cloud#220) ; **mika#2143** livré ; le workflow réactivé et **deux merges consécutifs** réussis |
| [#1651](https://github.com/senara-solutions/mika/issues/1651) | couche d'intention entre le match par mot-clé et la porte d'outils requis | `git rev-list --count origin/design/1651/skills-intent-layer-between-keyword..origin/main` rend **0** |
| [#1680](https://github.com/senara-solutions/mika/issues/1680) | glyphes cassés dans le résumé de dispatch webhook (TUI mika-dev) | `git rev-list --count origin/fix/1680/mika-dev-tui-broken-glyph-rendering-in..origin/main` rend **0** |
| [#1694](https://github.com/senara-solutions/mika/issues/1694) | dette de worktrees et de branches — audit et nettoyage automatisés | une branche `origin/*/1694/*` existe et porte un plan commité |
| [#1699](https://github.com/senara-solutions/mika/issues/1699) | scénario de désambiguïsation pré-enregistré (glm-5.2 vs sonnet-4-6) | `git rev-list --count origin/feat/1699/calibration-permission-policy-pre..origin/main` rend **0** |
| [#1913](https://github.com/senara-solutions/mika/issues/1913) | compatibilité Langfuse v4 (endpoint OTLP + format d'auth) | **le 2026-11-16** au plus tard — échéance imposée par l'amont, à traiter avant |
| [#2119](https://github.com/senara-solutions/mika/issues/2119) | un tenant cloud ne peut lire aucune page hors des quatre hôtes gouv.fr | `git merge-base --is-ancestor fb2f01a4 <sha de l'image déployée>` rend **vrai** — c'est-à-dire quand un tenant exécute une image postérieure à `fetch_url` (bloqué derrière mika-cloud#216) |
| [#2139](https://github.com/senara-solutions/mika/issues/2139) | migration de la famille `opentelemetry` d'un seul bloc (0.31 → 0.32) | quand `opentelemetry`, `opentelemetry_sdk` et `opentelemetry-otlp` ont une version **mutuellement alignée** publiée sur crates.io |
| [#2150](https://github.com/senara-solutions/mika/issues/2150) | vérifier que le sweep des lignes fantômes tend vers zéro (AC6 de #1934) | **le 2026-09-10 ou après**, soit sept jours pleins après le déploiement du correctif de #1934 |
| [#1812](https://github.com/senara-solutions/mika/issues/1812) | SearXNG auto-hébergé comme chemin d'escalade pour la recherche sous contrôle d'egress (design-only) | quand le **trigger E6** de mika#1806 est activé — c'est-à-dire quand une décision opérateur ouvre la contingence « backend de recherche sous contrôle d'egress ». Aucun build avant. |

## Ce qui n'entre pas ici

- Un ticket dont le travail est **en cours** : il reste ouvert. Un correctif groomé dont le
  dispatch est bloqué par un défaut de l'alimenteur n'est pas un dormeur — c'est du travail
  vivant empêché, et c'est le défaut qu'il faut fermer, pas le ticket.
- Un ticket sans condition vérifiable. S'il n'en a pas, il faut soit la trouver, soit
  admettre que le travail n'est pas réellement dû.
- Un ticket qu'on préfère ne pas faire. Celui-là se ferme sur son propre tracker, avec sa
  raison, et n'a rien à faire dans une file d'attente.
