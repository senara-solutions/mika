# mika#2001 — Les commandes de groom ne sont pas perdues : elles sont hors du périmètre de ce dépôt

**Ticket :** mika issue#2001
**Type :** fix (périmètre + consignation d'une décision)
**Date :** 2026-09-21

---

## Problème

Le ticket relaie un finding de mika-qa posé à la revue de PR#1994 :

> `AC3 ❌ — .claude/commands/ groom files removed from repo, Units 3-4 not implementable`

et demande de « déterminer si ces fichiers ont été déplacés, supprimés
volontairement, ou perdus ; puis soit rétablir le chemin attendu, soit amender le
ticket d'origine ».

**La réponse est « supprimés volontairement », et l'enquête déplace le diagnostic
de trois crans : le lien causal du ticket est faux, l'option « rétablir » est
structurellement nocive, et la substance des Units 3-4 a déjà été livrée il y a
deux jours par un autre ticket.**

### M1 — PR#1994 n'a supprimé aucun fichier `.claude/`

```
$ git show --stat 8f46caa9
 crates/mika-agent/src/calibration/roles/mika_arch.rs               | 226 +-
 .../calibration_fixtures/mika-arch/fire_disposition_gate.md        |  53 +
 .../calibration_fixtures/mika-arch/manifest.yaml                   |  14 +
 crates/mika-agent/tests/eval/skills/mika_arch_fire_disposition_gate.rs | 310 +
 crates/mika-agent/tests/eval/skills/mod.rs                         |   1 +
 docs/plans/2026-06-28-005-...-fire-disposition-plan.md             | 313 +
 docs/solutions/best-practices/fire-disposition-doctrine.md         |  49 +
 skills/bundled/mika-arch-groom-ticket/system_prompt.md             |  19 +
 skills/bundled/mika-arch-second-review/system_prompt.md            |  13 +
 9 files changed, 995 insertions(+), 3 deletions(-)
```

Neuf fichiers, **zéro** sous `.claude/`, **zéro suppression**. L'imputation du
ticket — « PR#1994 a été mergée […] mais les fichiers `.claude/commands/` de
grooming ne sont plus dans le repo » — juxtapose deux faits vrais et en infère un
lien qui n'existe pas.

### M2 — La suppression est délibérée, motivée par écrit, et antérieure de trois mois

```
$ git log --oneline --all -- .claude/commands/
…
b831cbd5 fix(commands): remove 18 out-of-scope mika-platform commands from mika repo
```

Le corps du commit (Vincent, **2026-05-26**, soit trois mois avant PR#1994)
énonce le motif sans ambiguïté :

> The previous commit accidentally staged command files that belong in
> mika-platform's `.claude/commands/`, not mika's. **Only 4 commands belong in
> mika**: mika-doc-audit.md, mika-issue.md, mika-issues.md, mika.md.

Les 18 fichiers retirés incluent exactement `mika-groom-ticket.md`,
`mika-groom-plan-only.md`, `mika-groom-milestone.md` et `mika-revise-plan.md`.
Un second commit de la même famille — `5045762a chore(commands): delete
propagated mika-groom-milestone.md copy (#1095)` — confirme que la propagation
d'une copie dans ce dépôt est traitée comme un défaut à corriger, pas comme
l'état attendu.

Cette organisation est par ailleurs **documentée** : `mika/CLAUDE.md` § Directory
Structure ne déclare que les quatre commandes, nommément.

### M3 — Pourquoi mika-qa a lu « removed », et pourquoi c'était de bonne foi

Le finding n'est pas une erreur d'inattention : c'est la lecture naturelle d'un
mécanisme conçu pour être invisible. `_seed_worktree_slash_commands()`
(mika#1415, `dispatch-lib.sh:1734`) copie les commandes du meta-repo dans chaque
worktree de dispatch **et les masque de `git status`** via l'`info/exclude` du
common-dir. Mesure dans ce worktree même :

```
$ ls .claude/commands/ | wc -l          # 27 fichiers présents sur disque
$ git ls-files .claude/commands/ | wc -l # 4 fichiers suivis
```

Les 23 autres — dont `mika-groom-ticket.md`, que la session de revue voyait
fonctionner — sont listés dans `$(git rev-parse --git-common-dir)/info/exclude`.
**Un observateur qui demande à git voit une absence là où le pipeline voit un
fichier qui marche.** La classe est réelle et coûteuse : mika#2306 a dû
re-mesurer exactement le même `git ls-files` le 19/09 pour établir le même fait.

### M4 — Units 3-4 visent des fichiers que ce dépôt ne peut pas éditer

Le plan de mika#1574 nomme ses cibles :

| Unit | Fichier visé | Suivi par `senara-solutions/mika` ? |
|---|---|---|
| Unit 3 | `.claude/commands/mika-groom-plan-only.md` | **non** |
| Unit 4 | `.claude/commands/mika-groom-ticket.md` | **non** |

Ce n'est donc pas une régression introduite par PR#1994 : c'est une **erreur de
périmètre du plan mika#1574 lui-même**, qui a assigné à un ticket `mika` deux
unités portant sur des fichiers `mika-platform`. La PR a livré les sept unités
qui relevaient de son dépôt (Units 1–2, 5–9, toutes visibles dans le `--stat`
ci-dessus) et ne pouvait structurellement pas livrer les deux autres.

### M5 — La substance des Units 3-4 est déjà livrée, par mika#2306

Units 3-4 sont de la **guidance**, le plan #1574 le dit lui-même :

> This is guidance to the pilot generating the plan, not structural enforcement —
> the structural enforcement is the architect gate (Unit 1/2).

Or cette guidance a été livrée le **2026-09-19** par mika#2306 (commit
`29e588bb`), par le seul canal que ce dépôt contrôle : la constante
`_FIRE_DISPOSITION_RULE` (`dispatch-lib.sh:2046`), injectée dans le `PROMPT` de
chaque dispatch de groom (`dispatch-lib.sh:2653`). Le commentaire du site
d'injection énonce le même constat que M2–M4, indépendamment :

> les commandes […] vivent dans le meta-repo et sont copiées dans chaque worktree
> par `_seed_worktree_slash_commands` (mika#1415), donc **un ticket ouvert sur
> `senara-solutions/mika` ne peut pas les éditer**. Ce PROMPT est le seul canal
> que ce dépôt contrôle. **La moitié commandes est nommée en suivi, pas simulée.**

Et mika#2306 a **déjà ouvert le suivi `mika-platform`** correspondant (§ Suivi de
son plan, qui décrit l'étape `5c` à ajouter aux trois commandes de groom).

**Conséquence directe pour ce ticket : il ne reste ni code ni prompt à écrire ici,
et il ne faut surtout pas ouvrir un second suivi `mika-platform`.**

---

## Requirements

- **R1** — Les artefacts de **ce dépôt** cessent de réclamer un travail
  structurellement impossible ici. Concrètement : le plan mika#1574, qui vit dans
  `docs/plans/` et porte encore Units 3-4 comme unités à implémenter.
- **R2** — La classe de confusion de M3 (« semé + exclu se lit comme supprimé »)
  est consignée là où un lecteur la cherchera, de sorte que la prochaine revue
  n'ait pas à rejouer l'enquête. Troisième occurrence du coût : mika-qa (08/26),
  mika#2306 (09/19), ce ticket (09/21).
- **R3** — Aucune duplication du suivi `mika-platform` déjà ouvert par mika#2306.
- **R4** — Aucune régression sur les Units 1–2 et 5–9, livrées et vertes. En
  particulier la **numérotation des unités du plan #1574 est préservée** :
  `mika_arch_fire_disposition_gate.rs:30` cite ce plan « § Units 5-7 ».

---

## Décisions

### D1 — « Rétablir le chemin attendu » est refusé sur preuve, pas sur préférence

Le ticket offre deux branches. La première casserait le pipeline, et le code le
dit à l'avance — commentaire de `_seed_worktree_slash_commands`
(`dispatch-lib.sh:1754`) :

> Invariant 1: the worktree's own tracked command wins — skip the copy. […] If a
> sub-repo ever tracks a meta-ONLY orchestration command name (e.g.
> `mika-groom-ticket.md`) **this would silently shadow it** (a mika#1173 risk) —
> revisit with an explicit must-seed set if that case ever arises.

Committer `mika-groom-ticket.md` dans `mika` ferait donc gagner la copie du
sous-repo sur celle du meta-repo, **silencieusement**, pour tous les dispatches
sur ce dépôt. On obtiendrait deux sources de vérité divergentes pour le
prescripteur de grooming, et la divergence ne produirait aucun signal — la forme
de panne la plus chère de cette maison. La branche « amender » est donc prise, et
c'est celle que le ticket lui-même cite en second.

### D2 — L'amendement porte sur le plan (dépôt), l'issue est une action opérateur

Le ticket dit « amender le ticket d'origine ». Un `gh issue edit` est une action
opérateur hors du warrant d'une session dispatchée — précédent écrit dans le plan
#1574 lui-même (rev 21) : *« a mechanical body rewrite is an operator/orchestrator
action outside this content-only revise session's warrant »*. Ce plan amende donc
l'artefact **versionné** (le plan, qui porte une `## Revision history` faite pour
ça et déjà utilisée 22 fois) et **nomme** l'action GitHub sans la simuler.

### D3 — Une section dans la doc existante, jamais un nouveau fichier

`docs/solutions/architecture-patterns/seed-scaffold-into-tracked-worktree-dir-via-git-exclude.md`
(2026-06-06) documente déjà le mécanisme. Ce qu'il ne documente pas, c'est son
**effet de lecture** : ce que le mécanisme fait croire à un observateur qui
interroge git. Créer un second fichier scinderait la question en deux endroits ;
la section va donc dans le fichier existant, qui est exactement là où un lecteur
enquêtant sur `.claude/commands/` arrivera.

### D4 — Aucun détecteur n'est livré, et il existe déjà

Le réflexe serait d'ajouter un test figeant « les 4 commandes suivies sont
exactement ces 4 ». **Il existe** :
`skills/bundled/_shared/tests/test_seed_worktree_slash_commands.sh:91` assert
littéralement `mika-groom-ticket.md` comme cas *meta-only seeded*, et vérifie
(l. 97, 104) qu'il atterrit dans `info/exclude` exactement une fois. Il tourne en
CI (`make test-dispatch-lib`, `.github/workflows/ci.yml:85`). Le détecteur de
cette classe est déjà vert ; en ajouter un second serait de la redondance sans
couverture nouvelle. Voir § Fire-Disposition.

---

## Scope Boundaries

**Dans le périmètre :** l'amendement du plan mika#1574 dans ce dépôt ; la
consignation de la classe de confusion ; une ligne de repérage dans `CLAUDE.md`.

**Hors périmètre, nommé :**

- **L'étape `5c` dans les trois commandes de groom** — c'est le suivi
  `mika-platform` **déjà ouvert par mika#2306**. Ce ticket le référence ; il ne le
  double pas et ne le ré-instruit pas.
- **`gh issue edit` sur mika#1574** et **la fermeture de mika#2001** — actions
  opérateur (D2), nommées dans la DoD.
- **Toute modification de `dispatch-lib.sh`** — `_FIRE_DISPOSITION_RULE` et son
  site d'injection sont livrés et fonctionnels ; y toucher rouvrirait mika#2306.
- **Un « must-seed set » explicite** dans `_seed_worktree_slash_commands` — le
  commentaire du code le réserve au jour où le cas se présente (« if that case
  ever arises »). Ce plan établit que le cas **ne doit pas** se présenter (D1) ;
  construire la parade d'un scénario qu'on vient de refuser serait prématuré.

---

## Implementation Units

| Unit | Fichier | Changement |
|---|---|---|
| **U1** | `docs/plans/2026-06-28-005-feat-dev-groom-doctrine-add-fire-disposition-plan.md` | Marquer Units 3 et 4 **hors périmètre `senara-solutions/mika`** — sans les renuméroter ni les supprimer (R4). Sous chacune : une ligne `**Hors périmètre (mika#2001) :**` nommant le dépôt propriétaire (`mika-platform`), la cause (b831cbd5), et le fait que la substance est livrée par mika#2306 via `_FIRE_DISPOSITION_RULE`. Ajouter une entrée `rev 23 (2026-09-21)` en tête de `## Revision history` résumant M1–M5 en trois phrases. Ajuster la ligne de `## Key Technical Decisions` qui énumère « `/ce:plan` guidance (Units 3–4) » pour qu'elle porte la même mention. |
| **U2** | `docs/solutions/architecture-patterns/seed-scaffold-into-tracked-worktree-dir-via-git-exclude.md` | Ajouter une section **« Effet de lecture : un fichier semé se lit comme un fichier supprimé »**. Contenu : les deux mesures de M3 (`ls` vs `git ls-files`), la raison (le masquage `info/exclude` est délibéré et sert la propreté du rebase), la règle de lecture (*l'absence dans `git ls-files` d'un fichier `.claude/commands/` présent sur disque est l'état **nominal** d'un worktree de dispatch, jamais la preuve d'une suppression*), et le geste qui tranche en une commande : `git log --oneline --all -- .claude/commands/`. Citer les trois occurrences du coût (mika-qa sur PR#1994, mika#2306, mika#2001). Ajouter `mika-2001` et `scope-boundary` aux `tags` du frontmatter. |
| **U3** | `CLAUDE.md` | § Directory Structure, entrée `.claude/commands/` : après l'énumération des quatre commandes, ajouter une demi-phrase — **les commandes d'orchestration (`/mika-groom-ticket`, `/mika-groom-plan-only`, `/mika-groom-milestone`, `/mika-revise-plan`) vivent dans `mika-platform` et sont semées par `_seed_worktree_slash_commands` (mika#1415) ; un ticket ouvert ici ne peut pas les éditer.** C'est la ligne qui aurait évité les trois occurrences de M3, placée sur la surface que toute session lit au démarrage. |

**Ordre :** non contraint. Les trois unités sont additives et purement
documentaires ; aucune ne refuse quoi que ce soit, aucune n'est lue par un test
(vérifié : la seule référence au plan #1574 hors `docs/plans/` est un commentaire
`//!` dans `mika_arch_fire_disposition_gate.rs:30`, qui cite « § Units 5-7 » —
d'où la préservation de numérotation exigée en R4).

---

## Verification Contract

| # | Vérification | Ce qu'elle établit |
|---|---|---|
| T1 | `grep -n "Hors périmètre (mika#2001)" docs/plans/2026-06-28-005-*.md` rend **deux** lignes | U1 — les deux unités sont marquées, pas une seule |
| T2 | `grep -cE "^### Unit [1-9]:" docs/plans/2026-06-28-005-*.md` rend **9**, et `### Unit 5:`/`### Unit 7:` existent toujours | R4 — la numérotation citée par `mika_arch_fire_disposition_gate.rs:30` est intacte |
| T3 | `grep -n "rev 23 (2026-09-21)" docs/plans/2026-06-28-005-*.md` rend une ligne, en tête de la `## Revision history` | U1 — l'amendement est daté et traçable |
| T4 | `make test-dispatch-lib` passe | R4 — contrôle de non-régression : le test qui atteste `mika-groom-ticket.md` comme meta-only reste vert (D4) |
| T5 | `git ls-files .claude/commands/ \| wc -l` rend **4** à la fin du travail | D1 — **contrôle négatif** : aucune des trois unités n'a rétabli un fichier, c'est-à-dire que la branche refusée l'est restée |
| T6 | `grep -n "mika-platform" CLAUDE.md \| grep -n "commands"` rend la ligne U3 | U3 — la ligne de repérage est posée |
| T7 | `grep -c "_FIRE_DISPOSITION_RULE" skills/bundled/_shared/dispatch-lib.sh` rend **2** (le site de définition, l. 2046 ; le site d'injection, l. 2653) | Scope Boundaries — mika#2306 n'a pas été rouvert. Le prédicat porte sur le **nom de la constante**, pas sur la chaîne `Fire-Disposition` : celle-ci apparaît 18 fois dans le fichier, dont l'essentiel à l'intérieur du corps de la constante, donc un compte sur elle mesurerait la rédaction du message et non l'intégrité du câblage |

T5 est le test qui compte le plus : c'est le seul qui puisse rougir si un futur
éditeur, lisant ce ticket trop vite, « répare » en committant les fichiers.

---

## Definition of Done

- U1, U2, U3 livrées ; T1–T7 vertes.
- Le corps de PR nomme explicitement les deux faits qui referment le finding
  mika-qa : PR#1994 n'a rien supprimé (M1), et la suppression est b831cbd5 du
  26/05 avec son motif (M2).
- Le corps de PR **référence** le suivi `mika-platform` de mika#2306 et déclare
  qu'aucun nouveau suivi n'est ouvert (R3).
- **Actions opérateur nommées, non exécutées par la session** (D2) : (a) amender
  le corps de mika#1574 pour y refléter le périmètre réel d'Units 3-4 ; (b) fermer
  mika#2001 en citant cette PR. Elles sont listées dans le corps de PR comme
  restant à faire.

---

## Acceptance criteria

*(dérivés — le ticket mika#2001 ne porte pas de section `## Acceptance criteria`.)*

- **AC1** — La question posée par le ticket (« déplacés, supprimés volontairement,
  ou perdus ? ») reçoit une réponse **étayée par des références vérifiables** :
  `supprimés volontairement`, commit b831cbd5 du 2026-05-26, motif cité, et les
  fichiers vivent dans `mika-platform`.
- **AC2** — Le finding `AC3 ❌` de mika-qa est réfuté sur son lien causal : PR#1994
  (8f46caa9) ne modifie aucun fichier sous `.claude/`, et la suppression lui est
  antérieure de trois mois.
- **AC3** — L'option « rétablir le chemin attendu » est **explicitement refusée**
  avec sa raison technique (shadowing silencieux du seeding, mika#1415 / mika#1173),
  et le dépôt suit toujours exactement 4 commandes après le travail (T5).
- **AC4** — Les Units 3-4 du plan mika#1574 sont marquées hors périmètre avec leur
  dépôt propriétaire, sans renumérotation des neuf unités (T1, T2).
- **AC5** — La classe de confusion « fichier semé + exclu ⇒ lu comme supprimé » est
  consignée dans `docs/solutions/`, avec le geste d'une ligne qui la tranche (U2).
- **AC6** — Aucun second ticket de suivi `mika-platform` n'est ouvert ; celui de
  mika#2306 est référencé (R3).
- **AC7** — Aucune régression : `make test-dispatch-lib` vert, `dispatch-lib.sh`
  inchangé (T4, T7).

---

## Fire-Disposition

**N/A — aucun livrable de ce plan n'est un détecteur.** Les trois unités sont
documentaires : un amendement de plan, une section de doc de solution, une
demi-phrase dans `CLAUDE.md`. Aucune n'a pour fonction primaire de signaler une
violation, aucune ne peut donc « firer » sur des données existantes.

Cette absence est un **choix argumenté, pas une omission** (D4). Le détecteur de
cette classe existe déjà et il est vert :
`skills/bundled/_shared/tests/test_seed_worktree_slash_commands.sh` assert
`mika-groom-ticket.md` comme fichier meta-only correctement semé et exclu, et
tourne à chaque PR via `make test-dispatch-lib` (`ci.yml:85`). En ajouter un
second — par exemple un scan figeant la liste des quatre commandes suivies —
n'apporterait aucune couverture nouvelle et créerait un second site à maintenir
pour un invariant déjà tenu.

Le **contrôle négatif T5** (`git ls-files .claude/commands/ | wc -l` = 4) joue le
rôle de garde-fou de ce ticket précis, sans être un détecteur livré : c'est une
vérification de non-régression exécutée à la revue, qui rougirait si le travail
avait pris la branche que D1 refuse.

Précédent de forme : le plan mika#1574 lui-même porte `## Fire-Disposition` =
N/A pour la même raison (« this ticket adds a review gate and documentation »).

---

## Suivi (hors périmètre, nommé)

- **`mika-platform` — l'étape `5c` dans les trois commandes de groom.** Suivi
  **déjà ouvert par mika#2306** (§ Suivi de son plan : ajouter la prescription
  `## Fire-Disposition` à `/mika-groom-plan-only`, `/mika-groom-ticket` et
  `/mika-revise-plan`). Ce ticket le référence et **n'en ouvre pas un second**.
  C'est aussi là que le contenu des Units 3-4 de mika#1574 atterrira, au dépôt
  qui possède les fichiers.
- **Fermeture de mika#1574.** Une fois son corps amendé (action opérateur, D2),
  ses neuf unités sont soit livrées (1–2, 5–9), soit réassignées au suivi
  ci-dessus (3–4). Rien n'y reste ouvert dans ce dépôt.

---

## Références

- `b831cbd5` — `fix(commands): remove 18 out-of-scope mika-platform commands from mika repo` (2026-05-26) : la suppression et son motif.
- `8f46caa9` — PR#1994, mika#1574 : les neuf fichiers livrés, aucun sous `.claude/`.
- `29e588bb` — mika#2306 : `_FIRE_DISPOSITION_RULE`, la substance d'Units 3-4 par le canal que ce dépôt contrôle.
- `5045762a` — `#1095` : précédent de suppression d'une copie propagée.
- mika#1415 — `_seed_worktree_slash_commands` : pourquoi les commandes ne sont pas dans ce dépôt.
- mika#1173 — le risque de shadowing qu'un rétablissement réaliserait.
- `docs/solutions/architecture-patterns/seed-scaffold-into-tracked-worktree-dir-via-git-exclude.md` — le mécanisme, à compléter par U2.

## Revision history

- rev 1 (2026-09-21) : plan initial. L'enquête demandée par le ticket est conduite
  et rendue dans le § Problème (M1–M5) ; elle infirme le lien causal du ticket
  (PR#1994 est innocente), refuse la branche « rétablir » sur preuve de code (D1),
  et constate que la substance des Units 3-4 est déjà livrée par mika#2306 — ce
  qui réduit le travail restant dans ce dépôt à trois unités documentaires.
