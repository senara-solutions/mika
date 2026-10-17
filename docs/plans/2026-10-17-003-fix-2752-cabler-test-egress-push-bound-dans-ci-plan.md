# mika#2752 — Câbler dans `ci.yml` les deux suites que le scope `workflow` avait laissées dehors (vol 1)

> Plan content-only produit par `/mika-groom-plan-only`. Ticket : senara-solutions/mika#2752.
> Labels portés : `bug`, `p1-important`, `infrastructure`, `ready`, `dispatch:loop`, `loop-substrate`.

---

## 1. Contexte : deux suites vertes que rien ne lance

### 1.1 L'écart, mesuré dans ce worktree le 2026-10-17

Deux suites sont présentes sur disque, portent une cible `make`, passent, et
n'apparaissent dans **aucun** workflow :

| suite | cible `make` | site de la cible | appelée par `ci.yml` ? |
|---|---|---|---|
| `skills/bundled/_shared/tests/test_egress_push_bound.sh` (mika#2736) | `test-egress-push-bound` | `Makefile:260-261` | **non** |
| `scripts/test-measure-webhook-turn-cost.sh` (mika#2680 vol 2) | `test-measure-webhook-turn-cost` | `Makefile:204-205` | **non** |

Les deux commandes de vérification de ce constat :

```
grep -n "test_egress_push_bound\|test-measure-webhook-turn-cost" Makefile .github/workflows/ci.yml
# → trois hits, tous dans Makefile ; zéro dans ci.yml
```

Et la cible fourre-tout n'est pas un filet : `grep -n 'make test$\|make test '
.github/workflows/*.yml` ne rend **rien**. `make test` est une cible locale que
CI n'appelle pas — c'est le constat littéral de mika#2578, et il tient encore.

Les deux suites sont donc dans l'état que mika#2578 nomme : *un garde que le
pipeline ne lance pas ne refuse rien.*

### 1.2 Pourquoi elles sont dehors, et pourquoi ce n'est pas un oubli de revue

Ce n'est pas une ligne oubliée. Les deux PR portaient leur hunk `ci.yml` et la
forge a refusé le push :

```
 ! [remote rejected] fix/2736/infra-fermer-la-moiti-git-push-de-l
   (refusing to allow a Personal Access Token to create or update
    workflow `.github/workflows/ci.yml` without `workflow` scope)
```

Diagnostic isolé par sonde pendant le vol de mika#2736 : le même commit poussé
seul vers une branche jetable **passe**, et seul le hunk `ci.yml` est refusé. Le
blocage était entièrement dans les permissions du credential que l'addon
d'egress injecte vers `github.com`. La PR mika#2747 (mergée) le confirme par
l'absence : ses neuf fichiers ne comptent pas `ci.yml`.

**L'opérateur a accordé *Workflows: write* au credential le 2026-10-17.** Ce
vol est donc la première écriture de `.github/workflows/` depuis la sandbox —
et son push EST la mesure de cette permission, pas une formalité (§ 5).

### 1.3 Ce que les deux suites tiennent, et qui n'est tenu par rien d'autre

- **`test_egress_push_bound.sh`** (143 assertions, 12 cas) tient le bord
  fail-closed de l'addon d'egress sur `git-receive-pack` : quatre motifs de
  refus, plus les trois cas POSITIFS mesurés au grooming de mika#2736 qui, s'ils
  régressaient, rendraient 403 **tout** push légitime du pilote — le probe
  d'authentification de 4 octets (CASE 2), le corps chunké au-delà du seuil
  (CASE 3), et l'injection du credential hôte sur un push autorisé (CASE 1.2,
  risque R6 : la décision et l'injection vivent dans deux hooks différents).
  Cette classe-là casse la boucle autonome entière, silencieusement, et le seul
  endroit où elle serait vue avant le premier dispatch est cette suite.
- **`test-measure-webhook-turn-cost.sh`** (1 contrôle positif + 6 négatifs)
  tient le lecteur de registre de mika#2680 : l'unité est le TOUR et la valeur
  est une SOMME. Sa mutation la plus naturelle — « une ligne par tour » — rend
  500 au lieu de 1100 sur sa fixture, en sous-comptant le plus fort sur les
  tours les plus chers, c'est-à-dire exactement la population que la mesure
  poursuit.

### 1.4 Le point 3 du ticket est arbitré, et il sort du vol

Le ticket demandait au groom d'arbitrer un lint `check-test-suites-wired.sh`.
**Arbitrage : la classe mérite d'être fermée, mais pas par la forme naïve, et
pas dans ce vol.** La mesure faite ici le montre :

| population (worktree, 2026-10-17) | compte |
|---|---|
| fichiers `test*.sh` (`skills/bundled/_shared/tests/` + `scripts/`) | 61 |
| cibles `make …` invoquées par `ci.yml` | 15 |
| suites invoquées par la cible locale `make test` | ≈ 45 |
| occurrences de `make test` dans `.github/workflows/*.yml` | **0** |

Un lint bidirectionnel naïf classerait ~45 suites en violation le jour de son
atterrissage ; sa Fire-Disposition demanderait 45 exceptions nommées, soit
précisément l'allowlist dont mika#2309 dit qu'elle est l'endroit où la
régression passe inaperçue. Il y a une **décision de forme** à prendre avant
l'implémentation (fermeture transitive ? registre déclaratif à la
`canonical-tokens.tsv` ? borne sur la croissance seule ?), et elle vaut son
propre grooming.

**Tracked in: senara-solutions/mika#2761** — ticket ouvert par ce grooming, avec
la mesure ci-dessus et les trois options de forme.

---

## 2. Les livrables, site par site

### D1 — `.github/workflows/ci.yml` : la ligne de mika#2736 dans l'étape existante

L'étape vit aux lignes 111-116 du job `check`. Deux modifications, un seul hunk :

```yaml
      - name: Pilot sandbox secret-channel guards (mika#2039, mika#2716, mika#2724, mika#2736)
        run: |
          make test-sandbox-secret-argv
          make verify-no-secret-in-setenv
          make test-egress-sentinel-staging
          make test-egress-sentinel-enforcement
          make test-egress-push-bound
```

- Le nom de l'étape gagne `, mika#2736` — le nom d'une étape multi-commandes est
  le seul endroit où un lecteur de log apprend quels contrats elle porte ; un
  nom qui cesse d'énumérer ses suites est un nom qui ment par omission.
- La ligne va **en queue** du bloc, à la suite des deux suites d'egress dont
  elle est la continuation thématique (staging → enforcement → push-bound), pas
  en tête.

### D2 — `.github/workflows/ci.yml` : l'étape de mika#2680 vol 2

Insérée **juste après** l'étape `Handler crash-step naming + trap placement
(mika#2532)` (lignes 108-109), donc avant l'étape de D1 :

```yaml
      - name: Webhook-turn register reader — the turn is a SUM (mika#2680)
        run: make test-measure-webhook-turn-cost
```

Étape à part et non ligne ajoutée à un bloc existant : elle n'appartient à
aucune des familles déjà groupées (ni secret-channel, ni rescue, ni dispatch),
et une étape nommée fait apparaître son verdict propre dans le log de job.

### D3 — `Makefile` : `test-measure-webhook-turn-cost` rejoint `.PHONY`

La cible est déclarée en `Makefile:204` mais absente de la liste `.PHONY` de la
ligne 4 — contrairement à `test-egress-push-bound`, qui y figure, et aux ~50
autres cibles du fichier. Tant que personne ne l'appelait, l'écart était inerte ;
à partir de D2, CI l'invoque, et une cible non-`.PHONY` est une cible qu'un
fichier homonyme à la racine court-circuiterait en rendant « up to date » —
c'est-à-dire un vert qui n'a rien exécuté, la forme exacte du défaut que ce vol
ferme. Un seul mot ajouté à la ligne 4, à côté de `test-handler-crash-step`.

---

## Fire-Disposition

Le vol n'**écrit** aucun détecteur : il en **arme** deux, qui existent, sont
complets et sont verts. C'est la sortie de l'option (b) de mika#1574 — « livrer
désarmé, plus un suivi tracké pour l'armer » — et ce vol EST ce suivi. Le gate
s'applique donc pleinement, et la disposition retenue est :

**Option (a) — exception nommée en allowlist — avec une allowlist vide, et le
vide est mesuré, pas supposé.**

- **Violations existantes : zéro.** Les deux suites ont été exécutées dans ce
  worktree le 2026-10-17, chacune au vert :
  `bash skills/bundled/_shared/tests/test_egress_push_bound.sh` → 12 cas,
  143 lignes `RESULT PASS`, zéro `RESULT FAIL` ;
  `bash scripts/test-measure-webhook-turn-cost.sh` → « 7 passées, 0 échouées ».
  Aucune entrée d'exception n'est donc écrite, et aucune ne doit l'être.
- **Aucun livrable n'atterrit avec un interrupteur de désarmement.** Pas de
  `continue-on-error`, pas de `if:`, pas de `|| true`, pas de `#[ignore]`. Une
  étape CI qui ne peut pas faire rougir le job est le même objet que l'étape
  absente — c'est-à-dire l'état d'avant ce vol.
- **L'arriéré de la classe n'est pas allowlisté ici, il est tracké.** Les ~45
  autres suites que `ci.yml` n'appelle pas (§ 1.4) ne reçoivent ni exception ni
  verdict dans ce vol : elles sont l'objet de mika#2761. Les inscrire dans une
  allowlist de ce vol serait fabriquer précisément la liste dont mika#2309 dit
  qu'elle anesthésie la règle.
- **Ce qui rougit si le vol est faux.** Si D1 ou D2 câble une cible qui
  n'existe pas, ou si la suite rougit sur le runner, le job `check` — requis sur
  chaque PR — devient rouge sur la PR de ce vol même. Le vol est donc son propre
  premier contre-exemple s'il se trompe, et il n'y a pas de fenêtre où il est
  vert à tort.

---

## 3. Ce que le vol ne fait PAS

- **Il ne modifie ni les deux suites, ni leurs cibles `make`, ni l'addon
  d'egress.** Elles sont livrées et vertes ; les toucher ferait perdre la seule
  propriété qui rend ce vol lisible — si le job rougit, la cause est le
  câblage ou l'environnement du runner, jamais un changement de contrat.
- **Il ne câble aucune autre suite.** Même celles du § 1.4 qui le mériteraient
  manifestement : leur population et leur forme de fermeture sont l'objet de
  mika#2761, et un vol qui en ajoute trois au passage rend la mesure de
  mika#2761 fausse avant qu'elle soit prise.
- **Il ne fait pas appeler `make test` par CI.** C'est une des trois options de
  forme de mika#2761, elle change le temps du job, et elle ne se décide pas dans
  un vol de six lignes.
- **Il ne touche pas au credential.** Le point 1 du ticket (scope `workflow`)
  est une action opérateur, faite le 2026-10-17, hors de portée d'une sandbox.

---

## 4. Contrat de vérification

| # | affirmation | comment elle est tenue |
|---|---|---|
| V1 | `make test-egress-push-bound` est invoquée par `ci.yml` | D1 — la ligne est dans le `run:` de l'étape, et `grep -n 'make test-egress-push-bound' .github/workflows/ci.yml` rend un hit |
| V2 | `make test-measure-webhook-turn-cost` est invoquée par `ci.yml` | D2 — idem, dans sa propre étape |
| V3 | les deux cibles existent et pointent sur les bons scripts | `Makefile:260-261` et `Makefile:204-205`, inchangés par le vol ; vérifiés par la simple exécution locale de chaque cible |
| V4 | les deux suites passent sur le runner qui les exécutera | le job `check` de la PR du vol lui-même — il tourne sur `[self-hosted, Linux, X64, gentux]`, la même famille de machine que celle où le vert local a été constaté |
| V5 | aucune étape n'est désarmée | relecture du diff : pas de `continue-on-error`, pas de `if:`, pas de `\|\| true` |
| V6 | le YAML reste valide et l'ordre des étapes est celui annoncé | le job `check` démarre et exécute les deux nouvelles étapes à leur place ; un YAML cassé échoue au parse avant le premier step |
| V7 | la cible nouvellement appelée est `.PHONY` | D3 — `grep -n '^\.PHONY' Makefile` contient `test-measure-webhook-turn-cost` |
| V8 | le push d'un hunk `.github/workflows/` est désormais accepté | le push du commit 1 (§ 5) — c'est la mesure, et son échec est un résultat, pas un incident à contourner |
| V9 | le YAML est valide **avant** le push-mesure, et les deux cibles câblées existent | pré-contrôle local du § 5, avant le commit 1 — sans lui, V8 n'est pas une mesure propre |

### V9 : pourquoi un pré-contrôle, alors que V6 couvre déjà le YAML

Parce que V6 et V8 ne mesurent pas la même chose et que, dans l'ordre du § 5,
V8 arrive le premier. Le commit 1 est poussé **pour apprendre** si la permission
*Workflows: write* existe ; c'est la seule information que ce vol produit et que
personne ne détient encore. Or `git push` ne distingue pas ses causes d'échec
pour l'observateur pressé, et deux diagnostics très différents se présentent au
même endroit :

- un refus de la forge → le point 1 du ticket n'est pas acquis, le vol s'arrête ;
- un YAML que le pilote a cassé → le vol continue, après correction.

Pousser sans pré-contrôle laisse la possibilité d'un troisième cas, le pire :
le push **réussit** (la permission est là, V8 est acquis) et le YAML est cassé,
découvert bien plus tard au parse du job. La permission aurait alors été
attestée par un commit qui casse `ci.yml` sur la branche. Trois commandes à coût
nul suppriment l'ambiguïté avant qu'elle existe :

```
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"
make -n test-egress-push-bound
make -n test-measure-webhook-turn-cost
```

`python3` + `PyYAML` sont présents dans ce worktree (vérifié le 2026-10-17) ;
ni `actionlint` ni `yamllint` ne le sont, et ce vol n'en installe aucun — la
charge utile de la vérification est « le fichier parse et les deux cibles se
résolvent », pas un audit de schéma GitHub Actions. `make -n` résout la cible
sans l'exécuter : il attrape la faute de frappe dans un nom de cible, qui est
l'autre manière dont ces cinq lignes peuvent être fausses tout en ayant l'air
justes.

### Les dépendances d'environnement sont attestées, pas supposées

Les deux suites ont chacune une dépendance externe. Aucune n'est un pari : une
étape **déjà verte** du même job en fait la preuve.

| suite | dépendance | déjà attestée par |
|---|---|---|
| `test_egress_push_bound.sh` | `python3` (ligne 84, heredoc) | `test_egress_sentinel_enforcement.sh`, qui utilise `python3` et tourne dans l'étape `Pilot sandbox secret-channel guards` |
| `test-measure-webhook-turn-cost.sh` | `jq`, `awk` | `test-dispatch-lib.sh`, qui utilise `jq` et tourne dans l'étape `Dispatch-lib assertion suite (mika#1772)` |

Ni l'une ni l'autre ne demande bwrap, git, mitmproxy ou le réseau : la première
le déclare dans son en-tête et stubbe le flow mitmproxy, la seconde travaille sur
des fixtures gelées sous `crates/mika-agent/tests/fixtures/mika2680/`.

### Le coût en temps de job est négligeable, et mesuré

`time` dans ce worktree le 2026-10-17 : `test_egress_push_bound.sh` 0,036 s ;
`test-measure-webhook-turn-cost.sh` 0,049 s. Soit **< 0,1 s** ajoutées à un job
dominé par `cargo clippy` et `cargo test`. L'argument « ça allonge la CI » ne
s'applique pas ici, et c'est une donnée, pas une impression.

### Aucun résidu sur le runner

`test_egress_push_bound.sh` pose un `trap 'rm -rf "$TMPROOT"' EXIT` (ligne 65)
sur un `mktemp -d`. `test-measure-webhook-turn-cost.sh` travaille dans
`$REPO_ROOT/target/mika2680-test`, qu'il `rm -rf` puis recrée à l'entrée
(lignes 53-55) — sous `target/`, donc gitignoré et hors du cache Rust utile.

---

## 5. Ordre de commit — le push du hunk `ci.yml` passe EN PREMIER

C'est le seul point non trivial du vol, et il est dicté par le § 1.2.

**Étape 0 — le pré-contrôle local (V9), avant de committer.** Les trois
commandes du § 4 ci-dessus. Elles coûtent moins d'une seconde et elles sont ce
qui fait du push suivant une mesure à une seule inconnue.

**Commit 1 — `.github/workflows/ci.yml` seul (D1 + D2), poussé immédiatement.**

La permission *Workflows: write* accordée le 2026-10-17 n'a jamais été exercée
depuis la sandbox. Tant que le push n'a pas eu lieu, elle est une affirmation ;
après, c'est un fait. Pousser ce hunk **avant** tout autre travail fait que
l'information arrive au premier tour utile plutôt qu'au dernier — exactement le
contraire de ce qui s'est passé sur mika#2736, où le refus a été découvert en
fin de pipeline, un vol déjà consommé.

Si le push est refusé avec :

```
refusing to allow a Personal Access Token to create or update workflow
`.github/workflows/ci.yml` without `workflow` scope
```

alors, conformément au corps du ticket : **le vol s'arrête proprement, sans
contournement**, et le dit dans son résultat. Aucun repli n'est autorisé — ni
pousser les deux hunks séparément, ni écrire le changement dans un autre
fichier, ni ouvrir la PR sans le hunk, ni demander à l'opérateur de coller quoi
que ce soit. Le résultat du vol est alors : *le scope est toujours absent, le
point 1 du ticket n'est pas acquis, rien d'autre n'a été tenté.*

Deux précisions sur cet état d'arrêt, parce qu'un arrêt mal rapporté se lit
comme un vol raté plutôt que comme la mesure qu'il est :

- **Le RESULT cite la ligne de refus verbatim.** L'orchestrateur doit pouvoir
  distinguer « la forge a refusé le scope » de n'importe quel autre échec de
  push (réseau, divergence, protection de branche) sans relire le log. Le
  verdict du vol porte donc le texte exact rendu par `git push`.
- **Le commit 1 reste sur la branche locale, et aucune PR n'est ouverte.** Il ne
  faut ni l'annuler, ni le reporter ailleurs : le hunk est correct et son seul
  défaut est de ne pas pouvoir partir. Le vol signale explicitement cet état
  (commits locaux, zéro PR) pour que le rattrapage post-flight ne le maquille pas
  en draft-PR de rescousse — un draft nommé « contenu sauvé » dirait le contraire
  de ce que le vol a mesuré.

**Commit 2 — `Makefile` (D3).** Une fois le commit 1 poussé, donc une fois la
permission attestée. Séparé parce qu'il n'a pas la même cause : D3 est une
conséquence de D2 (une cible que CI appelle doit être `.PHONY`), pas un élément
du câblage lui-même, et un diff qui les distingue se relit en une seconde.

**Puis la fin de pipeline** : `cargo fmt --check`, `cargo clippy --all-targets
--all-features -- -D warnings`, `cargo test`,
`scripts/verify-pipeline.sh origin/main`, puis `gh pr create` avec le corps écrit
dans `pr-body.md` à la racine du worktree (règle mika#2211).

Une note sur cette fin de pipeline, utile à qui la verra rougir : **le diff du
vol ne touche aucun crate.** Les trois fichiers candidats sont `ci.yml` et
`Makefile`, et aucun n'entre dans une unité de compilation. Si `cargo clippy` ou
`cargo test` rougit, la cause est dans `origin/main` ou dans l'environnement du
runner, jamais dans ce diff — et le vol ne doit pas partir en chasse d'une
régression Rust qu'il n'a pas pu produire. La fin de pipeline reste obligatoire :
elle est un contrôle de l'état de la branche, pas du hunk.

---

## 6. Risques

| # | risque | atténuation |
|---|---|---|
| R1 | Le push du hunk `ci.yml` est encore refusé | § 5 : arrêt propre et explicite au commit 1, avant tout autre travail. Le coût du vol est alors de quelques tours, pas d'un pilote. |
| R2 | Une suite verte en local rougit sur le runner | Dépendances attestées par des étapes déjà vertes du même job (§ 4). Si elle rougit malgré cela, la PR du vol le montre : c'est le job requis `check` qui l'exécute, et le verdict arrive sur la PR même. |
| R3 | L'insertion casse le YAML (indentation) | Attrapé **avant le commit 1** par le pré-contrôle V9 (§ 5 étape 0), là où la correction coûte une seconde et ne brouille pas le push-mesure. Second filet derrière lui : le job échoue au parse avant son premier step — un échec immédiat et sans ambiguïté, jamais un faux vert. L'indentation cible est celle des étapes voisines : six espaces avant `- name:`, huit avant `run:`. |
| R4 | Un conflit sur `ci.yml` avec une PR concurrente | Le fichier est touché par peu de PR ; en cas de conflit, le hunk de D2 se réinsère après l'étape mika#2532 et celui de D1 en queue du bloc secret-channel — l'ancrage est nommé, pas numérique. |
| R5 | Le renommage d'étape casse une protection de branche référençant le nom | Les protections référencent le nom du **job** (`Check`), pas celui d'une étape. Aucune règle required-check ne porte un nom d'étape. |

---

## 7. Hors périmètre

- Le lint de la classe (point 3 du ticket) — **Tracked in:
  senara-solutions/mika#2761** (§ 1.4).
- Faire appeler `make test` par CI, ou réduire l'arriéré des ~45 suites
  locales : même ticket.
- Toute modification des deux suites ou de l'addon d'egress (§ 3).
- La gestion du credential et de ses scopes : action opérateur.

---

## Definition of Done

- [ ] `ci.yml` porte `make test-egress-push-bound` en queue du `run:` de l'étape
      `Pilot sandbox secret-channel guards`, et le nom de l'étape porte
      `, mika#2736`.
- [ ] `ci.yml` porte l'étape `Webhook-turn register reader — the turn is a SUM
      (mika#2680)` invoquant `make test-measure-webhook-turn-cost`, placée juste
      après l'étape `Handler crash-step naming + trap placement (mika#2532)`.
- [ ] `test-measure-webhook-turn-cost` figure dans la liste `.PHONY` du
      `Makefile`.
- [ ] Aucune des deux étapes ne porte `continue-on-error`, `if:` ou `|| true`.
- [ ] Le pré-contrôle local a tourné **avant** le commit 1 : le YAML parse et
      les deux cibles se résolvent sous `make -n` (V9).
- [ ] Le commit 1 (hunk `ci.yml` seul) a été poussé **avant** tout autre travail,
      et le corps de PR rapporte si le push a été accepté — c'est la mesure de la
      permission *Workflows: write* (V8). En cas de refus, le résultat du vol cite
      la ligne de refus verbatim et aucune PR n'est ouverte.
- [ ] Le job `check` de la PR est vert, et son log montre les deux nouvelles
      étapes exécutées.
- [ ] `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Le corps de PR porte la ligne `Tracked in: senara-solutions/mika#2761`
      pour le point 3 différé.
- [ ] `Code review: delegated to mika-qa (mika#2694)` figure dans le corps de PR.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de sa section « Travail attendu » et du
contrat de vérification du § 4.

- **AC1** — `make test-egress-push-bound` est invoquée par
  `.github/workflows/ci.yml`, dans l'étape `Pilot sandbox secret-channel
  guards`, dont le nom énumère désormais `mika#2736`. *(V1)*
- **AC2** — `make test-measure-webhook-turn-cost` est invoquée par
  `.github/workflows/ci.yml`, dans une étape nommée `Webhook-turn register
  reader — the turn is a SUM (mika#2680)` placée immédiatement après l'étape
  mika#2532. *(V2, V6)*
- **AC3** — Les deux suites s'exécutent et passent dans le job `check` de la PR
  du vol, sur le runner `gentux`, sans `continue-on-error` ni `if:` ni
  `|| true`. *(V4, V5)*
- **AC4** — `test-measure-webhook-turn-cost` est déclarée `.PHONY`. *(V7)*
- **AC5** — Le push d'un hunk sous `.github/workflows/` depuis la sandbox est
  accepté par la forge ; en cas de refus pour scope manquant, le vol s'arrête
  sans contournement, cite la ligne de refus verbatim dans son résultat, n'ouvre
  aucune PR et laisse le commit 1 en place sur la branche locale. *(V8, § 5)*
- **AC7** — Le push-mesure du commit 1 n'a qu'une inconnue : le YAML a été
  validé et les deux cibles résolues localement avant de committer. *(V9, § 5
  étape 0)*
- **AC6** — Le point 3 du ticket (lint de la classe) est arbitré et renvoyé à un
  ticket numéroté, nommé par une ligne `Tracked in:` du corps de PR. *(§ 1.4,
  mika#2761)*

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `.github/workflows/ci.yml` (D1 : 1 ligne ajoutée + 1 ligne de nom modifiée) | 2 |
| `.github/workflows/ci.yml` (D2 : étape neuve — ligne vide + `name:` + `run:`) | 3 |
| `Makefile` (D3 : un mot sur la ligne `.PHONY`) | 1 |

Total estimé : 6 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 6 < 429 et 6 < 1000 : les deux
bornes passent avec trois ordres de grandeur de marge. Le vol n'écrit ni code
de production, ni test, ni documentation de référence — il ajoute cinq lignes à
un fichier de workflow et un mot à une ligne de `Makefile`. Il n'y a pas de
poste susceptible de déborder : chaque hunk est cité mot pour mot au § 2, et
son ancrage dans le fichier est nommé (après telle étape, en queue de tel bloc).

Le coût réel de ce vol n'est pas en lignes mais en **mesure** : le push du
commit 1 est la seule chose que le vol apprend et que personne ne sait encore
(§ 5). Si cette mesure est négative, le vol s'arrête à ce point et a quand même
rendu son information.
