# feat(groom) — le Plan-Size Gate gagne une borne PAR VOL, dérivée du plafond de tours du ticket, à côté de sa borne par PR

- **Ticket :** senara-solutions/mika#2693 (`p1-important`, `ready`, `dispatch:loop`, `loop-substrate`, `origin:manual`)
- **Branche :** `feat/2693/plan-size-gate-une-borne-par-vol-d-riv-e`
- **Date :** 2026-10-14
- **Lignée :** mika#2496 (plafond de tours, `_pilot_max_turns`) → mika#2542 (plafond par label, `PILOT_LABEL_TURN_CEILINGS`) → mika#2636 (borne par PR, `_plan_size_max_loc` + les deux gates architecte) → mika#2544 (titre numéroté) → **ce ticket**. Calibration : `docs/solutions/dev-loop/une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite.md` (facteur estimé → réel).

---

## Contexte

mika#2636 a posé **une** borne sur la taille d'un plan : 1 000 lignes de code
estimées hors `docs/`, par PR. Elle a été mesurée sur des diffs, et elle tient ce
qu'elle promet : plus aucun plan de 1 500 lignes ne passe le groom.

Le 2026-10-14, **trois vols sont morts au plafond de tours en passant cette
borne** :

| ticket | plafond | estimé | réel hors `docs/` | 1er Edit | dernier commit de travail | mort |
|---|---|---|---|---|---|---|
| mika#2683 | 200 | 832 | 1 432 | t39 | t171 | 201, dans `ce-simplify-code` |
| mika#2654 | 200 | 880 | 1 160 | t36 | t177 | 201, dans `ce-simplify-code` |
| mika#2687 (vol 1) | 150 | 443 | 994 | t32 | t136 | 151, dans la vérification finale |

Aucun n'a ouvert sa PR. `dispatch-lib` les a sauvés en brouillon (mika#2689,
#2690, #2691) et l'orchestrateur a fini les trois à la main.

**La lecture du défaut.** Le travail tient dans le vol : le dernier commit de
travail tombe à t171, t177, t136. Ce qui ne tient pas, c'est la **fin de
pipeline** — `ce-simplify-code`, `ce-code-review`, la vérification complète, le
compound, la PR. Et cette fin de pipeline coûte un nombre de **tours**, pas un
nombre de lignes. Or :

- la borne de mika#2636 est **par PR, en lignes**, et elle est la même pour tous
  les tickets ;
- la contrainte réelle est **par vol, en tours**, et elle **dépend du plafond du
  ticket** : 150 par défaut, 200 avec `loop-substrate`
  (`PILOT_LABEL_TURN_CEILINGS`, mika#2542).

Les deux bornes ne se remplacent pas. Celle de mika#2636 protège la *revue* (une
PR de 1 500 lignes est irrelisible même si elle a été écrite en un vol de 400
tours) ; celle de ce ticket protège le *vol*. Elles cohabitent, et la plus serrée
mord.

**L'essai manuel a déjà eu lieu.** Sur mika#2687, la règle par vol a été donnée
au groom par un commentaire opérateur. Le groom l'a appliquée : il a écrit une
section de dimensionnement, découpé le ticket en deux vols, porté le vol 2 en
`Tracked in:`. La règle peut donc vivre dans le groom. L'essai a aussi montré ses
deux défauts :

1. **Il supposait un plafond de 200.** #2687 ne porte pas `loop-substrate`, donc
   son plafond est 150 : la borne appliquée était presque deux fois trop large,
   et le vol 1 est mort quand même.
2. **Il ne peut pas être une consigne posée entre deux vols.** L'implémentation
   part dans la même seconde que le succès du groom (mika#1614) : aucune fenêtre
   ne s'ouvre. La règle doit être **mécanique dans le groom**.

---

## Décisions de grooming

Le ticket laisse quatre points ouverts. Ils sont tranchés ici, pas pendant
l'implémentation.

### D1 — Le groomeur reçoit la borne DÉJÀ CALCULÉE, et sa dérivation avec

Le ticket ouvre l'alternative : « il faut [transmettre le plafond] au groomeur,
ou lui donner la borne déjà calculée ». **Les deux**, et dans cet ordre de
priorité : la borne, parce qu'un groomeur qui doit faire l'arithmétique la fera
une fois sur deux ; sa dérivation complète, parce que le ticket l'exige
explicitement (« un nombre nu ne pourrait pas être recalibré ») et parce que le
plan doit pouvoir la **recopier** dans son § Taille estimée.

`dispatch-lib` connaît le plafond par les labels à l'instant de l'injection
(`LABELS` est posée dans `_set_up_worktree`, ~450 lignes avant le site
d'injection du prompt). Elle calcule donc la borne et l'écrit dans la règle.

### D2 — L'architecte lit le plafond DANS LE PLAN, pas dans le ticket

C'est la décision structurante, et elle est imposée par le canal : `_arch_ask`
livre à mika-arch **le contenu du fichier de plan sur stdin**, et rien d'autre
(`mika ask --agent mika-arch … - < "$plan_path"`). Son `system_prompt.md` est
**statique** : aucune interpolation au dispatch. L'architecte ne peut donc
connaître ni les labels du ticket, ni son plafond, autrement que parce que **le
plan les déclare**.

Conséquence, et c'est le livrable central : le format prescrit du § Taille
estimée gagne **deux lignes de déclaration** — le plafond de tours en vigueur, et
la borne par vol avec sa dérivation. L'architecte :

1. lit le plafond que le plan déclare ;
2. recalcule la borne avec les paramètres que **son** prompt porte en littéral ;
3. compare au `Total estimé :` ;
4. refuse (ITERATE en 1re passe, ESCALATE en 2nde) si le total dépasse, **sauf**
   découpage en vols dont le vol courant repasse sous la borne et dont la suite
   est nommée par une ligne `Tracked in:`.

**Plan qui ne déclare aucun plafond ⇒ l'architecte prend 150.** C'est la
direction fail-safe, et c'est exactement celle que `_pilot_label_turn_ceiling`
a déjà écrite pour l'oubli d'argument : « l'oubli est fail-safe vers 150, jamais
vers 200 ». Une omission produit la borne la plus serrée, donc le plan le plus
petit, jamais l'inverse.

**Borne nommée, pas découverte :** rien ne vérifie que le plafond déclaré par le
plan est celui du ticket. Un plan qui écrirait 200 sur un ticket sans
`loop-substrate` obtiendrait la borne large. Ce n'est pas refermé ici, et la
raison est que le canal manque : l'architecte ne voit pas les labels. La sonde
qui le verrait est la ligne de journal de porte 3 — qui dérive la borne du
plafond **réellement passé en argv** — et elle est renvoyée au vol 2 (§ Hors
périmètre). Fermer la boucle demanderait de passer le plafond à `_arch_ask`, ce
qui est un changement de contrat de canal, hors de ce ticket.

### D3 — Aucune variable d'environnement nouvelle ; quatre constantes nommées, recalibrables par édition

Le ticket demande que chaque paramètre soit « porté par une constante nommée et
recalibrable ». Quatre paliers d'environnement (`PLAN_SIZE_FLIGHT_ORIENTATION`,
`…_TAIL`, `…_THROUGHPUT`, `…_FACTOR`) coûteraient quatre résolveurs assignants,
quatre scans de co-location (patron S11), quatre déclarations dans
`PILOT_DISPATCH_ENV` (`crates/mika-agent/src/skills/executor.rs`) et quatre
entrées dans `docs/operator/environment-variables.md` — pour un besoin qu'aucune
mesure ne réclame : ces quatre valeurs se recalibrent à l'échelle du **mois**, en
lisant une distribution, pas à l'échelle de l'incident.

Décision : **quatre constantes shell nues, un seul site, recalibrables en
éditant ce site** — et le test de parité (AC2) force les deux prompts architecte
à bouger dans le même commit. C'est précisément le mécanisme que mika#2636 a
retenu pour son littéral de seuil.

`PLAN_SIZE_MAX_LOC` (la borne **par PR**) reste ce qu'elle est, inchangée.

### D4 — Arithmétique entière, par mise à l'échelle des constantes

`bash` n'a pas de flottants. 7,5 lignes/tour et un facteur de 2 se portent donc
par deux constantes entières **dont le nom dit l'échelle** :

- `_PLAN_SIZE_FLIGHT_LOC_PER_100_TURNS=750` — 750 lignes réelles pour 100 tours ;
- `_PLAN_SIZE_FLIGHT_ESTIMATE_FACTOR_PCT=200` — facteur estimé → réel, 200 %.

Les deux mises à l'échelle **s'annulent**, et c'est ce qui rend le calcul exact
en une seule division :

```
borne = (plafond − orientation − fin_de_pipeline) × LOC_PER_100_TURNS / FACTOR_PCT
```

- plafond 200 : `(200 − 35 − 50) × 750 / 200 = 115 × 750 / 200 = 431`
- plafond 150 : `(150 − 35 − 50) × 750 / 200 = 65 × 750 / 200 = 243`

Ce sont les « environ 430 » et « environ 245 » du ticket, à la troncature
entière près. **Le commentaire du site dit l'annulation explicitement**, pour
qu'un recalibreur qui passerait le débit à 9 lignes/tour écrive `900` et non
`9`.

---

## Conception

### La répartition des portes est celle de mika#2636, inchangée

```
porte 1, le PROMPT du groomeur   `_PLAN_SIZE_RULE`      intention  ← étendue
porte 2, l'ARCHITECTE            Plan-Size Gate L2/L3   REFUS      ← étendue
porte 3, le DISPATCH implement   `plan_size_estimate`   MESURE     ← vol 2
```

Détection permissive, décision stricte. La porte 2 reste **la seule qui
refuse** — avec les verdicts qui existent déjà (ITERATE / ESCALATE), donc sans
aucun mode d'échec neuf dans la boucle. La porte 3 ne refuse jamais : la file
groomée entière le lui doit (un plan déjà groomé ne repasse par aucune porte de
groom au dispatch suivant).

### L1 — `skills/bundled/_shared/dispatch-lib.sh` : le site unique de vérité

Un bloc neuf, posé à côté de `_plan_size_max_loc` (voisinage délibéré : les deux
bornes du même gate se lisent ensemble), portant les quatre constantes, la
dérivation en commentaire — **la dérivation, pas sa justification** : les trois
morts du 2026-10-14, la provenance de chaque paramètre et la méthode de
recalibrage vont dans la solution-doc existante
(`docs/solutions/dev-loop/une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite.md`),
que le bloc cite en six lignes de pointeur — et un résolveur :

```sh
_PLAN_SIZE_FLIGHT_ORIENTATION_TURNS=35
_PLAN_SIZE_FLIGHT_TAIL_TURNS=50
_PLAN_SIZE_FLIGHT_LOC_PER_100_TURNS=750
_PLAN_SIZE_FLIGHT_ESTIMATE_FACTOR_PCT=200

_plan_size_per_flight_bound() {   # $1 = plafond de tours résolu (vide = désarmé)
    # ASSIGNE : _PLAN_SIZE_PER_FLIGHT_BOUND
    #           _PLAN_SIZE_PER_FLIGHT_CEILING
    #           _PLAN_SIZE_PER_FLIGHT_CEILING_SOURCE  (resolved|disarmed_fallback)
    #           _PLAN_SIZE_PER_FLIGHT_NOTE            ("" | ceiling_below_pipeline_floor)
}
```

Quatre propriétés, chacune reprise d'un précédent tenu dans ce fichier :

1. **Il ASSIGNE, il n'imprime pas.** Même collision mika#2039 que
   `_pilot_log_dir`, `_pilot_max_turns` et `_plan_size_max_loc` : un accesseur
   imprimant se lirait `$(_plan_size_per_flight_bound)` et poserait
   `++ printf %s <valeur>` dans la trace `set -x`, forme qu'aucun scrubber ne
   couvre. Corollaire, assumé et testé : tout lecteur de
   `$_PLAN_SIZE_PER_FLIGHT_BOUND` appelle le résolveur **sur la même invocation
   logique**.
2. **Le plafond est PASSÉ, jamais lu par portée dynamique.** Exactement la
   contrainte que `_pilot_max_turns` s'est imposée pour `LABELS` : « un état posé
   500 lignes plus haut par une autre fonction peut être lu périmé sans que rien
   ne le dise ».
3. **Il ne relit jamais la table des labels.** mika#2542 AC5 est explicite :
   `PILOT_LABEL_TURN_CEILINGS` a **un seul lecteur**, « aucun autre lecteur de
   label ne décide d'un plafond ». Le résolveur de borne prend donc le plafond
   déjà résolu par `_pilot_max_turns`, en argument. Un second lecteur de labels
   serait la seconde table de classification que mika#2539 a nommée : juste le
   jour où on l'écrit, divergente en silence ensuite.
4. **Plafond vide ⇒ repli sur le défaut in-file (150), provenance dite.**
   `_PILOT_MAX_TURNS` est vide dans un seul cas : le ROLLBACK
   (`PILOT_MAX_TURNS=0` ou `""`), où claude-pilot retombe sur son `maxTurns=200`
   amont. Deux lectures étaient possibles et le choix est écrit : on **ne** prend
   **pas** 200, parce que ce 200 appartient à un composant qu'on ne possède pas
   et qu'un rollback est une posture d'incident ; on prend 150, la valeur que ce
   fichier possède, et qui est la direction prudente (borne plus serrée ⇒ plan
   plus petit ⇒ jamais une mort par volume). La provenance
   `disarmed_fallback` rend la décision visible au lieu de la taire.

**Plancher :** un plafond inférieur ou égal à
`orientation + fin_de_pipeline = 85` donne un nombre de tours de travail nul ou
négatif. La borne est alors **0**, jamais un négatif — « un nombre plausible,
présenté avec autorité, faux » est le défaut que l'ancre de
`_PLAN_SIZE_TOTAL_RE` a déjà dû fermer — et `_PLAN_SIZE_PER_FLIGHT_NOTE` porte
`ceiling_below_pipeline_floor`. La règle injectée dit alors au groomeur que le
plafond en vigueur ne peut héberger **aucune** implémentation et qu'il faut le
remonter à l'opérateur. Cas atteignable uniquement par un
`PILOT_MAX_TURNS=70` délibéré.

### L2 — `_PLAN_SIZE_FORMAT_SPEC` : deux lignes de déclaration, en FORME

Le format prescrit a **un site unique** depuis mika#2636, et il en a un parce que
sa première rédaction l'avait déjà fait diverger entre la règle du groomeur et le
finding synthétique du rattrapage. On étend donc la constante, pas ses deux
consommateurs :

```markdown
## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/foo.rs` | 120 |
| tests (`tests/eval/test_foo.rs`) | 95 |

Total estimé : 215 lignes

Plafond de tours : <plafond> (<label, ou « défaut »>)
Borne par vol : <borne> lignes estimées — (<plafond> − 35 − 50) × 7,5 / 2
```

Les chevrons sont **porteurs** : le spec est une FORME, et il est interpolé par
le finding de rattrapage, qui ne connaît pas le plafond du ticket. C'est la
règle (L3), qui le connaît, qui fournit les valeurs réelles juste après. Un spec
qui porterait `200` et `431` en dur prescrirait silencieusement la borne large à
tout ticket sans `loop-substrate`.

Les trois lignes d'exemple existantes et la ligne `Total estimé : 215 lignes`
sont **intouchées** : S13-bis les épingle, et AC4 exige que les tests de
mika#2636 passent sans modification.

### L3 — `_PLAN_SIZE_RULE_TEMPLATE` : la moitié par vol, avec les valeurs du ticket

Le gabarit gagne quatre marque-places substitués par `_plan_size_rule`, qui
prend désormais le plafond en argument :

| marque-place | valeur |
|---|---|
| `__PLAN_SIZE_MAX_LOC__` | inchangé — la borne par PR |
| `__PLAN_SIZE_CEILING__` | le plafond résolu (ou `150 (défaut, budget désarmé)`) |
| `__PLAN_SIZE_FLIGHT_BOUND__` | la borne par vol calculée |
| `__PLAN_SIZE_FLIGHT_DERIVATION__` | la dérivation, en clair, chiffres du ticket |

Le texte ajouté dit **cinq** choses, et cite son site de vérité (AC2) :

1. les deux bornes coexistent, la plus serrée mord ;
2. la borne par vol de **ce** ticket, avec son plafond et sa dérivation ;
3. ce que le plan doit écrire : les deux lignes de déclaration du § Taille
   estimée ;
4. au-delà : **découper en vols**, chacun mergeable seul, avec son ordre de
   commit et son plancher, la suite nommée par une ligne `Tracked in:` ;
5. en deçà : un seul vol, et le plan dit pourquoi.

Il nomme `_plan_size_per_flight_bound` dans
`skills/bundled/_shared/dispatch-lib.sh` comme site de vérité des paramètres —
ce que l'AC2 demande littéralement, et ce que le gate de mika#2636 fait déjà pour
son seuil.

**Pourquoi le prompt et pas `.claude/commands/mika-groom-plan-only.md` :** les
trois commandes de groom vivent dans `senara-solutions/mika-platform` et sont
semées dans le worktree par `_seed_worktree_slash_commands` (mika#1415). **Un
ticket ouvert sur `senara-solutions/mika` ne peut pas les éditer.** Ce prompt est
le seul canal que ce dépôt contrôle — raison déjà écrite par
`_FIRE_DISPOSITION_RULE` puis par `_PLAN_SIZE_RULE`.

Ce n'est pas l'enforcement de prompt que
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` condamne :
cette leçon porte sur une consigne qui dépend qu'un opérateur pense à la taper.
Une constante injectée par le substrat à chaque dispatch ne dépend d'aucune
mémoire — et la moitié structurelle est livrée à côté (les deux gates
architecte), ce que cette doctrine prescrit justement.

### L4 — Le site d'injection

Dans `_set_up_worktree`, la branche déjà gardée sur `SKILL = dev-groom` :

```sh
if [ "$SKILL" = "dev-groom" ]; then
    PROMPT=$(printf '%s\n\n%s' "$PROMPT" "$_FIRE_DISPOSITION_RULE")
    _pilot_max_turns "${LABELS:-}"; _plan_size_rule "$_PILOT_MAX_TURNS"
    PROMPT=$(printf '%s\n\n%s' "$PROMPT" "$_PLAN_SIZE_RULE")
fi
```

Quatre invariants préservés, chacun tenu par un test existant :

- la **condition sur le skill** reste écrite explicitement (contrôle négatif
  S13 / patron mika#2306 T3) ;
- la règle de taille reste la **dernière** règle du prompt (S13 : « la récence
  est le seul levier d'une règle en fin d'un prompt de 16 KiB ») ;
- la **première ligne** du `PROMPT` reste exactement `<repo>#<num>` (contrat
  mika#138, invariant 2 : sans lui, le dispatch tombe en mode free-text et aucun
  worktree n'est créé) ;
- `$_PILOT_MAX_TURNS` est lu sur la **même invocation logique** que
  `_pilot_max_turns` (scan de co-location mika#2496).

`_plan_size_rule` sans argument reste valide et résout le plafond par défaut : le
miroir `_t2636_inject` du harnais l'appelle ainsi, et AC4 exige qu'il continue de
passer sans modification.

### L5 — Les deux prompts architecte

Dans chacun, à l'intérieur de la section `### Plan-Size Gate (mika#2636)`
existante, un sous-bloc `#### Per-flight bound (mika#2693)`. Le niveau `####`
est délibéré : le fenêtrage de S14 est
`sed -n '/^### Plan-Size Gate/,/^### /p'`, et `#### ` n'apparie pas `^### ` — le
sous-bloc reste **dans** la fenêtre, donc dans la population que S14 lit déjà.

Chacun porte, en littéral :

```
**Per-flight parameters:** orientation 35 turns · pipeline tail 50 turns ·
throughput 750 lines per 100 turns · estimate→measured factor 200 %.
**Reference bounds:** 431 estimated lines at a 200-turn ceiling · 243 at a
150-turn ceiling.
```

puis l'arbre de décision (quatre branches, la quatrième étant l'échappatoire que
l'AC1 prescrit) :

1. le plan déclare son plafond et son total est **sous** la borne ⇒ la porte
   passe ;
2. le plan **ne déclare pas** de plafond ⇒ prendre **150** (fail-safe) et juger
   avec cette borne ; demander la déclaration dans le même finding ;
3. total **au-dessus** de la borne, **sans** découpage en vols dont la suite est
   nommée par une ligne `Tracked in:` ⇒ ITERATE (1re passe) / ESCALATE (2nde) ;
4. total au-dessus, **avec** un découpage en vols dont le **vol courant** repasse
   sous la borne et dont la suite est nommée par `Tracked in:` ⇒ la porte passe.
   **Juger le périmètre du vol courant, jamais la somme des vols** — même clause
   que la branche 4 du gate par PR.

Et la précédence, inchangée : la disposition la plus bloquante gagne, union des
F-findings. Les deux bornes sont **deux branches du même gate** ; la plus serrée
mord.

Le littéral `**Threshold:** 1000 lines` et les cinq branches existantes ne sont
pas touchés (AC4).

### L6 — Le harnais

Un bloc neuf dans `skills/bundled/_shared/test-dispatch-lib.sh`, après celui de
mika#2636, sondes `V1`–`V6`. Détail au § Contrat de vérification.

---

## Étapes d'implémentation

Ordre de commit, **et son plancher**. Le plancher est la ligne sous laquelle le
vol n'a rien livré : si les tours manquent, on s'arrête à une frontière de
commit, jamais au milieu d'une.

1. **`feat(groom): borne par vol — le site de vérité et son résolveur`**
   → L1. Les quatre constantes, le résolveur, le commentaire doctrinal.
2. **`test(groom): V1–V3 — la borne, ses paliers, sa co-location`**
   → les trois premières sondes. **PLANCHER** : en dessous, rien n'est livré, le
   résolveur serait du code mort non gardé.
3. **`feat(groom): la règle du groomeur porte la borne de SON ticket`**
   → L2 + L3 + L4 (spec, gabarit, site d'injection).
4. **`test(groom): V5 — l'injection, sa récence, sa conditionnalité`**
   → AC3 est déjà passé au commit 2–3 via V2. Deuxième frontière livrable : à ce
   point, porte 1 est complète et gardée.
5. **`feat(arch): le Plan-Size Gate juge aussi par vol`**
   → L5, les deux `system_prompt.md`.
6. **`test(groom): V4 + V6 — parité des prompts et anti-vacuité`**
   → AC2 y est. **Frontière de complétude des AC.** V4 est écrite **avant** le
   commit 5 dans l'ordre de travail (elle doit être vue rouge sur des prompts
   nus) et committée après, avec son correctif.
7. **`docs(solutions): la seconde borne du Plan-Size Gate est en TOURS`**
   → extension de la solution-doc existante : les trois morts du 2026-10-14, la
   provenance des quatre paramètres, la méthode de recalibrage. C'est le texte
   que le bloc L1 cite au lieu de le porter. Hors `docs/` : zéro ligne.

---

## Contrat de vérification

Chaque sonde nomme ce qu'elle a vu ROUGE avant le correctif — une sonde qui n'a
jamais été rouge ne mesure rien.

### V1 — le résolveur de borne, ses paliers

| entrée | borne | provenance | note |
|---|---|---|---|
| `200` | `431` | `resolved` | — |
| `150` | `243` | `resolved` | — |
| `120` | `131` | `resolved` | — |
| `""` (rollback) | `243` | `disarmed_fallback` | — |
| `85` | `0` | `resolved` | `ceiling_below_pipeline_floor` |
| `70` | `0` | `resolved` | `ceiling_below_pipeline_floor` |

Rouge avant : la fonction n'existe pas. La ligne `85` → `0` est le contrôle du
plancher : **jamais un négatif**.

### V2 — AC3, le même plan donne deux bornes selon le label

Sonde de bout en bout sur la **chaîne réelle** (`_pilot_max_turns` puis
`_plan_size_per_flight_bound`), pas sur le résolveur seul :

- `LABELS="p1-important,ready"` ⇒ plafond 150, borne **243** ;
- `LABELS="p1-important,ready,loop-substrate"` ⇒ plafond 200, borne **431** ;
- et la **règle injectée** porte `431` dans le second cas, `243` dans le
  premier — c'est la forme que le groomeur lit réellement.

Contrôle négatif porteur : `LABELS="not-loop-substrate"` ⇒ 243. L'appariement de
`_pilot_label_turn_ceiling` est exact, et un glob de sous-chaîne donnerait 431 —
c'est-à-dire la borne large sur un ticket qui n'y a pas droit.

Rouge avant : rien ne dérive de borne du plafond.

### V3 — co-location et non-impression (patron S11 / mika#2496)

- toute lecture de `$_PLAN_SIZE_PER_FLIGHT_BOUND` appelle le résolveur sur la
  même **invocation logique** (`_mika2496_logical_invocations`, réutilisé) ;
- contre-vacuité : au moins une lecture existe ;
- le résolveur ne porte ni `printf` ni `echo` en tête de ligne (mika#2039) ;
- aucune substitution `$(_plan_size_per_flight_bound)` ne subsiste ;
- le résolveur **ne nomme pas** `PILOT_LABEL_TURN_CEILINGS` — mika#2542 AC5, un
  seul lecteur de labels.

Rouge avant : s.o. (scan neuf) ; rouge **par mutation** : rendre le résolveur
imprimant, ou lui faire relire la table, fait tirer trois de ces cinq
assertions.

### V4 — AC2, la parité des deux prompts architecte (type S14)

Pour chacun des deux `system_prompt.md`, les quatre paramètres sont extraits par
`sed` ancré et comparés aux **constantes shell** ; puis les deux bornes de
référence du prompt sont comparées à ce que `_plan_size_per_flight_bound`
calcule pour 200 et 150.

C'est la moitié qui tient AC2 : sans elle, recalibrer le débit dans
`dispatch-lib` laisserait les deux prompts sur l'ancien et **la divergence serait
MUETTE** — exactement le défaut que S14 existe pour fermer sur le seuil par PR,
et que mika#2158 a mesuré sur `auto_pull.rs` (une regex commentée « Mirrors
GROOMED_VERDICT_RE » qui n'a suivi aucun des deux élargissements suivants).

Plus, sur chaque prompt : le sous-bloc existe, il nomme la ligne `Tracked in:`,
il nomme le repli à 150, et il cite `_plan_size_per_flight_bound`.
Et les verdicts restent ceux qui existent : ITERATE en 1re passe, ESCALATE en
2nde, et la 2nde **n'offre pas** d'ITERATE.

Rouge avant : les prompts ne portent aucun paramètre par vol.

### V5 — la règle injectée, à qui, et le format prescrit

- la règle porte les deux bornes, la dérivation, la ligne `Tracked in:` et le
  nom de son site de vérité ;
- aucun marque-place `__…__` ne subsiste après substitution ;
- `_PLAN_SIZE_FORMAT_SPEC` porte les deux lignes de déclaration **en forme** :
  chevrons présents, et **aucune** valeur en dur (ni `200`, ni `431`, ni `243`).
  C'est l'assertion qui empêche le finding de rattrapage — qui ne connaît pas le
  plafond du ticket — de prescrire la borne large à un ticket qui n'y a pas
  droit. Rouge **par mutation** : écrire `200` dans le spec la fait tirer ;
- elle est injectée pour `dev-groom` et **pas** pour `dev-pilot` (contrôle
  négatif porteur : le voisin `_PR_BODY_CONTAINMENT_RULE` est inconditionnel dans
  la même branche, donc la condition se **perd** en copiant le voisin) ;
- elle reste la **dernière** règle du prompt ;
- la première ligne du prompt reste exactement `mika#2693` ;
- `$_PILOT_MAX_TURNS` est co-localisé avec son résolveur sur le site d'injection.

Rouge avant : la règle ne porte pas de borne par vol ; rouge **par mutation** :
déplacer l'injection après le `fi` fait tirer l'assertion d'appartenance au bloc.

**Aucune sonde d'unicité du format prescrit n'est écrite** : **S13-bis la porte
déjà**. Elle asserte exactement **deux** interpolations de
`_PLAN_SIZE_FORMAT_SPEC` (la règle, et le finding synthétique du rattrapage) et
un seul site de définition. Étendre le *contenu* de la constante ne change ni
l'un ni l'autre, donc S13-bis reste verte **et reste la garde** — et elle doit
rester verte sans modification au titre de l'AC4. Écrire une seconde sonde
d'unicité serait la seconde table de classification de mika#2539 : juste le jour
de son écriture, divergente ensuite.

### V6 — anti-vacuité et herméticité (patron mika#2103 / mika#2205 / mika#2178 T9)

- le bloc de test a été extrait non vide, les deux prompts ont été **lus** (et
  non silencieusement absents) ;
- aucune invocation de forge (`gh`) dans la section ;
- la table d'exceptions des scans de source est déclarée et **assertée vide à
  l'exécution** (§ Fire-Disposition).

### Les gardes existantes, inchangées et vertes

- `make test-dispatch-lib` — **S1–S18 de mika#2636 passent sans modification**
  (AC4) ;
- `make check-pilot-turn-ceiling-labels` — la forme de fil de
  `PILOT_LABEL_TURN_CEILINGS` n'est pas touchée ;
- `make verify-bundled-skills` — structure des bundles ;
- `bash -n` sur les deux fichiers shell, `shellcheck` au niveau du dépôt ;
- `cargo test -p mika-common mika2398` (AC5) — **vert par construction** : ce
  plan ne touche aucune ligne de Rust, donc le recensement des régions
  `cfg(test)` du `ProductionScanner` est inchangé. Il est **exécuté quand même**,
  parce que l'AC le demande et qu'« inchangé » est une affirmation à attester,
  pas à supposer.

---

## Risques nommés

| # | risque | conduite |
|---|---|---|
| R1 | **Le plan déclare un plafond qui n'est pas celui du ticket.** L'architecte ne voit pas les labels (D2). | Non refermé, et dit. La sonde est la ligne de porte 3, renvoyée au vol 2 ; la fermeture demande de changer le contrat de `_arch_ask`. |
| R2 | **431 et 243 sont dérivés de paramètres posés sur n=3.** Le débit et le facteur dériveront. | C'est la raison d'être de la dérivation écrite : chaque paramètre est nommé, à un seul site, et la parité force les trois fichiers à bouger ensemble. Un nombre nu n'aurait pas été recalibrable. |
| R3 | **Deux bornes ⇒ un groomeur qui applique la mauvaise.** | La règle dit laquelle mord (la plus serrée) et donne les deux valeurs calculées. L'architecte juge les deux branches du même gate, avec précédence « la plus bloquante gagne ». |
| R4 | **Serrer le groom déplace la mort au lieu de la fermer :** des vols plus petits, donc plus de vols, donc plus de dispatches. | Coût assumé et nommé. Un vol de 400 lignes qui ouvre sa PR coûte moins qu'un vol de 900 qui meurt à 201 tours, se fait sauver en brouillon et finir à la main — c'est le calcul que les trois morts du 2026-10-14 ont déjà rendu. |
| R5 | **La règle du groomeur grossit un prompt déjà à 16 KiB.** | La moitié par vol est courte (~25 lignes) et reste la **dernière** chose que lit un groomeur, ce qui est le seul levier dont elle dispose (S13). |
| R6 | **Le repli à 150 sur budget désarmé sous-borne un vol qui disposait de 200 tours.** | Direction choisie et écrite (D1/L1 §4) : prudente, donc jamais une mort par volume. Visible par la provenance `disarmed_fallback`. |

---

## Hors périmètre

- **L'avis de tours restants envoyé au pilote en cours de vol** — ticket
  compagnon sur `claude-pilot`, comme le ticket le dit.
- **La forme de la fin de pipeline des pilotes** (garder `ce-simplify-code` et la
  revue par sous-agents alors que mika-qa relit au niveau du code) : décision en
  attente de l'opérateur. Elle changerait `_PLAN_SIZE_FLIGHT_TAIL_TURNS`, pas le
  mécanisme — c'est précisément pourquoi la valeur est une constante nommée.
- **Porte 3 — les champs par vol sur la ligne `plan_size_estimate`**
  (`per_flight_bound_loc=`, `ceiling=`, `ceiling_source=`,
  `per_flight_verdict=`), dérivés du plafond réellement passé en argv par
  `_emit_pilot_budget_line`. C'est la sonde qui recalibrera les quatre
  paramètres, et la seule qui verrait R1. **Vol 2.**
  → **Tracked in:** senara-solutions/mika#2693 (vol 2 — à ouvrir en ticket de
  suivi au merge de ce vol, intitulé « porte 3 : la ligne `plan_size_estimate`
  porte la borne par vol et le plafond réel »)
- **Un palier d'environnement par paramètre** (D3) : quatre résolveurs, quatre
  scans de co-location, quatre entrées dans `PILOT_DISPATCH_ENV` et dans la
  référence opérateur, pour un besoin que rien ne mesure.
- **Le rattrapage structurel des deux lignes de déclaration.**
  `_plan_size_retry_if_section_still_missing` continue de ne couvrir que
  l'**absence de section**. Une section présente mais sans déclaration de plafond
  est traitée par la branche 2 de l'arbre architecte (repli 150 + finding), pas
  par une seconde relance de pilote.
- **La borne par PR de mika#2636** : inchangée, défaut `1000`, palier
  `PLAN_SIZE_MAX_LOC` intact, tests intacts.

---

## Definition of Done

- [ ] `_plan_size_per_flight_bound` et ses quatre constantes existent à un seul
      site de `dispatch-lib.sh`, avec la dérivation et l'annulation des échelles
      en commentaire.
- [ ] La règle injectée au groomeur porte la borne de **son** ticket, sa
      dérivation, la conduite au-delà (vols + `Tracked in:`) et le nom de son
      site de vérité.
- [ ] Les deux `system_prompt.md` architecte portent le sous-bloc par vol, ses
      quatre paramètres en littéral, ses deux bornes de référence et son arbre à
      quatre branches.
- [ ] `V1`–`V6` passent, et chacune a été vue rouge (par absence ou par mutation
      documentée dans le commentaire de la sonde).
- [ ] `make test-dispatch-lib` vert, **S1–S18 de mika#2636 sans une ligne
      modifiée**.
- [ ] `make verify-bundled-skills`, `make check-pilot-turn-ceiling-labels`,
      `bash -n`, `shellcheck` propres.
- [ ] `cargo test -p mika-common mika2398` vert.
- [ ] La solution-doc porte la seconde borne et les trois morts du 2026-10-14.
- [ ] Le corps de PR porte les quatre décisions de grooming (D1–D4) et nomme le
      vol 2.

---

## Acceptance criteria

Transcrits verbatim de senara-solutions/mika#2693 :

- Un plan dont l'estimation dépasse la borne par vol, calculée pour le plafond de
  son ticket, rend `ITERATE`, sauf s'il porte un découpage en vols dont le vol
  courant repasse sous la borne et dont la suite est nommée par une ligne
  `Tracked in:`.
- La borne, ses paramètres et leur dérivation ont un seul site de vérité dans
  dispatch-lib. Le prompt du groomeur cite ce site, et un test de parité du type
  S14 tient les deux ensemble.
- Le même plan donne deux bornes différentes selon que le ticket porte
  `loop-substrate` ou non. Un test le montre.
- La borne par PR de mika#2636 reste inchangée, et ses tests passent sans
  modification.
- `cargo test -p mika-common mika2398` est vert.

**Correspondances :** AC1 → L5 (arbre, branches 3 et 4) + V4 ; AC2 → L1 + L3
(citation) + V4 ; AC3 → V2 ; AC4 → S1–S18 et S13-bis verts **sans une ligne
modifiée**, plus V5 (aucune valeur en dur dans le spec) ; AC5 → § Contrat de
vérification, gardes existantes.

---

## Fire-Disposition

Le plan livre deux détecteurs : `V3` (scan de co-location et de non-impression
sur la source de `dispatch-lib.sh`) et `V4` (scan de parité entre les constantes
shell et les deux prompts architecte statiques). Leur chemin de succès est
« aucune violation trouvée ».

**Option retenue : (a) exception nommée en allowlist — avec une allowlist
déclarée mais VIDE, les détecteurs livrés ARMÉS.**

Détail d'implémentation :

- **Population mesurée avant correctif : deux violations** — les deux prompts
  architecte, qui ne portent aujourd'hui aucun paramètre par vol (V4). Les deux
  sont **corrigées dans la même PR**, pas exceptées. V3 a une population vide dès
  le départ : son sujet est un résolveur que cette PR introduit. Après correctif
  la population totale est vide : il n'y a aucune violation à qui donner une
  entrée grep-visible.
- **L'allowlist est livrée VIDE plutôt qu'absente**, pour que l'ajout futur d'une
  exception soit un geste visible dans une table existante et non l'invention
  d'un mécanisme. Elle reprend à l'identique la forme que mika#2636 a posée pour
  ses trois scans de source :
  `T2693_SOURCE_SCAN_ALLOWLIST=()`.
- **Assertion auto-nettoyante :** la vacuité est assertée **à l'exécution**
  (`assert_eq … "0" "${#T2693_SOURCE_SCAN_ALLOWLIST[@]}"`), donc le test rougit
  **le jour où** une exception est ajoutée, pas seulement quand elle devient
  stale. Une table vide assertée vide est aussi ce qui distingue « aucune
  violation » de « le scan ne regarde rien ». Quand l'un de ces scans tire, on
  **arme le site** ; on n'ajoute pas de ligne ici (doctrine mika#2201).
- **Les détecteurs sont vus rouges avant d'être vus verts**, et c'est la
  condition d'armement : V4 est écrite et exécutée **avant** L5, sur l'arbre où
  les prompts ne portent rien — elle doit échouer en nommant le paramètre
  manquant. De même, V3 est vue rouge par mutation (résolveur rendu imprimant,
  ou relisant `PILOT_LABEL_TURN_CEILINGS`) et la mutation est consignée dans son
  commentaire.
- **Garde-fou anti-vacuité (V6)** : plancher de lisibilité sur les deux prompts
  et extraction non vide du bloc de test. Un scan qui ne trouve personne ne doit
  pas se lire comme un arbre propre — classe mika#2205, et c'est la forme que S14
  a déjà dû prendre (son bras `else` asserte `"lisible"` contre
  `"fichier introuvable: …"` plutôt que de passer en silence).

Les sondes comportementales V1, V2 et V5 ne sont pas des détecteurs au sens de
mika#2306 : leur sujet est le comportement d'un chemin d'exécution, pas la
recherche d'une violation dans le dépôt.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `dispatch-lib.sh` — constantes + `_plan_size_per_flight_bound` (code + pointeur de doctrine) | 70 |
| `dispatch-lib.sh` — `_PLAN_SIZE_FORMAT_SPEC` (deux lignes de déclaration, en forme) | 12 |
| `dispatch-lib.sh` — `_PLAN_SIZE_RULE_TEMPLATE` + `_plan_size_rule` | 45 |
| `dispatch-lib.sh` — site d'injection dans `_set_up_worktree` | 12 |
| `skills/bundled/mika-arch-groom-ticket/system_prompt.md` | 30 |
| `skills/bundled/mika-arch-second-review/system_prompt.md` | 28 |
| `test-dispatch-lib.sh` — V1–V2 (résolveur, ses paliers, deux bornes par label) | 60 |
| `test-dispatch-lib.sh` — V3 (co-location, non-impression, un seul lecteur de labels) | 35 |
| `test-dispatch-lib.sh` — V4 (parité des deux prompts) | 50 |
| `test-dispatch-lib.sh` — V5 (injection, récence, conditionnalité, spec en forme) | 30 |
| `test-dispatch-lib.sh` — V6 + allowlist vide + en-tête de bloc | 20 |

Total estimé : 392 lignes

Plafond de tours : 200 (loop-substrate)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

**Les deux bornes, et ce que ce plan en fait.**

- **Borne par PR (mika#2636) : 1 000.** 392 est très en dessous ; aucun découpage
  n'est requis à ce titre.
- **Borne par vol (ce ticket, appliqué à lui-même) : 431**, et ce plan est **un
  seul vol à 392**, soit 91 % de la borne. La fin de pipeline compte en entier
  (le vol tourne encore avec `ce-simplify-code` et `ce-code-review` ; mika#2694
  n'est pas mergé), donc il n'y a aucune marge à récupérer de ce côté : la marge
  de 39 lignes est la seule, et elle est mince. C'est assumé, parce que les deux
  retraits ci-dessous sont les seuls qui ne coupent aucune moitié du mécanisme —
  raboter une estimation pour atterrir plus bas serait exactement ce que la
  solution-doc de mika#2636 condamne (« une estimation produite sous la
  contrainte du nombre qu'elle doit battre est ancrée par lui »).

**Un seul vol, et voici pourquoi.** La rédaction initiale de ce plan annonçait
**517 lignes**, 20 % au-dessus de la borne. Trois retraits l'ont ramenée à 392, et
**aucun n'est un rabotage d'estimation** :

1. **La porte 3 sort du périmètre (−80).** Les champs par vol sur la ligne
   `plan_size_estimate` (`per_flight_bound_loc=`, `ceiling=`, `ceiling_source=`,
   `per_flight_verdict=`) ne sont réclamés par aucun AC, sont une **mesure** et
   jamais un refus, et emportent leurs deux sondes avec eux. C'est le seul
   livrable dont le retrait ne coupe aucune moitié d'un mécanisme de décision.
   → **Tracked in:** senara-solutions/mika#2693 (vol 2 — voir § Hors périmètre)
2. **La doctrine longue va dans `docs/` (−25).** Les trois morts du 2026-10-14, la
   provenance de chaque paramètre et la méthode de recalibrage vont dans la
   solution-doc existante, que le bloc L1 cite en six lignes. Le site de vérité
   garde la dérivation ; il perd sa justification, qui n'a jamais eu besoin d'être
   dans un `.sh`. Hors `docs/` : zéro ligne.
3. **La sonde d'unicité du format n'est pas écrite (−20).** S13-bis la porte déjà,
   et l'AC4 exige qu'elle reste verte sans modification : la dupliquer serait
   ajouter une seconde garde du même invariant, condamnée à diverger.

Ce qui reste couvre **les cinq critères d'acceptation en entier** : le site de
vérité, la règle du groomeur, les deux gates architecte, les six sondes. Un
découpage du reste couperait soit la règle du groomeur (porte 1), soit un des deux
prompts architecte (porte 2) — et la moitié restante serait une intention sans
refus, ou un refus sans intention. C'est pour cette raison que ce vol est unique :
non parce que 392 tient, mais parce que la prochaine coupe tombe au milieu d'un
mécanisme.

- **Vol 1 (cette PR) :** 392 lignes estimées. Mergeable seul : les deux portes
  qui décident sont complètes et gardées, et la borne par vol est en vigueur dès
  le merge.
- **Vol 2 :** porte 3, l'observabilité qui recalibrera les quatre paramètres.
  → **Tracked in:** senara-solutions/mika#2693 (vol 2, ticket de suivi à ouvrir
  au merge de ce vol — voir § Hors périmètre pour son intitulé et son contenu)

**Plancher du vol 1**, repris du § Étapes : commits 1–2 (le résolveur et ses
sondes V1–V3). En dessous, rien n'est livré. La frontière de complétude des AC
est le commit 6.

**Calibration citée**, telle que la consigne de groom la demande : mika#2687
vol 1, estimé 443, réel 994 (×2,24), dernier commit de travail au tour 136
sur 150 — c'est le point de données qui fixe le facteur prudent à 2.
