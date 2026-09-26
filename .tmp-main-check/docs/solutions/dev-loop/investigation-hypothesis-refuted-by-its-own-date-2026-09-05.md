---
module: skills/bundled/_shared/dispatch-lib
tags: [investigation, dev-groom, iterate-loop, hypothesis-dating, diagnostic-honesty, unreachable-code, mika-1772]
problem_type: best-practice
category: dev-loop
---

# Une hypothèse d'investigation vieillit, et sa date la réfute avant toute mesure

## Problème

mika#1772 a été ouvert le 2026-07-13 pour expliquer un échec de convergence de
`_iterate_groom_loop` survenu le 2026-07-04 sur mika#1723 : un plan **existait**
sur la branche, la boucle rendait non-zéro, et un `/mika-ask-arch` manuel
convergeait `GROOMED` en une passe le même jour.

Le ticket proposait trois candidats et une procédure d'investigation. Sept
semaines plus tard, au moment de l'implémenter :

- **candidat A** — « la session second-pass ne référence pas la first-pass » :
  le fil de session existe depuis `1eb5a034` (2026-05-25, mika#1271), **40 jours
  avant** l'incident ;
- **candidat B** — « la boucle rejette les paraphrases connues de l'architecte » :
  le parsing flou deux-paliers existe depuis `00c73aa2` (2026-05-27, mika#1272),
  **38 jours avant** l'incident ;
- **étape d'investigation 1** — « `grep` sur `server.log` … 14 hits » : `server.log`
  ne reçoit aucun stderr de `dispatch-lib`, et le fichier le dit de lui-même
  (`dispatch-lib.sh:1387`).

Deux candidats sur trois étaient **déjà dans le code au moment de l'incident
qu'ils prétendaient expliquer**. Le mécanisme qu'ils proposaient d'ajouter était
là quand les faits ont eu lieu. Aucune investigation n'était nécessaire pour le
savoir : deux `git log -S` suffisaient — soit quelques secondes, contre sept
semaines d'attente et une procédure qui aurait envoyé l'implémenteur fouiller
18 Go pour une ligne qui n'y a jamais été écrite.

## Cause racine

Un ticket d'investigation fige une liste de causes plausibles à l'instant où il
est écrit. Le code, lui, continue d'avancer. Rien dans le format « candidat A /
B / C » ne porte de date, donc rien ne signale que la moitié de la liste a cessé
d'être plausible avant même d'avoir été lue.

Le piège n'est pas l'erreur de diagnostic : c'est que la liste **paraît toujours
aussi valable** sept semaines plus tard. Elle est écrite au présent, elle nomme
des mécanismes réels, elle se lit comme une hypothèse ouverte. La seule chose
qui la réfute est extérieure au ticket — la date du commit qui a introduit le
mécanisme, comparée à la date de l'incident.

## Parade

**Dater les candidats contre l'historique du dépôt avant de les classer.** Pour
chaque cause proposée qui suggère d'*ajouter* un mécanisme, une question, avant
toute mesure :

> Ce mécanisme existait-il déjà le jour de l'incident ?

```bash
git log -S'<le mécanisme>' --format='%h %ad %s' --date=short -- <fichier>
```

Si le commit est **antérieur** à l'incident, le candidat est réfuté sans mesure.
Il ne coûte pas une investigation, il coûte une commande.

Ce test se fait au **grooming**, pas à l'implémentation : c'est là qu'il retire
des voies mortes du chemin de quelqu'un d'autre. Un plan qui ouvre sur trois
candidats dont deux sont datables doit les dater.

## Corollaire — d'où vient vraiment la fermeture

Ce qui a fermé la classe du 2026-07-04 n'est pas un correctif propre à
mika#1772. C'est **`f8b63530` (2026-07-25, mika#1823)** : un ré-essai borné de
la première passe quand l'architecte répond sans ligne `Disposition:`
— 1 tentative + 1 reprise, session préservée. Il a été écrit pour un autre
ticket, trois semaines après l'incident, sans que personne le rattache à
mika#1772. Mesuré : en neutralisant ce ré-essai, la boucle rend non-zéro sur la
signature du 2026-07-04 ; en le rétablissant, elle converge (`test-dispatch-lib.sh`,
bloc « the 2026-07-04 UNPARSED signature », cas A).

**Un ticket peut être résolu par un correctif adjacent, et ce qui manque alors
est la preuve, pas le code.** D'où la fixture : sans elle, rien n'échouait
quand on retirait le mécanisme qui ferme la classe, et la protection était
silencieusement révocable.

## Second enseignement — « injoignable » est une mesure, pas une intuition

Le seul défaut vivant restant dans mika#1772 vient de là. `f8b63530` a laissé
dans la boucle une branche `*)` annotée :

> *Unreachable after the retry loop above (mika#1823), which returns 1 explicitly
> on double-UNPARSED. Kept for safety — invariant violation if reached.*

Les deux affirmations sont fausses. La boucle de ré-essai ne fait **pas**
`return 1` sur double-UNPARSED : elle sort du `for` avec `$disposition` vide et
atterrit précisément dans cette branche. Et comme `_parse_disposition` n'émet
que `READY|ITERATE|ESCALATE` ou rien, cette branche ne tire **que** sur le
double-UNPARSED — ce n'est pas un filet de sécurité, c'est le terminal nominal
de la boucle.

Conséquence, et c'est la classe de défaut que mika#1772 dénonce, survivant à son
propre correctif : PR#2028 avait rendu chaque `return 1` porteur d'une raison
*mesurée*, remontée jusque dans `tasks.result`. Ce site-là écrivait
« invariant violation » — envoyant l'opérateur chercher un bug dans la boucle au
lieu de lire « l'architecte n'a pas émis la ligne, deux fois », qui est à la
fois le fait et le conseil (un re-kick le répétera, le ré-essai a déjà tourné).

**Un commentaire « unreachable » est une hypothèse comme une autre.** Il se
teste : on branche le harnais sur la fonction réelle et on regarde. Ici, quinze
lignes de sonde ont suffi à montrer que la branche tire, et à faire la
différence entre « la boucle a un bug » et « l'architecte n'a pas répondu ».

## Ce qui n'a pas pu être établi, et pourquoi c'est écrit ici

La preuve historique que réclamait le critère (a) du ticket **n'existe plus** :
le worktree du 2026-07-04 est détruit avec son `groom-verdict-trail.log`,
`GROOM_LOOP_FAILURE_REASON` n'existait pas encore, et le stderr de `dispatch-lib`
n'a pas de puits durable. La classification ci-dessus repose donc sur la
réfutation datée et sur la reproduction en fixture, pas sur la trace d'origine.

C'est une réponse, pas un échec — mais elle doit être dite. Une fermeture qui
tait l'absence de sa preuve fabrique la même divergence non auditée que celle
que ce ticket reproche au callback.

## Voir aussi

- `docs/solutions/2026-05-21-groom-post-flight-recovery-without-architect-verdict.md`
  — le § 80 identifiait le ré-essai comme remède, deux mois avant `f8b63530`.
- `docs/solutions/dev-loop/parse-verdict-tier-1b-symmetric-carryover-tolerance-2026-07-22.md`
  — la classe voisine : convergence perdue sur une divergence de *forme*, pas de fond.
