# Plan — les lecteurs `Fire-Disposition` tolèrent la numérotation, comme `Acceptance criteria`

**Ticket :** mika issue#2544
**Branche :** `fix/2544/grooming-gate-le-gate-fire-disposition`
**Type :** fix (substrat de boucle)
**Labels attendus :** `loop-substrate` (plafond de tours 200, mika#2542)

> **Note de re-groom (troisième passage).** Ce fichier est réécrit, pas doublé —
> un seul plan par ticket, sans quoi `_find_issue_plan` élirait l'un des deux au
> tri et l'architecte réviserait un plan qui argumente contre un corps qui ne dit
> plus ce qu'il contredit. Le deuxième passage avait déjà réancré le plan sur le
> corps rectifié du ticket. **Ce troisième passage n'a révisé aucune décision :
> toutes les mesures de la version précédente ont été re-vérifiées à HEAD et sont
> exactes.** Ce qu'il ajoute est six corrections de *prescription* — des endroits
> où le plan disait à l'implémenteur de vérifier la mauvaise chose (B1–B6,
> § *Ce que le troisième passage corrige*).

---

## Problème

Deux gates lisent un titre de section de plan produit par le même producteur —
`/ce:plan` plus un LLM — et ils ne tolèrent pas la même forme :

| gate | site | motif | numérotation |
|---|---|---|---|
| `Acceptance criteria` | `scripts/verify-pipeline.sh`, constante `AC_HEADING_RE` | `^##[[:space:]]+([0-9]+\.?[[:space:]]+)?Acceptance criteria`, lu avec `-i` | **tolérée** (mika#2516, n=3) |
| `Fire-Disposition` | `skills/bundled/_shared/dispatch-lib.sh`, fonction `_fd_retry_if_section_still_missing`, deux `grep` | `^## Fire-Disposition` | **rejetée** |

`/ce:plan` numérote ses titres de section (`## 0.` … `## 11.`) — c'est la cause
structurelle que mika#2516 a dû nommer après trois PR en une journée (#2509,
#2514, #2516). Le même producteur alimente les deux gates. Le second n'a jamais
reçu le correctif.

**Preuve, mesurée le 2026-09-26.** Pilote de groom `8712b2f8` (12:46:38Z) sur
mika#2542 : le plan livre un détecteur, donc la règle mika#2306 exige la section.
Le pilote a d'abord écrit `## 3. Fire-Disposition`, l'a corrigé en cours de
session, et le groom a tout de même échoué (review-anchor sur le brief
pré-correction). **L'asymétrie n'est donc pas établie comme cause de cet échec ;
elle est établie comme piège présent** — le ticket le dit lui-même, et ce plan ne
revendique rien de plus.

**Illustration involontaire, et elle vaut d'être notée.** Ce plan-ci livre des
détecteurs, donc il porte la section exigée — et il a dû l'écrire **sans numéro**,
parce que le code d'aujourd'hui est celui qu'il corrige. Un plan qui numérote sa
propre section `Fire-Disposition` paie le défaut pendant qu'il le décrit.

---

## Ce que la lecture du code établit

Le corps du ticket porte déjà sa propre rectification (§ *Rectification du corps
précédent*), et les mesures ci-dessous **la confirment** plutôt qu'elles ne la
corrigent. Ce paragraphe sépare donc ce qui est déjà au ticket de ce que ce
grooming ajoute — pour qu'un relecteur n'ait pas à trancher lequel des deux est à
jour.

### Confirmé par mesure (le ticket le dit déjà)

- **`verify-pipeline.sh` ne lit pas `Fire-Disposition`.**
  `grep -c 'Fire-Disposition' scripts/verify-pipeline.sh` → **0**. Aucun gate CI
  ne lit cette section, sur aucune surface.
- **Les deux `grep` ne produisent aucun verdict.** Ils n'émettent ni ITERATE ni
  ESCALATE — ce verdict est rendu par un LLM, dans deux prompts. Les `grep`
  décident d'une **relance de pilote de revise** et d'une **ligne de journal**.
- **Le gate architecte n'est pas mesuré.** Les deux prompts nomment la forme
  littérale ; aucun verdict n'a jamais été rendu sur une section numérotée. C'est
  un risque plausible, pas une mesure — et il est traité comme tel (D5).

### Ce que ce grooming ajoute

#### A1 — les numéros de ligne du ticket sont périmés, et c'est structurel

Le ticket localise ses lecteurs par nom de fonction, ce qui est juste. Mais une
ébauche antérieure de ce plan citait `dispatch-lib.sh:6705` et `:6771` ; les deux
vrais sites sont **6825** et **6891** à HEAD `f9ca764c`, et `_FIRE_DISPOSITION_RULE`
a glissé de 2584 à **2682**. Trois merges sur `main` (#2549, #2550, #2551) ont
décalé le fichier entre deux groomings.

**Conséquence tenue dans tout ce plan : aucun numéro de ligne n'est normatif.**
Les sites sont désignés par nom de fonction et de constante
(`_fd_retry_if_section_still_missing`, `_FIRE_DISPOSITION_RULE`, `AC_HEADING_RE`),
qui sont stables sous rebase. Les numéros n'apparaissent que comme repères de
lecture, jamais comme cible d'édition.

#### A2 — il y a deux chaînes de déclenchement, et le ticket n'en voit qu'une

Le terme 1 du prédicat est un `grep -qF` grossier sur la chaîne `Fire-Disposition`
dans les findings de **première passe** — documenté et voulu. Le défaut mord donc
dans deux cas :

- **(C1)** l'architecte a réclamé la section (ITERATE F-FD), le revise l'ajoute
  **numérotée**, le rattrapage relance pour rien ;
- **(C2)** le plan portait **déjà** la section numérotée et l'architecte critique
  son **contenu** (« ton option (a) n'a pas d'assertion auto-nettoyante ») — les
  findings contiennent donc la chaîne, le terme 1 est vrai, et le rattrapage
  relance pour rien.

**C2 est la chaîne la plus probable des deux**, et elle n'exige aucune ITERATE
préalable sur la section elle-même. Le ticket ne la nomme pas ; les fixtures la
couvrent (T12f).

#### A3 — les trois dommages, dont un est un instrument qui mesure faux

Sur `## 3. Fire-Disposition`, le terme 2 est **vrai**, donc le rattrapage s'arme
**sur un plan conforme** :

1. **Un pilote de revise entier est dépensé** pour ajouter une section déjà
   présente — tours, dollars, latence, un créneau de dispatch.
2. **Le finding synthétique F-FD est faux** : il affirme « la section … est
   TOUJOURS ABSENTE du plan révisé » sur un plan qui la porte. Le pilote de revise
   se voit prescrire d'ajouter ce qui existe — risque de section en double.
3. **`fire_disposition_still_missing_after_retry` est émis à tort.** Son
   doc-comment déclare le régime attendu à zéro et fait d'une occurrence soutenue
   la preuve que *« le pilote de revise ne sait pas écrire la section »*. Sur une
   section numérotée, il mesure faux. Classe mika#2205 : un instrument qui affirme
   avec autorité un état qui n'a pas eu lieu.

#### A4 — la garde de lecteur unique promise pour AC n'existe pas, et son fichier n'est pas en CI

`verify-pipeline.sh` affirme, dans le commentaire de `AC_HEADING_RE` :

> *Do not re-spell this pattern anywhere else; a single-reader guard in
> `scripts/verify-pipeline-test.sh` (mika#2516) reddens on a third literal
> spelling in command position.*

**Deux défauts empilés, tous deux mesurés :**

- `grep -n 'AC_HEADING_RE\|single-reader\|command position\|spelling' scripts/verify-pipeline-test.sh`
  → **zéro ligne**. La garde n'a pas été livrée (le plan de mika#2516 la prévoyait
  bien, à son § 3.1). C'est une garde **revendiquée et absente** — mika#2304 (*un
  champ qui affirme, avec autorité, l'override qui n'a pas eu lieu*) et mika#2205
  (une garde non déployée se lit exactement comme un arbre propre).
- Et même livrée, elle n'aurait rien tenu : **`verify-pipeline-test.sh` n'est
  lancé par aucun job CI ni aucune cible `make`**. `grep -rn 'verify-pipeline-test'`
  sur `Makefile` et `.github/workflows/` rend **zéro appelant** (re-mesuré à HEAD) ;
  le seul harnais shell câblé est `test-dispatch-lib.sh` (`Makefile:182`,
  `ci.yml:85`). `verify-pipeline.sh` lui-même **est** en CI (`ci.yml:452`), donc le
  gate tourne ; son harnais de test, non.

**Ce que ça décide.** Le garde de co-mutation doit vivre dans
`test-dispatch-lib.sh` — non par commodité (« `REPO_ROOT` y existe déjà »), mais
parce que **c'est le seul des deux harnais que CI exécute**. Le placer dans le
fichier que le commentaire nomme serait livrer un détecteur structurellement
désarmé. Et puisqu'il ne peut pas y vivre, **le commentaire doit être corrigé pour
nommer le bon fichier** : le laisser tel quel enverrait un futur lecteur chercher
une garde dans un harnais que personne ne lance, c'est-à-dire remplacerait une
fausse garantie par une autre.

---

## Ce que le troisième passage corrige

Six prescriptions envoyaient l'implémenteur vérifier la mauvaise chose. Aucune
décision n'est révisée ; ce sont des corrections de **ce qu'il faut faire**, pas
de **pourquoi**.

#### B1 — les deux sites portent DÉJÀ `-E` ; le risque est de le perdre, pas de l'oublier

Forme littérale mesurée à HEAD, aux deux sites :

```sh
! grep -qE '^## Fire-Disposition' "$plan_path" 2>/dev/null || return 0     # terme 2
if ! grep -qE '^## Fire-Disposition' "$plan_path" 2>/dev/null; then        # journalisation
```

Le changement est donc **`-qE` → `-qiE`**, jamais « ajouter `-E` ». La version
précédente de ce plan décrivait le piège comme « les deux sites ont impérativement
besoin de leur drapeau ERE », ce qui est vrai et **envoie vérifier la mauvaise
chose** : un implémenteur qui contrôle « `-E` est-il présent ? » trouvera oui dans
les deux cas, y compris sur la régression réelle. **La régression réelle est
d'écrire `-qi` en remplaçant `-qE`**, ce qui perd l'ERE silencieusement : `+`, `?`
et le groupe `( … )` redeviennent littéraux, le motif ne matche **rien**, les deux
termes deviennent toujours-vrais, et le rattrapage se déclenche sur **tout** plan
dont les findings mentionnent la chaîne. Le contrôle juste est donc sur le
**drapeau combiné** : `-qiE` aux deux sites, et c'est ce que V7 vérifie.

#### B2 — la clause d'U3 s'écrit en anglais

Les deux prompts architecte sont intégralement anglophones (mesuré : *« A plan
that includes detector-class deliverables … MUST return `ITERATE` — never
`READY`. »*). La version précédente prescrivait « une phrase par fichier » sans
nommer la langue, ce qui invite une clause française dans un prompt anglais servi
à un LLM. **La clause est en anglais**, dans le registre du fichier.

#### B3 — le précédent du fichier SAUTE quand le fichier est absent, et le copier désarmerait U4

`test-dispatch-lib.sh` contient déjà un test qui lit un fichier par `REPO_ROOT` —
T2211, sur `$REPO_ROOT/.claude/commands/mika.md`. Sa forme est :

```sh
if [ -f "$T2211_MIKA_CMD" ]; then
    …assertions…
fi
```

C'est-à-dire : **fichier absent ⇒ aucune assertion, et le test passe au vert.**
Copier ce précédent pour U4-2 (qui lit `$REPO_ROOT/scripts/verify-pipeline.sh`)
livrerait un scan qui se désarme silencieusement dès que le harnais tourne hors du
checkout — et un scan silencieusement inerte se lit exactement comme un arbre
propre (mika#2205), ce que la contre-vacuité d'U4-4 existe précisément pour
fermer. **Divergence délibérée** : U4 rougit sur fichier illisible, avec un
message qui dit *« fichier introuvable »* et non *« zéro lecteur »* — les deux
diagnostics envoient chercher des choses opposées.

#### B4 — le gate `pr-body-validation` est anglophone, et ce plan liste quatre suivis

`scripts/check-pr-body-consistency.sh` porte un `TRIGGER_PATTERN` **en anglais** :
`follow-up PR|will be (handled|done|fixed) in a (separate|follow-up|follow up) (PR|issue)|…|addressed in a follow-up`.
Toute occurrence dans le corps de PR **exige** une ligne `Tracked in: …#<N>` avec
un numéro réel, sinon le job `pr-body-validation` **échoue dur**.

Ce plan nomme quatre suivis. Deux conduites sûres, et une seule est à choisir :
soit le corps nomme les suivis **en français** (« suivi », « ticket de suivi »),
hors de la population du motif ; soit il les nomme en anglais **et** porte une
ligne `Tracked in: senara-solutions/mika#<N>` par suivi réellement ouvert.
**Inventer un numéro est pire que les deux** : la ligne satisfait le gate en
désignant un ticket qui n'existe pas.

#### B5 — le plan-fixture porte déjà `## Acceptance criteria` ; l'extension doit la garder

`_t2306_revise_probe` écrit, après le bloc conditionnel de la section FD :

```sh
printf '## Acceptance criteria\n\nAC1 — la sonde tourne.\n'
```

L'extension du paramètre (U2) remplace un `if` par un `case` **au-dessus** de
cette ligne, qui reste inchangée. La noter évite qu'une réécriture du bloc
l'emporte au passage et casse tout futur gate AC sur ce fixture.

#### B6 — cinq états supplémentaires, pas trois

La version précédente annonçait « trois états supplémentaires » puis en listait
cinq (`numbered`, `upper`, `ish`, `notes`, `subsub`). Le compte est **cinq**, plus
les deux existants (`yes`, `no`), soit sept états pour sept fixtures.

---

## La tolérance, mesurée plutôt que supposée

Motif candidat, copié terme pour terme sur `AC_HEADING_RE` :

```sh
_FD_HEADING_RE='^##[[:space:]]+([0-9]+\.?[[:space:]]+)?Fire-Disposition'
```

Mesuré avec `grep -qiE` sur quinze fixtures (`.pilot-scratch/fd-probe/measure.sh`
pendant un grooming antérieur ; chaque ligne re-dérivée à la lecture du motif) :

| fixture | lu | statut |
|---|---|---|
| `## Fire-Disposition` | **présent** | cas nominal, ticket ✓ |
| `## 3. Fire-Disposition` | **présent** | **le défaut fermé**, ticket ✓ |
| `## 3 Fire-Disposition` | **présent** | sans point, ticket ✓ |
| `## 11. Fire-Disposition` | **présent** | deux chiffres |
| `## fire-disposition` | **présent** | tolérance de casse (D2) |
| `##   Fire-Disposition` | **présent** | espaces multiples — élargissement assumé |
| `## Fire-Disposition (option a)` | **présent** | doit matcher : pas d'ancre de fin |
| `## Fire-Disposition Gate` | **présent** | idem |
| `### Fire-Disposition` | absent | ticket ✓ — le `#` qui suit `^##` n'est pas un `[[:space:]]` |
| `##Fire-Disposition` | absent | pas un titre Markdown (espace requis) |
| `` la section `## Fire-Disposition` est requise `` | absent | ticket ✓ — refusé par l'ancre `^` |
| `## Notes on Fire-Disposition` | absent | seul faux positif concevable, refusé |
| `## 1.1 Fire-Disposition` | absent | borne du préfixe, **assumée** (D2) |
| `## Fire-Disposition-ish` | **présent** | **divergence avec le ticket — voir D3** |

Deux de ces lignes méritent leur mécanisme écrit, parce qu'il n'est pas celui
qu'on suppose. `## 1.1 Fire-Disposition` : le groupe de préfixe consomme `1` puis
`.`, exige ensuite `[[:space:]]+` et trouve `1` — le groupe échoue ; sans le
groupe il faudrait `Fire-Disposition` juste après `## ` et on a `1.1`. Refusé par
**arithmétique du motif**, pas par une borne explicite. `## Notes on Fire-Disposition` :
refusé parce que le texte reste ancré juste après le préfixe optionnel, et c'est
la propriété que le contrôle négatif T12c existe pour tenir.

---

## Cartographie complète des lecteurs de section de plan

Établie par lecture exhaustive, pour que le périmètre soit bordé par un inventaire
et non par une impression. `grep -rn 'Fire-Disposition' scripts skills .github` ne
rend que des commentaires de plans antérieurs et ces sept sites.

| # | site (par nom) | nature | section | tolérance | touché ? |
|---|---|---|---|---|---|
| 1 | `verify-pipeline.sh` · `grep -qiE "$AC_HEADING_RE"` | gate CI | AC | numéro + casse | non |
| 2 | `verify-pipeline.sh` · `sed -nE "/$AC_HEADING_RE/I,…"` | gate CI | AC | numéro + casse | non |
| 3 | `dispatch-lib.sh` · `_fd_retry_if_section_still_missing`, terme 2 | prédicat de relance | FD | **aucune** | **oui (U1)** |
| 4 | `dispatch-lib.sh` · même fonction, re-test de journalisation | observabilité | FD | **aucune** | **oui (U1)** |
| 5 | `mika-arch-groom-ticket/system_prompt.md` · § *Fire-Disposition Gate* | gate LLM (1re passe) | FD | prose muette | **oui (U3)** |
| 6 | `mika-arch-second-review/system_prompt.md` · § *Fire-Disposition Gate* | gate LLM (2e passe) | FD | prose muette | **oui (U3)** |
| 7 | `dispatch-lib.sh` · `_FIRE_DISPOSITION_RULE` | prose injectée au producteur | FD | n/a (prescripteur) | non — refus raisonné (D5) |

Les sites 3 et 4 sont dans **une seule fonction**, six dizaines de lignes d'écart
— la forme exacte que mika#2516 a eue à réparer pour AC (*« The pattern is matched
at TWO sites seven lines apart »*).

---

## Décisions

### D1 — le motif est défini UNE fois et interpolé deux fois

C'est la leçon littérale de mika#2516, écrite dans son propre commentaire :
*« Written once because the two were written twice and drifted: mika#1639 had to
fix both together for case, and nothing was left holding them together. »*

Le motif vit dans une constante de portée fichier de `dispatch-lib.sh`,
`_FD_HEADING_RE`, définie près de `_FIRE_DISPOSITION_RULE` (même famille
mika#2306, même voisinage de lecture), **pas** en `local` dans la fonction. Deux
raisons, et la seconde est fonctionnelle : la constante doit être lisible par le
scan de co-mutation (U4) sans dérouler le flot ; et le harnais de sonde
`source`-ise `dispatch-lib.sh` dans un sous-shell, donc seule une définition au
niveau du fichier est en portée quand la fonction s'exécute. Le site choisi
(voisinage de `_FIRE_DISPOSITION_RULE`, ~2682) précède largement la fonction
(~6813), ce qui est de toute façon sans effet en bash mais garde la lecture
naturelle.

**Nom :** `_FD_HEADING_RE` et non `FD_HEADING_RE` comme l'écrit le ticket — la
convention du fichier est l'underscore initial pour les constantes de portée
fichier (`_MIKA_MANAGED_WORKTREE_SEGMENT`, `_PILOT_SCRATCH_DIRNAME`,
`_PILOT_EGRESS_SOCK`). Divergence de forme, pas de fond.

### D2 — tolérance : numérotation **et** casse

Quatre propriétés, chacune reprise de la décision mika#2516 et chacune mesurée
au § précédent :

- **préfixe de numérotation borné** à `<chiffres>[.]` — la forme mesurée.
  `## 1.1 Fire-Disposition` et `## Phase 3 — Fire-Disposition` ne sont **pas**
  couverts : élargir sur une devinette est ce que la règle « quand NE PAS élargir »
  refuse. Une telle forme rougit visiblement, avec le bon message, et c'est un n+1
  à mesurer (halte 4).
- **`[[:space:]]+` au lieu de l'espace littéral** — élargissement supplémentaire
  assumé : `##   Fire-Disposition` matche désormais. Nommé plutôt que découvert.
  Corollaire mesuré : `##Fire-Disposition` (sans espace) reste refusé, ce qui est
  correct — Markdown exige l'espace, ce n'est pas un titre H2.
- **pas d'ancre `$`** — `## Fire-Disposition (option a)` et
  `## Fire-Disposition Gate` doivent continuer de matcher ; ancrer la fin
  resserrerait le gate en prétendant l'assouplir.
- **le texte reste ancré juste après le préfixe optionnel** — donc
  `## Notes on Fire-Disposition` reste refusé. Contrôle négatif obligatoire.

**La casse (`-i`) est incluse, et l'argument n'est pas une mesure.** Aucune
occurrence de `## fire-disposition` n'a été observée ; l'argument est
**l'alignement lui-même**, qui est l'objet du ticket : les deux gates lisent un
titre écrit par le même producteur, et les séparer sur la casse recréerait un cran
plus loin l'asymétrie qu'on ferme. Pour AC la casse est mesurée à n=2 (mika#1639)
sur un titre plus banal que `Fire-Disposition`, dont la majuscule interne est
inhabituelle et donc **plus** exposée. Le coût en faux positifs réalistes est nul :
`## fire-disposition` **est** la section. Ce raisonnement est écrit au site.

### D3 — `## Fire-Disposition-ish` en tête de ligne est lu PRÉSENT, et c'est une décision

Le ticket liste comme cas négatif « `## Fire-Disposition-ish` **dans une ligne de
prose** ». La mesure sépare deux lectures que la formulation confond :

| forme | lu | conforme au ticket ? |
|---|---|---|
| `Fire-Disposition-ish` dans une ligne de prose | absent | **oui** — l'ancre `^##` suffit |
| `## Fire-Disposition-ish` seul, en tête de ligne | **présent** | **non** |

**Lecture retenue : le qualificatif « dans une ligne de prose » porte sur le cas**,
et les trois négatifs du ticket tombent alors tous par la même propriété — l'ancre
de début. Le motif est suffisant tel quel.

**Pourquoi on n'ancre pas la fin**, alors que ce serait le geste littéral :

1. Il faudrait une borne de mot (`Fire-Disposition($|[^-[:alnum:]])`) pour garder
   `## Fire-Disposition (option a)` et `## Fire-Disposition Gate`, qui sont des
   formes plausibles et doivent matcher. La borne est écrivable, mais elle porte le
   coût suivant.
2. `AC_HEADING_RE` n'a **aucune** borne de fin. En ajouter une à FD seul recrée,
   sur le suffixe, exactement l'asymétrie que ce ticket ferme sur le préfixe — un
   cran plus loin et plus discrète, puisque la co-mutation d'U4 ne porte que sur le
   préfixe.
3. Le faux positif que la borne éviterait n'existe pas : un titre
   `## Fire-Disposition-ish` dans un plan est une forme que personne n'a observée,
   et si elle apparaissait, **c'est la section** — la lire comme telle est le
   comportement utile.

**Coût nommé, et testé pour être lisible** (T12g) : un titre
`## Fire-Disposition-ish` est accepté comme la section. Si l'opérateur veut la
lecture stricte, c'est un **resserrement** qui doit bouger les **deux** motifs
ensemble — suivi nommé, hors périmètre, avec pour précondition qu'une telle forme
soit observée.

### D4 — le sens de l'asymétrie est fail-safe vers « la section est présente »

Un faux négatif du lecteur (ne pas voir une section présente) coûte un pilote de
revise, un finding mensonger et un événement faux — c'est le défaut mesuré. Un
faux positif (voir une section absente) coûte le retour au comportement d'avant
mika#2306, c'est-à-dire le risque ESCALATE que ce rattrapage existe pour réduire.
Les deux coûts sont réels, aucun n'est irréversible, et l'élargissement borné
ci-dessus réduit le premier sans créer de forme de faux positif plausible —
`## Notes on Fire-Disposition` reste refusé, et c'est testé.

### D5 — on ne prescrit pas « ne numérote pas » au producteur

Le remède qui vient à l'esprit — ajouter à `_FIRE_DISPOSITION_RULE` une phrase
« écris le titre sans numéro » — est **refusé**, sur deux motifs :

1. Il est déjà réfuté à n=1 par le ticket : le pilote de #2542 a reçu cette règle
   et a numéroté quand même.
   `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` couvre
   exactement cette classe.
2. Il ne répare aucun plan **déjà** numéroté, ni aucun revise qui renumérote ; le
   lecteur, lui, les répare tous. Doctrine déjà écrite à un fichier de là :
   *permissive detection, strict decision*.

### D6 — une phrase aux deux prompts architecte, et son statut est dit

Les deux gates LLM reçoivent une clause de vocabulaire : le titre peut porter un
numéro de section en tête, la section reste la même. Ce n'est **pas** de
l'enforcement par prompt — le prompt **est** le site de définition de ce gate, il
n'y en a pas d'autre ; la clause corrige une spécification, elle ne réprime pas un
comportement.

Pourquoi FD et pas AC, alors que les deux prompts sont muets sur la numérotation :
**asymétrie de conséquence.** Un raté du gate AC rend ITERATE, puis le plan passe
par un gate CI dont le motif est tolérant — deux filets. Un raté du gate FD au
**second** passage rend ESCALATE, et le prompt lui-même écrit *« No ITERATE exists
at second pass per the two-pass limit »* : aucun filet en aval. AC est nommé en
suivi.

**Ce que cette moitié achète est l'intention. La moitié qui tient est U1** — et ce
qu'U1 tient est le rattrapage, pas le verdict.

### D7 — le scan de site unique couvre les DEUX sections, depuis le harnais que CI lance

Puisque D1 crée pour FD le besoin d'un scan de co-mutation, U4 livre **un** scan
paramétré par nom de section qui couvre les deux — dans `test-dispatch-lib.sh`,
**parce que c'est le seul harnais shell câblé en CI** (A4). Le commentaire de
`verify-pipeline.sh` est corrigé pour nommer ce fichier : il n'est pas acceptable
qu'il continue de désigner un harnais que personne ne lance.

Le prédicat est **positionnel** (le littéral en argument de `grep`/`sed`), jamais
lexical : les deux messages `echo "FAIL: … '## Acceptance criteria' …"` de
`verify-pipeline.sh` citent le littéral sans être des lecteurs, et un scan lexical
les ferait rougir en permanence — c'est-à-dire serait désarmé le premier jour.
Précédent du prédicat positionnel : mika#2496,
`skills/bundled/_shared/test-dispatch-lib.sh` (les trois lancements de
`claude-pilot`).

**Contre-vacuité obligatoire**, et sa forme est décidée par B3 : le scan doit
asserter qu'il **voit** la population (≥ 1 lecteur trouvé par section) **et**
rougir distinctement si un fichier de la population est illisible. Un scan qui ne
regarde rien rend zéro violation.

### D8 — pas de single-source entre les deux motifs, et le prix est nommé

`verify-pipeline.sh` est lu par CI **depuis le dépôt** ; `dispatch-lib.sh` est une
projection du **binaire** (mika#2340, `_shared/` seedé par
`seed_bundled_skills_if_needed`). Aucun fichier commun n'est lisible par les deux
au même instant. Le prix est donc un doublon du sous-motif de préfixe, **tenu par
une assertion de co-mutation** (U4-3) plutôt que mécanisé. Précédent maison
identique et daté : la liste `{main, master}` de `cwd-guard` et
`GIT_OPS_PROTECTED_BRANCHES` (mika#2520), *« aucun single-source inter-langage
n'existe ici, le prix est nommé plutôt que mécanisé »*.

---

## Unités d'implémentation

### U1 — `dispatch-lib.sh` : un motif, deux interpolations

**Fichier :** `skills/bundled/_shared/dispatch-lib.sh`

1. Définir `_FD_HEADING_RE` (valeur en D2) près de `_FIRE_DISPOSITION_RULE`, avec
   un commentaire portant : la cause structurelle (`/ce:plan` numérote), le renvoi
   à `AC_HEADING_RE` comme jumeau assumé et non mécanisé (D8), les quatre bornes du
   motif et **pourquoi chacune**, la décision D3 sur l'absence d'ancre de fin, et
   l'interdiction de ré-orthographier le littéral ailleurs (tenue par U4).
2. Dans `_fd_retry_if_section_still_missing`, **terme 2** →
   `! grep -qiE "$_FD_HEADING_RE" "$plan_path" 2>/dev/null || return 0`
3. Dans la même fonction, **re-test de journalisation** →
   `if ! grep -qiE "$_FD_HEADING_RE" "$plan_path" 2>/dev/null; then`
4. **Le drapeau est `-qiE`, et le piège est de perdre le `E` (B1).** Les deux sites
   portent déjà `-qE` : la transformation est l'ajout du `i`, pas l'ajout du `E`.
   Écrire `-qi` fait perdre l'ERE **en silence** — le groupe et les quantificateurs
   redeviennent littéraux, le motif ne matche rien, les deux termes deviennent
   toujours-vrais, et le rattrapage tire sur tout plan dont les findings mentionnent
   la chaîne. Écrire le piège au site, et le vérifier sur le drapeau combiné (V7).
5. **Quoting** : la constante est définie en quotes simples (le motif contient `\.`
   et des crochets qu'aucune expansion ne doit toucher) et interpolée en quotes
   doubles aux deux sites.
6. Mettre à jour le doc-comment de la fonction : le terme 2 se lit désormais « ne
   porte pas de titre `Fire-Disposition`, numéroté ou non ».
7. **Le terme 1 n'est pas touché** : c'est un `grep -qF` sur les **findings**, pas
   sur le plan — population différente, délibérément grossière, documentée comme
   telle.

### U2 — les fixtures : cinq états de plus, et sept assertions

**Fichier :** `skills/bundled/_shared/test-dispatch-lib.sh` (harnais
`_t2306_revise_probe`, T1–T11)

Le harnais prend déjà `$2 = t2306_has_section` ∈ `{yes, no}` et écrit
`## Fire-Disposition\n\nOption (a) …` sous un `if`. **Étendre ce paramètre** à
**cinq** états supplémentaires (B6) plutôt que dupliquer le harnais, en remplaçant
le `if` par un `case` : `numbered` (`## 3. Fire-Disposition`), `upper`
(`## 3. FIRE-DISPOSITION`), `ish` (`## Fire-Disposition-ish`), `notes`
(`## 11. Notes on Fire-Disposition`), `subsub` (`## 1.1 Fire-Disposition`). Le mode
de comportement `second` reste inchangé.

**La ligne `## Acceptance criteria` qui suit le bloc reste telle quelle (B5)** —
elle est hors du `case`, et une réécriture du bloc qui l'emporterait casserait tout
futur gate AC sur ce fixture.

| id | fixture | attendu | ce que ça voit |
|---|---|---|---|
| T12a | findings réclamant FD + section **numérotée** | **zéro relance** | **le cœur du correctif** (C1) |
| T12b | idem, `## 3. FIRE-DISPOSITION` | zéro relance | tolérance de casse (D2) |
| T12c | idem, `## 11. Notes on Fire-Disposition` seul | **une** relance | la tolérance n'a pas avalé le refus |
| T12d | idem, `## 1.1 Fire-Disposition` | **une** relance | borne du préfixe assumée, pas un oubli |
| T12e | section numérotée, contrôle d'événement | **aucun** `fire_disposition_still_missing_after_retry` | la **moitié oubliée** |
| T12f | findings critiquant le **contenu** + section numérotée | zéro relance | la chaîne C2, que le ticket ne voit pas |
| T12g | `## Fire-Disposition-ish` seul | **zéro relance** (lu présent) | le coût de D3, rendu lisible |

**T12a est la fixture porteuse** — même rôle que T1 dans `verify-pipeline-test.sh`
(mika#2516). **T12e est obligatoire et non décorative** : avec le seul terme 2
élargi, T12a passe au vert **et** le re-test de journalisation émet toujours
l'événement faux sur le chemin nominal. C'est elle qui voit le correctif à moitié
appliqué.

T7 (« section déjà présente ⇒ zéro relance ») reste tel quel : c'est le contrôle
négatif de la forme non numérotée, et il doit rester vert pour que T12a prouve un
élargissement et non un désarmement.

### U3 — la clause de vocabulaire dans les deux gates architecte

**Fichiers :** `skills/bundled/mika-arch-groom-ticket/system_prompt.md` et
`skills/bundled/mika-arch-second-review/system_prompt.md`, § *Fire-Disposition
Gate (mika#1574)*, dans l'arbre de décision (branche 1 des deux).

Une phrase par fichier, **en anglais** (B2), dans le registre du fichier : le titre
de la section peut porter un numéro de section en tête (`## 3. Fire-Disposition`)
parce que le producteur de plans numérote ses titres ; la section est la même et le
gate passe. Formulée sur le **vocabulaire**, pas en injonction au groomeur.

Assertions correspondantes dans `test-dispatch-lib.sh` (les prompts bundled y sont
déjà lus par d'autres tests) : les deux fichiers contiennent la clause. C'est une
assertion de **présence**, pas de comportement — le comportement d'un LLM n'est pas
testable déterministement, et c'est dit au contrat de vérification.

### U4 — le scan de site unique et de co-mutation

**Fichier :** `skills/bundled/_shared/test-dispatch-lib.sh` (nouveau bloc, après
T12). `REPO_ROOT` y existe déjà (`SCRIPT_DIR/../../..`), donc la lecture de
`$REPO_ROOT/scripts/verify-pipeline.sh` ne demande aucune plomberie neuve.

Quatre assertions :

1. **Site unique FD** — aucune ligne de `dispatch-lib.sh` n'interroge le littéral
   `Fire-Disposition` **en position de motif** (`grep`/`sed`) autrement que via
   `"$_FD_HEADING_RE"`. Exclusions nommées, et ce sont des **définitions de
   population**, pas des exceptions : `_FIRE_DISPOSITION_RULE` et le corps du
   finding synthétique (prose), et le terme 1 (`grep -qF` sur les findings).
2. **Site unique AC** — même prédicat positionnel sur `verify-pipeline.sh` avec
   `Acceptance criteria` : tout `grep`/`sed` interrogeant ce littéral doit passer
   par `"$AC_HEADING_RE"`. **Ceci livre la garde qu'A4 a trouvée manquante.**
3. **Co-mutation** — la sous-chaîne de préfixe (`([0-9]+\.?[[:space:]]+)?`) est
   **littéralement identique** dans `_FD_HEADING_RE` et dans `AC_HEADING_RE`. C'est
   ce qui remplace le single-source impossible (D8) : le jour où l'un des deux
   évolue, ce test rougit au lieu de laisser les deux gates redivergents en silence.
4. **Contre-vacuité, en deux moitiés (B3)** — (i) au moins un lecteur trouvé par
   section, et `_FD_HEADING_RE` non vide ; (ii) **un fichier de la population
   illisible fait ROUGIR**, avec un message nommant le fichier introuvable — et non
   un saut silencieux à la T2211. C'est une divergence délibérée avec le précédent
   du fichier : sans elle, un renommage de constante, un déplacement de fichier ou
   une exécution hors checkout rend le scan inerte, ce qui se lit exactement comme
   un arbre propre (mika#2205).

### U5 — la fausse garantie de `verify-pipeline.sh` est corrigée

**Fichier :** `scripts/verify-pipeline.sh`, commentaire de `AC_HEADING_RE`
(deux lignes en prose, **aucune ligne de prédicat**).

Le commentaire nomme `scripts/verify-pipeline-test.sh` comme siège de la garde de
lecteur unique. Il doit nommer `skills/bundled/_shared/test-dispatch-lib.sh`, où
U4-2 la livre, et référencer mika#2544 à côté de mika#2516. Le prédicat AC, ses
deux lecteurs et son motif ne sont **pas** touchés.

### U6 — documentation

- `docs/solutions/workflow-issues/verify-pipeline-ac-heading-case-insensitive-2026-06-30.md` :
  ajouter mika#2544 comme **n=4 de la classe**, en notant que cette occurrence
  porte sur une **autre section et un autre fichier** — la classe n'est pas « le
  motif AC est trop strict » mais « un lecteur de titre de section de plan qui ne
  tolère pas la numérotation du producteur ». C'est cette formulation qui rend la
  classe récurrente lisible. Noter aussi la découverte A4 (le harnais non câblé),
  puisque ce document prescrit de lancer `bash scripts/verify-pipeline-test.sh` à
  la main sans dire que CI ne le fait pas.
- Aucune entrée `CLAUDE.md` : ce correctif ne crée ni réglage, ni événement, ni
  surface opérateur nouvelle (voir *Ce que ce travail n'achète pas*).

---

## Contrat de vérification

| # | contrôle | comment |
|---|---|---|
| V1 | `make test-dispatch-lib` vert, T1–T11 **inchangés** | job CI existant (`ci.yml:85`) |
| V2 | T12a–T12g passent, et **T12a vue rouge avant U1** | appliquer U2 avant U1 et le noter au corps de PR |
| V3 | **T12e vue rouge** avec le seul terme 2 élargi | c'est la moitié-appliquée que V2 ne voit pas |
| V4 | T12c/T12d **vues vertes avant U1** (la borne existait déjà) et après | prouve un élargissement, pas un désarmement |
| V5 | **U4-3 vue rouge** en désalignant un caractère du préfixe dans l'un des deux motifs | la co-mutation mord réellement |
| V6 | U4-2 verte sans toucher le **prédicat** AC de `verify-pipeline.sh` | si elle rougit, un second lecteur AC existe : le nommer avant tout élargissement |
| V7 | les deux sites portent `-qiE` — **drapeau combiné**, pas `-E` seul (B1) | `grep -n 'grep -qiE "\$_FD_HEADING_RE"' dispatch-lib.sh` rend **deux** lignes, et `grep -c "grep -qE '\^## Fire-Disposition'"` rend **zéro** |
| V8 | **U4-4(ii) vue rouge** en rendant `verify-pipeline.sh` illisible | le scan ne se désarme pas en silence (B3) |
| V9 | `bash -n skills/bundled/_shared/dispatch-lib.sh` et `shellcheck` sans régression | le quoting d'U1-5 |
| V10 | `make verify-bundled-skills` vert | U3 touche deux bundles |
| V11 | `bash scripts/verify-pipeline-test.sh` rend 0 | il n'est pas en CI (A4) : à lancer **à la main**, et à dire dans le corps de PR |

**Ce qui n'est PAS testable ici, écrit plutôt que découvert.** « Le gate architecte
accepte un titre numéroté » est exécuté par un LLM, contre un fournisseur réel. Le
contrat côté mika est *la clause est dans le prompt servi*, et U3 l'atteste
déterministement. La moitié comportementale est la sonde S2.

---

## Fire-Disposition

Ce plan livre des détecteurs : les assertions T12a–T12g (U2), les deux assertions
de présence de clause (U3) et le scan de source à quatre termes (U4). **Option (a)
— exception nommée en allowlist, table livrée vide et vacuité assertée à
l'exécution.**

Modèle exact : `T2306_ARCH_ASK_ALLOWLIST` (T9 de `test-dispatch-lib.sh`, table
`=()` dont la taille est assertée à zéro) et `scripts/test-guard-shared-checkout.sh`.
Le scan d'U4 porte une table `T2544_HEADING_READER_ALLOWLIST=()` dont le test
**asserte qu'elle compte zéro entrée**. Elle rougit donc le jour où une exception
est ajoutée, pas seulement quand elle devient stale — et une table vide assertée
vide est ce qui distingue « aucune violation » de « le scan ne regarde rien ».

**Violations préexistantes : zéro, et c'est établi par lecture, pas supposé.**
L'inventaire du § *Cartographie* est exhaustif. Les deux populations du scan
comptent respectivement **2 lecteurs FD** (tous deux passant par la constante après
U1) et **2 lecteurs AC** (tous deux passant par `AC_HEADING_RE` aujourd'hui, lignes
174 et 184). Les deux `echo "FAIL: …"` de `verify-pipeline.sh` citent le littéral
sans être des lecteurs : ils sont hors population par le prédicat **positionnel**
de D7 — ce n'est pas une exception, c'est la définition de la population.

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

**Lecture par fichier, et méfiance sur le jeton nu** : ce `.stderr` porte aussi la
prose du pilote, et une session qui **discute** de ces jetons — comme celle qui a
groomé ce ticket — produit un faux positif mesuré (mika#2050).

| événement | régime attendu **après** ce correctif | lecture |
|---|---|---|
| `fire_disposition_revise_retried` | **rare**, et strictement décroissant | chaque ligne restante est un plan qui n'a réellement pas la section |
| `fire_disposition_still_missing_after_retry` | **zéro** | une occurrence dit que `/mika-revise-plan` ne sait pas écrire la section — suivi mika-platform, **jamais** un troisième essai ici |

### Sondes post-déploiement, et leurs quatre haltes

> **Préalable, non négociable.** `skills/bundled/_shared/` est une projection du
> **binaire**, pas du checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer`
> doit porter le sha qu'on vient de bâtir. **Sans cette vérification, chacune des
> sondes ci-dessous décrit le binaire d'hier.**

**S1 — le défaut mesuré ne se reproduit pas** (premier groom livrant un détecteur
après déploiement). Un plan dont la section arrive numérotée ne déclenche ni
`fire_disposition_revise_retried` ni le second événement.

> **Halte 1 — l'un des deux apparaît sur un plan qui PORTE la section.** Le
> correctif n'a pas pris. **Ne pas élargir le motif par réflexe** : établir d'abord
> le déploiement (préalable ci-dessus), puis vérifier que **les deux** sites lisent
> la constante — c'est la moitié-appliquée que V3 existe pour voir.

**S2 — la moitié LLM (7 jours).** Aucun ESCALATE de second passage motivé par une
section `Fire-Disposition` absente alors que le plan en porte une, numérotée.

> **Halte 2 — un tel ESCALATE apparaît.** C'est la moitié prompt qui ne tient pas,
> **et c'est attendu comme possible** (D6). Ne pas durcir le prompt par réflexe : le
> seul levier structurel est de faire lire la section par du code avant le second
> passage, ce qui est un ticket à part, à ouvrir avec cette occurrence en
> précondition.

**S3 — contrôle positif, obligatoire.** `fire_disposition_revise_retried` doit
rester **non nul** sur la population qui le mérite : un plan livrant un détecteur et
n'ayant réellement aucune section.

> **Halte 3 — zéro des deux événements sur 7 jours.** On ne peut **rien** conclure.
> Vérifier qu'un groom a réellement convergé dans la fenêtre. *Un rattrapage
> silencieusement désarmé se lit exactement comme un rattrapage qui n'a rien à
> faire* (mika#2205) — et le mode de panne d'U1-4 (ERE perdu) produit très
> exactement l'inverse, un rattrapage qui tire sur tout : si les deux événements
> explosent, c'est là qu'il faut regarder avant le motif.

**S4 — la classe, pas l'occurrence.** Une nouvelle forme de titre non couverte
(`## 1.1 …`, `## Phase 3 — …`) rougit visiblement en T12d/T12c, avec le bon message.

> **Halte 4 — une telle forme arrive en production.** C'est un n+1 **à mesurer**,
> pas à absorber : élargir le préfixe est ce que D2 refuse explicitement sans
> mesure, et l'élargissement devra bouger les **deux** motifs ensemble (U4-3 rougira
> sinon, ce qui est son travail).

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
  vigueur ; « quelle tolérance ce dispatch a-t-il réellement appliquée ? » reste une
  question à laquelle on répond en lisant le binaire déployé, pas un grep. Assumé
  pour un prédicat dont les deux issues sont déjà journalisées.
- **Il ne câble pas `verify-pipeline-test.sh` en CI**, alors qu'A4 établit qu'il ne
  l'est pas. La garde AC est livrée dans le harnais qui **tourne** ; le harnais qui
  ne tourne pas reste un suivi.

---

## Definition of Done

- [ ] `_FD_HEADING_RE` défini une fois et interpolé aux deux sites, avec les quatre
      bornes, la décision D3 et le piège du drapeau combiné écrits au site (U1)
- [ ] Les cinq états supplémentaires de `_t2306_revise_probe` et T12a–T12g (U2),
      la ligne `## Acceptance criteria` du fixture **conservée** (B5)
- [ ] T12a vue **rouge** avant U1 ; T12e vue **rouge** sur un correctif à moitié
      appliqué (V2, V3) — les deux notées au corps de PR
- [ ] La clause de vocabulaire **en anglais** dans les deux prompts architecte +
      assertions de présence (U3, B2)
- [ ] Le scan à quatre termes dans `test-dispatch-lib.sh`, table d'exceptions
      **vide et assertée vide**, contre-vacuité en deux moitiés dont le refus sur
      fichier illisible (U4, B3)
- [ ] U4-3 vue **rouge** en désalignant un caractère (V5) ; U4-4(ii) vue **rouge**
      sur fichier illisible (V8)
- [ ] Les deux sites portent `-qiE` et plus aucun `grep -qE '^## Fire-Disposition'`
      ne subsiste (V7)
- [ ] Le commentaire de `AC_HEADING_RE` nomme le harnais qui porte réellement la
      garde, sans toucher au prédicat AC (U5)
- [ ] `make test-dispatch-lib`, `bash -n`, `shellcheck`, `make verify-bundled-skills`
      verts (V1, V9, V10) ; `bash scripts/verify-pipeline-test.sh` lancé **à la
      main** et rendu 0 (V11)
- [ ] `docs/solutions/…-ac-heading-case-insensitive-2026-06-30.md` porte mika#2544
      en n=4 avec la classe reformulée, et la découverte A4 (U6)
- [ ] Corps de PR écrit **sous le worktree** (`pr-body.md`), passé en
      `--body-file`, supprimé ensuite (mika#2211) — et les suivis y sont nommés en
      français, **ou** en anglais avec une ligne `Tracked in: …#<N>` par suivi
      réellement ouvert (B4)
- [ ] Suivis nommés : (1) câbler `verify-pipeline-test.sh` en CI ; (2) la clause de
      vocabulaire pour le gate **AC** des deux prompts architecte ; (3) la lecture
      par code de la section avant le second passage (précondition : halte 2) ;
      (4) le resserrement du suffixe sur les deux motifs (précondition : D3)

---

## Acceptance criteria

Les quatre premiers sont transcrits du § *Critères d'acceptation* du corps de
mika#2544 ; les six suivants sont dérivés du § *Remède* et du contrat de
vérification, et couvrent ce que les mesures de ce grooming ont ajouté.

- **AC1** — Les deux lecteurs de `dispatch-lib.sh` lisent un titre numéroté comme
  présent, via un motif défini une seule fois.
- **AC2** — Les cas négatifs listés au remède restent lus absents : `### Fire-Disposition`,
  `Fire-Disposition-ish` dans une ligne de prose, et une mention entre backticks.
  **Écart assumé et documenté (D3)** : `## Fire-Disposition-ish` **en tête de
  ligne** est lu présent, comme `## Fire-Disposition (option a)` et pour la même
  raison — l'absence d'ancre de fin, alignée sur `AC_HEADING_RE`. L'écart est
  couvert par T12g et son resserrement est un suivi.
- **AC3** — Un test rougit si un troisième `grep` littéral `^## Fire-Disposition`
  apparaît dans `dispatch-lib.sh`.
- **AC4** — Les deux prompts architecte disent explicitement que la numérotation
  est tolérée.
- **AC5** — La tolérance ne dégrade aucun refus : `## Notes on Fire-Disposition` et
  `## 1.1 Fire-Disposition` restent refusés, et le comportement sur un titre non
  numéroté est **inchangé** (T7 reste vert).
- **AC6** — Un correctif à moitié appliqué (un seul des deux sites) est **détecté**
  par une assertion, et non par l'absence de plainte.
- **AC7** — La co-mutation entre le motif FD et le motif AC est tenue par une
  assertion : désaligner l'un des deux préfixes fait rougir le build.
- **AC8** — La garde de lecteur unique pour AC, que le commentaire de
  `verify-pipeline.sh` affirmait sans qu'elle existe, est livrée **dans un harnais
  que CI exécute**, et ce commentaire nomme désormais ce harnais. Le prédicat AC,
  ses deux lecteurs et son motif ne sont pas modifiés.
- **AC9** — Aucune valeur de réglage, aucun événement et aucune variable
  d'environnement ne sont créés, déplacés ou supprimés.
- **AC10** — Le scan d'U4 **rougit** quand un fichier de sa population est
  illisible, au lieu de sauter silencieusement comme le fait le précédent T2211 du
  même harnais.

---

## Hors périmètre, délibérément

- **Le prédicat `Acceptance criteria` lui-même** — déjà corrigé (mika#2516), et
  exclu par le corps du ticket. U4-2 **lit** ses lecteurs, il n'en change aucun ;
  U5 ne touche qu'un commentaire.
- **Câbler `verify-pipeline-test.sh` en CI** — découverte A4, réelle et dont la
  valeur dépasse ce ticket : le harnais qui teste le gate AC de CI n'est pas
  lui-même en CI. Le câbler est un élargissement de périmètre (772 lignes, dépôts
  git jetables, `gh` mocké) qui peut rougir sur des causes sans rapport.
  **Suivi**, sans précondition : c'est une inertie établie, pas une hypothèse.
- **Corriger le saut silencieux de T2211** (`if [ -f ]` sur `mika.md`), défaut de
  même classe que B3 trouvé en chemin. U4 ne le reproduit pas ; le réparer *lui*
  changerait la population d'un test voisin sans rapport avec ce ticket. **Suivi**,
  sans précondition : l'inertie est mesurée, à la ligne 6104.
- **La clause de vocabulaire pour le gate AC des deux prompts architecte** — même
  absence que pour FD, mais AC dispose d'un filet CI en aval que FD n'a pas (D6).
  **Suivi**, sans précondition.
- **Élargir le préfixe à `## 1.1` / `## Phase 3 — `** — refusé sans mesure (D2), et
  la borne est **testée** pour que la prochaine occurrence rougisse avec le bon
  message plutôt que de passer en silence.
- **Resserrer le suffixe des deux motifs** (borne de mot après le nom de section) —
  la lecture littérale du cas `-ish` du ticket. Refusé avec sa raison (D3) ;
  **suivi** conditionné à l'observation d'une telle forme, et il devra bouger les
  **deux** motifs ensemble.
- **Prescrire au producteur de ne pas numéroter** — refusé avec sa raison (D5).
- **Le review-anchor sur le brief pré-correction**, seconde cause de l'échec du
  groom #2542 selon le ticket — cause distincte, ticket distinct.
- **Faire lire la section par du code avant le second passage architecte** — le
  seul levier structurel sur la moitié LLM, blast radius large (il faudrait décider
  qui rend le verdict), **suivi** conditionné à la halte 2.
- **Le terme 1 du prédicat de rattrapage** (`grep -qF` sur les findings de première
  passe) — délibérément grossier, documenté comme tel, et resserrer ce terme
  changerait la population du rattrapage plutôt que sa lecture du plan.
