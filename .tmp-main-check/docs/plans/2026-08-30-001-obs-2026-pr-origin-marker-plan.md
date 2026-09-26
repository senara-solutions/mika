# obs(loop) — l'origine d'une PR devient un fait posé par son producteur (mika#2026)

> Ticket : senara-solutions/mika#2026 — `p1-important`, milestone « Substrat de boucle »
> Branche : `obs/2026/loop-l-origine-d-une-pr-loop-la-main-n`

## Problème

La lecture de cadence demande « mergées **et origine** ». La moitié « mergées » est exacte ;
l'origine n'a aucun instrument. Toute répartition loop/à-la-main produite aujourd'hui serait
une estimation habillée en mesure.

## Cause nommée (mesurée le 2026-08-30)

La seule trace d'origine existante — `tasks.metadata.$.claude_pilot.pr_url` — est portée par
un **canal de texte à quatre maillons** :

```
dispatch-lib découvre la PR  →  ligne « PR: <url> » dans RESULT  →  callback traverse
mika-dev + task-engine  →  regex ^PR:\s+ (dispatcher.rs:2272)  →  écriture DB
```

`extract_callback_fields` (`crates/mika-agent/src/task_engine/dispatcher.rs:2262`) n'écrit
`pr_url` que si un callback bien formé atteint le moteur. Mesures sur `~/.mika/data/mika.db`
le 2026-08-30 :

| Mesure | Valeur |
|---|---|
| Lignes `tasks` portant un `claude_pilot.pr_url`, **tous dépôts, depuis toujours** | **43** |
| PR mergées le 2026-08-27 sur `mika` | 5 (#2014, #2015, #2016, #2017, #2018) |
| Parmi elles, enregistrées en base | **0** |
| Tâches `ready-label` du 26–27/08 portant un `pr_url` | 2 (#2019, #2021) — et **les deux tâches sont `failed`** |

Deux faits en découlent :

1. Le compteur mesure **les callbacks bien formés qui ont atteint le moteur**, pas les PR
   produites par la boucle. C'est un instrument qui mesure sa propre plomberie.
2. Le succès du canal n'est même pas corrélé au succès de la tâche : les deux seules lignes
   de la fenêtre viennent de tâches en `failed`.

**Voie 1 écartée, avec sa raison.** Fiabiliser `pr_url` reviendrait à durcir les quatre
maillons d'un canal dont aucun n'a la PR pour objet — et le marqueur resterait dans une base
qui, perdue, emporte la mesure. Le fait « cette PR vient de la boucle » doit vivre **sur la
PR**, pas dans un enregistrement latéral.

## Décision de conception

**Voie 2 : marqueur posé par le producteur, sur l'artefact, au moment de la production.**

Un label `origin:loop` est apposé par `dispatch-lib.sh` — en shell, structurellement, jamais
par le prompt du pilote (cf. `feedback_prompt_enforcement_fragile` : le prompt échoue au
substrat de boucle). Trois disciplines gouvernent le reste :

1. **Aucune reconstitution a posteriori.** Ni nom de branche, ni auteur, ni fenêtre
   temporelle. Ces trois inférences se trompent exactement le jour où la réponse compte —
   l'orchestrateur utilise le même `derive-branch-name` que la boucle, et l'auteur GitHub est
   `samidarko` des deux côtés.
2. **Un marqueur absent se lit « inconnu ».** Jamais « à la main » par défaut. Une valeur par
   défaut qui ressemble à une réponse est la façon dont un instrument ment.
3. **Coupure nette et datée.** L'absence de `origin:loop` n'est informative qu'après la date
   où le marqueur est réellement déployé. Avant : `inconnu`.

### Où vient la date de coupure (« epoch »)

Elle n'est **pas écrite à la main** — une constante devinée au moment d'écrire le code serait
fausse dès que le déploiement traîne. Elle n'est **pas non plus sondée** : la première version
lisait le `mtime` du `dispatch-lib.sh` installé, ce qui est plausible et faux.
`seed_support_dirs` réécrit ce fichier à chaque démarrage du daemon (`std::fs::write`, sans
garde de hash), donc le `mtime` traque le dernier redémarrage, pas la mise en service. Sur un
hôte qui relance plusieurs fois par jour, la coupure aurait avancé en continu et rouvert la
fenêtre aveugle après chaque relance — un mensonge à la source d'apparence honnête.

**Le producteur l'inscrit lui-même**, une seule fois, au premier marquage réussi
(`_record_pr_origin_epoch` → `~/.mika/state/pr-origin-epoch`). Résolution :

1. `MIKA_PR_ORIGIN_EPOCH` (surcharge explicite de l'opérateur, ISO-8601) ;
2. le fichier écrit par le producteur ;
3. sinon : **indéterminée**.

Le sens de l'erreur est le bon : une PR portant `origin:loop` compte loop quelle que soit sa
date ; l'epoch ne gouverne que ce que veut dire le **silence**. Et le silence est jugé sur la
date d'**ouverture** de la PR, pas de merge : une PR loop ouverte avant la mise en service et
mergée après n'a jamais été en position d'être marquée, donc elle se lit « inconnue ». La
déclarer « non-boucle » serait une réponse fausse et assurée sur exactement les PR à cheval
sur la bascule.

Epoch indéterminée ⇒ **tout ce qui n'est pas marqué est `inconnu`**, avec la marche à suivre
imprimée. Le rapport ne devine jamais.

## Ce qui est construit

### A. Producteur — `skills/bundled/_shared/dispatch-lib.sh`

`_stamp_pr_origin <repo> <pr_ref> [origin=loop]` :

- no-op rc 0 si `repo` ou `pr_ref` est vide ;
- `gh pr edit <pr_ref> --repo senara-solutions/<repo> --add-label origin:<origin>` ;
- si l'appel échoue (label inexistant sur ce dépôt — `dispatch-lib` dispatche aussi
  `mika-cloud`, `mika-skills`, `claude-pilot-py`, qui n'ont pas le label-sync de `mika`) :
  `gh label create` idempotent, puis **un** retry ;
- échec final : message nommé sur stderr (`pr_origin.stamp_failed: …`), rc 1 ;
- **fail-open chez l'appelant** : les trois sites appellent avec `|| true`. Un marqueur
  manquant coûte une ligne `inconnu` dans un rapport ; il ne doit jamais coûter un dispatch.

Trois sites d'appel — les trois points, et les seuls, où `dispatch-lib` tient une PR qu'il
vient de produire (audit `grep` de tous les `gh pr list --head` / `gh pr create` du fichier) :

| Site | Ligne (avant patch) | Chemin |
|---|---|---|
| Découverte sur récupération de crash | ~721 | `_PR_URL` |
| Découverte normale post-session | ~2048 | `PR_URL` |
| Création par le sauvetage mika#1396 | ~4199 | `RESCUED_PR_URL` |

Limite assumée et nommée : les sites 1 et 2 découvrent par `gh pr list --head "$BRANCH"`. Si
une PR pré-existante sur cette branche est retrouvée, elle est marquée `origin:loop`. Ce n'est
pas un faux positif : `dispatch-lib` a lancé un pilote sur cette branche, la boucle a porté ce
travail. Le producteur parle de ce qu'il a produit.

### B. Vocabulaire — `.github/labels.yml`

Section `── Origin ──` : `origin:loop`, `origin:spawn`, `origin:manual`.

Honnêteté sur la couverture : **seul `origin:loop` a un producteur structurel dans ce dépôt.**
`origin:spawn` et `origin:manual` sont du vocabulaire que le rapport sait lire et que
l'orchestrateur ou un spawn peuvent poser à l'ouverture ; leur pose automatique vit hors du
dépôt `mika` (skill `/mika-spawn`, meta-repo) et n'est pas promise ici. Cela suffit à la
question posée : après l'epoch, `origin:loop` présent ⇒ loop, absent ⇒ non-loop.

### C. Lecteur — `scripts/pr-origin-report.sh`

```
scripts/pr-origin-report.sh [--repo mika] [--since <ISO|YYYY-MM-DD>] [--until <ISO|YYYY-MM-DD>] [--limit N]
```

Une seule commande rend la lecture. Classement d'une PR mergée dans la fenêtre, dans cet
ordre :

| Condition | Catégorie |
|---|---|
| auteur `app/dependabot` | `dependabot` |
| porte `origin:loop` | `loop` |
| porte `origin:spawn` | `spawn` |
| porte `origin:manual` | `manual` |
| non marquée **et** `mergedAt ≥ epoch` | `non-loop (non marqué)` |
| non marquée **et** (`mergedAt < epoch` ou epoch indéterminée) | `inconnu` |

`dependabot` n'est pas une heuristique : l'auteur *est* le producteur. C'est le seul cas où
l'auteur est un fait d'origine et non une devinette.

Sortie : la fenêtre, l'epoch **et sa provenance**, le tableau des comptes, la liste des PR par
catégorie, le total. Quand l'epoch est indéterminée, une bannière le dit et donne la commande.

### D. Tests

- `skills/bundled/_shared/tests/test_stamp_pr_origin.sh` — `gh` stubé sur `PATH` : pose
  nominale, label absent → `label create` + retry, échec total (rc 1, message, pas de crash),
  arguments vides (no-op rc 0), idempotence.
- `scripts/test-pr-origin-report.sh` — `gh` stubé renvoyant un jeu fixe contenant **les cinq
  PR du 2026-08-27 (#2014–#2018, non marquées)**, une PR marquée `origin:loop`, une
  dependabot, une non marquée postérieure à l'epoch. Assertions :
  - **cas du 27/08 (AC4)** : les cinq tombent en `inconnu`, et **jamais** en `manual` ;
  - epoch indéterminée : tout non marqué en `inconnu` + bannière présente ;
  - post-epoch non marqué : `non-loop (non marqué)` ;
  - `origin:loop` compté loop même antérieur à l'epoch.

## Definition of Done

- `_stamp_pr_origin` existe, est appelée aux trois sites, fail-open, bornée par `timeout`,
  et ne revendique qu'une PR non revendiquée.
- `.github/labels.yml` porte la section Origin.
- `scripts/pr-origin-report.sh` produit la répartition en une commande.
- Les deux suites de tests passent.
- La cause du trou `pr_url` est nommée dans le corps de la PR, avec ses mesures.
- `bash scripts/verify-pipeline.sh` passe.

## Acceptance criteria

- [ ] Pour toute PR mergée sur un dépôt contrôlé, on peut déterminer son origine (loop / à la main) **à partir des artefacts seuls**, sans consulter une liste tenue à la main.
- [ ] La détermination est vérifiable *a posteriori* sur les PR déjà mergées, ou bien le ticket dit explicitement à partir de quelle date la mesure devient valide (une coupure nette et datée vaut mieux qu'une couverture floue).
- [ ] Une commande ou requête unique rend la lecture : nombre mergé sur une fenêtre, réparti par origine.
- [ ] Le cas du 27/08 (5 merges loop, 0 `pr_url`) est utilisé comme cas de test : la nouvelle mesure doit les compter correctement, ou dire pourquoi elle ne peut pas rétroactivement.
- [ ] Si la voie 1 est retenue, la cause du trou est nommée dans le ticket ou le PR — pas seulement colmatée.

## Hors périmètre

- Changer la cadence ou le volume de dispatch.
- Attribuer une origine à autre chose que les PR.
- La pose automatique de `origin:spawn` / `origin:manual` (hors dépôt `mika`).
