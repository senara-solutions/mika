---
module: crates/mika-common/src/llm/budget.rs
tags: [timeouts, configuration, coupled-constants, asymmetric-knob, retry-budget, structural-gate, mika-2189, mika-1660]
problem_type: architecture-pattern
category: architecture-patterns
---

# Deux constantes couplées dont une seule a un bouton

## Problème

`llm_calls` porte, sur les 7 jours arrêtés au 2026-09-05, **209 échecs** sous un
seul message :

```
LLM transport error: failed to read response body: error decoding response body:
request or response body error: operation timed out
```

Leur distribution de latence n'a pas de queue — elle a **deux valeurs** : 240 s
(171 fois) et 120 s (37 fois). Ce n'est pas de la variance fournisseur. C'est un
couperet client franchi une fois ou deux : `DEFAULT_HTTP_TIMEOUT_SECS = 120`,
posé sur `reqwest` en `.timeout(...)`, qui borne la requête **entière, lecture du
corps comprise**.

Le remède évident — relever le plafond — était disponible depuis mika#1660, qui
avait exposé `MIKA_LLM_HTTP_TIMEOUT_SECS`. Personne ne l'avait appliqué, et c'est
la partie intéressante : **il était inapplicable sans casser autre chose.**

## Cause racine

Deux nombres gouvernent un tour d'agent :

| nombre | rôle | réglable avant mika#2189 |
|---|---|---|
| plafond par appel | ce que `reqwest` reçoit | **oui** (mika#1660) |
| enveloppe d'agent | la deadline du tour | **non** — `const` nue, sans variable d'environnement |

L'enveloppe doit *contenir* le plafond : une passe mika-arch consomme 3,1 appels
en moyenne, et l'enveloppe doit aussi héberger les appels d'outils. Relever le
seul plafond à 300 s dans une enveloppe de 300 s fait qu'**un appel avale le
budget entier d'une passe qui en demande trois.**

L'asymétrie est le défaut, pas la valeur. Un bouton sur une moitié d'une paire
couplée n'est pas « à moitié réglable » : il est **piégé**. Il donne à l'opérateur
un geste qui a l'air d'être le remède et qui produit une seconde panne, moins
lisible que la première.

Deux amplificateurs, tous deux de la même famille :

1. **Les seuils de rejeu étaient calibrés en littéral sur le plafond par défaut.**
   `TYPICAL_CALL_DURATION_SECS = 90`, `RETRY_BUFFER_SECS = 30`,
   `TRANSPORT_RETRY_MIN_REMAINING_SECS = 60` — soit exactement 0,75 ×, 0,25 × et
   0,50 × un plafond de 120 s. Au premier réglage non-défaut, ces trois nombres
   décrivent une géométrie qui n'existe plus. Ils mentent en silence.

2. **Une seconde constante promettait par commentaire ce que le type ne tenait
   pas.** `TEAM_AGENT_TIMEOUT_SECS = 300` portait la doc « matches
   AGENT_TOTAL_TIMEOUT_SECS ». Un `grep` du nom évident trouvait trois lecteurs
   sur quatre — et le quatrième était celui qui aurait dérivé silencieusement au
   premier changement de valeur.

## Solution

**Faire voyager la paire ensemble, et refuser les paires incohérentes.**

`LlmTimeoutBudget` détient `(plafond, enveloppe)` et *dérive* les trois seuils de
rejeu du plafond effectif, en fractions. Au plafond par défaut les fractions
reproduisent 90 / 30 / 60 **exactement** : la migration est un no-op mesurable à
la géométrie livrée, et suit tout réglage ultérieur.

L'invariant de contenance (`plafond < enveloppe`) est vérifié à la **construction
du fournisseur** — le même point du cycle de vie où mika#1660 paniquait déjà sur
un plafond trop petit. Conséquence assumée et écrite plutôt que découverte : *un
`mika` qui démarre n'est pas la preuve que ses budgets sont valides ; le premier
appel l'est.*

L'enveloppe se lit sur le **fournisseur**, qui détient déjà le plafond. Prendre
les deux au même endroit est ce qui les empêche de re-diverger.

### Le coût qu'un plafond relevé crée vraiment

Un plafond est un plafond : le relever **ne peut pas** ralentir un appel qui
réussissait. La régression est de l'autre côté — **un appel qui échoue devient
plus cher**. C'est le point qu'un correctif de timeout oublie presque toujours de
borner, parce que la métrique qu'on regarde après coup est la latence des succès,
et elle est bonne.

D'où `max_attempts = floor(enveloppe / plafond)`, qui rend
`tentatives × plafond ≤ enveloppe` vrai **par construction**. Au défaut cela vaut
2 — précisément la signature à 240 s que la mesure montre.

Et il faut se garder de dire que c'est un no-op pur, parce que ce serait plus
confortable et moins vrai : avant, une troisième tentative pouvait démarrer à
`remaining == seuil` exactement et porter l'échec à 360 s, hors de l'enveloppe de
300 s. Ce cas-limite est fermé.

## Ce qui a été rejeté, sur mesure et non sur avis

Le ticket proposait trois remèdes. Deux ont été refusés **par une mesure citée**,
ce qui vaut mieux qu'un jugement :

- **Rejeu transport borné côté client** — déjà présent (`MAX_RETRIES = 3`). La
  mesure ne montre jamais plus de deux tentatives, et l'avortement est écrit dans
  le code. Surtout : *un appel qui a besoin de 150 s n'aboutira pas davantage à
  la troisième tentative de 120 s.* Le remède était là et la mesure le montrait
  inopérant sur cette classe.
- **Repli de modèle pour mika-arch** — 195 des 209 pannes sont hors mika-arch,
  sur un autre modèle, chez le même fournisseur. Le repli aurait déplacé 7 % du
  problème et fermé le ticket.

C'est le rendement principal de la mesure : elle n'a pas seulement désigné le
coupable, elle a **disqualifié deux remèdes plausibles** qu'un raisonnement
d'analogie aurait retenus.

## La date, et ce qu'elle exclut

mika-arch : **zéro erreur du 08-27 au 09-02**, puis 3 le 09-03, 5 le 09-04, 6 le
09-05. Lecture tentante : « incident fournisseur du 09-03 ». Lecture correcte :
son prompt système est passé de ~54 Ko à 59,8 Ko le 2026-09-01, et les 14 erreurs
sont **toutes** sur cette variante. La flotte, elle, saignait depuis au moins le
08-27.

Le seuil est fixe ; c'est la distribution qui a glissé dessous. Une chronologie
qui commence quand *on* a commencé à regarder n'est pas une chronologie du
phénomène.

## Comment le reproduire ailleurs

Trois questions, dans cet ordre, dès qu'on ajoute un bouton à une valeur :

1. **Quelle autre valeur doit contenir celle-ci ?** Si la réponse n'est pas
   « aucune », les deux prennent un bouton dans le même changement, ou aucune.
2. **Quel code est calibré en littéral sur la valeur par défaut ?** Ces littéraux
   deviennent des fractions, sinon ils mentent au premier réglage. Vérifier que
   la conversion est un no-op à la valeur par défaut est le test qui empêche de
   « corriger » la calibration au jugé.
3. **Que coûte l'échec après le réglage ?** Le succès va bien par construction ;
   c'est le pire cas d'échec qu'il faut borner, et de préférence par une identité
   arithmétique plutôt que par une observation d'après-coup.

Et une garde, pas un commentaire : quand une constante est retirée, un test qui
scanne les sources est ce qui empêche un quatrième lecteur de réapparaître. Une
garde qui couvre trois appelants sur quatre est une garde qui ment — et le nom
« évident » de la constante n'est pas le seul nom sous lequel elle se lit.

## Références

- Plan : `docs/plans/2026-09-05-003-fix-2189-openrouter-read-timeout-budget-plan.md`
- Implémentation : `crates/mika-common/src/llm/budget.rs`,
  `crates/mika-agent/src/planning/policy.rs`
- Garde structurelle : `policy::tests::no_bare_agent_timeout_constant_remains`
- Antécédent : mika#1660 (le bouton sur la moitié), mika#1744 (le seuil transport)
