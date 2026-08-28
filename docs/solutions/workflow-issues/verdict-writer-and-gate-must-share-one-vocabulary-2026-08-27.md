---
module: dispatch-lib, mika-agent/skills/executor
tags: [dispatch-gate, grooming, loop-substrate, idempotency, lockstep]
problem_type: silent-invisibility-loop
category: workflow-issues
---

# Quand l'écrivain d'un verdict et son lecteur n'ont pas le même vocabulaire

**Incident fondateur : mika#2012, 2026-08-27.** 25 requeues mesurés sur 5 tickets
en 13 h, produisant 6 branches ne contenant que des plans markdown. Aucune
erreur, aucun log d'échec, aucune alarme. La boucle tournait — elle ne
produisait simplement rien.

## Le mécanisme

Un gate de dispatch lit le corps d'un ticket pour décider s'il est groomé
(`check_grooming_markers`, `executor.rs`). Une fonction séparée écrit ce corps
(`_write_canonical_callout`, `dispatch-lib.sh`). Les deux vivent dans des
langages différents, des dépôts logiques différents, et rien ne les relie.

L'écrivain connaissait deux formes de verdict. Le processus de grooming en
produit trois : la sortie « first-pass READY » est légitime
(`/mika-groom-ticket` Phase 3 étape 10), et n'avait aucun stage. L'écrivain
tombait dans son `*)`, retournait 1 avec un simple `WARN` sur stderr, et
n'écrivait rien.

Conséquence en chaîne :

1. Aucun verdict dans le corps → le gate ne voit pas le ticket comme groomé.
2. Le ticket reste éligible → il est re-dispatché en `dev-groom`.
3. Le grooming re-tourne, re-génère un plan, et **empile** un second callout.
4. Retour à l'étape 1, indéfiniment.

## Ce qui rend cette classe si difficile à voir

**L'échec est silencieux par construction.** Un `return 1` sur une condition
prévisible ne réveille personne. Le seul symptôme observable est une absence :
des dispatches qui tournent sans que le compteur de PR bouge. Il faut compter
des requeues pour le voir, et personne ne compte les requeues.

**Le re-grooming ressemble au grooming.** Rien dans les logs ne distinguait un
deuxième passage d'un premier. C'est ce qui a permis aux 13 heures de passer.

## Les règles qui en sortent

**1. Un écrivain et son lecteur partagent un vocabulaire déclaré, testé des deux
côtés.** Ce n'est pas une convention de style : toute forme que le lecteur
accepte et que l'écrivain ne reconnaît pas, ou l'inverse, produit une boucle.
Le sens de l'écart détermine seulement laquelle des deux boucles vous obtenez.
Ici, l'écart initial était `second-pass \(GROOMED\)` côté écrivain contre
`second-pass \(GROOMED[\s\)\.,;:—-]` côté lecteur — l'écrivain manquait
`(GROOMED — session-id: …)`, la forme qu'il émettait lui-même.

**2. Un cas `*)` inatteignable en théorie doit être bruyant en pratique.** La
branche par défaut n'est pas de la paperasse défensive : c'est l'endroit où le
vocabulaire non partagé se manifeste. Elle doit nommer la conséquence, pas
seulement l'erreur — `NO VERDICT WRITTEN, ticket will re-groom` plutôt que
`unknown stage`.

**3. Ne jamais préfixer un bloc structurel — remplacer.** Préfixer paraît sûr
(on ne détruit rien). Il produit des corps où deux blocs se contredisent et où
le lecteur humain ne sait pas lequel fait foi. Le chemin de plan périmé du bloc
ancien survit à la branche qu'il nommait.

**4. Le remplacement se borne au préambule.** Un ticket qui *documente* le
format cite ces lignes exactes plus bas dans son corps — celui de mika#2012 le
fait. Un `grep -v` appliqué à tout le corps supprime silencieusement cette
documentation en « corrigeant » une mise en forme.

**5. Un gate de refus prouve, il ne devine pas.** Le gate de #2012 refuse un
re-grooming quand un plan est *committé sur la branche*, pas quand le corps
*mentionne* un plan. La version naïve refuserait le grooming d'un ticket dont
le plan n'a jamais été poussé — l'échouant définitivement. C'est strictement
pire que la boucle : une boucle gaspille des dispatches, un ticket échoué n'est
jamais travaillé. **Dans le doute, laisser passer.**

**6. Ne jamais lire `FETCH_HEAD` dans un dépôt à dispatches concurrents.** C'est
un fichier unique de `$GIT_DIR` partagé par tout processus touchant le checkout.
mika#1001 autorise un `implement` et un `groom` simultanés sur le même
sous-dépôt : un fetch voisin entre le fetch et le `cat-file` fait tester l'arbre
d'une autre branche. Fetcher dans une ref nommée par la branche
(`refs/dispatch-gate/<branch>`) — déterministe, non partagée.

**7. Le second passage doit crier différemment du premier.** Un re-grooming
autorisé mais anormal (le corps revendique un plan absent de la branche) émet
son propre signal, distinct du refus. Sans quoi les deux populations sont
indistinguables dans les logs — et c'est exactement ce qui a coûté les 13 heures.

## Sémantique de sortie

Rappel couplé, hérité de mika#988 : sur une condition **prévisible**, la sortie
est `_deliver_callback` + `exit 0`, jamais `exit 1`. Un `exit 1` est emballé en
`HANDLER CRASH` par le trap EXIT ; mika-dev lit l'enveloppe de crash, pose une
question de confirmation, et s'immobilise (stall de 7 h le 2026-05-06).
`exit 1` est réservé aux bugs réels du handler.

## Vérification

- `skills/bundled/_shared/test-dispatch-lib.sh` — 345 assertions, dont un test
  témoin qui conserve l'ancien pattern et prouve qu'il ratait la forme
  canonique, et un test qui prend un corps à deux blocs empilés pour vérifier
  qu'il n'en reste zéro, contenu réel intact.
- `crates/mika-agent/src/skills/executor.rs` — 17 tests `test_grooming_markers`,
  dont le garde-fou qui exige qu'un `first-pass (READY)` **nu** échoue toujours :
  c'est la disposition émise en cours de grooming, avant que le plan soit
  committé. L'accepter dispatcherait des tickets sans plan sur la branche.
