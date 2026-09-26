# Plan — les lecteurs `Fire-Disposition` tolèrent la numérotation, comme `Acceptance criteria`

**Ticket :** mika issue#2544
**Branche :** `fix/2544/grooming-gate-le-gate-fire-disposition`
**Type :** fix (substrat de boucle)
**Labels attendus :** `loop-substrate` (plafond de tours 200, mika#2542)

---

## Problème

Deux gates lisent un titre de section de plan produit par le même producteur —
`/ce:plan` plus un LLM — et ils ne tolèrent pas la même forme :

| gate | site | motif | numérotation |
|---|---|---|---|
| `Acceptance criteria` | `scripts/verify-pipeline.sh:173` | `^##[[:space:]]+([0-9]+\.?[[:space:]]+)?Acceptance criteria`, lu avec `-i` | **tolérée** (mika#2516, n=3) |
| `Fire-Disposition` | `skills/bundled/_shared/dispatch-lib.sh:6705` et `:6771` | `^## Fire-Disposition` | **rejetée** |

`/ce:plan` numérote ses titres de section (`## 0.` … `## 11.`) — c'est la cause
structurelle que mika#2516 a dû nommer après trois PR en une journée (#2509,
#2514, #2516). Le même producteur alimente les deux gates. Le second n'a jamais
reçu le correctif.

**Preuve, mesurée le 2026-09-26.** Pilote de groom `8712b2f8` (12:46:38Z) sur
mika#2542 : le plan livre un détecteur, donc la règle mika#2306 exige la section.
Le pilote a d'abord écrit `## 3. Fire-Disposition`, l'a corrigé en cours de
session, et le groom a tout de même échoué (review-anchor sur le brief
pré-correction).

---

## Ce que la lecture du code déplace dans le ticket

Quatre rectifications. Chacune change le remède, les AC, ou le périmètre — elles
sont donc le premier livrable de ce grooming et non une note de marge.

### R1 — `verify-pipeline.sh` ne lit pas `Fire-Disposition` du tout

Le corps du ticket ouvre sur « `verify-pipeline.sh` traite les deux gates de
section de plan de façon asymétrique ». `grep -n 'Fire-Disposition'
scripts/verify-pipeline.sh` rend **zéro ligne**. Aucun gate CI ne lit cette
section, sur aucune surface.

L'asymétrie est réelle mais elle est **entre deux fichiers et deux natures de
lecteur** : un gate CI (`verify-pipeline.sh`, tolérant depuis mika#2516) et le
rattrapage shell de mika#2306 (`dispatch-lib.sh`, strict). Conséquence pratique :
il n'y a **pas** de single-source possible entre les deux — `verify-pipeline.sh`
est lu par CI depuis le dépôt, `dispatch-lib.sh` est une projection du **binaire**
(mika#2340, `_shared/` seedé par `seed_bundled_skills_if_needed`). Le prix est
donc un doublon, à nommer plutôt qu'à mécaniser — précédent maison identique et
daté : la liste `{main, master}` de `cwd-guard`/`GIT_OPS_PROTECTED_BRANCHES`
(mika#2520), *« aucun single-source inter-langage n'existe ici, le prix est nommé
plutôt que mécanisé »*.

### R2 — les numéros de ligne du ticket sont périmés, et les vrais sites disent autre chose

Le ticket cite `dispatch-lib.sh:6607` et `:6673`. À HEAD `45340592`, la 6607 est
`[ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ]` et la 6673 est une ligne de
commentaire. Les deux vrais lecteurs sont **6705** (terme 2 du prédicat de
`_fd_retry_if_section_still_missing`) et **6771** (re-test de journalisation dans
la même fonction). Les deux sont dans **une seule fonction**, sept dizaines de
lignes d'écart — la forme exacte que mika#2516 a eue à réparer pour AC (*« The
pattern is matched at TWO sites seven lines apart »*).

### R3 — l'effet du strict est l'INVERSE de celui que le ticket décrit

Le ticket dit qu'un `## 3. Fire-Disposition` numéroté « est rejeté » et produit
ITERATE puis ESCALATE. Lu sur le code, le grep strict de `dispatch-lib.sh` ne
peut pas produire ce verdict — il n'émet aucune disposition. Ce qu'il produit :

```sh
# ligne 6705 — terme 2 du prédicat de rattrapage
! grep -qE '^## Fire-Disposition' "$plan_path" 2>/dev/null || return 0
```

Sur `## 3. Fire-Disposition`, le grep ne trouve rien, donc le terme 2 est
**vrai**, donc le rattrapage **s'arme sur un plan conforme**. Trois dommages, et
aucun n'est celui que le ticket annonce :

1. **Un pilote de revise entier est dépensé** pour ajouter une section déjà
   présente — tours, dollars, latence, et un créneau de dispatch.
2. **Le finding synthétique F-FD injecté dans `/mika-revise-plan` est faux** : il
   affirme « la section … est TOUJOURS ABSENTE du plan révisé » sur un plan qui la
   porte. Le pilote de revise se voit prescrire d'ajouter ce qui existe — risque
   de section en double, ou de révision au jugé.
3. **La ligne 6771 émet `fire_disposition_still_missing_after_retry`** — un
   événement dont le doc-comment déclare le régime attendu à zéro et dont une
   occurrence soutenue est censée dire *« le pilote de revise ne sait pas écrire la
   section »*. Sur une section numérotée il mesure faux. Classe mika#2205 : un
   instrument qui affirme avec autorité un état qui n'a pas eu lieu.

**Et il y a deux chaînes de déclenchement, pas une.** Le terme 1 est un `grep -qF`
grossier sur la chaîne `Fire-Disposition` dans les findings de première passe —
c'est documenté et voulu. Donc le défaut mord quand :

- **(C1)** l'architecte a réclamé la section (ITERATE F-FD), le revise l'ajoute
  **numérotée**, le rattrapage relance pour rien ;
- **(C2)** le plan portait déjà la section numérotée et l'architecte critique son
  **contenu** (« ton option (a) n'a pas d'assertion auto-nettoyante ») — les
  findings contiennent donc la chaîne, le terme 1 est vrai, et le rattrapage
  relance pour rien. **C2 est la chaîne la plus probable des deux**, et le ticket
  ne la voit pas.

### R4 — le verdict ESCALATE n'a jamais été mesuré ; le gate architecte est un LLM

Le ticket rapporte, en le citant comme mesure, que la section numérotée « aurait
produit ITERATE en 1re passe puis ESCALATE en 2e “sans recours” » — **mots du
pilote**, dit le ticket lui-même. Le pilote a corrigé avant ; aucun verdict n'a été
rendu sur une section numérotée. C'est une **crainte du producteur**, corroborée
par un vrai grep strict qui existe ailleurs, et non une mesure du gate.

Le gate qui rend ITERATE/ESCALATE vit dans deux prompts —
`skills/bundled/mika-arch-groom-ticket/system_prompt.md:73` et
`mika-arch-second-review/system_prompt.md:72` — et il est exécuté par un **LLM**,
qui n'applique aucune regex. Les deux prompts disent « a non-empty
`## Fire-Disposition` section » sans mentionner la numérotation ; le gate AC de ce
même prompt dit « a non-empty `## Acceptance criteria` section », **également sans
la mentionner**. Côté prompt, **l'asymétrie n'existe pas** : les deux sont muets.

Ce que ça change : la moitié strictement mesurée du défaut est le grep de
`dispatch-lib.sh`, et c'est elle qui porte le correctif. La moitié prompt est un
risque **plausible et non mesuré**, traité d'une phrase et justifié plus bas par
une asymétrie de **conséquence**, pas par une mesure.

---

## Cartographie complète des lecteurs de section de plan

Établie par lecture exhaustive, pour que le périmètre soit bordé par un inventaire
et non par une impression.

| # | site | nature | section | tolérance actuelle | touché ? |
|---|---|---|---|---|---|
| 1 | `verify-pipeline.sh:174` (`grep -qiE`) | gate CI | AC | numéro + casse | non (hors périmètre ticket) |
| 2 | `verify-pipeline.sh:184` (`sed -nE …/I`) | gate CI | AC | numéro + casse | non |
| 3 | `dispatch-lib.sh:6705` | prédicat de rattrapage | FD | **aucune** | **oui (U1)** |
| 4 | `dispatch-lib.sh:6771` | re-test de journalisation | FD | **aucune** | **oui (U1)** |
| 5 | `mika-arch-groom-ticket/system_prompt.md:73-88` | gate LLM (1re passe) | FD | prose muette | **oui (U3)** |
| 6 | `mika-arch-second-review/system_prompt.md:72-80` | gate LLM (2e passe) | FD | prose muette | **oui (U3)** |
| 7 | `dispatch-lib.sh:2584` (`_FIRE_DISPOSITION_RULE`) | prose injectée au producteur | FD | n/a (prescripteur) | non — refus raisonné (D4) |

Aucun autre lecteur : `grep -rn 'Fire-Disposition' scripts skills .github` ne rend
que des commentaires de plans antérieurs et ces sept sites.

---

## Décisions

### D1 — le motif est défini UNE fois et interpolé deux fois

C'est la leçon littérale de mika#2516, écrite dans son propre commentaire : *« Written
once because the two were written twice and drifted: mika#1639 had to fix both
together for case, and nothing was left holding them together. »* Les sites 3 et 4
sont dans la même fonction et ont exactement la forme qui a dérivé pour AC.

Le motif vit dans une constante de portée fichier de `dispatch-lib.sh`,
`_FD_HEADING_RE`, définie près de `_FIRE_DISPOSITION_RULE` (même famille
mika#2306, même voisinage de lecture), **pas** en `local` dans la fonction : la
constante doit être lisible par le scan de co-mutation (U4) via
`declare -p` / lecture de source sans dérouler le flot.

### D2 — tolérance : numérotation **et** casse, et la casse est justifiée par l'alignement

Motif retenu, copié terme pour terme sur `AC_HEADING_RE` :

```sh
_FD_HEADING_RE='^##[[:space:]]+([0-9]+\.?[[:space:]]+)?Fire-Disposition'
```
lu avec `grep -qiE` aux deux sites.

Quatre propriétés, chacune reprise de la décision mika#2516 :

- **préfixe de numérotation borné** à `<chiffres>[.]` — la forme mesurée. `## 1.1
  Fire-Disposition` et `## Phase 3 — Fire-Disposition` ne sont **pas** couverts :
  élargir sur une devinette est ce que la règle « quand NE PAS élargir » refuse.
  Une telle forme rougit visiblement, avec le bon message, et c'est un n+1 à
  mesurer.
- **`[[:space:]]+` au lieu de l'espace littéral** — élargissement supplémentaire
  assumé : `##&nbsp;&nbsp;Fire-Disposition` (double espace) matchera désormais.
  Nommé plutôt que découvert.
- **pas d'ancre `$`** — `## Fire-Disposition (option a)` doit continuer de
  matcher ; ancrer la fin resserrerait le gate en prétendant l'assouplir.
- **le texte reste ancré juste après le préfixe optionnel** — donc
  `## Notes on Fire-Disposition` reste refusé. Contrôle négatif obligatoire.

**La casse (`-i`) est incluse, et l'argument n'est pas une mesure.** Aucune
occurrence de `## fire-disposition` n'a été observée ; l'argument est
**l'alignement lui-même**, qui est l'objet du ticket : les deux gates lisent un
titre écrit par le même producteur, et les séparer sur la casse recréerait un cran
plus loin l'asymétrie qu'on ferme. Pour AC la casse est mesurée à n=2 (mika#1639)
sur un titre bien plus banal que `Fire-Disposition`, dont la majuscule interne est
inhabituelle et donc plus exposée. Le coût d'un élargissement de casse est nul en
faux positifs réalistes : `## fire-disposition` **est** la section. Ce raisonnement
est écrit au site, pour qu'un futur relecteur trouve l'argument plutôt que de
devoir le rejouer.

### D3 — le sens de l'asymétrie est fail-safe vers « la section est présente »

Un faux négatif du lecteur (ne pas voir une section présente) coûte un pilote de
revise, un finding mensonger et un événement faux — c'est le défaut mesuré. Un
faux positif (voir une section absente) coûte le retour au comportement d'avant
mika#2306, c'est-à-dire le risque ESCALATE que ce rattrapage existe pour réduire.
Les deux coûts sont réels, aucun n'est irréversible, et l'élargissement borné
ci-dessus réduit le premier sans créer de forme de faux positif plausible —
`## Notes on Fire-Disposition` reste refusé, ce qui est le seul faux positif
concevable et il est testé.

### D4 — on ne prescrit pas « ne numérote pas » au producteur

Le remède qui vient à l'esprit — ajouter à `_FIRE_DISPOSITION_RULE` une phrase
« écris le titre sans numéro » — est **refusé**, sur deux motifs :

1. Il est déjà réfuté à n=1 par le ticket : le pilote de #2542 a reçu cette règle
   et a numéroté quand même. `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
   couvre exactement cette classe.
2. Il ne répare aucun plan **déjà** numéroté, ni aucun revise qui renumérote ; le
   lecteur, lui, les répare tous. Doctrine déjà écrite à un fichier de là :
   *permissive detection, strict decision*.

### D5 — une phrase aux deux prompts architecte, et son statut est dit

Les deux gates LLM reçoivent une clause de vocabulaire : le titre peut porter un
numéro de section en tête, la section reste la même. Ce n'est **pas** de
l'enforcement par prompt — le prompt **est** le site de définition de ce gate, il
n'y en a pas d'autre ; la clause corrige une spécification, elle ne réprime pas un
comportement.

Pourquoi FD et pas AC, alors que les deux prompts sont muets : **asymétrie de
conséquence.** Un raté du gate AC rend ITERATE, puis le plan passe par un gate CI
dont le motif est tolérant — deux filets. Un raté du gate FD au **second** passage
rend ESCALATE, et le prompt lui-même écrit *« No ITERATE exists at second pass per
the two-pass limit »* : il n'y a aucun filet en aval. AC est nommé en suivi.

**Ce que cette moitié achète est l'intention. La moitié qui tient est U1** — et ce
qu'U1 tient est le rattrapage, pas le verdict.

### D6 — le scan de site unique couvre les DEUX sections, et referme une fausse garantie trouvée en chemin

`verify-pipeline.sh:166-167` affirme : *« a single-reader guard in
scripts/verify-pipeline-test.sh (mika#2516) reddens on a third literal spelling in
command position »*. Ce garde **n'existe pas** : `grep -n 'AC_HEADING_RE\|command
position\|spelling' scripts/verify-pipeline-test.sh` rend zéro ligne, et le fichier
ne contient aucun scan de source. C'est une garde revendiquée et absente —
mika#2304 (*« un champ qui affirme, avec autorité, l'override qui n'a pas eu
lieu »*), et mika#2205 (une garde non déployée se lit exactement comme un arbre
propre).

Puisque D1 crée pour FD le besoin d'un scan de co-mutation, U4 livre **un** scan
paramétré par nom de section qui couvre les deux — ce qui rend le commentaire de
`verify-pipeline.sh` vrai du même geste, sans y toucher. Le prédicat est
**positionnel** (le littéral en argument de `grep`/`sed`), jamais lexical : les
deux messages `echo "FAIL: … '## Acceptance criteria' …"` citent le littéral sans
être des lecteurs, et un scan lexical les ferait rougir en permanence, c'est-à-dire
serait désarmé le premier jour. Précédent du prédicat positionnel : mika#2496,
`skills/bundled/_shared/test-dispatch-lib.sh` (les trois lancements de
`claude-pilot`).

**Contre-vacuité obligatoire** : le scan doit asserter qu'il **voit** la population
(≥ 1 lecteur trouvé par section). Un scan qui ne regarde rien rend zéro violation.

---

## Unités d'implémentation

### U1 — `dispatch-lib.sh` : un motif, deux interpolations

**Fichier :** `skills/bundled/_shared/dispatch-lib.sh`

1. Définir `_FD_HEADING_RE` (valeur en D2) près de `_FIRE_DISPOSITION_RULE`
   (~ligne 2584), avec un commentaire portant : la cause structurelle (`/ce:plan`
   numérote), le renvoi à `AC_HEADING_RE` comme jumeau assumé et non mécanisé
   (mika#2340 empêche le partage), les quatre bornes du motif et **pourquoi
   chacune**, et l'interdiction de ré-orthographier le littéral ailleurs (tenue par
   U4).
2. Ligne 6705 → `! grep -qiE "$_FD_HEADING_RE" "$plan_path" 2>/dev/null || return 0`
3. Ligne 6771 → `if ! grep -qiE "$_FD_HEADING_RE" "$plan_path" 2>/dev/null; then`
4. **Piège de quoting, à écrire au site** : la constante est définie en quotes
   simples et interpolée en quotes doubles. Les deux sites ont impérativement
   besoin de leur drapeau ERE (`-E`) : sans lui, `+`, `?` et le groupe `( … )` sont
   des littéraux, le motif ne matche **rien**, et les deux termes deviennent
   toujours-vrais — le rattrapage se déclencherait sur **tout** plan dont les
   findings mentionnent la chaîne. C'est le mode de panne que mika#2516 a dû
   documenter pour son propre motif, et il est silencieux.
5. Mettre à jour le doc-comment de `_fd_retry_if_section_still_missing` : le terme 2
   se lit désormais « ne porte pas de titre `Fire-Disposition`, numéroté ou non ».

### U2 — les onze assertions existantes, et les nouvelles

**Fichier :** `skills/bundled/_shared/test-dispatch-lib.sh` (harness
`_t2306_revise_probe`, T1–T11, ~ligne 7482)

La harness prend déjà `$2 = t2306_has_section` ∈ `{yes, no}` et écrit
`## Fire-Disposition\n\nOption (a) …`. Étendre le paramètre à un **troisième
état** `numbered`, qui écrit `## 3. Fire-Disposition`, plutôt que de dupliquer la
harness. Le mode de comportement `second` (qui appende la section au second tour)
reste inchangé.

Nouvelles assertions — **T12** (le cœur du correctif) et ses contrôles :

| id | fixture | attendu |
|---|---|---|
| T12a | findings réclamant FD + section **numérotée** | **zéro relance**, aucun `fire_disposition_revise_retried` |
| T12b | même fixture, titre `## 3. FIRE-DISPOSITION` | zéro relance (tolérance de casse, D2) |
| T12c | findings réclamant FD + `## 11. Notes on Fire-Disposition` seul | **une** relance — la tolérance n'a pas avalé le refus |
| T12d | findings réclamant FD + `## 1.1 Fire-Disposition` | **une** relance — borne du préfixe assumée, pas un oubli |
| T12e | section numérotée, contrôle de l'événement | aucun `fire_disposition_still_missing_after_retry` |

**T12a est la fixture porteuse, et elle seule distingue un correctif complet d'un
correctif à moitié appliqué** — même rôle que T1 dans `verify-pipeline-test.sh`
(mika#2516). Avec seul le site 6705 élargi, T12a passe au vert **et** le site 6771
émet toujours l'événement faux sur le chemin nominal. T12e est donc obligatoire et
non décorative : c'est elle qui voit la moitié oubliée.

T7 (« section déjà présente ⇒ zéro relance ») reste tel quel : c'est le contrôle
négatif de la forme non numérotée, et il doit rester vert pour que T12a prouve un
élargissement et non un désarmement.

### U3 — la clause de vocabulaire dans les deux gates architecte

**Fichiers :** `skills/bundled/mika-arch-groom-ticket/system_prompt.md` (§ *Fire-Disposition
Gate*, ~l.73) et `skills/bundled/mika-arch-second-review/system_prompt.md` (~l.72)

Une phrase par fichier, dans l'arbre de décision du gate : le titre de la section
peut porter un numéro de section en tête (`## 3. Fire-Disposition`) parce que le
producteur de plans numérote ses titres ; la section est la même et le gate passe.
Formulée sur le **vocabulaire**, pas en injonction au groomeur.

Assertions correspondantes dans `test-dispatch-lib.sh` (les prompts bundled y sont
déjà lus par d'autres tests) : les deux fichiers contiennent la clause. C'est une
assertion de présence, pas de comportement — le comportement d'un LLM n'est pas
testable déterministement, et c'est dit dans le contrat de vérification.

### U4 — le scan de site unique et de co-mutation

**Fichier :** `skills/bundled/_shared/test-dispatch-lib.sh` (nouveau bloc, après
T12 ; `REPO_ROOT` y existe déjà à ~l.5049, donc la lecture de
`$REPO_ROOT/scripts/verify-pipeline.sh` ne demande aucune plomberie neuve)

Quatre assertions :

1. **Site unique FD** — aucune ligne de `dispatch-lib.sh` n'interroge le littéral
   `Fire-Disposition` **en position de motif** (`grep`/`sed`) autrement que via
   `"$_FD_HEADING_RE"`. Exclusions nommées : `_FIRE_DISPOSITION_RULE` et le corps
   du finding synthétique (prose, pas motifs), et le terme 1 du prédicat, qui est
   un `grep -qF` sur les **findings** et non sur le plan — population différente,
   délibérément grossière, documentée comme telle.
2. **Site unique AC** — même prédicat positionnel sur `verify-pipeline.sh` avec
   `Acceptance criteria` : tout `grep`/`sed` interrogeant ce littéral doit passer
   par `"$AC_HEADING_RE"`. **Ceci rend vraie l'affirmation de
   `verify-pipeline.sh:166-167`** (D6).
3. **Co-mutation** — la sous-chaîne de préfixe de numérotation
   (`([0-9]+\.?[[:space:]]+)?`) est **littéralement identique** dans `_FD_HEADING_RE`
   et dans `AC_HEADING_RE`. C'est ce qui remplace le single-source impossible
   (R1) : le jour où l'un des deux évolue, ce test rougit au lieu de laisser les
   deux gates redivergents en silence.
4. **Contre-vacuité** — au moins un lecteur trouvé par section, et
   `_FD_HEADING_RE` non vide. Sans elle, un renommage de constante ou un
   déplacement de fichier rend un scan silencieusement inerte, qui se lit exactement
   comme un arbre propre (mika#2205).

### U5 — documentation

- `docs/solutions/workflow-issues/verify-pipeline-ac-heading-case-insensitive-2026-06-30.md` :
  ajouter mika#2544 comme **n=4 de la classe**, en notant que cette occurrence
  porte sur une **autre section et un autre fichier** — la classe n'est pas « le
  motif AC est trop strict » mais « un lecteur de titre de section de plan qui ne
  tolère pas la numérotation du producteur ». C'est cette formulation qui rend la
  classe récurrente lisible.
- Aucune entrée `CLAUDE.md` : ce correctif ne crée ni réglage, ni événement, ni
  surface opérateur nouvelle (voir *Ce que ce travail n'achète pas*).

---

## Contrat de vérification

| # | contrôle | comment |
|---|---|---|
| V1 | `make test-dispatch-lib` vert, T1–T11 **inchangés** | job CI existant (`ci.yml:85`) |
| V2 | T12a–T12e passent, et T12a **vue rouge** avant U1 | l'implémenteur applique U2 avant U1 et le note |
| V3 | T12e **vue rouge** avec le seul site 6705 élargi | c'est la moitié-appliquée que V2 ne voit pas |
| V4 | T12c/T12d **vues vertes** avant U1 (la borne existait déjà) et après | prouve que la tolérance n'a pas avalé le refus |
| V5 | U4-3 **vue rouge** en désalignant un caractère du préfixe dans l'un des deux motifs | la co-mutation mord réellement |
| V6 | U4-2 **vert sans modifier `verify-pipeline.sh`** | si elle rougit, un second lecteur AC existe : le nommer avant tout élargissement |
| V7 | `bash -n skills/bundled/_shared/dispatch-lib.sh` et `shellcheck` sans régression | le quoting de D2/U1-4 est le piège du correctif |
| V8 | `make verify-bundled-skills` vert | U3 touche deux bundles |

**Ce qui n'est PAS testable ici, écrit plutôt que découvert.** « Le gate architecte
accepte un titre numéroté » est exécuté par un LLM, contre un fournisseur réel. Le
contrat côté mika est *la clause est dans le prompt servi*, et U3 l'atteste
déterministement. La moitié comportementale est la sonde S2.

---

## Fire-Disposition

Ce plan livre des détecteurs : les assertions T12a–T12e (U2) et le scan de source à
quatre termes (U4). **Option (a) — exception nommée en allowlist, table livrée
vide et vacuité assertée à l'exécution.**

Modèle exact : `T2306_ARCH_ASK_ALLOWLIST` (T9, `test-dispatch-lib.sh` ~l.7768) et
`scripts/test-guard-shared-checkout.sh`. Le scan d'U4 porte une table d'exceptions
`T2544_HEADING_READER_ALLOWLIST=()` dont le test **asserte qu'elle compte zéro
entrée**. Elle rougit donc le jour où une exception est ajoutée, pas seulement
quand elle devient stale — et une table vide assertée vide est ce qui distingue
« aucune violation » de « le scan ne regarde rien ».

**Violations préexistantes : zéro, et c'est établi par lecture, pas supposé.**
L'inventaire du § *Cartographie* est exhaustif ; les deux populations du scan
comptent respectivement 2 lecteurs FD (tous deux passant par la constante après U1)
et 2 lecteurs AC (tous deux passant par `AC_HEADING_RE` aujourd'hui). Les deux
`echo "FAIL: …"` de `verify-pipeline.sh` citent le littéral sans être des lecteurs
et sont hors population par le prédicat **positionnel** de D6 — ce n'est pas une
exception, c'est la définition de la population.

**Résolution quand le scan tire : on route le site vers la constante, on n'ajoute
pas de ligne à l'allowlist** (doctrine mika#2201). Un lecteur qu'on ne veut pas
router vers la constante est un lecteur à supprimer.

Toute entrée future doit porter **les trois** propriétés de l'option (a) — nommer
la donnée précise, référencer un ticket de suivi, porter une assertion
auto-nettoyante — et non la seule troisième.

---

## Surfaces opérateur et sondes

**Aucune surface nouvelle.** Ce correctif ne crée ni variable d'environnement, ni
compteur, ni événement de journal. Ce qu'il change est le **régime attendu de deux
événements existants**, et c'est là que se lit son effet :

```bash
# 1. Le rattrapage s'arme-t-il encore sur des plans conformes ?
grep -h 'fire_disposition_revise_retried' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail

# 2. L'événement dont le régime attendu est ZÉRO
grep -h 'fire_disposition_still_missing_after_retry' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail
```

**Ancrage obligatoire sur `^dispatch-lib: `** si le préfixe est présent, et de
toute façon lecture par fichier : ce `.stderr` porte aussi la prose du pilote, et
une session qui **discute** de ces jetons — comme celle qui a groomé ce ticket —
produit un faux positif mesuré (mika#2050).

| événement | régime attendu **après** ce correctif | lecture |
|---|---|---|
| `fire_disposition_revise_retried` | **rare**, et strictement décroissant | chaque ligne restante est un plan qui n'a réellement pas la section |
| `fire_disposition_still_missing_after_retry` | **zéro** | une occurrence dit que `/mika-revise-plan` ne sait pas écrire la section — suivi mika-platform, **jamais** un troisième essai ici |

### Sondes post-déploiement, et leurs quatre haltes

> **Préalable, non négociable.** `skills/bundled/_shared/` est une projection du
> **binaire**, pas du checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer`
> doit porter le sha qu'on vient de bâtir. **Sans cette vérification, chacune des
> sondes ci-dessous décrit le binaire d'hier.**

**S1 — le défaut mesuré ne se reproduit pas (premier groom livrant un détecteur
après déploiement).** Un plan dont la section arrive numérotée ne déclenche ni
`fire_disposition_revise_retried` ni le second événement.

> **Halte 1 — l'un des deux apparaît sur un plan qui PORTE la section.** Le
> correctif n'a pas pris. **Ne pas élargir le motif par réflexe** : établir d'abord
> le déploiement (préalable ci-dessus), puis vérifier que **les deux** sites lisent
> la constante — c'est la moitié-appliquée que V3 existe pour voir.

**S2 — la moitié LLM (7 jours).** Aucun ESCALATE de second passage motivé par une
section `Fire-Disposition` absente alors que le plan en porte une, numérotée.

> **Halte 2 — un tel ESCALATE apparaît.** C'est la moitié prompt qui ne tient pas,
> **et c'est attendu comme possible** (D5). Ne pas durcir le prompt par réflexe :
> le seul levier structurel est de faire lire la section par du code avant le second
> passage, ce qui est un ticket à part, à ouvrir avec cette occurrence en
> précondition.

**S3 — contrôle positif, obligatoire.** `fire_disposition_revise_retried` doit
rester **non nul** sur la population qui le mérite : un plan livrant un détecteur
et n'ayant réellement aucune section.

> **Halte 3 — zéro des deux événements sur 7 jours.** On ne peut **rien** conclure.
> Vérifier qu'un groom a réellement convergé dans la fenêtre. *Un rattrapage
> silencieusement désarmé se lit exactement comme un rattrapage qui n'a rien à
> faire* (mika#2205) — et le mode de panne d'U1-4 (ERE manquant) produit très
> exactement l'inverse, un rattrapage qui tire sur tout : si les deux événements
> explosent, c'est là qu'il faut regarder avant le motif.

**S4 — la classe, pas l'occurrence.** Une nouvelle forme de titre non couverte
(`## 1.1 …`, `## Phase 3 — …`) rougit visiblement en T12d/T12c, avec le bon
message.

> **Halte 4 — une telle forme arrive en production.** C'est un n+1 **à mesurer**,
> pas à absorber : élargir le préfixe est ce que D2 refuse explicitement sans
> mesure, et l'élargissement devra bouger les **deux** motifs ensemble (U4-3
> rougira sinon, ce qui est son travail).

---

## Ce que ce travail n'achète pas

- **Il ne fait pas passer un gate architecte.** Le verdict reste rendu par un LLM
  sur un prompt ; U3 corrige la spécification de ce gate, rien de plus.
- **Il ne répare pas le groom de mika#2542**, dont l'échec est attribué par le
  ticket à un review-anchor sur le brief pré-correction — une cause distincte que
  ce plan ne touche pas et ne prétend pas fermer.
- **Il n'ajoute aucun compteur ni aucun événement.** Le seul instrument est le
  changement de régime de deux événements existants, et **leur silence ne prouve
  rien tant que S3 n'est pas établi**.
- **Il ne rend pas le réglage observable.** Aucune ligne n'émet le motif en
  vigueur ; « quelle tolérance ce dispatch a-t-il réellement appliquée ? » reste
  une question à laquelle on répond en lisant le binaire déployé, pas un grep.
  Assumé pour un prédicat dont les deux issues sont déjà journalisées.

---

## Definition of Done

- [ ] `_FD_HEADING_RE` défini une fois et interpolé aux deux sites, avec les quatre
      bornes et le piège ERE écrits au site (U1)
- [ ] Le troisième état `numbered` de `_t2306_revise_probe` et T12a–T12e (U2)
- [ ] T12a vue **rouge** avant U1 ; T12e vue **rouge** sur un correctif à moitié
      appliqué (V2, V3) — noté dans le corps de PR
- [ ] La clause de vocabulaire dans les deux prompts architecte + assertions de
      présence (U3)
- [ ] Le scan à quatre termes, table d'exceptions **vide et assertée vide**, avec
      contre-vacuité (U4)
- [ ] U4-3 vue **rouge** en désalignant un caractère (V5)
- [ ] U4-2 verte **sans toucher** `verify-pipeline.sh` (V6)
- [ ] `make test-dispatch-lib`, `bash -n`, `shellcheck`, `make verify-bundled-skills`
      verts (V1, V7, V8)
- [ ] `docs/solutions/…-ac-heading-case-insensitive-2026-06-30.md` porte mika#2544
      en n=4 avec la classe reformulée (U5)
- [ ] Corps de PR écrit **sous le worktree** (`pr-body.md`), passé en
      `--body-file`, supprimé ensuite (mika#2211)
- [ ] Suivis ouverts : la clause de vocabulaire pour le gate **AC** des deux prompts
      architecte ; la lecture par code de la section avant le second passage
      (précondition : halte 2)

---

## Acceptance criteria

Dérivés du § *Remède* du ticket et du contrat de vérification — le corps de
mika#2544 ne porte pas de section `## Acceptance criteria`.

- **AC1** — Les deux lecteurs `Fire-Disposition` de `dispatch-lib.sh` reconnaissent
  un titre numéroté (`## 3. Fire-Disposition`), avec la **même** tolérance de
  préfixe que `AC_HEADING_RE` de `verify-pipeline.sh`.
- **AC2** — Le motif est défini à **un seul endroit** et interpolé aux deux sites ;
  un troisième lecteur littéral en position de motif fait rougir le build.
- **AC3** — La tolérance ne dégrade aucun refus : `## Notes on Fire-Disposition`
  reste refusé, et le comportement sur un titre non numéroté est **inchangé**
  (T7 reste vert).
- **AC4** — Un correctif à moitié appliqué (un seul des deux sites) est **détecté**
  par une assertion, et non par l'absence de plainte.
- **AC5** — La co-mutation entre le motif FD et le motif AC est tenue par une
  assertion : désaligner l'un des deux préfixes fait rougir le build.
- **AC6** — L'affirmation de `verify-pipeline.sh:166-167` sur l'existence d'un garde
  de lecteur unique pour AC devient **vraie**, sans modifier
  `verify-pipeline.sh` ni le prédicat AC.
- **AC7** — Les deux gates architecte nomment la tolérance de numérotation dans
  leur § *Fire-Disposition Gate*.
- **AC8** — La documentation de la classe récurrente porte mika#2544 comme n=4,
  avec la classe formulée sur « un lecteur de titre de section de plan » et non sur
  la seule section AC.
- **AC9** — Aucune valeur de réglage, aucun événement et aucune variable
  d'environnement ne sont créés ou déplacés.

---

## Hors périmètre, délibérément

- **Le prédicat `Acceptance criteria` lui-même** — déjà corrigé (mika#2516), et
  exclu par le corps du ticket. U4-2 **lit** ses lecteurs, il n'en change aucun.
- **La clause de vocabulaire pour le gate AC des deux prompts architecte** — même
  absence que pour FD, mais AC dispose d'un filet CI en aval que FD n'a pas (D5).
  **Suivi**, sans précondition : c'est une inconsistance connue, pas une
  hypothèse.
- **Élargir le préfixe à `## 1.1` / `## Phase 3 — `** — refusé sans mesure (D2),
  et la borne est **testée** pour que la prochaine occurrence rougisse avec le bon
  message plutôt que de passer en silence.
- **Prescrire au producteur de ne pas numéroter** — refusé avec sa raison (D4).
- **Le review-anchor sur le brief pré-correction**, seconde cause de l'échec du
  groom #2542 selon le ticket — cause distincte, ticket distinct.
- **Faire lire la section par du code avant le second passage architecte** — le
  seul levier structurel sur la moitié LLM, blast radius large (il faudrait décider
  qui rend le verdict), **suivi** conditionné à la halte 2.
- **Le terme 1 du prédicat de rattrapage** (`grep -qF` sur les findings de première
  passe) — délibérément grossier, documenté comme tel, et resserrer ce terme
  changerait la population du rattrapage plutôt que sa lecture du plan.
