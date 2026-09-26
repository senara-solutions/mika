# feat(loop-substrate) : le plafond de tours du pilote se résout depuis le label du ticket

> **Ticket :** `senara-solutions/mika#2542`
> **Branche :** `feat/2542/max-turns-par-label-loop-substrate-200-d`
> **Classe :** loop-substrate. Origine : bearing Prime 2026-09-26 12:20 sur dossier n=3.
> **Réf. mécanisme :** mika#2496 (`_pilot_max_turns` / `PILOT_MAX_TURNS` / `pilot_budget_armed`).

---

## 0. Ce que la lecture du code déplace dans le ticket

C'est le premier livrable, et il change la **conception**, pas seulement la prose.

### R1 — le « défaut global 150 » n'existe pas dans le code : le défaut in-file est DÉSARMÉ

Le ticket écrit « défaut global reste **150** » et « Le global reste 150 ». Le code dit autre
chose. `skills/bundled/_shared/dispatch-lib.sh:325` :

```sh
_pilot_max_turns() {
    # LE défaut de flotte, un seul site. Vide = désarmé ; pour armer, écrire le
    # plafond ici (`local _default=120`) une fois V2 rapportée sur le ticket.
    local _default=""
```

Épinglé mot pour mot par le harnais, `test-dispatch-lib.sh:6548` :

```
assert_eq "mika#2496: sans surcharge, le défaut de flotte est DÉSARMÉ (V2 non fournie)" \
    "|default|" "$(_mika2496_resolve_probe __UNSET__)"
```

**Le 150 observé est une variable d'environnement**, et son site est attesté par écrit dans
`crates/mika-agent/src/skills/executor.rs:328` (doc-comment de `PILOT_DISPATCH_ENV`) :

> « (`PILOT_MAX_TURNS=150` est déjà posé dans `~/.mika/.env` […]) »

Donc la provenance réellement émise aujourd'hui sur un dispatch non-substrat est
**`source=env`**, jamais `source=default` — et le ticket hésite lui-même sur ce point
(« montre `max_turns=150 source=default` (**ou `env`**) »).

### R1-bis — la conséquence est ÉLIMINATOIRE pour la conception

Si le palier `label` est placé **sous** `env` dans la cascade, alors sur l'hôte de production —
où `PILOT_MAX_TURNS=150` est posé — l'env gagne **toujours** et le mécanisme livré ici est
**entièrement inerte**. Mergé, déployé, zéro effet, aucune ligne rouge.

C'est la classe mika#2205 appliquée au correctif lui-même : *une garde qu'on n'a pas déployée
se lit exactement comme une flotte saine.* L'ordre de la cascade n'est donc pas un détail de
mise en œuvre, c'est le livrable central — voir § D1.

### R2 — `loop-substrate` n'est déclaré NULLE PART dans `.github/labels.yml`

Vérifié : `grep -n "loop-substrate" .github/labels.yml` ne rend **rien**. Et
`.github/workflows/labels.yml` :

```yaml
on:
  push:
    branches: [main]
    paths:
      - ".github/labels.yml"
...
          delete-other-labels: true
```

Donc **le label est en danger aujourd'hui, indépendamment de ce ticket** : le prochain push
touchant `labels.yml` — pour n'importe quelle raison — le supprime du dépôt **et de chaque
ticket le portant**, sans événement `unlabeled` et sans une ligne de journal.

Construire un mécanisme d'enforcement qui lit ce label sans le déclarer livrerait un détecteur
dont le silence serait indistinguable d'une flotte saine. C'est la classe
`docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md`,
dont le dépôt compte déjà cinq occurrences (`dispatch:ssc`, `dispatch:mpc`,
`operator-review`/`blocked`, `human-review-required` mika#2199, `needs-multi-agent-review`
mika#2201). **La déclaration est un livrable obligatoire du même commit** — et c'est aussi ce
qui fait passer la garde G1 d'emblée, allowlist vide.

### R3 — le ticket a raison sur un point, et c'est celui qui rend le travail petit

« lire le label du ticket (**déjà résolu au dispatch**) » : exact.
`dispatch-lib.sh:2739`, dans `_set_up_worktree()` :

```sh
LABELS=$(printf '%s' "$ISSUE_JSON" | jq -r '[.labels[].name] | join(",")' 2>/dev/null)
```

Aucun appel `gh` supplémentaire n'est nécessaire. Le format est un CSV `a,b,c`.

---

## 1. Décisions

### D1 — la cascade, et son ordre EST le livrable

Quatre paliers. L'ordre est contraint par R1-bis en haut et par le rollback documenté de
mika#2496 en bas :

| # | condition | résultat | `source=` |
|---|---|---|---|
| 1 | `PILOT_MAX_TURNS` **défini** et (vide ou `0`) | **ROLLBACK** — le drapeau n'est pas passé | `env` |
| 2 | un label de la table est porté | ce plafond (`200`) | **`label`** |
| 3 | `PILOT_MAX_TURNS` entier positif | cette valeur | `env` |
| 4 | sinon | le défaut in-file (`150`) | `default` |

**Palier 1 au-dessus du label, et c'est ce qui garde mika#2496 vrai.** Son `0` est documenté
comme *« LE ROLLBACK »* — « claude-pilot retombe sur son propre `maxTurns=200`, soit le
comportement d'avant mika#2496 à l'octet près ». Un label qui écraserait le rollback ferait
cesser le rollback d'être un rollback, pendant un incident, sans que rien ne le dise.

**Palier 2 au-dessus du palier 3** : imposé par R1-bis. C'est aussi la lettre du ticket
(« Source = **label du ticket** (pas une variable globale) »).

**Coût nommé, et il est réel.** Un opérateur qui poserait `PILOT_MAX_TURNS=50` pour arrêter une
hémorragie de coût **ne borne pas** la classe `loop-substrate` sous 200 : le label gagne. Les
deux gestes qui marchent alors sont retirer le label du ticket, ou poser `PILOT_MAX_TURNS=0`
(palier 1), qui désarme la borne mika du dispatch. Ce coût est accepté et non compensé par une
variable d'échappement de plus : `max(label, env)` a été écarté (voir § 8, arbitrage refusé n°1)
parce qu'il rendrait `source=` inexprimable en un mot sans mesure qui le demande.

### D2 — armer le défaut in-file à 150

Le ticket exige que « un runaway sur un ticket non-substrat doit **toujours** être coupé à
150 ». Aujourd'hui cette borne n'existe que par **configuration d'hôte** (R1) : elle est
invisible dans le code, non testable, et disparaît avec une ligne de `~/.mika/.env`. Livrer une
**exception** (`loop-substrate → 200`) à une **règle qui n'existe pas dans le code** est la forme
la plus fragile possible du travail demandé.

**La V2 de mika#2496 — mesure bloquante préalable à l'armement — est maintenant satisfaite**, et
c'est le ticket lui-même qui la fournit : `150` tourne en production depuis le 2026-09-24, la
population saine passe dessous (aucune troncature nominale rapportée), et les trois seuls
dépassements mesurés sont exactement la classe qu'on exempte. Le commentaire du code prescrit
ce geste de sa propre main : *« ou porter ce défaut à 120 une fois V2 rapportée sur le ticket »*.

**Ce n'est pas une violation du « hors périmètre » du ticket.** Celui-ci dit « Le plafond global
(reste 150) » : la **valeur** ne change pas — 150 avant, 150 après. Ce qui change est que la
règle devient vraie dans le code au lieu d'être vraie sur un hôte.

**Repli explicite si l'architecte exclut D2** : garder `local _default=""` et ne livrer que le
palier 2. Le comportement observable sur l'hôte de production est identique (substrat 200 par
label, reste 150 par env) ; ce qu'on perd est la garantie structurelle et l'AC « défaut global
= 150 » devient une propriété de l'environnement, à écrire comme telle dans le corps de PR.

### D3 — appariement EXACT sur un élément CSV, jamais un glob de sous-chaîne

`_label_to_type` (`dispatch-lib.sh:7210`) emploie `*bug*` — tolérable pour choisir un préfixe de
commit, **faux** pour un mécanisme qui relève un plafond de coût : `*loop-substrate*`
apparierait `not-loop-substrate` ou `loop-substrate-v2`. Le CSV vient de `join(",")`, donc on
encadre (`,${LABELS},`) et on cherche `,loop-substrate,`. Fixtures négatives obligatoires.

### D4 — la table est UN SEUL SITE NOMMÉ, et son format est un format de fil

Exigé par le ticket (« une seule table nommée »). En shell, une constante tableau
`PILOT_LABEL_TURN_CEILINGS` au format `label=plafond`, parce que c'est la forme qu'un script de
garde peut parser sans exécuter le fichier :

```sh
# mika#2542 — la table label → plafond de tours. UN SEUL SITE.
PILOT_LABEL_TURN_CEILINGS=(
    "loop-substrate=200"
)
```

Le format est **pinné par un test** : la garde G1 en extrait les clés, donc une réécriture qui
changerait la forme rendrait la garde silencieusement aveugle (classe mika#2205).

### D5 — argument explicite, jamais la portée dynamique de bash

`LABELS` est une globale du script (`dispatch-lib.sh:2739`, sans `local`), donc *techniquement*
lisible depuis les trois sites de lancement. Elle n'est pas lue implicitement pour autant :
`_pilot_max_turns "${LABELS:-}"`. La raison est celle que le doc-comment de `_pilot_log_dir`
écrit déjà — un accesseur qui dépend d'un état posé 500 lignes plus haut par une **autre**
fonction peut être lu périmé, et rien ne le dirait.

Un site qui oublierait l'argument résout `$1` vide, donc **aucun label, donc le défaut global** :
l'oubli est **fail-safe vers 150**, jamais vers 200 — visible (un implement substrat tronqué),
jamais destructeur. La garde G2 le refuse quand même, parce que sans elle ce site serait
silencieusement hors du mécanisme.

### D6 — la provenance dit AUSSI quel label a décidé

`source=label` seul ne suffira plus dès qu'une seconde entrée existe dans la table. La ligne
porte donc, **et seulement quand le label a décidé**, un champ de plus :

```
dispatch-lib: pilot_budget_armed max_turns=200 source=label label=loop-substrate cost_bound=absent_upstream
```

Extension compatible : tout grep existant sur `^dispatch-lib: pilot_budget_armed`, `max_turns=`
ou `source=` continue d'apparier. La position du champ est après `source=` et avant
`cost_bound=`, qui reste en dernier.

### D7 — les trois sites reçoivent l'argument, uniformément

`_run_claude_pilot` (3233, chemin nominal implement **et** groom), `_launch_revise_pilot` (6532)
et `_fd_retry_if_section_still_missing` (6652) passent `"${LABELS:-}"`. Un dispatch de texte
libre (mika#1593) n'a pas d'issue, donc pas de label, donc retombe aux paliers 3/4 — correct,
un texte libre n'a pas de classe. Les deux sites de revise sont sur le chemin groom, qu'un
plafond de 200 n'atteindra jamais ; on les arme quand même, parce qu'une asymétrie à expliquer
coûte plus cher qu'un plafond qui ne mord pas.

### D8 — `200` est une valeur POSÉE, pas mesurée, et ça doit être écrit

Les trois runs ont été **coupés** à 151 ; on ne sait pas combien de tours ils auraient pris. 200
est le pari du bearing (+33 %). Si la même classe recoupe à 201, c'est la **valeur** qu'il faut
revoir, pas le mécanisme — et la sonde S2 le dira sans ambiguïté grâce à `source=label` (voir
§ 6 : `200` est aussi le défaut amont de claude-pilot, et l'instrument est ce qui sépare les deux).

---

## 2. Livrables

### R-1 — `skills/bundled/_shared/dispatch-lib.sh`

1. **La table** `PILOT_LABEL_TURN_CEILINGS` (D4), posée immédiatement avant `_pilot_max_turns`,
   avec son commentaire : ce que le label déclare, pourquoi la valeur est posée et non mesurée
   (D8), et le fait que ses clés sont sous garde G1.
2. **Le helper** `_pilot_label_turn_ceiling` — **il ASSIGNE, il n'imprime pas**
   (`_PILOT_LABEL_CEILING`, `_PILOT_LABEL_CEILING_NAME`), pour la raison mika#2039 que
   `_pilot_max_turns` documente déjà : une substitution `$(...)` dans ce voisinage pose
   `++ printf %s <valeur>` dans la trace `set -x`, forme qu'aucun scrubber ne couvre. Rend 0
   quand un label a décidé, 1 sinon. Appariement exact CSV (D3).
3. **`_pilot_max_turns` prend un argument** (`${1:-}`) et implémente la cascade D1. Son défaut
   in-file passe à `150` (D2). L'invalidité (`_PILOT_MAX_TURNS_INVALID`) est posée
   **indépendamment** de la résolution : un `PILOT_MAX_TURNS=abc` sur un ticket `loop-substrate`
   résout 200 par label **et** émet `pilot_budget_invalid`. Le résolveur ne relit jamais
   `$_PILOT_MAX_TURNS` (contrainte existante, conservée).
4. **`_emit_pilot_budget_line`** ajoute le champ `label=` conditionnellement (D6).
5. **Les trois sites de lancement** passent `"${LABELS:-}"` (D7).

### R-2 — `.github/labels.yml`

Déclarer `loop-substrate` dans la section *Component*, avec une description qui nomme sa
conséquence machine — sans quoi un futur éditeur le lit comme un label de rangement :

```yaml
- name: loop-substrate
  color: "0052cc"
  description: "Substrat de la boucle autonome (dispatch, gardes, reapers). Relève le plafond de tours pilote à 200 (mika#2542)."
```

### R-3 — `scripts/check-pilot-turn-ceiling-labels.sh` (détecteur G1)

Motif `scripts/check-dispatch-seats-declared.sh` (mika#2092), repris **avec ses codes de
sortie** : `0` propre, `1` divergence, `2` fichier illisible, `3` **une liste n'a pas pu être
parsée du tout** — *une garde qui ne trouve rien à vérifier doit le dire au lieu de passer*
(anti-vacuité, classe mika#2205).

Le sens du scan : **toute clé de `PILOT_LABEL_TURN_CEILINGS` doit être déclarée dans
`labels.yml`**. Un seul sens, délibérément — l'inverse (« tout label déclaré doit être dans la
table ») n'a aucun sens ici, `labels.yml` portant des dizaines de labels qui ne sont pas des
paliers de tours. La différence avec mika#2092, dont la garde est bidirectionnelle, est que
là-bas les deux listes sont le **même** vocabulaire ; ici l'une est un sous-ensemble.

Arguments optionnels (`dispatch-lib.sh`, `labels.yml`) pour que le harnais négatif puisse le
pointer sur des fixtures — même contrat que son modèle.

### R-4 — `scripts/test-check-pilot-turn-ceiling-labels.sh` (le contrôle négatif de G1)

Fixtures **vues rouges** : une table dont la clé manque à `labels.yml` → exit 1 ; une table
vide → exit 3 ; un fichier illisible → exit 2. Fixture **vue verte** : table + déclaration
cohérentes → exit 0. Sans ce harnais, G1 peut devenir inerte (renommage du tableau, changement
de format) et se lire exactement comme un arbre propre.

### R-5 — `Makefile` + `.github/workflows/ci.yml`

Cible `check-pilot-turn-ceiling-labels` (le script + son harnais négatif, motif
`check-dispatch-seats-declared` lignes 256-258) et un job CI `pilot-turn-ceiling-labels-lint`
sur le modèle de `dispatch-seats-lint` (`ci.yml:283`). **Une garde non branchée en CI est une
garde qui n'existe pas.**

### R-6 — `skills/bundled/_shared/test-dispatch-lib.sh` (détecteurs G2, G3, G4)

Extension du bloc mika#2496 existant, **sans toucher** à ses assertions de scan de lancement ni
à son allowlist vide (le ticket l'exige : « ce ticket change la VALEUR résolue, pas le fait
qu'elle soit passée »).

### R-7 — documentation

`mika/CLAUDE.md`, section `Optional (pilot turn budget, armed at the source — mika#2496)` :
le tableau de la cascade, `source=label`, la table, le nouveau défaut in-file, les surfaces et
les sondes. Plus une ligne dans la sous-section *Labels* rappelant que `loop-substrate` porte
désormais une conséquence machine.

---

## Fire-Disposition

Ce plan livre quatre détecteurs : **G1** (table ↔ `labels.yml`), **G2** (les trois sites passent
l'argument), **G3** (le comportement de la cascade), **G4** (la valeur du défaut in-file).

**Disposition retenue : (a) exception nommée en allowlist — et les deux allowlists sont
LIVRÉES VIDES.**

Ce n'est pas une facilité : c'est possible parce que **le même commit répare l'unique violation
préexistante**. La seule violation que G1 pourrait accuser est `loop-substrate` absent de
`labels.yml` (R2), et R-2 la ferme dans le même commit. G2 ne peut accuser que les trois sites
que R-1 arme. Il n'existe donc **aucune donnée à exempter**, et la doctrine mika#2201 s'applique
telle quelle : *« on déclare, on n'allowliste pas »* — quand G1 tire, la résolution est une
ligne dans `labels.yml`, jamais une entrée d'exemption.

Les deux allowlists existent malgré leur vacuité, et sont **pinnées vides par une assertion**,
sur le modèle littéral de mika#2496 (`assert_eq "mika#2496: l'allowlist des lancements non
bornés est livrée vide" "0" "${#MIKA2496_LAUNCH_EXCEPTIONS[@]}"`) :

```sh
MIKA2542_CEILING_LABEL_EXCEPTIONS=()   # aucune. Quand G1 tire : déclarer le label.
MIKA2542_UNARGUMENTED_SITES=()         # aucune. Quand G2 tire : passer "${LABELS:-}".
assert_eq "mika#2542: l'allowlist des labels non déclarés est livrée vide" "0" \
    "${#MIKA2542_CEILING_LABEL_EXCEPTIONS[@]}"
assert_eq "mika#2542: l'allowlist des sites sans argument est livrée vide" "0" \
    "${#MIKA2542_UNARGUMENTED_SITES[@]}"
```

**Assertion auto-nettoyante** — l'exigence (a) de mika#1574 même sur une allowlist vide : chaque
allowlist est comparée **dans les deux sens**. Une entrée qui n'apparie plus rien fait rougir le
build, donc une exemption devenue périmée est retirée le jour de la réparation et non des mois
plus tard. Le même mécanisme rend la vacuité tenable : livrée vide, elle **reste** vide sans
qu'un relecteur ait à y penser.

**Anti-vacuité de chaque détecteur** — un détecteur qui ne regarde plus rien passe :
- G1 : exit `3` si la table ou la liste de labels ne se parse pas du tout (R-3).
- G2 : cardinalité **assertée à 3** (le nombre de sites de lancement), reprise du motif
  mika#2496 (`assert_eq "mika#2496: le scan voit exactement les trois sites de lancement" "3"`).
  Sans elle, un prédicat devenu trop étroit passerait en ne regardant rien.
- G3 : chaque palier de la cascade a son assertion, **et** les fixtures d'appariement négatif
  de D3 sont **vues rouges** avant d'être vues vertes.
- G4 : le défaut in-file est comparé à `150` littéral ; il rougit si quelqu'un le déplace sans
  refaire l'arithmétique de ce plan.

---

## 4. Contrat de vérification

### V1 — la cascade, palier par palier (G3, harnais shell)

Extension de `_mika2496_resolve_probe` avec un second argument (les labels). Attendus, au format
`valeur|source|invalid` :

| `PILOT_MAX_TURNS` | labels | attendu |
|---|---|---|
| non défini | `""` | `150\|default\|` |
| non défini | `loop-substrate` | `200\|label\|` |
| `120` | `""` | `120\|env\|` |
| `120` | `loop-substrate` | `200\|label\|` ← **R1-bis : le label bat l'env** |
| `0` | `loop-substrate` | `\|env\|` ← **rollback préservé, le label n'a pas voix** |
| `""` | `loop-substrate` | `\|env\|` |
| `abc` | `""` | `150\|default\|abc` |
| `abc` | `loop-substrate` | `200\|label\|abc` ← le label décide **et** l'invalidité est dite |

**Les six assertions mika#2496 existantes restent, valeur mise à jour pour les deux qui
dépendent du défaut** (`|default|` → `150|default|`). Leur modification est le signal de D2 :
elles sont ce qui pinne le défaut de flotte, donc leur diff **est** la décision.

### V2 — appariement exact, fixtures négatives VUES ROUGES (G3/D3)

`not-loop-substrate`, `loop-substrate-v2`, `xloop-substrate`, `loop-substratex` → **aucun**
relèvement. Et le contrôle de bonne foi, vu **vert** : `ready,loop-substrate,p1-important`
relève bien (le label au milieu d'un CSV), ainsi qu'en tête et en queue de liste.

### V3 — le drapeau atteint l'argv avec la valeur du label (G3)

Extension de `_mika2496_argv_probe` : labels `loop-substrate` → l'argv porte
`--max-turns 200`. Rollback → le drapeau est **absent** de l'argv. C'est le contrat côté mika ;
« le SDK coupe à 200 » est exécuté dans un autre processus et relève de la sonde S2.

### V4 — les trois sites passent l'argument (G2)

Sur les invocations **logiques** (continuations `\` rejointes — deux des trois sites portent le
drapeau sur une ligne de continuation, un prédicat à la ligne physique les accuserait à tort ;
c'est la leçon déjà payée par le scan mika#2496) : tout appel de `_pilot_max_turns` dans
`dispatch-lib.sh` est suivi d'un argument. Cardinalité assertée à 3.

### V5 — la ligne d'observabilité (G3)

`pilot_budget_armed` porte `source=label label=loop-substrate` quand le label décide, **et
aucun champ `label=`** sinon (contrôle négatif : un champ vide se lirait comme un label nommé
« vide »). `cost_bound=absent_upstream` reste en dernier et inchangé. L'assertion mika#2496
(AC7) « aucun site ne passe `--max-budget` » reste verte.

### V6 — la garde G1 et son contrôle négatif

`bash scripts/check-pilot-turn-ceiling-labels.sh` → `0` sur l'arbre réel, en **annonçant le
nombre de clés vues** (`1 ceiling label(s) checked`). `bash scripts/test-check-pilot-turn-ceiling-labels.sh`
→ `0`, chaque fixture rouge vue rouge et la verte vue verte.

### V7 — non-régression du bloc mika#2496

`make test-dispatch-lib` vert en entier. Les assertions de scan de lancement, de co-localisation,
de non-impression du résolveur, d'absence de `$(_pilot_max_turns)` et d'AC7 sont **inchangées**.

### V8 — ce qui n'est PAS testable ici, écrit plutôt que découvert

« Le pilote s'arrête bien à 200 tours » est exécuté par le SDK claude-pilot, dans un autre
processus, contre un vrai fournisseur. Le contrat **côté mika** est *le plafond résolu depuis le
label atteint l'argv*, et V3 l'atteste déterministiquement. La moitié comportementale est la
sonde S2.

---

## 5. Surfaces opérateur

```bash
# 1. Sous quel plafond, et par quelle porte ? (ancrage OBLIGATOIRE)
grep -h '^dispatch-lib: pilot_budget_armed' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail

# 2. Les dispatches relevés par le label
grep -h '^dispatch-lib: pilot_budget_armed.*source=label' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | wc -l

# 3. CONTRÔLE POSITIF — combien de dispatches ont résolu un budget, toutes portes ?
grep -l '^dispatch-lib: pilot_budget_armed' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | wc -l

# 4. Le plafond a-t-il mordu ?
grep -h 'error_max_turns' "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail
```

| événement / champ | régime attendu | lecture |
|---|---|---|
| `pilot_budget_armed` | **un par dispatch** | son absence = un binaire antérieur (classe mika#2340), **jamais** « pas de budget » |
| `source=label` | **non vide, minoritaire** | chaque ligne est un implement substrat qui a reçu ses 200 tours |
| `source=env` / `default` avec `max_turns=150` | **majoritaire** | le trafic nominal, borné comme avant |
| `source=label` sur un ticket **sans** `loop-substrate` | **zéro** | appariement trop large — voir Halte 4 |
| `pilot_budget_invalid` | **vide** | une coquille dans la variable, nommée entre guillemets |
| `error_max_turns` avec `source=label` | **non vide, faible** | un emballement coupé à 200 : **c'est le succès**, pas l'échec — mais voir D8/Halte 2 |

**Ancrage `^dispatch-lib: ` obligatoire, jamais le jeton nu.** Ce `.stderr` porte aussi la prose
du pilote, et mika#2050 a mesuré le faux positif : une session *discutant* du signal se lisait
comme une émission. Une session de grooming de **ce ticket-ci** recrée exactement ce faux
positif.

**Limite héritée, non refermée ici** : les deux pilotes de revise redirigent vers un `mktemp`
qu'ils suppriment quelques lignes plus bas, donc leur ligne n'est persistée nulle part
(Signal S, § *Limit — the revise path captures nothing*). La sonde ci-dessus vaut pour le
chemin de dispatch nominal.

---

## 6. Sondes post-déploiement, et leurs cinq haltes

> **Préalable, non négociable.** `skills/bundled/_shared/` est une projection du **binaire**,
> pas du checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer` doit porter le sha qu'on
> vient de bâtir. **Sans cette vérification, chaque sonde ci-dessous décrit le binaire d'hier.**

**S1 — le relèvement atteint l'argv (premier implement `loop-substrate` après déploiement).**
Une ligne `pilot_budget_armed max_turns=200 source=label label=loop-substrate`, et
`--max-turns 200` dans l'argv réel.

> **Halte 1 — `max_turns=150 source=env` sur un ticket qui porte bien le label.** Le palier
> `label` est sous `env` dans la cascade livrée : c'est R1-bis réalisé, le ticket est inerte.
> **Ne pas retirer `PILOT_MAX_TURNS` de `~/.mika/.env` pour « faire marcher »** — ce serait
> masquer le défaut en désactivant la borne de flotte. Réparer l'ordre de la cascade.

> **Halte 1-bis — aucune ligne du tout.** Établir le déploiement **avant** de toucher au
> résolveur (préalable ci-dessus). *Une ligne absente ne prouve rien tant qu'on n'a pas établi
> que le binaire qui tourne sait l'écrire.*

**S2 — la classe substrat cesse d'être tronquée (30 jours).** Les implements `loop-substrate`
n'apparaissent plus dans `error_max_turns`, **alors que le contrôle positif (surface 3) est non
nul**. Zéro des deux ne prouve rien (classe mika#2205).

> **Halte 2 — un implement substrat coupe à 201.** Ce n'est **pas** une panne du mécanisme :
> c'est D8 réalisé, la valeur posée est trop basse. `source=label` sur la ligne est ce qui
> prouve que 201 vient de **notre** 200 et non du défaut amont de claude-pilot, qui vaut la même
> chose (`types.py:77`). Le remède est la valeur dans la table, avec la mesure des tours
> réellement consommés ; le mécanisme est confirmé, pas mis en cause.

**S3 — non-régression du trafic nominal (30 jours).** La part des dispatches **non**-substrat
finissant `error_max_turns` reste marginale, et `max_turns=150` reste la valeur majoritaire.

> **Halte 3 — elle porte du trafic nominal.** D2 a armé une borne sous la population saine :
> **relever le défaut in-file avec la distribution**, jamais laisser un frein couper le travail
> courant. *Un frein est un frein, pas un chemin.*

**S4 — contrôle négatif de l'appariement (7 jours).** Zéro `source=label` sur un ticket qui ne
porte pas `loop-substrate`.

> **Halte 4 — une occurrence.** L'appariement est trop large (D3 non tenu) : un plafond relevé
> sur une classe qui n'y a pas droit est un frein de coût désarmé en silence. Lire le champ
> `label=` pour voir quelle clé a apparié, réparer l'encadrement CSV, et **vérifier que les
> fixtures négatives de V2 ont bien été vues rouges**.

**S5 — le label survit à un sync (au prochain push touchant `labels.yml`).**
`gh label list --repo senara-solutions/mika | grep loop-substrate` rend toujours le label, et
les tickets le portant le portent encore.

> **Halte 5 — le label a disparu.** R2 n'a pas atterri, ou a été retiré depuis. Le mécanisme
> est **silencieusement inerte** et se lit exactement comme une flotte sans ticket substrat.
> C'est ce que G1 existe pour empêcher : vérifier d'abord que le job CI
> `pilot-turn-ceiling-labels-lint` tourne réellement sur les PR.

---

## 7. Ce que ce travail n'achète PAS

- **Il ne garantit pas qu'un implement substrat tient en 200 tours.** Il retire une troncature
  mesurée à 151 et déplace la borne de 33 % ; les trois runs de référence ayant été **coupés**,
  le nombre de tours qu'ils auraient pris est inconnu (D8). La sonde S2 est ce qui le mesurera.
- **Aucun compteur, aucun événement de journal nouveau.** Le seul instrument est le champ
  `source=label` sur une ligne qui existait déjà, et **son silence ne prouve rien tant que
  personne n'exécute les sondes** — sur un `.stderr` par dispatch, l'absence de `source=label`
  peut simplement vouloir dire qu'aucun ticket substrat n'a été dispatché.
- **Il ne rend pas le budget observable en base.** Pas de ligne `audit_events`, pas de compteur
  moteur : la ligne vit sur le sillon forensique par dispatch, exactement comme avant.
- **Il ne borne aucun coût en dollars.** Le frein en dollars n'existe pas en amont (le
  `if config.maxBudgetUsd > 0:` de `_sdk_guardrail_kwargs` se termine sur `pass`), et aucun site
  ne passe `--max-budget` — invariant mika#2496 AC7, conservé et re-testé. Relever un plafond
  de **tours** de 150 à 200 augmente mécaniquement le coût plafond de la classe substrat
  (≈ 0,43 USD/tour mesuré sur #2484, soit ≈ +21 USD au pire par dispatch relevé). C'est le prix
  assumé du travail que la troncature détruisait, et il est **mesuré** par
  `pilot_cost_overrun` (mika#2496 U4) sans être **empêché**.

---

## 8. Arbitrages refusés, et pourquoi

1. **`max(label, env)` plutôt que « le label prime ».** Séduisant — ni le label ni l'opérateur ne
   pourrait alors couper le travail que l'autre autorise. Refusé : `source=` cesserait de nommer
   **une** porte (que vaut-il quand env=300, label=200, résultat 300 ?), donc l'instrument
   perdrait la propriété qui fait tout l'intérêt de mika#2293, pour un cas d'usage que personne
   n'a mesuré. À rouvrir si la Halte 3 ou un incident montre un opérateur ayant besoin de
   **baisser** sous la classe substrat.
2. **Un glob `*loop-substrate*` à la manière de `_label_to_type`.** Refusé par D3 : le voisin
   est tolérant parce qu'il choisit un préfixe de commit, celui-ci relève un plafond de coût.
3. **Lire `$LABELS` par portée dynamique.** Refusé par D5 — fragilité que `_pilot_log_dir`
   documente déjà, sans compter qu'un site oublié serait invisible.
4. **Laisser le défaut in-file désarmé.** Refusé par D2, avec son repli écrit : l'exception
   serait livrée avant la règle qu'elle exempte.
5. **Une variable `PILOT_MAX_TURNS_SUBSTRATE` pour régler le 200 sans redéploiement.** Refusé :
   le ticket exige que la source soit le **label**, et une seconde variable globale rouvrirait
   la porte que D1 vient de fermer. La valeur se change dans la table, en une ligne, et la
   table est le site unique que le ticket demande.

---

## 9. Hors périmètre, délibérément

- **La valeur du plafond global** (reste `150` — D2 la rend vraie dans le code, ne la déplace pas).
- **Le frein en dollars** : `_sdk_guardrail_kwargs`, `--max-budget`, `error_max_budget_usd` —
  suivi `senara-solutions/claude-pilot`, comme mika#2496 l'a écrit.
- **Le compteur de tours en temps réel par frontière** — même suivi cpp ; c'est la moitié
  *exposition* que mika#2496 a déférée et que ce ticket ne réclame pas.
- **La classe deny-death (mika#1686, SSC)** : retard de déploiement claude-pilot, orthogonal au
  compteur de tours. Le ticket l'exclut nommément.
- **Le défaut amont `maxTurns=200` de claude-pilot** : inchangé, et c'est ce qui rend le rollback
  `PILOT_MAX_TURNS=0` équivalent au comportement d'avant mika#2496 à l'octet près.
- **Le sillon stderr du chemin revise** (limite Signal S, non refermée ici).
- **Retirer `PILOT_MAX_TURNS=150` de `~/.mika/.env`.** Recommandé mais **pas un livrable de
  code** : une fois D2 posé, la variable est redondante, et la retirer fait basculer la
  provenance des dispatches nominaux de `env` à `default` — donc rend la ligne plus lisible.
  C'est un geste d'opérateur sur l'hôte, à mentionner dans le corps de PR, jamais à exécuter
  depuis un bac à sable de dispatch.

- **L'asymétrie de tolérance entre les deux gates de section de plan.** Trouvée en rédigeant ce
  plan, réelle, et **non refermée ici**. `scripts/verify-pipeline.sh:173` tolère la numérotation
  du titre (`AC_HEADING_RE='^##[[:space:]]+([0-9]+\.?[[:space:]]+)?Acceptance criteria'`, ajoutée
  par mika#2516 après **n=3** occurrences), tandis que les deux lecteurs du gate
  Fire-Disposition (`dispatch-lib.sh:6607` et `:6673`) exigent `^## Fire-Disposition` **strict**.
  Un plan qui numérote ses sections — forme naturelle et fréquente — satisfait donc le premier
  gate et échoue le second, avec pour conséquence un ITERATE de première passe puis un ESCALATE
  **sans recours** en seconde. Ce plan y a échappé en retirant son propre numéro ; le prochain
  n'y échappera pas. **Suivi à ouvrir** : appliquer la tolérance mika#2516 aux deux lecteurs de
  `Fire-Disposition`. Hors périmètre ici parce que c'est le prédicat d'un **autre** gate, dont le
  blast radius est la convergence de tout grooming, et non le plafond de tours.

---

## 10. Acceptance criteria

*Transcrits de l'attendu du ticket, plus les critères que R1/R2 rendent nécessaires.*

- **AC1** — Un dispatch d'un ticket portant `loop-substrate` lance claude-pilot avec
  `--max-turns 200`, et sa ligne `pilot_budget_armed` porte `max_turns=200 source=label`.
- **AC2** — Un dispatch d'un ticket **sans** ce label est borné à `150` et sa ligne porte
  `source=default` (ou `source=env` si la variable est posée sur le service). Aucun champ
  `label=` n'est émis.
- **AC3** — **Le palier label prime sur `PILOT_MAX_TURNS` entier** : sur l'hôte de production,
  où `PILOT_MAX_TURNS=150` est posé, un ticket `loop-substrate` reçoit bien 200. *Sans AC3 le
  ticket est inerte (R1-bis).*
- **AC4** — Le rollback mika#2496 est **préservé** : `PILOT_MAX_TURNS=0` ou vide n'émet pas
  `--max-turns`, **y compris** sur un ticket `loop-substrate`.
- **AC5** — Le mapping label→plafond vit à **un seul site nommé**
  (`PILOT_LABEL_TURN_CEILINGS`), et aucun second lecteur de label ne décide d'un plafond.
- **AC6** — L'appariement est **exact sur un élément du CSV** : `not-loop-substrate`,
  `loop-substrate-v2` et leurs variantes ne relèvent rien.
- **AC7** — `loop-substrate` est **déclaré dans `.github/labels.yml`**, et une garde CI refuse
  toute clé de la table non déclarée (exit 3 si la table ne se parse pas).
- **AC8** — Les gardes mika#2496 sont **intactes** : les trois sites portent toujours
  `--max-turns`, l'allowlist des lancements non bornés reste vide, la co-localisation tient, le
  résolveur n'imprime pas, aucun site ne passe `--max-budget`.
- **AC9** — Le défaut in-file de `_pilot_max_turns` vaut `150` et est pinné par une assertion
  (D2) ; si l'architecte exclut D2, le repli documenté au § D2 s'applique et AC2 devient une
  propriété de l'environnement, écrite comme telle dans le corps de PR.
- **AC10** — Les trois sites de lancement passent les labels au résolveur, cardinalité assertée
  à 3.

---

## 11. Definition of Done

- [ ] R-1 : table, helper assignant, cascade D1, défaut `150`, champ `label=`, trois sites armés.
- [ ] R-2 : `loop-substrate` déclaré dans `.github/labels.yml` avec sa conséquence machine.
- [ ] R-3 / R-4 : garde G1 + son harnais négatif, fixtures rouges **vues rouges**.
- [ ] R-5 : cible `make` + job CI `pilot-turn-ceiling-labels-lint`.
- [ ] R-6 : V1–V5 et V7 verts ; les deux allowlists livrées vides et pinnées vides, double sens.
- [ ] R-7 : `mika/CLAUDE.md` § mika#2496 étendu (cascade, `source=label`, sondes, haltes).
- [ ] `make test-dispatch-lib` vert en entier ; `cargo clippy` et `cargo fmt` propres.
- [ ] `bash scripts/check-pilot-turn-ceiling-labels.sh` → 0 en annonçant un compte non nul.
- [ ] **Tail ownership.** PR sur `feat/2542/max-turns-par-label-loop-substrate-200-d`,
      **`Closes #2542`**, reviewer `mika-platform-qa`.
- [ ] Le corps de PR **nomme R1 et R2** — la rectification du « défaut global 150 » et le label
      non déclaré — parce que les deux sont des faits sur l'état du dépôt que le ticket ne porte
      pas, et que le second était un risque **antérieur** à ce travail.
- [ ] Le corps de PR nomme le coût du § 7 (+21 USD au pire par dispatch relevé) et la sonde S2
      comme condition de révision de la valeur `200` (D8).
- [ ] Le corps de PR nomme le geste opérateur du § 9 (retirer `PILOT_MAX_TURNS` de
      `~/.mika/.env`) comme **recommandé et non livré**.
