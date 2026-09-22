---
module: .github, crates/mika-agent/src/perimeter, crates/mika-gateway/src/github.rs
tags: [dependabot, cross-repo, topologie, mika-1997, mika-1729, perimetre, auto-merge, mesure]
problem_type: architecture
category: cross-repo-patterns
---

# La topologie Dependabot des trois dépôts autonomes, et ce qu'elle décide

**Tickets :** mika#1997, qui ferme R1 / AC1 de mika#1729.
**Écrit le 2026-09-20**, contre des lectures vérifiées de l'arbre à `fc96a341`.

Ce document existe parce qu'un fichier de configuration ne peut pas être livré
depuis le dépôt qui a pris la décision. mika#1729 exigeait
`.github/dependabot.yml` sur **trois** dépôts ; sa PR#1995 n'en a livré qu'un, et
mika-qa l'a relevé au moment de la revue :
`AC1 ❌ — dependabot.yml only on mika, missing from mika-cloud + mika-platform`.
La PR a été mergée, le finding est devenu mika#1997, et la partie du travail qui
appartient à `mika` est celle-ci : **la décision, écrite là où elle se relit, et
un vérificateur qui la rend observable.**

---

## 1. Un fichier, un dépôt — la contrainte que rien ne contourne

`.github/dependabot.yml` n'a d'effet qu'à la racine `.github/` de **son** dépôt.
Il n'existe aucun mécanisme de synchronisation cross-repo dans cette maison, et
le précédent qu'on croit tenir n'en est pas un : le workflow `labels.yml` lit
`.github/labels.yml` **du même dépôt**. Rien ici ne pousse un fichier ailleurs,
et ce travail n'en crée pas.

La conséquence est structurelle, pas administrative. Le dispatch dérive son
worktree **par dépôt** (`dispatch-lib.sh` appelle `derive-worktree-path --branch
… --repo …`) et le bac à sable `bwrap` ne binde que ce worktree-là. **Un
dispatch sur `senara-solutions/mika` ne peut pas écrire dans
`senara-solutions/mika-cloud`** — ce n'est pas une permission à élargir, c'est la
topologie du dispatch. Un ticket qui dirait « ajouter les deux fichiers » et
partirait en dispatch depuis `mika` produirait une PR qui ne touche rien.

**La route retenue est donc : un ticket frère par dépôt cible.** Les deux cibles
sont déjà dans `DISPATCHABLE_REPOS`
(`crates/mika-agent/src/webhook_dispatch.rs:102`), donc un ticket ouvert sur
chacune est groomable et dispatchable par la boucle, dans le worktree de son
propre dépôt, sans machinerie nouvelle. Les deux corps sont rédigés en § 8.

**Deux routes refusées, et pour des raisons différentes.** Le commit manuel de
l'opérateur est une **échappatoire légitime** — deux fichiers d'une quarantaine
de lignes dont ce document fixe le contenu intégralement ; si l'opérateur
préfère ce geste, § 8 est ce qu'il copie. Écrire par
`gh api PUT /repos/.../contents/...` est **refusé** : cela écrit sur `main` sans
PR ni revue, c'est-à-dire contourne mika-qa, c'est-à-dire contourne exactement la
discipline que mika#1729 sert. Sur `mika-platform`, ce serait en plus écrire sans
revue dans le dépôt qui héberge la boucle.

---

## 2. La table de topologie

| dépôt | écosystème | fichiers qu'une PR touche | périmètre | qui merge |
|---|---|---|---|---|
| `mika` | `cargo` | `Cargo.toml`, `Cargo.lock` (racine) | MECHANICAL | auto-merge (mika-qa → mika-dev) |
| `mika` | `github-actions` | `.github/workflows/*.yml` | DECISION-CORE | **opérateur, par dessein** |
| `mika-cloud` | `cargo` *(si manifeste racine — à vérifier, § 3)* | idem | MECHANICAL | auto-merge |
| `mika-cloud` | `github-actions` | `.github/workflows/*.yml` | DECISION-CORE | **opérateur** |
| `mika-platform` | `github-actions` **seul** | `.github/workflows/*.yml` | DECISION-CORE | **opérateur, 100 %** |

Le routage vers la revue est **déjà acquis pour les trois**, et rien n'est à
ajouter côté gateway : `INTERNAL_REPOS`
(`crates/mika-gateway/src/github.rs:261`) contient les trois dépôts, et
`route_event("pull_request", Some("opened"))` route vers `mika-qa` **sans filtre
d'auteur** (`github.rs:336-338`). Dès qu'une PR Dependabot s'ouvre sur l'un des
deux nouveaux dépôts, la revue part par le chemin qui existe.

---

## 3. Les écosystèmes se vérifient dans le dépôt cible — ils ne se recopient pas

La présomption de départ, héritée de R1 de mika#1729 :

- **`mika-cloud`** : `cargo` (`/`) + `github-actions` (`/`).
- **`mika-platform`** : `github-actions` (`/`) **seul**. C'est un méta-dépôt : les
  sous-dépôts portent leurs propres manifestes Cargo/npm, et les dupliquer au
  niveau du méta-dépôt produirait des PR sur des manifestes qu'il ne contient
  pas.

**Ce n'est qu'une présomption, et elle est portée comme telle.** Elle n'était pas
vérifiable depuis le worktree de `mika` — le bac à sable n'y monte ni `mika-cloud`
ni `mika-platform`. Le vérificateur est une commande, à lancer dans le worktree
du dépôt cible au moment de l'implémentation :

```bash
ls Cargo.toml package.json pyproject.toml 2>/dev/null
ls .github/workflows/*.yml 2>/dev/null | head
```

Deux règles en tirent leur forme, et elles sont dissymétriques :

- **Un manifeste présent que la présomption ignore** (p. ex. un `package.json`
  racine sur `mika-cloud`) est **déclaré** avec les autres, et la divergence est
  écrite dans le corps de la PR.
- **Un manifeste absent que la présomption suppose** fait **tomber** son
  écosystème. Déclarer `cargo` sur un dépôt sans `Cargo.toml` produit un
  écosystème muet, c'est-à-dire une couverture qui a l'air acquise et ne l'est
  pas — très exactement la forme de panne que mika#1997 existe pour fermer.

---

## 4. L'étalement des jours est arithmétique, pas esthétique

`mika` reste **lundi** (son fichier n'est pas touché) ; `mika-cloud` prend
**mardi** ; `mika-platform` prend **mercredi**.

Le calcul, avec `open-pull-requests-limit: 5` par écosystème :

| | écosystèmes | pire cas PR |
|---|---|---|
| `mika` | 2 | 10 |
| `mika-cloud` | 2 | 10 |
| `mika-platform` | 1 | 5 |
| **même jour** | | **≤ 25** |
| **étalé** | | **≤ 10 sur le jour le plus chargé** |

La capacité de mika-qa est **mesurée** et tient dans une phrase de `CLAUDE.md`
(§ mika#2347) : enveloppe de 600 s, exécution sérialisée par `agent_lock`, soit
**~6 revues/heure**. Vingt-cinq PR le même lundi matin, c'est plus de quatre
heures de QA saturée, pendant lesquelles **les PR de la boucle autonome
elle-même font la queue derrière**. Dix, c'est ~1 h 40. mika#2347 venait
précisément de descendre `MAX_PER_TICK` de 3 à 1 sur le réconciliateur de revue
pour cette arithmétique-là ; livrer les trois dépôts sur le même jour rouvrirait
la saturation par l'autre bout.

L'étalement ne coûte rien : la planification Dependabot est par fichier, donc par
dépôt. `open-pull-requests-limit: 5` et le groupement `minor`/`patch` sont
conservés à l'identique — ils bornent déjà le volume et sont la moitié de la
raison pour laquelle le fichier de `mika` a la forme qu'il a.

---

## 5. L'asymétrie DECISION-CORE est VOULUE, et ce n'est pas un défaut à réparer

`classify_pr_files` (`crates/mika-agent/src/perimeter/rules.rs`) est **agnostique
au dépôt** — elle ne prend que des chemins. Sa liste `MECHANICAL_EXACT`
(`rules.rs:129`) ne contient, en fait de manifestes de dépendances, que
`Cargo.toml` / `Cargo.lock` (racine), `pyproject.toml` / `uv.lock` et `LICENSE`.
**`package.json`, `package-lock.json`, `Chart.yaml` et `values.yaml` n'y sont
pas.** Et un test épingle la décision, mot pour mot
(`crates/mika-agent/src/perimeter/tests.rs:425`) :

```rust
fn mika_dependabot_workflow_bump_stays_decision_core() {
    // github-actions ecosystem Dependabot PRs touch `.github/workflows/*.yml`,
    // which is NOT on the MECHANICAL allowlist (workflows carry secrets /
    // permissions surface). Deliberately operator-gated (mika#1729): a workflow
    // bump routes to the operator rather than auto-merging.
```

**La conséquence, écrite avant la vérification et non après :** avec
l'écosystème `github-actions` seul, **toutes** les PR Dependabot de
`mika-platform` seront routées à l'opérateur. La revue autonome s'y déclenchera —
c'est la promesse de mika#1997 et elle sera tenue — mais **aucune ne se mergera
seule**. Sans cette phrase, une sonde de vérification lirait une garde voulue
comme un échec.

**Deux promesses à ne jamais confondre**, parce qu'elles divergent par dépôt :

| promesse | `mika` | `mika-cloud` | `mika-platform` |
|---|---|---|---|
| **la revue se déclenche** | oui | oui | **oui** |
| **la chaîne aboutit seule** | bumps cargo racine uniquement | bumps cargo racine uniquement | **jamais** |

**Ne pas « réparer » cette asymétrie.** Ajouter `.github/workflows/` ou
`package.json` à `MECHANICAL_EXACT` pour faire passer les PR de `mika-platform`
ouvrirait un contournement d'auto-merge pour des PR décisionnelles — des PR qui
touchent la surface de secrets et de permissions du CI. Aucune ligne de
`crates/mika-agent/src/perimeter/` ne doit bouger pour mika#1997.

---

## 6. Pas d'auto-merge dans le fichier, et c'est une ratification

L'AMEND de Prime (2026-07-06) a ratifié **Path 1** — configuration de topologie
plus capacité de relecture — et **rejeté Path 2**, l'auto-merge Dependabot.
Aucun `dependabot.yml` livré sous mika#1997 ne porte de configuration
d'auto-merge, et aucun workflow GitHub Actions d'auto-merge n'est ajouté. Le
merge passe par la revue de mika-qa, dont le chemin Dependabot effectue une
vérification changelog/advisory **distincte du CI** (section obligatoire
`DEP-REVIEW:`, `skills/bundled/qa-review/system_prompt.md`).

Cette contrainte est déjà écrite en tête du fichier de `mika`. Elle doit l'être
dans les deux autres — c'est pourquoi les deux corps de § 7 la portent.

---

## 7. Le contenu des deux fichiers

Écosystèmes à **confirmer** selon § 3 avant de poser le fichier ; ce qui suit est
la présomption, complète et prête.

### `senara-solutions/mika-cloud` → `.github/dependabot.yml`

```yaml
# Dependabot configuration for the mika-cloud repo (mika#1997, fermant R1 / AC1
# de mika#1729).
#
# Load-bearing precondition for the autonomous Dependabot review chain:
#   Dependabot → mika-qa approve → mika-dev merge
#
# Prime AMEND (2026-07-06) ratified Path 1 (topology config + reviewer
# capability), NOT Path 2 (dependabot.yml auto-merge). Do NOT add auto-merge
# config here — merge must route through mika-qa's review, whose Dependabot-PR
# path performs a distinct-from-CI changelog/advisory breaking-change check.
#
# `day: tuesday` is NOT cosmetic. The three autonomous repos stagger their
# schedule days so the weekly burst stays under mika-qa's measured capacity
# (~6 reviews/hour). See, in the mika repo:
#   docs/solutions/cross-repo-patterns/dependabot-topologie-trois-depots-2026-09-20.md
#
# NOTE: `.github/workflows/*.yml` bumps are DECISION-CORE by design and route to
# the operator rather than auto-merging (mika#1729, pinned by
# perimeter::tests::mika_dependabot_workflow_bump_stays_decision_core). That is a
# deliberate gate, not a defect.
version: 2
updates:
  # Rust dependencies (workspace Cargo.toml + member crates).
  # DELETE THIS BLOCK if the repo root carries no Cargo.toml: a declared
  # ecosystem with no manifest is silent, and silence reads as coverage.
  - package-ecosystem: "cargo"
    directory: "/"
    schedule:
      interval: "weekly"
      day: "tuesday"
    open-pull-requests-limit: 5
    groups:
      cargo-minor-patch:
        update-types:
          - "minor"
          - "patch"

  # GitHub Actions used by the CI/release workflows.
  - package-ecosystem: "github-actions"
    directory: "/"
    schedule:
      interval: "weekly"
      day: "tuesday"
    open-pull-requests-limit: 5
    groups:
      github-actions-minor-patch:
        update-types:
          - "minor"
          - "patch"
```

**Le bloc `ignore` de `sqlite-vec` n'est PAS recopié**, sauf si
`grep sqlite-vec Cargo.toml` rend quelque chose dans le dépôt cible. Ce bloc de
`mika` documente un défaut mesuré d'un crate précis (mika#2142) dont le pin `=`
vit dans le `Cargo.toml` de `mika` depuis son commit initial. Recopier une
justification de quarante lignes qui ne s'applique pas produit un commentaire
faux dans un fichier de configuration — et la condition de réveil qu'il porte
(« quand une release ships le fichier manquant ») deviendrait la consigne de
quelqu'un qui n'a rien à réveiller.

### `senara-solutions/mika-platform` → `.github/dependabot.yml`

```yaml
# Dependabot configuration for the mika-platform meta-repo (mika#1997, fermant
# R1 / AC1 de mika#1729).
#
# Load-bearing precondition for the autonomous Dependabot review chain:
#   Dependabot → mika-qa approve → mika-dev merge
#
# Prime AMEND (2026-07-06) ratified Path 1 (topology config + reviewer
# capability), NOT Path 2 (dependabot.yml auto-merge). Do NOT add auto-merge
# config here — merge must route through mika-qa's review.
#
# `github-actions` ONLY, and that is deliberate: this is a meta-repo. The
# sub-repos carry their own Cargo/npm manifests and their own dependabot.yml;
# declaring those ecosystems here would open PRs against manifests this repo
# does not contain.
#
# `day: wednesday` is NOT cosmetic — the three autonomous repos stagger their
# schedule days so the weekly burst stays under mika-qa's measured capacity
# (~6 reviews/hour). See, in the mika repo:
#   docs/solutions/cross-repo-patterns/dependabot-topologie-trois-depots-2026-09-20.md
#
# EXPECTED CONSEQUENCE, stated up front so nobody reads it as a failure: with
# `github-actions` as the only ecosystem, EVERY Dependabot PR on this repo is
# DECISION-CORE and routes to the operator. The autonomous review DOES fire —
# that is mika#1997's promise and it is kept — but nothing here auto-merges.
# Pinned by perimeter::tests::mika_dependabot_workflow_bump_stays_decision_core.
version: 2
updates:
  - package-ecosystem: "github-actions"
    directory: "/"
    schedule:
      interval: "weekly"
      day: "wednesday"
    open-pull-requests-limit: 5
    groups:
      github-actions-minor-patch:
        update-types:
          - "minor"
          - "patch"
```

---

## 8. Les deux corps de tickets frères, prêts à poser

### `senara-solutions/mika-cloud`

> **Titre :** `chore: ajouter .github/dependabot.yml (ferme R1/AC1 de mika#1729 pour mika-cloud)`
>
> **Contexte.** `senara-solutions/mika#1729` a livré la chaîne de revue autonome
> des PR Dependabot (Dependabot → mika-qa approve → mika-dev merge) sur les trois
> dépôts autonomes. Son R1 / AC1 exigeait `.github/dependabot.yml` sur **les
> trois** ; seul `mika` l'a reçu. `senara-solutions/mika#1997` coordonne la
> fermeture, et ce ticket en est la moitié `mika-cloud`.
>
> **À faire.** Ajouter `.github/dependabot.yml` à la racine de ce dépôt, avec le
> contenu fixé au § 7 de
> `senara-solutions/mika:docs/solutions/cross-repo-patterns/dependabot-topologie-trois-depots-2026-09-20.md`.
>
> **Avant d'écrire le fichier, vérifier les manifestes réellement présents :**
> ```bash
> ls Cargo.toml package.json pyproject.toml 2>/dev/null
> ls .github/workflows/*.yml 2>/dev/null | head
> ```
> Un manifeste présent que la présomption ignore est **déclaré** en plus, et la
> divergence est écrite dans le corps de la PR. Un manifeste absent fait
> **tomber** son écosystème : un écosystème déclaré sans manifeste est muet, et
> la couverture a alors l'air acquise sans l'être.
>
> **Contraintes.**
> - `day: tuesday` — l'étalement borne la rafale hebdomadaire sous la capacité
>   mesurée de mika-qa (~6 revues/heure). Ne pas recopier le `monday` de `mika`.
> - **Aucune configuration d'auto-merge**, ni dans ce fichier ni dans un workflow
>   (AMEND de Prime du 2026-07-06 : Path 1 ratifié, Path 2 rejeté).
> - **Ne pas recopier le bloc `ignore` de `sqlite-vec`** sauf si
>   `grep sqlite-vec Cargo.toml` rend quelque chose ici.
> - `open-pull-requests-limit: 5` et le groupement `minor`/`patch` sont conservés.
>
> **À savoir avant de lire le résultat.** Les PR `github-actions` sont
> DECISION-CORE **par dessein** et routent vers l'opérateur au lieu de
> s'auto-merger. C'est une garde voulue, épinglée par
> `perimeter::tests::mika_dependabot_workflow_bump_stays_decision_core` dans le
> dépôt `mika` — pas un défaut de ce ticket.
>
> **Vérification.** Depuis un checkout de `mika` :
> `scripts/verify-dependabot-topology.sh` doit rendre `0` une fois les deux
> tickets frères livrés.

### `senara-solutions/mika-platform`

> **Titre :** `chore: ajouter .github/dependabot.yml (ferme R1/AC1 de mika#1729 pour mika-platform)`
>
> **Contexte.** Identique au ticket frère `mika-cloud` ci-dessus :
> `senara-solutions/mika#1729` exigeait `.github/dependabot.yml` sur les trois
> dépôts autonomes, seul `mika` l'a reçu, et `senara-solutions/mika#1997`
> coordonne la fermeture.
>
> **À faire.** Ajouter `.github/dependabot.yml` à la racine de ce dépôt, avec le
> contenu fixé au § 7 de
> `senara-solutions/mika:docs/solutions/cross-repo-patterns/dependabot-topologie-trois-depots-2026-09-20.md`.
>
> **Contraintes.**
> - **`github-actions` uniquement.** C'est un méta-dépôt : les sous-dépôts
>   portent leurs propres manifestes et leur propre `dependabot.yml`. Déclarer
>   `cargo` ou `npm` ici ouvrirait des PR contre des manifestes que ce dépôt ne
>   contient pas.
> - `day: wednesday` — voir l'étalement ci-dessus.
> - **Aucune configuration d'auto-merge**, ni fichier ni workflow.
>
> **Conséquence attendue, à écrire avant de la constater.** Avec
> `github-actions` pour seul écosystème, **toutes** les PR Dependabot de ce dépôt
> sont DECISION-CORE et routent vers l'opérateur. La revue autonome **se
> déclenche bien** — c'est ce que mika#1997 demande d'établir — mais **aucune PR
> ne se mergera seule ici**. Ce n'est pas un échec : c'est la garde de mika#1729,
> épinglée par test.
>
> **Vérification.** Depuis un checkout de `mika` :
> `scripts/verify-dependabot-topology.sh` doit rendre `0` une fois les deux
> tickets frères livrés.

---

## 9. Le vérificateur

`scripts/verify-dependabot-topology.sh` (dans ce dépôt) lit les trois dépôts et
rapporte, pour chacun : présence du fichier, écosystèmes déclarés, jours de
planification, limite de PR. Il n'ouvre rien, n'écrit rien, ne sonde aucun
upstream. C'est un lecteur, dans la même famille que
`scripts/smoke-search-substrate` (mika#2407) et pour la raison écrite là-bas :
*un réglage qu'on ne peut pas observer n'est pas un réglage, c'est un espoir.*

```bash
scripts/verify-dependabot-topology.sh                  # les trois dépôts
scripts/verify-dependabot-topology.sh owner/repo ...   # une liste explicite
```

Trois codes de sortie, et **le troisième n'est pas un succès** :

| code | signification | remède |
|---|---|---|
| `0` | les dépôts interrogés portent tous un fichier qui déclare au moins un écosystème | — |
| `1` | il en manque au moins un — fait **établi** (dépôt lisible, fichier absent), ou fichier présent mais inerte | poser le fichier, par une PR du dépôt concerné |
| `2` | **rien n'a pu être établi** : `gh` manquant, non authentifié, jeton sans portée, réseau | authentifier `gh`, vérifier la portée du jeton |

**Deux propriétés méritent d'être dites, parce qu'elles décident de lectures.**

*L'absence établie prime sur l'indétermination.* Un dépôt établi nu et un autre
illisible rendent `1` : « la couverture n'est pas complète » est déjà vrai, quoi
que dise le troisième. Rendre `2` ferait disparaître un fait derrière une
incertitude.

*Un 404 n'est pas toujours une absence.* GitHub répond 404 — et non 403 — pour un
dépôt privé hors de portée du jeton. Lu naïvement, un jeton mal porté ferait dire
« le fichier manque » d'un fichier peut-être présent, c'est-à-dire produirait
exactement la fausse mesure que ce script existe pour empêcher. Sur 404 du
fichier, et seulement là, le script relit le dépôt lui-même ; s'il n'est pas
lisible, le verdict est INDÉTERMINÉ.

**Ce n'est pas câblé au CI, à dessein.** Un job dans `mika` qui affirmerait
l'état de fichiers d'**autres** dépôts rougirait sur la moindre erreur `gh`
transitoire et demanderait un jeton à portée cross-repo sur le chemin critique.
C'est une commande d'opérateur, et c'est dit.

`scripts/test-verify-dependabot-topology.sh` simule `gh` et rejoue les deux états
du monde — **avant** la phase 2 (`1`, les deux dépôts nommés) et **après** (`0`).
Sans lui, le contrôle négatif du plan ne serait vérifiable qu'une seule fois,
puisque l'état « d'avant » cesse d'exister dès que les deux fichiers sont livrés.

---

## 10. Les haltes

**Halte 1 — la revue ne se déclenche pas sur un nouveau dépôt.**
Ne pas toucher au `dependabot.yml` par réflexe. Vérifier d'abord que le dépôt est
dans `INTERNAL_REPOS` (il l'est, `github.rs:261`) puis que l'événement
`pull_request.opened` est **arrivé**. Sa perte est une classe connue et
instrumentée (file bornée mika#1870 → 429 → circuit breaker → DLQ), et le
rattrapage est `qa_review_reconcile` (mika#2334), pas ce fichier.

**Halte 2 — une PR Dependabot reste ouverte sans merge sur `mika-cloud`.**
Lire d'abord les fichiers qu'elle touche. Si elle touche `.github/workflows/` ou
un manifeste hors `MECHANICAL_EXACT`, c'est le § 5 et c'est nominal. **Ne pas
élargir `MECHANICAL_EXACT`** : ce serait ouvrir un contournement d'auto-merge
pour des PR décisionnelles afin de réparer un symptôme qui n'en est pas un.

**Halte 3 — le volume déborde malgré l'étalement.**
Le levier est `open-pull-requests-limit`, par dépôt et par écosystème, dans le
fichier concerné. Ce n'est pas un réglage de mika-qa, et ce n'est pas une raison
de désarmer le réconciliateur de revue (mika#2347).

**Halte 4 — le vérificateur rend `0` alors qu'un dépôt est visiblement nu.**
C'est un vérificateur qui n'atteste rien, donc le défaut est en lui. Lancer
`scripts/test-verify-dependabot-topology.sh` : sa section 1 est exactement ce
contrôle négatif, et elle rougira.
