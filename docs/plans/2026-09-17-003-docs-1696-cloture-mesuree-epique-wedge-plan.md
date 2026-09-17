# mika#1696 — clore l'épique du wedge du 2026-06-30 sur une mesure, pas sur un souvenir

**Ticket :** `senara-solutions/mika#1696` — *Incident — autonomous loop wedge, 2026-06-30*
**Type :** documentation d'incident + routage des restes (aucun code moteur)
**Date du grooming :** 2026-09-17 — **2 mois et 17 jours après l'incident**

---

## 1. Ce que ce ticket est devenu, et pourquoi ça décide du plan

mika#1696 n'est pas un ticket d'implémentation. C'est un **parent de coordination**
ouvert le 2026-06-30 pour qu'un après-midi de pannes se lise « 1 incident,
5 sous-correctifs » plutôt que « 5 nouvelles dettes ». Son corps le dit
explicitement, et il nomme lui-même son propre livrable :

> *After closure: orchestrator-CC documents the incident as compound learning +
> closes the epic.*

Le travail de code, lui, vit dans les sous-tickets — chacun ouvert, numéroté et
dispatché séparément. **Écrire ce code sous #1696 court-circuiterait ces
sous-tickets** et rendrait la condition de clôture 2 du ticket
(« mika#1694 *has shipped* ») incohérente avec son propre véhicule : un épique ne
peut pas être le véhicule de livraison de l'enfant dont il attend la livraison.

D'où la forme de ce plan : **un document de solution + un routage explicite des
restes**, zéro ligne de moteur. Ce n'est pas un descopage — c'est le périmètre que
le ticket se donne. Le dépôt a le précédent : `#2343` (docs(2331)), `#2341`
(docs(2337)), `#2187` (docs(solutions)) sont des PR entièrement documentaires
fusionnées dans les trois dernières semaines.

## 2. La mesure : les 5 conditions de clôture, relues au 2026-09-17

Le corps de #1696 énumère cinq conditions. Voici leur état, **mesuré sur le dépôt**,
pas rappelé de mémoire. C'est l'apport réel de ce grooming : quatre des cinq sont
soit satisfaites, soit caduques, soit non actionnables ici, et la cinquième a
changé de forme.

### Condition 1 — « Deploy bundle {1685+1679+1383} is live » → **satisfaite, et le filet a récidivé**

| sous-fix | état | preuve |
|---|---|---|
| mika#1685 (`--no-verify` rescue) | **fusionné** | `d8f780ab … (#1690)` |
| mika#1679 (Edit 4 absorbé, gate #1383/#1396) | **fusionné** | `2aa9bc5c … (#1698)` |
| mika#1684 (Step 1.5 rescue-header) | absorbé dans 1679 | — |
| filet secret-scan que `--no-verify` ne contourne pas (mika#1689) | **fusionné** | `5db56354 … (#2017)` |

La « deploy unit » est en service et son filet **fonctionne** : les rescues sont
attrapées, promues et fusionnées sur `main`.

**Mais la classe de défaut que ce filet attrape est revenue en rafale, pendant ce
grooming.** Distribution complète des commits `wip(…): trailing content after
pilot end_turn (mika#1383)` sur toute l'histoire du dépôt :

```
1  ·  2026-06   ← l'incident fondateur de #1696
0  ·  2026-07
0  ·  2026-08
4  ·  2026-09   ← 15, 16, 16 et 17 septembre
```

Six semaines de silence, puis quatre déclenchements en trois jours — dont un
(`3048aeb8`, mika#2354) est l'avant-dernier commit de `main` au moment où ce plan
est écrit. **Ce n'est pas un résidu de juin, c'est une récidive en cours.** Elle
sort du périmètre de #1696 (dont la condition portait sur le *déploiement* du
bundle, et il a eu lieu) et doit devenir son propre ticket — voir §4, F1.

### Condition 2 — « mika#1694 has shipped » → **NON livrée, et l'énoncé a changé de forme**

Rien n'est livré : aucune cible `worktrees-audit` / `worktrees-clean` dans le
`Makefile`, aucun script `scripts/*worktree*`, aucun reaper côté moteur. Le seul
nettoyage existant est une **instruction de prompt** (`self-dev/system_prompt.md`
étape 2), c'est-à-dire précisément la forme d'application que ce dépôt a mesurée
comme inopérante sur le substrat de boucle
(`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`).

Mais les deux moitiés de #1694 ont **divergé** entre juin et septembre :

| axe | 2026-06-30 (corps du ticket) | 2026-09-17 (mesuré) | verdict |
|---|---|---|---|
| worktrees | 13 | **2** | résolu autrement |
| branches `origin` non fusionnées | « 30+ » | **166** | **×5,5** |

Les worktrees sont traités en pratique par la skill opérateur
`/mika-platform-worktree-cleanup`. La dette de **branches**, elle, a explosé :
78 des 166 sont antérieures à septembre, dont 60 datent de mai/juin ou avant —
trois à six mois de branches mortes.

**Et un piège structurel a été trouvé en chemin, qui est le vrai apport du
grooming sur cet axe.** Ce dépôt fusionne en **squash** : chaque titre de `main`
porte un `(#NNNN)`. Après un squash-merge, la branche d'origine **n'est jamais
ancêtre de `main`**. Donc :

- `git branch -r --no-merged origin/main` compte 166 branches — dont la quasi-
  totalité sont déjà fusionnées côté forge. Le chiffre **surestime massivement**.
- Symétriquement, un `make worktrees-clean` bâti sur le prédicat naïf
  `git branch --merged` **ne reaperait jamais rien**, sur aucune branche, en
  silence. Il se lirait comme un outil qui n'a rien trouvé à nettoyer.

C'est la classe de panne que ce dépôt documente à répétition : *un instrument
silencieusement inerte se lit exactement comme un instrument qui n'a rien trouvé*
(mika#2205, mika#2131). Le prédicat de #1694 doit interroger **l'état de la PR
côté forge** (`gh pr list --state merged --head <branch>`), jamais l'ancestry git.

Ce constat appartient à #1694 et doit y être porté (§4, F2). Il n'est pas
implémenté ici.

### Condition 3 — « mika#1687 hypothesis confirmed for both stuck pilots » → **dépassée**

Le sous-ticket a produit un correctif livré (`ffb09cf4 … (#1859)`, le
*childless-parent reaper*), et une lignée entière l'a depuis dépassé : watchdog
PID #959, reaper `stuck-pending` #2045 (`c936003c … (#2061)`), reaper de stall
silencieux #2249/#2277, phantom sweep #1712/#2156. Toute cette famille est
documentée dans `CLAUDE.md`.

La condition telle qu'écrite — confirmer l'hypothèse **pour les deux pilotes
bloqués** du 2026-06-30 — porte sur deux lignes de tâche vieilles de 2,5 mois.
Elle n'est ni vérifiable (les tables ont été compactées depuis) ni utile : la
classe a reçu cinq correctifs structurels. **Condition à déclarer dépassée, sans
chercher à la satisfaire à la lettre.**

### Condition 4 — « mika#1680 reframed (revert + calibration suite hardening) » → **caduque, et auto-contradictoire**

Cette condition exige un recadrage dont la première moitié est le **revert de
modèle**. Or le même corps de ticket, quelques paragraphes plus haut, porte la
décision de Vincent du 2026-07-01 ~00:30Z :

> *« glm stays. Drop the revert entirely; it's not happening, so everything that
> hung off it goes too. »*

La condition de clôture 4 n'a jamais été mise à jour après cette décision : **le
ticket exige une chose que le ticket a annulée.** Le temps a tranché une seconde
fois — la flotte est passée à glm-5.3/kimi depuis (cf. `CLAUDE.md` § budgets LLM),
donc « revert vers Sonnet-4-6 » ne désigne plus aucun état atteignable.

Ce qui reste de #1680 (le *CJK bleed*, classé par Vincent « tracked annoyance, not
a blocker ») n'a aucun commit dans ce dépôt. Sa seconde moitié — durcir la suite de
calibration en garde de swap de modèle — est un vœu, pas une condition de clôture
d'incident. **Condition à déclarer caduque**, le reste routé (§4, F4).

### Condition 5 — « mika#1686 routed to its determined fix shape » → **non actionnable dans ce dépôt**

Le classifieur de permission que #1686 met en cause est celui de **claude-pilot**
(le corps le dit : « cpp permission classifier »). Et la skill `permission-policy`
a été **retirée de ce dépôt** : `50e13e59 chore(mika): retire mika-relay agent +
permission-policy skill (mika#1193) (#1348)`.

Le défaut, lui, est toujours vivant — et mesuré **dans cette session de grooming** :
la deuxième commande lancée ici a été refusée par
`policy allow (bash-grep) vetoed — command chains a tier3-dangerous or
command-substitution tail onto the allowed prefix`, sur un simple enchaînement de
`git log --grep` et d'`echo`. C'est exactement la forme n=8+ que #1686 décrivait le
2026-06-30, inchangée au 2026-09-17.

Mais son substrat vit dans `senara-solutions/claude-pilot`, **hors de l'allowlist
de dépôts dispatchables de la boucle** (même contrainte que claude-pilot#168, cf.
`CLAUDE.md`). Aucune PR sur `senara-solutions/mika` ne peut satisfaire cette
condition. **À router, pas à satisfaire** (§4, F3).

## 3. Périmètre de ce travail

**Dans le périmètre :**

1. Un document `docs/solutions/dev-loop/` portant l'apprentissage composé de
   l'incident — le livrable que #1696 se donne lui-même.
2. Le routage explicite des restes en constats nommés (F1–F5), chacun avec son
   véhicule : ticket de suivi, sous-ticket existant, ou hors dépôt.

**Hors périmètre, délibérément :**

- Toute implémentation de **mika#1694** (audit/clean/reaping de branches) — c'est
  un sous-ticket ouvert et distinct, dont #1696 attend la livraison.
- Toute intervention sur **mika#1686** — substrat hors dépôt.
- Toute correction de la récidive **mika#1383** — nouveau défaut, nouveau ticket.
- Toute modification du corps de #1696 ou commentaire sur le ticket — le pipeline
  autonome (`dispatch-lib::_write_canonical_callout`) en est le seul écrivain.
- Le reaping effectif des 166 branches — un geste destructif qui n'appartient ni à
  un plan ni à un pilote.

## 4. Les constats à router (contenu du document)

| id | constat | véhicule |
|---|---|---|
| **F1** | mika#1383 récidive : 4 rescues en 3 jours (15–17/09) après 6 semaines à zéro, contre 1 seule en juin. Le filet attrape, mais la cause produit à nouveau. | **Nouveau ticket.** La condition 1 de #1696 portait sur le déploiement du bundle, qui a eu lieu. |
| **F2** | #1694 a changé de forme : worktrees 13→2 (résolu par la skill opérateur), branches 30+→166 (×5,5, dont 78 antérieures à septembre). Et en dépôt squash-merge, un prédicat `git branch --merged` est **structurellement inerte** — il faut lire l'état de la PR côté forge. | **Porter sur mika#1694** (re-spécification), ne pas implémenter ici. |
| **F3** | #1686 toujours vivant — reproduit dans cette session même — mais son substrat (`permission-policy`) a quitté ce dépôt en mika#1193/#1348. | **Router vers `senara-solutions/claude-pilot`**, par spawn manuel (hors allowlist de la boucle). |
| **F4** | La condition de clôture 4 exige un revert que le même ticket a annulé le 2026-07-01, et que le temps a rendu inatteignable. | **Déclarer caduque** dans le document. |
| **F5** | La condition 3 (#1687) est dépassée par cinq correctifs structurels descendants ; sa lettre porte sur deux tâches de juin non vérifiables. | **Déclarer dépassée** dans le document. |

**Leçon transversale, qui est la raison d'être du document :** un épique
d'incident dont les conditions de clôture sont figées à l'instant de l'incident
**pourrit**. Ici, à 2,5 mois, une condition sur cinq était encore vraie telle
qu'écrite. Deux étaient satisfaites par d'autres véhicules, une était
auto-contradictoire avec une décision prise dans son propre corps, une visait un
substrat sorti du dépôt. Un épique doit être daté, et **re-mesuré avant d'être
exécuté** — jamais exécuté sur son énoncé d'origine.

## 5. Limite de ce grooming, déclarée

**Cette session n'avait pas de `GH_TOKEN`** (`gh auth` a échoué au premier appel).
Tout l'état ci-dessus est reconstruit depuis le dépôt local : `git log`,
`git branch -r`, le `Makefile`, l'arbre des skills.

Conséquence pour l'implémenteur : **l'état *GitHub* des sous-tickets (#1694,
#1686, #1687, #1680, #1697) n'a pas pu être lu.** Un sous-ticket peut avoir été
fermé sans commit dans ce dépôt (fermé comme caduc, migré, ou dupliqué). Le
premier geste de l'implémentation est donc de relire ces cinq états via
`gh issue view`, et **d'amender le document si un état contredit le tableau du
§2** — un document qui affirmerait « non livré » sur un ticket fermé serait pire
qu'un document absent. Aucune des mesures purement locales (les quatre rescues
#1383, les 166 branches, les 2 worktrees, l'absence de cible `Makefile`, le retrait
de `permission-policy`) ne dépend de cette lecture.

## 6. Étapes d'implémentation

1. **Relire l'état GitHub des cinq sous-tickets** (`gh issue view <n> --repo
   senara-solutions/mika` pour 1694, 1686, 1687, 1680, 1697). Noter état et date de
   fermeture éventuelle. Amender le §2 du document en cas de contradiction.
2. **Écrire** `docs/solutions/dev-loop/epique-dincident-perime-avant-son-execution-2026-09-17.md`
   avec le frontmatter YAML de la maison (`module`, `tags`, `problem_type`,
   `category`), portant : l'anatomie du wedge (résumée depuis le corps), le tableau
   des cinq conditions relues, les cinq constats F1–F5 avec leur véhicule, et la
   leçon transversale du §4.
3. **Ouvrir le ticket de suivi F1** (récidive mika#1383) via `/mika-issue`, en y
   portant la distribution mensuelle mesurée et les quatre SHA
   (`3048aeb8`, `15280de2`, `1b7acca9`, `e85a0b46`).
4. **Porter F2 en commentaire sur mika#1694** : les deux mesures de septembre et le
   piège du prédicat squash-merge. Ne rien implémenter.
5. **Porter F3 en commentaire sur mika#1686** : substrat hors dépôt depuis
   mika#1193, à traiter par spawn manuel sur `claude-pilot`.
6. Vérifier que rien d'autre que le document n'est modifié (`git diff --stat`).

**Note de dispatch (mika#2211) :** le corps de la PR s'écrit dans un fichier
`pr-body.md` **à la racine du worktree**, passé en `--body-file pr-body.md`, puis
supprimé. Jamais dans `/tmp` (la permission-policy refuse toute écriture hors
worktree), jamais en heredoc.

## 7. Contrat de vérification

| # | quoi | comment |
|---|---|---|
| V1 | Le document existe et porte le frontmatter de la maison | `head -8` du fichier ; `module`, `tags`, `problem_type`, `category` présents |
| V2 | Les cinq conditions de clôture y sont chacune tranchées | une section par condition, chacune portant un verdict explicite parmi {satisfaite, non livrée, dépassée, caduque, non actionnable ici} |
| V3 | Chaque mesure chiffrée est reproductible | chaque chiffre du document est accompagné de la commande qui le produit |
| V4 | Aucun code moteur touché | `git diff --stat origin/main` ne montre que des fichiers sous `docs/` |
| V5 | Les constats sont routés, pas seulement listés | F1 a un numéro de ticket ; F2 et F3 ont une URL de commentaire |
| V6 | Le pipeline passe | `scripts/verify-pipeline.sh` vert (U2 : `## Acceptance criteria` présent et non vide dans ce plan) |

**Condition de halte.** Si la relecture GitHub de l'étape 1 révèle que #1694 est
**fermé et livré** par un véhicule invisible au dépôt local, alors la condition 2
est satisfaite et l'épique se ferme sur les cinq conditions : le document reste
dû, mais F2 devient un constat historique et l'étape 4 disparaît. Ne pas rouvrir
#1694 par réflexe.

## Definition of Done

- Le document de solution est écrit, commité, et lisible sans le corps du ticket
  d'origine (il se suffit à lui-même).
- Les cinq conditions de clôture de mika#1696 sont chacune tranchées par un verdict
  explicite, chaque verdict adossé à une preuve reproductible (SHA, commande, ou
  citation datée du ticket).
- F1 a donné lieu à un ticket ouvert ; F2 et F3 ont donné lieu à un commentaire sur
  leur sous-ticket respectif.
- Aucun fichier hors `docs/` n'est modifié.
- `scripts/verify-pipeline.sh` est vert.
- mika#1696 est prêt à être fermé par l'opérateur — le plan ne le ferme pas
  lui-même (la fermeture d'un épique est un geste opérateur).

## Acceptance criteria

*Le corps de mika#1696 ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de sa « Closure condition » et du contrat de
vérification du §7.*

- **AC1** — Un fichier `docs/solutions/dev-loop/*.md` existe, porte le frontmatter
  YAML à quatre clés (`module`, `tags`, `problem_type`, `category`) conforme aux
  documents voisins de ce répertoire, et documente l'incident du 2026-06-30.
- **AC2** — Le document tranche **les cinq** conditions de clôture de mika#1696,
  une par une, chacune avec un verdict explicite et une preuve : condition 1
  satisfaite (SHA `d8f780ab`, `2aa9bc5c`), condition 2 non livrée (absence de cible
  `Makefile` et de script), condition 3 dépassée (SHA `ffb09cf4` et lignée),
  condition 4 caduque (citation datée de la décision du 2026-07-01 qui l'annule),
  condition 5 non actionnable ici (SHA `50e13e59`).
- **AC3** — Le document porte la distribution mensuelle mesurée des rescues
  `wip(…) (mika#1383)` — 1 en juin, 0 en juillet, 0 en août, 4 en septembre — avec
  la commande qui la reproduit.
- **AC4** — Le document porte les deux mesures de dette de #1694 au 2026-09-17
  (2 worktrees, 166 branches `origin` non fusionnées) **et** l'énoncé du piège
  squash-merge : un prédicat fondé sur `git branch --merged` est structurellement
  inerte dans ce dépôt, le prédicat correct interroge l'état de la PR côté forge.
- **AC5** — Le document porte la leçon transversale : un épique d'incident dont les
  conditions de clôture sont figées à l'instant de l'incident périme, et doit être
  re-mesuré avant d'être exécuté.
- **AC6** — Un ticket de suivi est ouvert pour la récidive mika#1383 (F1), portant
  les quatre SHA mesurés ; son numéro est cité dans le document.
- **AC7** — Un commentaire est posté sur mika#1694 portant F2, et un sur mika#1686
  portant F3 — sauf si l'étape 1 établit que le sous-ticket concerné est déjà
  fermé, auquel cas le document le consigne à la place.
- **AC8** — `git diff --stat origin/main` ne liste aucun fichier hors `docs/` :
  aucune ligne de moteur, de skill ou de script n'est modifiée par cette PR.
- **AC9** — `scripts/verify-pipeline.sh` termine sans échec.
