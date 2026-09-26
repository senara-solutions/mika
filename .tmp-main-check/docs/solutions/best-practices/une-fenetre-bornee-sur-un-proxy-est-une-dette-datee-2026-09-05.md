---
title: Une fenêtre bornée sur un proxy est une dette datée, pas une réparation — bornez-la, comptez l'épargne, nommez la mesure qui la retire
date: 2026-09-05
category: best-practices
module: crates/mika-agent/src/task_engine/engine.rs, crates/mika-agent/src/db.rs
tags: [reaper, liveness, proxy, predicate-drift, telemetry, fail-closed, threshold, mutation-testing, env-var-clamp]
problem_type: silent-logic-error
component: task-engine
severity: high
applies_when:
  - "Un prédicat discrimine sur un STATUT alors que sa question porte sur la VIVACITÉ"
  - "On élargit une garde avec une fenêtre temporelle faute de mesure directe"
  - "Deux fonctions nommées comme équivalentes répondent à la même question"
  - "Une garde s'abstient à l'intérieur du SQL, donc sans trace côté application"
  - "Un bouton d'environnement règle un seuil qui entre dans un strftime"
related: [mika#2181, mika#2156, mika#2169, mika#2045, mika#1163, mika#1652]
---

# Une fenêtre bornée sur un proxy est une dette datée

## Le défaut

Le faucheur `stuck_pending` protégeait une parente tant qu'un wrapper différé
existait en `status = 'pending'`. Mais la promotion écrit `completed`, et le tour
silencieux qui consomme le wrapper ne passe `delivered` qu'à son **retour**,
plusieurs minutes plus tard. Entre les deux, le wrapper promu était invisible.

Le tick est fatal parce que les trois étapes vivent dans le **même** passage de
60 s et dans cet ordre : promotion → dispatch → fauche. Le faucheur s'exécutait
quelques millisecondes après la promotion, concluait « rien ne représente cette
parente », re-armait ; au tick suivant la fente était libre, le wrapper neuf
était promu, redevenait invisible, et le faucheur re-armait encore.
`MAX_STUCK_REARMS = 2` → parente `failed` en deux ticks. Trace : promotion
15:31:03Z, expiration 15:33:03Z, réponse du tour 15:34:43Z.

Le prédicat répondait à « existe-t-il un wrapper **en attente de promotion** ».
La question était « existe-t-il un wrapper **vivant** ».

## La leçon, dans sa forme générale

C'est la deuxième instance de la même classe en 48 h, sur deux faucheurs voisins
du même fichier — voir
[[a-reaper-that-reads-a-proxy-instead-of-the-process-2026-09-04]], dont les
résidus nommaient déjà celui-ci. La classe : **un balayeur discrimine sur un
indice corrélé au travail plutôt que sur le travail**. Quand la mesure directe
n'existe pas pour la population visée, la tentation est une fenêtre temporelle
sur le proxy. Elle est légitime — mais elle est une **dette datée**, et elle ne
s'acquitte qu'à trois conditions.

### 1. Bornez la fenêtre, et sachez dire pourquoi

Une fenêtre non bornée transforme le cadavre en bouclier. Ici : sur le chemin
d'erreur du tour, le wrapper est re-armé mais `mark_task_delivered` n'est jamais
appelé — il reste `completed` non-`delivered` pour toujours. Sans borne, le
faucheur ne touchait plus jamais cette parente, et le défaut réparé se retournait
en fuite silencieuse dans l'autre sens.

Corollaire non évident : **la borne doit échouer fermé sur son propre bouton**.
SQLite rend `NULL` pour un modificateur `strftime` hors plage, et `x > NULL` vaut
`NULL`. Une variable d'environnement démesurée ne réglait donc pas la fenêtre :
elle rendait la branche insatisfiable et **restaurait silencieusement le défaut
d'origine**. Un seuil qui entre dans un `strftime` a besoin d'un plafond, pas
seulement d'un plancher. Le seuil frère n'en avait pas besoin — son `NULL`
alimente une comparaison où il ne fait que ne rien sélectionner. La direction
d'échec dépend de la clause, pas de la forme du parseur.

### 2. Comptez ce que la garde épargne

L'épargne vit dans un `NOT EXISTS` SQL : une parente épargnée ne devient jamais
candidate et ne traverse jamais l'application. Aucun log, aucun audit, aucun
compteur — et le seul cri du faucheur était conditionné à la même requête
élargie. Un désarmement systématique se serait lu comme des compteurs à zéro,
**indiscernable d'un régime sain**.

Élargir une garde la rend plus silencieuse exactement à l'endroit où on lui
apprend à s'abstenir. Le doc de la veille avait déjà tiré la règle —
« faucher, **et le compter** » — et livré `phantom_sweep_spared`. La même
règle vaut pour l'abstention : `stuck_pending_sheltered_by_promoted_wrapper`.
Le compteur doit exclure l'abstention **ordinaire** (ici, la mise en file
normale) et ne compter que l'abri nouvellement introduit — sinon il mesure le
régime permanent au lieu du changement.

### 3. Nommez la mesure directe qui retire la dette, et mesurez le résidu

La fenêtre ne couvre pas tout, et le dire est la moitié du travail. Mesuré ici :
139 des 799 wrappers (17 %) livrent au-delà de la fenêtre ; 8 parentes ont été
expirées 2820–4996 s après promotion, hors de portée de **toute** valeur de la
constante. Environ un dixième de la classe d'expiration fautive survit.

Le plan écartait cette traîne comme « des redémarrages de serveur, pas des tours
sains ». **L'argument coupe dans le mauvais sens** : un wrapper retardé par un
redémarrage est un wrapper *sain*, donc précisément celui qu'il ne faut pas
tuer. Agrandir la constante achèterait la couverture en aveuglant le faucheur
plus longtemps. Le résidu se retire par une mesure **directe** de vivacité —
récence des lignes d'activité, la forme que [[1652-team-runs-orphan-reaper-and-tool-error-as-ok-not-err]]
emploie déjà pour les team runs — pas par un proxy plus large.

#### La dette est acquittée pour la cause A (mika#2184, 2026-09-21)

Ce paragraphe prescrit de **nommer** la mesure qui retire la dette. Il doit donc
dire quand elle arrive — et, surtout, **ce qu'elle ne couvre pas**. Une dette
qu'on déclare soldée sans dire son reste est une dette qu'on a seulement cessé
de regarder.

mika#2184 livre la mesure directe : `find_deferred_wrapper_activity_age_secs`
joint `parente → wrappers différés → sessions (sessions.task_id) → llm_calls /
tool_calls` — **par égalité**, pas par le `LIKE` sur `session_id` auquel
mika#1652 doit se rabattre, la colonne `sessions.task_id` existant depuis la v19
et étant écrite par `dispatch_resume_agent` lui-même. Le seuil
(`MIKA_STUCK_PENDING_ACTIVITY_WINDOW_SECS`, 600 s) ne rentre **pas** dans la SQL :
la requête rend un **âge**, une fonction pure le classe, et l'épargne se
journalise — parce que le défaut nommé au § 2 ci-dessus était précisément une
épargne muette dans un `NOT EXISTS`, et le reproduire une deuxième fois dans la
même fonction aurait re-creusé le trou qu'on venait de combler.

**Mais la lecture qui a produit ce paragraphe était incomplète, et la
rectification est ce qui compte ici.** Un wrapper `completed` non `delivered`
au-delà de la fenêtre a **trois** causes, et elles ne se mesurent pas de la même
façon :

| # | cause | activité sur la session du wrapper ? | statut |
|---|---|---|---|
| A | le tour tourne et est lent | **oui** | **acquittée** par mika#2184 |
| B | `AgentBusy` : le lock est pris ailleurs | **non** — `dispatch_resume_agent` rend `Err` *avant* d'ouvrir la session | **non couverte**, suivi nommé |
| C | redémarrage du service pendant l'attente | **non** — rien ne tournait | traitée par l'**aveu**, pas par la mesure |

Le § ci-dessus affirmait que la traîne se retirait par la mesure directe. C'est
vrai de A seulement. **Si la traîne est faite de B et de C, une mesure d'activité
ne la couvre pas — parce qu'il n'y a rien à mesurer.** Pour C, le remède n'est pas
un capteur mais une tri-valeur : un process dont l'uptime est inférieur à la
fenêtre n'a **pas pu** observer, donc « zéro ligne » n'y est pas un silence
(`NotYetObservable`, forme [[2277]]). Pour B, mika#2184 **refuse** de couvrir avec
sa raison écrite : le seul signal disponible serait « l'agent a de l'activité
ailleurs », vrai quasi en permanence sur mika-dev, et la garde cesserait de
refuser quoi que ce soit tout en produisant des lignes d'épargne rassurantes.

**Corollaire qui généralise, et c'est la part transportable :** une mesure
directe n'est directe que sur la population où le phénomène *produit un signal*.
Avant de déclarer qu'elle retire une dette, caractériser la population — sinon on
remplace un proxy par un capteur qui ne regarde pas où le défaut se produit, et
le vert obtenu est le « rouge vacuux » que le § suivant condamne.

## Deux prédicats nommés comme équivalents doivent l'être structurellement

`has_pending_deferred_wrapper_child` posait la même question que la clause du
faucheur et y répondait autrement. Zéro appelant de production **n'est pas une
propriété stable** : c'est un piège armé pour le prochain appelant. Renommer
(`has_live_deferred_wrapper_child`) fait attraper les sites par le compilateur —
`pending` décrivait le prédicat, pas la question.

Mais le renommage ne suffit pas : deux copies SQL synchronisées à la main
divergent. La règle du dépôt le dit déjà — si la fourche est inévitable, le test
de parité part dans le **même** commit. Ici, un test qui fait passer neuf états
dans les deux prédicats et affirme qu'ils s'accordent. Une mutation qui rétrécit
un seul des deux le fait rougir *seul*, parmi 4310.

## Le test qui n'a jamais été rouge ne prouve rien — et la vérifier coûte trois minutes

Deux pièges rencontrés, tous deux résolus par la mutation plutôt que par la
lecture :

- **Un rouge peut être vacuux.** La séquence prescrivait « paramètre accepté ET
  lié, clause SQL inchangée ». Irréalisable : rusqlite refuse un paramètre lié
  mais non référencé, et ce premier rouge échouait 14 tests sur de la plomberie
  — il ne démontrait rien. L'intention (la signature d'abord, pour que le
  compilateur attrape les appelants) se porte en différant la **liaison** d'un
  commit. Le rouge devient alors exact : 13 verts, 1 rouge, le rejeu de la trace.
- **Un angle mort peut être vert.** Les 15 sites d'appel passaient les deux
  fenêtres **égales** (`2700, 2700`). Une transposition des deux paramètres
  traversait la suite entière sans un rouge. Un seul test avec des fenêtres
  **inégales** ferme l'angle : la mutation « fenêtres transposées » le fait
  rougir seul pendant que 16 autres passent.

Règle : quand deux paramètres adjacents ont le même type et le même défaut,
**au moins un test doit les distinguer**. Et un garde neuf se vérifie en cassant
le code qu'il garde, pas en relisant son assertion.

## Application

- Élargir une garde ? Bornez, comptez l'épargne, nommez la mesure directe.
- Seuil dans un `strftime` ? Plafond obligatoire — l'extrême doit échouer fermé.
- Deux prédicats jumeaux ? Test de parité dans le même commit.
- Paramètres adjacents de même type ? Un test aux valeurs inégales.
- Garde ajoutée ? Mutez le code gardé et exigez le rouge.
