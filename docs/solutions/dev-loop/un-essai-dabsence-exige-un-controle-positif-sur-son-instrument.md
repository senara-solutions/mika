---
title: "Un essai d'absence n'est publiable qu'avec le contrôle positif de son instrument"
date: 2026-10-15
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh
component: development_workflow
problem_type: best-practice
severity: medium
symptoms:
  - "`grep` d'une signature d'invocation rendant 0 fichier sur 44 — résultat identique à celui qu'on espère et à celui d'une signature fausse"
  - "`find -newermt '2026-10-14 20:11'` classant comme post-merge un transcript clos à 19:12Z, parce que l'instant de référence est UTC et le mtime local"
  - "`gh pr list --limit 120` rendant exactement 120 lignes — un recensement qui se lit comme un compte et n'en est pas un"
  - "`git merge-base --is-ancestor` déclarant « allégé » un vol dont tous les commits propres précèdent la consigne de 3 minutes"
root_cause: measurement_instrument_unvalidated
resolution_type: documentation
tags: [mika-2694, mika-2693, pilot-transcripts, essai-borne, controle-positif, instrument, simplify, code-review, dev-loop]
---

# Un essai d'absence n'est publiable qu'avec le contrôle positif de son instrument

## Ce que ce document est

Le relevé de l'essai borné demandé par mika#2694 — le pipeline pilote allégé
(ni `ce-simplify-code` ni `ce-code-review`, vérification complète et PR gardées,
consigne portée par `CLAUDE.md` depuis `6f1e5a4a`, PR #2695, mergée le
**2026-10-14T20:11:06Z**) — et la méthode calibrée qui a produit ce relevé,
avec les **sept** propriétés de l'instrument mesurées en route. Quatre d'entre
elles sont des pièges : chacune rend un chiffre qui ressemble exactement au
résultat attendu.

Chaque affirmation porte la commande qui l'a produite. Chaque affirmation
d'**absence** porte en plus son **contrôle positif** : la même commande, mot
pour mot, trouvant au moins une occurrence dans un témoin où l'occurrence
existe. Sans ce contrôle, un zéro mesure la signature, pas le comportement.

---

## I. Le vol d'essai, et pourquoi ce n'est pas le plus ancien

### Recensement de la fenêtre

```bash
gh pr list --repo senara-solutions/mika --state all \
  --search 'created:>=2026-10-14T20:11:06Z' --limit 100 \
  --json number,createdAt,labels,state
```

→ **20 PR**, toutes `origin:loop`. 20 < 100, donc la page n'a pas rempli : le
compte est réel.

> **Piège 1 — un recensement par page n'est pas un compte.** La première forme
> essayée, `gh pr list --state all --limit 120`, rend **exactement 120**
> lignes ; portée à `--limit 400`, elle rend **exactement 400**. Le dépôt
> compte plus de 400 PR : la page remplit toujours, et le filtre par date
> s'applique *après*, sur un échantillon tronqué par le haut. La forme
> `--search 'created:>=…'` borne par le prédicat et non par la page ; sa
> longueur de retour, comparée à `--limit`, est la preuve de non-troncature.
> C'est la même précaution que l'`exit 3` de `scripts/pr-origin-report.sh`.

### Le test d'appartenance, et son défaut

Le `CLAUDE.md` **n'est pas** dans le transcript d'un vol de pilote (propriété
mesurée au grooming de mika#2694 : la phrase stable
`conversation-first AI executive assistant` rend 0 occurrence sur deux vols
post-merge). « Ce vol portait la consigne » ne se lit donc pas dans le
transcript et doit se dériver du dépôt. Le plan prescrivait :

```bash
git merge-base --is-ancestor 6f1e5a4a <branche-du-vol>
```

La PR `origin:loop` la plus ancienne de la fenêtre est **#2702**
(2026-10-14T20:37:16Z, mika#2693), et sa branche **passe** ce test :

```bash
git branch -r --contains 6f1e5a4a   # → origin/feat/2693/plan-size-gate-une-borne-par-vol-d-riv-e
```

Elle ne portait pourtant pas la consigne.

```bash
git log --format='%h | a:%ai | c:%ci | %s' \
  6f1e5a4a..origin/feat/2693/plan-size-gate-une-borne-par-vol-d-riv-e
```

→ onze commits, **dates d'auteur** de `2026-10-14 18:36:23 +0200` à
`22:08:29 +0200` (= 20:08:29Z, soit **2 min 37 s avant** le merge), et
**dates de commit toutes identiques** à `22:15:51 +0200` (= 20:15:51Z, soit
**après** le merge). Un seul rebase, postérieur au vol, a introduit `6f1e5a4a`
dans cette branche.

> **Piège 2 — l'ascendance d'un commit est réécrite par le rebase, la date
> d'auteur ne l'est pas.** `--is-ancestor` seul classerait #2702 « allégé ».
> Le critère d'appartenance est donc **conjonctif** : `6f1e5a4a` ancêtre
> **et** les commits propres du vol datés (date d'**auteur**) après l'instant
> du merge. `_set_up_worktree` rebase sur `origin/main` à la préparation :
> c'est cette préparation-là que la date d'auteur du premier commit du vol
> situe, et aucune relecture ultérieure de l'historique ne la déplace.

### Le vol d'essai désigné

La PR suivante, **#2703** (2026-10-14T21:09:36Z, mika#2697, `origin:loop`,
MERGED le 2026-10-15T05:46:06Z), passe le critère conjonctif :

```bash
gh pr view 2703 --repo senara-solutions/mika --json commits
```

→ premier commit propre `f4883021` (`docs(plans): groom mika issue#2697`)
daté **2026-10-14T20:28:06Z**, soit 17 minutes après le merge ; implémentation
`a88f45dd` à 21:08:09Z. Le worktree a donc été préparé sur une base portant la
consigne.

Sa branche d'origine, `fix/2697/mika-manager-rapports-livr-s-l-endpoint`,
**n'existe plus** (supprimée à la fusion) : elle est absente de
`git branch -r --contains 6f1e5a4a`. La commande du plan échoue sur un vol
fusionné ; la liste de commits de la PR, conservée par la forge, la remplace.

### Jonction transcript ↔ PR

```bash
cd ~/.mika/data/pilot-transcripts && grep -l 'pull/2703' *.jsonl
```

→ trois fichiers : `24ed814d-…`, et `07f5b3ca-…` + `d053218b-…`. Les deux
derniers sont les **sessions d'observation** du présent essai (premier horodatage
`2026-10-15T17:05:09Z` et `17:16:38Z`, et `grep -c '2026-10-15-004-chore-2694'`
→ 5 et 2 : ils citent ce plan). Reste
**`24ed814d-4ae9-4db9-afcc-c0242ab91307.jsonl`**, mtime
`2026-10-14 23:10:03 +0200` = 21:10:03Z, soit 27 s après l'ouverture de #2703.

```bash
grep -oh 'pull/2703[^"]\{0,20\}' 24ed814d-….jsonl | sort | uniq -c
#   2 pull/2703\n\n## Ce qui est li
```

> **Piège 3 — la session qui mesure est dans la population mesurée.** Le corps
> du ticket et le plan **citent** `ce-simplify-code` et `ce-code-review`. Tout
> transcript d'une session d'observation contient donc les deux chaînes, et une
> mesure non anchorée l'enregistre comme une invocation (§ III).

---

## II. Le candidat au « crochet inerte » se dissout

Le grooming avait identifié **un seul** transcript, parmi les 44 postérieurs au
merge, invoquant `ce-simplify-code` *et* `ce-code-review` :
`db73ec9f-57d8-416c-b6e1-a79c14a81a09.jsonl`. C'était le seul candidat au
critère 3 (« la consigne est inerte »).

Il n'est pas postérieur au merge.

```bash
ls -l --time-style=full-iso db73ec9f-….jsonl
# -rw-r----- … 1661050 2026-10-14 21:12:27.739664922 +0200 …
```

`21:12:27 +0200` = **19:12:27Z**, soit **58 minutes avant** le merge de
20:11:06Z.

> **Piège 4 — l'instant de référence est UTC, le `mtime` est local.** Le filtre
> `find … -newermt '2026-10-14 20:11'` interprète son argument en heure
> **locale**. Sous `+0200`, il sélectionne les fichiers postérieurs à 18:11Z et
> inclut deux heures de vols **antérieurs** à la consigne. Le seuil correct est
> `-newermt '2026-10-14 22:11:06'` (local), et la seule forme qui ne se trompe
> pas est celle qui affiche son décalage : `ls -l --time-style=full-iso` rend
> le `+0200` explicitement. (`TZ=UTC ls` est refusé par la permission-policy du
> bac à sable des pilotes.)

Ce que `db73ec9f` est réellement :

```bash
grep -oh 'mika#26[0-9][0-9]' db73ec9f-….jsonl | sort | uniq -c | sort -rn | head -3
#  66 mika#2693    49 mika#2636    14 mika#2687
grep -oh 'num_turns[^,]\{0,16\}' db73ec9f-….jsonl   # → num_turns": 201
grep -oh 'subtype[^,]\{0,24\}'   db73ec9f-….jsonl   # → subtype": "error_max_turns"
```

Le vol de **mika#2693**, mort à **201/200** en `error_max_turns` — et le commit
`2519e156 wip(mika#1383): auto-PR-create rescue for mika#2693`, daté
`21:12:36 +0200`, est postérieur de 9 secondes à la clôture de ce transcript :
c'est le filet de rescue qui a ouvert #2702, pas le vol.

**Conséquences.** (a) Le seul candidat au critère 3 était un vol **pré-consigne**,
dont l'invocation des deux skills est entièrement légitime et ne dit rien du
crochet. (b) C'est une **troisième** occurrence mesurée, après #2683 et #2654,
du mode de panne que mika#2694 a été ouvert pour corriger : mort au plafond dans
la fin de pipeline.

---

## III. Les quatre critères de l'essai

Signature d'invocation, établie avant toute mesure :

```bash
grep -oh '"name": "Skill", "input": {"skill": "compound-engineering:[a-z-]*"' <témoin>
# → "name": "Skill", "input": {"skill": "compound-engineering:ce-simplify-code"
```

Deux propriétés de forme s'y lisent, et toutes deux ont fait rendre zéro à une
première tentative :

- **les espaces.** `"skill":"compound-…"` sans espace après les deux-points
  rend 0 fichier sur 44 — faux négatif complet (mesuré au grooming).
- **l'ancrage.** La sous-chaîne nue `compound-engineering:ce-simplify-code`
  compte aussi les **mentions en prose**. Elle désigne 2 fichiers dans la
  population post-merge : les deux sessions d'observation, qui citent le nom
  sans l'invoquer. La forme ancrée sur l'appel d'outil `Skill` en désigne **0**.
  Un détecteur de régression doit s'ancrer, sans quoi il criera sur tout
  document parlant de ce ticket.

### Critère 2 — aucune invocation

```bash
cd ~/.mika/data/pilot-transcripts
find . -name '*.jsonl' -newermt '2026-10-14 22:11:06' | wc -l   # → 45
find . -name '*.jsonl' -newermt '2026-10-14 22:11:06' -exec \
  grep -l '"name": "Skill", "input": {"skill": "compound-engineering:ce-simplify-code"' {} \;
# → (vide)
find . -name '*.jsonl' -newermt '2026-10-14 22:11:06' -exec \
  grep -l '"name": "Skill", "input": {"skill": "compound-engineering:ce-code-review"' {} \;
# → (vide)
```

**0 invocation sur les 45 transcripts post-consigne** (dont 2 d'observation),
et en particulier 0 et 0 sur le vol d'essai `24ed814d`.

**Contrôle positif, même commande mot pour mot :**

| témoin pré-consigne | ticket | `ce-simplify-code` ancré | `ce-code-review` ancré |
|---|---|---|---|
| `500a76a5-…` | mika#2683 | **1** | 0 |
| `59ffa68e-…` | mika#2654 | **1** | 0 |
| `db73ec9f-…` | mika#2693 | **1** | **1** |

Les deux signatures trouvent. L'absence mesure donc le comportement. ✅

### Critère 1 — PR ouverte dans le budget

```bash
grep -oh 'num_turns[^,]\{0,20\}' 24ed814d-….jsonl   # → num_turns": 94
grep -oh 'subtype[^,]\{0,24\}'   24ed814d-….jsonl   # → subtype": "success"
```

**94 tours sur un plafond de 200**, terminaison `success` — 106 tours de marge.
À comparer aux 201/`error_max_turns` des trois vols pré-consigne ci-dessus. ✅

Le mécanisme se lit dans un seul chiffre :

```bash
grep -c '"parent_tool_use_id": null'    24ed814d-….jsonl   # → 157
grep -c '"parent_tool_use_id": "toolu'  24ed814d-….jsonl   # →   0
```

**Zéro tour de sous-agent.** Contre 70 pour `500a76a5` et 88 pour `59ffa68e`.
Ce que la fin de pipeline allégée économise n'est pas de la prose : c'est un
éventail de sous-agents.

### Critère 4 — la vérification a tourné, et avant la PR

Le JSONL est append-only : le numéro de ligne **est** l'ordre chronologique.
Sur `24ed814d` (158 lignes) :

| ligne | `grep -n` | étape |
|---|---|---|
| 57, 61, 78, 93 | `cargo test` | suite |
| 106 | `cargo fmt` | format |
| 107 | `cargo clippy` | lint |
| 117 | `verify-pipeline` | ⚠️ voir ci-dessous |
| 140 | `git commit` (dernier) | commit |
| 149 | `git push` | push |
| 152 | `verify-pipeline`, `cargo fmt` | revérification |
| **153** | **`gh pr create`** | **PR** |

L'ordre de l'avis claude-pilot#289 — « commite, pousse, vérifie, ouvre la
PR » — est tenu, avec une vérification avant le commit **et** une seconde
juste avant l'ouverture. ✅ avec une réserve :

```bash
grep -oh 'verify-pipeline[^"]\{0,120\}' 24ed814d-….jsonl
# → verify-pipeline.sh origin/main 2>&1 | tail -40
# → verify-pipeline.sh origin/main` | ⚠️ **non exécutée** — même refus de politique…
# → verify-pipeline.sh origin/main` sont refusées par la permission-policy de ce bac à sable
#   (`no matching policy rule -- denied by default`)
```

`scripts/verify-pipeline.sh` a été **appelé et refusé** par la permission-policy
du bac à sable (comme `make verify-bundled-skills`), et le corps de #2703 le
déclare au lieu de le revendiquer. **Cause étrangère à ce ticket** (famille
mika#2716, egress et empreinte d'outils du bac à sable) : elle frappe aussi les
vols non allégés. Ce qui a réellement tourné avant la PR : la suite complète
`cargo test -p mika-agent` (6141 tests), `cargo clippy --all-targets -D warnings`,
`cargo fmt --check`.

### La ligne fixe

```bash
gh pr view 2703 --repo senara-solutions/mika --json body --jq .body | grep 'Code review'
# → Code review: delegated to mika-qa (mika#2694)
```

Présente au mot près. ✅

### Critère 3 — le crochet est-il inerte ?

**Non : il tient.** Aucun vol post-consigne n'invoque l'un des deux skills
(critère 2, avec contrôle positif). Le candidat unique du grooming était un vol
**pré-consigne** (§ II). La barrière de complétion du plugin — qui refuse
« Never silently ship a non-mechanical change with no review of any kind » — a
donc bien reconnu le seuil de projet déclaré dans `CLAUDE.md` : le pilote a noté
la délégation dans le corps de PR et ouvert la PR sans revue par sous-agents.

**Aucun levier suivant n'est à nommer.** La phrase de saut reconnue et le réglage
du plugin, que le ticket envisageait en secours, restent inutiles.

**Borne de cette conclusion.** Un crochet respecté sur un échantillon de un vol
désigné, dans une population de 43 vols où aucun n'invoque ni l'un ni l'autre.
C'est une absence large et une attestation d'ordre étroite : l'ordre des étapes
et la ligne fixe ne sont vérifiés que sur #2703.

---

## IV. Relevé de couverture (AC3)

La question : la perte annoncée — réutilisation et duplication, efficacité,
correction **non évidente** — s'est-elle **vue** ?

### Ce que mika-qa a signalé sur la PR allégée

```bash
gh pr view 2703 --repo senara-solutions/mika --json reviews,comments
```

→ `nComments: 0`, trois revues de `mika-platform-qa` :

| instant | verdict | profondeur | fichiers |
|---|---|---|---|
| 21:17:29Z | `hold[review]` | — | callback **coupé** après 20 steps d'outil, aucun verdict de contenu |
| 21:29:55Z | `pass` | `code-level` | 7 |
| 22:16:40Z | `pass` | `code-level` | 17 (diff complet après merge de main) |

Deux lectures au niveau du code, **aucune trouvaille**. (Le `hold[review]`
initial est un défaut du côté QA — budget de steps du tour de callback — et non
une trouvaille.)

### Ce que simplify signalait sur les vols précédents

**mika#2683, transcript `500a76a5-…`** (201 tours, `error_max_turns`).
`ce-simplify-code` invoqué ligne 279 ; sous-agents « Revue de réutilisation du
code » et « Revue de qualité du code » dépêchés lignes 286 et 337 — le troisième
(efficacité) n'a jamais été dépêché. La prose du vol après la ligne 337 :

```bash
tail -n +337 500a76a5-….jsonl | grep -oh '"content": \[{"text": "[^"]\{0,420\}'
# contrôle positif du marqueur : grep -c '"content": \[{"text"' sur tout le
# fichier → 28 ; après la ligne 337 → 5
```

> « L'agent a été interrompu — je fais les deux passes restantes en direct »
> « Confirmé, et le doc du survivant interdit nommément cette copie
>   (« Nom neutre, et **un seul corps** », mika#2653) »
> « Les trois constats tiennent. J'applique le `branch_key` partagé »
> « Maintenant le constat n° 2 (prédicat de fenêtre recopié) et le n° 4
>   (littéral machine retapé dans un test) »

**Quatre constats numérotés, trois confirmés** : un corps dupliqué que le
document du survivant interdit par son nom, un prédicat de fenêtre recopié, un
littéral machine retapé dans un test. **Exactement la classe réutilisation /
duplication** que la grille fermée de mika-qa (Step 3b, « for these specific
issues only ») ne couvre pas.

**mika#2654, transcript `59ffa68e-…`** (201 tours, `error_max_turns`). Les
**trois** sous-agents dépêchés (« Revue réutilisation de code », « Revue qualité
de code », « Revue efficacité », lignes 303/336/340). Après la ligne 340 :

```bash
tail -n +340 59ffa68e-….jsonl | grep -oh '"name": "[A-Za-z]*"' | sort | uniq -c
#  54 Bash   12 Read   8 Edit   1 Agent
tail -n +340 59ffa68e-….jsonl | grep -c '"content": \[{"text"'   # → 0
# contrôle positif : même motif sur tout le fichier → 24
```

75 appels d'outils, dont **8 `Edit`**, et **zéro ligne de prose** : aucun verdict
n'a été rendu, le vol a été coupé en pleine passe.

### Ce que les trouvailles ont produit

```bash
tail -n +279 500a76a5-….jsonl | grep -c 'git commit'   # → 0
tail -n +297 59ffa68e-….jsonl | grep -c 'git commit'   # → 0
```

**Zéro commit après l'invocation de simplify, sur les deux vols.** Les 8 `Edit`
de chaque vol ont été appliqués au worktree et jamais validés. Et aucun des deux
transcripts ne contient de `pull/[0-9]{4}` : ni l'un ni l'autre n'a ouvert sa PR.
Les PR #2689 et #2690 portent toutes deux le label `wip-rescue` et ont été créées
18 s et 23 s après la clôture de leur transcript — par le filet de rescue.

### Verdict

**La perte de couverture est réelle, nommée, et démontrée dans les deux sens.**

- **Elle existe.** Trois trouvailles de classe duplication/réutilisation sur
  #2683, confirmées par le vol lui-même. Et mika-qa a approuvé le **même diff**
  (PR #2689, quatre revues, `DEPTH: code-level`, 13 fichiers) **sans en lever
  aucune** — la démonstration directe que cette classe est hors de sa grille.
- **Elle n'achetait rien, sur ces deux vols.** Aucune des trouvailles n'a été
  validée : zéro commit après simplify, zéro PR ouverte par le vol. La
  couverture a produit du travail que le dépôt n'a jamais reçu, et elle a coûté
  aux deux vols leur fin de pipeline.

**Ce que cet échantillon ne permet pas de dire.** Un vol allégé, deux témoins.
Que mika-qa n'ait rien signalé sur #2703 **ne prouve pas** qu'il n'y avait rien
à signaler : ce diff n'a jamais été soumis à un œil de classe réutilisation. La
question « un défaut de duplication atteint-il maintenant `main` plus souvent ? »
reste **ouverte** et ne se tranchera pas par un relevé de transcripts — elle
demande une mesure sur les régressions de `main`, à bâtir, hors de ce ticket.

Ce qui est tranché : le coût de la couverture retirée était **certain** (trois
vols morts au plafond dans la fin de pipeline, aucune PR ouverte par le vol), sa
valeur livrée était **nulle** sur l'échantillon observé.

---

## V. Règle à retenir

**Un essai d'absence n'est publiable qu'avec le contrôle positif de son
instrument** : la même commande, mot pour mot, trouvant au moins une occurrence
dans un témoin où l'occurrence existe. Les sept propriétés mesurées ici rendent
toutes, en cas d'erreur, le chiffre qu'on espérait :

| # | propriété | le zéro qu'elle fabrique | parade |
|---|---|---|---|
| 1 | page `gh` pleine | compte tronqué lu comme complet | `--search` + longueur < `--limit` |
| 2 | rebase réécrit l'ascendance | vol pré-consigne classé allégé | date d'**auteur** du premier commit propre |
| 3 | la session d'observation est dans la population | invocation fantôme | exclure l'observateur, ancrer la signature |
| 4 | `-newermt` est local, l'instant de merge UTC | 2 h de vols pré-consigne inclus | seuil local explicite, `--time-style=full-iso` |
| 5 | les espaces de la signature JSON | 0 sur 44, « comme espéré » | établir la forme par `grep -oh` avant de compter |
| 6 | les accents sont `\uXXXX` dans le transcript | `grep 'Revue de qualité'` → 0 | motifs sans accent, ou forme échappée |
| 7 | le transcript ne contient que le `response_body` | pas de `tool_result` : « rien n'est revenu » est inobservable | mesurer le côté assistant (commits, prose, appels d'outils) |

La n° 6 se mesure en deux commandes sur `500a76a5-…`, et c'est tout ce qu'il
faut pour la croire :

```bash
grep -c 'Revue de qualité du code'          500a76a5-….jsonl   # → 0
grep -c 'Revue de qualit\\u00e9 du code'     500a76a5-….jsonl   # → 1
```

La n° 7 a été attrapée par son contrôle : `grep -c 'tool_result'` rendait 0
après la dépêche des relecteurs — et **aussi 0 sur le fichier entier**. Un
transcript de pilote enregistre une ligne par réponse LLM
(`{"schema_version": "v1", …, "response_body": {…}}`), main session et
sous-agents distingués par `parent_tool_use_id`. Les résultats d'outils n'y
sont pas. L'affirmation « aucun relecteur n'a rien renvoyé » n'était pas
mesurable ; elle a été remplacée par ce qui l'est : zéro commit, zéro prose,
zéro PR.

## Liens

- mika#2694 — le ticket de l'essai ; `CLAUDE.md` § *Pilot flights end lean*.
- mika#2693 — borne de dimensionnement par vol ; son propre vol (`db73ec9f`) est
  la troisième mort au plafond mesurée ici.
- claude-pilot#289 — « commite, pousse, vérifie, ouvre la PR ».
- `docs/solutions/dev-loop/investigation-hypothesis-refuted-by-its-own-date-2026-09-05.md`
  — même famille : une mesure réfutée par sa propre date.
- `docs/solutions/dev-loop/un-marqueur-de-completude-doit-lire-si-la-session-a-conclu-2026-09-29.md`
  — même famille : un marqueur qui affirme plus que ce qu'il a lu.
