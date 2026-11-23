# fix(3121) — le plan lancé est confronté à ce qu'`origin` porte sur la branche nommée (un vol)

**Ticket :** senara-solutions/mika#3121
**Branche :** `fix/3121/dispatch-le-plan-lanc-n-est-confront-ni`
**Sources :** commentaires 54b et 38 (37) du parking senara-solutions/mika#2983

## Le défaut, tel qu'il se mesure

`_detect_plan_on_branch` est le site qui **décide quel plan un pilote exécute** :
il lit le callout `Plan:` du corps du ticket, et sur la seule condition
`-f "$WORKTREE_DIR/$PLAN_PATH"` il réécrit `ENTRY_COMMAND` en
`/ce-work <plan>` (`dispatch-lib.sh:13598-13624`). **Cette condition mesure
l'existence d'un fichier dans un worktree ; elle ne mesure rien de ce qu'`origin`
porte.** Deux incidents distincts sont passés par ce trou, et ils ne sont pas la
même confrontation manquante.

| date | ticket | ce que le pilote a lancé | ce qu'`origin` portait | confrontation absente |
|---|---|---|---|---|
| 17/11 | mika#1727 V3c-2b | le plan **rev 2 non committé** du worktree, hérité de la boucle de révision ; dev-pilot `be5cca2e` (12:36:36Z) l'a committé **lui-même** 16 s plus tard en `9c9d0b67` | le plan de HALTE `59c94bb1` | **A** — le plan lancé vs le plan poussé |
| 09/11 | mika#1727 après V2b-1b | `/ce-work` sur le plan d'une **tranche déjà livrée**, dans le worktree de V2b-2 (dev-pilot `649f81d0`) | la tranche neuve, dont le plan n'était pas celui du callout | **B** — le plan lancé vs la tranche de la branche nommée |

**La chaîne du 17/11 est établie dans le code, pas déduite.**
`_launch_revise_pilot` révise le plan **sur disque, en place** et ne commite
jamais — la révision est détectée par un sha256 avant/après
(`dispatch-lib.sh:10449-10586`). `_iterate_groom_loop` juge donc le fichier sur
disque. `_write_canonical_callout` estampille ensuite
`(committed on branch @ <head_sha>)` avec
`head_sha=$(git -C "$WORKTREE_DIR" rev-parse --short HEAD)`
(`dispatch-lib.sh:11141-11330`) — **une affirmation non mesurée** : rien entre le
`rev-parse` et le mot « committed » ne vérifie que le plan est dans ce commit.
Puis `_detect_plan_on_branch` sert le fichier sans confronter `origin`.

**Pourquoi la confrontation B est bien ouverte, et non déjà fermée par les
liaisons existantes.** Trois gardes la touchent et aucune ne peut la rendre :

- **mika#2034** (liaison plan↔ticket, dans `_committed_plan_on_branch`) est
  *réfutation seule* — elle ne refuse que si l'en-tête du plan réclame un **autre**
  numéro de ticket. Le 09/11 les deux tranches portaient le même numéro : rien à
  réfuter.
- **mika#2981 / mika#3001** (preuve de groom liée au plan) ont atterri les
  2026-10-31 et 2026-11-01, donc **avant** le 09/11 : la liaison qu'elles
  ajoutent est satisfaite par une **paire périmée auto-cohérente** — le vieux
  callout et la vieille preuve nomment le même plan.
- **`_plan_provenance`** (`dispatch-lib.sh:4949-4965`) mesure déjà exactement le
  discriminant de B — elle compare `${gate_ref}:${candidate}` à
  `origin/main:${candidate}` et sait dire *« inherited unchanged from main, not
  committed on this branch »* — mais son doc-comment déclare *« Describes, never
  decides. Deliberately NOT a gate condition »*, et ce refus est **juste pour la
  porte de grooming** qu'elle sert : y bloquer sur l'héritage re-échouerait les
  tickets (KTD1). Le discriminant existe ; aucun site ne l'applique **à la porte
  de lancement**.

**Un discriminant qu'il ne faut pas essayer d'utiliser.** L'en-tête `**Branch:**`
du plan lui-même ne sert à rien : **66 fichiers sur 1 309** de `docs/plans/` en
portent un. Une garde fondée dessus serait N/A sur 95 % de la population.

**Le commentaire 38 laisse la question ouverte verbatim**, et le plan du vol
mika#3118 lui a explicitement donné ce périmètre : *« le troisième écart du
commentaire 54 — le plan jugé par le second passage n'est pas celui que `origin`
porte, et aucune étape ne compare le plan jugé et lancé au plan de la branche sur
origin. […] À garder dans le parking #2983. »*

## Requirements

**R1 — Le plan servi est un fait publié par son producteur.**
`_detect_plan_on_branch` publie le chemin qu'il vient d'élire dans une globale
nommée `_LAUNCHED_PLAN_PATH`, posée sur la **seule** branche où il réécrit
`ENTRY_COMMAND`. Une globale vide signifie « aucun plan n'est servi » et doit
rester le cas nominal de `dev-groom` et du repli `/mika`.

**R2 — Le plan servi est confronté à ce qu'`origin` porte, en trois mesures.**
Une fonction neuve `_confront_launch_plan` mesure :

- `W` — le contenu servi : `git hash-object` du fichier du worktree ;
- `Bb` — ce que la **branche nommée** porte sur `origin`, après
  `git fetch --quiet --force origin refs/heads/<branch>:refs/dispatch-launch/<branch>`
  puis `git rev-parse refs/dispatch-launch/<branch>:<plan>` ;
- `Bm` — ce que `origin/main` porte au même chemin :
  `git rev-parse origin/main:<plan>`.

**R3 — Cinq issues, chacune sous son propre nom greppable.** `nominal`
(`W == Bb` et `Bb != Bm`), `launch_plan_not_pushed` (`W != Bb`),
`launch_plan_absent_on_origin` (la branche est sur `origin`, le chemin n'y est
pas), `launch_plan_inherited_from_main` (`W == Bb == Bm`), `launch_plan_unmeasured`
(la forge est injoignable). Les quatre dernières refusent. Deux populations sous
un même nom ne sont pas soustrayables (mika#2205), et c'est précisément le compte
que l'on veut lire.

**R4 — « Je n'ai pas pu regarder » n'est pas « tout va bien ».** Un refus par
défaut sur l'inmesurable, selon la doctrine mika#2981 : *un vol ne peut pas partir
parce que le réseau manque*. `launch_plan_unmeasured` est classé **à part** de
`launch_plan_absent_on_origin` par le code de sortie de
`git ls-remote --heads origin <branch>` : une branche absente est une mesure, une
forge injoignable n'en est pas une.

**R5 — Le refus est une HALTE, jamais un `auto_skipped`.** Un `auto_skipped`
résout la tâche parente en `completed`, parce que *« le refus est un résultat
correct — le ticket EST groomé »* (`dispatcher.rs:5187-5190`). Ici le ticket n'est
**pas** implémenté : le marquer `completed` le retirerait du compte des vraies
pannes. Le refus prend donc la forme de la **bannière de halte** déjà en service
— `RESULT` dont la **première ligne** est `STATUS=LAUNCH_PLAN_DIVERGED`, puis
`exit 1` — lue par `parse_dispatch_halt_status` (`dispatcher.rs:5311-5317`), qui
rend la parente `failed`.

**R6 — Le relevé a un dénominateur.** La ligne de recensement est émise sur
**toutes** les branches, y compris `nominal` : sans elle, « aucun vol n'a été
refusé » et « la garde ne tourne pas » rendent les mêmes octets
(mika#2293 / mika#3050 / mika#2205). Son **absence** signifie un binaire
antérieur au vol (mika#2340), jamais une absence de divergence.

## Design

**D1 — `_LAUNCHED_PLAN_PATH`, et pourquoi ce n'est pas un réemploi du voisin.**
La globale est posée dans `_detect_plan_on_branch` juste après
`_PLAN_SIZE_PLAN_PATH` (`dispatch-lib.sh:13622`), sur la même branche et par le
même précédent exact — `_PLAN_CALLOUT_REFUSAL` et `PILOT_SHIPPING_TAIL` sont déjà
des globales, pour la même raison (`PLAN_PATH` y est `local`). Elle n'est **pas**
une lecture de `_PLAN_SIZE_PLAN_PATH`, bien que les deux vaudraient la même
chaîne : ce voisin est le discriminant d'émission de la **ligne de taille**, lue
après le vol, tandis que celle-ci est le discriminant d'une **porte**, lue avant.
Les fusionner ferait du désarmement de la porte un effet de bord de l'édition de
l'estimation. Remise à `""` dans `dispatch_claude_pilot`, à côté du
`GROOM_CALLOUT_PLAN=""` qui y est déjà.

**D2 — Un seul point d'étranglement, et il est muet quand la question n'a pas de
sens.** `_confront_launch_plan` commence par
`[ -n "${_LAUNCHED_PLAN_PATH:-}" ] || return 0`. Cela couvre d'un seul terme
`dev-groom` (qui ne sert aucun plan), le repli `/mika` (callout illisible ou
fichier absent) et le dispatch de révision. Pas de test sur `$SKILL` : la question
est « un plan est-il servi ? », et la globale **est** cette réponse.

**D3 — Un espace de refs dédié, `refs/dispatch-launch/<branch>`.**
`_committed_plan_on_branch` écrit déjà dans `refs/dispatch-gate/<branch>`. Un
dispatch de `groom` et un dispatch d'`implement` tournent **en parallèle** sur un
même agent depuis mika#1001 : partager la ref rendrait la mesure de l'un
attribuable à l'autre. Deux espaces, deux populations. Le `--force` est requis —
la ref est réécrite à chaque vol.

**D4 — Le fetch tourne dans `$WORKTREE_DIR`.** Le worktree partage le
`common-dir` du sous-dépôt (`git worktree add` depuis `$SUB_REPO_DIR`), donc la
ref écrite y est visible et `origin/main` y est déjà résolu par
`_set_up_worktree`. Aucune seconde racine à passer.

**D5 — La table des issues, et l'ordre dans lequel elle est évaluée.**
L'ordre importe, parce que deux conditions peuvent être vraies ensemble :

1. `git ls-remote --heads origin <branch>` échoue (code ≠ 0) →
   `launch_plan_unmeasured`. **Premier**, parce que toute mesure en aval serait un
   vide interprété comme une absence.
2. `ls-remote` réussit et ne rend rien → la branche n'est pas sur `origin` →
   `launch_plan_absent_on_origin`.
3. `Bb` vide (branche présente, chemin absent) → `launch_plan_absent_on_origin`.
4. `W != Bb` → `launch_plan_not_pushed`. C'est la **confrontation A**.
5. `W == Bb == Bm` → `launch_plan_inherited_from_main`. C'est la **confrontation
   B** : le plan servi n'est pas une production de cette branche, il est hérité de
   `main`. Équivalent mesurable :
   `git log origin/main..<branch> -- <plan>` est vide.
6. Sinon → `nominal`.

**D6 — Le placement : entre `_handle_dry_run` et `_run_claude_pilot`.**
La chaîne devient (`dispatch-lib.sh:14186-14191`) :

```
_setup_gh_auth → _scrub_env → _set_up_worktree → _detect_plan_on_branch
  → _handle_dry_run → _confront_launch_plan → _run_claude_pilot
```

Deux propriétés achetées par ce placement, et aucune autre position ne les a
toutes deux : `_handle_dry_run` **retire le worktree** puis `exit 0`
(`dispatch-lib.sh:6454-6478`), donc être **après** lui dispense la porte de
connaître la sémantique du dry-run et lui évite de refuser un dispatch qui
n'allait jamais partir ; être **avant** `_run_claude_pilot` est ce qui fait de
l'arrêt un pré-vol — aucun tour de pilote n'est consommé. Le siège de ticket
réclamé par `_set_up_worktree` (mika#2155, `ISSUE_SEAT_CLAIMED`) est rendu par le
trap `EXIT`, qui tourne aussi sur `exit 1`.

**D7 — Le refus, mécanisme exact.** `RESULT` est composé avec
`STATUS=LAUNCH_PLAN_DIVERGED` **seule sur la première ligne**, puis le nom de
l'issue, les trois mesures (`W`, `Bb`, `Bm` abrégés), le chemin du plan, la
branche, et la **route de retour**. Puis `exit 1` — et non
`_deliver_callback` + `exit 0`. Le trap `EXIT` n'emballe en `HANDLER CRASH` que
quand `RESULT` est **vide** (`dispatch-lib.sh:3496-3498`) ; avec `RESULT` posé, la
bannière remonte verbatim, exactement comme `_branch_reuse_gate`
(`dispatch-lib.sh:4388-4396`, dont l'appelant fait `exit 1` en le disant :
`:6067-6073`).

**D8 — La route de retour nommée dans le refus, par issue.** Un refus sans remède
se fait désarmer. Pour `launch_plan_not_pushed` : le plan jugé n'est pas celui
d'`origin` — committer et pousser le plan sur la branche, puis re-dispatcher.
Pour `launch_plan_inherited_from_main` : le callout nomme un plan déjà fondu dans
`main` ; le remède est un **re-groom** (ré-appliquer le label `ready`), qui
réécrira le callout — c'est déjà ce que font mika#3001 et mika#3117 sur un vol
mergé. Pour `launch_plan_absent_on_origin` : même route. Pour
`launch_plan_unmeasured` : rien à corriger côté dépôt, re-dispatcher quand la
forge répond.

**D9 — Le troisième bras du lecteur Rust.** `parse_dispatch_halt_status` gagne
`"STATUS=LAUNCH_PLAN_DIVERGED" => Some("LAUNCH_PLAN_DIVERGED")`. Les deux bornes
de son prédicat restent intactes et sont load-bearing : **égalité de la première
ligne** (pas `contains` — un `RESULT` qui *cite* la bannière pour l'expliquer
serait compté comme la halte, classe mika#2590) et **pas un préfixe `STATUS=`
nu** (la famille porte aussi `CANCELLED_BY_*`, qui ont leur propre issue). Le
jeton est déclaré **classe B**, tolérance `exact:line-anchored`, dans
`scripts/canonical-tokens.tsv` avec ce site — une bannière reformulée côté shell
ferait retomber la parente en zombie, en silence.

**D10 — Un faux positif analysé plutôt que supposé.** La population à risque pour
la confrontation B est « plan fondu dans `main` par une PR `Refs`, ticket
re-dispatché sur une branche neuve » : le plan y est bien hérité de `main` et le
ticket a du travail réel devant lui. Le refus y est **le bon comportement** — le
callout pointe un plan livré, donc le ticket doit être re-groomé avant de
repartir — et c'est exactement ce que la route de retour de D8 nomme. Ce n'est
pas une exception à excepter, c'est le cas que la garde existe pour attraper : il
**est** l'incident du 09/11.

## Périmètre

**Dans le vol :** la publication du plan servi, la fonction de confrontation, ses
cinq issues, la ligne de recensement, la bannière de halte et son lecteur Rust
ancré, l'entrée `canonical-tokens.tsv`, et les fixtures de
`test-dispatch-lib.sh`.

**Hors du vol, délibérément, et nommé :** faire de l'estampille
`(committed on branch @ <sha>)` de `_write_canonical_callout` un **fait mesuré**
plutôt qu'une affirmation (`dispatch-lib.sh:11141-11330`). C'est la cause amont du
17/11 et non son symptôme ; la fermer demande de décider ce que le callout dit
quand le plan n'est pas dans `HEAD` — refuser d'écrire, écrire sans l'estampille,
ou commiter — et aucune de ces trois réponses ne tient dans la borne de ce vol en
plus de la porte. La porte de lancement rend l'incident **non atteignable** sans
attendre cette décision : c'est ce qui rend le découpage légitime.

Tracked in: senara-solutions/mika#2983

**Hors du vol, hors périmètre :** la boucle de révision elle-même
(`_launch_revise_pilot` ne commite pas — c'est son contrat), et la porte de
grooming (`_plan_provenance` reste descriptive, par la décision KTD1).

## Verification Contract

**V1 — `nominal` ne refuse pas et se laisse compter.** Fixture : le plan est
committé sur la branche, poussé, et diffère de `origin/main`. `_confront_launch_plan`
rend 0, `ENTRY_COMMAND` est inchangé, et la ligne de recensement porte
`verdict=nominal`. C'est le **dénominateur** de R6, et le cas qui prouve que la
garde n'est pas un refus inconditionnel.

**V2 — `launch_plan_not_pushed` (confrontation A).** Fixture : le fichier du
worktree est modifié sans commit, la branche sur `origin` porte la version
d'avant. `RESULT` commence par `STATUS=LAUNCH_PLAN_DIVERGED`, porte
`launch_plan_not_pushed`, et la fonction sort non-nul.

**V3 — `launch_plan_absent_on_origin`.** Deux sous-cas, même issue : branche
absente d'`origin` ; branche présente, chemin absent dans son arbre.

**V4 — `launch_plan_inherited_from_main` (confrontation B).** Fixture : le plan
est identique dans `origin/main` et sur la branche, et le worktree le sert.
Refus. **Son contrôle négatif est V1** — même fixture à un commit près, seul le
contenu du plan sur la branche diffère : la paire est ce qui sépare « la garde
mesure l'héritage » de « la garde refuse tout plan ».

**V5 — `launch_plan_unmeasured` est classé à part.** Fixture : `origin` est
injoignable (remote pointé sur un chemin inexistant). Le refus porte
`launch_plan_unmeasured` et **non** `launch_plan_absent_on_origin` — les deux
noms sont asserted distincts, parce que c'est la seule manière de garder les deux
populations soustrayables.

**V6 — Le no-op est positif, pas supposé.** Fixture : `_LAUNCHED_PLAN_PATH` vide
(le cas `dev-groom`). La fonction rend 0, `RESULT` reste vide, et **aucune** ligne
de recensement n'est émise — la question n'a pas de sens, donc elle n'a pas de
réponse à compter.

**V7 — L'ordre d'appel est épinglé à quatre termes.** L'assertion existante de
`test-dispatch-lib.sh:303-306` passe de trois à quatre éléments :
`_set_up_worktree → _detect_plan_on_branch → _handle_dry_run → _confront_launch_plan`.
Une porte placée avant `_handle_dry_run` refuserait un dry-run ; placée après
`_run_claude_pilot`, elle ne refuserait rien.

**V8 — Les deux globales naissent sur la même branche.** Assertion de cardinalité
sur la source de `_detect_plan_on_branch` : `_PLAN_SIZE_PLAN_PATH` et
`_LAUNCHED_PLAN_PATH` y sont posées une fois chacune, dans la même branche `if`.
Elle rougit dans les **deux** sens — si l'une disparaît, et si l'une migre hors de
la branche.

**V9 — Le lecteur Rust, et ses deux bornes.** `parse_dispatch_halt_status` rend
`Some("LAUNCH_PLAN_DIVERGED")` sur un `RESULT` dont la première ligne est la
bannière ; rend `None` sur un `RESULT` qui la **cite** en deuxième ligne ou en
prose (pas de `contains`) ; rend `None` sur `STATUS=CANCELLED_BY_OPERATOR`
(pas de préfixe nu). Et
`canonical_tokens::tests::mika2201_every_match_site_is_declared` reste verte, ce
qui exige l'entrée `canonical-tokens.tsv`.

**V10 — Les portes locales.** `bash skills/bundled/_shared/test-dispatch-lib.sh`,
`make verify-bundled-skills`, `cargo test -p mika-agent`,
`bash scripts/verify-pipeline.sh origin/main`.

## Definition of Done

1. `_detect_plan_on_branch` publie `_LAUNCHED_PLAN_PATH` sur la seule branche où
   il réécrit `ENTRY_COMMAND`, et `dispatch_claude_pilot` la remet à `""`.
2. `_confront_launch_plan` existe, mesure `W`/`Bb`/`Bm` via
   `refs/dispatch-launch/<branch>`, et rend l'une des cinq issues de R3.
3. Les quatre issues de refus posent `RESULT` avec
   `STATUS=LAUNCH_PLAN_DIVERGED` en première ligne et sortent non-nul ; aucune ne
   passe par `auto_skipped`.
4. La ligne de recensement est émise sur les cinq branches, `nominal` comprise, et
   porte le nom de l'issue.
5. Le site d'appel est entre `_handle_dry_run` et `_run_claude_pilot`.
6. `parse_dispatch_halt_status` porte le troisième bras ; le jeton est déclaré
   classe B avec son site dans `scripts/canonical-tokens.tsv`.
7. V1 à V9 sont vertes ; V10 passe.
8. Le périmètre reporté est nommé par la ligne `Tracked in:` de la section
   *Périmètre*, et reporté dans le corps de la PR.

## Acceptance criteria

**AC1** — Un dispatch `dev-pilot` dont le plan servi diffère de ce que la branche
porte sur `origin` **n'atteint pas** `_run_claude_pilot` : il sort sur
`STATUS=LAUNCH_PLAN_DIVERGED` / `launch_plan_not_pushed`. Rejoué sur la forme du
17/11, ce dispatch est refusé là où il partait.

**AC2** — Un dispatch dont le plan servi est identique dans `origin/main` et sur
la branche est refusé sous `launch_plan_inherited_from_main`. Rejoué sur la forme
du 09/11, ce dispatch est refusé là où il partait.

**AC3** — Un dispatch nominal — plan committé sur la branche, poussé, distinct de
`main` — part sans changement observable : `ENTRY_COMMAND` identique, aucun
`RESULT` posé par la porte, et une ligne de recensement `verdict=nominal`. Le
taux de refus se lit donc avec son dénominateur.

**AC4** — Une forge injoignable refuse le vol sous `launch_plan_unmeasured`, nom
distinct de `launch_plan_absent_on_origin`, et jamais sous `nominal`.

**AC5** — Un dispatch `dev-groom` est inchangé bit pour bit : la porte rend 0 sans
mesurer et sans émettre de ligne.

**AC6** — Le refus rend la tâche parente **`failed`** et non `completed` : le
ticket reste dans le compte des pannes réelles. Vérifié par le bras Rust et son
test, pas par relecture.

**AC7** — Chacune des cinq issues a un nom greppable unique, et aucune n'est un
sous-cas silencieux d'une autre : les cinq noms apparaissent une fois chacun dans
la source et une fois chacun dans les assertions.

**AC8** — `make verify-bundled-skills`, la suite `test-dispatch-lib.sh`,
`cargo test -p mika-agent` et `scripts/verify-pipeline.sh origin/main` passent.

## Fire-Disposition

Ce vol livre des détecteurs : la porte `_confront_launch_plan` elle-même, les six
fixtures V1-V6, l'assertion d'ordre V7, l'assertion de cardinalité V8 et le test
du lecteur Rust V9. Option retenue : **(a) exception nommée en allowlist, avec une
population d'exception vide et mesurée.**

- **Population mesurée aujourd'hui : zéro, et la mesure est faite plutôt
  qu'affirmée.** Les deux scans structurels (V8, et l'ordre d'appel V7) portent sur
  du code que ce vol écrit lui-même : `grep -c '_LAUNCHED_PLAN_PATH'` sur
  `dispatch-lib.sh` rend **0** avant le vol, et `grep -c '_confront_launch_plan'`
  rend **0** aussi. Il n'existe aucun site préexistant à excepter.
- **L'allowlist naît donc vide, tenue par des assertions de cardinalité
  auto-nettoyantes** plutôt que par une liste : V8 fixe à **une** chacune des deux
  poses de globale dans la même branche `if`, et V7 fixe la séquence d'appel à
  **quatre** termes nommés. Chacune rougit dans les deux sens — quand un site
  apparaît, et quand le site attendu disparaît — ce qui est ce qui empêche
  l'exception de devenir stale sans que rien ne le dise. Précédents directs :
  `EXPECTED_AUTOFIRE_CONSENT_SITES = 1` (mika#3118) et
  `EXPECTED_ENGAGEMENT_SITES = 3` (mika#3117).
- **La porte de production, elle, naît ARMÉE, et sa population de refus est
  mesurée nulle sur la flotte d'aujourd'hui** : un dispatch nominal pousse son
  plan avant de lancer le pilote (c'est ce que `_push_branch` fait pour `dev-groom`
  avant l'auto-fire), donc `W == Bb` est le cas courant et V1 l'atteste. Les deux
  incidents connus sont les deux seules formes où elle mord, et mordre y est le
  but. Si l'implémentation découvre une troisième forme nominale que la porte
  refuse, elle lui crée une entrée grep-visible nommant la **mesure précise**
  (laquelle des trois comparaisons, et sa valeur), référençant mika#3121, et la
  fait compter par la cardinalité — jamais un élargissement du prédicat.
- **Rien n'est livré désarmé** (option b) : les détecteurs partent armés dans la
  même PR, verts par construction sur le code du vol. **Aucune halte n'est
  requise** (option c) : le périmètre est tranché, la question de forme du refus
  est résolue (D7, bannière de halte et non `auto_skipped`), et le total est sous
  la borne.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — `_confront_launch_plan`, la globale `_LAUNCHED_PLAN_PATH`, sa remise à zéro, le site d'appel | 120 |
| `skills/bundled/_shared/test-dispatch-lib.sh` — fixtures V1 à V6, ordre V7, cardinalité V8 | 112 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — troisième bras de `parse_dispatch_halt_status`, son doc-comment, son test V9 | 32 |
| `scripts/canonical-tokens.tsv` — une entrée classe B | 1 |

Total estimé : 265 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 265 < 279. La marge est de 14 lignes, donc serrée,
et trois propriétés mesurables la rendent réelle plutôt que comptable :

1. **Aucune primitive de mesure n'est écrite.** Les trois mesures de R2 sont des
   appels `git` d'une ligne chacun, et le motif
   `git fetch … refs/heads/<b>:refs/<ns>/<b>` puis `rev-parse <ref>:<path>` existe
   déjà, tourne en production et est testé : `_committed_plan_on_branch`
   (`dispatch-lib.sh:5020-5122`) le fait avec `refs/dispatch-gate/`, et
   `_plan_provenance` (`:4949-4965`) fait déjà la comparaison `branch_blob` vs
   `main_blob`. Ce vol en est un **appelant sur un second espace de refs**, pas un
   auteur.
2. **Le mécanisme de refus est repris tel quel, et c'est ce qui a été tranché
   avant d'estimer.** La bannière de halte + `exit 1` est en service à deux sites
   (`_branch_reuse_gate`, la porte de rebase), le trap `EXIT` la laisse passer
   verbatim dès que `RESULT` est posé, et le lecteur Rust est un `match` à ajouter
   d'un bras. L'alternative — un `auto_skipped` neuf — aurait demandé un
   discriminant de classe côté moteur pour éviter de résoudre la parente en
   `completed` ; elle est écartée par R5, pas reportée.
3. **Les fixtures réutilisent le harnais `FIXTURE_CLONE`.** Les ~20 cas de
   `_committed_plan_on_branch` (`test-dispatch-lib.sh:1377-1800`) posent déjà un
   dépôt git de fixture avec un `origin` local ; les six nouveaux cas ajoutent des
   branches à ce dépôt, pas un harnais.

Ce qui aurait fait sauter la borne, et qui est donc hors du vol : rendre
l'estampille de `_write_canonical_callout` mesurée. Elle touche la composition du
callout, son `awk` de remplacement borné au préambule, l'idempotence de
mika#2981, les fixtures de callout et la décision de ce qu'on écrit quand le plan
n'est pas dans `HEAD` — de l'ordre de 200 lignes à elle seule, soit ~465 au total.
D'où le découpage en vols, et non en phases d'un même vol : ce vol est mergeable
seul et rend les deux incidents mesurés non atteignables.

## Ordre de commit

1. **`fix(3121): le plan servi est un fait publié par son producteur`** —
   `_LAUNCHED_PLAN_PATH` posée dans `_detect_plan_on_branch` et remise à zéro dans
   `dispatch_claude_pilot`, sans lecteur. Aucun changement de comportement ; le
   vocabulaire se relit seul.
2. **`fix(3121): la bannière de halte d'un plan lancé divergent`** — le troisième
   bras de `parse_dispatch_halt_status`, son doc-comment, l'entrée
   `canonical-tokens.tsv` et le test V9. Le lecteur arrive avant l'écrivain, donc
   aucune fenêtre où une bannière remonterait non lue.
3. **`fix(3121): le plan lancé est confronté à ce qu'origin porte`** —
   `_confront_launch_plan`, ses cinq issues, sa ligne de recensement et son site
   d'appel. C'est le commit qui change le comportement, et il est seul à le faire.
4. **`test(3121): les cinq issues, l'ordre d'appel et la cardinalité des globales`**
   — V1 à V8.

**Plancher du vol.** Les commits 1 à 3 laissent un comportement correct mais non
prouvé, et le commit 2 seul laisse un lecteur sans écrivain — inoffensif, mais pas
une fermeture. Le commit 4 est ce qui rend la porte durable : sans V1 et V4, rien
ne sépare « la garde mesure l'héritage » de « la garde refuse tout plan », et
c'est la paire qu'un futur éditeur consultera avant de la toucher. Un vol qui
s'arrêterait après le 3 serait à reprendre, pas à compléter : la PR n'est
mergeable qu'avec les quatre.
