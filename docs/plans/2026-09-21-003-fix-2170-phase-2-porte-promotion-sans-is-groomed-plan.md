# mika#2170 — la Phase 2 est la seule entrée de la porte non précédée d'`is_groomed`

**Issue:** senara-solutions/mika#2170
**Type:** fix (observabilité + lisibilité d'un refus ; **aucune décision de promotion ne change**)
**Priorité au ticket:** p3-nice-to-have
**Branche:** `fix/2170/auto-pull-la-phase-2-est-la-seule-entr-e`

---

## Problème

### Ce que le ticket affirme, et qui se vérifie

Les trois faits qualitatifs du corps sont exacts à HEAD (`c8520787`) :

| chemin | filtre en amont | atteint la porte ? |
|---|---|---|
| Phase 0 `phase0_feed_ready_pool` | `is_groomed` | non |
| Phase 1 `phase1_promote_groomed` | `is_groomed` via `select_best_candidate` | non |
| **Phase 2** `phase2_reconcile_stuck_ready` | label `ready` + opérateur-held + plan mal attribué + âge + in-flight | **oui** |

Le commentaire qui nomme la classe est bien au site d'appel de la Phase 2
(`crates/mika-agent/src/auto_pull.rs:3830-3841`), la fixture `#2048` est gelée, et
`auto_pull_replay_2048_no_plan_file_is_refused_and_the_prefix_is_the_sole_cause`
asserte la classe à découvert.

**Les numéros de ligne du corps sont périmés** — `auto_pull.rs` fait 7286 lignes,
le site Phase 2 est à `:3841` et non `:2653`, `is_groomed` à `:484` et non
`:301`. Aucune conclusion n'en dépend ; c'est dit pour qu'un relecteur qui suit
les références ne conclue pas que le code a changé de forme.

### F1 — la population est plus étroite que le corps ne le dit

`classify_promotion` rend `Promote { detail: "no_branch_callout" }` sur
`StalenessMeasurement::NoBranchCallout` (`:924-928`). Un ticket `ready` **sans**
callout `> - **Branch:**` ne peut donc pas être refusé : il promeut.

La population à risque n'est pas « un ticket `ready` non groomé ». C'est :

> un ticket `ready` portant un callout `Branch:` **ancré et valide**, dont la
> branche **existe** sur origin, est **`behind_by > 0`**, et dont `changed_files`
> ne contient **aucun** fichier sous `docs/plans/`.

C'est-à-dire un ticket **partiellement** groomé — branche annoncée, plan absent
ou callout non canonique. Le corps décrit « une branche faite à la main, sans
plan, portant un unique commit de code » : exact, à condition qu'un callout
`Branch:` la désigne.

### F2 — LE RÉSULTAT PRINCIPAL : la condition de réveil est invérifiable telle qu'écrite

La condition de réveil du dormeur est :

> le premier enregistrement d'audit émis par `phase2_stuck_rescue` portant
> `reason=salvage_work_on_stale_branch` **dont la liste `non_plan_files` ne
> contient aucun frère sous `docs/plans/`**.

Or `non_plan_files` est défini (`:668-675`) par :

```rust
changed.unwrap_or(&[]).iter().filter(|f| !f.starts_with(PLAN_PATH_PREFIX)).cloned().collect()
```

**Par construction, `non_plan_files` ne contient jamais un fichier sous
`docs/plans/`.** La condition est donc **trivialement vraie pour tout refus
salvage**, quel qu'il soit.

Les deux lectures possibles échouent, chacune à sa façon :

1. **À la lettre** — elle est satisfaite par le **premier** refus salvage venu,
   y compris sur une branche de grooming parfaitement normale portant son plan
   *et* du code (le cas **nominal** de mika#2140, corps gelé : fixture `#1680`,
   4 × `crates/**` + 1 plan). Un lecteur appliquant la lettre réveille le ticket
   sur la population que mika#2140 a construite la règle pour attraper — pas sur
   celle que mika#2170 surveille.
2. **Dans son intention** (« la branche ne porte aucun plan ») — elle se teste
   sur `changed_files`, que l'audit **n'émet pas**. `staleness_audit_json`
   (`:1014-1072`) émet `changed_files_count` (un cardinal), `non_plan_files`
   (liste plafonnée à `MAX_NAMED_FILES = 10`) et `non_plan_files_count` (total
   non tronqué). L'intention est calculable — par la soustraction
   `changed_files_count − non_plan_files_count == 0` — mais **par aucun des
   champs que la condition nomme**.

C'est la raison pour laquelle personne n'a pu dire, en sortant le ticket du parc
le 21/09, si sa condition était remplie : **l'instrument ne répond pas à la
question qu'il prétend poser.** Classe miroir de mika#2272, où la condition de
flip était *insatisfiable* ; ici elle est *trivialement satisfiable*, ce qui est
pire — elle ne se tait pas, elle répond faux.

Un dormeur dont l'instrument est faux est moins qu'un dormeur : c'est une veille
qui se croit armée.

### F3 — la résolution 1 du ticket est réfutée

« Filtrer `is_groomed` dans le chemin de sauvetage de la Phase 2 » supprimerait
le rattrapage sur l'**état d'entrée nominal du pipeline**. `CLAUDE.md` § mika#2020
l'écrit mot pour mot :

> `ready` on an ungroomed ticket is the pipeline's **nominal** entry state, and
> re-driving it is how dev-groom gets another chance.

Et mika#996 (auto-groom on dispatch) rend ce secours **productif** : un ticket
`ready` sans callout `Plan:` qui atteint le dispatch fait partir `dev-groom`
d'abord. Secourir un ticket non groomé n'est donc pas un gaspillage, c'est la
voie normale.

On échangerait le filet de la population **majoritaire** de la Phase 2 contre la
fermeture d'un trou dont la population mesurée est **vide**. Le coût que le
ticket qualifie de « peut-être correct » est en réalité une régression de
contrat, mesurable et documentée ailleurs.

### F4 — la dissymétrie n'est pas un oubli : elle découle de deux rôles

Le tableau du ticket aligne trois « entrées de la porte » comme trois instances
d'une même chose. Ce sont **deux** choses :

- **Phase 0 et Phase 1 *posent* `ready`.** Elles décident d'engager un ticket que
  personne n'avait engagé. Exiger qu'il soit groomé est la condition même de
  l'engagement.
- **Phase 2 *rejoue* un `ready` que quelqu'un d'autre a déjà posé** — opérateur,
  webhook, ou la boucle elle-même. Elle ne décide pas d'engager : elle répare une
  livraison perdue.

Exiger `is_groomed` au rattrapage reviendrait à **désavouer la décision d'un
opérateur** qui a posé `ready` à la main. La dissymétrie est la conséquence de la
différence de rôle, pas son oubli. C'est ce qui réfute la résolution 1 par
construction plutôt que par un coût — et c'est ce qui manquait au corps du
ticket, qui pose l'alignement des trois chemins comme un bien en soi.

### F5 — la résolution 2 du ticket est réfutée

La raison de la règle salvage est écrite au site (`:953-976`) et n'est **pas** la
prédiction de conflit :

> a stale branch carrying partial work from a dead pilot has **two** legitimate
> resolutions — rebase the work, or abandon it — and choosing between them is a
> judgement about *work*, not about git.

Une branche faite à la main portant du code est le cas **le plus fort** de cette
règle, pas son faux positif : le travail y est humain et délibéré, donc le
jugement humain est encore plus dû. Élargir `PLAN_PATH_PREFIX` pour la laisser
passer trahirait la raison de la règle, en plus de heurter l'interdiction
explicite de mika#2123 KTD2b.

### F6 — le défaut réel est dans ce que le refus DIT, pas dans ce qu'il décide

Le refus est **correct**. Ce qui est faux est son diagnostic.

Sur une branche sans aucun plan, `reason` (`:783-795`) annonce :

> « modifie **{n} fichier(s) hors `docs/plans/`** — du travail qui n'est pas du
> grooming »

— vrai, et **sans information** : *tout* y est hors grooming, la branche n'a
jamais été une branche de grooming. Et `remedy` (`:811-817`) prescrit :

> « décide du sort du **travail partiel** porté par `{branch}` : le rebaser et le
> garder, ou l'abandonner explicitement en re-groomant #N sur une branche neuve »

— un cadrage de **reliquat de pilote mort** appliqué à du travail humain
délibéré. L'opérateur qui lit ça sur sa propre branche cherche un pilote qui n'a
jamais existé, puis se voit proposer « abandonner » ce qu'il vient d'écrire.

C'est exactement le patron que mika#2140 a lui-même employé : le refus était
correct, ce qui manquait était de **nommer les fichiers** pour que l'opérateur
puisse agir. Ici ce qui manque est de **nommer la nature de la branche**.

### F7 — un piège à ne pas prescrire

Groomer le ticket sur cette branche **ne lève pas** le refus : `dev-groom` ajoute
`docs/plans/x.md`, le code reste, `non_plan_files` reste non vide, le refus
revient au tick suivant. Le message ne doit donc pas prescrire le grooming comme
remède — ce serait une gestuelle inerte, classe mika#2361 (reposer `ready` sur un
ticket tenu).

---

## Requirements

- **R1.** La question « cette branche porte-t-elle un fichier de plan ? » est
  lisible **directement** sur la trace d'audit, sans soustraction de champs.
- **R2.** Le refus sur une branche sans plan est **comptable séparément** du refus
  sur une branche de grooming portant du code.
- **R3.** Le diagnostic servi à l'opérateur nomme la nature de la branche, et ne
  qualifie pas de « travail partiel » ce qui peut être du travail complet.
- **R4.** Le remède ne prescrit aucune gestuelle inerte (F7).
- **R5.** **Aucune décision de promotion ne change.** Tout ticket promu avant
  l'est après ; tout ticket refusé avant l'est après, sous un slug possiblement
  différent.
- **R6.** La dissymétrie Phase 2 est documentée **comme correcte**, avec les deux
  réfutations, au site où un futur lecteur la rencontrera.
- **R7.** Aucune décision de politique n'est prise — ni élargissement de
  `PLAN_PATH_PREFIX`, ni filtre `is_groomed` en Phase 2. L'interdiction écrite de
  mika#2140 est respectée.
- **R8.** La scission de slug est traitée comme un **format de fil** : datée,
  documentée, avec la règle de lecture de part et d'autre du déploiement.

---

## Décisions

### D1 — nouveau variant d'enum, pas un booléen sur le variant existant

`RefusalReason::StaleBranchWithoutPlan { branch, behind_by, ahead_by, changed_files }`
plutôt qu'un champ `carries_plan: bool` sur `SalvageWorkOnStaleBranch`.

Le `match` sur `RefusalReason` est exhaustif à quatre sites (`slug`, `branch`,
`reason`, `remedy`) : **le compilateur force chacun à décider**, ce qui est le
patron maison (modèle `hosting_ground_truth_line`, mika#2290 ; `ReadyLabelGate`,
mika#2323). Un booléen laisserait `slug()` cesser d'être un mapping
variant → chaîne, et rendrait la garde de format de fil indirecte.

### D2 — le nom du slug est `stale_branch_without_plan`

Il nomme le **fait mesuré** (la branche ne porte aucun plan), jamais une
inférence sur qui l'a produite. `hand_made_branch` serait une hypothèse sur
l'auteur, que la porte ne mesure pas.

### D3 — le slug est un format de fil : la scission est datée, pas rétroactive

`reason` atterrit dans `audit_events` et un opérateur en fait des `GROUP BY`. Les
lignes déjà posées gardent `salvage_work_on_stale_branch` : les réécrire rendrait
faux ce qu'elles disaient au moment où elles ont été écrites. **Un opérateur qui
compare de part et d'autre du déploiement doit sommer les deux noms.** Précédent
identique et raisonnement repris tel quel : mika#2361, scission
`operator_review_or_blocked` / `abandoned_operator_held`.

La moitié qui ne change pas de nom reste comparable à elle-même : une branche de
grooming portant du code continue de produire l'ancien slug.

### D4 — `plan_files_count` est **posé**, pas laissé à dériver

Le champ est ajouté à `staleness_audit_json`. Laisser l'opérateur calculer
`changed_files_count − non_plan_files_count` serait reproduire exactement le
défaut F2 : une question calculable que personne ne calcule. Doctrine mika#2131 —
la valeur qu'un opérateur `GROUP BY` est posée, pas dérivée.

C'est une **dimension brute**, pas une classification (Signal O, mika#2331) : un
compte, dont le booléen « porte un plan » se dérive, et non l'inverse. Il couvre
aussi les **promotions**, où il n'y a pas de slug de refus à lire.

`null` — jamais `0` — quand la liste n'a pas pu être lue, comme ses deux voisins.

### D5 — troncature : le nouveau slug n'est émis que sur une liste complète

L'endpoint `compare` de GitHub plafonne `files` à **300** entrées. Une branche
modifiant davantage pourrait voir son unique fichier de plan coupé, et le nouveau
slug affirmerait « aucun plan » à tort.

`GITHUB_COMPARE_FILES_CAP: usize = 300` ; le nouveau variant n'est produit que si
`changed_files.len() < CAP`. Au-delà, l'ancien variant est produit — c'est-à-dire
le comportement d'aujourd'hui, donc **aucune régression**. Application du patron
maison « un signal qu'on ne peut pas lire n'est jamais un terme satisfait »
(mika#2277), dans le sens sûr : on ne prétend pas mesurer une absence sur une
liste qui peut être coupée.

### D6 — le remède de fond ne change pas ; le diagnostic, si

F7 établit que groomer ne lève pas le refus. Le remède reste donc « décide du
sort du travail porté par cette branche ». Ce qui change :

- le **fait posé** — cette branche ne porte aucun fichier de plan, ce n'est pas
  le reliquat d'un pilote de grooming ;
- le mot **« partiel »**, retiré : la porte ne sait pas si ce travail est
  inachevé, et l'affirmer est une inférence qu'elle n'a pas mesurée ;
- une **phrase de butée** disant que groomer le ticket ne lèvera pas ce refus.

### D7 — le ticket se ferme ; la classe passe du ticket à l'instrument

Le corps dit : « le ticket existe pour que la classe reste visible ». Après ce
travail, la classe est visible **dans l'instrument** — un slug comptable en une
requête — plutôt que dans un ticket ouvert. C'est exactement le changement de
support que `docs/dormeurs.md` a ratifié le 2026-09-03 (« la visibilité change de
support »).

La **décision de politique** (faut-il un jour élargir le préfixe, ou filtrer ?)
reste ouverte et attend toujours un cas réel — mais elle l'attend désormais avec
un **compte**, pas avec un argument. Si la sonde ci-dessous rend une population
non nulle, c'est là qu'elle se prend, dans son propre ticket.

mika#2170 n'était pas dans `docs/dormeurs.md` (vérifié) ; il n'y entre pas non
plus — il n'y a plus de condition d'attente à consigner.

### D8 — ce qui est explicitement refusé

| refusé | pourquoi |
|---|---|
| filtrer `is_groomed` en Phase 2 | F3 + F4 — supprime le filet de l'état d'entrée nominal, et désavoue une décision opérateur |
| élargir `PLAN_PATH_PREFIX` | F5 — trahit la raison de la règle ; KTD2b l'interdit |
| un prédicat « document vs code » | même raison ; et la porte n'a pas de checkout pour juger du contenu |
| réécrire les lignes d'audit historiques | D3 — rendrait faux ce qu'elles disaient |
| prescrire le grooming comme remède | F7 — gestuelle inerte |

---

## Scope Boundaries

**Dans le périmètre :** `staleness_audit_json` (un champ additif), `RefusalReason`
(un variant + ses quatre `match`), `classify_promotion` (l'aiguillage entre les
deux variants, à décision constante), le commentaire du site d'appel Phase 2, les
tests, une addition courte à `CLAUDE.md`, et **la rectification du corps de
mika#2170** (U8 — geste de grooming en Phase 4, pas du pilote).

**Hors périmètre, nommé :**

- `PLAN_PATH_PREFIX` — inchangé (D8).
- `is_groomed` — inchangé, et aucun filtre ajouté en Phase 2 (D8).
- Les **décisions** de `classify_promotion` — aucune promotion ne devient refus,
  aucun refus ne devient promotion (R5).
- L'ordre des règles (salvage avant distance) — inchangé.
- `FILTER_PROMOTION_GATE` du ledger d'exclusion — il couvre tout refus de la
  porte et ne se scinde pas : le ledger répond « la porte a refusé », la raison
  vit dans `auto_pull_staleness_measured`. Scinder les deux ferait deux sites à
  garder d'accord pour une même question.
- Les fixtures — aucune n'est modifiée, ré-enrichie ni ajoutée. Le travail se
  teste intégralement sur celles qui sont gelées.
- **Aucun `cargo build` de vérification dans le worktree de grooming** : le
  commentaire opérateur du 21/09 borne le lot sur le disque (« tant que le disque
  reste sous 75 % ») et un `target/` pèse 25 à 44 Go (mika#2420). La compilation
  est le travail du pilote d'implémentation, pas de la passe de grooming.

---

## Implementation Units

### U1 — `plan_files_count` dans l'audit

`crates/mika-agent/src/auto_pull.rs`, `staleness_audit_json`. Champ additif,
calculé sur la liste lue : `changed_files_count − non_plan_files_count`, `null`
quand la liste n'a pas pu être lue. Doc-comment nommant la limite D5 (le compte
porte sur ce qui a été lu ; un `changed_files_count` à 300 signale une liste
possiblement coupée).

Additif ⇒ aucune requête existante ne casse.

### U2 — le variant `StaleBranchWithoutPlan`

`RefusalReason` + ses quatre `match` (`slug`, `branch`, `reason`, `remedy`).
`slug()` rend `"stale_branch_without_plan"`. Doc-comment du variant : la
population, le discriminant, et le renvoi à D5 pour la troncature.

### U3 — l'aiguillage dans `classify_promotion`

À décision constante. Sous la règle salvage existante, une fois `non_plan`
non vide établi :

- liste complète (`< CAP`) **et** aucun fichier sous le préfixe → nouveau variant ;
- sinon → variant existant.

Aucune branche nouvelle ne promeut ; aucune branche promue ne refuse.

### U4 — le message

`reason` : nommer que la branche ne porte **aucun** fichier de plan, donner le
compte et les fichiers (même plafond `MAX_NAMED_FILES`), et ne pas qualifier le
travail de « partiel ».
`remedy` : même fond que l'existant (décider du sort du travail), **plus** la
butée F7 — groomer ce ticket n'y changera rien, le plan s'ajoutera au code et le
refus reviendra.

### U5 — le commentaire du site d'appel Phase 2

`auto_pull.rs:3825-3841`. Réécrire : la dissymétrie est **correcte** (F4, avec les
deux rôles), les deux résolutions du corps sont **réfutées** (F3, F5), la
condition de réveil d'origine était invérifiable (F2) et le discriminant vit
désormais dans le slug. Le commentaire est le site où un futur lecteur rencontre
la question ; c'est là que la réponse doit être, pas seulement dans un plan.

### U6 — tests, à découvert sur les fixtures gelées

Dans `crates/mika-agent/tests/auto_pull_promotion_gate.rs` et les tests unitaires
du module (qui seuls voient le rendu privé des messages) :

| test | fixture | attendu |
|---|---|---|
| population positive | `2048` (3 config, **aucun plan**) | `StaleBranchWithoutPlan`, slug `stale_branch_without_plan`, message sans « partiel » |
| **contrôle négatif 1** | `1680` (4 × `crates/**` + 1 plan) | **garde** `salvage_work_on_stale_branch` |
| **contrôle négatif 2** | `1727` (1 plan + 1 doc hors préfixe) | **garde** `salvage_work_on_stale_branch` |
| invariant F2 | — | `non_plan_files` ne peut contenir aucun `docs/plans/` : le pin de la raison pour laquelle la condition de réveil était fausse |
| R5 | les 7 fixtures | l'ensemble `{promu}` / `{refusé}` est **inchangé** par le patch |
| D5 | liste synthétique ≥ `CAP` sans plan | **ancien** variant |

Le test `auto_pull_replay_2048_…` existant est **étendu, pas remplacé** : son
assertion sur `non_plan_files` reste, et son doc-comment est mis à jour (il
affirme aujourd'hui que la population vivante est vide « au 2026-09-04 » — la
date reste, la conclusion change de support).

**Les deux** tests de contrôle négatif — `auto_pull_replay_1680_is_refused_by_name`
(`:40`) et `auto_pull_replay_1727_is_the_measured_boundary_case` (`:253`) —
deviennent les **contrôles négatifs nommés** de la scission, et **chacun porte le
commentaire qui le dit**. Pas seulement le premier nommé : un contrôle dont le rôle
n'est pas écrit se lit comme une simple assertion de slug, et c'est celui-là qu'un
futur lecteur « harmonisera » vers le nouveau variant — rouvrant en silence ce que
la scission ferme. La protection vaut pour chaque contrôle, pas pour le seul premier
nommé (mika#2120 AC2 ; discipline de frontière — chaque contrôle porte son rôle).

Le cas de `1727` demande une **extension**, pas une réécriture, et c'est ce qui rend
l'omission dangereuse : son commentaire porte déjà un rôle — contrôle **positif** de
la question du préfixe (mika#2140), avec l'assertion auto-nettoyante sur
`behind_by > THRESHOLD` — et ce rôle-là dit « le préfixe classe ce doc comme
non-grooming : littéralement vrai, sémantiquement discutable ». Lue seule, cette
phrase invite précisément à croire que `stale_branch_without_plan` serait plus juste
ici. Elle ne l'est pas : la branche **porte** un plan, et c'est ce que le
commentaire étendu doit poser. Les deux rôles coexistent sur le même test ; aucune
des deux assertions existantes n'est retirée.

### U7 — `CLAUDE.md`

Addition **courte** à la section auto-pull existante : les deux slugs, la règle
de lecture de part et d'autre du déploiement (D3), la requête de comptage et sa
halte (cf. Verification Contract).

### U8 — rectification du corps de mika#2170 (**geste de grooming en Phase 4, pas du pilote**)

Trois affirmations du corps sont réfutées par ce plan, et le corps reste tel quel :

| affirmation du corps | statut | où |
|---|---|---|
| la condition de réveil (« … dont la liste `non_plan_files` ne contient aucun frère sous `docs/plans/` ») | **invérifiable telle qu'écrite** — trivialement vraie pour tout refus salvage, `non_plan_files` ne pouvant par construction contenir le frère qu'elle cherche | F2 |
| « Les deux résolutions possibles, à trancher au réveil » (filtrer `is_groomed` ; élargir `PLAN_PATH_PREFIX`) | **réfutées**, chacune par une raison distincte — la première supprime le filet de l'état d'entrée nominal et désavoue une décision opérateur, la seconde trahit la raison de la règle salvage et heurte l'interdiction de mika#2123 KTD2b | F3/F4, F5 |
| « Il ne se ferme pas : la classe reste réelle, seule sa population est vide » | **renversé** — la classe change de support, du ticket vers l'instrument | D7 |

Sans rectification, le prochain lecteur du ticket lit une spécification réfutée
présentée comme vivante — et il la lit **avant** la fermeture, la fenêtre s'ouvrant
au grooming et non au merge.

Forme, selon la convention maison (mika#2169, mika#2158, appliquée sur mika#2162),
**l'original conservé, jamais réécrit** :

- un **encadré daté en tête de corps** nommant les trois lignes du tableau
  ci-dessus — pour chacune : ce qui est remplacé ou réfuté, sa raison en une
  phrase, et le renvoi à ce plan ;
- un **commentaire d'avis d'édition** posté sur le ticket, pour que la
  modification du corps soit datée et attribuée dans la timeline plutôt que
  silencieuse.

**Une trace rectifiée dit ce qui était vrai au moment où elle a été écrite** :
c'est exactement le raisonnement que D3 applique aux lignes d'audit déjà posées,
et un corps de ticket est une trace de spécification au même titre (mika#2361,
précédent de scission de format de fil que ce plan invoque déjà).

**Pourquoi ce n'est pas le travail du pilote.** D7 fait dépendre la fermeture du
ticket de la ratification opérateur d'un changement de support : c'est une décision
de grooming, prise avec le plan sous les yeux, pas un effet de bord d'un patch. Le
pilote ne touche pas au corps du ticket — seul le groomer écrit U8, en Phase 4.

**Réserve déclarée, non prétendue :** `gh` n'est pas authentifié dans ce worktree
de grooming, donc la forme byte-exacte de l'encadré de mika#2162 n'a pas été relue
ici. Ce qui est prescrit est la **convention** (encadré daté en tête + commentaire
d'avis d'édition, original conservé) ; si mika#2162 porte une forme plus précise,
c'est elle qui fait foi.

---

## Verification Contract

### Surfaces opérateur

```sql
-- La population que mika#2170 surveillait, comptable en une requête.
SELECT json_extract(after_value, '$.reason') AS reason, COUNT(*)
  FROM audit_events
 WHERE tool_name = 'auto_pull_staleness_measured'
   AND json_extract(after_value, '$.outcome') = 'refuse'
 GROUP BY 1;

-- Les branches sans plan, avec leur phase.
SELECT created_at, reasoning AS phase, after_value
  FROM audit_events
 WHERE tool_name = 'auto_pull_staleness_measured'
   AND json_extract(after_value, '$.reason') = 'stale_branch_without_plan'
 ORDER BY created_at DESC;
```

**Régime attendu : `stale_branch_without_plan` à zéro.** La population mesurée le
2026-09-04 était vide et rien ne l'a alimentée depuis.

### Sonde post-déploiement, et ses quatre haltes

1. **Non-régression (la première, et la seule bloquante).** Sur 48 h, la
   distribution `outcome = 'promote'` / `'refuse'` est inchangée en volume. Un
   basculement est la seule chose que ce travail ne doit pas produire : il ne
   touche aucune décision. **Halte** — désarmer par revert, pas par réglage.
2. **Attribution.** Toute ligne `stale_branch_without_plan` est un cas réel de la
   classe. **C'est un résultat, pas une panne** : c'est la mesure que le ticket
   attendait depuis le 2026-09-04. Noter le ticket, la branche et la date.
3. **Halte — le nouveau slug porte le trafic nominal.** Si
   `stale_branch_without_plan` domine `salvage_work_on_stale_branch`, le
   discriminant est inversé ou la troncature mord : lire `changed_files_count`
   **avant** de toucher au prédicat. Une branche de grooming porte toujours son
   plan ; si elle est comptée sans, c'est le préfixe qui n'est pas apparié, pas la
   population qui a changé.
4. **Halte — la décision de politique.** Une population non nulle **n'autorise
   pas** à élargir le préfixe ni à filtrer en Phase 2 de sa propre autorité :
   c'est la décision que mika#2140 réserve, et elle se prend dans son propre
   ticket, avec le compte de la sonde 2 en pièce jointe.

**Le silence ne prouve rien si aucun ticket `ready` ne porte de callout
`Branch:`.** Vérifier que `outcome = 'promote'` avec
`measurement = 'measured'` est non vide avant de conclure quoi que ce soit d'un
zéro (classe mika#2205).

### Contrôles locaux

`cargo test -p mika-agent auto_pull` et `cargo test -p mika-agent --test
auto_pull_promotion_gate`, plus `cargo clippy` et `cargo fmt`. Exécutés par le
pilote d'implémentation, pas dans ce worktree (Scope Boundaries).

---

## Definition of Done

- `plan_files_count` émis sur chaque décision, `null` quand illisible.
- `RefusalReason::StaleBranchWithoutPlan` existe, avec ses quatre `match` étendus.
- L'aiguillage est à décision constante, et un test le pose sur les 7 fixtures.
- Le message ne dit plus « partiel » sur cette population et porte la butée F7.
- Le commentaire du site Phase 2 porte la réponse (F2–F5), pas la question.
- Les deux contrôles négatifs (`1680`, `1727`) gardent l'ancien slug, et **chacun
  des deux tests porte le commentaire qui le nomme contrôle négatif de la
  scission** (celui de `1727` étendu, son rôle mika#2140 conservé).
- `CLAUDE.md` porte les deux slugs, la règle de lecture datée, la requête et ses
  haltes.
- **Le corps de mika#2170 est rectifié** : encadré daté en tête nommant les trois
  réfutations (condition de réveil remplacée ; deux résolutions écartées avec leur
  raison ; support de visibilité déplacé du ticket vers l'instrument), plus un
  commentaire d'avis d'édition posté sur le ticket, l'original conservé. **Geste de
  grooming en Phase 4 (U8) — pas du pilote d'implémentation.**
- `cargo test`, `cargo clippy`, `cargo fmt` verts.
- Aucune fixture modifiée ; `PLAN_PATH_PREFIX` et `is_groomed` inchangés.

---

## Acceptance criteria

Le corps de mika#2170 n'a pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés des Requirements et du Verification Contract.

- **AC1.** La trace d'audit permet de répondre « cette branche porte-t-elle un
  fichier de plan ? » **sans soustraction de champs**, sur toute décision de la
  porte, promotion comprise.
- **AC2.** Un refus sur une branche sans plan est comptable séparément d'un refus
  sur une branche de grooming portant du code, par une requête `GROUP BY` sur un
  seul champ.
- **AC3.** **Aucune décision de promotion ne change** : sur les 7 fixtures gelées,
  l'ensemble des branches promues et celui des branches refusées sont identiques
  avant et après.
- **AC4.** Sur la fixture `2048` (aucun plan), le refus porte
  `stale_branch_without_plan` et son message ne qualifie pas le travail de
  « partiel ».
- **AC5.** Sur les fixtures `1680` et `1727` (plan présent), le refus **garde**
  `salvage_work_on_stale_branch`, et **chacun des deux tests porte le commentaire
  qui le nomme contrôle négatif de la scission** — la protection vaut pour chaque
  contrôle, pas pour le seul premier nommé (mika#2120 AC2). Celui de `1727` est
  étendu : son rôle de contrôle positif du préfixe (mika#2140) et ses deux
  assertions existantes sont conservés.
- **AC6.** Le remède servi à la population sans plan ne prescrit aucune gestuelle
  inerte, et dit explicitement que groomer le ticket ne lèvera pas le refus.
- **AC7.** Une liste `files` possiblement tronquée (≥ 300) ne produit **jamais**
  le nouveau slug.
- **AC8.** Un test pose que `non_plan_files` ne peut contenir aucun chemin sous
  `docs/plans/` — la raison pour laquelle la condition de réveil d'origine était
  invérifiable, gelée pour qu'elle ne soit pas réécrite ainsi.
- **AC9.** `PLAN_PATH_PREFIX` est inchangé, `is_groomed` est inchangé, et aucun
  filtre n'est ajouté en Phase 2.
- **AC10.** Le commentaire du site d'appel Phase 2 énonce la dissymétrie comme
  correcte et nomme les deux résolutions écartées avec leur raison.
- **AC11.** Le corps de mika#2170 porte un **encadré daté** nommant les trois
  rectifications — condition de réveil remplacée et pourquoi ; deux résolutions
  réfutées et pourquoi ; support de visibilité déplacé du ticket vers l'instrument
  — et un **commentaire d'avis d'édition** est posté sur le ticket. L'original est
  conservé, jamais réécrit. Sans cela, la trace de la réfutation ne vit que dans un
  fichier de plan et le prochain lecteur du ticket prend une spécification réfutée
  pour la spécification en vigueur.

---

## Fire-Disposition

**Le livrable EST un détecteur, et il est livré armé — mais il ne peut pas
« firer » sur une décision.**

`stale_branch_without_plan` est un nom donné à une population que la porte
refusait déjà. Il ne refuse rien de neuf : son émission est conditionnée à un
refus que le code produit aujourd'hui, à l'identique (AC3). Le détecteur ne
change donc pas ce qui arrive aux tickets ; il change ce que la trace en dit.

**Ce que ça implique pour la lecture du premier tir.** Une ligne
`stale_branch_without_plan` en production **n'est pas une régression** : c'est la
mesure que mika#2170 attendait depuis le 2026-09-04, et son apparition est le
succès de l'instrument, pas sa panne. C'est la halte 2 du Verification Contract,
et elle est formulée en ces termes précisément pour qu'un opérateur ne la traite
pas comme une alerte.

**Le seul tir qui serait une panne** est celui de la halte 3 — le nouveau slug
porterait le trafic nominal des branches de grooming. Il est tenu par deux
contrôles négatifs gelés (`1680`, `1727`) qui rougissent avant le déploiement, et
il est lisible en production sur `changed_files_count`.

**Aucun détecteur n'est ajouté pour la dissymétrie elle-même** (U5) : c'est de la
documentation, et elle n'a rien à signaler — la dissymétrie est établie comme
correcte, donc il n'y a pas d'état à surveiller. Y adjoindre une garde
reviendrait à surveiller une décision, ce que la maison ne fait pas.

---

## Suivi (hors périmètre, nommé)

- **La décision de politique reste ouverte.** Élargir `PLAN_PATH_PREFIX` ou
  filtrer `is_groomed` en Phase 2 sont réfutés *en l'état de la mesure* (F3, F5).
  Si la sonde 2 rend une population non nulle et que les cas réels montrent un
  coût, la question se rouvre — **dans son propre ticket**, avec le compte. Ce
  plan ne la préempte pas ; il la rend décidable.
- **`docs/dormeurs.md` — la forme des conditions de réveil.** mika#2170 est le
  premier cas mesuré d'une condition de réveil **trivialement satisfiable** : le
  contrat d'entrée du registre exige aujourd'hui qu'un lecteur puisse dire *sans
  contexte* si elle est remplie, mais pas qu'elle puisse être **fausse**. Une
  condition qui ne peut pas ne pas être remplie satisfait la lettre du contrat et
  n'en sert pas l'objet. **Ticket de suivi à ouvrir** : ajouter au contrat
  l'exigence d'un discriminant réfutable, et re-lire les 11 entrées actuelles à
  cette aune. Ce ticket-ci n'en porte rien — un seul cas est mesuré, et dessiner
  une règle sur un point est ce qui produit la mauvaise règle.
- **Les numéros de ligne dans les corps de tickets.** Ceux de mika#2170 étaient
  périmés de ~1200 lignes en 17 jours. Purement observationnel, aucun ticket
  ouvert.

---

## Références

- `crates/mika-agent/src/auto_pull.rs` — `is_groomed` `:484`, `PLAN_PATH_PREFIX`
  `:284`, `non_plan_files` `:668`, `classify_promotion` `:918`,
  `RefusalReason` `:723`, `staleness_audit_json` `:1014`, site Phase 2 `:3825-3855`

  *(Numéros relevés à HEAD `c8520787`. Ils périment — c'est ce qui est arrivé à
  ceux du corps du ticket en 17 jours ; les symboles, eux, sont stables.)*
- `crates/mika-agent/tests/auto_pull_promotion_gate.rs` —
  `auto_pull_replay_2048_no_plan_file_is_refused_and_the_prefix_is_the_sole_cause`
  `:151`, `auto_pull_replay_1680_is_refused_by_name` `:40`
- `crates/mika-agent/tests/fixtures/auto_pull_compare/PROVENANCE.md` — le tableau
  des 7 fixtures et les lignes `#2048`, `#1680`, `#1727`
- mika#2140 — le prédicat par fichiers, la mesure des 18 tickets, l'interdiction
  faite à l'implémenteur de trancher la politique
- mika#2123 — la porte de promotion, KTD2b (ne pas prédire le conflit)
- mika#2020 — `ready` non groomé est l'état d'entrée nominal du pipeline
- mika#996 — auto-groom on dispatch, qui rend le secours d'un non-groomé productif
- mika#2361 — précédent de scission de format de fil, datée et non rétroactive ;
  une trace rectifiée dit ce qui était vrai au moment de l'écriture (D3, U8)
- mika#2169, mika#2158 — convention maison de rectification d'un corps de ticket
  réfuté par son grooming ; appliquée sur mika#2162 (DoD : encadré daté en tête +
  commentaire d'avis d'édition, original conservé). Voir U8.
- mika#2120 AC2 — rendre une garde plus permissive ne doit pas rouvrir ce qu'elle
  ferme ; la protection vaut pour **chaque** contrôle négatif, pas pour le seul
  premier nommé (U6, AC5)
- mika#2131 — la valeur qu'un opérateur agrège est posée, pas dérivée
- mika#2277 — un signal qu'on ne peut pas lire n'est jamais un terme satisfait
- mika#2272 — la condition de flip insatisfiable (classe miroir de F2)
- `docs/dormeurs.md` — le contrat d'une condition de réveil

---

## Revision history

- **2026-09-21 — v1.** Plan initial. Le grooming déplace le ticket : les deux
  résolutions qu'il proposait sont réfutées (F3, F5), sa dissymétrie est établie
  comme correcte (F4), et sa **condition de réveil est invérifiable telle
  qu'écrite** (F2) — `non_plan_files` ne peut, par construction, jamais contenir
  le frère qu'elle cherche. Le travail livré répare l'instrument et le
  diagnostic, sans prendre aucune des deux décisions de politique que mika#2140
  réserve à un cas réel.

- **rev 2 (2026-09-21)** — révision sur findings de première passe architecte
  (`.iterate/findings-1.md`, `Disposition: ITERATE`). Les deux findings sont
  adressés ; aucun ne reste ouvert.
  - **F1 (bloquant)** — le plan réfutait le ticket sans prescrire la rectification
    de son corps. Ajout de l'unité **U8** (encadré daté en tête + commentaire
    d'avis d'édition, original conservé, rédigé par le groomer en Phase 4 et non
    par le pilote), avec le tableau des trois affirmations réfutées — condition de
    réveil (F2), deux résolutions à trancher (F3/F4, F5), support de visibilité
    (D7 contre « Il ne se ferme pas »). Répercuté sur le **DoD** (c'est
    l'exigence que le finding demandait explicitement d'y inscrire), sur
    **AC11**, et sur les Scope Boundaries. Citations conservées : convention
    maison mika#2169/#2158 appliquée sur mika#2162 ; cohérence avec mika#2361 (D3),
    dont le raisonnement — une trace rectifiée dit ce qui était vrai au moment de
    l'écriture — s'applique au corps d'un ticket comme aux lignes d'audit. Réserve
    déclarée dans U8 : `gh` n'étant pas authentifié dans ce worktree, c'est la
    convention qui est prescrite, non une forme relue sur mika#2162.
  - **F2 (affûtage)** — U6 n'exigeait le commentaire de rôle que sur `1680`. Les
    **deux** contrôles négatifs le portent désormais, et AC5 comme le DoD le
    posent. Précision ajoutée en chemin : le test `1727` **existe déjà**
    (`auto_pull_replay_1727_is_the_measured_boundary_case`, `:253`) et porte un
    commentaire de rôle — mais pour un **autre** rôle, contrôle positif de la
    question du préfixe (mika#2140). Son rôle actuel dit littéralement que le
    préfixe classe son doc comme non-grooming, « sémantiquement discutable » :
    lue seule, cette phrase invite à croire que `stale_branch_without_plan` serait
    plus juste là, alors que la branche porte un plan. Le commentaire est donc
    **étendu**, pas remplacé, et ses deux assertions existantes sont conservées —
    ce qui rend l'omission signalée par F2 plus coûteuse qu'un simple oubli de
    symétrie. Citation conservée : mika#2120 AC2.
