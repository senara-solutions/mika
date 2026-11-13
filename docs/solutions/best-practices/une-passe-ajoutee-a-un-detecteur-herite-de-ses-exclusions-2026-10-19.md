---
title: "Une passe ajoutée à un détecteur hérite de ses exclusions, et un fixture vert doit être posé dans la population"
date: 2026-10-19
category: best-practices
module: scripts
problem_type: best_practice
component: lint
severity: medium
tags: [lint, detector, false-positive, fixture, anti-vacuity]
applies_when:
  - Ajouter une deuxième passe de détection à côté d'une passe existante dans un même détecteur
  - Écrire un fixture vert censé prouver la retenue d'une règle
  - Écrire une exclusion qui reconnaît un attribut par regex dans un texte de balise
---

# Une passe ajoutée à un détecteur hérite de ses exclusions

## Le contexte

mika#1799, vol 1. `scripts/check-cta-primitives.sh` refuse toute CTA peinte à
la main au lieu de passer par `<Button>`. Le détecteur avait une passe : un
token `bg-*` de couleur pleine sur un `<button>` ou un `<a>`, soumis à trois
exclusions (translucide, préfixe de variante, et la famille des tokens sans
couleur : `transparent`, paliers de surface, `bg-clip-*`, `bg-gradient-*`…).

Le vol a ajouté une **deuxième passe** : un `bg-gradient-*` accompagné d'un stop
`from-*` est un fill, rapporté par son stop. Le pilote a recopié les exclusions 1
et 2 dans la nouvelle passe, et pas l'exclusion 3. La revue l'a trouvé par sonde.

## Leçon 1 — Une nouvelle passe repart de zéro, sauf si on lui recopie tout

Les exclusions de la passe d'origine étaient écrites **sur les tokens `bg-*`**.
La nouvelle passe lit des tokens `from-*`, que ces exclusions ne voient jamais.
Elle ne pouvait donc en hériter que si on les traduisait. Celles qui manquaient
ont produit trois faux rouges, chacun reproduit sur fixture :

| fixture | ce que la passe gradient rapportait | exclusion d'origine qui l'aurait couvert |
|---|---|---|
| `bg-gradient-to-r from-transparent via-white/10 to-transparent` | `from-transparent` | `bg-(transparent\|inherit\|current\|none)` |
| `bg-gradient-to-b from-surface-container to-bg` | `from-surface-container` | `bg-(bg\|card\|surface…)` |
| `<a className="bg-clip-text text-transparent bg-gradient-to-r from-accent …">` | `from-accent` | `bg-clip-*` : le dégradé peint les glyphes, pas un fond |

Un faux rouge sur un détecteur de ce genre se règle en le mettant en sourdine,
et c'est le mode de défaillance que son en-tête dit fermer en premier.

**La règle.** Quand on ajoute une passe à un détecteur, on écrit la table des
exclusions existantes et on décide, ligne par ligne, ce que chacune devient dans
la nouvelle passe : traduite, dominante, ou délibérément levée (et dite). Le
silence sur une ligne de la table, c'est la ligne qui fera le premier faux rouge.

## Leçon 2 — Un fixture vert posé hors de la population ne prouve rien

Le plan prévoyait le fixture N7 pour tenir le faux positif naturel de la règle :
le texte en dégradé (`bg-clip-text text-transparent` + stops). Le fixture le
posait sur un `<h1>`. Or le détecteur n'évalue que `<button>`, `<a>` et
`<Button>`. N7 était vert **parce que le scan ne regardait pas la balise**, pas
parce que la règle savait se retenir. Sur un `<a href>`, la même chaîne de
classes faisait feu.

**La règle.** Un fixture vert qui prétend démontrer la retenue d'un prédicat
doit placer son sujet **dans la population que le prédicat évalue**. Le bon
contrôle est sur une balise que le scan regarde, et il doit rougir quand la
retenue est retirée. N7b fait les deux : il est sur `<a>` et `<button>`, et il
rougit sur le détecteur d'avant le correctif.

## Leçon 3 — Une regex d'attribut qui tolère « n'importe quoi » entre `=` et la valeur

L'exclusion des rôles d'état (`role="switch"`, `tab`…) admettait n'importe quelle
suite non alphanumérique entre `role=` et le mot. Elle reconnaissait donc aussi
`role === "tab"` dans un `cn(...)` et `title="role=tab"`. Ce sont deux CTA
réelles qu'une exclusion faisait sortir de la population, sans bruit : un faux
**vert**.

**La règle.** Une exclusion est une porte de sortie de la population. Sa regex
doit nommer la forme exacte qu'elle accepte (ici un `{` optionnel puis une quote),
et pas la tolérer par défaut. Le fixture négatif N6c garde cette forme.

## Comment ça a été trouvé

Par sonde, pas par lecture. Les relecteurs ont construit des arbres jetables et
lancé `bash scripts/check-cta-primitives.sh <arbre>` dessus. Les trois classes de
défaut ci-dessus sont invisibles à la lecture du diff, parce que chaque ligne
ajoutée est correcte prise seule. Elles n'apparaissent que si l'on fait tourner
le détecteur sur la forme qu'il n'a jamais vue.

## Référence

- `scripts/check-cta-primitives.sh` : passe gradient et `STATE_ROLE`
- `scripts/test-check-cta-primitives.sh` : fixtures N5c, N6c, N7b, N9b
- Voisins : `un-plancher-danti-vacuite-peut-se-rembourrer-avec-le-trou-quil-garde.md`,
  `une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md`
