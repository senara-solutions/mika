# chore(ci) — câbler les 25 suites `unwired` du registre, et laisser le lint arbitrer (mika#2793)

> Fire-Disposition de l'arriéré que mika#2761 a mesuré. Le périmètre est
> **exactement** la colonne `unwired` de `scripts/suite-wiring.tsv`, soit 25
> entrées au 2026-10-19. Le travail n'est pas d'écrire un garde : les 25 gardes
> existent, ils sont écrits, revus, mergés — et aucun ne tourne. Le travail est
> de les brancher, et de réparer ce que leur silence a laissé pourrir.

---

## Constat, mesuré dans l'arbre le 2026-10-19

Les 25 suites ont été **lancées à la main** sur la branche de grooming, une par
une (`bash <chemin>`), et leur sortie lue. Ce recensement est le fait central du
plan : il n'était pas dans le ticket, et il change le travail.

**Trois des 25 sont déjà rouges, et une quatrième n'est pas hermétique.**
C'est le coût exact que mika#2578 nomme : un garde que le pipeline ne lance pas
ne refuse rien — et pendant qu'il ne refuse rien, il pourrit sans témoin.

### Les 10 suites sous `scripts/`

| suite | état mesuré |
|---|---|
| `check-release-debuginfo.sh` | vert |
| `test-check-release-debuginfo.sh` | vert, 32/0 |
| `test-dispatch-symmetry.sh` | vert |
| `test-measure-empty-turns.sh` | vert, 34/0 |
| `test-measure-pilot-cycle-emptiness.sh` | **ROUGE, 34/1** |
| `test-mika-groom-milestone.sh` | **ROUGE, 24/1** |
| `test-smoke-search-substrate.sh` | vert, 22/0 |
| `test-smoke-webhook-chain.sh` | vert, 39/0 |
| `test-verify-dependabot-topology.sh` | vert, 24/0 |
| `verify-pipeline-test.sh` | vert ici, 29/0 — **mais non hermétique**, voir C3 |

### Les 11 suites offline sous `skills/bundled/_shared/`

Toutes vertes, mesurées : `test-trace-redaction.sh` ; `tests/test_parse_disposition.sh` ;
`tests/test_finalize_pr_gate.sh` ; `tests/test_force_push_guard.sh` (14/0) ;
`tests/test_push_with_rebase_retry.sh` ; `tests/test_rescue_cause_token.sh` (52/0) ;
`tests/test_rescue_closes_guard.sh` (29/0) ; `tests/test_rescue_commit_no_verify.sh` (9/0) ;
`tests/test_rescue_scratch_not_published.sh` (46/0) ;
`tests/test_seed_worktree_slash_commands.sh` (11/0) ;
`tests/test_stamp_issue_seat.sh` (92/0).

Aucune n'a besoin de `cargo`, de `bwrap`, du réseau ni d'un jeton. Sept
configurent l'identité git de leurs dépôts de fixture elles-mêmes ; les quatre
autres ne commitent pas.

### Les 4 suites à précondition d'environnement

| suite | précondition | état mesuré |
|---|---|---|
| `tests/test_rescue_fmt_clean.sh` | `cargo fmt` — **skip** explicite sans cargo (sa ligne 83) | vert, 23/0 |
| `tests/test_egress_relay_socket_and_env.sh` | `bwrap` réel — skip sans (ligne 55) | vert, 15/0 |
| `tests/test_sandbox_git_usable.sh` | `bwrap` réel + racine clonable (ligne 66) | **skip ici** : « ⊘ skipped — could not create the probe worktree » (worktree imbriqué) |
| `tests/test_sandbox_log_dir_bound.sh` | `bwrap` réel + `/var/log`, `/var/lib`, `/srv` hôte | **ROUGE, 11/5** |

Ces quatre skippent **proprement** quand leur précondition manque. C'est ce qui
les rend câblables (`ci.yml:133-149` le fait déjà pour deux suites, au motif
écrit qu'une suite qui dégrade en skip est sans danger) — et c'est aussi ce qui
rend leur câblage dangereux : **sur un runner sans la précondition, elles sont
vertes par skip permanent**, c'est-à-dire décoratives, le défaut mika#2103 une
couche plus haut. Leur câblage doit donc être accompagné de la lecture du log,
pas seulement d'un job vert (AC5).

### Les trois rouges, avec leur cause

**C1 — `test-measure-pilot-cycle-emptiness.sh` : un bit de mode.**
L'unique assertion rouge est `the script is committed as 100644, not 100755`.
Le script mesuré est `scripts/measure-pilot-cycle-emptiness` (sans extension —
il échappe donc à la population `D` du lint, qui est définie par convention de
nommage). Il est appelé par `bash "$MEASURE_PATH"`, donc le bit ne change rien à
son exécution : il change ce que **le dépôt déclare**, et c'est précisément ce
que l'assertion garde. Réparation : `git update-index --chmod=+x`.

**C2 — `test-mika-groom-milestone.sh` : une assertion périmée depuis onze mois.**
Les deux assertions `LLM override for groom-milestone` et
`LLM override uses opus-4-7` (lignes 73-74) cherchent le littéral
`skill_name: "mika-arch-groom-milestone"` dans `well_known_agents.rs`. Il n'y est
plus : `6ccb79c4` — *fix(mika-arch): always_on=true + remove per-skill LLM
overrides (mika#949)* — a retiré les overrides par skill, et les quatre specs
d'agents bien connus portent aujourd'hui `llm_overrides: &[]`
(`well_known_agents.rs:88`, `:109`, `:305`, `:361`). La suite date de mika#879 ;
elle asserte un site que mika#949 a supprimé **volontairement**.

Ce n'est donc pas une régression de comportement et il n'y a rien à escalader :
c'est un réalignement de la suite sur son producteur, la classe de mika#2788.
**Garde-fou avant de toucher à la suite** : vérifier que les quatre
`llm_overrides` sont bien vides. S'il en restait un, la lecture serait inverse
(l'override aurait été perdu pour un agent et pas pour les autres) et la règle
mika#1190 — aucun swap de modèle sans calibration — s'appliquerait.

**C3 — `verify-pipeline-test.sh` : 29 commits sur une identité héritée.**
Le harnais crée son dépôt de fixture avec `git init` et y fait 29 `git commit`
sans jamais déclarer `user.email` / `user.name` — ni par `git config`, ni par
`-c user.…`, ni par `GIT_AUTHOR_*` (vérifié par grep, zéro occurrence). Il passe
ici parce que la machine porte une config globale. Sur un runner hébergé neuf,
le premier commit échoue (`Author identity unknown`). La suite est verte et
**non hermétique** : son vert mesure la machine autant que le code.

**C4 — `test_sandbox_log_dir_bound.sh` : 5 rouges, dont deux contrôles d'hôte.**
Les cinq sont `control: /var/lib exists host-side`, `control: /srv exists
host-side`, et les trois refus d'écriture dans `/var/log`, `/var/lib`, `/srv`.
Deux d'entre elles sont des contrôles d'existence de chemins **de la machine**,
pas du code : la suite suppose un hôte que ce Gentoo ne présente pas. Un runner
`ubuntu-22.04` les porte tous les trois, donc la suite y passera probablement —
mais *probablement* n'est pas une mesure, et c'est la seule des 25 dont l'état
cible doit être établi par un aller-retour CI. Le plan lui donne une clause de
sortie bornée (Fire-Disposition, cas 2).

### Ce que le lint tient déjà, et qui dispense d'inventer une mécanique

`scripts/check-suite-wiring.sh` est l'arbitre et il est bidirectionnel :

- **D1** rougit sur une suite câblée nulle part et non déclarée ;
- **D2** rougit sur une entrée de registre dont la suite est devenue atteignable.

Donc retirer une ligne **sans** câbler ⇒ `[D1]`, et câbler **sans** retirer la
ligne ⇒ `[D2]`. Les deux moitiés sont forcées dans le même commit par
construction : aucune discipline à ajouter, aucune liste à tenir à jour. Le
`# min-population: 85` du registre borne la population et **n'a pas à changer** :
ce plan ne supprime aucune suite.

Le fichier `scripts/suite-wiring.tsv` **reste** après le travail : il garde ses
deux entrées `operator` (`measure-plan-header-coverage.sh`,
`verify-dependabot-topology.sh`, hors périmètre) et il porte le `min-population`
sans lequel le lint sort en 3.

---

## Décisions de conception

**D1 — Câbler par H1 (`bash <chemin>` littéral), une seule exception H2.**
Le lint accepte deux sauts : H1, un workflow nomme le chemin ; H2, un workflow
nomme `make <t>` dont la recette nomme le chemin. H1 est retenu pour 23 des 25 :
un lecteur du YAML voit la suite, sans ouvrir le Makefile. L'exception est la
paire `check-release-debuginfo.sh` + `test-check-release-debuginfo.sh`, câblée
par `make check-release-debuginfo` : la cible existante (`Makefile:337-339`)
encode l'**ordre** garde-puis-preuve-de-garde, et le dupliquer dans le YAML
créerait un second site de vérité pour cet ordre. Le lint suit H2 et les deux
lignes de registre partent ensemble.

**D2 — Deux nouveaux jobs sur `ubuntu-22.04`, et rien dans `check` pour les 21.**
Frontière : la racine (`scripts/` vs `skills/bundled/_shared/`), qui est aussi
une frontière de nature — outillage du dépôt d'un côté, plomberie de dispatch de
l'autre. Les 21 n'ont besoin ni de `cargo` ni de `bwrap`, donc un runner hébergé
suffit, et deux jobs parallèles n'allongent pas le chemin critique. Le job
`check` est le plus long du pipeline et le check requis : y verser 21 suites
shell le rallongerait sans rien prouver de plus.

**D3 — Un step par suite, pas un `run: |` groupé.**
Le nom du step est ce qu'un lecteur de la page *Checks* voit quand une suite
rougit. Un bloc groupé rend « laquelle a rougi » invisible sans ouvrir le log —
et les suites les plus susceptibles de rougir gardent la plomberie de dispatch,
lue en situation par un pilote bloqué. Coût : ~3 lignes par suite ; c'est le prix
du diagnostic, et il est payé une fois.

**D4 — Les 4 préconditionnées vont dans le job `check` (self-hosted gentux).**
C'est le seul runner qui porte `cargo` **et** où les suites sandbox réelles
tournent déjà (`ci.yml:139-140`, `:148-149`). Les y mettre est la seule façon
que leurs assertions soient *exécutées* plutôt que *skippées*. Corollaire
obligatoire : AC5 exige la lecture du log, suite par suite, parce qu'un job vert
ne distingue pas « 15 assertions passées » de « ⊘ skipped ».

**D5 — Les trois réparations sont dans le périmètre, et arrivent AVANT le câblage.**
Un câblage rouge n'est pas un câblage, et le ticket exige que la ligne de
registre parte dans le même commit que le câblage. Les réparations C1/C2/C3
forment donc le commit 1, **sans toucher au registre** : à cet instant les 25
suites sont encore déclarées, le lint est encore vert, et chaque réparation est
lisible seule.

**D6 — Pas de `git config --global` dans le YAML pour réparer C3.**
Ce serait vert en cachant la non-hermeticité : la suite continuerait à mesurer
la machine, et la machine suivante la retrouverait. L'identité se déclare sur le
dépôt de fixture, dans `setup_test_repo()`.

**D7 — Un `operator` de complaisance est refusé par construction.**
Aucune des 25 n'est basculée en `operator` : la mesure ci-dessus montre que
toutes tournent hors-ligne. « Needs bwrap » n'est pas « needs the outside
world », et le registre le dit déjà dans son en-tête. Les deux seules issues
pour une suite que le câblage n'arrive pas à rendre verte sont celles de la
Fire-Disposition : rester `unwired` avec un ticket de suivi nommé, ou être
réparée.

**D8 — Pourquoi un seul vol.**
Le total estimé (185 lignes, voir § *Taille estimée*) passe sous la borne par
vol de 279 et sous le seuil par PR de 1000, donc mika#2693 commande un vol
unique. Il tient parce que le travail est répétitif et non ramifié : 25 steps de
trois lignes, un registre qui perd 25 lignes, et trois réparations dont deux
sont d'une ligne. Le seul poste à profondeur inconnue est C4, et il est borné
par la clause de sortie de la Fire-Disposition plutôt que par une estimation.

**D9 — Hors du diff, et dit ici pour ne pas être découvert plus tard.**
L'ajout des deux nouveaux jobs à la liste des *required status checks* de `main`
est un réglage de dépôt, pas un fichier (le jeton de cette session n'y accède
pas : `403` sur `branches/main/protection`). Le contrat du lint mika#2761 est
l'**atteignabilité depuis un workflow**, que ce plan satisfait entièrement ; la
promotion en check requis est une action opérateur à mentionner dans le corps de
PR, non un critère d'acceptation.

---

## Travail, dans l'ordre des commits

### Commit 1 — réparer ce que le silence a laissé pourrir (registre intact)

1. `scripts/measure-pilot-cycle-emptiness` → `git update-index --chmod=+x`
   (C1). Vérifier ensuite `bash scripts/test-measure-pilot-cycle-emptiness.sh`
   → 35/0.
2. `scripts/test-mika-groom-milestone.sh` (C2) : vérifier d'abord que les quatre
   `llm_overrides` de `well_known_agents.rs` sont vides ; si oui, retirer les
   deux assertions des lignes 73-74 et laisser à leur place une ligne de
   commentaire citant `6ccb79c4` / mika#949 — *l'override par skill a été retiré
   volontairement ; la suite n'a plus de site à asserter*. Si **un** override
   subsistait, ne pas toucher la suite : s'arrêter et remonter (mika#1190).
   Vérifier : 25/0 ou 24/0 selon le nombre d'assertions restantes, zéro échec.
3. `scripts/verify-pipeline-test.sh` (C3) : dans `setup_test_repo()`, après
   `git init`, déclarer `user.email` et `user.name` sur le dépôt de fixture.
   Vérifier : 29/0, puis relire la fonction pour s'assurer qu'aucun autre
   `git init` du fichier n'est resté sans identité.

### Commit 2 — les 10 suites de `scripts/`, registre réduit dans le même commit

Nouveau job `offline-script-suites` (`ubuntu-22.04`, `actions/checkout` au SHA
épinglé déjà utilisé dans `ci.yml`), un step par suite, avec en tête du job un
commentaire disant ce que le job est : **les gardes de l'outillage du dépôt, qui
ont été écrits puis laissés sans lanceur**, et citant mika#2793 / mika#2761.
Neuf steps `bash scripts/<suite>.sh` + un step `make check-release-debuginfo`
(D1).

Dans le **même commit**, retirer les 10 lignes correspondantes de
`scripts/suite-wiring.tsv`. Ne pas toucher `min-population`.

### Commit 3 — les 11 suites offline de `skills/bundled/_shared/`

Nouveau job `dispatch-lib-offline-suites`, même forme, onze steps
`bash skills/bundled/_shared/…`. Commentaire de tête : ces suites gardent la
plomberie que chaque vol du loop traverse (rescue, push, seat, gate de PR,
redaction des transcripts) ; aucune ne touche le réseau.

Retirer leurs 11 lignes du registre dans le même commit.

### Commit 4 — les 4 préconditionnées, dans le job `check`

Quatre steps ajoutés au job `check`, à la suite des deux steps sandbox
existants (`ci.yml:139-149`), avec un commentaire nommant la précondition de
chacun et la raison pour laquelle ce job est le bon : c'est le seul runner qui
porte `cargo` et un `bwrap` réel. Puis :

1. Pousser, lire le log du job `check` et **mesurer** : chaque suite affiche-t-elle
   un compte d'assertions non nul, ou la ligne `⊘ skipped` ?
2. Pour celles qui exécutent réellement et passent : retirer leur ligne du
   registre (même commit, ou un commit de suivi si la mesure arrive après).
3. Pour `test_sandbox_log_dir_bound.sh` (C4) : si ses cinq assertions passent sur
   le runner, rien d'autre à faire. Si elles rougissent, **une** tentative de
   correction : rendre les deux contrôles d'existence hôte (`/var/lib`, `/srv`)
   skippants comme la paire `/var/log/mika` l'est déjà dans la même suite — un
   contrôle d'environnement qui échoue doit dire « absent », pas « rouge ». Si
   cela ne converge pas, appliquer la clause de sortie (Fire-Disposition, cas 2).

### Commit 5 — ce que le dépôt déclare après

1. En-tête de `scripts/suite-wiring.tsv` : la phrase qui annonce
   « The wiring of the whole `unwired` column is tracked collectively by
   mika#2793 » devient fausse dès que la colonne est vide — la réécrire pour dire
   ce qui est vrai après, et ce que `grep -c unwired` doit rendre.
2. `docs/solutions/best-practices/une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md` :
   clore le recensement du 2026-09-28 (ses quatre harnais nommés sont câblés) et
   y inscrire les trois défauts que le câblage a révélés — c'est la matière du
   document, et la preuve que la couche 0 de sa taxonomie coûte réellement.
3. `CLAUDE.md` : la phrase du § CI/CD qui décrit le registre comme « la taille de
   l'arriéré » doit refléter son nouvel état. Une phrase, pas un paragraphe.

---

## Contrat de vérification

| ce qui est vérifié | comment |
|---|---|
| le registre et l'arbre s'accordent | `bash scripts/check-suite-wiring.sh` → exit 0, et la ligne de rapport montre `unwired=0` (ou le résidu exact que la Fire-Disposition autorise) |
| le lint n'a pas été affaibli | `bash scripts/test-check-suite-wiring.sh` → vert (il pointe un arbre de fixture, donc il est insensible au travail ; le vérifier est ce qui le prouve) |
| chacune des 25 suites passe | les 25 lancées à la main, `bash <chemin>`, zéro échec — la mesure refaite après les réparations |
| les 3 sandbox ne sont pas vertes par skip | lecture du log CI du job `check` : compte d'assertions non nul, absence de `⊘ skipped` (AC5) |
| aucune suite n'a disparu | `check-suite-wiring.sh` ne sort pas en 3 ; `min-population: 85` inchangé |
| le dépôt reste sain | `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, `make verify-bundled-skills` |
| fin de pipeline | `bash scripts/verify-pipeline.sh origin/main` |

---

## Fire-Disposition

Ce plan livre des détecteurs : il **arme** 25 gardes existants (leur chemin de
succès est « aucune violation trouvée »), et il touche le registre d'un lint.
Disposition retenue : **(a) exception nommée en allowlist**, avec une allowlist
qui existe déjà et qui s'auto-nettoie — `scripts/suite-wiring.tsv`.

**Cas 1 — les trois suites rouges mesurées (C1, C2, C4).**
Elles sont **réparées**, pas exemptées : un bit de mode (C1), deux assertions
réalignées sur leur producteur avec le commit cité (C2), et pour C4 une tentative
de correction bornée. Une exception pour une suite réparable en une ligne serait
une entrée d'arriéré cachée derrière un mot — ce que l'en-tête du registre
refuse explicitement.

**Cas 2 — la clause de sortie, bornée, pour une suite qui résiste.**
Si `test_sandbox_log_dir_bound.sh` (ou une suite sandbox que le runner ne peut
pas exécuter réellement) ne converge pas en **un** aller-retour CI, sa ligne
**reste** dans `scripts/suite-wiring.tsv` en mode `unwired`, avec sa raison
réécrite pour dire la mesure (« cinq assertions rougissent sur un hôte sans
`/srv` ; diagnostic renvoyé à mika#NNNN ») et le ticket de suivi dans la colonne
4. Cette entrée est grep-visible (`grep unwired scripts/suite-wiring.tsv`), elle
nomme la donnée précise, elle cite un ticket, et elle est **auto-nettoyante par
construction** : la direction `[D2]` du lint rougit le jour où quelqu'un la
câble. C'est l'allowlist canonique de mika#1574 — et c'est pourquoi elle ne
dégénère pas en celle que mika#2309 condamne.

**Ce qui n'est pas autorisé** : basculer une de ces suites en `operator`. La
mesure du § *Constat* établit que les 25 tournent hors-ligne ; `operator` dirait
le contraire, et un `operator` de complaisance est exactement l'entrée d'arriéré
déguisée que le ticket interdit (D7).

**Pas de livraison désarmée (b)** : un job CI ajouté puis neutralisé serait un
garde que le pipeline ne lance pas, soit la définition du défaut qu'on répare.
**Pas de halte (c)**, sauf sur une seule découverte : si C2 révélait qu'un
override LLM subsiste pour un autre agent, l'implémentation s'arrête et remonte
(mika#1190, aucun swap de modèle sans calibration passante).

---

## Definition of Done

- [ ] `grep -c unwired scripts/suite-wiring.tsv` ne compte plus que l'en-tête
      (zéro entrée de données), ou le résidu exact que le cas 2 de la
      Fire-Disposition autorise, chacun avec sa raison mesurée et son ticket.
- [ ] Les 25 suites sont atteignables depuis un workflow, et
      `bash scripts/check-suite-wiring.sh` le confirme en sortant 0.
- [ ] Chaque retrait de ligne du registre est **dans le même commit** que le
      câblage de la suite correspondante.
- [ ] Les trois défauts mesurés (C1 bit de mode, C2 assertion périmée, C3
      non-hermeticité git) sont réparés, chacun dans un commit lisible seul.
- [ ] Le log CI atteste que les trois suites sandbox exécutent leurs assertions
      au lieu de skipper.
- [ ] `min-population: 85` est inchangé, et `scripts/suite-wiring.tsv` existe
      toujours avec ses deux entrées `operator`.
- [ ] `cargo test`, clippy, fmt, `make verify-bundled-skills` et
      `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Le corps de PR nomme l'action opérateur de D9 (promotion des deux nouveaux
      jobs en checks requis) et, si le cas 2 s'applique, le `Tracked in:` du
      résidu.

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son § *Travail attendu* et du contrat de vérification.

- **AC1 — l'arriéré est vidé, et le lint l'arbitre.**
  `bash scripts/check-suite-wiring.sh` sort 0 et rapporte `unwired=0`, ou
  `unwired=<n>` où les `n` entrées restantes sont exactement celles que la
  Fire-Disposition cas 2 autorise, chacune portant une raison qui cite une mesure
  et un ticket de suivi.

- **AC2 — câblage et registre bougent ensemble.**
  Pour chaque suite câblée, le commit qui ajoute son step retire sa ligne du
  registre. Vérifiable par relecture de l'historique : aucun commit intermédiaire
  ne laisse `check-suite-wiring.sh` rouge en `[D1]` ou `[D2]`.

- **AC3 — les quatre suites nommées par le ticket sont atteignables.**
  `scripts/verify-pipeline-test.sh`, `scripts/test-smoke-search-substrate.sh`,
  `scripts/test-measure-pilot-cycle-emptiness.sh` et
  `scripts/test-mika-groom-milestone.sh` sont nommées par un
  `.github/workflows/*.yml`, et la correction du recensement de mika#1801 est
  faite : `check-release-debuginfo.sh` et son harnais sont câblés aussi.

- **AC4 — les trois rouges mesurées sont vertes, par réparation et non par
  contournement.**
  `scripts/measure-pilot-cycle-emptiness` est committé `100755` ;
  `test-mika-groom-milestone.sh` ne cherche plus un site retiré par mika#949 et
  cite `6ccb79c4` à l'endroit de l'assertion supprimée ;
  `verify-pipeline-test.sh` déclare l'identité git de son dépôt de fixture et ne
  dépend plus d'une config globale.

- **AC5 — aucune suite n'est verte par skip permanent.**
  Le log du job qui porte les trois suites sandbox montre, pour chacune, un
  compte d'assertions non nul et pas la ligne `⊘ skipped`. Une suite qui skippe
  sur le runner retenu n'est pas comptée comme câblée : sa ligne reste au
  registre avec sa mesure (Fire-Disposition cas 2).

- **AC6 — aucune suite n'est supprimée et aucune n'est requalifiée `operator`.**
  La population reste au-dessus du plancher déclaré (`min-population: 85`
  inchangé), et le registre ne gagne aucune entrée `operator` : la mesure du plan
  établit que les 25 tournent hors-ligne.

- **AC7 — ce que le dépôt déclare est à jour.**
  L'en-tête du registre ne décrit plus la colonne `unwired` comme un arriéré
  collectif tracké par mika#2793 si elle est vide ; le recensement de mika#1801
  est clos dans son propre document, avec les trois défauts révélés ; la phrase
  de `CLAUDE.md` sur le registre dit l'état d'après.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `.github/workflows/ci.yml` — job `offline-script-suites` (10 steps + commentaires) | 55 |
| `.github/workflows/ci.yml` — job `dispatch-lib-offline-suites` (11 steps + commentaires) | 52 |
| `.github/workflows/ci.yml` — 4 steps préconditionnés dans `check` + commentaires | 26 |
| `scripts/suite-wiring.tsv` — 25 lignes retirées + en-tête réécrit | 30 |
| `scripts/test-mika-groom-milestone.sh` — réalignement sur mika#949 (C2) | 8 |
| `scripts/verify-pipeline-test.sh` — identité git du dépôt de fixture (C3) | 4 |
| `scripts/measure-pilot-cycle-emptiness` — bit `100755` (changement de mode) | 0 |
| `skills/bundled/_shared/tests/test_sandbox_log_dir_bound.sh` — contrôles d'hôte skippants (C4, conditionnel) | 10 |

Total estimé : 185 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Un seul vol. Il tient parce que le travail est répétitif et non ramifié — 25
steps de trois lignes, un registre qui perd 25 lignes, trois réparations dont
deux font une ligne — et parce que le seul poste à profondeur inconnue (C4) est
borné par la clause de sortie de la Fire-Disposition plutôt que par une
estimation : il consomme un aller-retour CI, puis il se déclare.

---

## Références

- La livraison du refus : mika#2761 (`scripts/check-suite-wiring.sh`, job `suite-wiring-lint`)
- La doctrine : mika#2578 — un garde que le pipeline ne lance pas ne refuse rien
- Les deux récidives : mika#2752
- Le recensement et son discriminant : mika#1801
- Le piège de l'allowlist, fermé par la bidirectionnalité : mika#2309
- Le contrôle de population positive : mika#2205
- Un garde que personne n'a vu rougir est une décoration : mika#2103
- Le retrait des overrides LLM par skill : `6ccb79c4`, mika#949
- Aucun swap de modèle sans calibration : mika#1190
- Ce que l'absence de CI de `verify-pipeline-test.sh` a déjà coûté : mika#2516
