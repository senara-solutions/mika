# mika#3119 — le secours wip-rescue lit la consigne de non-fermeture et la halte déclarée

Branche : `fix/3119/dispatch-lib-le-secours-wip-rescue`
Type : fix
Ticket : senara-solutions/mika#3119 (sources : commentaires 40, 46, 49, 52, 57 ; 47 en additif du parking #2983)

## Contexte

Le secours post-vol de `dispatch-lib.sh` (classes `dirty-worktree` mika#1282,
`commit-pushed-no-pr` mika#1396, `no-shipping-tail` mika#2492) produit deux
artefacts : un push (`_push_branch`, site de push unique, mika#1407) et une PR
draft dont le corps est composé par `_compose_rescue_pr_body`. Les deux sont
aujourd'hui **inconditionnels sur deux axes que le ticket mesure**.

### Défaut A — `Closes #N` écrit sans lire le corps du ticket

`skills/bundled/_shared/dispatch-lib.sh:13292-13294`, dans
`_compose_rescue_pr_body` (ouverte en 13254) :

```sh
if _rescue_diff_carries_work "$wt_dir"; then
    diff_marker="carries-work"
    issue_ref="Closes #${issue_num}"
else
    diff_marker="incident-only"
    issue_ref="Refs #${issue_num}"
    lede="> **This recovery carries no fix.** …"
fi
```

Le seul discriminant est la **forme du diff**. Le corps du ticket n'est jamais
lu. Sur un ticket-parapluie à plusieurs vols — #1727 en est le cas mesuré — le
corps déclare explicitement que ses PR portent `Refs`, et aucun `Closes`,
`Fixes` ni `Resolves`. Le secours écrit `Closes #1727` quand même.

Population mesurée (corrigée à la main chaque fois) : **#2977, #3069, #3077,
#3081, #3092, #3100, #3108** — sept occurrences. Sur #3108, le champ GraphQL
`closingIssuesReferences` renvoyait bien #1727 : la fermeture n'était pas
hypothétique, elle était armée.

L'asymétrie des coûts décide du sens du fail-closed :

| erreur | coût |
|---|---|
| `Refs` écrit à tort | l'opérateur ferme le ticket à la main — un geste, visible, réversible |
| `Closes` écrit à tort | le merge ferme un parapluie à plusieurs vols de sa fin — mesuré 7 fois |

C'est le même arbitrage que mika#2157, qui fait déjà écrire `Refs` au composeur
sur un diff `incident-only`.

### Défaut B — une halte déclarée par le pilote est invisible au secours

Les têtes de ticket portent une doctrine de halte : *« HALTE borne : N lignes …
pour `M` »* ⇒ **pas de commit supplémentaire, pas de push, pas de PR**, et
*« une halte ne se lève jamais par un amendement du plan »*. Un pilote qui
déclare cette halte finit son tour proprement, donc sans queue de livraison —
ce qui est exactement la signature de la classe `no-shipping-tail` (mika#2492),
le chemin **nominal** du `/ce-work` sous les queues allégées de mika#2694. Le
secours ne distingue pas les deux : il pousse et ouvre une draft.

Mesuré sur **#3081, #3092, #3100, #3108**. Effet secondaire sur #3081 :
`_measure_pipeline_verified` a répondu `rescue-pipeline-verified: yes`, et le
démon `auto_resume_wip_rescue_drafts` (mika#1852) a dé-drafté la PR en revue —
le travail halté est parti en revue sans que personne ne l'ait décidé.

Pourquoi le fait est invisible : `_run_claude_pilot` (6480) ne lit du JSON de
claude-pilot que `status`, `session_id`, `turns`, `subtype`,
`termination_reason`, `api_error_status`, `cost_usd`, `duration_ms`
(6581-6612). **Aucun champ ne porte le dernier message du pilote.** La
déclaration de halte n'existe que dans le log de session, que dispatch-lib
adresse déjà sous `_SESSION_LOG_PATH` (posé en 6568, gardé en 6569-6572).

Deux logs mesurés, qui donnent la forme du prédicat :

| log | règle citée (tête) | déclaration (queue) | distance à EOF |
|---|---|---|---|
| `4492a670-…` (9654 l.) | l. 100 | l. 9630 — `**HALTE borne : 379 lignes ajoutées pour \`310\`**` | 24 l. |
| `5813f896-…` (7294 l.) | l. 88 | l. 7268 — `**HALTE borne : 455 lignes pour 450.**` | 26 l. |

Un `grep "HALTE borne"` nu rend donc un **faux positif garanti** : la règle est
citée dans le prompt, près de la tête, avec le littéral `N`. Classe de faux
positif déjà rencontrée (cité-vs-déclaré, commentaire de parking 41,
`completion_claim`). Le prédicat doit être ancré sur deux propriétés
indépendantes : des **chiffres** après `HALTE borne :`, et une **fenêtre de
queue**.

## Ce que la mesure contredit dans le corps du ticket

Le corps affirme comme un fait : *« Le corps de #3092 atteste fmt, clippy et
verify-pipeline verts sans qu'ils aient tourné. »* La mesure ne le soutient
pas, et il ne faut pas « corriger » un défaut qui n'existe pas :

- la source de l'affirmation (commentaire 49) dit elle-même *« Non vérifié :
  soit dispatch-lib l'a lancé, soit c'est un gabarit »* ;
- `_measure_pipeline_verified` (13007) est **armée par défaut**
  (`_rescue_verify_enabled`, 13134) et exécute réellement ses six termes —
  0 compound-traversal, 1 `_rescue_diff_carries_work`, 2 `git status --porcelain`
  sur `RESCUE_EXCLUDE_PATHSPEC`, 3 `cargo fmt --all --check`,
  4 `cargo clippy --workspace --all-targets -- -D warnings`,
  5 `./scripts/verify-pipeline.sh origin/main` — en **fail-closed**, sous budget
  (`_rescue_verify_budget_secs`, 900 s par défaut ; rc 124 ⇒ terme `budget`).

Donc `rescue-pipeline-verified: yes` sur #3092 est une **attestation produite
par une exécution réelle**, pas un gabarit. Ce vol ne touche pas à ce
mécanisme ; il en modifie seulement la **conséquence** quand une halte est
déclarée (§ Conception, consommateur 2).

## Conception

Deux prédicats neufs, chacun avec un site de lecture unique, et trois
consommateurs.

### Prédicat 1 — `_issue_forbids_closing_ref`

Entrée : le corps du ticket. Il est déjà global :
`ISSUE_BODY=$(printf '%s' "$ISSUE_JSON" | jq -r '.body // empty')`, posé en 5845
dans `_set_up_worktree`, donc visible depuis le bloc de secours — aucune requête
`gh` nouvelle, aucun tour réseau.

Sortie, trois valeurs — la troisième existe parce que « je n'ai pas pu lire » et
« le corps n'interdit rien » ne sont pas la même chose (classe mika#2205) :

| valeur | condition | mot-clé émis |
|---|---|---|
| `refs-only` | une **même ligne** du corps porte une négation (`aucun`, `pas de`, `jamais de`, `ni`, `no`) **et** au moins un des trois mots-clés (`Closes`, `Fixes`, `Resolves`), insensible à la casse | `Refs` |
| `closing` | corps lisible, aucune ligne de cette forme | `Closes` |
| `undetermined` | `ISSUE_BODY` vide ou absent | `Refs` |

La conjonction est **par ligne** et non sur le corps entier : une négation en
tête et un `Closes` quatre-vingts lignes plus bas ne doivent pas se rencontrer.

`undetermined` retombe sur `Refs` par l'asymétrie du § Défaut A, et sans
exception — un corps illisible est une population déjà anormale, où un geste
opérateur coûte moins qu'un parapluie fermé.

### Prédicat 2 — `_pilot_declared_halt`

Entrée : `_SESSION_LOG_PATH`. Conjonction de deux ancrages indépendants :

1. une ligne matchant `HALTE borne[[:space:]]*:[[:space:]]*[0-9]` — des
   **chiffres**, ce que la règle citée ne porte jamais (elle porte `N`) ;
2. cette ligne tombe dans la **fenêtre de queue** : les 120 dernières lignes du
   log. Les deux déclarations mesurées sont à 24 et 26 lignes de l'EOF ; les
   citations de règle à 88 et 100 lignes de la **tête**, sur des logs de 7 294 et
   9 654 lignes — plusieurs ordres de grandeur de marge dans les deux sens.

Fail-closed vers **« pas de halte »** : log absent, vide ou illisible ⇒ le
secours nominal s'applique. Un détecteur de halte qui se trompe dans l'autre
sens retiendrait un travail livrable sur une panne de bind de log — panne déjà
nommée et instrumentée (`pilot_log_guard.missing`, 6571).

### Consommateur 1 — `_compose_rescue_pr_body` prend un neuvième argument

Signature actuelle : `$1` répertoire de worktree, `$2` classe de secours,
`$3` fait de classe, `$4` numéro de ticket, `$5` vérifié, `$6` terme en échec,
`$7` extrait, `$8` traversée composée. Le `$9` porte la sortie du prédicat 1.

**Invariant AC4 conservé tel quel** : la fonction porte un invariant
byte-identique pour le repli `MIKA_RESCUE_VERIFY_ENABLED=0`. `$9` **absent** doit
donc rendre le corps pré-correctif bit pour bit — `Closes` sur `carries-work`,
et aucun marqueur neuf. L'absence est le défaut, jamais `closing`.

Un marqueur HTML neuf rend le fait lisible sur l'artefact, puisque le corps de
PR est la seule surface durable d'un dispatch réussi (Signal M / mika#2050) :

```html
<!-- rescue-closing-ref: refs-only|closing|undetermined -->
```

Il n'est émis que lorsque `$9` est non vide, ce qui est exactement ce que
l'invariant AC4 exige.

### Consommateur 2 — le refus structurel sur halte déclarée

Dans le bloc Unit 2 du secours (14380-14720, à l'intérieur de
`dispatch_claude_pilot`, 13921), à côté des deux refus existants —
`rescue_adds_nothing_to_main` (mika#3038) et `rescue_scratch_only`
(mika#2503), dont la forme est le précédent exact :

- `_recovery_pr_due=0` — aucune draft ouverte ;
- `_set_pr_status_line "NO_PR: pilot_halt_declared"` — un seul `PR:`/`NO_PR:`
  par callback (`_set_pr_status_line`), motif en snake_case, classe B de
  `scripts/canonical-tokens.tsv` ;
- `_set_outcome_line "Outcome: PIPELINE_INCOMPLETE — pilot_halt_declared"` —
  une seule ligne `Outcome:` par callback (`_set_outcome_line`) ;
- dans le RESULT, la **commande exacte de publication manuelle** (push puis
  `gh pr create --draft`), parce que le refus doit être réversible d'un geste
  nommé.

`_measure_pipeline_verified` n'est pas appelée sur cette branche : sans draft,
il n'y a pas de corps à marquer, et le démon mika#1852 n'a rien à dé-drafter.
C'est ce qui ferme l'effet secondaire de #3081.

### Consommateur 3 — la garde de push

Le ticket (titre, commentaire 57) demande **« ni push ni PR »**. La doctrine du
fichier dit l'inverse pour le push : *rien n'est perdu*, et les deux refus
existants ne retiennent que la draft. Les deux lectures sont incompatibles et le
plan tranche, en nommant le coût plutôt qu'en le taisant.

**Tranché : les deux abstentions.** `_push_branch` (8944) rend `0` en tête
quand la halte est déclarée, avant sa décision sur `origin/$BRANCH..HEAD`, et
émet une ligne WARN nommée. Raison : le push est ce qui arme le démon mika#1852
et ce qui rend le travail halté indiscernable d'un travail livrable pour tout
consommateur en aval. Retenir la draft sans retenir le push laisse une branche
poussée qu'aucun artefact ne qualifie.

**Coût nommé** : le travail halté reste local au worktree de dispatch. Il
survit tant que le worktree survit, et il est perdu si le worktree est nettoyé
avant que l'opérateur n'agisse. C'est pour cela que le RESULT porte les deux
commandes de publication, dans l'ordre, et que la ligne WARN est nommée — le
refus est observable et réversible, il n'est pas silencieux.

Le lint `scripts/check-pilot-push-sites.sh` reste satisfait : aucun site de push
neuf n'est introduit, le site unique rend seulement plus tôt.

### Consommateur 4 — l'énumération Rust

`crates/mika-agent/src/task_engine/dispatcher.rs` : `parse_no_pr_reason`
(~6523) accepte déjà tout `[a-z_]+` via `RE_NO_PR = (?m)^NO_PR:\s+([a-z_]+)`,
donc le motif neuf est parsé sans changement de code. Mais son commentaire de
doc (6505-6517) dit de lui-même qu'il est *« la seule énumération qu'un
opérateur lit »* : un motif accepté par le parseur et nommé nulle part est un
motif que personne ne peut chercher — `rescue_scratch_only` l'a été jusqu'à ce
que mika#3038 ajoute son voisin et découvre le trou. `pilot_halt_declared` est
donc ajouté **dans le même commit** à ce commentaire et au tableau de
`test_parse_no_pr_reason_known_reasons` (7917-7935, sept motifs aujourd'hui).

### Pas d'interrupteur d'environnement, et c'est un refus argumenté

Aucune variable `MIKA_*` neuve. Une variable coûte sa déclaration, sa ligne dans
`docs/operator/environment-variables.md` et son injection côté
`executor.rs`, faute de quoi elle est décorative (mika#2165) — et les deux refus
précédents du même bloc (mika#2503, mika#3038) n'en portent aucune. Ici
l'interrupteur **est la commande** : le RESULT nomme le push et le
`gh pr create` qui lèvent le refus en deux gestes.

## Tests

### Extension de `skills/bundled/_shared/tests/test_rescue_closes_guard.sh`

La suite existante (mika#2157) est l'hôte naturel : elle construit de vrais
dépôts git temporaires avec un vrai `origin/main`, source `../dispatch-lib.sh`,
et porte déjà `assert_contains` / `assert_not_contains` / `assert_eq`. Cas
ajoutés :

1. corps portant la consigne ⇒ `Refs #N`, aucun `Closes`/`Fixes`/`Resolves`,
   marqueur `rescue-closing-ref: refs-only` ;
2. corps ordinaire sur diff `carries-work` ⇒ `Closes #N`, marqueur `closing` —
   le **contrôle négatif** qui sépare « le prédicat lit le corps » de « le
   prédicat écrit toujours `Refs` » ;
3. `ISSUE_BODY` vide ⇒ `Refs #N`, marqueur `undetermined` ;
4. négation et mot-clé sur **deux lignes distinctes** ⇒ `closing` (la
   conjonction est par ligne) ;
5. **invariant AC4** : `$9` absent ⇒ corps byte-identique au corps
   pré-correctif, et aucune occurrence de `rescue-closing-ref`.

### Nouvelle suite `skills/bundled/_shared/tests/test_pilot_halt_declared.sh`

1. déclaration chiffrée en queue ⇒ halte (fixture réduite du log
   `4492a670-…`, sous `.pilot-scratch/` pour la préparation, figée dans la
   suite) ;
2. règle citée en tête avec le littéral `N`, **sans** déclaration ⇒ pas de
   halte — le contre-témoin du faux positif mesuré ;
3. les deux ensemble (règle en tête **et** déclaration en queue) ⇒ halte
   (fixture réduite du log `5813f896-…`) ;
4. déclaration présente mais **hors** fenêtre de queue ⇒ pas de halte ;
5. log absent, puis log vide ⇒ pas de halte (fail-closed dans le bon sens) ;
6. sur halte : `_recovery_pr_due` à `0`, exactement une ligne
   `NO_PR: pilot_halt_declared`, exactement une ligne `Outcome:`, et
   `_push_branch` rendant sans pousser.

### Câblage CI — obligatoire, une ligne

`Makefile:145-167` énumère chaque `_shared/tests/*.sh` (l. 150 la garde
`closes`, l. 151 `pipeline verified`). La suite neuve y est ajoutée, sinon
`suite-wiring-lint` (`scripts/check-suite-wiring.sh` + `scripts/suite-wiring.tsv`)
refuse la PR : une garde que le pipeline ne lance jamais ne refuse rien.

### Rust

`test_parse_no_pr_reason_known_reasons` passe de sept à huit motifs.

## Ordre de commit

Le vol est interruptible à chaque palier, et chaque palier laisse l'arbre vert :

1. `_issue_forbids_closing_ref` + `$9` de `_compose_rescue_pr_body` + les cinq
   cas de `test_rescue_closes_guard.sh` — ferme le défaut A, mesuré sept fois ;
2. `_pilot_declared_halt` + la suite neuve + sa ligne de `Makefile` — le
   détecteur, armé, avec témoin et contre-témoin ;
3. le refus dans le bloc Unit 2 + la garde de `_push_branch` ;
4. `dispatcher.rs` (commentaire de doc + test) — même commit que le palier 3,
   le motif et son énumération ne se séparent pas.

## Risques

- **Faux positif du prédicat 2 sur un pilote qui *cite* la règle en queue de
  log.** Couvert par l'ancrage sur les chiffres : la règle citée porte `N`.
  Le cas 2 de la suite neuve est ce contrôle.
- **Faux positif du prédicat 1** sur un corps disant « ne pas oublier
  `Closes` ». Rend `Refs`, donc coûte un clic — accepté par l'asymétrie.
- **Perte de travail halté** si le worktree est nettoyé avant l'opérateur.
  Nommé au § Consommateur 3, mitigé par les commandes dans le RESULT.
- **Régression de l'invariant AC4** si `$9` prend une valeur par défaut au
  lieu de rester absent. Tenu par le cas 5 de la suite étendue.

## Definition of Done

- `_issue_forbids_closing_ref` et `_pilot_declared_halt` existent dans
  `skills/bundled/_shared/dispatch-lib.sh`, chacune avec un commentaire d'en-tête
  nommant sa source de données, son sens de fail-closed et le ticket.
- `_compose_rescue_pr_body` accepte `$9` et émet `rescue-closing-ref` ;
  l'invariant byte-identique sous `$9` absent est asserté.
- Le bloc Unit 2 porte un troisième refus structurel : `_recovery_pr_due=0`,
  `NO_PR: pilot_halt_declared`, `Outcome: PIPELINE_INCOMPLETE —
  pilot_halt_declared`, et les commandes de publication manuelle dans le RESULT.
- `_push_branch` rend sans pousser quand la halte est déclarée, sous une ligne
  WARN nommée.
- `pilot_halt_declared` est nommé dans le commentaire de doc de
  `parse_no_pr_reason` **et** dans `test_parse_no_pr_reason_known_reasons`.
- `test_rescue_closes_guard.sh` porte les cinq cas du § Tests ;
  `test_pilot_halt_declared.sh` existe, porte ses six cas, et est câblé dans
  `Makefile`.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, la suite complète et `scripts/verify-pipeline.sh origin/main`
  passent. `suite-wiring-lint`, `check-canonical-tokens.sh` et
  `check-pilot-push-sites.sh` sont verts.
- Le corps de la PR porte `Refs mika#3119` — pas `Closes` — tant que #3119
  n'est pas le dernier vol de son périmètre, et
  `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés des quatre constats mesurés qu'il énumère.

- **AC1** — Sur un ticket dont le corps interdit les mots-clés de fermeture, le
  corps de PR produit par le secours porte `Refs #N` et **aucune** occurrence de
  `Closes`, `Fixes` ou `Resolves`, y compris sur un diff `carries-work`.
  Attesté par le cas 1 de `test_rescue_closes_guard.sh`.
- **AC2** — Sur un ticket dont le corps n'interdit rien, le comportement
  nominal `Closes #N` est conservé sur un diff `carries-work`. Attesté par le
  cas 2 (contrôle négatif).
- **AC3** — Un corps de ticket vide ou illisible rend `Refs #N` et un marqueur
  `rescue-closing-ref: undetermined` ; l'indétermination est lisible sur
  l'artefact et ne se confond pas avec « le corps n'interdit rien ». Attesté par
  le cas 3.
- **AC4** — Sous `$9` absent (repli `MIKA_RESCUE_VERIFY_ENABLED=0`), le corps
  produit est **byte-identique** au corps pré-correctif et ne contient aucun
  `rescue-closing-ref`. Attesté par le cas 5.
- **AC5** — Un log de session dont la queue porte une déclaration de halte
  chiffrée fait rendre « halte » au prédicat ; un log qui ne porte que la règle
  citée en tête, avec le littéral `N`, fait rendre « pas de halte ». Attesté par
  les cas 1 à 4 de `test_pilot_halt_declared.sh`.
- **AC6** — Halte déclarée ⇒ aucune PR draft ouverte, exactement une ligne
  `NO_PR: pilot_halt_declared`, exactement une ligne `Outcome:`, et aucun push.
  Attesté par le cas 6.
- **AC7** — Log de session absent, vide ou illisible ⇒ le secours nominal
  s'applique (push et draft), sans halte inférée. Attesté par le cas 5.
- **AC8** — `pilot_halt_declared` est nommé dans le commentaire de doc de
  `parse_no_pr_reason` et dans `test_parse_no_pr_reason_known_reasons`, dans le
  même commit que le site qui l'émet.
- **AC9** — `test_pilot_halt_declared.sh` est atteint par un workflow, via le
  `Makefile` ; `suite-wiring-lint` est vert sans entrée neuve dans
  `scripts/suite-wiring.tsv`.
- **AC10** — Le RESULT d'un secours halté nomme les deux commandes qui lèvent
  le refus (push, puis `gh pr create --draft`), dans l'ordre.

## Fire-Disposition

Le vol livre des détecteurs : `_pilot_declared_halt` (garde qui retient le push
et la draft), `_issue_forbids_closing_ref` (garde sur le mot-clé de fermeture),
et les deux suites de test.

**Option retenue : (a) exception nommée en allowlist — allowlist vide par
construction, sans exception.**

Les deux prédicats portent sur la **sortie d'une session future** (le log de
session) et sur le **corps d'un ticket distant**, jamais sur un artefact
versionné du dépôt. Il n'existe donc aucune population existante du dépôt qui
les violerait, et l'allowlist atterrit vide — ce n'est pas une dérogation
accordée, c'est l'absence d'objet.

Ce qui remplace l'assertion auto-nettoyante d'une allowlist non vide, puisqu'un
détecteur armé sans violation existante doit quand même prouver qu'il peut
rougir, c'est la **paire témoin / contre-témoin**, exigée dans les deux
suites :

| suite | témoin (le détecteur rougit) | contre-témoin (il ne rougit pas) |
|---|---|---|
| `test_pilot_halt_declared.sh` | cas 1 et 3 — fixtures réduites des deux logs réels mesurés, déclaration chiffrée en queue | cas 2 — règle citée en tête avec le littéral `N` ; cas 4 — déclaration hors fenêtre ; cas 5 — log absent ou vide |
| `test_rescue_closes_guard.sh` | cas 1 — corps portant la consigne | cas 2 — corps ordinaire ; cas 4 — négation et mot-clé sur deux lignes |

Les deux détecteurs atterrissent **armés** : aucun `#[ignore]`, aucun
`MIKA_*` d'armement, aucun suivi différé. L'option (b) est écartée parce qu'un
détecteur désarmé ne refuserait pas le huitième `Closes #1727` ; l'option (c)
est écartée parce que l'arbitrage qui aurait justifié une remontée — « push ou
pas push sur halte » — est tranché dans ce plan, avec son coût nommé.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — `_pilot_declared_halt` + en-tête | 60 |
| `skills/bundled/_shared/dispatch-lib.sh` — `_issue_forbids_closing_ref` + en-tête | 50 |
| `skills/bundled/_shared/dispatch-lib.sh` — `_compose_rescue_pr_body` (`$9` + marqueur) | 35 |
| `skills/bundled/_shared/dispatch-lib.sh` — garde dans `_push_branch` | 25 |
| `skills/bundled/_shared/dispatch-lib.sh` — refus dans le bloc Unit 2 | 40 |
| `skills/bundled/_shared/tests/test_rescue_closes_guard.sh` (5 cas ajoutés) | 75 |
| `skills/bundled/_shared/tests/test_pilot_halt_declared.sh` (nouvelle suite, 6 cas) | 120 |
| `Makefile` (câblage de la suite) | 1 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` (doc + test) | 8 |

Total estimé : 414 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et pourquoi il tient.** 414 est sous la borne de 429 et très
en dessous du seuil par PR de 1000. La marge sur la borne par vol est mince
(15 lignes), et c'est l'ordre de commit qui l'absorbe plutôt qu'un découpage
artificiel : le palier 1 (170 lignes, défaut A — le mal mesuré sept fois) est
mergeable seul et laisse l'arbre vert, donc un vol qui approcherait du plafond
livre d'abord ce qui a été mesuré sept fois et renvoie la halte à un vol suivant
sans rien casser. Aucun découpage en vols n'est déclaré ici parce qu'aucun n'est
requis : le total passe sous les deux bornes.

Les lignes de `dispatch-lib.sh` sont estimées à la densité de commentaire du
fichier (un en-tête de fonction y fait couramment 30 à 60 lignes de prose
d'ancrage) ; c'est ce qui explique un total de 210 lignes pour cinq
interventions dont aucune ne dépasse 15 lignes de logique.
