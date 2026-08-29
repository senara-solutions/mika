# Plan — mika#2047 — Désactiver le workflow Release (release-please)

**Issue:** senara-solutions/mika#2047
**Branch:** `ci/2047/release-main-rouge-release-please-tourne`
**Milestone:** Loop self-repair
**Ticket de reprise:** senara-solutions/mika#2048
**Statut:** prêt à implémenter

## Contexte

`main` est rouge en continu, et le rouge ne vient pas du code : il vient de
`.github/workflows/release-pr.yml`, qui échoue à chaque push sur `main`. Un `main` durablement rouge
apprend à ignorer le rouge — c'est précisément la classe de dégât qu'on ferme partout ailleurs cette
semaine.

Ce chemin a déjà une histoire longue, compoundée dans
`docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` : quatre étapes de dérive
— quatre classes de panne (A à D) traversant quatre outils successifs, jusqu'à la migration
git-cliff → release-please du 2026-05-09 (mika#1049). Ce plan ajoute la première entrée qui ne tente
pas de réparer.

## Preuves

Toutes issues du log de l'étape `Release Please` du run `33245981885` (`2d7dfc74`, 2026-08-29 09:38Z).
Aucune n'est à re-collecter.

### La cause 1 écrite dans le ticket est falsifiée

Le ticket suppose que le `with:` du workflow, qui ne passe que `token`, prive release-please de sa
config manifest faute de `config-file` / `manifest-file`. C'est faux, pour deux raisons indépendantes :

1. Ces deux entrées valent `default: ''` dans l'`action.yml` du SHA épinglé `5c625bfb` ; release-please
   retombe alors sur ses propres constantes, `release-please-config.json` et
   `.release-please-manifest.json` — exactement les fichiers présents à la racine.
2. Le log montre la lecture **directement** :

```
❯ Fetching release-please-config.json from branch main
❯ Fetching .release-please-manifest.json from branch main
✔ Building candidate release pull request for path: .        ← packages { "." }
❯ type: rust                                                  ← "release-type": "rust"
```

(Les deux premières lignes sont la preuve forte ; les deux suivantes montrent que le contenu lu est
bien le nôtre. À noter, un contre-signal apparent : le log affiche plus loin `component: ` vide. Il
n'invalide rien — la branche visée reste `release-please--branches--main--components--mika` — mais
c'est une ligne à ne pas citer comme preuve.)

Ajouter les deux entrées écrirait explicitement ce que l'action fait déjà. Le run échouerait à
l'identique.

### Les commits non parsables sont non fatals

```
❯ commit could not be parsed: 35ccfa86… / error message: Error: unexpected token '(' at 15:22
❯ commit could not be parsed: 6a602807… / cec032d9…
❯ commits: 1190
✔ Considering: 1190 commits
```

Trois commits sautés — le compte exact dans le log — et le run continue, jusqu'à considérer 1190
commits. Bruit de log, pas la cause de l'échec.

### La vraie cause fatale — la stratégie `rust` face à un workspace virtuel

```
⚠ No workspace manifest package name found
✔ found workspace with 1 members, upgrading all
❯ Fetching crates/*/Cargo.toml from branch main
⚠ member crates/* declared but did not find Cargo.toml
✔ updating 0 submodules
❯ versions map: Map(0) {}
…
❯ Fetching Cargo.toml from branch main
✖ is not a package manifest (might be a cargo workspace)
##[error]release-please failed: is not a package manifest (might be a cargo workspace)
```

Deux défauts enchaînés, tous deux dans la stratégie `rust` :

1. **Le glob n'est pas développé.** `members = ["crates/*"]` (`Cargo.toml:2`) est traité comme un
   chemin littéral : release-please cherche le fichier `crates/*/Cargo.toml`. Aucun membre n'est
   résolu → `versions map: Map(0)`.
2. **Le `Cargo.toml` racine est ensuite passé à l'updater de paquet**, qui exige une section
   `[package]`. Notre racine est un workspace virtuel — `[workspace]` + `[workspace.package]`, pas de
   `[package]` — donc exception fatale et sortie non nulle.

### Décision opérateur amont

Aucun consommateur connu de ce canal : dernière étiquette `v0.12.2` du **2026-05-09**, et le
déploiement se fait depuis `main` via `make deploy`.

**Mesure à charge, relevée en relecture, qui ne renverse pas la décision.** « Personne ne consomme »
est faux au sens littéral : les assets portent 38 téléchargements sur `v0.12.2` et 132 toutes releases
confondues, et `install.sh` — proposé en voie d'installation principale dans le README — tire depuis
GitHub Releases. Ce qui reste vrai, et qui est le point qui compte : **ce workflow ne produit plus
rien depuis le 2026-05-09**. 300 runs relevés entre le 2026-06-05 et le 2026-08-29, 300 en échec.
L'éteindre ne retire donc à ces consommateurs résiduels rien qu'ils obtenaient encore ; ils sont figés
sur `v0.12.2` depuis mai, et le resteront — c'est un état antérieur à ce changement, pas un effet de
ce changement. Le fait est reporté dans mika#2048.

Réparer coûterait un changement de stratégie
release-please **plus** une boucle de vérification qui n'existe pas sans salir `main` ou fabriquer de
fausses PR de release. La décision est prise en amont de ce plan et n'est pas rouverte ici :
désactivation propre, reprise fichée en mika#2048.

## Périmètre

**Dans le périmètre :**
- `.github/workflows/release-pr.yml` — désactivation du déclencheur automatique et rédaction de la
  raison dans le fichier.
- `docs/deployment.md` § « Release PR » et « Release Binaries » — le document décrit un workflow
  actif ; il doit décrire l'état réel.
- `crates/mika-agent/docs/deployment.md` — copie miroir embarquée dans le binaire
  (`scripts/sync-agent-docs.sh:13-23`, `crates/mika-agent/build.rs:16-26`). **Le contrôle requis
  `Docs Sync` échoue si elle n'est pas resynchronisée**, et le binaire embarquerait sinon une doc
  décrivant un canal de release encore actif.
- `README.md`, `CONTRIBUTING.md`, `CLAUDE.md` — quatre affirmations y promettent des releases
  automatiques. Extension de périmètre assumée : ce sont les mêmes prétentions que celles corrigées
  dans `deployment.md`, et les laisser reviendrait à remplacer un rouge visible par une doc fausse.
  (Deux d'entre elles étaient déjà obsolètes avant ce changement : `README.md` et `CONTRIBUTING.md`
  promettaient des releases via **release-plz**, outil remplacé le 2026-05-09.)
- `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` — ajout de l'étape 5, à
  l'étape `/ce:compound`.

**Hors périmètre — explicitement :**
- `Cargo.toml`, `release-please-config.json`, `.release-please-manifest.json` — intacts. Ils restent
  la matière première de mika#2048 ; les modifier maintenant, c'est réparer à moitié un chemin qu'on
  éteint.
- `.github/workflows/release.yml` (binaires multi-plateformes, déclenché sur étiquette `v*`) — reste
  en place. Il devient dormant faute d'étiquettes produites, ce qui est documenté, pas corrigé : une
  étiquette posée à la main doit continuer de produire ses binaires.
- Les commits non parsables — non fatals, donc rien à corriger.
- La branche résiduelle `release-please--branches--main--components--mika--release-notes` sur
  `origin` — laissée en place, signalée dans le corps de la PR.
- Tout autre workflow.

## Décisions

### D1 — Retirer le déclencheur `push: main`, garder `workflow_dispatch`

**Décision.** Supprimer le bloc `on.push` et ne conserver que `workflow_dispatch:`.

**Pourquoi.** C'est le déclencheur `push: main` qui produit le rouge : un run par merge. Sans lui, le
workflow ne s'exécute plus jamais tout seul, et `main` ne peut plus être teint par ce chemin — ce qui
est exactement le critère de succès du ticket.

`workflow_dispatch` est conservé délibérément, et c'est un choix, pas un oubli : il reste le point
d'entrée manuel de la reprise, et un dispatch qui échoue est un acte volontaire d'un mainteneur, pas
un rouge subi.

**Ce n'est pas pour autant un bac à sable, et l'en-tête le dit.** L'action est invoquée sans
`target-branch` (`release-pr.yml`, job `release-please`), donc elle cible la branche par défaut quel
que soit le `ref` choisi au dispatch : un dispatch « depuis un ref de test » créerait une vraie PR de
release sur `main` et, si `release-please` réussissait, `update-lockfile` pousserait sur la branche
`release-please--…`. mika#2048 doit ajouter `target-branch` **avant** de s'en servir pour vérifier
quoi que ce soit. Constat de relecture ; l'affirmation initiale de ce plan (« vérifier sans pousser
sur `main` ») était plus forte que la preuve.

**Alternatives rejetées :**
- **`if: false` sur le job.** Explicitement écarté par le ticket : « pas un `if: false` muet ». Même
  commenté, ça laisse un workflow qui se déclenche, s'exécute et affiche des jobs « skipped » à chaque
  push — un bruit permanent qui ressemble à un état transitoire.
- **Supprimer le fichier.** Rend impossible d'écrire la raison dans le fichier, ce que le ticket exige.
  L'historique git n'est pas une surface de lecture pour quelqu'un qui se demande pourquoi il n'y a
  plus de releases.
- **Renommer en `.yml.disabled`.** Sort le fichier de la validation de schéma GitHub Actions et interdit
  `workflow_dispatch`, donc supprime la voie de vérification dont mika#2048 a besoin.

### D2 — La raison vit dans un bloc d'en-tête du fichier, et le nom du workflow porte l'état

**Décision.** Deux surfaces, pas une :

1. Un bloc de commentaires en tête de `release-pr.yml` : ce qui est désactivé, depuis quand, la cause
   technique exacte (les deux lignes de log qui la prouvent), la décision opérateur qui la motive, et
   le renvoi vers mika#2047 (diagnostic) et mika#2048 (reprise).
2. Le champ `name:` devient `Release (disabled — see mika#2048)`.

**Pourquoi la seconde surface.** L'onglet Actions liste les workflows par leur `name:`. Sans ça,
l'état « éteint » n'est lisible qu'en ouvrant le fichier — c'est-à-dire seulement par quelqu'un qui
soupçonne déjà quelque chose. Le coût est nul : aucun contrôle requis ne porte sur ce nom (voir D3).

### D3 — Aucun contrôle requis ne dépend de ce workflow — vérifié

**Vérification faite :** `repos/senara-solutions/mika/rulesets/13380969` (« main protection »,
`enforcement: active`). Les `required_status_checks` sont : `Check`, `Dashboard`, `Docs Site`,
`Docs Sync`, `Pipeline Artifacts`, `Security`. Aucun contexte issu de `release-pr.yml`.

**Conséquence :** ni retirer le déclencheur, ni renommer le workflow ne peut laisser une PR bloquée en
attente d'un contrôle qui ne viendra jamais. C'est la seule façon dont une désactivation peut se
retourner en panne, et elle est écartée par mesure, pas par raisonnement.

### D4 — Le job `update-lockfile` ne devient pas orphelin — aucune modification

**Décision.** Ne pas toucher au job `update-lockfile`.

**Pourquoi.** Il porte `needs: release-please`. Deux cas, tous deux sains :
- **Pas de déclencheur** — le workflow entier ne démarre pas ; aucun job n'existe, donc aucun orphelin.
- **Dispatch manuel** — `release-please` échoue, et la sémantique `needs:` marque `update-lockfile`
  comme *skipped*, jamais *failed*. Il ne peut pas s'exécuter sur une branche `release-please--…` qui
  n'a pas été créée.

Le supprimer serait détruire du contexte dont mika#2048 a besoin : ce job encode le fait que
release-please ne régénère pas `Cargo.lock`, et son `ref:` en dur documente le format de branche dérivé
de `component`.

### D5 — Corriger `docs/deployment.md`, qui décrit aujourd'hui un workflow actif

**Décision.** Réécrire la section « Release PR (`release-pr.yml`) » pour dire l'état réel et renvoyer à
mika#2048, et ajouter une phrase à la section « Release Binaries » indiquant que `release.yml` est
dormant faute d'étiquettes produites automatiquement.

**Pourquoi.** `docs/deployment.md:113-119` affirme « Runs on push to `main` » et décrit un cycle de
release vivant. Laisser cette prose en place, c'est remplacer un rouge visible par une doc fausse —
le même dégât, en moins détectable.

## Definition of Done

- [ ] `.github/workflows/release-pr.yml` ne se déclenche plus sur `push: main` ; seul
      `workflow_dispatch` subsiste.
- [ ] Le fichier porte, en en-tête, la raison de la désactivation, la cause technique avec ses preuves,
      et les renvois vers mika#2047 et mika#2048.
- [ ] Le `name:` du workflow signale l'état désactivé.
- [ ] `docs/deployment.md` décrit l'état réel du chemin release, et `crates/mika-agent/docs/deployment.md`
      est resynchronisé (contrôle requis `Docs Sync`).
- [ ] `README.md`, `CONTRIBUTING.md` et `CLAUDE.md` ne promettent plus de releases automatiques.
- [ ] Aucun fichier hors des deux listes du périmètre n'est modifié — en particulier ni `Cargo.toml`, ni
      `release-please-config.json`, ni `.release-please-manifest.json`, ni un autre workflow.
- [ ] Le YAML est valide (parse sans erreur) et `on:` ne contient plus que `workflow_dispatch`.
- [ ] `docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md` gagne son étape 5.
- [ ] La PR référence `Closes #2047` et mentionne mika#2048.

## Acceptance criteria

Transcrits verbatim de la section « Critères d'acceptation » de senara-solutions/mika#2047.

- [ ] `main` n'est plus rouge du fait de ce workflow.
- [ ] Si les releases sont conservées : un run réussi produit (ou met à jour) la PR de release, avec
      la version du workspace correctement propagée.
- [ ] Si elles sont abandonnées : le workflow est désactivé avec la raison écrite dans le fichier, et
      la décision est tracée dans le ticket — pas un `if: false` muet.
- [ ] Les commits non parsables sont soit tolérés silencieusement, soit corrigés en amont par une
      convention de message documentée. Test anti-vacuité : un commit à parenthèses ne doit pas faire
      échouer le run, et un commit conventionnel valide doit toujours produire son entrée de
      changelog.

**Lecture de ces critères sous la branche « abandonnées », qui est celle retenue :**

- Critère 1 — satisfait par D1. Vérification : après merge, le prochain push sur `main` ne produit
  aucun run du workflow `Release`.
- Critère 2 — **sans objet** ; c'est la branche « conservées », que la décision opérateur n'a pas
  retenue. Son contenu est reporté dans mika#2048 comme critère de succès de la reprise.
- Critère 3 — satisfait par D1 + D2 (raison dans le fichier, pas d'`if: false`) et par le commentaire
  de vérification déjà posté sur mika#2047 (décision tracée dans le ticket).
- Critère 4 — satisfait par **tolérance silencieuse**, et c'est une constatation, pas un changement.
  Le test anti-vacuité a deux moitiés, et il faut être exact sur laquelle est prouvée :
  - « un commit à parenthèses ne doit pas faire échouer le run » → **prouvé** par le run
    `33245981885` : les trois commits concernés sont sautés et le run continue jusqu'à
    `Considering: 1190 commits`.
  - « un commit conventionnel valide doit toujours produire son entrée de changelog » → **non
    observable, et il ne le sera pas.** Le run est mort sur `Fetching Cargo.toml` avant toute
    génération de changelog ; aucune entrée n'a été produite, donc rien n'atteste cette moitié. Et
    sous la branche « abandonnées » elle devient sans objet : plus aucun changelog n'est généré.
    Elle est reportée dans mika#2048, où elle redevient vérifiable.

  Aucune convention de message n'est introduite : elle contraindrait 1190 commits d'historique pour
  un chemin qu'on éteint.

## Vérification

| # | Contrôle | Commande / preuve | Attendu |
|---|----------|-------------------|---------|
| 1 | YAML valide | `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release-pr.yml'))"` | pas d'erreur |
| 2 | Plus de déclencheur `push` | `yaml.safe_load(...)['on']` (ou `[True]`, YAML 1.1 interprète `on` en booléen) | `{'workflow_dispatch': None}` seul |
| 3 | Raison présente | `grep -c 'mika#2048' .github/workflows/release-pr.yml` | ≥ 1 |
| 4 | Pas d'`if: false` | `grep -n 'if: false' .github/workflows/release-pr.yml` | aucune correspondance |
| 5 | Périmètre respecté | `git diff --name-only origin/main...HEAD` — **pas** `main`, qui peut être périmé en local | uniquement les fichiers des deux listes du périmètre |
| 6 | Contrôles requis intacts | ruleset 13380969 (relevé en D3) | aucun contexte `release-pr` |
| 7 | Copie miroir synchronisée | `bash scripts/sync-agent-docs.sh && git diff --exit-code crates/mika-agent/docs/` | pas de diff (contrôle requis `Docs Sync`) |
| 8 | Plus de prétention fausse | `grep -rn "automated release\|release-plz" README.md CONTRIBUTING.md CLAUDE.md` | aucune affirmation de release automatique active |
| 9 | Critère de succès final | après merge, `gh run list --workflow=release-pr.yml` | aucun run nouveau sur `main` |

Le contrôle 9 ne peut se faire qu'après merge — c'est la nature du critère. Il appartient à la
vérification post-merge, pas au pipeline de cette PR.
