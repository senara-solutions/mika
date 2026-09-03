---
module: mika-agent/auto_pull + skills/bundled/_shared/dispatch-lib.sh
tags: [loop-substrate, livelock, predicate-drift, grooming, auto-pull, dispatch, cross-language]
problem_type: logic_error
category: dev-loop
created: 2026-09-03
ticket: mika#2158
---

# Deux prédicats pour une seule notion, dans deux langages — et le refus qui ne produit rien

## Le problème

La boucle autonome demandait un grooming toutes les dix minutes pour trois tickets **déjà groomés**, et le dispatcher le refusait à chaque fois. Douze refus en trois heures et demie, sans qu'aucun état ne change entre deux tours. Pendant ce temps, trois tickets `p1` portant chacun un plan committé et validé par l'architecte n'étaient **jamais** dispatchés en implémentation.

Ce n'est pas une panne : les deux moitiés faisaient exactement ce pour quoi elles sont écrites.

## La cause

Deux fonctions répondent à la question « ce ticket est-il groomé ? ». Elles vivent dans deux langages, lisent deux surfaces différentes, et ne sont reliées par rien.

| | où | ce qu'il lit | verdict observé |
|---|---|---|---|
| `is_groomed()` | `crates/mika-agent/src/auto_pull.rs:301-314` (Rust) | la **prose** du callout `> - **Grooming history:**` dans le corps de l'issue, via une regex exigeant `second-pass (GROOMED` | **non groomé** |
| garde de dispatch | `skills/bundled/_shared/dispatch-lib.sh:1831` (Bash) | le **fichier de plan** résolu sur la branche de dispatch, et son entête ne réclamant pas un autre ticket | **déjà groomé** |

Le premier alimente la promotion (`auto_pull` décide qui passe en `implement`). Le second garde le dispatch de grooming. Le premier dit non, donc le feeder redemande ; le second dit « c'est déjà fait », donc rien ne se passe ; le verdict du second n'atteint jamais le premier. **Livelock.**

### Les trois formes qui font échouer le prédicat de prose

Mesuré sur les corps réels des six tickets `ready` (3 vrais, 3 faux) :

1. **Une première passe `READY` n'écrit jamais « second-pass ».** Et ce n'est pas une négligence : `.claude/commands/mika-groom-ticket.md` phase 3 étape 10 prescrit *« skip to Phase 5 »* quand l'architecte n'a rien à redire. Le prédicat exige la trace d'un aller-retour que la spec supprime quand le plan est bon du premier coup — **mieux le grooming se passe, moins le ticket est visible.**
2. **La langue.** `mika-arch seconde passe (GROOMED, session …)` — verdict correct, forme française. Le dépôt écrit ses tickets et ses plans en français ; le prédicat n'accepte que l'anglais.
3. **Un `GROOMED` rendu après arbitrage.** `second-pass (ESCALATE, périmètre) → arbitrage rendu → mika-arch (GROOMED)`. L'état final est le bon ; `GROOMED` ne suit simplement pas `second-pass (`. Le prédicat lit un ordre de mots, pas un état.

## La solution

**Mesurer l'artefact, pas son récit.** La garde Bash a raison parce qu'elle résout un fichier et lit son entête ; le prédicat Rust se trompe parce qu'il note la façon dont l'histoire a été racontée. Quand deux surfaces répondent à la même question, l'une d'elles doit être la source de vérité, et le test croisé doit exister.

Deux exigences, au-delà de la correction de la regex :

- **Les deux prédicats doivent s'accorder sur les mêmes entrées**, et un test croisé sur des corps figés doit échouer en cas de désaccord. Sans ça, corriger l'un des deux ne fait que déplacer la divergence.
- **Un refus doit produire un effet.** Quand la garde répond `already_groomed`, soit le ticket est promu dans la foulée, soit il est marqué pour ne pas être redemandé au tour suivant. *Un refus qui laisse l'état inchangé et se répète toutes les dix minutes n'est pas une garde, c'est une boucle.* Critère de non-régression : deux tours consécutifs du feeder ne doivent pas produire deux refus identiques sur le même ticket.

Le plus troublant : le refus **contenait déjà le remède**, en toutes lettres dans sa propre note — *« Dispatch dev-pilot to implement, or remove the plan from the branch to force a fresh groom. »* Le système savait quoi faire. Il n'avait aucun chemin entre ce qu'il sait à un endroit et ce qu'il décide à l'autre.

## Trois instances dans la même nuit — le motif, pas l'anecdote

Le 2026-09-03, **trois** couples de composants ont été mesurés en désaccord sur une même question, chacun dans un mécanisme différent de la boucle :

| question | qui répond A | qui répond B | écart mesuré |
|---|---|---|---|
| « ce ticket est-il groomé ? » | `is_groomed()` lit la **prose** du callout → non | garde de `dispatch-lib` résout le **fichier de plan** → oui | 12 refus en 3 h 30, 3 `p1` jamais implémentés (mika#2158) |
| « le grooming est-il le goulot ? » | le journal l'**affirme** (`// R7`) | le code ne peut pas le savoir : `candidates.is_empty()` couvre deux cas opposés | a produit l'action inverse de celle qui était due (mika#2161) |
| « le créneau est-il libre ? » | le **bail** expire en 120 s → libre | la **garde** lit le rappel actif → occupé | 4 h 12 de divergence, 8 réveils pour rien (mika#2162) |

Aucune des six moitiés n'est buggée prise isolément. Chacune fait ce que son commentaire annonce. Ce qui manque à chaque fois est le **même** : rien ne réconcilie les deux réponses, et rien ne casse quand elles divergent.

C'est pour ça que le motif se répète : il ne laisse pas de trace d'erreur à corriger. On ne le trouve qu'en comptant des répétitions dans un journal — douze refus, huit réveils — c'est-à-dire en cherchant une régularité, pas une panne.

## La classe, réutilisable

**Un prédicat qui teste une formulation déguisée en test d'état.** Le signe distinctif : il exige une chaîne de caractères produite par un humain ou un agent, alors que la propriété qu'il prétend mesurer a une trace matérielle ailleurs (un fichier, une ligne en base, un ref git). À chaque fois que la formulation change légitimement — une langue, un chemin plus court, une étape sautée parce qu'elle était inutile — le prédicat rend faux sur un état vrai.

Le second signe, plus dur à voir : **la divergence ne se manifeste pas comme une erreur.** Les deux côtés loguent sereinement, chacun cohérent avec lui-même. Ce qu'on observe est une absence — du travail qui n'arrive jamais — et une absence ne déclenche aucune alerte. C'est en comptant les refus répétés dans le journal qu'on la voit, pas en cherchant une panne.

## Comment on l'a trouvé

En posant une prédiction **avant** de regarder : « si ces trois tickets sont invisibles au prédicat, le moteur doit les redemander en grooming ». Puis en lisant les tâches. Les trois prédits, exactement les trois observés ; et les trois qui passent le prédicat partaient bien en `implement`.

La première hypothèse — « ce défaut fabrique les onze sessions de grooming vivantes sur la machine » — était **fausse**, et vérifier a coûté une commande : ces dispatchs ne lancent aucune session, la garde les refuse avant. Deux faits voisins dans le temps ne sont pas un lien de cause. Voir aussi `docs/solutions/best-practices/run-the-new-check-against-live-state-before-calling-it-done-2026-08-29.md`.

## Références

- `crates/mika-agent/src/auto_pull.rs:301-314` — le prédicat de prose
- `skills/bundled/_shared/dispatch-lib.sh:1831` — la garde qui lit l'artefact
- `.claude/commands/mika-groom-ticket.md` phase 3 étape 10 — la spec qui supprime la seconde passe sur `READY`
- mika#2158 — le ticket, avec le tableau des six corps mesurés
- mika#2120 — le frère sur l'autre axe du même prédicat (préfixe du chemin de plan)
- mika#2020 / mika#1887 — `PlanOwnership`, la classe voisine : un callout qui pointe vers le plan d'un autre ticket
- mika#2161 — deuxième instance : le journal du feeder affirme un goulot qu'il ne peut pas connaître
- mika#2162 — troisième instance : le bail de créneau expire en 120 s quand la garde tient 4 h
