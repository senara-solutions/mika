---
title: Une absence mesurée dans le bac à sable est une propriété du bac à sable, et l'appelant réel nomme la primitive
date: 2026-09-28
category: best-practices
module: packages/ui, dev-loop
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - "Un plan de halte-et-remontée déclare une précondition « non tenue » parce qu'un dépôt voisin ou un fil de commentaires est illisible depuis le pilote"
  - "Un ticket d'extraction de primitive UI porte une précondition « au moins un callsite réel lisible »"
  - "Le nom de la primitive vient du ticket (ex. `PasswordInput`) et non d'un appelant lu"
  - "Un re-dispatch rejoue les mêmes mesures et rend le même verdict « bloqué »"
related_components:
  - frontend
  - documentation
tags:
  - halte-et-remontee
  - precondition
  - pilot-sandbox
  - callsite-reel
  - nommage
  - luminescent-core
  - lc-2b
---

# Une absence mesurée dans le bac à sable est une propriété du bac à sable, et l'appelant réel nomme la primitive

## Context

mika#2562 (LC.2b, suite de mika#1801) devait extraire trois primitives dans
`@samidarko/ui` — le ticket les appelait `PasswordInput` / `SecretField`,
`AuthCard` / `CenteredCard` et `Stepper`. Leur population dans `mika` est nulle :
le seul consommateur est `mika-cloud`. Le ticket posait donc une précondition
explicite : **(g1)** une section de rulebook décrivant les patterns, **ou (g2)**
au moins un appelant réel lisible, « même par simple lecture d'un fichier de la
console ».

Trois dispatches successifs ont rendu le même verdict : précondition non tenue.
Le plan de halte mesurait g2 ainsi : « `/data/workspace/mika-platform/` ne porte
que `claude-pilot` et `mika` — `mika-cloud` absent de l'hôte ». Et `gh` n'était
authentifié dans aucun des trois bacs à sable, donc le pilote ne lisait que le
**corps** du ticket, jamais ses commentaires.

Les deux constats étaient vrais **du bac à sable** et faux **de l'hôte**. Le
[Pilot sandbox](../../../CONCEPTS.md) monte une liste blanche de chemins, pas la
racine de l'hôte ; `mika-cloud` existait sur l'hôte, simplement non monté.
L'orchestrateur a lu trois appelants réels de `mika-cloud` à `origin/main` (chemins relatifs à ce dépôt-là, pas à `mika`) —
`web/src/pages/onboarding/ApiKeysStep.tsx`, `web/src/pages/Login.tsx` /
`Signup.tsx`, et `StepProgress` dans `web/src/pages/Onboarding.tsx` — et les a
postés en commentaire de mika#2562. g2 était tenu ; le plan de halte était périmé.

## Guidance

**1. Une absence mesurée depuis le bac à sable est une propriété de la frontière,
pas du monde.** Quand un pilote conclut « X est absent » (un dépôt voisin, un
fil de commentaires, un credential), la phrase exacte est « X est absent **de ce
que je peux voir** ». Si la seule raison d'un blocage est une telle absence,
**aucun re-dispatch ne le lèvera** : le déblocage vient d'un geste extérieur
(l'orchestrateur lit et dépose). Le plan de halte l'avait tiré lui-même pour
`gh` (« Halte 1 ouverte, et structurellement ») ; il ne l'avait pas tiré pour
`mika-cloud`, qu'il avait écrit « absent de l'hôte ». Écrire la mesure avec son
point de vue (« absent du bac à sable ») aurait orienté la remontée directement
vers l'orchestrateur au lieu de trois dispatches.

**2. Un plan de halte doit porter sa propre condition de péremption, et le pilote
doit la suivre.** La Halte 1 du plan disait : *« si un commentaire opérateur tient
g1 ou g2 … alors §2.2 est périmé et ce plan doit être révisé plutôt que suivi »*.
Au dispatch suivant, le pilote (tâche mika-dev `b8417083`) a lu le commentaire, a constaté
que la condition était réalisée, et a **révisé** le plan (§0 ancrage, §5.1 axes
d'API dérivés des appelants, §7 périmètre livré) au lieu de rejouer la halte. La
halte est conservée dans le plan, marquée « RÉALISÉE » : supprimée, elle
cesserait d'expliquer pourquoi le déblocage devait venir du dehors.

**3. Le nom de la primitive vient de l'appelant lu, pas du ticket.** Le ticket
offrait `PasswordInput` en premier. L'appelant réel l'a réfuté : le seul champ
masqué de la console est une **clé API** Anthropic, et l'authentification passe
entièrement par Google OAuth — aucun écran ne porte de mot de passe.
`PasswordInput` aurait nommé une population nulle. La primitive s'appelle
`SecretField` (`packages/ui/src/components/SecretField.tsx`).

La même lecture a **dissous** la question d'API que le plan avait ouverte
(« `autocomplete` vaut-il `current-password` ou `new-password`, et si les deux,
est-ce une prop ou deux composants ? ») : sans mot de passe, il n'y a pas d'axe
login/signup. `autoComplete` reste une prop plate défaut `off`, la valeur de
l'appelant — pas une union discriminée, qui aurait été décorative (le piège que
LC.2 D1 avait nommé).

**4. Les axes d'API se lisent sur l'appelant, un par un.** Ce que les trois
callsites ont fixé, et qu'aucune conception a priori n'aurait deviné :

| primitive | axe fixé par l'appelant | conséquence |
|---|---|---|
| `SecretField` | bouton **texte** `Show` / `Hide`, erreur rendue par l'appelant sous le champ | pas d'icône ; `invalid` / `describedBy` sont porteurs (sinon l'erreur est hors de portée d'un lecteur d'écran) |
| `AuthCard` | `<Logo />` est un composant du consommateur ; contenu libre ; pied « Powered by… » sur `Login` seulement, hors carte | `logo` est un slot `ReactNode` ; le pied n'est pas dans la primitive |
| `Stepper` | étapes **dynamiques selon le tier** (`getStepIds(tier)`), étape finale filtrée de l'affichage, pilotée par l'état serveur | `current` est un **id**, jamais un index ; `<ol>` et pas `<nav>` (aucune étape cliquable) |

**5. Pas de règle d'enforcement sans population.** Aucune règle « hand-rolled
Stepper is a review fail » n'a été ajoutée à `packages/ui/CLAUDE.md` : dans `mika`
elle serait verte chaque jour sans rien regarder. La règle appartient au dépôt qui
porte la surface (LC.3, `mika-cloud`). Voir
[une garde que rien n'invoque se lit exactement comme un arbre propre](une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md).

## Why This Matters

Sans le point 1, la classe se reproduit : un pilote mesure une absence depuis sa
frontière, l'écrit comme un fait du monde, et chaque re-dispatch rejoue la même
mesure pour rendre le même verdict — trois fois ici. Le coût n'est pas le
re-dispatch, c'est que la remontée désigne le mauvais geste : elle demandait à
Vincent une section de rulebook (g1, décision §8), alors qu'un orchestrateur sur
l'hôte pouvait lever g2 en lisant trois fichiers.

Sans le point 3, l'extraction devient une invention sous un nom plausible :
`PasswordInput` aurait porté un axe `current-password` / `new-password` sans
aucun appelant, et le premier consommateur réel l'aurait trouvé faux — exactement
le défaut que LC.2 avait refusé de commettre en reportant ces trois primitives.

## When to Apply

- Avant d'écrire « absent » dans un plan de halte : préciser le point de vue
  (bac à sable ou hôte) et, si c'est le bac à sable, adresser la remontée à
  l'orchestrateur, pas au re-dispatch.
- Avant de suivre un plan de halte : lire les **commentaires** du ticket, pas
  seulement le corps (le pilote ne les reçoit pas si `gh` n'est pas authentifié).
- Avant de nommer une primitive extraite : nommer ce que l'appelant lu fait, même
  si le ticket propose un autre nom en premier.

## Examples

Formulation qui cache la frontière (plan de halte, avant) :

> B1′ (g2) : aucun callsite lisible — `mika-cloud` absent de l'hôte.

Formulation qui la montre, et route le déblocage :

> B1′ (g2) : aucun callsite lisible **depuis ce bac à sable** — `mika-cloud` n'y est
> pas monté et `gh` n'y est pas authentifié. Levable seulement de l'extérieur :
> l'orchestrateur lit un appelant sur l'hôte et le dépose en commentaire du ticket.

## Related

- mika#2562 — le ticket, ses commentaires (appelants de `mika-cloud`), la PR mika#2574
- mika#1801 — LC.2, qui a reporté ces trois primitives plutôt que de les deviner
- `docs/plans/2026-09-28-001-feat-2562-lc-2b-precondition-non-tenue-plan.md` — §0, §5.1, §7, Halte 1
- [A groomed plan is a shape contract, not a fact contract](groomed-plan-is-a-shape-contract-not-a-fact-contract-2026-08-27.md) — un plan groomé peut porter une prémisse fausse ; ici, une mesure vraie depuis un point de vue trop étroit
