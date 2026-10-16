---
title: Une échéance murale ne remplace pas un signal de fin
date: 2026-10-15
category: best-practices
module: crates/mika-agent/src/skills
problem_type: best_practice
component: agent-core
severity: medium
tags: [tests, flaky, tokio, join-handle, anti-vacuite, ci, mika-2677]
applies_when:
  - "Un test attend l'effet d'un `tokio::spawn` et sonde la base jusqu'à une échéance"
  - "Un test est rouge en CI et vert sur relance, sans changement de code"
  - "Une aide de test s'appelle `await_*` et porte un `Instant::now() + …`"
  - "Un contrôle négatif conclut sur une ABSENCE après un sommeil"
---

# Une échéance murale ne remplace pas un signal de fin

## Le problème

`skills::executor::tests::mika2532::the_stderr_of_a_crash_on_a_terminal_row_is_persisted`
a échoué deux fois en CI — PR #2659 le 2026-10-05, PR #2676 (run 37582008160,
tentative 1) le 2026-10-07 — et est passé au vert sur relance les deux fois,
sans changement de code. Ni l'une ni l'autre PR ne touchait
`skills/executor.rs`.

Le test lançait un handler via `spawn_long_running_exec`, qui crée un task de
surveillance avec `tokio::spawn` et **jetait le `JoinHandle`**. Faute de signal
de fin, l'aide `await_handler_failure` sondait la ligne de tâche toutes les
20 ms jusqu'à une échéance à l'horloge murale, `SETTLE_MS = 4_000`. Sous charge
CI — tests multi-thread en parallèle, runner partagé — le sous-processus et
l'écriture en base peuvent dépasser 4 s. L'aide rendait `None` et le test
échouait sur `the cause of a pre-result crash must reach the row`, alors que le
comportement testé était correct.

La même constante existait à l'identique dans `mod mika2634`, avec en plus un
`NEGATIVE_CONTROL_MARGIN_MS = 250` que deux contrôles négatifs dormaient avant
de conclure sur une absence.

## La lecture

**Une échéance murale met la charge du runner sur le chemin critique du
verdict.** Sa valeur doit excéder la durée réelle de l'opération ; la durée
réelle dépend de la machine ; donc la machine a une voix dans le vert. C'est
une configuration qui a l'air d'une marge de sécurité, et dont le seul mode de
défaillance est le faux rouge — le plus coûteux, parce qu'il consomme une
fenêtre de recette par relance et qu'il entraîne à relancer plutôt qu'à lire.

Le signal existait déjà. `tokio::spawn` **rend** un `JoinHandle`, et
`spawn_long_running_exec` le jetait. Laisser tomber un `JoinHandle`
**détache** le task, il ne l'annule pas : le retourner ne change donc rien pour
les appelants qui l'ignorent. Le coût de la correction était d'un `;` retiré et
d'un type de retour.

**Et l'échéance empoisonne aussi les contrôles négatifs**, d'une manière plus
discrète que le faux rouge. Un `is_none()` après un sondage épuisé est
ambigu : il peut signifier « l'écriture n'a pas eu lieu » (ce que le contrôle
veut établir) ou « le chemin n'avait pas encore tourné » (une absence
d'observation, classe mika#2205). Le runner chargé est exactement la
circonstance qui produit la seconde lecture — donc le contrôle censé attraper
un faux positif serait le premier à cesser d'attraper quoi que ce soit, en
silence. Après un join, le même `None` est une **absence constatée sur un task
terminé**, et il n'y a plus qu'une lecture.

## La règle

Quand un test attend l'effet d'un task asynchrone :

1. **Cherche le signal de fin avant d'écrire une échéance.** Un `JoinHandle`,
   un canal, une notification, une écriture observable. L'absence apparente
   d'un signal est souvent un handle jeté en amont, pas une fatalité de forme.
2. **Joins, puis lis une fois.** Une aide nommée `await_*` qui boucle sur
   `Instant::now()` devient une aide `read_*` sans boucle. Le gain n'est pas la
   vitesse (ici 4 s → 0,05 s par module, accessoirement) : c'est que le verdict
   ne dépend plus d'une horloge.
3. **Un `handle.await` nu n'est pas la réponse.** Un sous-processus réellement
   pendu ferait pendre le binaire de test jusqu'au timeout du job CI — un rouge
   moins lisible qu'un panic. Enveloppe le join dans un `tokio::time::timeout`
   avec un budget large. La différence avec `SETTLE_MS` est structurelle, pas
   une question de valeur : ce budget n'est atteignable que par une panne
   franche, le chemin normal ne s'en approche jamais, et il n'est donc pas sur
   le chemin critique du vert.
4. **Rends le refus de la barrière observable.** Une aide qui avalerait son
   timeout rendrait verts par défaut d'observation tous les tests qui
   l'appellent — la classe mika#2205, déplacée d'un cran vers l'outillage.
   Paramètre le budget et consacre un `#[should_panic]` à un handler
   délibérément pendu.
5. **Le join remplace le sommeil des contrôles négatifs, mais pas leur
   anti-vacuité.** La barrière qui établit que le chemin a tourné reste
   nécessaire ; seul son corps devient une lecture unique. Ce qui change est la
   cause nommée : plus « le chemin n'a pas tourné en 4 s », mais « le task a
   joint sans avoir écrit la clé ».

## La limite du join

**Un join est une barrière complète pour ce que le task `await` dans son propre
corps — et pour rien de plus.** Un `tokio::spawn` imbriqué ajouté plus tard
rouvrirait le trou en silence : le join retournerait avant l'écriture du task
imbriqué, et les tests redeviendraient dépendants d'une course sans qu'aucune
ligne de test n'ait changé. La mitigation est le doc-comment de l'aide, qui le
dit à l'endroit où un tel ajout se lirait. Si vous en ajoutez un, l'écriture
qu'il porte a besoin de son propre signal.

## Références

- Ticket : senara-solutions/mika#2677
- Code : `crates/mika-agent/src/skills/executor.rs` — `spawn_long_running_exec`,
  `join_handler_monitor`, `join_handler_monitor_within`
- Classe voisine : [un plancher d'anti-vacuité peut se rembourrer avec le trou
  qu'il garde](un-plancher-danti-vacuite-peut-se-rembourrer-avec-le-trou-quil-garde.md)
