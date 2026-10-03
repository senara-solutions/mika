---
module: mika-agent
tags: [audit-events, idempotency, append-only, budget, ledger, rerun, merge-gate]
problem_type: architecture-pattern
---

# Une réservation append-only précède l'effet qu'elle borne

**Date :** 2026-10-03
**Ticket :** mika#2617 (phase B, U3/AC2)
**Voisins :** mika#1869 (dedup durable par `head_sha`), mika#2347 (le ledger
décide), mika#2158 (un compteur remis à zéro par l'action qu'il compte ne borne
rien), mika#2020 (budget de re-drive)

## Le problème

Un budget « au plus une fois » tenu dans `audit_events` se lit par un **compte**
(`count_recent_audit_events_for_target`). La question qui décide de tout est :
**quand écrit-on la ligne par rapport à l'effet qu'elle borne ?**

Dans mika#2617, l'effet est une relance de CI (`gh run rerun --failed`) et le
budget est « jamais deux fois sur la même tête ». Deux ordres sont possibles, et
ils n'ont pas le même mode de panne :

| ordre | mode de panne |
|---|---|
| relance, **puis** écriture | un crash entre les deux laisse le budget intact → la relance **rejoue**, et c'est exactement ce que l'invariant interdit |
| écriture, **puis** relance | un échec de la relance **consomme** le budget → une relance perdue |

## Ce qui tranche

**L'asymétrie des coûts, et elle est locale.** Ici, un faux « déjà relancé »
coûte une relance perdue sur une PR qui attend de toute façon un humain ; un faux
« jamais relancé » relance en boucle, et chaque tour coûte un run CI complet. La
réservation précède donc l'effet.

Ce n'est pas un universel : l'ordre juste est celui dont le mode de panne est le
moins cher **pour cet effet-là**. Un effet idempotent et gratuit (poser un label
déjà posé) supporte l'ordre inverse ; un effet qui consomme une ressource ne le
supporte pas.

## La conséquence que ça impose au contenu de la ligne

`audit_events` est **append-only** : une ligne ne se révise pas. Écrite avant
l'effet, elle ne peut donc pas porter l'**issue** de cet effet — à cet instant
l'issue n'est pas connue, et la nommer serait inventer.

D'où la répartition :

- la ligne d'audit porte `attempted`, et **elle est le budget** ;
- l'issue (`triggered` / `refused` / …) vit sur la **ligne de journal**, champ
  `outcome`.

Le réflexe contraire — écrire une seconde ligne après l'effet, avec l'issue —
casse le budget : le prédicat est un compte, et une seconde ligne le fait passer
à deux.

## Le corollaire qu'on découvre en voulant instrumenter l'épuisement

Le plan de mika#2617 demandait « deux échecs ⇒ WARN **plus** ligne d'audit ».
La ligne d'audit a dû être écartée, pour deux raisons qui se composent :

1. **Elle polluerait le compte.** Une ligne « épuisé » sous le même `tool_name`
   que les tentatives rend le compte faux pour le prédicat qui le lit.
2. **Elle serait du churn.** Un seul push produit jusqu'à huit
   `check_suite.completed` (mika#1869), donc une ligne par *évaluation* d'une
   tête déjà relancée, sur une population qui ne change pas — ce que la doctrine
   mika#2131 borne.

Et surtout : **l'épuisement n'est pas un fait neuf.** L'information durable
(« cette tête a eu sa relance ») est déjà la ligne de tentative ; « le budget est
épuisé » en est le corollaire arithmétique. Le WARN, lui, reste — la vivacité est
ce que l'opérateur lit.

## Le test qui tient l'invariant

Il ne porte pas sur l'effet (qui demande un réseau) mais sur **le compte après la
tentative** :

```rust
let first = maybe_rerun(/* … */).await;   // la relance échoue (pas de `gh`)
assert!(matches!(first, RerunOutcome::Refused(_)));
assert_eq!(count_for(&key).await, 1, "la réservation précède l'effet");

let second = maybe_rerun(/* … */).await;
assert_eq!(second, RerunOutcome::AlreadySpent { run_id });
assert_eq!(count_for(&key).await, 1, "l'épuisement n'écrit rien");
```

Le succès de l'effet est hors de portée d'un test d'unité et vit dans une sonde
opérateur. Ce qui est attestable sans réseau — et ce qui compte — est que la
réservation a bien été écrite **avant**, et qu'une seconde tentative est refusée.

## La clé, et le sens de son fail-safe

`rerun:{repo}#{pr}@{head_sha}:{run_id}`, interrogée par **égalité exacte** (pas
un `LIKE`, donc le piège `#234` ↔ `#2343` de mika#2347 ne s'ouvre pas ; le `@`
est écrit quand même pour qu'un futur lecteur de préfixe soit sûr par
construction).

Deux propriétés que la *forme* de la clé achète sans une colonne de plus :

- **un nouveau sha rouvre le budget de lui-même** — nouveau code, nouvelle
  chance ;
- un `head_sha` **vide** (le `#[serde(default)]` d'un `headRefOid` absent) n'est
  **jamais** traité comme une tête : ce serait une clé partagée par toutes les
  têtes illisibles, donc **un budget pour toutes**. Il est lu comme illisible, et
  l'illisible refuse (fail-closed).

## Ce que ça n'achète pas

L'ordre réservation-puis-effet ne rend pas l'effet fiable. Il rend l'invariant
« au plus une fois » vrai **en travers d'un crash et d'un redémarrage**, ce qu'un
ledger en mémoire ne peut pas faire — et il le rend vérifiable par un compte, ce
qu'un drapeau ne peut pas faire.
