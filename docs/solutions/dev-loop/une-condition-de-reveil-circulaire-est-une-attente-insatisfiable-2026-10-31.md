---
module: docs/dormeurs.md, crates/mika-agent/src/milestone_manager, skills/bundled/_shared/dispatch-lib.sh
tags: [dormancy, wake-condition, circular-dependency, condition-rot, grooming, support-change, loop-substrate]
problem_type: best-practice
category: dev-loop
created: 2026-10-31
ticket: mika#2854
---

# Une condition de réveil circulaire est une attente insatisfiable — et elle est invisible

## TL;DR

Un travail dû mais non exécutable se met en dormance avec une **condition de réveil**. Le
contrat de `docs/dormeurs.md` refuse les conditions floues (« plus tard », « si ça
revient ») en les nommant *des fermetures déguisées en attentes*. Il ne dit rien d'une
forme plus coûteuse : la condition **précise, tranchable, et toujours fausse**.

Sur mika#2854, la condition de réveil écrite le 2026-10-22 était :

> - **Condition de réveil :** quand `MIKA_WEBHOOK_COMMENT_TRIAGE` passe à `on`.
> - D'ici là, l'interrupteur reste à `off` par défaut. Le passer à `on` exige cette mesure d'abord.

La mesure est due quand l'interrupteur est armé ; l'armement exige la mesure. Les deux
lignes se mordent la queue. Un lecteur peut trancher la condition à tout instant — et elle
rend **faux** à tout instant. Le ticket est donc resté ouvert, donc dispatchable, et la
boucle l'a re-groomé quatre fois.

## Le test de lecture qui manquait

Le contrat de `docs/dormeurs.md` demande : *« un lecteur peut-il dire, sans contexte, si
elle est remplie ? »*. C'est un test de **précision**, et la condition de mika#2854 le
passait sans difficulté.

Il faut un second test, qui porte sur la **satisfiabilité** :

> **Existe-t-il un état du monde où cette condition rend vrai, et cet état est-il
> atteignable sans l'exécution du travail qui dort ?**

Une condition qui échoue à ce test est pire qu'une condition floue, pour trois raisons :

1. **Elle a la forme d'une condition valable.** Un nom de variable, une valeur, un verbe
   d'état : elle passe la relecture par ressemblance.
2. **Elle ne se signale jamais.** Une condition floue finit par agacer quelqu'un ; une
   condition tranchable qui rend `false` se contente de rendre `false`, indéfiniment et
   sans bruit.
3. **Elle inverse l'ordre réel.** Sur mika#2854, la mesure ne **suit** pas l'armement :
   elle le **précède** et le fonde — elle produit le chiffre sur lequel l'opérateur décide
   d'armer. La condition écrite avait pris l'aval pour l'amont, et le vrai bloqueur était
   nommé deux lignes plus haut dans le même corps (`senara-solutions/claude-pilot#290`,
   ouverte au 2026-10-31).

La règle qui en sort : **la condition de réveil nomme le bloqueur en amont, jamais la
conséquence en aval.** Si la condition mentionne l'effet que le travail doit produire, elle
est circulaire.

## Le coût constaté

Quatre sessions de grooming sur mika#2854, chacune un vol de pilote entier dépensé sur un
travail qui n'était pas exécutable : `29e19a520` et `c1b375d7d` (2026-10-20), `141058a26`
(2026-10-21), `5b892e53c` (2026-10-31). Aucune n'a été fautive : chacune a lu un ticket
ouvert, labellisé, avec un plan à produire, et a produit le plan.

C'est la propriété qui rend la classe chère : **rien dans la chaîne ne se comporte mal.**
Le défaut est entièrement dans une ligne de prose du corps du ticket, et il se paie en
tours de pilote.

## Pourquoi la réponse est un changement de support, pas un lecteur de plus

La tentation est de corriger la ligne. Elle laisserait le ticket **ouvert**, donc
dispatchable, et elle ne réglerait rien pour la porte qui a produit la dépense.

Mesure du 2026-10-31, et c'est elle qui tranche : **chaque porte lit la dormance pour son
compte, ou pas du tout.**

- Le bassin du manager la lit : `RefusalReason::Dormant` vit dans
  `crates/mika-agent/src/milestone_manager/` (mika#2952), et ses fixtures citent mika#2854
  verbatim.
- La porte qui a produit les quatre vols ne la lit pas :
  `grep -rln "Dormant\|dormancy" --include="*.rs" crates/` ne rend **que** des fichiers de
  `milestone_manager/` (plus deux tests), et côté bash `dispatch-lib.sh` n'a **aucune**
  lecture de dormance.

Le manager refuse donc d'admettre un ticket pendant qu'une autre porte le groome. Ajouter
un lecteur de dormance de plus réglerait une porte et laisserait les suivantes — la forme
même que la boucle a déjà payée par classes successives (mika#2263, *« 3e classe »*).

Le dépôt a une réponse ratifiée qui agit sur **toutes** les portes à la fois, parce qu'elle
n'est pas un lecteur mais un **état** : la décision de l'opérateur du 2026-09-03, *« la
visibilité change de support : le registre versionné remplace le ticket ouvert »*. Le
travail passe dans `docs/dormeurs.md` avec sa condition, et le ticket se **ferme**. Un
ticket fermé n'est dispatchable par aucune porte, qu'elle lise la dormance ou non.

Et la fermeture est **réversible par construction** : le § *Réveil* du registre dit de
rouvrir le ticket cité, qui conserve tout son historique. Ce n'est pas une perte de
visibilité — les onze autres entrées en place l'attestent, toutes sur des tickets `CLOSED`.

## Ce qu'un dormeur doit porter, puisqu'on ne reviendra pas le chercher

Deux entrées du registre (#2609, #2357) portent un « **Au réveil, …** », et pas par
élégance : un épique d'incident a déjà périmé avant son exécution faute de cette phrase
([`epique-dincident-perime-avant-son-execution-2026-09-24.md`](epique-dincident-perime-avant-son-execution-2026-09-24.md)).
Une entrée utile porte donc, en plus de sa condition :

- **où est l'instrument**, s'il est déjà livré (sur mika#2854 il l'était : 11 cas verbatim
  et leur témoin dégénéré, sur `main` depuis le vol 3) ;
- **la commande qui le joue**, et la voie admise pour la jouer ;
- **la règle d'interprétation du résultat** — sur mika#2854, *un rouge est un constat à
  rendre à l'opérateur, jamais une invitation à retoucher le gabarit* (discipline
  mika#1190).

Symétriquement, **l'instrument doit pointer son entrée de registre**. Qui lance la suite au
réveil part du test, pas du registre : s'il ne trouve dans le message de saut que la
commande à rejouer, il n'apprend pas qu'il existe une procédure, ni laquelle. Le pointeur
va aux deux endroits.

## Comment appliquer ça au prochain dormeur

1. Écrire la condition, puis lui appliquer les **deux** tests : *tranchable sans contexte*
   **et** *satisfiable sans le travail qui dort*.
2. Si la condition mentionne un effet du travail, la réécrire sur le bloqueur en amont.
   Nommer ce bloqueur (ticket, version, date) plutôt que l'état qu'il empêche.
3. Vérifier le bloqueur par une commande, et écrire **cette** commande dans la condition.
4. Fermer le ticket et porter la ligne dans `docs/dormeurs.md`, avec son « Au réveil, … ».
   Ne pas se contenter de corriger le corps du ticket : un ticket ouvert reste
   dispatchable par les portes qui ne lisent pas la dormance, et elles sont la majorité.
5. Faire pointer l'instrument vers l'entrée, si un instrument existe.

Refs mika#2854, mika#2680, mika#2952, mika#2263, senara-solutions/claude-pilot#290.
