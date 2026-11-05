# mika#3040 — le Halt event cite le refus qui a tué la session, pas le premier de la liste

- **Ticket :** senara-solutions/mika#3040
- **Labels :** `bug`, `p2-normal`, `ready`, `dispatch:loop`, `loop-substrate`
- **Dépôt :** mika seul. claude-pilot n'est pas touché.
- **Site :** `skills/bundled/_shared/dispatch-lib.sh`, lu sur `9a478c3c4`.
- **Gel des tickets (jusqu'au 01/12) :** ce vol n'ouvre aucun ticket de suivi.
  Un reste éventuel part en commentaire sur mika#3040 ou dans la PR.

---

## 0. Le vol, en une phrase

Un `result` qui déclare « the deny's lethality marker says (terminal) » joint en
preuve le **premier** refus de la session, qui est non terminal dans 98 cas sur
118. Le vol fait converger les deux lectures sur un seul événement : quand un
refus terminal existe dans le fichier, c'est lui que le Halt event cite.

---

## 1. L'état mesuré

### 1.1 La population (reprise du corps du ticket)

Rejeu sur les 3 162 `.stderr` de `/var/log/claude-pilot/`, du 2026-05-15 au
2026-11-03. Chaque refus y est lu comme un **événement complet**, avec les
bornes de `_policy_deny_excerpt` : le marqueur de létalité, une ligne qui ouvre
un autre événement par `[`, ou 12 lignes.

| population | sessions |
|---|---|
| au moins un refus | 1 771 |
| au moins un événement de refus **terminal** | 118 |
| dont le premier refus **est** le terminal | 20 |
| dont le premier refus **n'est pas** le terminal | **98** |
| les mêmes 98, depuis le 2026-10-01 | 11 |

Le cas d'origine, groom `e71ada95` (mika#3030) — et, au 2026-11-05 07:57Z, la
seule ligne de `tasks` dont le `result` contient `Halt event:` :

| heure | refus | marqueur | cité par le `result` |
|---|---|---|---|
| 19:53:52Z | `git -C …/claude-pilot rev-parse --git-dir`, `cause=default-deny` | (non-terminal) | **oui**, comme « Halt event » |
| 19:59:41Z | `grep -c 'bash -c' ~/.mika/data/pilot-transcripts/*.jsonl …`, `cause=tier3 [bash-grep]` | (terminal) | non |

### 1.2 Les deux lectures et leur divergence, lues sur `9a478c3c4`

| élément | lieu | ce qu'il lit |
|---|---|---|
| extrait | l. 8141-8160 | `_policy_deny_excerpt` imprime le **premier** `[policy:deny]` du flux |
| létalité | l. 8195-8216 | `_policy_deny_lethality` rend `terminal` dès qu'**un** `(terminal)` figure dans le fichier |
| site dev-pilot | l. 8324-8335, l. 8368 | les deux appelées séparément, puis `Halt event: ${POLICY_DENY}` |
| site dev-groom | l. 8559-8575, l. 8607 | idem, copie quasi-littérale |
| tests | `test-dispatch-lib.sh` l. 9985 et suivantes, bloc mika#2493 | **aucune fixture ne porte deux refus de létalités différentes** |

Le défaut n'est dans aucune des deux fonctions : chacune est correcte pour sa
propre question. Il est dans le fait que le site pose **deux questions à des
objets différents** — la létalité au fichier, l'extrait au premier événement —
puis rédige une phrase qui les présente comme portant sur le même. Le
comportement est inchangé depuis `1c6f30fbc` ; seules les lignes ont bougé.

### 1.3 Pourquoi le bloc de test existant ne l'a pas vu

Le bloc mika#2493 a douze fixtures. T5 porte bien cinq refus dont un terminal —
mais il n'interroge que `_policy_deny_lethality`. T13 et T14 interrogent bien
`_policy_deny_excerpt` — mais sur des fixtures à refus unique. La paire
« plusieurs refus **et** extrait » n'existe nulle part : le défaut vit
exactement dans la case que le produit cartésien des fixtures laisse vide.
C'est ce qui fait d'AC1 une fixture, non une relecture.

---

## 2. Le geste

### 2.1 G1 — l'extrait choisit l'événement terminal quand il en existe un

`_policy_deny_excerpt` cesse de s'arrêter au premier événement. Son `awk`
devient une machine à deux candidats : il parcourt **tous** les événements de
refus du flux, retient le premier rencontré (candidat de repli) et le premier
qui porte `(terminal)` (candidat terminal), et imprime le terminal s'il existe,
le repli sinon.

**Le prédicat ne prend aucun paramètre, et c'est la mesure qui le permet.** Si
un événement de refus porte `(terminal)`, alors `_policy_deny_lethality` — qui
grep le fichier entier — rend nécessairement `terminal` : « un événement
terminal existe » ⟹ « la létalité de la session est `terminal` ». AC1 est donc
tenu sans que la fonction ait à recevoir la létalité. L'implication inverse est
fausse, et son cas est nommé : la létalité peut valoir `terminal` par la **prose
du pilote** (le coût déjà nommé au-dessus de `_policy_deny_lethality`, classe
Signal-S de mika#2050, explicitement hors vol ici) sans qu'aucun événement ne
porte le marqueur. Là, la fonction retombe sur le premier événement — le
comportement d'aujourd'hui, inchangé. Cette asymétrie est ce qui rend le geste
sans paramètre : passer la létalité en argument n'ajouterait aucune information
que le flux ne porte déjà.

**Premier terminal, pas dernier.** `_policy_deny_lethality` documente déjà la
raison (M4, vérifiée sur `da4aa7ae`) : un refus terminal **termine** sa session,
ce qui fait coïncider « le premier terminal » et « le dernier terminal » sur la
population réelle. Les deux formulations rendent le même événement, et seule la
première survit à une troncature de fin de fichier — la propriété que mika#2493
a nommée et qu'il n'y a aucune raison de perdre ici.

Forme de la machine à états (le programme awk à écrire ; `max` reste
`${_POLICY_DENY_EXCERPT_MAX_LINES:-12}`) :

| état | ligne | action |
|---|---|---|
| au repos | contient `[policy:deny]` | ouvre un événement : buffer ← la ligne, `n=1` ; si la ligne porte `((non-)?terminal)`, clôture |
| au repos | autre | ignorée |
| en cours | commence par `[` (après blancs) | **clôture** l'événement courant ; si cette même ligne contient `[policy:deny]`, elle **ouvre** le suivant |
| en cours | autre | `n++`, ajoute la ligne **indentée de 4 espaces** ; clôture si elle porte le marqueur ou si `n >= max` |
| fin de flux | — | clôture ; imprime le candidat terminal s'il existe, sinon le candidat de repli |

Clôturer, c'est : enregistrer le buffer comme candidat de repli s'il n'y en a
pas encore ; puis comme candidat terminal s'il n'y en a pas encore **et** que
le buffer contient la sous-chaîne littérale `(terminal)`.

Trois propriétés de l'awk actuel sont **conservées à l'identique**, parce que
chacune est tenue par un test existant ou par un commentaire qui en nomme le
prix :

1. **La discrimination est une sous-chaîne littérale.** `index(buf, "(terminal)")`,
   jamais une regex de mot. `(non-terminal)` ne contient pas `(terminal)` : la
   parenthèse ouvrante qu'elle exige est occupée par le `-`. C'est la propriété
   que T6 vérifie directement, et l'inverser reclasserait d'un coup les 1 093
   refus non terminaux mesurés.
2. **Les lignes de continuation restent indentées de 4 espaces.** Non
   cosmétique : l'indentation neutralise les deux jetons de reclassification
   ancrés en début de ligne (`^STATUS=CANCELLED`, `^Outcome: PIPELINE_INCOMPLETE`)
   qu'une continuation pourrait sinon ouvrir — mika#2493 D5. La première ligne
   reste à fleur, parce que l'appelant l'interpole juste après `Halt event: `.
3. **Le programme ne court-circuite jamais son entrée.** Il drainait déjà stdin
   après impression pour que le `sed` amont ne prenne pas `SIGPIPE` et ne soit
   pas promu au statut du pipeline sous `pipefail` (classe mika#2055). La
   nouvelle forme le fait *structurellement* : elle n'imprime qu'en `END`, donc
   elle lit tout le flux par construction. La garde devient une conséquence de
   la forme au lieu d'être une précaution — à noter dans le commentaire, pour
   qu'une « simplification » future qui réintroduirait un `exit` sache ce
   qu'elle casse.

Le **coût déjà nommé** par mika#2493 est inchangé et reste nommé : les trois
jetons de reclassification **non ancrés** ne sont pas neutralisés. Un
commandement refusé dont le texte contient littéralement `PIPELINE FAILURE:`
reclasserait la session. Exposition préexistante ; la scrubber contredirait la
doctrine de non-troncature que cette fonction existe pour honorer.

### 2.2 G2 — un seul appel, `_policy_deny_read`

AC3. Les deux sites cessent de construire `POLICY_DENY` eux-mêmes et appellent
une seule fonction, qui lit le fichier **une fois** et publie les deux valeurs :

| avant (×2 sites) | après (×2 sites) |
|---|---|
| `POLICY_DENY=""` | `_policy_deny_read "$PERSISTENT_STDERR_PATH"` |
| `POLICY_DENY_LETHALITY="undeclared"` | |
| `if [ -f … ] && [ -r … ]; then` | |
| `POLICY_DENY=$(sed 's/\x1b…//g' … \| _policy_deny_excerpt \|\| true)` | |
| `POLICY_DENY_LETHALITY=$(_policy_deny_lethality …)` | |
| `fi` | |

`PERSISTENT_STDERR_PATH` reste construite **au site** (`_pilot_log_dir` puis
`"$_PILOT_LOG_DIR/${LOG_ID}.stderr"`) : le site sait quel fichier est le sien,
la fonction sait comment le lire. Les deux lignes d'initialisation et la garde
`-f`/`-r` entrent dans `_policy_deny_read`, qui règle `POLICY_DENY=""` et
`POLICY_DENY_LETHALITY="undeclared"` **d'entrée**, avant toute lecture — le
fail-open d'aujourd'hui, à l'identique, pour un fichier absent, illisible ou
vide (T7).

**Elle publie, elle n'imprime pas.** Deux valeurs sortent d'une seule lecture,
et l'une des deux est un texte libre multi-ligne : un retour par stdout
exigerait un séparateur dans un flux qui ne peut pas en garantir l'absence.
Même motif que `_resolve_halt_subtype`, qui publie `_HALT_SUBTYPE_RESOLVED` et
dont le commentaire porte déjà cette justification. Aucun des deux sites ne
déclare `local POLICY_DENY` : l'affectation atteint la même variable
qu'aujourd'hui, et `_rescue_cause_token` (l. 7758) la lit inchangée.

**Le prix, nommé :** le strip ANSI quitte les sites. mika#2493 l'y avait laissé
délibérément — « the ANSI strip stays here, at the site », écrit deux fois.
L'intention de ce commentaire était que la normalisation soit **visible**, pas
qu'elle soit **dupliquée** ; et dupliquée deux fois, elle était précisément la
condition matérielle pour que les deux sites puissent divergent. Un seul site
pour un `sed` qui normalise reste un site visible. Le commentaire de
`_policy_deny_read` doit porter ce renversement explicitement, faute de quoi
la prochaine lecture de dispatch-lib verra une consigne de mika#2493 enfreinte
sans savoir qu'elle l'a été sciemment.

### 2.3 G3 — le scan qui refuse un troisième site

AC3, seconde moitié. Sur le modèle exact de T8, déjà dans ce bloc : une
allowlist **déclarée vide** et tenue vide par une assertion, plus un scan
structurel, plus un contrôle de bonne foi.

```sh
POLICY_DENY_CONSTRUCTION_SITES_ALLOWED=""   # mika#3040 : LIVRÉE VIDE, et un test le tient
```

Le scan porte sur `$DISPATCH_LIB` et compte trois choses :

| mesure | valeur attendue | ce qu'elle refuse |
|---|---|---|
| lignes contenant `POLICY_DENY=$(` | **1** | un site qui reconstruirait l'extrait par substitution de commande |
| appels à `_policy_deny_excerpt` hors de sa définition | **1** | un site qui appellerait l'extracteur directement, sans la létalité |
| appels à `_policy_deny_lethality` hors de sa définition | **1** | la lecture dissociée, qui est le défaut d'aujourd'hui |
| appels à `_policy_deny_read` | **≥ 2** | *bonne foi* : un fichier sans aucun site passerait vert à vide |

Les trois premières lignes doivent toutes tomber dans `_policy_deny_read` ; la
quatrième est ce qui empêche le scan de mesurer le vide. Comme T8 l'écrit pour
sa propre population, **« on déclare, on n'allowliste pas »** (mika#2201,
mika#2323) : quand ce scan tire, la résolution est de router le nouveau site
par `_policy_deny_read`, jamais d'ajouter une entrée.

---

## 3. Ce que ce vol ne fait pas

- **Le texte d'aide qui suit le Halt event** (l. 8370 et l. 8609), identique aux
  deux sites. Il reste dupliqué. Factoriser la prose est un geste distinct de
  celui qui fait converger la lecture, et le mêler ici rendrait la diff de ce
  vol illisible pour l'architecte.
- **La lecture de `(terminal)` dans la prose du pilote** par
  `_policy_deny_lethality` — coût déjà nommé au-dessus de la fonction, borné par
  la garde U1 (la branche n'est atteinte que si aucun livrable n'a été produit).
  Hors vol par le ticket.
- **Le refus `[bash-grep]` de 19:59:41Z lui-même**, qui relève de claude-pilot.
- **Aucune `docs/solutions/`** : le ticket n'en demande pas, et la doctrine du
  geste tient dans les commentaires des deux fonctions touchées.

---

## 4. Fire-Disposition

Ce plan livre un détecteur : le scan structurel de G3, dont le chemin de succès
est « aucun site ne construit `POLICY_DENY` hors de `_policy_deny_read` ».

**Option (a) — exception nommée en allowlist, livrée VIDE et armée.**

- **Détail d'implémentation :** l'allowlist est la variable
  `POLICY_DENY_CONSTRUCTION_SITES_ALLOWED`, déclarée dans le bloc de test,
  grep-visible, et initialisée à la chaîne vide avec le commentaire
  `# mika#3040 : LIVRÉE VIDE, et un test le tient`.
- **Zéro violation existante, et c'est la refonte qui le produit :** le dépôt
  porte aujourd'hui deux sites de construction divergents. G2 les supprime tous
  les deux au profit d'un appel unique. Le détecteur atterrit donc **armé et
  vert**, sans aucune exception à accorder. C'est le même atterrissage que T8
  dans ce même bloc (`POLICY_DENY_UNGUARDED_ALLOWED=""`), et le précédent est à
  citer dans le commentaire.
- **Assertion auto-nettoyante :**
  `assert_eq "AC3: allowlist des sites de construction — zero entries" "" "$POLICY_DENY_CONSTRUCTION_SITES_ALLOWED"`.
  Une exception ne peut pas devenir *stale* ici, parce qu'elle ne peut pas
  exister : toute entrée ajoutée rougit cette assertion au premier run.
- **Suivi :** aucun ticket (gel jusqu'au 01/12). Si le scan devait un jour
  exiger une exception réelle, la discussion part en commentaire sur mika#3040.
- **Contrôle de bonne foi, obligatoire :** l'assertion `≥ 2` sur les appels à
  `_policy_deny_read`. Sans elle, un scan qui ne trouve plus aucun site rend
  `0:0:0` et passe vert à vide — le mode de panne exact que mika#2578 et
  mika#2752 ont payé deux fois.

---

## 5. Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#3040.

- **AC1.** Quand la létalité de la session est `terminal`, le Halt event cite
  l'événement terminal. Test sur une fixture qui porte deux refus, le premier
  `(non-terminal)` et le second `(terminal)` : l'extrait contient le second et
  ne contient pas le premier.
- **AC2.** Contrôles négatifs dans le même bloc : une fixture avec un seul refus
  `(non-terminal)` donne la note de refus non terminal, sans Halt event ; une
  fixture avec un seul refus `(terminal)` donne un extrait identique à l'actuel.
  Les tests existants du bloc mika#2493 restent verts.
- **AC3.** Les deux sites (dev-pilot et dev-groom) passent par le même appel.
  Un scan épinglé par un test refuse un troisième site qui construirait
  `POLICY_DENY` autrement.
- **AC4.** Sonde : rejouer la fonction livrée sur les 118 sessions du tableau
  Mesure, et écrire sur le ticket le compte de sessions dont le Halt event cite
  le terminal. La cible est **118 sur 118**, contre 20 aujourd'hui. Pour chaque
  `result` produit après le deploy et qui contient `Halt event:`, l'événement
  cité porte `(terminal)`. La sonde est fermable au deploy, puisque le rejeu
  porte sur un corpus déjà présent.

### 5.1 Comment chacun est tenu

**AC1** — nouvelle fixture `two_denies.stderr` dans le bloc mika#2493, de la
forme exacte du cas d'origine `e71ada95` : un refus `cause=default-deny` à deux
lignes marqué `(non-terminal)`, une ligne `[debug]` entre les deux, un second
refus à trois lignes marqué `(terminal)` portant `[bash-grep]`, et une ligne
`[done]` finale. Quatre assertions sur `_t2493_excerpt` de cette fixture :

| assertion | ce qu'elle tient |
|---|---|
| `assert_contains` `[bash-grep]` | le `[rule-id]` du **terminal** est dans l'extrait |
| `assert_contains` `(terminal)` | le marqueur cité concorde avec la létalité annoncée |
| `assert_not_contains` `rev-parse --git-dir` | le premier refus, non terminal, **n'est pas** cité |
| `assert_not_contains` `[debug]` / `[done]` | les bornes d'événement tiennent toujours entre deux refus |

Plus un **contrôle de bonne foi** sur la même fixture, du modèle de celui de
T13 : `_policy_deny_lethality` y rend bien `terminal`. S'il rougit, la fixture
ne reproduit plus la forme mesurée et les quatre assertions ci-dessus ne
mesurent plus rien.

**AC2, premier contrôle négatif** — un seul refus `(non-terminal)` ⇒ la note
annexée, sans Halt event. La fixture `nonterm.stderr` existe déjà (T1). Le
chemin bout-en-bout est déjà couvert par T10/T11 (`_annex_policy_deny_note`),
donc l'ajout est l'assertion que l'extrait de cette fixture est **inchangé** —
c'est le refus `(non-terminal)` qui sort, le repli ayant servi.

**AC2, second contrôle négatif** — un seul refus `(terminal)` ⇒ extrait
identique à l'actuel. Fixture `term.stderr`, existante (T2), plus une fixture
*terminal multi-ligne* pour que le candidat terminal soit exercé sur une forme à
continuations et non seulement sur un mono-ligne.

**AC2, non-régression** — `bash skills/bundled/_shared/test-dispatch-lib.sh`
en entier. Les douze tests du bloc mika#2493 sont verts, T13 et T14 compris :
toutes leurs fixtures portent un refus unique, donc le candidat terminal est
soit absent (repli, comportement identique), soit l'unique événement.

**AC3** — le scan de §2.3, dans le même bloc, avec son allowlist vide et son
contrôle de bonne foi `≥ 2`.

**AC4** — script de sonde sous `.pilot-scratch/3040/`, cf. §7.

---

## 6. Definition of Done

- [ ] `_policy_deny_excerpt` sélectionne le premier événement **terminal** du
      flux, et retombe sur le premier événement quand aucun n'en porte le
      marqueur (G1).
- [ ] Les trois propriétés conservées de §2.1 le sont : sous-chaîne littérale,
      indentation à 4 espaces des continuations, aucun court-circuit de stdin.
- [ ] `_policy_deny_read` existe, publie `POLICY_DENY` et
      `POLICY_DENY_LETHALITY`, porte le strip ANSI et la garde `-f`/`-r`, et son
      commentaire nomme le renversement de la consigne « strip at the site » de
      mika#2493 (G2).
- [ ] Les deux sites (l. ~8324-8335 et l. ~8559-8575) sont réduits à
      `_policy_deny_read "$PERSISTENT_STDERR_PATH"`, la construction du chemin
      restant au site.
- [ ] L'ordre des conjoints des deux branches de halt est **intact** :
      `if [ -n "$POLICY_DENY" ] && [ -z "$VALID_PLAN" ] && [ "$POLICY_DENY_LETHALITY" = "terminal" ]`.
      Il est porteur (mika#2493 D1) et trois assertions existantes le mesurent
      par sous-chaîne littérale — T8, Test 13, Test 14.
- [ ] Le scan G3 est livré armé, vert, avec son allowlist vide et son contrôle
      de bonne foi (§4).
- [ ] `bash skills/bundled/_shared/test-dispatch-lib.sh` est vert en entier,
      aucun `SKIP` nouveau.
- [ ] `shellcheck` sur `dispatch-lib.sh` et `test-dispatch-lib.sh` ne régresse
      pas (mêmes exclusions qu'aujourd'hui).
- [ ] `make verify-bundled-skills` est vert.
- [ ] `scripts/verify-pipeline.sh origin/main` est vert.
- [ ] La sonde AC4 a tourné, et son compte est écrit dans le corps de la PR
      **et** en commentaire sur mika#3040.
- [ ] Le corps de PR porte `Closes #3040` et
      `Code review: delegated to mika-qa (mika#2694)`.

---

## 7. La sonde AC4, et comment elle se ferme

`/var/log/claude-pilot/` est **lisible depuis le worktree de dispatch** —
vérifié au grooming. La sonde n'est donc pas une demande à l'opérateur : le
pilote d'implémentation la fait tourner lui-même.

Script à écrire sous `.pilot-scratch/3040/probe-ac4.sh`, lancé par
`bash .pilot-scratch/3040/probe-ac4.sh`. Il source `dispatch-lib.sh`, parcourt
les `*.stderr` du répertoire, et pour chacun :

1. `_policy_deny_lethality "$f"` ; ne retient que les `terminal` — c'est la
   population des 118 ;
2. `_policy_deny_read "$f"`, puis teste si `$POLICY_DENY` contient la
   sous-chaîne littérale `(terminal)` ;
3. imprime trois comptes : la population `terminal`, celles dont l'extrait porte
   `(terminal)`, celles dont il ne le porte pas.

Le même script, lancé en *mode témoin* sur l'ancienne fonction (extraite de
`9a478c3c4` via `git show` sous `.pilot-scratch/`), doit rendre **20** — sans ce
témoin, un 118/118 ne distingue pas « la fonction corrige » de « la mesure du
ticket était fausse ». Les deux nombres vont ensemble dans le corps de PR.

| lecture | témoin (`9a478c3c4`) | livrée | source |
|---|---|---|---|
| population `terminal` | 118 | 118 | tableau Mesure |
| extrait citant le terminal | 20 | **118** | cible AC4 |

La sonde est **fermable au deploy** : le corpus est déjà présent, le rejeu est
hermétique, rien n'attend une session future. Le second membre d'AC4 — « pour
chaque `result` produit après le deploy qui contient `Halt event:`, l'événement
cité porte `(terminal)` » — est la conséquence du rejeu, non une seconde
observation à attendre ; au 2026-11-05 07:57Z, `tasks` ne contient d'ailleurs
qu'une seule ligne de cette forme.

Le script reste un **brouillon** sous `.pilot-scratch/` : il n'est pas livré, et
il n'est jamais supprimé (mika#2548).

---

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — awk de sélection de `_policy_deny_excerpt` (code + commentaire) | 55 |
| `skills/bundled/_shared/dispatch-lib.sh` — `_policy_deny_read` (code + commentaire) | 35 |
| `skills/bundled/_shared/dispatch-lib.sh` — les deux sites d'appel réduits (net négatif, compté à 0) | 0 |
| tests (`skills/bundled/_shared/test-dispatch-lib.sh`) — AC1 : fixture deux refus + 4 assertions + bonne foi | 45 |
| tests (`skills/bundled/_shared/test-dispatch-lib.sh`) — AC2 : deux contrôles négatifs + fixture terminal multi-ligne | 35 |
| tests (`skills/bundled/_shared/test-dispatch-lib.sh`) — AC3 : allowlist + scan + bonne foi | 40 |

Total estimé : 210 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 210 est à moins de la moitié de la borne par
vol et au cinquième du seuil par PR. Deux raisons de forme s'ajoutent au
nombre : la surface est **un seul fichier de code et un seul fichier de test**,
sans crate Rust, sans compilation, donc sans tour de `cargo build` ; et le bloc
de test existe déjà — l'implémenteur l'étend, il ne le crée pas. La boucle de
vérification est `bash test-dispatch-lib.sh`, de l'ordre de quelques secondes,
ce qui rend les itérations peu coûteuses en tours. Le seul poste au coût
incertain est la sonde AC4 (3 162 fichiers × deux sous-processus), bornée à
quelques tours parce qu'elle tourne en une passe de script et imprime trois
nombres.

---

## 9. Ordre de commit

Chaque commit laisse la suite de tests verte — c'est le plancher, et il
interdit de livrer un test rouge en attendant le suivant.

1. **`fix(3040): l'extrait de refus cite l'événement terminal quand il existe`**
   — G1 (l'awk) **et** les fixtures AC1/AC2 dans le même commit. Les deux sont
   indissociables : la fixture AC1 est rouge avant G1, et G1 sans elle est un
   changement de comportement non mesuré. Les douze tests de mika#2493 restent
   verts à ce commit, ce qui est la non-régression d'AC2.
2. **`refactor(3040): les deux sites lisent le refus par un seul appel`**
   — G2 (`_policy_deny_read` + réduction des deux sites) et G3 (allowlist +
   scan + bonne foi). Le scan est écrit dans le même commit que la refonte
   qu'il épingle : écrit avant, il serait rouge ; écrit après, rien ne garantit
   qu'il a jamais été rouge. L'assertion d'ordre des conjoints (T8, Test 13,
   Test 14) est le garde-fou de ce commit.
3. **La sonde AC4** ne fait l'objet d'aucun commit — brouillon
   `.pilot-scratch/`. Ses deux nombres (témoin et livrée) vont dans le corps de
   PR et en commentaire sur mika#3040.
