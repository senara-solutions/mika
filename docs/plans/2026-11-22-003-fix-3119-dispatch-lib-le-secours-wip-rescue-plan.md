# mika#3119 — le secours wip-rescue lit la consigne de non-fermeture du ticket

Branche : `fix/3119/dispatch-lib-le-secours-wip-rescue`
Type : fix
Ticket : senara-solutions/mika#3119 (sources : commentaires 40, 46, 49, 52, 57 ; 47 en additif du parking #2983)

**Périmètre de ce vol : le défaut A seul.** Le ticket porte deux défauts
indépendants ; la mesure du § Taille estimée les met ensemble à ~900 lignes, soit
plus du double de la borne par vol. Le défaut B est renvoyé au vol 2, nommé au
§ Vol 2 — hors périmètre.

## Contexte

Le secours post-vol de `dispatch-lib.sh` (classes `dirty-worktree` mika#1282,
`commit-pushed-no-pr` mika#1396, `no-shipping-tail` mika#2492) produit deux
artefacts : un push (`_push_branch`, site de push unique, mika#1407) et une PR
draft dont le corps est composé par `_compose_rescue_pr_body`. Les deux sont
aujourd'hui inconditionnels sur deux axes que le ticket mesure.

### Défaut A — `Closes #N` écrit sans lire le corps du ticket (CE VOL)

`skills/bundled/_shared/dispatch-lib.sh:13294` et `:13297`, dans
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

### Défaut B — une halte déclarée est invisible au secours (VOL 2, hors périmètre)

Résumé conservé ici parce qu'il justifie le découpage et parce que le vol 2 doit
pouvoir être groomé depuis ce texte sans re-mesurer les logs.

Les têtes de ticket portent une doctrine de halte : *« HALTE borne : N lignes …
pour `M` »* ⇒ pas de commit supplémentaire, pas de push, pas de PR. Un pilote
qui déclare cette halte finit son tour proprement, donc sans queue de livraison
— exactement la signature de la classe `no-shipping-tail` (mika#2492), le chemin
**nominal** du `/ce-work` sous les queues allégées de mika#2694. Le secours ne
distingue pas les deux : il pousse et ouvre une draft.

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
citée dans le prompt, près de la tête, avec le littéral `N`. Le prédicat du vol 2
devra être ancré sur deux propriétés indépendantes — des **chiffres** après
`HALTE borne :`, et une **fenêtre de queue**.

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
par une exécution réelle**, pas un gabarit. Ce vol ne touche pas à ce mécanisme.

## Conception

Un prédicat neuf, un site de lecture unique, un consommateur.

### Prédicat — `_issue_forbids_closing_ref`

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

### Consommateur — `_compose_rescue_pr_body` prend un neuvième argument

Signature actuelle : `$1` répertoire de worktree, `$2` classe de secours,
`$3` fait de classe, `$4` numéro de ticket, `$5` vérifié, `$6` terme en échec,
`$7` extrait, `$8` traversée composée. Le `$9` porte la sortie du prédicat.
Le site d'appel unique est `dispatch-lib.sh:14636`.

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

### Pas d'interrupteur d'environnement, et c'est un refus argumenté

Aucune variable `MIKA_*` neuve. Une variable coûte sa déclaration, sa ligne dans
`docs/operator/environment-variables.md` et son injection côté `executor.rs`,
faute de quoi elle est décorative (mika#2165) — et les deux refus du même bloc
(mika#2503, mika#3038) n'en portent aucune. Ici le repli existe déjà :
`MIKA_RESCUE_VERIFY_ENABLED=0` rend `$9` absent, donc le comportement
pré-correctif, par l'invariant AC4.

### Aucune entrée neuve dans `scripts/canonical-tokens.tsv`

Vérifié : le registre déclare le **jeton** `NO_PR` (l. 376, classe B, site
`dispatcher.rs::RE_NO_PR`) et `PIPELINE_INCOMPLETE` (l. 359, 365), jamais les
motifs individuels — ni `rescue_scratch_only`, ni `rescue_adds_nothing_to_main`
n'y figurent. Ce vol n'émet aucun motif `NO_PR` neuf et n'introduit aucun jeton
machine : **rien à déclarer**. Une entrée ajoutée « par prudence » ferait rougir
le scan d'exhaustivité `canonical_tokens::tests::mika2201_every_match_site_is_declared`.

## Tests

### Extension de `skills/bundled/_shared/tests/test_rescue_closes_guard.sh`

La suite existante (mika#2157, 234 lignes, 31 assertions) est l'hôte naturel :
elle construit de vrais dépôts git temporaires avec un vrai `origin/main`, source
`../dispatch-lib.sh`, et porte déjà `assert_contains` / `assert_not_contains` /
`assert_eq`. Cas ajoutés :

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

**Hermétisme (mika#2793)** : tout dépôt fixture qui commite déclare son propre
`user.email` / `user.name` sur le dépôt, jamais par héritage de la configuration
globale de l'hôte — c'est la non-hermétique que mika#2793 a dû réparer dans
`scripts/verify-pipeline-test.sh`, et le job CI qui héberge cette suite l'exige
explicitement en commentaire.

### Câblage CI — rien à faire, et c'est vérifié

`test_rescue_closes_guard.sh` est **déjà** atteint par un workflow :
`.github/workflows/ci.yml:741`, step *« A rescue draft carries no Closes line
(mika#2157) »*, dans le job `dispatch-lib-offline-suites`. Les cas ajoutés sont
donc câblés par construction, et `suite-wiring-lint` reste vert sans entrée neuve
dans `scripts/suite-wiring.tsv`.

**Correction d'un fait que le grooming précédent avait faux**, consignée ici
parce que le vol 2 marchera dedans : la ligne de la target `test:` du `Makefile`
(l. 150) **ne câble rien au CI**. Aucun workflow n'appelle `make test` ; le CI
nomme des targets dédiées une par une, ou invoque `bash <chemin>` directement.
`ci.yml:200-204` le dit de lui-même : *« this suite was in `make test` — a local
target CI does not call — so its guards gated nothing on a PR »* (mika#2578,
puis mika#2752 deux fois). Pour une suite offline de dispatch-lib, le câblage
canonique est un **step dans le job `dispatch-lib-offline-suites`** — la forme de
son voisin `test_rescue_tree_equals_main.sh` (mika#3038, `ci.yml:755`).

## Ordre de commit

Le vol est interruptible à chaque palier, et chaque palier laisse l'arbre vert :

1. `_issue_forbids_closing_ref` + son en-tête d'ancrage — la fonction seule,
   sans consommateur, donc sans changement de comportement observable ;
2. `$9` de `_compose_rescue_pr_body` + le marqueur + le site d'appel 14636 ;
3. les cinq cas de `test_rescue_closes_guard.sh`, dont le contrôle négatif et
   l'invariant AC4.

Le plancher du vol est le palier 2 : en deçà, le défaut A reste entier.

## Risques

- **Faux positif du prédicat** sur un corps disant « ne pas oublier `Closes` ».
  Rend `Refs`, donc coûte un clic — accepté par l'asymétrie du § Défaut A.
- **Régression de l'invariant AC4** si `$9` prend une valeur par défaut au
  lieu de rester absent. Tenu par le cas 5.
- **Sous-estimation de la suite.** La base fait 234 lignes pour ~8 cas ; le cas 5
  coûte plus qu'un cas ordinaire (il construit deux corps et les compare octet à
  octet). L'estimation à 150 lignes porte déjà cette prime ; si elle est dépassée,
  le palier 3 se scinde par cas plutôt que de déborder le plafond.

## Vol 2 — ce que ce vol NE livre PAS

Le défaut B (§ Contexte) est renvoyé entier. Périmètre attendu, re-mesuré à
**~660 lignes** — donc lui-même **au-dessus de la borne par vol de 429**, et à
découper à son tour lors de son grooming (détecteur armé + suite d'abord ; refus,
garde de push et énumération Rust ensuite) :

- `_pilot_declared_halt` — prédicat sur `_SESSION_LOG_PATH`, conjonction de deux
  ancrages (chiffres après `HALTE borne :`, fenêtre des 120 dernières lignes),
  fail-closed vers « pas de halte » ;
- le troisième refus structurel du bloc Unit 2 (14380-14720), à côté de
  `rescue_adds_nothing_to_main` (14464) et `rescue_scratch_only` (14513) :
  `_recovery_pr_due=0`, `NO_PR: pilot_halt_declared`,
  `Outcome: PIPELINE_INCOMPLETE — pilot_halt_declared`, et les commandes de
  publication manuelle dans le RESULT ;
- la garde de `_push_branch` (8944) — **l'arbitrage reste ouvert** : le ticket
  demande « ni push ni PR », la doctrine du fichier dit *rien n'est perdu* et les
  deux refus existants ne retiennent que la draft. Le vol 2 doit trancher en
  nommant le coût (travail halté local au worktree, perdu si le worktree est
  nettoyé) ;
- `parse_no_pr_reason` (`dispatcher.rs:6522`) : le regex `[a-z_]+` accepte déjà
  le motif, mais son commentaire de doc (6505-6517) se dit *« la seule énumération
  qu'un opérateur lit »*. `pilot_halt_declared` va donc dans ce commentaire **et**
  dans `test_parse_no_pr_reason_known_reasons` (7916, sept motifs aujourd'hui),
  dans le même commit que le site qui l'émet ;
- `test_pilot_halt_declared.sh` + son **step dans `dispatch-lib-offline-suites`**
  (§ Câblage CI ci-dessus), sans quoi `suite-wiring-lint` refuse la PR.

La spécification longue de ce vol, prête à servir de corps de ticket, est écrite
dans `.pilot-scratch/followup-body.md` sur cette branche.

Tracked in: senara-solutions/mika#3119 — ce ticket reste le tracker du vol 2.
La PR de ce vol porte `Refs mika#3119` et non `Closes` (§ Definition of Done),
donc #3119 ne se ferme pas à son merge. L'ouverture d'un ticket dédié au vol 2
n'a pas pu être faite depuis cette session : `gh issue create` est refusé en
session dispatchée (`handled by mika-issue skill -- direct CLI use denied`),
y compris via la skill. Si l'opérateur préfère un tracker séparé, le corps est
celui du fichier ci-dessus et cette ligne doit alors le nommer.

## Definition of Done

- `_issue_forbids_closing_ref` existe dans
  `skills/bundled/_shared/dispatch-lib.sh`, avec un commentaire d'en-tête nommant
  sa source de données (`ISSUE_BODY`, posé en 5845), son sens de fail-closed et le
  ticket.
- `_compose_rescue_pr_body` accepte `$9` et émet `rescue-closing-ref` ;
  l'invariant byte-identique sous `$9` absent est asserté.
- Le site d'appel `dispatch-lib.sh:14636` passe le neuvième argument.
- `test_rescue_closes_guard.sh` porte les cinq cas du § Tests, et tout dépôt
  fixture qui commite déclare sa propre identité git.
- Aucune entrée neuve dans `scripts/canonical-tokens.tsv`, aucune variable
  `MIKA_*` neuve, aucun site de push neuf.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, la suite complète et `scripts/verify-pipeline.sh origin/main`
  passent. `suite-wiring-lint`, `check-canonical-tokens.sh` et
  `check-pilot-push-sites.sh` sont verts.
- Le corps de la PR porte `Refs mika#3119` — pas `Closes`, puisque le vol 2 reste
  à livrer sur ce ticket — et `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés du défaut A, seul périmètre de ce vol.

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
- **AC5** — La conjonction du prédicat est **par ligne** : une négation et un
  mot-clé de fermeture situés sur deux lignes distinctes rendent `closing`.
  Attesté par le cas 4.
- **AC6** — Les cas ajoutés sont atteints par un workflow sans modification de
  câblage : le step `ci.yml:741` les exécute, et `suite-wiring-lint` est vert sans
  entrée neuve dans `scripts/suite-wiring.tsv`.
- **AC7** — Le défaut B n'est pas livré, et la suite est nommée : le plan porte
  un § Vol 2 et une ligne `Tracked in:`.

## Fire-Disposition

Le vol livre un détecteur : `_issue_forbids_closing_ref`, garde sur le mot-clé de
fermeture, plus les cinq cas de test qui l'attestent.

**Option retenue : (a) exception nommée en allowlist — allowlist vide par
construction, sans exception.**

Le prédicat porte sur le **corps d'un ticket distant**, jamais sur un artefact
versionné du dépôt. Il n'existe donc aucune population existante du dépôt qui le
violerait, et l'allowlist atterrit vide — ce n'est pas une dérogation accordée,
c'est l'absence d'objet.

Ce qui remplace l'assertion auto-nettoyante d'une allowlist non vide, puisqu'un
détecteur armé sans violation existante doit quand même prouver qu'il peut
rougir, c'est la **paire témoin / contre-témoin**, exigée dans la suite :

| témoin (le détecteur rougit) | contre-témoin (il ne rougit pas) |
|---|---|
| cas 1 — corps portant la consigne, sur un diff `carries-work` (la combinaison exacte des sept occurrences mesurées) | cas 2 — corps ordinaire ⇒ `Closes` conservé ; cas 4 — négation et mot-clé sur deux lignes ⇒ `closing` ; cas 5 — `$9` absent ⇒ corps pré-correctif inchangé |

Le détecteur atterrit **armé** : aucun `#[ignore]`, aucun `MIKA_*` d'armement,
aucun suivi différé. L'option (b) est écartée parce qu'un détecteur désarmé ne
refuserait pas le huitième `Closes #1727`. L'option (c) est écartée parce que
l'arbitrage qui aurait justifié une remontée — « push ou pas push sur halte » —
appartient au vol 2 et y est explicitement laissé ouvert (§ Vol 2) : ce vol ne
touche ni `_push_branch` ni le bloc Unit 2.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — `_issue_forbids_closing_ref` + en-tête | 70 |
| `skills/bundled/_shared/dispatch-lib.sh` — `_compose_rescue_pr_body` (`$9`, marqueur, doc) + site d'appel 14636 | 40 |
| `skills/bundled/_shared/tests/test_rescue_closes_guard.sh` (5 cas ajoutés) | 150 |

Total estimé : 260 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Découpage en vols : oui, et voici la mesure qui l'impose.** L'estimation n'est
pas devinée à la densité du fichier ; elle est calibrée sur le **précédent exact**,
mika#3038 (`9a478c3c`), qui a livré **un seul** refus structurel du même bloc avec
sa suite :

| livrable de mika#3038 | lignes réelles |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` (1 prédicat + 1 refus) | 258 |
| `skills/bundled/_shared/tests/test_rescue_tree_equals_main.sh` (suite neuve) | 741 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` (doc + table) | 19 |
| `.github/workflows/ci.yml` (le step) | 7 |
| `Makefile` (la target dédiée) | 8 |
| **total hors `docs/`** | **1033** |

Les suites voisines confirment l'ordre de grandeur : `test_rescue_closes_guard.sh`
234 lignes, `test_rescue_cause_token.sh` 454, `test_rescue_tree_equals_main.sh`
741, `test_rescue_pipeline_verified.sh` 1023. Une suite neuve de six cas de cette
classe — fixtures de log, dépôts git réels, `gh` simulé, assertions sur
`_push_branch` — vaut ~450 lignes, pas 120.

Le périmètre complet des deux défauts se re-mesure donc à **~900 lignes** : sous le
seuil par PR de 1000, mais **plus du double de la borne par vol de 429**. C'est la
borne par vol qui mord, et elle impose le découpage. C'est aussi la classe de mort
que mika#2693 décrit — mika#2683 avait estimé 832 pour 1 432 réelles.

**Vol courant : 260 lignes, soit 169 de marge sous la borne.** Le défaut A est
choisi comme vol 1 pour trois raisons, dans cet ordre : il est le seul des deux
dont le périmètre tient réellement sous la borne ; c'est celui qui a été mesuré
**sept fois** ; et il ne touche ni `_push_branch`, ni le bloc Unit 2, ni
`dispatcher.rs`, ni le câblage CI — donc il ne dépend d'aucun arbitrage ouvert et
il est mergeable seul.

La suite est nommée au § Vol 2, re-mesurée à ~660 lignes, avec la consigne de la
découper à son tour.
