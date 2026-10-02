# mika#2636 — l'estimation de taille devient une section obligatoire du plan

**Ticket :** senara-solutions/mika#2636 · **Tier 1** (rail implement, relevé par le
commentaire opérateur du 2026-10-02) · **Branche :**
`feat/2636/groom-l-estimation-de-taille-devient-un`

---

## 1. Constat, et ce que la lecture du code déplace dans le ticket

Le ticket est juste sur le défaut et approximatif sur trois points de localisation.
Les rectifications sont le premier livrable du grooming : elles changent la
répartition des livrables, pas la cible.

### Le défaut, mesuré, n=2 sur compteur corrigé (cpp#259)

| # | ticket | pilote | tours | lignes hors `docs/` | plan portait une estimation ? |
|---|---|---|---|---|---|
| 1 | mika#2161 | `73720a14` | 151 (plafond) | ≈ 1 470 (PR rescue #2635) | **non** |
| 2 | mika#2633 | `2bd6fca0` | 151 (plafond) | ≈ 1 170 (PR rescue #2640) | **non** |
| — | mika#1960 ph. 2 | — | n'est pas mort de volume | 716 mesurées (≈ 515 estimées) | **oui** |

Deux sur deux au-dessus de 1 000 lignes sans estimation, tous deux morts au
plafond. La pratique « découper en phases au groom » existe depuis le
2026-10-01 et **rien ne la vérifie** : la seconde passe architecte a validé le
plan de mika#2161 en `PLAN_GROOMED` sans que quoi que ce soit demande la taille.

### R1 — « le vérificateur de `PLAN_GROOMED` côté dispatch-lib » n'existe pas comme vérificateur de *contenu*

AC2 propose deux sites alternativement. Le premier, lu littéralement, désigne un
objet qui n'existe pas : `_measure_cycle_output` (P4) et `_parse_verdict` lisent
**la sortie architecte**, jamais le plan. Les deux seuls lecteurs du *fichier*
plan dans `dispatch-lib.sh` sont `_find_issue_plan` (existence, mika#1421) et
`_fd_retry_if_section_still_missing` (contenu, deux `grep`, mika#2306).

Donc la porte dispatch-lib est **à construire**, pas à étendre — et son précédent
est exact, un mois plus ancien, et a déjà payé ses leçons (mika#2544 sur la
numérotation des titres, la source du prédicat, le budget d'une seule relance,
l'observabilité pure). **Ce plan l'imite terme pour terme plutôt que d'inventer
une seconde mécanique pour la même classe.**

### R2 — un `grep` ne peut pas juger un découpage ; l'architecte ne peut pas être le seul à regarder

AC2 demande qu'un plan dépassant le seuil soit refusé « à moins d'un découpage en
phases explicite ». Aucun prédicat lexical ne décide si un découpage est **réel**
ni si un total est **crédible**. Et l'inverse est tout aussi vrai : la moitié
architecte seule est de l'enforcement de prompt, que
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` borne
(mika#2120 : neuf récurrences sous prompt contre zéro quand la main de l'opérateur
écrivait la consigne).

D'où la répartition canonique de la maison — **détection permissive, décision
stricte** :

| question | juge | mécanisme |
|---|---|---|
| la section est-elle **présente** ? | `grep` | terme 2 du prédicat de relance (L1.d) |
| le total est-il **parsable** ? | `grep`/`sed` | extraction (L1.c), pour la journalisation |
| le total est-il **crédible** ? | architecte | Plan-Size Gate (L2, L3) |
| le découpage est-il **réel** ? | architecte | idem |

### R3 — la population mesurée n'est atteinte par AUCUNE porte de groom

C'est la rectification la plus lourde, et elle promeut AC5 du rang d'extra
d'observabilité à celui de **seule moitié qui couvre la population existante**.

Les deux plans morts étaient groomés **avant** ce correctif (#2161 groomé le
21/09, réécrit le 01/10 ; #2633 groomé le 02/10). Un plan déjà groomé sur disque,
dont le corps de ticket porte déjà les trois signaux du gate, **ne repasse par
aucune porte de groom** au dispatch suivant : `_detect_plan_on_branch` lit le
callout, bascule `ENTRY_COMMAND` sur `/ce-work <plan>`, et le pilote part. Un
refus posé au groom ne protège donc rien de ce qui est déjà en file.

Corollaire, et il décide la disposition d'AC5 : **au dispatch, on journalise et
on ne refuse jamais.** Refuser un dispatch dont le plan ne porte pas la section
gèlerait d'un coup toute la file groomée — c'est-à-dire la totalité des plans
existants de `docs/plans/`. L'absence est **nommée** (`total_loc=absent`), pas
refusée.

### R4 — le seuil n'est configurable que d'un côté, et il faut le dire

AC2 dit « 1 000 lignes, configurable ». Le jugement vit chez l'architecte, dont
le prompt est un `system_prompt.md` **statique** (aucune interpolation au
dispatch). Le seuil vit donc **deux fois** : littéral dans le prompt architecte,
variable (`PLAN_SIZE_MAX_LOC`) dans `dispatch-lib.sh` pour la règle injectée et
la journalisation.

Conséquence, écrite plutôt que découverte : **un opérateur qui baisse
`PLAN_SIZE_MAX_LOC` change ce que le groomeur VISE, pas ce que l'architecte
REFUSE.** Doublon assumé, du même type que `_FD_HEADING_RE` / `AC_HEADING_RE`
(« le préfixe est doublé, et c'est test-dispatch-lib.sh qui les tient ensemble »),
et tenu par le même moyen : un test lit le littéral du prompt architecte et le
défaut de dispatch-lib, et rougit sur une divergence.

---

## 2. Architecture — trois portes, une seule refuse

```
  ┌─ porte 1 : le PROMPT du groomeur ──────────────────────────┐
  │  _PLAN_SIZE_RULE injectée à chaque dispatch dev-groom      │  intention
  │  → /ce:plan ignore mika#2636, le substrat le compense      │
  └────────────────────────────────────────────────────────────┘
  ┌─ porte 2 : l'ARCHITECTE ───────────────────────────────────┐
  │  Plan-Size Gate — 1re passe ITERATE, 2de passe ESCALATE    │  REFUS
  │  + rattrapage structurel dispatch-lib si le revise         │
  │    n'a pas ajouté la section (copie de mika#2306)          │
  └────────────────────────────────────────────────────────────┘
  ┌─ porte 3 : le DISPATCH implement ──────────────────────────┐
  │  plan_size_estimate total_loc=… sur le .stderr forensique  │  MESURE
  │  jamais un refus — la file groomée existante en dépend     │
  └────────────────────────────────────────────────────────────┘
```

La porte 2 est la seule qui refuse, et son refus est celui qui existe déjà
(ITERATE / ESCALATE). Aucun nouveau mode d'échec n'est introduit dans la boucle.

---

## 3. Le format de la section (AC1) — un format de fil

La section doit être lisible par un humain **et** parsable par un `grep` au
dispatch. Forme canonique, prescrite par `_PLAN_SIZE_RULE` et par les deux
prompts architecte :

```markdown
## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/foo.rs` | 120 |
| `skills/bundled/_shared/dispatch-lib.sh` | 180 |
| tests (`tests/eval/test_foo.rs`) | 95 |

Total estimé : 395 lignes
```

Deux jetons, deux motifs, **un site de définition chacun** :

| motif | ce qu'il apparie | lecteurs |
|---|---|---|
| `_PLAN_SIZE_HEADING_RE` | `^##[[:space:]]+([0-9]+\.?[[:space:]]+)?Taille estimée` | terme 2 du rattrapage, re-test de journalisation |
| `_PLAN_SIZE_TOTAL_RE` | `^Total estimé[[:space:]]*:[[:space:]]*([0-9]+)` | extraction au dispatch (porte 3) |

Le motif de titre est le **jumeau terme pour terme** de `_FD_HEADING_RE` : préfixe
de numérotation optionnel borné à `<chiffres>[.]` (parce que `/ce:plan` numérote
ses titres — c'est le défaut que mika#2544 a dû corriger après qu'un rattrapage
Fire-Disposition a tiré à tort sur un plan conforme), `[[:space:]]+`, casse
repliée par `-i`, pas d'ancre `$`, texte ancré juste après le préfixe. **Ne pas
redécouvrir cette leçon : la copier.**

`Total estimé` porte l'unité dans le texte (`lignes`) mais le motif ne capture que
le nombre : un total écrit `395 lignes` et un total écrit `395` apparient tous
deux. Hors-périmètre nommé : `1 400` avec séparateur de milliers n'apparie pas —
la forme prescrite n'en porte pas, et élargir sur une devinette est refusé
(mika#2544 D-même-raisonnement). Un total non apparié est journalisé
`total_loc=unparsable`, jamais `0`.

**Langue du jeton.** Français, parce qu'AC1 le prescrit littéralement (« une
section **« Taille estimée »** »). Conforme à la règle mika#2201 telle qu'elle est
écrite — *une forme qu'un lecteur strict ne voit pas est refusée ; une forme qu'un
lecteur tolérant voit est admise* : les deux lecteurs livrés ici sont les seuls au
monde, ils lisent le jeton qu'ils prescrivent, et ils replient la casse.

---

## 4. Livrables

### L1 — `skills/bundled/_shared/dispatch-lib.sh`

**L1.a — `_PLAN_SIZE_RULE`** (≈ 22 l + commentaire). Constante de prompt, posée
juste après `_FIRE_DISPOSITION_RULE`, et appendue au prompt **sous la même
condition `[ "$SKILL" = "dev-groom" ]`** (ligne 3680). Elle devient la règle la
plus récente que lit un groomeur ; le commentaire de `_PILOT_SCRATCH_RULE` dit que
la récence est le seul levier d'une règle de prompt, donc l'ordre est à écrire
explicitement et le test T-ordre le tient.

Son contenu prescrit : le titre exact, le format du tableau, la ligne
`Total estimé :`, le seuil en vigueur (interpolé depuis `_plan_size_max_loc`), et
la conduite au-dessus du seuil — **découper en phases, la suite renvoyée à un
ticket ou à une phase nommée**. Elle cite le ticket par référence ; elle ne
reformule pas la doctrine du plafond de tours.

La règle vit **ici et non dans `.claude/commands/mika-groom-plan-only.md`** pour
la raison que `_FIRE_DISPOSITION_RULE` a déjà dû écrire : les trois commandes de
groom vivent dans `senara-solutions/mika-platform` et sont semées dans le worktree
par `_seed_worktree_slash_commands` (mika#1415), donc un ticket ouvert sur
`senara-solutions/mika` **ne peut pas les éditer**. Ce prompt est le seul canal que
ce dépôt contrôle. La moitié commandes est un suivi nommé, pas une simulation.

**L1.b — `_plan_size_max_loc()`** (≈ 30 l). Résout `PLAN_SIZE_MAX_LOC`, trois
paliers maison : absent ou vide → défaut `1000` ; entier positif → cette valeur ;
**illisible, `0` ou négatif → défaut, plus un `plan_size_threshold_invalid`
nommant la valeur entre guillemets.** Le `0` ne désarme pas : sur une garde dont le
rôle est de borner un coût, une coquille ne doit pas être un désarmement
silencieux. Nom **nu** (sans préfixe `MIKA_`), par le précédent tenu dans ce
fichier — `PILOT_LOG_DIR` et `PILOT_MAX_TURNS` le sont, et `sandboxed_pilot_env`
(`env_clear()` + allowlist positive) n'admet pas plus un `PILOT_*` nu qu'un
`MIKA_*` : le nom nu est une convention sur le relais `inject_pilot_dispatch_env`,
jamais un contournement de scrub (mika#2508).

Co-location : chaque lecture de `$_PLAN_SIZE_MAX_LOC` appelle `_plan_size_max_loc`
sur la **même ligne**, le mécanisme déjà en place pour `_pilot_log_dir` et
`_pilot_max_turns`, et le test de co-location le refuse autrement.

**L1.c — `_plan_size_total_loc()`** (≈ 40 l). Lit un fichier plan, rend le total
sur stdout, ou rien. Trois issues, et **elles sont distinctes** : section absente,
section présente mais total non apparié, total apparié. Les trois alimentent trois
valeurs distinctes de la ligne de journal — `absent`, `unparsable`, `<n>`. Les
confondre rendrait « le plan n'a pas été dimensionné » et « le plan a été
dimensionné dans une forme que le lecteur ne sait pas lire » indiscernables, alors
que les remèdes sont opposés (groomer vs réparer le motif).

**Les blocs clôturés sont retirés AVANT toute lecture, et ce plan en est la
preuve vivante.** Son §3 documente le format dans un bloc ```` ```markdown ````
qui contient un titre `## Taille estimée` **et** une ligne
`Total estimé : 395 lignes` — l'exemple, pas la mesure. Un lecteur naïf qui prend
le premier match rapporterait 395 pour un plan qui en annonce 530 : un nombre
plausible, présenté avec autorité, faux. Pire, le **terme 2 du rattrapage** (L1.d)
apparierait le titre cité et conclurait que la section est présente — un plan qui
*documente* le format sans le remplir passerait la garde.

Les deux lecteurs strippent donc les blocs clôturés d'abord, exactement comme
`auto_pull::is_groomed` le fait pour les trois prédicats de callout (mika#2120) :
une ligne légitimement citée à l'intérieur d'un fence n'est pas une déclaration.
Même conduite sur un fence non terminé : **rien n'est strippé** (le corps entier
est évalué), le sens de l'arbitrage étant celui de mika#2120 — un faux positif
coûte une relance de revise, un faux négatif a coûté quinze heures de boucle.

Et après le strip, **le dernier match gagne**, jamais le premier : un plan peut
légitimement porter un total par phase avant son total global, et le dernier est
celui qui conclut. Figé par S16.

**L1.d — `_plan_size_retry_if_section_still_missing()`** (≈ 65 l + commentaire).
**Copie structurelle de `_fd_retry_if_section_still_missing`**, greffée sur la même
branche `sha256` réussie de `_launch_revise_pilot`, appelée juste après elle
(ligne 7352). Compteur `_PLAN_SIZE_REVISE_RETRIED` remis à zéro à chaque entrée de
`_launch_revise_pilot`, global à dessein.

Prédicat, **conjonction de deux `grep`, jamais un jugement** :
1. l'architecte a réclamé la section ⇔ le findings-file de **première passe**
   (`$1`) contient `Taille estimée` ;
2. la section est absente ⇔ le plan révisé ne porte pas le titre
   (`$_PLAN_SIZE_HEADING_RE`).

**La source du terme 1 est portante.** Le findings ciblé que la fonction écrit
elle-même (`findings-1-size.md`) est **interdit** comme source : il contient
nécessairement la chaîne, donc un prédicat qui le relirait serait vrai par
construction — la garde relancerait quand l'architecte n'a rien demandé, et le
test de relance-unique resterait vert sur une garde qui ne regarde plus la sortie
architecte. Le compteur casserait la boucle ; il ne rendrait pas le défaut
visible. C'est le piège que mika#2306 a documenté ; on ne le retombe pas.

**Budget architecte inchangé** : aucun `_arch_ask` sur ce chemin. Ce qui est
élargi est le budget du *revise*, qui n'est le contrat de personne, et d'une seule
tentative. **Cette fonction ne refuse jamais rien** : elle réessaie, puis laisse
passer en journalisant. Un échec dur ici déplacerait l'ESCALATE d'une porte au
lieu de le lever.

**Un seul revise, deux findings possibles.** Si les findings de première passe
réclament **à la fois** `Fire-Disposition` et `Taille estimée`, les deux
rattrapages peuvent tirer dans la même invocation de `_launch_revise_pilot`, à la
suite, chacun avec son propre compteur. Coût : jusqu'à deux pilotes de revise
supplémentaires sur un plan qui ignore deux sections. Acceptable (chacun est
borné à un, et l'alternative est un ESCALATE sans recours), et **nommé** plutôt
que découvert. L'ordre est `Fire-Disposition` puis `Taille estimée` — arbitraire,
figé par test pour que la composition soit reproductible.

**L1.e — porte 3, la pose** (≈ 20 l). Dans `_detect_plan_on_branch`, sur la
branche où le fichier plan est confirmé présent dans le worktree (ligne 9363),
poser la **globale** `_PLAN_SIZE_TOTAL` depuis `_plan_size_total_loc`. Globale par
le précédent exact de la même fonction : `_PLAN_CALLOUT_REFUSAL` et
`PILOT_SHIPPING_TAIL` y sont déjà, et pour la même raison (`PLAN_PATH` y est
`local`).

**L1.f — porte 3, l'émission** (≈ 15 l). Une ligne sœur de `pilot_budget_armed`,
émise depuis `_emit_pilot_budget_line` (appelée par `_run_pilot_sandboxed`, donc
**dans** la redirection `2>"$STDERR_FILE"` que `_run_claude_pilot` persiste en
`$PILOT_LOG_DIR/<task-id>.stderr`) :

```
dispatch-lib: plan_size_estimate total_loc=1400 threshold_loc=1000 verdict=over_threshold plan=docs/plans/….md
dispatch-lib: plan_size_estimate total_loc=absent threshold_loc=1000 verdict=unknown plan=docs/plans/….md
```

**Le sink n'est pas négociable et c'est la leçon la plus coûteuse de ce
plan.** Émettre avant la ligne de lancement enverrait la ligne sur le stderr
propre de `dispatch-lib`, que l'exécuteur ne lit **que** dans sa branche
`if !status.success()` : sur un dispatch qui réussit — et un dispatch implement
réussit — le tuyau est jeté sans être lu et la ligne n'atterrit dans **aucun
fichier**. C'est le Signal M, mesuré par mika#2050, et `_detect_plan_on_branch`
porte déjà ce commentaire mot pour mot pour son propre refus. Poser la ligne à
côté de `pilot_budget_armed` achète en plus la corrélation que Prime demande
**sans jointure** : « ce dispatch avait 150 tours pour 1 400 lignes » se lit dans
un seul fichier, deux lignes consécutives.

Émise **seulement** quand un plan-on-branch a été détecté (donc `SKILL =
dev-pilot`), jamais sur les pilotes de revise, pour qui la question n'a pas de
sens. Et **toujours** quand il l'a été, y compris `total_loc=absent` : sans cela,
zéro ligne se lirait comme « tous les plans sont dimensionnés » alors qu'elle
voudrait dire « aucun ne l'est » — classe mika#2205 appliquée à la sonde de ce
ticket. `absent` et jamais `0` : *un `null` n'est jamais un `0`* (mika#2331).

**L1.g — `plan_size_threshold_resolved`** (≈ 8 l). Une ligne, au même sink, portant
`threshold_loc` et `source` ∈ `{env, default}`. Doctrine mika#2293 : *un réglage
qu'on ne peut pas observer n'est pas un réglage, c'est un espoir* — et la borne
R4 ci-dessus rend cette provenance d'autant plus nécessaire, puisque la valeur
observée ne gouverne qu'une moitié.

### L2 — `skills/bundled/mika-arch-groom-ticket/system_prompt.md` (≈ 28 l)

**Plan-Size Gate (mika#2636)**, posé à côté du Fire-Disposition Gate (l. 73) dont
il copie la forme — arbre de décision explicite, mention de la tolérance de
numérotation (mika#2544), vocabulaire de verdict inchangé :

1. Section absente ⇒ **ITERATE**, jamais READY.
2. Section présente, total ≤ 1 000 ⇒ gate passe.
3. Section présente, total > 1 000, **aucun** découpage en phases nommant la
   suite (ticket de suivi ou phase nommée) ⇒ **ITERATE**.
4. Section présente, total > 1 000, découpage explicite dont le périmètre de
   cette PR retombe sous le seuil ⇒ gate passe.
5. Total manifestement incrédible au regard des livrables énumérés ⇒ **ITERATE**,
   et c'est le jugement que le `grep` ne peut pas rendre (R2).

### L3 — `skills/bundled/mika-arch-second-review/system_prompt.md` (≈ 22 l)

Même gate, même arbre, verdict **ESCALATE** au lieu d'ITERATE — « No ITERATE
exists at second pass per the two-pass limit », repris de son voisin.

### L4 — `skills/bundled/_shared/test-dispatch-lib.sh` (≈ 190 l)

Série de tests, modelée sur T1–T12j de mika#2306/#2544. AC4 est couvert par les
trois premiers, **chacun vu rouge avant d'être vu vert** :

| # | assertion | contrôle |
|---|---|---|
| S1 | plan sans section + findings la réclamant ⇒ relance unique, événement émis | **vu rouge** |
| S2 | plan à 515 lignes ⇒ `verdict=under_threshold`, aucune relance | positif |
| S3 | plan à 1 400 lignes sans découpage ⇒ `verdict=over_threshold` | **vu rouge** côté prompt |
| S4 | `## 7. Taille estimée` apparie (numérotation, mika#2544) | **vu rouge** si le motif est littéral |
| S5 | findings ne réclamant pas la section ⇒ **aucune** relance (contrôle négatif) | — |
| S6 | findings illisible ⇒ aucune relance, aucun événement (fail-safe) | — |
| S7 | relance **unique** : second appel après `_PLAN_SIZE_REVISE_RETRIED=1` ⇒ no-op | — |
| S8 | `findings-1-size.md` n'est **jamais** la source du terme 1 (scan de source) | — |
| S9 | `total_loc=absent` ≠ `total_loc=0` ; section sans total ⇒ `unparsable` | — |
| S10 | la ligne est émise depuis `_emit_pilot_budget_line`, pas avant le lancement (scan de source, classe mika#2050) | — |
| S11 | co-location : toute lecture de `$_PLAN_SIZE_MAX_LOC` appelle `_plan_size_max_loc` sur la même ligne | — |
| S12 | `PLAN_SIZE_MAX_LOC` invalide ⇒ défaut + événement nommant la valeur | — |
| S13 | `_PLAN_SIZE_RULE` est la **dernière** règle du prompt d'un groomeur, et absente du prompt `dev-pilot` (contrôle négatif de condition, calqué sur T3) | — |
| S14 | le littéral de seuil des **deux** prompts architecte égale le défaut de `_plan_size_max_loc` (R4, calqué sur T12j) | — |
| S15 | les deux rattrapages composent : findings réclamant les deux sections ⇒ deux relances, dans l'ordre figé | — |
| S16 | **sur ce plan même** : les deux lecteurs rendent `530` et non `395`, et un plan qui ne porte le titre que dans un fence est lu « section absente » | **vu rouge** |

**S14 est le test qui tient R4.** Sans lui, baisser le défaut shell laisserait les
prompts architecte sur 1 000 et la divergence serait muette.

**S16 prend ce plan pour fixture**, et c'est délibéré : il est le seul fichier de
l'arbre à porter le format **à la fois** en exemple clôturé et en mesure réelle,
donc le seul qui distingue un lecteur qui strippe d'un lecteur qui ne strippe pas.
Un fixture synthétique l'aurait fait aussi, mais celui-ci est en plus
**auto-nettoyant** : si un futur éditeur retire l'exemple du §3, l'assertion
rougit et nomme ce qu'elle a perdu.

### L5 — `CLAUDE.md` (hors compte de taille)

Section d'observabilité : les trois noms d'événement, leur régime attendu, le
tableau de lecture, les sondes post-déploiement et leurs haltes, et ce que le
travail n'achète pas. Hors compte parce que la mesure du ticket exclut
explicitement « docs, plan et CLAUDE.md ».

---

## 5. Fire-Disposition

Ce plan livre des détecteurs : les quinze assertions de L4, dont trois scans de
source (S8, S10, S11), plus deux gates de prompt (L2, L3) et un rattrapage
structurel (L1.d). Option retenue : **(c) périmètre — aucun détecteur livré ici
n'a de population pré-existante, et le seul lecteur qui verrait la population
existante est délibérément non-refusant.** Détail, site par site :

| détecteur | population pré-existante | disposition |
|---|---|---|
| L1.d (rattrapage) | **aucune** — ne lit qu'un plan en cours de cycle de groom, avec des findings de première passe en main. Un plan ancien sur disque n'entre jamais dans son prédicat | gate N/A par construction |
| L2, L3 (gates architecte) | **aucune** — ne jugent qu'un plan soumis à une passe. Un re-groom d'un plan ancien produit ITERATE, ce qui **est** AC3 : l'estimation est ajoutée au re-groom | gate N/A par construction |
| L1.e/f (porte 3) | **la file groomée entière** — tous les plans de `docs/plans/` sont sans section | **journalise, ne refuse jamais** (R3). L'absence est nommée `total_loc=absent`, comptée, et c'est la mesure de la couverture |
| S8, S10, S11 (scans de source) | **aucune** — portent sur du code neuf ; allowlists livrées **vides** | — |

**Le site que ce plan refuse délibérément d'armer est `scripts/verify-pipeline.sh`.**
Il porte déjà `AC_HEADING_RE` (mika#1600/#2516), donc il est le précédent exact
pour « une section obligatoire dans un plan, vérifiée en CI ». Y ajouter
`## Taille estimée` ferait rougir **toute** PR touchant un plan antérieur à ce
correctif, c'est-à-dire la population entière. Le fermer demanderait une borne
d'époque ou une allowlist sur un gate CI bloquant : décision distincte, blast
radius distinct, **ticket de suivi**. AC2 ne nomme pas la CI, et l'armer ici
serait un élargissement de périmètre déguisé en cohérence.

---

## 6. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` (L1.a–L1.g, commentaires maison inclus) | 275 |
| `skills/bundled/mika-arch-groom-ticket/system_prompt.md` (L2) | 28 |
| `skills/bundled/mika-arch-second-review/system_prompt.md` (L3) | 22 |
| `skills/bundled/_shared/test-dispatch-lib.sh` (L4, seize assertions + fixtures) | 205 |
| `CLAUDE.md` (L5) | hors compte par la mesure du ticket |

Total estimé : 530 lignes

**Sous le seuil de 1 000, et ce n'est pas une coïncidence de rédaction :** livrer
un plan de 1 500 lignes pour le ticket qui refuse les plans de 1 500 lignes serait
la réfutation la plus courte de son propre livrable. Aucun découpage en phases
n'est donc requis.

Marge de l'estimation : le ratio mesuré sur mika#1960 phase 2 est 716/515 ≈ 1,39.
Appliqué ici, 530 × 1,39 ≈ 737 lignes — toujours sous le seuil, avec de la marge.
Le poste le plus incertain est L4, dont le volume suit la densité de commentaire
exigée par les fixtures de `test-dispatch-lib.sh`.

---

## 7. Verification contract

**V1 — AC4, les trois assertions vues rouges.** S1, S3 et S4 doivent être exécutées
**contre le code d'avant le correctif** et constatées rouges avant d'être rendues
vertes. Un test vert dès sa première exécution n'atteste pas qu'il regarde quelque
chose (classe mika#2205). Geste : `bash skills/bundled/_shared/test-dispatch-lib.sh`.

**V2 — le rattrapage ne tire pas sur un plan conforme.** S2 et S5, le contrôle
négatif que mika#2544 a dû livrer après qu'un rattrapage a relancé un pilote pour
rien sur un plan numéroté conforme.

**V3 — la ligne de journal atterrit.** Geste d'opérateur sur l'hôte, après
`make deploy` :

```bash
cat ~/.mika/skills/.manifest-writer          # le sha doit être celui qu'on vient de bâtir
grep -h '^dispatch-lib: plan_size_estimate' \
  "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail
```

**Halte V3 —** zéro ligne : établir d'abord que le binaire servi porte le
correctif. `skills/bundled/` est une projection du **binaire**, pas du checkout
(mika#2340) ; sans cette vérification la sonde décrit le binaire d'hier.
Contrôle positif obligatoire : `grep -c '^dispatch-lib: pilot_budget_armed'` sur
le même glob — zéro des deux ne prouve rien.

**V4 — AC5, le matériau de la re-mesure.** Sur 30 jours, croiser `total_loc` avec
le volume mesuré du diff de la PR correspondante. **C'est le livrable que Prime
attend**, et ce travail livre le thermomètre, pas la conclusion : si la
distribution montre que le seuil de 1 000 est mal placé, c'est le seuil qui bouge,
jamais la mesure qui est désarmée.

**V5 — contrôle négatif de bruit, 7 jours.** `plan_size_threshold_invalid` et
`plan_size_still_missing_after_retry` doivent rester vides. Une occurrence
soutenue du second dit que le pilote de revise **ne sait pas écrire la section** —
donc que le correctif est côté `/mika-revise-plan` (suivi `mika-platform`), **pas
une troisième relance ici**. C'est mot pour mot la halte que mika#2306 a écrite
pour son jumeau.

**V6 — `make verify-bundled-skills`** (mika#1575) : les deux prompts architecte
restent structurellement valides.

---

## 8. Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2636.

- [ ] **AC1.** Un plan groomé porte une section **« Taille estimée »** au format
  fixe : lignes de code hors `docs/` par livrable, et le total.
- [ ] **AC2.** La porte de groom (le vérificateur de `PLAN_GROOMED` côté
  dispatch-lib, ou la seconde passe architecte) **refuse** un plan sans cette
  section. Un plan dont le total dépasse le seuil (1 000 lignes, configurable) est
  refusé à moins d'un découpage en phases explicite, qui renvoie la suite à un
  ticket ou à une phase nommée.
- [ ] **AC3.** Re-groom d'un plan ancien : l'estimation est **ajoutée** même quand
  le plan n'est que re-mesuré (le cas de mika#2161).
- [ ] **AC4.** Test : un plan sans section de taille est refusé (**vu rouge**) ;
  un plan à 515 lignes passe ; un plan à 1 400 lignes sans découpage est refusé.
- [ ] **AC5.** Le total estimé est journalisé au dispatch, pour qu'il soit comparé
  au volume mesuré du diff. C'est le matériau de la re-mesure du plafond voulue
  par Prime.

**Correspondance AC → livrable**, et les deux bornes à lire avec :

| AC | livrables | borne |
|---|---|---|
| AC1 | L1.a (prescription), L2/L3 (gate), §3 (format de fil) | — |
| AC2 | L2, L3 (le refus), L1.d (le rattrapage structurel) | le « vérificateur de `PLAN_GROOMED` côté dispatch-lib » n'existe pas comme vérificateur de contenu (R1) : la porte est construite, et c'est l'architecte qui refuse |
| AC3 | L2 (ITERATE sur section absente ⇒ ajout au re-groom), L1.d | — |
| AC4 | L4, S1/S2/S3 vus rouges | — |
| AC5 | L1.e, L1.f, L1.g | **journalise sans refuser** (R3) — refuser au dispatch gèlerait la file groomée existante |

---

## 9. Definition of Done

1. `_PLAN_SIZE_RULE` injectée à chaque dispatch `dev-groom`, et **seulement** à
   ceux-là (S13), en dernière position du prompt.
2. Les deux prompts architecte portent le Plan-Size Gate, avec le même seuil
   littéral que le défaut shell (S14).
3. `_plan_size_retry_if_section_still_missing` greffée sur la branche réussie de
   `_launch_revise_pilot`, budget d'une relance, prédicat à deux `grep`, source du
   terme 1 interdite au findings ciblé (S7, S8).
4. `plan_size_estimate` émis depuis `_emit_pilot_budget_line`, sur tout dispatch
   plan-on-branch, avec `absent` / `unparsable` / `<n>` distincts (S9, S10).
5. Les deux lecteurs strippent les blocs clôturés et prennent le **dernier**
   match ; ce plan sert de fixture et rend `530`, jamais `395` (S16).
6. `PLAN_SIZE_MAX_LOC` résolu en trois paliers, provenance journalisée (S11, S12).
7. Les seize assertions passent ; S1, S3, S4 et S16 ont été **vues rouges**.
8. `make verify-bundled-skills` et `bash skills/bundled/_shared/test-dispatch-lib.sh`
   verts.
9. `CLAUDE.md` porte la section d'observabilité, ses sondes et ses haltes.
10. Ce plan lui-même porte `## Taille estimée` **et** est lu correctement par le
    lecteur qu'il prescrit — l'auto-cohérence est le premier contrôle positif du
    livrable, et S16 la transforme en assertion plutôt qu'en intention.

---

## 10. Ce que ce travail n'achète PAS

- **Il ne borne pas le volume d'une implémentation.** Il rend l'estimation
  obligatoire et la mesure lisible ; un pilote peut toujours écrire 1 400 lignes
  sur un plan qui en annonçait 400. C'est la limite honnête, et c'est pourquoi
  AC5 existe : la comparaison estimé/mesuré est ce qui dira si l'estimation vaut
  quelque chose.
- **Il ne rattrape ni mika#2161 ni mika#2633.** Les deux sont morts, leurs PR de
  rescue sont ouvertes, et **rien ici ne rétro-estampille** un plan : fabriquer une
  estimation datée d'un dispatch qu'on n'a pas observé serait l'inverse de ce que
  ce travail défend. La sonde est le **prochain** dispatch.
- **Il ne re-mesure pas le plafond.** Il livre le matériau que Prime a demandé
  (V4), pas la conclusion. Le seuil de 1 000 est **posé** par le ticket sur n=2, pas
  mesuré sur une distribution — et il est à réviser sur la première distribution
  que cette mesure produira.
- **Il ne couvre pas un dispatch sans plan-on-branch.** Un `/mika` sur un ticket
  non groomé n'émet aucune ligne de taille, par construction : il n'y a pas de plan
  à lire. Population nommée, hors mesure.
- **Il ne refuse rien au dispatch**, et la file groomée existante le lui doit (R3).
- **Il ne surveille rien.** Les seuls instruments sont les greps de V3/V5 et la
  corrélation de V4, et **leur silence ne prouve rien tant que personne ne les
  exécute** — d'où le contrôle positif obligatoire sur `pilot_budget_armed`.

---

## 11. Hors périmètre, délibérément

- **`scripts/verify-pipeline.sh`** — le gate CI. Refusé avec sa raison en §5 :
  population pré-existante totale, borne d'époque ou allowlist requise sur un gate
  bloquant. **Ticket de suivi.**
- **La moitié `.claude/commands/`** — `/mika-groom-plan-only.md`,
  `/mika-groom-ticket.md`, `/mika-revise-plan.md` vivent dans
  `senara-solutions/mika-platform` et sont structurellement hors d'atteinte d'un
  ticket ouvert sur `senara-solutions/mika` (mika#1415). **Suivi nommé**, comme
  mika#2306 l'a nommé pour sa propre moitié.
- **`scripts/canonical-tokens.tsv`** — non touché, et la raison est mesurée :
  `Fire-Disposition` n'y est **pas** déclaré non plus, et le commentaire de
  `_FD_HEADING_RE` dit que le survey « ne voit PAS … un motif porté par une
  variable ». Les deux motifs livrés ici sont portés par variable, donc hors
  population des deux lecteurs de mika#2201. **Dit plutôt que découvert.**
- **Le seuil du jugement architecte, rendu configurable** — demanderait
  d'interpoler un `system_prompt.md` statique au dispatch, c'est-à-dire de changer
  le contrat de chargement des prompts bundled. R4 nomme la borne ; S14 empêche la
  divergence silencieuse.
- **La mesure automatique du diff** pour clore la boucle estimé/mesuré sans geste
  humain — demande de lire le diff d'une PR fusionnée depuis le moteur. Autre
  population, autre site. **Suivi**, précondition : que V4 montre que la
  corrélation vaut d'être automatisée.
- **Le relèvement de `PILOT_MAX_TURNS`** comme remède alternatif. Un plafond plus
  haut achète du volume contre du coût sans rien dire de la taille, et la mémoire
  MPC pose explicitement le volume comme le discriminant. Hors sujet pour ce
  ticket, et la décision appartient à la re-mesure de Prime.
- **Les séparateurs de milliers** dans `Total estimé` (`1 400`, `1,400`) — non
  appariés, forme non prescrite, élargissement sur devinette refusé (mika#2544).
