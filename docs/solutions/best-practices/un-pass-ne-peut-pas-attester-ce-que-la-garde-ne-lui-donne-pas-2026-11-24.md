---
title: Un pass ne peut pas attester ce que la garde ne lui donne pas
date: 2026-11-24
category: best-practices
module: mika-agent/evidence
problem_type: best_practice
component: qa-review
severity: high
applies_when:
  - Une garde décide sur l'état du monde et le texte qu'elle laisse passer affirme cet état
  - Un modèle se voit interdire la lecture d'une source et continue d'en affirmer le contenu
  - Un bras « en attente » d'un classement est laissé ouvert pour ne pas refuser le cas nominal
tags:
  - qa-review
  - verdict
  - gardes-structurelles
  - ci
  - mika-3137
  - mika-2455
  - mika-2423
---

## La classe

**Une garde qui décide sur l'état du monde, et jamais sur ce que le texte en
affirme, laisse passer l'affirmation fausse.**

mika#2455 a posé la porte CI↔verdict de mika-qa. Elle lit les vérifications
requises sur la tête revue et refuse un `pass` qu'un check **rouge** contredit.
Son bras `pending` a été laissé ouvert délibérément, et pour une bonne raison :
`pull_request.opened` route vers mika-qa sans aucun terme de CI, donc une revue
qui commence avant la fin de la CI est le cas **nominal**. Refuser cette
population aurait refusé le nominal.

Le trou n'était pas dans ce choix. Il était dans son **grain** : la porte
décidait sur l'état des checks et ne regardait pas ce que le corps en disait.
Un `pass` muet sur la CI et un `pass` écrivant « CI green » tombaient dans le
même bras, donc tous deux passaient — alors que le premier est honnête et le
second est une fabrication.

## Ce qui a été mesuré

Quatre occurrences, en deux jours :

| PR | instant du `pass` | ce que le corps affirmait | fin réelle des requis |
|---|---|---|---|
| #3133 | 2026-11-21T15:46:18Z | « check_suite success on 176bab64 » | 15:54Z, `Security` **rouge** |
| #3133 | 2026-11-21T16:39:26Z | « CI green » | 17:02:26Z, vertes |
| #3139 | 2026-11-22T18:59:56Z (84 s après l'ouverture) | idem | 19:15:45Z, vertes |
| #3143 | 2026-11-22T20:44:36Z (61 s après l'ouverture) | idem | 21:00:02Z, vertes |

Trois fois sur quatre, l'affirmation s'est trouvée vraie *après coup*. C'est
précisément ce qui rend la classe dangereuse : elle ne produit un dégât visible
qu'une fois sur quatre, et le reste du temps elle entraîne tout le monde à lui
faire confiance.

## Pourquoi le modèle l'écrivait

Parce que **rien ne le lui donnait, et rien ne le lui interdisait non plus**.
mika#2423 interdit à mika-qa de lire la CI — son scope est le diff et les
artefacts de pipeline, la porte de merge lit la CI un cran en aval, sans limite
de temps. L'interdiction de **lire** était écrite ; l'interdiction d'**affirmer**
ne l'était pas. Entre les deux, le modèle comble avec ce qu'il a sous la main :
l'état d'un événement `check_suite` déclencheur, qui atteste **une** suite et
jamais l'ensemble des requis — un push peut en produire jusqu'à huit
(mika#1869).

## Le correctif, et sa forme

Trois propriétés, dans cet ordre de robustesse décroissante.

**1. Le détecteur décide sur la FORME.** `body_attests_ci_concluded` cherche la
cohabitation, sur une même ligne, d'un **sujet CI** et d'un **terme de
conclusion**, et désarme la ligne sur un négateur. Jamais une liste des phrases
mesurées : doctrine mika#1804 — la régression réaliste n'est pas de retaper
« CI green », c'est d'écrire une autre tournure qui voulait dire la même chose.

**2. L'asymétrie est choisie, et écrite.** La liste des négateurs est
délibérément trop large. Conséquence assumée : un corps dont une autre
proposition porte un « no » ou un « not » échappe au détecteur — un faux
négatif. L'autre côté de l'arbitrage serait de **refuser une revue honnête**,
ce qui coûte un cycle de dispatch et apprend au modèle que la garde est
arbitraire. Un test nomme le faux négatif et dit en toutes lettres qu'il
n'énonce pas un comportement souhaitable, pour que le jour où l'on resserre la
liste, on sache quelle population on gagne.

**3. Le refus est auto-suffisant.** Il nomme les vérifications en cours, **cite**
la ligne qu'il reproche, et nomme les deux sorties sans en prescrire aucune :
garder le `pass` en remplaçant la phrase par `CI-Status: pending at review
time — <liste>`, ou réécrire le verdict en `hold[review]`. Il ne prescrit pas
`block[ci]` : rien n'est rouge, un pilote de réparation CI n'aurait rien à
réparer.

Et la conséquence qui tient tout le reste : `AllowedPending` porte désormais
`pending: Vec<String>`, comme `Refused` portait déjà `failing`. **Une garde qui
dit « la CI n'a pas conclu » sans nommer quoi ne donne rien à vérifier.** Le
champ n'a pas été ajouté pour le confort du message ; sans lui le refus n'était
pas rédigeable.

## La règle à emporter

Quand une garde refuse sur un **fait** et laisse passer une population où ce
fait est indéterminé, poser la question suivante : *dans cette population, le
texte peut-il affirmer le fait ?* Si oui, la garde a deux sujets et n'en traite
qu'un.

Et le corollaire sur l'ordre de livraison : la **garde** mord dès le binaire,
le **prompt** attend `make deploy` (mika#2340). Livrer les deux, dans cet
ordre — la garde tient sans le prompt, le prompt évite à la garde de devoir
tirer.

## Disposition de tir

Le détecteur est livré **armé** : un test scanne le prompt `qa-review`
embarqué (`include_str!`) et rougit si une de ses lignes déclenche le
détecteur — car toute revue recopiant cette ligne serait refusée. Il en a
trouvé une au premier tir (`system_prompt.md:681`, qui portait `checks` et
`successful` sans négateur), et la règle du vol était explicite : **corriger
la ligne du prompt, pas inscrire une exception**. L'allowlist est auto-nettoyante
dans les deux sens — chaque entrée doit encore déclencher — et elle est livrée
**vide**, son vide étant asserté.

## Sondes

| ligne | niveau | ce qu'elle dit |
|---|---|---|
| `qa_ci_coherence_refused_pending_attested` | WARN | un `pass` attestant la CI a été refusé sur un requis pendant ; porte `pending_checks` et `attested_line` |
| issue d'audit `refused_pending_attested` | — | cinquième valeur, en **ajout de queue** derrière les quatre de mika#2455 ; un `GROUP BY outcome` couvrant plusieurs mois n'est pas coupé en deux (doctrine mika#2131) |
| issue d'audit `allowed_pending` | — | inchangée : c'est le `pass` **muet** sur un requis pendant, et sa population reste comptable à part |

Les deux issues sont exclusives sur un même tour. Leur co-occurrence
compterait le tour deux fois ; un test d'eval le refuse.

## Voir aussi

- `docs/solutions/best-practices/un-interdit-de-prompt-qui-recidive-se-ferme-dans-le-moteur-2026-10-14.md`
  — la classe sœur : l'interdit de prompt seul ne tient pas.
- mika#2455 (la porte CI↔verdict), mika#2423 (mika-qa ne lit pas la CI),
  mika#1869 (un push produit jusqu'à huit `check_suite`), mika#1804 (décider
  sur la forme), mika#3135 (vu aussi, hors périmètre).
