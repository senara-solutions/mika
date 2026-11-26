# mika#3118 — un plan en halte ne sort pas du groom en feu vert (vol 2 sur 2)

**Vol 2 sur 2** — le vol 1 (`docs/plans/2026-11-22-002-fix-3118-task-engine-l-auto-fire-du-dev-pilot-plan.md`,
mergé par `35239e0d`, PR #3139) a fermé la moitié *consentement* : l'auto-fire mika#1614 relit le label
`ready` sur la forge et refuse un dispatch sans consentement. Ce vol ferme la moitié *halte du plan*,
celle que le § Périmètre du vol 1 a nommée et renvoyée ici (commentaires 58a / 59a / 54 du parking #2983).

## Le fait mesuré, et ce qui le laisse passer aujourd'hui

Le 19/11, le groom `93828860` a produit un plan portant une section de halte **en tête** —
`## HALTE — la borne du groom est dépassée` (l. 11 de
`docs/plans/2026-11-19-002-feat-1727-v4c-le-verrou-des-points-d-entree-plan.md`), suivie de
« **`Total estimé : 312 lignes` ajoutées, pour une borne du groom de `300`.** » et de
« **La décision appartient à Vincent** : lever la borne, recouper le vol, ou arbitrer autrement…
Le groom n'en choisit aucune. » La boucle d'architecte a convergé, le groom est sorti en
`status: success`, le callout `second-pass (GROOMED)` a été écrit, et l'auto-fire a lancé le pilote
`137111d7` à 15:47:31Z : **1,34 $ et zéro commit**. Le 17/11, la même classe a coûté un pilote tué à la
main vers 13:04Z (`be5cca2e`, plan `59c94bb1` portant
`## HALTE BORNE — le périmètre ne tient pas, et ce plan ne le recoupe pas`).

Depuis, neuf tickets ont atterri dans la zone — #3139, #3140, #3141, #3117, #3121, #3142 (vols 2a/2b),
#3171, #3122 — et **aucun ne porte la halte jusqu'au moteur** :

| ticket | ce qu'il a fermé | pourquoi il ne ferme pas celle-ci |
|---|---|---|
| #3139 | relecture du consentement `ready` à l'auto-fire | le 19/11 le label `ready` **était** posé : le consentement était sain |
| #3140 | six sorties de précondition nommées | la halte n'est pas une précondition manquante, c'est un `success` qui mentait |
| #3117 | l'auto-pull lit le dernier verdict de groom (3 sites) | le verdict du 19/11 était `GROOMED`, pas `ESCALATE` |
| #3121 | plan lancé ≠ plan de `origin` (`LAUNCH_PLAN_DIVERGED`) | le plan en halte **était** celui de `origin` : aucune divergence |
| #3142 | `_pilot_declared_halt` — la halte d'un **pilote** refuse la draft | lit le log de session d'un pilote, en aval ; ici la halte est dans le **plan**, en amont |
| #3171 | une halte déclarée recommande un re-groom | conséquence d'une halte déjà détectée ; rien ne la détecte côté groom |
| #3122 | `_OPERATOR_RESERVED_RULE` injectée au prompt du groomeur et au fichier de constats | **du texte de prompt**, donc un filet de jugement LLM ; aucun mécanisme |

mika#3122 est le voisin le plus proche et c'est précisément ce qui rend ce vol nécessaire : il demande à
mika-arch de rendre `ESCALATE` sur une halte réservée, par instruction. Ce vol livre le **net mécanique
derrière ce jugement** — quand l'architecte laisse passer, la machine refuse quand même.

## Périmètre

**Dans ce vol.** Une porte, posée dans `_iterate_groom_loop`, sur les **deux** branches qui convergent en
`GROOMED` (`skills/bundled/_shared/dispatch-lib.sh:12088` *ready-to-groomed* et `:12256`
*iterate-to-groomed*), **avant** `_write_canonical_callout` : elle lit le plan sur le disque du worktree,
et si le plan déclare une halte elle route la convergence vers `_escalate_groom` avec une cause nouvelle
au lieu d'écrire le callout GROOMED.

**Hors de ce vol, nommé et chiffré.** Le pendant d'**audit moteur** — une ligne `audit_events`
`groom_plan_halt_declared` écrite par `skills/executor.rs` sur le modèle de
`note_groom_proof_unbound_to_plan` (`:2346`), plus le scan de synchronisation shell ↔ Rust de mika#2641
pour les deux valeurs de cause neuves. Coût estimé ~120 lignes, ce qui ferait passer ce vol à ~476,
au-dessus de la borne. **Il n'est pas requis pour que le recensement ait un dénominateur** : les deux
populations voyagent déjà dans la même colonne `tasks.result` (voir § *Le dénominateur existe déjà*), donc
l'`audit_events` est un confort de requêtage, pas la condition d'observabilité.
**Tracked in:** senara-solutions/mika#3118 (vol 3 — confort d'audit ; ce vol-ci clôt le défaut).

## Pourquoi la porte vit AVANT le callout, et pas après `_iterate_groom_loop`

Deux emplacements ont été pesés sur la mesure, et un seul tient.

**P2 — après le retour de `_iterate_groom_loop`, au site qui compose `Outcome: PLAN_GROOMED`
(`dispatch-lib.sh:14760-14869`), en lisant `$WORKTREE_DIR/$GROOM_CALLOUT_PLAN`.** Un seul site, et le
chemin du plan y est déjà validé. **Refusé** : à cet instant `_write_canonical_callout` a **déjà** écrit
`second-pass (GROOMED)` dans le corps du ticket. `check_grooming_markers` (`skills/executor.rs`, Pin B)
lit ce marqueur ; un repose du label `ready` passerait donc la porte de dispatch et lancerait le pilote sur
le plan en halte par la voie webhook. On aurait fermé l'auto-fire et laissé la porte de derrière ouverte.

**P1 — dans `_iterate_groom_loop`, sur les deux bras `GROOMED`, avant le callout.** Retenu, pour cinq
propriétés qui ne se cumulent nulle part ailleurs :

1. **`Outcome: ESCALATE` suffit, et ne coûte aucune ligne de Rust.** La précondition d'étape 2 de
   `try_dispatch_pilot_after_groom_success` (`task_engine/dispatcher.rs:5944`) exige
   `Outcome: PLAN_GROOMED` ; un `ESCALATE` n'est pas un feu vert par construction.
2. **La disposition du producteur est préservée telle quelle.** Le bras `elif grep -qE '^Outcome: ESCALATE'`
   de `:14766` existe depuis mika#2545 : il ne repose aucun marqueur retryable.
3. **Le callout GROOMED n'est jamais écrit**, donc Pin B refuse aussi la voie webhook — la porte de
   derrière de P2 n'existe pas.
4. **Pas de boucle « re-groom à l'infini » (mika#2012).** mika#3117 a livré le filtre
   `groom_escalate_terminal` : l'auto-pull exclut un `ESCALATE` terminal à ses trois sites d'engagement.
   Le seul chemin de reprise restant est un repose **manuel** du label `ready`, borné par un geste humain
   à chaque tour.
5. **mika#3122 prescrit littéralement cette sortie** : « son verdict est `ESCALATE`, et `ESCALATE` est une
   sortie NOMINALE, pas un échec de grooming ». La porte ne fabrique pas une sémantique nouvelle, elle
   applique mécaniquement celle que le prompt demande.

**Le chemin du plan se résout localement, sans réseau.** `_write_canonical_callout` le prouve :
il appelle `_find_issue_plan` (mika#1421), réduit en relatif, refuse un chemin absolu
(`write_canonical_callout_plan_outside_worktree`), vérifie `[ -f "$WORKTREE_DIR/$plan_relpath" ]`
(`write_canonical_callout_plan_missing`) et n'affecte `GROOM_CALLOUT_PLAN` qu'à `:11590`. La porte emploie
la même résolution — `_find_issue_plan` — et ne touche pas `GROOM_CALLOUT_PLAN`, qui reste la publication
du callout.

## La grammaire de halte, mesurée et non prescrite

Le vol 1 avait nommé l'ingrédient : « la grammaire de halte reste à poser ». Elle se **mesure** au lieu de
se décréter, exactement comme mika#3142 a mesuré la forme de la halte d'un pilote.

Les deux incidents écrivent la halte comme un **titre H2 dont le premier mot est `HALTE`**, en tête de
plan : `## HALTE — la borne du groom est dépassée` (l. 11) et
`## HALTE BORNE — le périmètre ne tient pas, et ce plan ne le recoupe pas` (l. 9). Mesure sur les
**1331 fichiers** de `docs/plans/` : `^## *HALTE` apparie **exactement un** fichier — le vrai positif.
Le second vit sur `59c94bb1` et n'a jamais été mergé.

Les faux positifs réalistes sont exclus par l'ancre H2 :

| forme | où | pourquoi elle n'apparie pas |
|---|---|---|
| `**HALTE 1 —**` … `**HALTE 4 —**` | plans 2420 et 2518, l. 572-700 | de la prose en gras, pas un titre ; et en queue de fichier |
| `HALTE borne : N lignes` | 45+ citations de la règle mika#2693, l. 88-100 | jamais en `^## ` |
| `HALTE borne[[:space:]]*:` | `dispatch-lib.sh`, `test-dispatch-lib.sh`, `tests/test_pilot_halt_declared.sh` depuis mika#3142 | hors `docs/plans/`, et la porte ne lit qu'un fichier de plan |

**Ce vol ne prescrit PAS la grammaire dans les prompts**, et c'est un choix argumenté, pas un report.
Deux raisons : (i) prescrire une forme ne protège pas de la dérive d'un LLM — le détecteur doit mordre sur
l'observé, pas sur le prescrit, et mika#3142 a tranché la même question dans le même sens ; (ii) le texte
du prompt de groom porte depuis mika#3122 une contrainte lexicale portante — `_OPERATOR_RESERVED_RULE`
(`:5616-5683`) ne doit épeler **ni** `## Fire-Disposition` **ni** `## Taille estimée`, sans quoi les deux
rattrapages `_fd_retry_if_section_still_missing` (`:10675`) et
`_plan_size_retry_if_section_still_missing` (`:10806`) relanceraient un pilote de revise à chaque ITERATE
sur un finding synthétique faux. Y ajouter une grammaire de halte, c'est ajouter un littéral à tenir en
phase dans la zone la plus fragile du fichier, pour un gain nul sur le détecteur.
Le coût de l'option écartée, chiffré : ~60 lignes (texte de prompt + scan de synchronisation + cas).
Le risque résiduel est nommé en § Risques, avec sa direction.

## La direction du fail-safe, et pourquoi elle est l'INVERSE de celle de mika#3142

mika#3142 répond « pas de halte » à **toute** indétermination — variable non posée, fichier absent, vide,
illisible, `tail` en échec — parce qu'un faux positif y **retient un artefact** : push plus draft de
secours, exactement la perte que mika#1282 existe pour empêcher. La direction y est explicitement
asymétrique, et son § Risques le dit.

Ici le rapport de coûts est renversé, et il est mesuré :

| erreur | ce qu'elle coûte | mesure |
|---|---|---|
| faux **positif** (halte déclarée à tort) | un dispatch retenu. Le plan est commité, poussé par `_push_branch`, le corps d'`_escalate_groom` dit pourquoi. L'opérateur repose `ready` : un geste | — |
| faux **négatif** (halte manquée) | un pilote part sur un plan en halte | 1,34 $ / zéro commit le 19/11 ; un pilote tué à la main le 17/11 |

Donc **l'indétermination penche vers la halte**, avec une exception nommée et une distinction de
population :

- plan **trouvé et lu**, aucun motif → `pas de halte`. L'absence de motif dans un fichier lu n'est pas une
  indétermination : c'est une réponse négative.
- plan **trouvé mais illisible** (`[ -r ]` faux, `grep` en échec) → `halte`, cause
  `plan_halt_unreadable`. Fail-closed : un vol ne part jamais *parce que* la lecture a échoué (mika#2981).
- plan **absent** (`_find_issue_plan` ne rend rien) → `pas de halte`, **et c'est l'exception**. Cette
  branche est déjà la grâce nommée `GROOM_PROOF_UNBOUND_GRACE`, comptée par l'événement
  `groom_proof_unbound_to_plan` (écrivain unique : `skills/executor.rs:2346`). Y répondre « halte »
  détruirait la grâce qui existe pour préserver la convergence, et transformerait un groom sans plan
  détectable en `ESCALATE` systématique.

**Deux causes, jamais une (mika#2205).** « Le plan déclare une halte » et « je n'ai pas pu lire si le plan
déclare une halte » sont deux populations, et une population fusionnée n'est plus soustractible — c'est
la discipline que le vol 1 a appliquée à `ready_absent_at_autofire` contre
`consent_unreadable_at_autofire`. D'où **deux** valeurs de cause neuves, pas une.

## Le dénominateur existe déjà (mika#3050)

« Un recensement qui ne compte que les anomalies n'a pas de dénominateur. » Ici le dénominateur est acquis
sans ligne de sonde neuve, parce que les deux populations voyagent dans la **même colonne** :

| population | forme dans `tasks.result` |
|---|---|
| groom sain | `Outcome: PLAN_GROOMED — <plan>` |
| halte lue | `Outcome: ESCALATE` + `Groom-halt-cause: plan_halt_declared` |
| plan illisible | `Outcome: ESCALATE` + `Groom-halt-cause: plan_halt_unreadable` |
| plan absent (grâce) | `Outcome: PLAN_GROOMED` nu + `audit_events` `groom_proof_unbound_to_plan` |

Le parseur `groom_halt_cause` de `dispatcher.rs:6586` lit déjà une cause quelconque — son test
`some_future_cause` (`:8929`) le pin — donc les deux valeurs neuves sont lisibles côté moteur **sans
constante Rust neuve**. Le scan `mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust`
(`canonical_tokens.rs:6686`) est ancré nommément sur `GROOM_HALT_CAUSE_LINE_PREFIX` et
`GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE` : il ne demande rien aux valeurs voisines, donc il reste vert.
C'est cette mesure qui fait tenir le vol sous la borne.

**L'absence d'une ligne `Groom-halt-cause: plan_halt_*` ne dit pas « aucune halte »** mais « binaire
antérieur au vol » (classe mika#2340) : `skills/bundled/` est lu au build, donc rien n'est en vigueur
avant `make deploy`.

## Ordre de commit

Chaque étape laisse l'arbre vert et la suite passante.

1. **Les deux constantes de cause** dans la famille `GROOM_HALT_CAUSE_*` (`dispatch-lib.sh:11268-11420`),
   avec le commentaire de famille qui dit « LA CAUSE DÉCIDE DU CORPS ». Aucun appelant encore : le scan de
   cardinalité S-c de `test-dispatch-lib.sh` compte les sites d'appel d'`_escalate_groom` **qui passent une
   cause**, pas les causes déclarées, donc il reste vert.
2. **Le détecteur `_plan_declared_halt <chemin>`**, calqué structurellement sur `_pilot_declared_halt`
   (`:8230-8302`) : prédicat pur, un argument, aucune variable globale lue, `grep -qE` sur une
   **here-string** et jamais sur un tube (sous `pipefail`, `grep -q` ferme le tube et le 141 du producteur
   devient le statut du pipeline — faux négatif intermittent que `scripts/verify-no-sigpipe-grep.sh`
   ne verrait pas ; correctif mesuré de mika#3142, pas une préférence).
3. **La nouvelle suite** `skills/bundled/_shared/tests/test_plan_halt_declared.sh`, qui atteste le
   détecteur en isolation sur les **deux formes mesurées**, la prose en gras, la citation de la règle
   mika#2693, et les trois branches d'indétermination. Elle rougit sans l'étape 4 : elle ne teste que le
   détecteur.
4. **Le wiring** : une cible dans le `Makefile` et une étape dans `.github/workflows/ci.yml`, sur le
   modèle exact de mika#3142 (`Makefile:238`, `ci.yml:762`). `suite-wiring-lint` refuse une suite
   qu'aucun workflow n'atteint, et `unwired=0` depuis mika#2793 : une suite non câblée ouvrirait une
   entrée de backlog avec sa propre Fire-Disposition. Ce vol n'en ouvre pas.
5. **La porte `_groom_plan_halt_gate`** : résout le plan via `_find_issue_plan`, applique la table de
   direction du fail-safe ci-dessus, pose `GROOM_LOOP_FAILURE_REASON` et appelle `_escalate_groom` avec la
   cause. Encore aucun appelant.
6. **Les deux insertions** aux bras `GROOMED` de `:12088` et `:12256`, avant `_write_canonical_callout`.
   C'est le commit qui change le comportement.
7. **Le scan de cardinalité** dans `test-dispatch-lib.sh` : exactement deux bras `GROOMED` portent la
   porte, et le contrôle négatif prouve que le scan voit un bras non câblé.

Plancher mergeable : les étapes 1 à 7. Sans l'étape 6 rien ne change ; sans l'étape 7 la porte peut
disparaître d'un bras sans qu'aucun test ne rougisse.

## Fire-Disposition

Ce vol livre des détecteurs — la suite `test_plan_halt_declared.sh` et le scan de cardinalité de
`test-dispatch-lib.sh` — donc la section est armée. **Option (a) : exception nommée en allowlist, livrée
vide**, comme le vol 1.

**Le scan de cardinalité.** `EXPECTED_PLAN_HALT_GATE_SITES = 2` : la porte est posée sur exactement les
deux bras `GROOMED` de `_iterate_groom_loop`. Population d'exceptions **mesurée vide** — `grep -n
"_write_canonical_callout" dispatch-lib.sh` rend deux sites d'appel (`:12088`, `:12256`), et le scan les
confronte un à un. L'allowlist `PLAN_HALT_GATE_SITE_EXCEPTIONS` est livrée **vide**, et son vide est
asserté par une assertion sœur auto-nettoyante : quand le scan tire, on câble le bras, on n'excepte pas le
bras. Si une troisième branche de convergence apparaît un jour, le scan rougit — c'est l'effet voulu, et
le ticket de suivi est celui qui ajoute cette branche, jamais une entrée d'exception.

**Contrôle négatif, parce qu'un détecteur sur un dépôt propre n'est pas attesté comme voyant.** La suite
fabrique un bras `GROOMED` sans porte dans une copie du fichier sous `.pilot-scratch/` et vérifie que le
scan le refuse ; et elle fabrique les deux formes de halte mesurées plus les faux positifs du tableau de
§ *La grammaire de halte* pour mesurer le détecteur sur des entrées des deux signes.

**Rien n'atterrit désarmé** : pas de `#[ignore]`, pas de `skip`, pas de halte-et-remontée. Le vol est
entièrement armé en CI dès l'étape 4.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` (2 causes, `_plan_declared_halt`, `_groom_plan_halt_gate`, 2 insertions) | 120 |
| `skills/bundled/_shared/tests/test_plan_halt_declared.sh` (nouvelle suite) | 175 |
| `skills/bundled/_shared/test-dispatch-lib.sh` (scan de cardinalité + contrôle négatif + cas de grammaire) | 55 |
| wiring (`Makefile`, `.github/workflows/ci.yml`) | 6 |

Total estimé : 356 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 356 est sous 429, et sous le seuil par PR de 1000. Le vol 1 avait estimé
cette moitié à « de l'ordre de 400 lignes à elle seule » ; deux mesures de ce grooming l'ont ramenée à
356, et ce sont elles qui décident :

- **Le coût Rust est nul.** Le parseur `groom_halt_cause` accepte déjà une cause quelconque (`dispatcher.rs:8929`
  `some_future_cause`), et les quatre scans `mika2641_*` de `canonical_tokens.rs:6609-6744` sont ancrés
  nommément sur `architect_unreadable` — aucun n'exige qu'une valeur de cause neuve ait un pendant Rust.
  L'hypothèse du vol 1 — « le lecteur Rust ancré de la ligne » plus « le scan de synchronisation
  shell ↔ Rust » — portait ~120 lignes qui n'ont pas lieu d'être écrites.
- **La grammaire n'est pas prescrite**, donc le prompt de `dev-groom` n'est pas touché : ~60 lignes de
  moins, et la zone lexicalement contrainte de mika#3122 reste intacte.

Le confort d'audit moteur, chiffré à ~120 lignes, est la seule chose renvoyée plus loin, et il est nommé
en § Périmètre avec son `Tracked in:`. L'architecte juge le périmètre de ce vol, non la somme des vols.

## Definition of Done

- Un plan portant `## HALTE …` en tête ne produit plus `Outcome: PLAN_GROOMED` : la convergence route vers
  `_escalate_groom` avec `Groom-halt-cause: plan_halt_declared`, et `_write_canonical_callout` n'est pas
  appelé.
- Les deux bras `GROOMED` de `_iterate_groom_loop` portent la porte, et un scan le refuse si l'un la perd.
- Les deux formes de halte mesurées (`## HALTE —`, `## HALTE BORNE —`) sont appariées ; les trois classes de
  faux positifs mesurées ne le sont pas.
- Un plan absent laisse la grâce `GROOM_PROOF_UNBOUND_GRACE` intacte ; un plan illisible rend
  `plan_halt_unreadable` et jamais `plan_halt_declared`.
- La nouvelle suite est atteinte par CI, et `scripts/check-suite-wiring.sh` rend toujours `unwired=0`.
- `bash scripts/verify-pipeline.sh origin/main`, `cargo test`, `cargo clippy`, `cargo fmt --check`,
  `make verify-bundled-skills` et `bash skills/bundled/_shared/test-dispatch-lib.sh` passent.

## Acceptance criteria

Le corps du ticket mika#3118 ne porte pas de section `## Acceptance criteria` ; les critères ci-dessous
sont dérivés du périmètre et du contrat de vérification de ce vol.

- **AC1** — Sur le bras *ready-to-groomed*, un plan dont la ligne 11 est `## HALTE — la borne du groom est
  dépassée` produit `Outcome: ESCALATE` portant `Groom-halt-cause: plan_halt_declared`, et le corps du
  ticket ne reçoit aucun marqueur `second-pass (GROOMED)`.
- **AC2** — Même propriété sur le bras *iterate-to-groomed*, avec la forme
  `## HALTE BORNE — le périmètre ne tient pas, et ce plan ne le recoupe pas`.
- **AC3** — Contrôle négatif de la grammaire : un plan portant `**HALTE 1 —**` en prose, ou une citation
  `HALTE borne : 312 lignes` de la règle mika#2693, converge normalement en `PLAN_GROOMED`. Les deux cas
  sont des assertions distinctes.
- **AC4** — Un plan trouvé mais illisible rend `Groom-halt-cause: plan_halt_unreadable` ; aucun chemin ne
  fusionne cette population avec `plan_halt_declared` (mika#2205).
- **AC5** — Un groom dont `_find_issue_plan` ne rend rien converge en `PLAN_GROOMED` nu : la grâce
  `GROOM_PROOF_UNBOUND_GRACE` et son événement `groom_proof_unbound_to_plan` sont inchangés.
- **AC6** — `_plan_declared_halt` est attesté en isolation : un argument, aucune variable globale lue,
  une here-string et jamais un tube, et `scripts/verify-no-sigpipe-grep.sh` reste vert.
- **AC7** — Le scan de cardinalité compte exactement 2 bras `GROOMED` portant la porte, son allowlist est
  livrée vide, son vide est asserté, et son contrôle négatif prouve qu'il voit un bras non câblé.
- **AC8** — La nouvelle suite est nommée par une cible `Makefile` et par une étape `ci.yml` ;
  `scripts/check-suite-wiring.sh` rend `unwired=0`.
- **AC9** — Les quatre scans `mika2641_*` de `canonical_tokens.rs` restent verts sans modification : la
  preuve que les deux causes neuves n'exigent aucune constante Rust.

## Risques

| risque | effet | parade |
|---|---|---|
| Un groomeur futur déclare la halte dans une forme que `^##[[:space:]]*HALTE` n'apparie pas (titre H3, mot en anglais, titre en milieu de fichier) | faux négatif : un pilote repart sur un plan en halte — exactement l'incident du 19/11 | nommé et non résolu par ce vol. La direction du fail-safe ne couvre **pas** ce cas : un fichier lu sans motif rend « pas de halte ». Le filet de jugement de mika#3122 reste en première ligne, et l'option « prescrire la grammaire » est chiffrée à ~60 lignes en § *La grammaire de halte* |
| Un plan parle de halte sans en déclarer une (prose, citation de règle, plan rétrospectif) | faux positif : un dispatch retenu | l'ancre H2 exclut les trois classes mesurées sur 1331 fichiers, et AC3 les pin. Coût d'un résidu : un geste opérateur |
| L'opérateur repose `ready` sur un ticket dont le plan porte toujours la halte | cycle groom → `ESCALATE` → groom | borné par un geste humain à chaque tour, et mika#3117 exclut déjà un `ESCALATE` terminal de l'auto-pull : pas de livelock machine |
| La porte disparaît d'un seul bras lors d'une refonte de `_iterate_groom_loop` | un bras de convergence redevient un feu vert, en silence | le scan de cardinalité d'AC7 ; c'est sa raison d'être |
| Le vol est mergé mais pas déployé | la porte n'existe pour aucun agent | `skills/bundled/` est lu au build : l'absence de `Groom-halt-cause: plan_halt_*` se lit « binaire antérieur au vol » (mika#2340), jamais « aucune halte ». Sonde : `make deploy` puis `cat ~/.mika/skills/.manifest-writer` |
