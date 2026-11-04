---
title: "Un signal de vivacité ne se lit pas dans un fichier écrit à la mort"
date: 2026-11-04
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh
component: development_workflow
problem_type: logic-error
severity: high
symptoms:
  - "`API Error 5xx` dans le `.log` d'un vol, trois minutes après un `pilot_egress_startup.begin` du relais"
  - "`pilot_daemon_freshness.stale` suivi d'une relance du démon alors qu'un pilote stream — 7 fois sur 63 démarrages du relais, 6 en PIPELINE FAILURE"
  - "un garde-fou de déférence VERT en test et faux en production, sur une fixture que la production ne produit jamais"
  - "`find -newermt '-300 seconds'` sur `*.stderr` ne rendant rien pendant tout un vol, et rendant quelque chose juste après"
root_cause: liveness_read_from_post_mortem_artifact
resolution_type: code-fix
tags: [mika-3031, mika-2029, mika-2780, claude-pilot, dispatch-lib, vivacite, deference, daemon-freshness, fixture, newermt, dev-loop]
---

# Un signal de vivacité ne se lit pas dans un fichier écrit à la mort

## La panne, en une phrase

`_pilot_sibling_pilot_in_flight` devait répondre « un pilote frère vole-t-il ? »
avant de tuer un démon d'egress. Il cherchait la réponse dans un `*.stderr` de
moins de 300 s sous `/var/log/claude-pilot`. Or le `.stderr` d'un pilote est
copié **après** `PILOT_EXIT=$?` : il naît à la **sortie**. Le signal était donc
faux pendant **toute** la durée de chaque vol, et vrai pendant les 300 s qui
**suivent** un vol fini — exactement à l'envers de ce qu'il prétendait dire.

Sept relances mesurées sont tombées sur un pilote vivant. Les **sept** portent un
`API Error 5xx` dans le `.log` du vol, 3 à 6 min après la coupure du tunnel
CONNECT ; **six** ont fini en `PIPELINE FAILURE`, la septième a survécu.

## La leçon transférable

**Un artefact écrit à la mort d'un processus ne peut pas témoigner de sa vie.**
La question « est-ce vivant ? » se lit sur ce que le processus écrit *pendant*
qu'il vit, jamais sur ce que son épilogue dépose. Quand les deux fichiers portent
le même préfixe et vivent dans le même répertoire, rien dans le code ne distingue
l'un de l'autre : il faut avoir regardé *quand* chacun naît.

Le geste qui tranche tient en une commande, et il n'a pas été fait avant d'écrire
le prédicat :

```sh
stat -c '%n %w %y' /var/log/claude-pilot/<id>.log /var/log/claude-pilot/<id>.stderr
```

Trois vols mesurés : le `.log` naît au lancement, le `.stderr` 4 min 32 s,
6 min 34 s et 7 min 50 s plus tard — et sa naissance tombe à la seconde sur la
fin du pilote, chaque fois.

### La bonne forme : l'appariement, pas la fenêtre

Le signal corrigé ne regarde plus un âge :

> il existe un `<id>.log` dont le `<id>.stderr` n'existe pas.

Vrai pendant tout le vol, **quel que soit son âge**. Et c'est bien l'âge qu'il
fallait abandonner, pas seulement le fichier : le mtime d'un `.log` suit les
**tours** du pilote, non le temps mural. Trois intervalles de plus de 300 s ont
été mesurés sur un seul vol, dont un de **602 s**. Un prédicat « `.log` écrit
récemment » aurait déplacé la panne au lieu de l'éteindre — et il aurait été vert
en test, parce qu'une fixture fraîche n'a jamais de silence de dix minutes.

### Le correctif a sa propre panne symétrique, et elle se nomme

Un appariement **sans** plafond d'ancienneté est faux dans l'autre sens : un vol
tué par `SIGKILL` ne copie jamais son `.stderr`, donc son `.log` resterait un
« vol en cours » pour toujours et gèlerait la relance — c'est-à-dire mika#2780
réintroduit par le correctif de mika#3031. L'hôte portait **878** `.log`
orphelins au moment de la mesure.

D'où un **plafond d'ancienneté** de 1 800 s, dérivé plutôt que choisi :

- **plancher** — le plus grand silence qu'un pilote *vivant* s'autorise, qu'il
  écrit lui-même dans son `.log` : `toolWaitCeiling` = `rateLimitCeiling` =
  1 800 s. Trois fois le plus long silence mesuré, 602 s.
- **plafond** — 553 fois sous le plus jeune orphelin *mort* de l'hôte, 11,5 jours.

Trois ordres de grandeur entre les deux bornes : le placement ne demande aucun
arbitrage fin. Et c'est le signe qu'un nombre dérivé vaut mieux qu'un nombre
plausible — celui qu'il remplace, `300`, était présenté comme l'`idle_timeout` du
pilote, qui vaut en réalité 480 s. Le commentaire citait un budget que le pilote
ne porte pas.

## Pourquoi les tests étaient verts

Les deux tests de déférence fabriquaient un vol par `touch
"$tmp/pilotlogs/frere.stderr"`. Ils étaient donc verts sur un état que la
production **ne produit jamais pendant un vol**, et muets sur le seul qu'elle
produit. Un garde-fou peut rester vert pendant 17 jours sur une fixture qui
n'existe pas.

**La règle qui en sort : une fixture d'état de processus se construit à partir
d'un `stat` de l'état réel, pas à partir de ce que le prédicat lit.** Écrire la
fixture en miroir du prédicat garantit le vert et ne mesure rien — c'est le même
piège que le contrôle positif absent de
[un-essai-dabsence-exige-un-controle-positif-sur-son-instrument](un-essai-dabsence-exige-un-controle-positif-sur-son-instrument.md),
pris par l'autre bout.

Le correctif a donc commencé par un commit **délibérément rouge** : les fixtures
passent à l'état mesuré, et les cinq assertions tombent en nommant la panne du
ticket.

```
✗ V3  : attendu stale_deferred launches=1 — obtenu stale launches=2 first=dead
✗ V3b : idem, sur un vol de 600 s
✗ V3c : attendu stale — obtenu stale_deferred (un pilote SORTI diffère encore)
```

`launches=2 first=dead`, c'est le démon tué sous un pilote vivant : les 7
relances du ticket, reproduites à la demande.

## La trace secondaire : une mesure de grooming qui ne se rejoue pas

Le plan de ce vol portait deux mesures sur les formes de `-newermt`, et **aucune
des deux ne s'est reproduite** dans le bac à sable du pilote. Elles sont
consignées ici parce que la manière dont elles ont été fausses est instructive.

| mesure du plan | ce qui a été re-mesuré |
|---|---|
| M2 — `bfs` refuse `-newermt "-1800 seconds"` (`Invalid timestamp`) | `bfs 4.1.1` l'**accepte** et l'interprète **correctement** |
| M3 — `@epoch` est mal interprété | `@epoch` est **correct** sous `bfs` comme sous GNU findutils 4.10.0 |

M3 n'était pas une erreur de `find` mais une **erreur d'arithmétique dans les
libellés** du tableau : `@1762244000` est en novembre **2025**, non 2026, et
`@1793780000` est ≈ l'instant de la mesure, non « un an dans le futur ». `find`
avait raison ; les deux colonnes de gauche étaient fausses. Une mesure dont la
colonne *attendu* est calculée à la main est une mesure dont il faut vérifier la
colonne attendue — et le re-calcul tient en un `date -u -d @<n>`.

**Conséquence sur le correctif, et c'est la partie à retenir :** la forme ISO 8601
absolue a été retenue quand même, mais sa justification a changé de nature. Ce
n'est pas un défaut reproduit, c'est un **durcissement de portabilité** — l'ISO
absolu est la seule forme correcte sous *toutes* les implémentations mesurées et
la seule que GNU documente, pour le coût d'un `date`. La différence compte : une
fausse mesure écrite dans un commentaire de code y survit des années et se
propage au prochain lecteur, qui n'a aucune raison de la rejouer. Le commentaire
du correctif dit donc « la seule forme que GNU documente », jamais « les autres
sont mal interprétées ».

Le test de bonne foi correspondant a été écrit en conséquence : il mesure que
l'ISO absolu est exact **dans les deux sens** sur un mtime connu (l'assertion qui
compte, puisqu'un `-newermt` mal interprété ne lève aucune erreur et rend
simplement le mauvais ensemble), et consigne `@epoch` en regard sans lui prêter de
tort. La cause « un `find` hostile » est tenue par une **surcharge** `find() { … }`
dans le sous-shell de la sonde, jamais par le `find` de l'hôte de CI — sinon le
test serait vert là où la forme relative est acceptée et n'aurait jamais tiré sur
celle qui la refuse.

## Un troisième jeton plutôt qu'un repli muet

Passer en ISO absolu introduit une dépendance : `date -u -d`, qui n'avait aucun
précédent dans `dispatch-lib.sh` (son *format* en a cinq, son option `-d` aucun).
Un `date` non-GNU la ferait échouer.

Les deux replis muets sont faux dans les deux sens — rendre « un frère vole »
gèle la relance pour toujours, rendre « aucun » réintroduit en silence la panne de
ce ticket. Le prédicat a donc **trois** sorties, et le verdicteur mappe la
troisième sur `undecidable` : démon conservé, cause nommée sur stderr. Même
raison que la branche par défaut du `case` de la découverte du porteur, écrite
pour mika#2029.

Détail d'implémentation qui vaut d'être noté, parce qu'il a failli coûter un
détecteur : un test existant épingle que le verdicteur écrit **exactement trois**
lignes nommées. Une quatrième l'aurait rougi. Les deux causes d'indécidabilité
sortent donc par la **même** émission, avec leur cause en variable — ce qui
supprime une duplication au lieu d'en ajouter une, et laisse le détecteur armé.

## Sites

| quoi | où |
|---|---|
| le prédicat corrigé | `skills/bundled/_shared/dispatch-lib.sh`, `_pilot_sibling_pilot_in_flight` |
| le plafond d'ancienneté et sa dérivation | même fichier, `_PILOT_DAEMON_SIBLING_WINDOW_SECONDS` |
| le mappage de la troisième sortie | même fichier, `_pilot_daemon_freshness` |
| les fixtures d'état de vol | `skills/bundled/_shared/test-dispatch-lib.sh`, `_pilot_flight_fixture` |
| les trois tests de bonne foi | même fichier, bloc « mika#3031: bonne foi » |
| les deux scans structurels | même fichier, bloc « mika#3031: deux scans structurels » |
| la lecture opérateur des cinq jetons | `docs/operator/pilot-egress-relay.md` § 4.3 |
