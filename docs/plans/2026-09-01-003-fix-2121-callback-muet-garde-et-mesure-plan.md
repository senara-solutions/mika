---
issue: mika#2121
title: Un callback muet ne dit rien — garde structurelle, puis mesure - Plan
type: fix
scope_repo: mika
priority: p1-important
date: 2026-09-01
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Un callback muet ne dit rien — garde structurelle, puis mesure - Plan

## Goal Capsule

**Objectif.** Trente-huit dispatches sur un même ticket ont produit zéro PR sans que
rien ne le signale. Le motif `callback_delivered_without_pr_url` est **exact sur le
symptôme et muet sur la cause**, et aucune garde ne compte les échecs sur le chemin
qui dispatche. Deux choses doivent cesser : qu'un échec soit silencieux, et qu'une
série d'échecs continue indéfiniment.

**Moyen.** Partie A — le callback dit toujours quelque chose (`NO_PR: <raison>`), le
reaper distingue les raisons, et un compteur d'échecs **consécutifs par ticket sur le
chemin de dispatch** retire le label, alarme et fiche au troisième. Partie B — la
mesure qui dira *pourquoi*, avant tout correctif de la cause racine.

**Hiérarchie d'autorité.** AC du ticket > ce plan > jugement de l'implémenteur.
**Une exception explicite, argumentée en KTD4 : l'AC-G4.** Son intention est portée,
son mécanisme est rectifié. La rectification est signalée, pas appliquée en silence.

**Conditions d'arrêt.**
- S'arrêter si un correctif de la **cause racine** est écrit avant que la Partie B
  n'ait rendu ses chiffres (AC5). La garde n'est pas un correctif de cause : elle
  protège contre le silence, et elle est juste quelle que soit la cause.
- S'arrêter si la garde compte un **total cumulé** au lieu d'une **série
  consécutive**. Une garde qui crie faux se fait désarmer (AC-G6).
- S'arrêter si le retrait du label devient inconditionnel ou non idempotent. Le
  geste est destructif ; il se déclenche sur un seuil nommé, une fois.
- S'arrêter si une AC compte des appels d'outil avec le marqueur `[tool]`. Il rend
  zéro sur des sessions qui en font trente-trois — le corps du ticket consigne ce
  piège ; le marqueur correct est `user message (tool result) received`, publié
  avec son contrôle.

**Profil d'exécution.** Deux surfaces : `dispatch-lib.sh` (bash) et `mika-agent`
(Rust, `task_engine` + `server` + `db`). Partie A séquentielle ; Partie B
indépendante de A et parallélisable.

**Propriété de la queue.** PR sur `mika`, routée vers mika-qa. La frontière A/B est
une frontière de PR naturelle si l'opérateur veut livrer la garde plus tôt.

## Product Contract

### Résumé

Un chemin d'échec muet est indiscernable d'un chemin qui n'a pas été pris. Ce plan
rend l'échec bavard (Partie A), puis le mesure (Partie B). La garde part en premier
et ne dépend d'aucun résultat de mesure.

### Cadrage du problème

306 tâches parentes `self_dev` marquées `failed` avec `callback_delivered_without_pr_url`
entre le 2026-07-28 et le 2026-08-31 — première cause d'échec de la boucle.

**Le motif est exact, et c'est ça le problème.** Sur #1651, les seules vraies PR sont
PR#1845 et PR#1879, toutes deux closes le 2026-07-29 au plus tard ; ses 38 échecs
courent du 2026-08-04 au 2026-08-31, **entièrement après**. Le reaper dit la vérité :
rien n'a été produit. On ne cherche pas un parseur à réparer, on cherche pourquoi le
pilote ne produit rien.

**Pourquoi 38 fois sans détection.** Les trois sites de `dispatch-lib.sh` qui émettent
la ligne `PR:` n'ont **aucun `else`**. L'absence de PR est communiquée par l'absence
d'une ligne, et quatre états distincts produisent la même sortie : rien. Le reaper lit
cette absence et écrit le motif générique. Rien à alarmer, rien à compter, rien à
distinguer d'une panne de `gh`.

**Et la garde qui existe est sur le mauvais chemin.** `ready_label_handler.rs` ne
contient aucune occurrence de `failure_count`, `circuit` ni `CIRCUIT_BREAKER`
(contrôle positif : `ready` y apparaît **145** fois — vérifié, le compte du ticket est
exact). Une fois le label posé, le chemin webhook dispatche sans compteur.

### Décisions clés

- **La garde d'abord, la cause ensuite.** Elle est correcte quelle que soit la réponse
  de la Partie B. Régit la séquence.
- **Consécutif, jamais cumulé.** Régit KTD3, AC-G5, AC-G6.
- **Nommer l'état, pas seulement l'absence.** Régit KTD1, KTD2.
- **Le compteur du dispatch est neuf ; celui d'`auto_pull` n'est pas réutilisé.**
  Régit KTD4 — c'est la rectification de l'AC-G4.

### Exigences

- **R1** — Les trois sites d'émission produisent toujours une ligne : `PR:` ou
  `NO_PR: <raison>`. L'erreur de `gh` cesse d'être avalée. (AC-G1)
- **R2** — Le reaper lit `NO_PR:` et écrit `callback_no_pr_<raison>`. Le motif
  générique reste **exactement** pour les callbacks portant ni `PR:` ni `NO_PR:` —
  c'est-à-dire un producteur non mis à jour, qui doit rester visible. (AC-G2)
- **R3** — Un compteur d'échecs **consécutifs par numéro d'issue sur le chemin de
  dispatch**. Au troisième : alarme cm vers `samidarko`, retrait du label `ready`,
  ticket ouvert et lié. (AC-G3)
- **R4** — Un ticket qui ne produit rien cesse d'occuper le bassin `pullable`.
  (**Intention héritée de l'AC-G4, retirée le 2026-09-02** — voir la section dédiée.
  L'exigence survit à l'AC : c'est elle qui portait le sens.)
- **R5** — Une PR ouverte remet la série à zéro. (AC-G5)
- **R6** — Contrôle négatif : échec, échec, succès, échec → compteur à **1**, aucune
  alarme. (AC-G6)
- **R7** — Preuve de non-vacuité : le test du déclenchement au 3ᵉ échoue si le seuil
  est retiré. Démontré et consigné dans la PR. (AC-G7)
- **R8** — Partition vrai-échec / faux-échec, chiffrée et reproductible. (AC1)
- **R9** — Répartition des vrais échecs par cause terminale, lue dans **les deux**
  journaux du pilote — ils sont disjoints, chercher dans un seul ment. (AC2)
- **R10** — Taux **par dispatch**, avant/après cpp#129/#131, jamais en compte brut. (AC3)
- **R11** — Si la population faux-échec est non vide, le site producteur qui omet
  `^PR: ` est nommé au `file:line` et couvert par un test. (AC4)
- **R12** — Aucun correctif de la **cause racine** avant que R8 et R9 n'aient rendu
  leurs chiffres. Ne s'applique pas à la Partie A. (AC5)

### Sources

Toutes relues sur `origin/main` @ `50d969a7` le 2026-09-01. Les numéros de ligne du
corps du ticket ont dérivé sur trois références ; les valeurs ci-dessous sont les
valeurs courantes.

- `skills/bundled/_shared/dispatch-lib.sh:822-829` — site 1, chemin crash.
- `skills/bundled/_shared/dispatch-lib.sh:2689-2696` — site 2, chemin principal.
- `skills/bundled/_shared/dispatch-lib.sh:5007-5056` — site 3, chemin sauvetage.
- `crates/mika-agent/src/task_engine/engine.rs:1570-1590` — le reaper, écrit le motif.
- `crates/mika-agent/src/task_engine/dispatcher.rs:3792` — **second** site d'écriture
  du motif (le corps le nomme ; voir KTD5 sur le commentaire « SOLE WRITER »).
- `crates/mika-agent/src/task_engine/dispatcher.rs:2273` — le parseur ancré,
  `Regex::new(r"(?m)^PR:\s+(https?://github\.com/\S+)")`. *(Le corps disait `:1780` —
  dérive.)*
- `crates/mika-agent/src/auto_pull.rs:58` — `CIRCUIT_BREAKER_THRESHOLD = 3`.
  *(Le corps disait `:19` — dérive.)*
- `crates/mika-agent/src/auto_pull.rs:2024`, `:2155` — les deux déclenchements ;
  chacun ne fait que `continue` / `return None`.
- `crates/mika-agent/src/auto_pull.rs:1687`, `:2075`, `:2224`, `:2502`, `:2509` — les
  **cinq** sites d'incrément de `failure_count`, tous dans une branche
  `if let Err(e) = gh_apply_label(...)`.
- `crates/mika-agent/src/db.rs:4741` — la doc qui énonce la sémantique :
  *« `failure_count` means "the `gh` call failed" »*.
- `crates/mika-agent/src/server/ready_label_handler.rs:85` —
  `try_handle_ready_label_dispatch`, le point d'entrée du chemin de dispatch.
- `crates/mika-agent/src/milestone_manager/spawn.rs:1029` — `emit_auth_alarm`, le
  précédent d'alarme à seuil + refroidissement à imiter.

## Planning Contract

### Décisions techniques clés

- **KTD1 — Une ligne négative explicite, pas un silence.** Chaque site gagne un `else`
  émettant `NO_PR: <raison>`. La sortie d'erreur de `gh` cesse d'être avalée : elle
  est capturée et classée. Le contrat de sortie devient **total** — un callback porte
  toujours exactement une des deux lignes.

- **KTD2 — Le jeu de raisons diffère selon le site, parce que les sites diffèrent.**
  *(Rectification : le corps présente les trois sites comme ayant « la même forme »
  avec `gh pr list`. Vérifié : le site 3 est un `gh pr create`, pas un `gh pr list`.)*
  - Sites 1 et 2 (`gh pr list`) : `no_pr_on_branch`, `gh_query_failed`,
    `branch_unset`, `repo_unset` — le jeu de l'AC-G1, qui leur va exactement.
  - Site 3 (`gh pr create`, chemin de sauvetage) : son silence signifie *« la création
    de la PR de sauvetage a échoué »*, pas *« aucune PR sur la branche »*. Il reçoit
    une cinquième raison, `rescue_pr_create_failed`. Le jeu de l'AC-G1 est **étendu**,
    jamais réduit : l'intention (nommer l'état) est tenue sur les trois sites.

- **KTD3 — Un compteur neuf, table neuve, sémantique consécutive.** `dispatch_no_pr_streak`
  (`repo_full_name`, `issue_number`, `streak_count`, `last_no_pr_at`, `last_reason`,
  `last_alarm_at`), migration additive. Deux gestes : `bump` (incrément) sur un
  callback sans `PR:`, `clear` (mise à zéro) dès qu'un `PR:` est vu. « Consécutif »
  est porté par la **structure** — la remise à zéro est inconditionnelle sur succès —
  et non par une convention que le prochain lecteur devra deviner.

- **KTD4 — L'AC-G4 : intention portée, mécanisme rectifié. À trancher par
  l'architecte.** L'AC-G4 demande de retirer `ready` « quand `failure_count` atteint
  `CIRCUIT_BREAKER_THRESHOLD` ». **Mesuré : `failure_count` ne compte pas les
  dispatches sans PR.** Ses cinq sites d'incrément sont tous dans une branche
  `if let Err(e) = gh_apply_label(...)`, et `db.rs:4741` le dit en toutes lettres :
  *« `failure_count` means "the `gh` call failed" »*.

  Appliquer l'AC-G4 à la lettre retirerait donc le label `ready` **parce que l'API
  GitHub a été instable trois fois** — un geste destructif déclenché par du bruit de
  transport, exactement la « garde qui crie faux » que l'AC-G6 interdit. Le site
  `auto_pull.rs:2073` rend l'absurdité visible : l'incrément a lieu quand *poser*
  `ready` a échoué ; l'AC-G4 *retirerait* alors `ready` d'un ticket où il n'a jamais
  été posé.

  **L'intention de l'AC-G4 est juste et elle est portée :** un ticket disjoncté ne
  doit plus occuper une place du bassin `pullable`. Ce plan la tient par le **nouveau**
  compteur de KTD3 — dont la sémantique est « ce ticket ne produit rien », celle que
  l'AC-G4 croyait invoquer — et **n'ajoute aucun retrait de label au disjoncteur
  d'`auto_pull`**. Le blocage mesuré (#1651 et #1403 occupant `pullable: 2`) est levé
  par le même geste, avec le bon déclencheur.

  **Ce que ce plan ne fait pas :** il ne modifie pas le disjoncteur d'`auto_pull` ni sa
  sémantique. **Rectification signalée, jamais appliquée en silence.**

  **Mesure du 2026-09-02 — la motivation de l'AC-G4 n'existe plus.** L'AC-G4 justifie sa
  demande par un état daté : *« un ticket disjoncté continue d'occuper le bassin
  (`pullable`) — c'est l'état mesuré de #1651 et #1403 aujourd'hui »*. Cet état a été
  levé entre-temps, par une autre porte :

  - Le geste demandé **existe déjà** : `auto_pull.rs:1697`,
    `gh_remove_label(github_token, issue_number, "ready")`, dans `abandon_stuck_ready`.
    Il n'était pas absent — il était **inatteignable**, parce que la ligne au-dessus
    (`gh_apply_label(..., "operator-review")`, `:1684`) échouait toujours et sortait en
    `return`. C'est le défaut que porte **mika#2127**.
  - Les labels `operator-review` et `blocked` ont été déclarés puis synchronisés le
    2026-09-01 (`46eeef98` / mika#2128 à 12:42, `04945721` / mika#2130 à 13:22).
    Contrôle live : les deux existent sur le dépôt (50 labels au total).
  - **Effet observé le même jour à 17:12Z : #1651 et #1403 ont perdu `ready` et portent
    `blocked`.** Ils ne comptent plus dans le bassin `pullable`.
  - **Et c'est la troisième confirmation, indépendante, que `failure_count` ne compte pas
    des dispatches :** il est figé à **3** avec `last_failure_at` au **2026-08-31** pour
    les deux tickets. Il a cessé d'incrémenter exactement quand le label a commencé à
    exister — parce qu'il ne comptait que des `gh_apply_label` en échec.

  **Ce que cette mesure établit, et ce qu'elle n'établit pas.** Elle n'établit **pas**
  que l'AC-G4 est satisfaite : sa lettre demande le retrait au **déclenchement du
  disjoncteur** (`auto_pull.rs:2024`, `:2155`), qui ne fait toujours que passer son tour.
  Elle établit que **le mal que l'AC-G4 nomme est guéri** — par le chemin d'abandon, pas
  par le disjoncteur — et que le compteur qu'elle invoque mesure autre chose. L'AC-G4 est
  donc une AC dont la prémisse est morte et dont le mécanisme est faux.

  **Tranché le 2026-09-02 par l'opérateur : option (c), l'AC-G4 est retirée.** mika-arch
  avait rendu `ESCALATE` en première passe (F1 BLOCKING, session `d88d1008`) au motif
  que *« l'architecte ne peut pas ratifier une divergence de spec unilatéralement »* —
  la correction revenait donc à l'opérateur, et elle a eu lieu, sur le corps du ticket
  comme ici. Le retrait est documenté à la section suivante plutôt qu'appliqué en
  silence : **une AC retirée sans raison écrite est une intention perdue.**

- **KTD5 — Le commentaire « SOLE WRITER » est faux et il est réparé au passage.**
  `engine.rs:1570` porte `// SOLE WRITER: callback_delivered_without_pr_url` alors que
  `dispatcher.rs:3792` écrit le même motif — le corps du ticket nomme d'ailleurs les
  deux. Un commentaire qui ment sur l'unicité d'un écrivain est un piège pour le
  prochain lecteur de ce chemin, et l'U2 touche exactement ces lignes. Correction
  d'une ligne, dans le périmètre du geste.

- **KTD6 — L'alarme imite un précédent, elle n'en invente pas un.**
  `milestone_manager/spawn.rs:1029` (`emit_auth_alarm`) porte déjà seuil,
  refroidissement, et des tests nommés (« ne ré-émet pas dans le refroidissement »,
  « ne se déclenche jamais pour les classes non concernées »). L'alarme de l'AC-G3
  suit cette forme plutôt qu'une nouvelle.

- **KTD7 — Le ticket auto-ouvert est idempotent.** Avant de créer, chercher un ticket
  ouvert déjà lié au même numéro et portant le marqueur d'auto-ouverture ; si présent,
  commenter plutôt que dupliquer. Une garde qui fiche en double se fait désarmer aussi
  vite qu'une garde qui crie faux. Aucun helper de création d'issue n'existe côté
  agent (vérifié) : c'est une surface neuve, et son idempotence est un livrable, pas
  une intention.

- **KTD8 — Ordre des trois effets au déclenchement : alarmer, retirer, ficher.**
  L'alarme part d'abord — c'est le seul effet qui ne peut pas échouer à moitié. Le
  retrait de label et l'ouverture de ticket sont *fail-open* et journalisés : leur
  échec ne doit jamais empêcher les deux autres, ni faire échouer le dispatch en cours.

### Contraintes vérifiées (mesurées le 2026-09-01 sur `50d969a7`, non supposées)

- Les trois sites de `dispatch-lib.sh` sont **confirmés sans `else`**. Sites 1 et 2 en
  `gh pr list`, site 3 en `gh pr create` (cf. KTD2).
- `ready_label_handler.rs` : `failure_count` **0**, `circuit` **0**,
  `CIRCUIT_BREAKER` **0**, `ready` **145**. Le contrôle du corps est exact au compte près.
- Les deux déclenchements du disjoncteur (`auto_pull.rs:2024`, `:2155`) ne font que
  passer leur tour (`continue` / `return None`) : ni retrait de label, ni alarme. La
  prémisse de l'AC-G4 sur ce point est exacte.
- `reset_auto_pull_failure` **existe** (`db.rs:10002`) et est appelé en trois endroits.
  Le compteur d'`auto_pull` n'est donc pas cumulatif-à-vie — mais il compte des échecs
  de `gh`, pas des dispatches (KTD4).
- Aucun helper de création d'issue côté `mika-agent` (contrôle négatif de sonde :
  motif inexistant → 0 occurrence). L'AC-G3 (c) est une surface neuve.
- **Mesuré le 2026-09-02 (voir KTD4) :** `operator-review` et `blocked` existent
  désormais sur le dépôt (contrôle live, 50 labels) ; `auto_pull.rs:1697` porte déjà le
  `gh_remove_label(..., "ready")` que l'AC-G4 réclame, en aval du `return` de `:1684` ;
  #1651 et #1403 ont perdu `ready` le 2026-09-01 à 17:12Z et portent `blocked` ; leur
  `failure_count` est figé à 3 avec `last_failure_at` au 2026-08-31.
- **Dérives de numéros de ligne** relevées, sans effet sur le fond : parseur `^PR: `
  en `:2273` (corps : `:1780`) ; `CIRCUIT_BREAKER_THRESHOLD` en `:58` (corps : `:19`) ;
  sites `dispatch-lib.sh` décalés de 1 à 20 lignes.

### Séquencement

**Partie A** — U1 → U2 → U3 → U4 → U5. U2 dépend du format posé par U1 ; U4
(non-vacuité) dépend d'U3.
**Partie B** — U6 → U7, U8 indépendante, U9 conditionnée au résultat d'U6.
A et B ne se bloquent pas mutuellement. Frontière de PR naturelle entre les deux.

## AC-G4 — retirée le 2026-09-02, et pourquoi

Retrait décidé par l'opérateur (option (c)), consigné ici pour que l'intention ne se
perde pas avec l'AC.

**Ce qu'elle demandait.** *« Le retrait du label est appliqué **aussi** au disjoncteur
d'`auto_pull` existant : quand `failure_count` atteint `CIRCUIT_BREAKER_THRESHOLD`,
`ready` est retiré. »*

**Sa prémisse d'origine.** *« Sans quoi un ticket disjoncté continue d'occuper le bassin
(`pullable`) sans pouvoir être tiré — c'est l'état mesuré de #1651 et #1403
aujourd'hui. »*

**La mesure qui l'a tuée (2026-09-02).** Cet état n'existe plus, et le compteur qu'elle
invoque ne mesure pas ce qu'elle croyait :

- `operator-review` et `blocked` ont été déclarés puis synchronisés le 2026-09-01
  (`46eeef98`/mika#2128 à 12:42, `04945721`/mika#2130 à 13:22). Contrôle live : les deux
  existent sur le dépôt, parmi 50 labels.
- **Le même jour à 17:12Z, #1651 et #1403 ont perdu `ready`** et portent `blocked`. Les
  deux tickets qui *étaient* la justification de l'AC ne bloquent plus le bassin.
- `failure_count` reste figé à **3** sur les deux, `last_failure_at` au **2026-08-31** :
  il a cessé d'incrémenter exactement quand le label a commencé à exister — parce qu'il
  ne comptait que des `gh_apply_label` en échec, jamais des dispatches sans PR. C'est la
  troisième confirmation du fait, et la première qui vienne du monde et non du code (les
  deux autres : les cinq sites d'incrément, et `db.rs:4741`).

**Les deux endroits où son intention est déjà couverte.**

1. **Côté `auto_pull`** — `auto_pull.rs:1697`, `gh_remove_label(..., "ready")` dans
   `abandon_stuck_ready`. Le geste **existait déjà** ; il était seulement inatteignable,
   en aval du `return` de `:1684`. La synchro des labels l'a rendu atteignable, et
   l'effet est mesuré ci-dessus. Sa fragilité résiduelle (si l'application échoue pour
   une autre raison — quota, permission — `ready` reste posé) appartient à **mika#2127**,
   qui reste ouvert.
2. **Côté dispatch** — le compteur `dispatch_no_pr_streak` de l'**AC-G3**, dont la
   sémantique est « ce ticket ne produit rien », celle que l'AC-G4 croyait invoquer. Son
   franchissement retire `ready` (U3), et **U4** vérifie que le bassin se libère.

**Ce qui survit au retrait.** L'exigence **R4** et l'unité **U4**. L'AC disparaît, son
intention reste nommée et testée — c'est tout l'objet de cette section.

## Fire-Disposition

Requis par le Fire-Disposition Gate (mika#1574). Ce plan livre un **détecteur qui agit**
— au troisième échec consécutif il retire un label, alarme et ouvre un ticket. Trois
effets, dont un destructif. Disposition contre le schéma canonique **(a) exception
nommée / (b) posé-désactivé / (c) halte-et-remontée**.

**Le détecteur de l'AC-G3 → (c) halte-et-remontée.** C'est l'événement même que la
garde existe pour attraper : trois dispatches consécutifs sans PR sur le même ticket.
Halter ce ticket-là (retrait de `ready`) et le remonter (alarme + ticket) est le
résultat voulu, pas un faux positif. **Aucune liste blanche par ticket** — une
exception nommée rouvrirait le trou pour le ticket exact qui l'a révélé.

**Le tir sur données préexistantes est structurellement impossible, pas seulement
improbable.** La table `dispatch_no_pr_streak` est **neuve et vide** à la migration.
Aucun historique n'est rétro-importé — ni les 306 échecs, ni les 38 de #1651. Le
compteur ne peut donc atteindre 3 qu'après **trois dispatches réels observés
post-déploiement**. La classe de faux positif que le gate redoute le plus — un
détecteur qui se déclenche sur des données antérieures à son existence — est fermée
par construction. C'est aussi ce qui rend le déploiement sûr sans phase désactivée.

**Pourquoi pas (b) posé-désactivé.** Un détecteur posé mais inerte laisserait le
silence en place, qui est le défaut. Et son coût d'observation est nul : la table
partant vide, un premier tir ne peut arriver qu'après trois dispatches vraiment
observés — le monde fournit lui-même la phase d'observation.

**Ce qui borne le geste destructif.** Le retrait de `ready` est *fail-open* et
journalisé (KTD8) ; il ne peut pas faire échouer un dispatch en cours. Il est
réversible d'un geste : l'opérateur repose le label, et une PR ouverte remet la série
à zéro (AC-G5). L'ouverture de ticket est idempotente (KTD7).

**Les tests → (c) halte-et-remontée.** Un test rouge bloque la PR. Aucune liste
blanche, aucun `#[ignore]`. Si l'AC-G6 rougit, c'est la garde qui est fausse, pas le
test qu'il faut détendre.

## Implementation Units — Partie A (la garde, non conditionnée)

### U1. Le callback dit toujours quelque chose

- **Fichier :** `skills/bundled/_shared/dispatch-lib.sh` (sites `:822`, `:2689`, `:5007`).
- **Approche.** Chaque `if [ -n "$..._PR_URL" ]` gagne un `else` émettant
  `NO_PR: <raison>`. La sortie d'erreur de `gh` est capturée (fin de
  `2>/dev/null || true` sur ces trois appels) et classée en raison. Sites 1–2 : jeu de
  l'AC-G1. Site 3 : `rescue_pr_create_failed` (KTD2). Les gardes amont
  (`[ -n "$REPO" ] && [ -n "$BRANCH" ]`) gagnent leur branche négative
  (`repo_unset` / `branch_unset`), aujourd'hui muettes elles aussi.
- **Vérification.** `bash skills/bundled/_shared/test-dispatch-lib.sh` ; `shellcheck`
  propre. Un test par raison, plus **un test de totalité** : sur les trois sites,
  toute sortie porte exactement une ligne `PR:` ou `NO_PR:`, jamais zéro, jamais deux.
- **Couvre :** R1 / AC-G1.

### U2. Le reaper distingue, et le commentaire cesse de mentir

- **Fichiers :** `crates/mika-agent/src/task_engine/engine.rs`,
  `crates/mika-agent/src/task_engine/dispatcher.rs`.
- **Approche.** Parseur ancré pour `NO_PR:` en miroir de celui de `:2273`. Quand la
  ligne est présente, écrire `callback_no_pr_<raison>`. **Le motif générique reste
  intact** pour un callback portant ni `PR:` ni `NO_PR:` — un producteur non mis à
  jour doit rester visible, c'est la lettre de l'AC-G2. Corriger au passage le
  commentaire `// SOLE WRITER` d'`engine.rs:1570` (KTD5), les deux écrivains étant
  `engine.rs:1577` et `dispatcher.rs:3792`.
- **Vérification.** `cargo test -p mika-agent task_engine`. Tests : une raison connue →
  motif spécifique ; **contrôle négatif** : un callback ancien (ni `PR:` ni `NO_PR:`)
  → toujours `callback_delivered_without_pr_url`, inchangé.
- **Couvre :** R2 / AC-G2.

### U3. Le compteur consécutif sur le chemin de dispatch, et ses trois effets

- **Fichiers :** `crates/mika-agent/src/db.rs` (migration + gestes),
  `crates/mika-agent/src/async_db.rs`, `crates/mika-agent/src/server/ready_label_handler.rs`.
- **Approche.** Migration additive : table `dispatch_no_pr_streak` (KTD3). `bump` sur
  callback sans `PR:`, `clear` sur `PR:` vu. Dans `try_handle_ready_label_dispatch`
  (`:85`), au franchissement du seuil 3 : alarme cm vers `samidarko` nommant ticket,
  compte et dernière raison (forme d'`emit_auth_alarm`, KTD6) → retrait de `ready` →
  ticket idempotent lié (KTD7). Ordre et *fail-open* per KTD8.
- **Vérification.** `cargo test -p mika-agent`. Tests :
  - Trois échecs consécutifs → les trois effets, une seule fois.
  - **AC-G6, contrôle négatif :** échec, échec, **succès**, échec → `streak_count == 1`,
    **zéro** alarme, label intact.
  - **AC-G5 :** un `PR:` vu remet à zéro, inconditionnellement.
  - **Idempotence (KTD7) :** deux franchissements ne créent qu'un ticket.
  - **Fail-open (KTD8) :** un retrait de label en échec ne fait pas échouer le dispatch
    et n'empêche pas l'alarme.
  - **Table vide au départ :** aucun tir possible sans trois dispatches observés —
    le contrôle qui rend vraie l'affirmation de la Fire-Disposition.
- **Couvre :** R3, R5, R6 / AC-G3, AC-G5, AC-G6.

### U4. Le bassin se libère — l'intention héritée de l'AC-G4, par le bon déclencheur

- **Fichiers :** aucun nouveau — c'est une **conséquence** d'U3, vérifiée explicitement.
- **Approche.** Le retrait de `ready` par U3 sort le ticket du bassin `pullable`. Le
  disjoncteur d'`auto_pull` **n'est pas modifié** (KTD4). Un test dédié établit que
  la conséquence a bien lieu.
- **Vérification.** Test : un ticket ayant franchi le seuil n'est plus compté
  `pullable` par l'alimenteur. Rejeu sur l'état mesuré de #1651 / #1403.
- **Couvre :** R4 (intention héritée de l'AC-G4 retirée — voir la section dédiée).

### U5. La preuve que la garde n'est pas vide

- **Fichiers :** aucun durablement — manipulation temporaire, restaurée.
- **Approche.** Retirer le seuil du code (le rendre inatteignable), lancer
  `cargo test -p mika-agent`, capturer la sortie **rouge**, restaurer, capturer le vert.
  Coller les deux dans le corps de la PR.
- **Vérification.** Le corps de PR porte les deux sorties.
- **Couvre :** R7 / AC-G7.

## Implementation Units — Partie B (la mesure, avant tout correctif de cause)

### U6. Partition vrai-échec / faux-échec

- **Approche.** Requête reproductible (script versionné, pas une requête jetable) :
  pour chaque tâche `callback_delivered_without_pr_url`, une PR a-t-elle été ouverte
  pour son issue dans une fenêtre de ±2 h autour d'`updated_at` ? Sortie : deux
  populations chiffrées. Le script est un livrable — un chiffre non rejouable n'est
  pas une mesure.
- **Couvre :** R8 / AC1.

### U7. Répartition des vrais échecs par cause terminale

- **Approche.** Sur la population vrai-échec, classer par cause terminale de session
  (refus, `maxTurns`, `idle_timeout`, throttling, transport), **en lisant les deux
  journaux disjoints du pilote** — `/var/log/claude-pilot/<id>.stderr` et le journal
  d'egress. Chercher dans un seul ment. Les huit sessions de la nuit du 08-31→09-01,
  journaux intacts, servent d'échantillon de départ.
- **Contrainte de marqueur (non négociable).** Tout compte d'appels d'outil utilise
  `user message (tool result) received`, **jamais** `[tool]`, et **publie son contrôle**
  (positif + négatif) à côté du chiffre. Le corps du ticket consigne qu'une conclusion
  a déjà été tirée du mauvais marqueur puis retirée.
- **Couvre :** R9 / AC2.

### U8. Taux par dispatch, avant et après cpp#129/#131

- **Approche.** Taux de `callback_delivered_without_pr_url` **par dispatch**, sur des
  fenêtres comparables, contre la ligne de base de la nuit du 08-31→09-01 inscrite au
  ticket. **Contrôle négatif obligatoire :** un dénominateur de dispatches, jamais un
  compte brut — une baisse parce que la boucle a moins tourné n'est pas une amélioration.
- **Couvre :** R10 / AC3.

### U9. Le producteur qui omet `^PR: `, si la population faux-échec est non vide

- **Approche.** **Conditionnée à U6.** Si la population faux-échec est vide, l'unité
  est close en le constatant, avec le chiffre — pas silencieusement sautée. Sinon :
  nommer le site producteur au `file:line` et le couvrir d'un test.
- **Couvre :** R11 / AC4.

## Verification Contract

- `cargo test -p mika-agent` — vert, contrôles positifs **et** négatifs d'U2 et U3.
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `bash skills/bundled/_shared/test-dispatch-lib.sh` — vert, dont le test de totalité d'U1.
- `shellcheck` propre sur `dispatch-lib.sh`.
- **Preuve de non-vacuité (U5)** : sorties rouge et verte dans le corps de la PR.
- **Post-déploiement, opérateur :** le taux `callback_delivered_without_pr_url` **par
  dispatch** sur une fenêtre d'au moins une vague complète, et le compte de
  `callback_no_pr_<raison>` par raison. Si la seconde répartition reste vide alors que
  des dispatches échouent, c'est le producteur qui n'a pas été redéployé — pas la
  garde qui ne marche pas. **Mesurer au moins une période du composant le plus lent
  avant de conclure.**

## Acceptance criteria

Transcrits du ticket, avec l'unité qui satisfait chacun.

**Partie A**
- [x] **AC-G1** — Ligne négative explicite sur les trois sites, erreur de `gh` capturée. → **U1**, avec le jeu de raisons **étendu** d'une cinquième valeur pour le site 3 (KTD2 ; le site est un `gh pr create`, pas un `gh pr list`).
- [x] **AC-G2** — Le reaper distingue ; le motif générique reste pour un producteur non mis à jour. → **U2**.
- [x] **AC-G3** — Compteur consécutif par ticket sur le chemin de dispatch ; alarme + retrait + ticket au 3ᵉ. → **U3**.
- **AC-G4** — **RETIRÉE le 2026-09-02 (décision opérateur, option (c)).** Prémisse morte, mécanisme faux, intention déjà couverte deux fois. Raison écrite en section « AC-G4 — retirée le 2026-09-02, et pourquoi ». Ce qui en survit : **R4** et **U4**.
- [x] **AC-G5** — Un succès remet la série à zéro. → **U3**.
- [x] **AC-G6** — Contrôle négatif : échec, échec, succès, échec → compteur 1, zéro alarme. → **U3**.
- [x] **AC-G7** — Non-vacuité du seuil, démontrée et consignée dans la PR. → **U5**.

**Partie B**
- [x] **AC1** — Partition vrai/faux échec, reproductible. → **U6**.
- [x] **AC2** — Répartition par cause terminale, **deux journaux**, marqueur correct + contrôle publié. → **U7**.
- [x] **AC3** — Taux **par dispatch**, avant/après, contrôle négatif du dénominateur. → **U8**.
- [x] **AC4** — Site producteur nommé si la population faux-échec est non vide ; close avec son chiffre sinon. → **U9**.
- [x] **AC5** — Aucun correctif de cause racine avant U6 et U7. La Partie A n'y est pas soumise. → Séquencement + Conditions d'arrêt.

## Definition of Done

**Global.**
- R1–R12 satisfaits, chacun tracé à une unité posée.
- Aucun callback des trois sites ne peut sortir muet — vérifié par le test de totalité, pas par relecture.
- Le motif générique subsiste **exactement** pour les producteurs non mis à jour.
- Le compteur est consécutif par structure : la remise à zéro sur succès est inconditionnelle.
- Le disjoncteur d'`auto_pull` est **inchangé**, et la raison est écrite (KTD4).
- Le retrait de l'AC-G4 porte sa raison dans le plan, et le corps du ticket le reflète —
  aucun des deux ne diverge de l'autre.
- Le commentaire `// SOLE WRITER` ne ment plus.
- Les trois effets du déclenchement sont *fail-open* : aucun ne peut faire échouer un dispatch.
- L'ouverture de ticket est idempotente, démontrée par test.
- Aucun correctif de cause racine dans le diff — vérifiable par l'absence de changement de comportement du pilote.
- La preuve de non-vacuité d'U5 est dans le corps de la PR.
- Tout chiffre de la Partie B est accompagné de son contrôle, et de son dénominateur quand c'est un taux.

**Par unité.** La Vérification de chaque unité passe.
