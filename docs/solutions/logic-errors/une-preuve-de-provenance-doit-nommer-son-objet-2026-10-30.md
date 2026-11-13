---
title: "Une preuve de provenance doit nommer son objet, sinon elle prouve qu'un geste a eu lieu — pas que CE geste a eu lieu"
date: 2026-10-30
category: logic-errors
module: skills/executor
problem_type: logic_error
component: tooling
symptoms:
  - "Three simultaneously-valid grooming proofs for a single ticket, one per flight (mika#2968: e53f9ae7 vol 1, 19aea690 vol 2a, 6b7695ce vol 2b)"
  - "has_completed_groom_for_issue returns Converged for a multi-flight ticket as soon as ANY terminal groom callback exists under a parent carrying the issue URL"
  - "dev-pilot departs for flight N on flight 1's proof, so flight N's plan and callouts could have been hand-stamped — the very class the gate exists to refuse"
  - "No incident: filed as a safety ticket on reading the code and counting the rows, before the debit"
root_cause: logic_error
resolution_type: code_fix
severity: high
tags:
  - dispatch-gate
  - grooming-marker
  - autonomous-loop
  - provenance
---

# Une preuve de provenance doit nommer son objet

## Le défaut, en une phrase

La porte de mika#2287 demandait *« ce ticket a-t-il été groomé ? »* alors que la
question qui décide est *« CE vol a-t-il été groomé ? »*. Les deux rendent le même
octet sur un ticket à un seul vol, et divergent sur tous les autres.

La requête (`has_completed_groom_for_issue`, `db/tasks.rs`) joint un callback de
groom terminal à une parente portant l'URL du ticket. Aucun de ses termes ne lit
l'**objet** du groom. Sur mika#2968, mesuré le 2026-10-30, trois preuves étaient
donc valides simultanément — une par vol — et n'importe laquelle satisfaisait la
porte pour n'importe quel vol, y compris un vol dont le callout `Plan:` aurait
été posé à la main. C'est la classe que mika#2287 nomme lui-même (*« markers may
be pre-stamped by hand »*), rentrée par la fenêtre du temps.

## La classe

**Une preuve dont l'identité est celle du sujet et non celle de l'acte se
périme sans le dire.** Elle est juste le premier jour, et reste « juste » tous
les jours suivants parce que rien ne la relie à ce qu'elle était censée
attester. Le symptôme n'est pas un faux positif visible : c'est une porte qui
*répond* tout en ayant cessé de *mesurer*.

Signes qu'on y est :

1. La preuve est indexée par un identifiant **stable** (un ticket, un dépôt, un
   utilisateur) là où le geste qu'elle atteste est **répété** (un vol, un
   déploiement, une session).
2. Le cas à une occurrence — le seul couvert par les tests — rend la même
   réponse que le cas sain. Les tests restent verts pour toujours.
3. La population fautive **croît avec le succès du système** : plus la boucle
   enchaîne de vols par ticket, plus l'écart s'ouvre.

## Le remède : lier la preuve à son objet, chez le producteur

Deux termes étaient possibles, et le choix n'est pas cosmétique.

| terme | ce qu'il lit | pourquoi écarté / retenu |
|---|---|---|
| **le plan nommé par la preuve** | le `Plan:` du corps courant, comparé au suffixe que la preuve porte | **retenu** — le plan est l'*objet* du groom, et la comparaison est hors réseau, donc testable « dans le même appel » |
| l'instant du dernier merge | « issue → PR liée → dernier merge » | écarté — ce lecteur n'existe pas (`gh_merged_pr_for_branch` est clé de branche), et il aurait mis une dépendance réseau sur un chemin qui échoue-ouvert sans jeton : une porte que l'absence d'un jeton désarme |

Un merge est un **proxy** de « un vol s'est terminé ». Le plan est l'objet même.
Quand les deux sont disponibles, préférer l'objet : le proxy se décorrèle le jour
où quelqu'un change le cycle de vie, et personne ne pense alors à la porte.

**Le producteur publie, le lecteur compare.** La frappe est suffixée à la source
(`_set_outcome_line "Outcome: PLAN_GROOMED — <plan repo-relatif>"` dans
`dispatch-lib.sh`), parce que le seul instant où le plan est connu de façon
fiable est celui où le callout canonique est écrit. Reconstituer la liaison du
côté lecteur, à partir de l'horodatage ou du nom de branche, c'est redevenir un
proxy.

## Les trois pièges du passage, et comment ils sont tenus

### 1. Lire le suffixe ailleurs que sur la ligne ancrée (classe mika#2590)

Un `result.contains(" — ")` global attrape une mention en prose. La liaison
devient alors falsifiable par le texte du modèle — exactement le défaut que
mika#2590 a fermé sur le marqueur lui-même. Le suffixe se lit par
`strip_prefix(GROOM_SUCCESS_MARKER)` **sur la ligne ancrée**, et le lecteur reste
unique (`task_state/tasks.rs`).

### 2. Refuser les preuves d'avant, donc arrêter la boucle le jour du merge

Trois producteurs légitimes frappent encore une preuve **nue** : toutes celles
déjà en base ; la fenêtre entre le merge et `make deploy`, puisque
`skills/bundled/` est lu au **build** (motif mika#2340) ; et le flux opérateur
`/mika-groom-ticket`, qui écrit son propre callout. Une porte qui les refuse est
une porte qu'on désarme dans l'heure.

D'où une **grâce nommée, comptée et datée** : constante
`GROOM_PROOF_UNBOUND_GRACE` dont le doc-comment dit la donnée exacte qu'elle
épargne, ses trois producteurs, et comment elle se vide (rétention de 30 jours
sur les tâches). Pas un `if` discret : une exception grep-visible qui porte son
propre site de retrait.

### 3. Rendre la grâce invisible (classe mika#2205, appliquée à sa propre sonde)

Le piège le plus coûteux. Avec un seul état neuf (`PlanMismatch`), « accepté sur
la grâce » et « accepté avec une preuve liée » rendent **les mêmes octets** à
l'étage supérieur : l'événement d'audit qui mesure la grâce n'est pas écrivable,
donc la grâce n'expire jamais. Il faut **deux** états —
`Converged` / `ConvergedUnbound` — avec `is_converged()` vrai pour les deux, de
sorte qu'aucun lecteur existant ne change, et **deux** noms d'événement
distincts : l'un compte les vols que la porte a arrêtés, l'autre ceux qu'elle a
laissés partir sans liaison. Sous un seul nom les deux populations ne se
soustraient pas.

## La fixture, et ce qu'elle apprend sur le schéma

`idx_tasks_manual_active_ref_url` (v17) est un index unique **partiel** : deux
tâches `manual` **actives** ne peuvent pas porter la même `reference_url`. Un
ticket à plusieurs vols a donc toujours ses parentes antérieures terminales — et
la preuve du vol 1 survit malgré ça, parce que le prédicat ne filtre le statut
que de l'**enfant**. C'est précisément ce qui rend le trou atteignable, et c'est
ce que la fixture d'AC2 doit reproduire pour dire quelque chose de vrai.

## Doctrine de fixture : le fil, jamais la constante

Une fixture porte la **forme de fil** — le littéral ` — ` — et non la constante
que le lecteur lit. Sinon un changement de séparateur déplace les deux côtés
ensemble et le test passe des deux côtés d'une régression. L'égalité entre le
littéral et la constante est tenue par un test dédié, dont c'est le seul sujet.

## À quoi le reconnaître ailleurs

Chercher les gardes dont la clé de lecture est le **sujet** quand l'acte est
répétable :

- une attestation de revue indexée par PR alors que la PR reçoit des pushes ;
- un reçu de calibration indexé par agent alors que le modèle change ;
- un consentement indexé par utilisateur alors que la portée du consentement
  change.

Dans les trois cas la question à poser est la même : *si cet acte se répète, la
preuve du premier satisfait-elle la porte pour le second ?* Si oui, la preuve ne
nomme pas son objet.

## Références

- mika#2981 (ce ticket), mika#2287 (la porte), mika#2590 (le marqueur ancré),
  mika#2968 (les trois preuves mesurées), mika#2980 (l'orphelinage qui a conduit
  à la lecture).
- [grooming-provenance-gate-reads-a-parent-row-a-sibling-mechanism-flips](grooming-provenance-gate-reads-a-parent-row-a-sibling-mechanism-flips.md)
  — la porte, son défaut fondateur, et pourquoi son bypass l'avait masquée.
- [a-guard-that-reads-its-evidence-from-the-claim-cannot-refute-it-2026-08-30](../best-practices/a-guard-that-reads-its-evidence-from-the-claim-cannot-refute-it-2026-08-30.md)
  — pourquoi les marqueurs du corps ne suffisent pas, donc pourquoi cette porte
  existe.
- [guard-must-read-substance-not-the-shape-its-producer-happens-to-emit-2026-09-04](../architecture-patterns/guard-must-read-substance-not-the-shape-its-producer-happens-to-emit-2026-09-04.md)
  — même famille : lire la substance et non la forme que le producteur émet par
  accident. Ici la substance manquait **chez le producteur**, et c'est lui qu'il
  a fallu enrichir.
