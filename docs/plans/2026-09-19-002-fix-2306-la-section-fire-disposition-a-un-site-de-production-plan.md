# mika#2306 — La section `## Fire-Disposition` a un site de production

**Ticket :** mika issue#2306
**Type :** fix (substrat de la boucle de grooming)
**Date :** 2026-09-19

---

## Problème

Le ticket énonce : « les plans groomés par le moteur peuvent ne PAS contenir la
section `## Fire-Disposition` que mika#1574 exige ; mika-arch ESCALATE alors à
juste titre — et la porte de dispatch ne voit jamais `Outcome: PLAN_GROOMED` ».

C'est exact, et la mesure déplace le diagnostic d'un cran : **le défaut n'est pas
que la section manque, c'est que son absence brûle l'unique itération de la
boucle, sur un motif purement formel et parfaitement évitable en amont.**

### Trois maillons, mesurés

**M1 — Le producteur ne l'émet pas, et c'est un fait déjà écrit dans ce dépôt.**
`/ce:plan` est un plugin tiers (`compound-engineering`) qui n'a aucune
connaissance de mika#1574. Ce n'est pas une hypothèse : le Acceptance-Criteria
Gate de `mika-arch-groom-ticket/system_prompt.md` énonce déjà le raisonnement
**mot pour mot** pour la section sœur —

> `/ce:plan` (the third-party `compound-engineering` marketplace plugin) does not
> produce that section […] Guaranteeing the section here, at groom time, is what
> prevents the mika#1531/#1533/#1557/#1558 `block[pipeline]` failure class.
> **Grooming is the surface we control between the third-party producer and our
> validator.**

`## Acceptance criteria` a reçu ce traitement (mika#1600/#1627).
`## Fire-Disposition` ne l'a jamais reçu.

**M2 — Aucune commande de groom ne prescrit la section.** Mesure exhaustive sur
les trois commandes du pipeline :

| Commande | `## Acceptance criteria` | `## Fire-Disposition` |
|---|---|---|
| `/mika-groom-plan-only` (boucle autonome) | **étape 5b**, explicite | **absente** |
| `/mika-groom-ticket` (opérateur) | — | **absente** |
| `/mika-revise-plan` (branche ITERATE) | mentionnée (« honor existing AC ») | **absente** |

`grep -n "Fire-Disposition" .claude/commands/*.md` → **zéro occurrence.**

**M3 — La boucle n'a qu'un seul ITERATE, et le revise ne vérifie pas ce qu'il a
révisé.** `_iterate_groom_loop` (`dispatch-lib.sh`) est un automate à deux passes
strictes :

```
first-pass READY    → second-pass → GROOMED | ESCALATE
first-pass ITERATE  → _launch_revise_pilot (1×) → second-pass → GROOMED | ESCALATE
first-pass ESCALATE → mort
```

Le gate second-passe est sans recours, et il le dit :

> A revised plan that includes detector-class deliverables without a
> `## Fire-Disposition` section MUST return `ESCALATE` — never `GROOMED`.
> **(No ITERATE exists at second pass per the two-pass limit.)**

Et `_launch_revise_pilot` valide sa révision ainsi :

```sh
local pre_hash;  pre_hash=$(sha256sum "$plan_path" | cut -d' ' -f1)
# … lance /mika-revise-plan …
local post_hash; post_hash=$(sha256sum "$plan_path" | cut -d' ' -f1)
if [ "$pre_hash" != "$post_hash" ]; then … return 0    # « convergé »
```

**Le critère est « le contenu a changé », jamais « le finding a été traité ».**
Un revise qui corrige une virgule sans ajouter la section demandée est, pour la
boucle, indistinguable d'un revise réussi. Elle enchaîne alors sur le
second-pass, qui ESCALATE — et l'unique itération a été dépensée pour rien.

### Population

Mesures prises sur cette branche au 2026-09-19, **chacune avec sa commande** —
un chiffre dont la méthode n'est pas écrite n'est pas reproductible, et se lit
comme faux dès que le lecteur choisit une autre regex :

| Mesure | Commande | Compte |
|---|---|---|
| Plans datés | `ls docs/plans/ \| grep -cE '^20[0-9]{2}-[0-9]{2}-[0-9]{2}-'` | 878 |
| Section AC, forme canonique | `grep -lE '^## Acceptance criteria' docs/plans/20*.md \| wc -l` | 322 |
| Section AC, toute profondeur/casse | `grep -liE '^#+ *Acceptance criteria' docs/plans/20*.md \| wc -l` | 593 |
| Section FD, forme canonique | `grep -lE '^## Fire-Disposition' docs/plans/20*.md \| wc -l` | 70 |

**L'écart 322 / 593 sur la même section est lui-même un résultat**, et il porte :
la moitié du corpus écrit le titre AC sous une autre profondeur ou une autre
casse. Il rappelle que « la section est-elle là ? » n'a de réponse stable qu'avec
sa regex — le fait même qui disqualifie une garde `grep` en CI (D2, motif 2).

**La FD ajoutée après un ITERATE architecte : classe prouvée, compte borné.**
La recherche lexicale liant `Fire-Disposition` à un finding architecte sur une
même ligne rend **13** fichiers, dont le présent plan (qui cite le motif sans
l'avoir subi) :

```sh
grep -liE '(Fire.Disposition.{0,80}(soulev|mika-arch|F[0-9])|(soulev|mika-arch).{0,80}Fire.Disposition)' docs/plans/20*.md
```

C'est une **borne supérieure lexicale, pas un compte vérifié** : distinguer un
plan qui a subi l'ITERATE d'un plan qui cite la doctrine demande de lire les
treize. Le plan ne fabrique pas ce compte. Mais l'argument ne repose pas dessus —
il repose sur l'**existence** de la classe, et deux témoins la prouvent, nommés
et vérifiables à la ligne près :

- `docs/plans/2026-09-01-002-fix-2126-telegram-url-nue-plan.md:232` — « Requis par
  le Fire-Disposition Gate (mika#1574), **soulevé par mika-arch en première**
  [passe] ».
- `docs/plans/2026-09-07-004-fix-2228-label-write-via-app-identity-plan.md:44` —
  la section est littéralement titrée `## Fire-Disposition (F4)` : le `(F4)` est
  le numéro du finding architecte auquel elle répond.

Chacun a consommé une passe architecte pour ajouter une section d'une quinzaine
de lignes que le groomeur pouvait écrire du premier coup. C'est le coût que ce
plan supprime, et il suffit qu'il soit réel.

Le ratio 70/878 (8,0 %) ne se lit **pas** comme un taux de défaut : la section est
**conditionnelle** (mika#1574 : « Plan has no detector-class deliverables ⇒ gate
is N/A »). Il n'existe aucun moyen déterministe de compter la population qui
*aurait dû* la porter — l'obstacle même qui disqualifie la garde CI (D2).

---

## Requirements

- **R1** — Tout pilote de groom dispatché par la boucle autonome reçoit, dans le
  canal que le substrat contrôle, la prescription d'émettre `## Fire-Disposition`
  quand le plan porte un livrable détecteur.
- **R2** — Un revise lancé sur un finding réclamant `## Fire-Disposition` ne peut
  plus rendre « convergé » alors que la section est toujours absente.
- **R3** — Le budget architecte (deux passes) est **inchangé**. Aucun appel
  `_arch_ask` supplémentaire.
- **R4** — Le chemin nominal (plan sans détecteur, ou plan portant déjà la
  section) ne paie rien : ni appel, ni branche, ni ligne de journal.
- **R5** — Toute information illisible sort le dispatch de la population traitée
  plutôt que de l'y faire entrer (fail-safe dans le sens de la maison).
- **R6** — Ce qui n'est pas livrable dans ce dépôt est **nommé**, pas simulé.

---

## Décisions

### D1 — Le site de prescription est le `PROMPT` construit par `dispatch-lib`, et c'est une contrainte de périmètre, pas une préférence

Les trois commandes de groom ne sont **pas trackées par ce dépôt** :

```
$ git ls-files .claude/commands/
.claude/commands/mika-doc-audit.md
.claude/commands/mika-issue.md
.claude/commands/mika-issues.md
.claude/commands/mika.md
```

Elles vivent dans `mika-platform` et sont **semées** dans le worktree par
`_seed_worktree_slash_commands()` (mika#1415), dont le commentaire énonce
l'invariant : « the worktree's own tracked command wins ». Un ticket ouvert sur
`senara-solutions/mika` ne peut donc pas éditer `/mika-groom-plan-only`.

Le précédent qui tranche est dans le fichier même, à vingt lignes du site
d'injection — `_PR_BODY_CONTAINMENT_RULE` (mika#2211) :

> The rule belongs HERE and not only in a `.claude/commands/mika.md`, because
> **this is the one channel every pilot of every repo reads.** The pilot is
> launched with exactly two inputs […]: the entry command, resolved from the
> TARGET repo's worktree, and this PROMPT.

C'est la même configuration, pour la même raison, et le remède est déjà écrit
dans ce dépôt. `_FIRE_DISPOSITION_RULE` suit `_PR_BODY_CONTAINMENT_RULE`.

**Ce n'est pas du prompt-enforcement au sens que
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` condamne.**
La leçon de mika#2120 est précise : *« a fix that lives in a prompt lasts exactly
as long as the memory of whoever writes the prompt »* — neuf récurrences quand la
consigne dépendait qu'un opérateur pense à la taper, zéro quand il y pensait. Une
constante injectée par le substrat **à chaque dispatch** ne dépend d'aucune
mémoire. Et la moitié structurelle est livrée séparément (U2), ce qui est
exactement la forme que cette doctrine prescrit : jamais la prescription seule.

### D2 — Une garde CI dans `verify-pipeline.sh` est REFUSÉE, sur trois motifs mesurés

Le réflexe est de transposer mika#1600 intégralement : la prescription **plus**
la garde `verify-pipeline.sh` U2. Elle est écartée, et c'est un livrable du plan.

1. **Elle arriverait après l'autorité qu'elle contredirait.**
   `verify-pipeline.sh` tourne sur la PR (`ci.yml:344`, `BASE_REF=origin/main`).
   Le grooming, lui, n'ouvre pas de PR : il commite le plan sur la branche et
   écrit le callout sur l'issue. La CI ne voit donc le plan qu'au moment de la PR
   **d'implémentation** — après que mika-arch a rendu `GROOMED`. Une garde là
   ferait rougir une PR dont le plan a déjà été déclaré conforme par l'architecte :
   deux autorités en contradiction sur le même artefact, l'opérateur arbitrant à
   la main. Le contraste avec AC est net — AC est inconditionnelle, donc la CI et
   l'architecte ne peuvent pas diverger.

2. **La conditionnalité n'est pas exprimable en bash.** « Le plan porte-t-il un
   livrable détecteur ? » est le jugement sémantique que mika#1574 confie à un
   LLM. Les deux contournements échouent : une heuristique lexicale (« test »,
   « garde », « lint ») rend un faux positif sur quasiment tout plan de ce dépôt ;
   et rendre la section **inconditionnelle** contredirait frontalement la doctrine
   (« gate is N/A »), ce qui est une décision produit hors du périmètre d'un
   ticket substrat.

3. **Elle déplacerait le blocage sans le lever.** Le ticket demande que la boucle
   dispatche. Troquer un `ESCALATE` architecte contre un rouge CI laisse le
   ticket immobile, une porte plus loin.

Si une garde déterministe devient souhaitable un jour, sa précondition est une
**mesure** : la proportion de plans GROOMED portant un détecteur sans la section.
Cette mesure n'existe pas et ce plan ne la fabrique pas.

### D3 — Le budget élargi est celui du *revise*, jamais celui de l'architecte

Deux budgets distincts sont en jeu, et les confondre serait toucher à la doctrine :

- Le **budget architecte** : deux passes. C'est le design de mika#1271, il borne
  le coût LLM et il n'est pas touché (R3).
- Le **budget revise** : un lancement de `_launch_revise_pilot`. C'est un détail
  d'implémentation, pas un contrat — aucun document ne le prescrit.

U2 relance le revise **une** fois, avec un finding ciblé, sans appeler
`_arch_ask`. Le plan révisé part ensuite au second-pass exactement comme
aujourd'hui. La boucle deux-passes est intacte ; ce qui change est qu'elle ne la
dépense plus sur un plan dont on sait déjà qu'il sera refusé.

### D4 — Le prédicat est déterministe des deux côtés, et inerte sur le chemin nominal

La garde U2 est une **conjonction de deux `grep`**, jamais un jugement :

- l'architecte a réclamé la section ⇔ ses findings contiennent la chaîne
  `Fire-Disposition` (c'est le vocabulaire imposé par son propre gate) ;
- la section est absente ⇔ le plan révisé ne porte pas `^## Fire-Disposition`.

Si l'un des deux termes est faux, **rien ne se passe** : comportement d'avant le
correctif, bit pour bit. Un plan sans détecteur ne paie rien, un revise qui a fait
son travail ne paie rien (R4). Et un findings-file illisible sort le dispatch de
la population (R5), il ne l'y fait pas entrer.

### D5 — La prescription est conditionnée à `dev-groom`

`$SKILL` est le discriminant, déjà disponible dans `_set_up_worktree` et déjà
employé au même endroit (`dispatch-lib.sh:2203`). Injecter la règle pour
`dev-pilot` serait du bruit dans le prompt d'un pilote qui n'écrit pas de plan.

---

## Implementation Units

| # | Fichier | Changement |
|---|---|---|
| **U1** | `skills/bundled/_shared/dispatch-lib.sh` | Constante `_FIRE_DISPOSITION_RULE`, posée à côté de `_PR_BODY_CONTAINMENT_RULE` (~l.1935). Elle nomme la section, ses trois options canoniques (a)/(b)/(c) en une ligne chacune, et la règle N/A explicite. Injectée dans `PROMPT` au site mika#2178/#2211 (~l.2480), **après** `_PR_BODY_CONTAINMENT_RULE` — les trois invariants de position documentés au site restent vrais et la première ligne du `PROMPT` reste exactement `<repo>#<num>` (contrat mika#138). Conditionnée `[ "$SKILL" = "dev-groom" ]` (D5). |
| **U2** | `skills/bundled/_shared/dispatch-lib.sh` | Dans `_launch_revise_pilot`, après la comparaison `sha256` **réussie** (branche `pre_hash != post_hash`, l.5255) : si les findings réclamaient `Fire-Disposition` et que le plan révisé ne porte toujours pas `^## Fire-Disposition`, écrire un findings-file ciblé (`findings-1-fd.md`) et relancer le pilote de revise **une seule fois**. Le retour reste `0` (le plan *a* changé au premier tour) — la garde ajoute une tentative, elle ne crée pas de mode d'échec. |
| **U2b** | *(idem)* | **Terminaison, et la distinction qui la rend vraie :** après la seconde tentative, la section est re-testée **pour journaliser, jamais pour reboucler**. La relance est gardée par `_FD_REVISE_RETRIED` (U3), donc un second échec ne peut que produire `fire_disposition_still_missing_after_retry` et rendre la main — il n'existe aucun chemin qui réarme le lancement. C'est ce qui réconcilie « une seule relance » (R3, budget) et AC7 (l'événement doit savoir si la section manque encore) : le prédicat est évalué deux fois, il n'autorise l'action qu'une. |
| **U3** | `skills/bundled/_shared/dispatch-lib.sh` | Un compteur de garde (`_FD_REVISE_RETRIED`) explicite, remis à zéro à l'entrée de `_launch_revise_pilot`, pour que la terminaison soit lisible sans dérouler le flot de contrôle. |
| **U4** | `skills/bundled/_shared/test-dispatch-lib.sh` | Les dix tests T1–T10 — voir § Verification Contract. Portent le harnais de 31 à 41 ; aucune famille d'assertion nouvelle. |
| **U5** | `docs/solutions/best-practices/fire-disposition-doctrine.md` | Section « Site de production » : la doctrine décrit aujourd'hui la règle et son gate, jamais qui écrit la section. Ajouter les deux sites (prescription `PROMPT`, rattrapage revise) et le renvoi au suivi `mika-platform`. |

### Journal

Deux événements sur `stderr` (le canal de `dispatch-lib`, collecté dans le log
pilote) :

- `fire_disposition_revise_retried` — la garde a relancé le revise. **Régime
  attendu : rare.** Chaque ligne est une itération architecte que la boucle n'a
  pas gaspillée.
- `fire_disposition_still_missing_after_retry` — la seconde tentative n'a pas
  produit la section ; le plan part au second-pass en sachant qu'il sera refusé.
  **Régime attendu : zéro.** Une occurrence soutenue dit que le pilote de revise
  ne sait pas écrire la section, donc que le correctif à faire est le suivi
  `mika-platform` (`/mika-revise-plan`), **pas** un troisième essai ici.

L'absence de la première ligne n'est jamais à elle seule une preuve de succès :
elle se lit aussi « aucun groom n'a tourné ». Lire le volume de dispatches
`dev-groom` sur la même fenêtre avant de conclure (mika#2205).

---

## Verification Contract

Le harnais `skills/bundled/_shared/test-dispatch-lib.sh` existe — **31** fonctions
de test au 2026-09-19 (`grep -cE '^test_[a-z0-9_]+\(\)' skills/bundled/_shared/test-dispatch-lib.sh`)
— et porte déjà les deux familles employées ici : assertions de **forme de code**
(`declare -f` + `assert_contains`) et assertions **comportementales** sur worktree
temporaire. T1–T10 portent le total à 41 ; aucune famille nouvelle n'est requise.

| # | Test | Ce qu'il attrape |
|---|---|---|
| T1 | `_FIRE_DISPOSITION_RULE` est non vide et nomme les trois options (a)/(b)/(c) | Une règle tronquée qui prescrit la section sans dire quoi y écrire |
| T2 | La règle est injectée dans `PROMPT` **pour** `SKILL=dev-groom` | La prescription n'atteint pas le pilote |
| T3 | **Contrôle négatif** : elle n'est **pas** injectée pour `SKILL=dev-pilot` | D5 régressée, bruit dans le prompt d'implémentation |
| T4 | La première ligne du `PROMPT` reste `<repo>#<num>` après injection | Contrat mika#138 et invariant de position 2 (la regex ancrée manquerait, le dispatch tomberait en free-text et **aucun worktree** ne serait créé) |
| T5 | Comportemental : findings réclamant FD + plan révisé sans la section ⇒ le revise est relancé exactement **une** fois | R2 — le cœur du correctif |
| T6 | **Contrôle négatif** : findings sans mention de FD ⇒ zéro relance | R4 — la garde n'est pas inerte mais bavarde |
| T7 | **Contrôle négatif** : plan révisé portant déjà `^## Fire-Disposition` ⇒ zéro relance | R4 — le chemin nominal ne paie rien |
| T8 | Findings-file illisible ⇒ zéro relance, retour inchangé | R5 — fail-safe |
| T9 | Forme de code : le bras de garde ne contient aucun appel `_arch_ask` | R3 — un futur éditeur qui « améliorerait » la garde en redemandant l'avis de l'architecte doublerait le budget LLM sans qu'aucun test de comportement ne rougisse |
| T10 | Comportemental : seconde tentative **échouée** ⇒ `fire_disposition_still_missing_after_retry` émis exactement une fois sur `stderr`, retour inchangé, et **aucune troisième** relance | AC7, et la terminaison de U2b. Sans lui, une garde qui rendrait la main en silence sur ce chemin passerait T5 en vert : l'échec de second tour deviendrait indistinguable d'un succès, ce qui est précisément l'angle mort que le plan reproche au critère `sha256` |

**T3, T6 et T7 sont porteurs, pas décoratifs.** Sans eux, une garde qui relance
*toujours* passerait T5 en vert tout en doublant le coût de chaque grooming du
dépôt. T9 est structurel pour la même raison qu'il est structurel ailleurs dans
ce dépôt : la régression qu'il attrape ne rendrait aucune décision fausse, elle
changerait le budget en silence.

---

## Definition of Done

- U1–U5 livrés ; `bash skills/bundled/_shared/test-dispatch-lib.sh` vert.
- `make lint` et `make test` verts.
- Le ticket de suivi `mika-platform` est ouvert et référencé dans le corps de la
  PR (voir § Suivi).
- La doctrine `fire-disposition-doctrine.md` nomme les sites de production.

---

## Acceptance criteria

Le corps de mika#2306 ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés des Requirements et du Verification Contract.

- **AC1** — Un dispatch `dev-groom` reçoit dans son `PROMPT` une prescription
  nommant `## Fire-Disposition` et ses trois options canoniques. Vérifiable sans
  modèle, par le champ `prompt` du JSON de dry-run. (T1, T2)
- **AC2** — Un dispatch `dev-pilot` ne reçoit **pas** cette prescription. (T3)
- **AC3** — La première ligne du `PROMPT` d'un dispatch `dev-groom` est exactement
  `<repo>#<num>`. (T4)
- **AC4** — Quand les findings de première passe réclament `## Fire-Disposition`
  et que le plan révisé ne la porte pas, `_launch_revise_pilot` relance le pilote
  de revise exactement une fois. (T5)
- **AC5** — Dans les trois cas négatifs — findings sans mention de FD, section
  déjà présente, findings-file illisible — le nombre de relances est zéro et la
  valeur de retour est celle d'avant le correctif. (T6, T7, T8)
- **AC6** — Aucun chemin introduit par ce plan n'appelle `_arch_ask`. Le nombre
  d'appels architecte par grooming reste de deux au maximum. (T9)
- **AC7** — `fire_disposition_still_missing_after_retry` est émis, et lui seul,
  quand la seconde tentative échoue : la boucle continue vers le second-pass au
  lieu de s'interrompre, et **aucune troisième relance n'a lieu**. (T10)
- **AC8** — `docs/solutions/best-practices/fire-disposition-doctrine.md` nomme les
  deux sites de production et le ticket de suivi `mika-platform`.

---

## Fire-Disposition

Requise par le Fire-Disposition Gate (mika#1574). Les livrables détecteurs de ce
plan sont les dix tests T1–T10 de U4, dont **T9 est un scan de forme de code** —
la classe qui peut tirer sur de l'existant.

**Option retenue : (a) exception nommée — table vide, et la vacuité est assertée.**

- **T1–T8 et T10** sont des tests de comportement **sur du code que ce plan crée**
  (`_FIRE_DISPOSITION_RULE`, le bras de garde de `_launch_revise_pilot`, le
  compteur `_FD_REVISE_RETRIED`). Ils ne peuvent structurellement pas tirer sur de
  l'existant : leur sujet n'existait pas avant ce plan. T10 en particulier asserte
  l'émission d'un événement que ce plan introduit, sur un chemin que ce plan
  introduit. Aucune exception concevable.
- **T9** est le seul à scanner du code préexistant — le corps de
  `_launch_revise_pilot`. Son prédicat porte sur le **bras de garde introduit par
  U2**, jamais sur la fonction entière : `_launch_revise_pilot` ne contient aucun
  appel `_arch_ask` aujourd'hui (vérifié : les appels `_arch_ask` vivent dans
  `_iterate_groom_loop`, fonction distincte). La table d'exceptions est donc
  **vide**, et T9 porte l'assertion auto-nettoyante correspondante : *si le scan
  devait un jour exempter une occurrence, l'exemption doit être nommée ici et
  datée.* Une table vide assertée vide est ce qui distingue « aucune violation »
  de « le scan ne regarde rien » — modèle repris de
  `scripts/test-guard-shared-checkout.sh` (« Fire-Disposition — allowlist: zero
  entries », assertée).

**Ce qui n'est pas un détecteur ici, et pourquoi la distinction compte.** U1 et U2
sont du **code de production** : U1 pose une chaîne dans un prompt, U2 ajoute une
tentative. Aucun des deux n'a pour fonction primaire de signaler une violation —
U2 en particulier ne **refuse** jamais rien : il réessaie, puis laisse passer en
journalisant. C'est ce qui lui évite d'être lui-même un détecteur, et c'est
délibéré : un U2 qui échouerait sur section manquante aurait déplacé l'ESCALATE
d'une porte, ce que D2 refuse explicitement.

**Aucun plan existant n'est relu.** Ce travail ne touche ni `verify-pipeline.sh`
(D2) ni aucun chemin lisant `docs/plans/**` en masse. Les **808** plans sans
`## Fire-Disposition` (878 − 70, § Population) restent exactement ce qu'ils sont ;
aucune garde introduite ici ne les regarde.

---

## Suivi (hors périmètre, nommé)

**`mika-platform` — la prescription dans les commandes de groom.** Le remède
complet ajoute une étape `5c` à `/mika-groom-plan-only` (jumelle de la `5b` que
mika#1600 a posée pour `## Acceptance criteria`), la même à
`/mika-groom-ticket`, et une consigne à `/mika-revise-plan` pour que la branche
ITERATE sache écrire la section qu'on lui réclame. Ces trois fichiers vivent dans
`senara-solutions/mika-platform` (D1) : **ticket à ouvrir**, référencé dans le
corps de la PR.

**L'ordre n'est pas contraint, et c'est une propriété, pas un oubli.** U1 et U2
sont tous deux fail-safe et additifs : U1 ajoute du texte à un prompt, U2 ajoute
une tentative. Ni l'un ni l'autre ne refuse quoi que ce soit, donc livrer ce
ticket avant son suivi ne peut pas casser un grooming qui marchait. C'est la
différence exacte avec la garde CI de D2, dont l'ordre *aurait* été contraint —
et c'est une raison de plus de ne pas la livrer ici.

**Hors périmètre également :** la cause de la non-convergence du grooming de ce
ticket lui-même (commentaire opérateur du 18/09 : « re-armement différé épuisé,
noop_completion ») est un défaut de dispatch — famille mika#1124/#1172, Signal J
— sans rapport avec le contenu des plans.

**Défaut adjacent relevé en chemin, non traité :** `verify-pipeline.sh` ne
contrôle que **le premier** plan d'une PR (`PLAN_FILE=$(echo "$PLAN" | head -1)`,
l.109). Une PR modifiant deux plans n'en voit qu'un vérifié pour ses AC. Réel,
indépendant de ce ticket, **ticket de suivi**.

---

## Risques

| Risque | Portée | Traitement |
|---|---|---|
| Le prompt de groom s'allonge | ~10 lignes sur un prompt qui porte déjà le corps du ticket (≤ 16 KiB, mika#2178) et la règle mika#2211 | Négligeable et borné ; précédent direct |
| La garde relance un revise inutile | Coût : une session pilote | Trois contrôles négatifs (T6, T7, T8) ; prédicat conjonctif ; terminaison par compteur explicite (U3) |
| Le pilote de revise ne sait toujours pas écrire la section | La seconde tentative échoue | `fire_disposition_still_missing_after_retry` le dit, et **désigne le suivi `mika-platform`** plutôt qu'un troisième essai ici |
| La prescription dérive du gate architecte | Deux formulations de mika#1574 | La règle U1 cite la doctrine par référence (`mika#1574`, trois options nommées), elle ne la reformule pas |

---

## Références

- mika#1574 — Fire-Disposition Gate ; `docs/solutions/best-practices/fire-disposition-doctrine.md`
- mika#1600 / mika#1627 — le précédent `## Acceptance criteria` : même producteur tiers, même trou, remède en deux moitiés
- mika#2211 — `_PR_BODY_CONTAINMENT_RULE` : le précédent d'une règle injectée par `dispatch-lib` parce que le fichier de commande est hors de portée
- mika#2178 — le site d'injection dans `PROMPT` et ses trois invariants de position
- mika#1271 — le contrat deux-passes et la séparation pilote-contenu / dispatch-lib-convergence
- mika#2120 — `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
- mika#1415 — `_seed_worktree_slash_commands` : pourquoi les commandes ne sont pas dans ce dépôt
- mika#2286 — l'essai 5, dont ce ticket est le prérequis direct
