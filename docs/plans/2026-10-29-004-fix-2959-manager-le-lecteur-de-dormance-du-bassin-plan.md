# fix(manager) — le lecteur de dormance lit la déclaration, pas seulement son orthographe la plus décorée

**Ticket :** senara-solutions/mika#2959 (`bug`, `p1-important`, `dispatch:loop`, `loop-substrate`)
**Branche :** `fix/2959/manager-le-lecteur-de-dormance-du-bassin`
**Base mesurée :** `origin/main` à `8d127552e`
**Lié :** mika#2952 (le lecteur de dormance, vol 2/2 — le code que ce vol corrige), mika#2958 (le geste qui groomerait la relève), mika#2956 (même classe : un lecteur du bassin plus lâche ou plus étroit que la vérité qu'il lit), mika#2700, mika#2205 (deux populations sous un nom), mika#2309 (une garde par forme, jamais par allowlist de valeurs), mika#764 (frontière de caractère), mika#2340 (l'absence de mesure n'est pas une mesure)

---

## 1. Ce qui est mesuré

### 1.1 Le lecteur, tel qu'il est

`crates/mika-agent/src/milestone_manager/pool.rs:247-272` :

```rust
pub(crate) fn dormancy_reading(body: &str) -> DormancyReading {
    for ligne in body.lines() {
        let texte = ligne
            .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '>' | '-' | '*' | '_'));
        let ouvre_sur_le_marqueur = texte.chars().count() >= DORMANCY_MARKER.chars().count()
            && DORMANCY_MARKER
                .chars()
                .zip(texte.chars())
                .all(|(attendu, lu)| attendu.eq_ignore_ascii_case(&lu));
        if !ouvre_sur_le_marqueur {
            continue;
        }
        …
        return DormancyReading::Dormant { condition };
    }
    DormancyReading::Awake
}
```

Une seule porte : le marqueur **ouvre** la ligne, `#` n'étant pas une décoration retirée. Sa doc (l. 244-246) refuse le titre de section explicitement, sur une mesure faite sur #2472.

### 1.2 Le recensement complet des corps ouverts portant le marqueur, relu le 2026-10-29

`gh issue list --repo senara-solutions/mika --state open --search '"condition de réveil"'` rend **treize** tickets ouverts. Voici la ligne décisive de chacun, et ce que le lecteur actuel en fait. La colonne « texte » est la ligne après la normalisation de tête **actuelle** (espaces, `>`, `-`, `*`, `_`).

| ticket | ligne mesurée | texte normalisé (début) | lu aujourd'hui |
|---|---|---|---|
| #2854 | `> - **Condition de réveil :** quand …` | `Condition de réveil :** quand …` | **Dormant** |
| #2472 | `> **Condition de réveil :** feu vert …` | `Condition de réveil :** feu vert …` | **Dormant** |
| #2117 | `> **Condition de réveil du fil physique RT-005 (opérateur, 2026-08-31) :** aucun` | `Condition de réveil du fil …` | **Dormant** |
| #2912 | `**Condition de réveil : quand Vincent …**` | `Condition de réveil : quand …` | **Dormant** |
| #2913 | idem #2912 | idem | **Dormant** |
| #2592 | `## Condition de réveil (orchestrateur, 2026-10-17)` | `## Condition de réveil (…` | `Awake` |
| #2871 | `## Dormant : condition de réveil` | `## Dormant : condition de réveil` | `Awake` |
| #1761 | `> **Dormant, ouvert. Condition de réveil :** Vincent ou Prime …` | `Dormant, ouvert. Condition de réveil :** …` | `Awake` |
| #2262 | `> - **Dormeur visible** (règle ratifiée 2026-09-01) — **condition de réveil datée et concrète :** …` | `Dormeur visible** (règle …` | `Awake` |
| #1721 | `## Dormant, ouvert (2026-10-23) : condition de réveil` | `## Dormant, ouvert (2026-10-23) : …` | `Awake` |
| #2194 | `> **Dormant, ouvert (2026-10-19).** … Deux conditions de réveil, posées par …` | `Dormant, ouvert (2026-10-19).** …` | `Awake` par cette ligne |
| #1727 | *aucune ligne ne porte le marqueur avant le 2026-10-29* | — | `Awake` |
| #2959 | le corps **cite** les orthographes des autres, en prose et en table | `Le lecteur de dormance du bassin …` | `Awake` |

Deux mesures que le ticket ne nomme pas, et qui comptent :

- **#2262 et #1721 sont deux dormants supplémentaires** que le lecteur rate, sur deux orthographes de la même famille (« Dormeur visible … condition de réveil », « Dormant, ouvert (date) : condition de réveil »). Ils n'apparaissaient pas dans la relève parce qu'ils portent `operator-gated`/`blocked`, et que `OperatorHeld` mord **avant** la dormance (`pool.rs:961`, épinglé par `mika2952_un_ticket_tenu_et_dormant_lit_dabord_le_label`). Leur arbitrage ne change donc pas ; ce qui change, c'est que le motif cesse d'être faux en amont. La classe est plus large que les quatre tickets du rapport : **sept** corps ouverts sur treize sont mal lus.
- **#2194 est dormant par une autre ligne** (`> **Condition de réveil :** quand l'une des deux conditions …`, ajoutée le 2026-10-29) et sa première ligne porte le pluriel, `conditions de réveil` — qui ne contient **pas** la sous-chaîne `Condition de réveil`, le `s` s'insérant avant ` de`. Le pluriel n'est donc pas, et ne devient pas, une orthographe reconnue. Ce vol ne l'ajoute pas : aucun corps mesuré ne le porte **seul**.

### 1.3 Les deux faux positifs mesurés — ce qui interdit la correction naïve

Deux corps réels, l'un fermé l'autre ouvert, disent pourquoi « accepter les titres » et « accepter le marqueur n'importe où » sont tous deux faux.

**(a) L'historique d'un ticket réveillé — #2872 (fermé, réveillé le 2026-10-22), ligne 17 :**

```
## Réveillé le 2026-10-22 (ancienne condition de réveil)
```

Un lecteur qui accepterait tout titre de section contenant le marqueur lirait **réveillé** comme **dormant**. C'est le cas d'historique exact que l'AC2 demande, et il est mesuré, pas inventé.

**(b) Un ticket qui décrit la dormance d'un autre — #2959, ce ticket même.** Son corps porte en prose :

```
Le lecteur de dormance du bassin (`pool::dormancy_reading`, mika#2952 vol 2) ne reconnaît qu'une orthographe. La ligne doit **commencer** par `Condition de réveil`, une fois retirés …
```

et, en table, les quatre orthographes des autres tickets. Un lecteur qui accepterait le marqueur **n'importe où** sur la ligne lirait #2959 — `p1-important`, actionnable, et qui corrige précisément ce lecteur — comme dormant. C'est l'argument qui tranche la question que l'AC1 laisse ouverte sur le marqueur en milieu de ligne.

**(c) Le contrôle déjà épinglé — #2472, ligne 1 :**

```
## Pourquoi ce ticket est `blocked` — condition de réveil (2026-09-22)
```

Un titre qui *parle* de la condition. `mika2952_le_corpus_dormant_mesure_rend_dormant_avec_sa_condition` épingle `Awake` pour cette ligne seule (AC3). Ce vol ne doit pas le retourner.

### 1.4 Ce que #2506 (fermé) ajoute à la question du titre

```
### Condition de réveil (datée, concrète)
Réveiller (retirer DORMANT, groomer, poser `ready`) dès **l'un** de :
```

Ici le titre **est** la déclaration : la condition vit sur les lignes suivantes, sous une puce. C'est le corps qui interdit de refuser les titres en bloc. Et il dit aussi la limite de ce vol : la citation tirée d'un titre vaut la ligne du titre, pas la condition — point traité au § 2.3.

---

## 2. La décision : ce que le lecteur accepte, ce qu'il refuse, et pourquoi

Le critère est une **forme**, pas une liste de littéraux (mika#2309) : la ligne doit ouvrir sur une déclaration, et c'est tout ce qui est demandé.

### 2.1 Deux portes, une normalisation étendue d'un caractère

La normalisation de tête retire désormais **`#`** en plus de `>`, `-`, `*`, `_` et des espaces. Puis :

- **Porte 1 — le marqueur ouvre la ligne.** Comportement actuel, étendu aux titres par la seule addition de `#`. Couvre #2854, #2472 l.3, #2117, #2912, #2913 (inchangés), **et désormais** #2592 (`## Condition de réveil (orchestrateur, …)`) et #2506 (`### Condition de réveil (datée, concrète)`).
- **Porte 2 — la ligne ouvre sur un mot de dormance et porte le marqueur ailleurs.** Le texte normalisé commence par l'un des deux ouvreurs mesurés — `dormant`, `dormeur`, casse ignorée, en préfixe — **et** contient le marqueur. Couvre #2871 (`Dormant : condition de réveil`), #1761 (`Dormant, ouvert. Condition de réveil :`), #2262 (`Dormeur visible** … condition de réveil datée et concrète :`), #1721 (`Dormant, ouvert (2026-10-23) : condition de réveil`).

C'est l'ensemble : rien d'autre n'est accepté. En particulier, **le marqueur en milieu de ligne n'est pas accepté en soi** — il l'est seulement derrière un ouvreur de dormance. C'est l'arbitrage que l'AC1 demande, et le § 1.3(b) est sa raison : accepté en soi, il endort #2959 lui-même.

### 2.2 Ce que les deux portes refusent, vérifié ligne à ligne

| ligne refusée | pourquoi |
|---|---|
| `## Réveillé le 2026-10-22 (ancienne condition de réveil)` (#2872) | ouvre sur `Réveillé`, pas sur le marqueur ni sur un ouvreur de dormance |
| `## Pourquoi ce ticket est \`blocked\` — condition de réveil (2026-09-22)` (#2472) | ouvre sur `Pourquoi` — l'arbitrage de mika#2952 tient (AC3) |
| `Le lecteur de dormance du bassin … ne reconnaît qu'une orthographe` (#2959) | ouvre sur `Le` ; `dormance` en milieu de ligne n'est pas un ouvreur |
| `| #2592 | \`## Condition de réveil (orchestrateur…)\` | C'est un titre de section … |` (#2959) | `|` n'est pas une décoration retirée : le texte ouvre sur `| #2592 |` |
| `> **Enfant 8 de #2491** … **DORMANT** — voir Condition de réveil en bas.` (#2506 l.5) | ouvre sur `Enfant` ; #2506 reste dormant par sa l.31, qui est la déclaration |
| `> **Redémarrage : 2026-10-23 …** … C'est la condition de réveil.` (#2871 l.3) | ouvre sur `Redémarrage` ; #2871 reste dormant par sa l.11 et sa l.13 |

**Le coût accepté, nommé.** Une ligne qui ouvre sur `Dormant`/`Dormeur` **et** cite le marqueur est lue dormante, même si elle raconte un passé — par exemple « Dormant jusqu'au 2026-10-22, ce ticket a été réveillé ; sa condition de réveil était … ». Aucun des quinze corps mesurés (treize ouverts, #2872 et #2506 fermés) ne porte cette forme. Le coût est porté par l'allowlist auto-nettoyante du § Fire-Disposition, pas par une liste de mots interdits : une liste de mots de réveil (`Réveillé`, `ancienne`, `remplie`) serait exactement l'allowlist de valeurs que mika#2309 condamne, et la forme suffit ici.

### 2.3 La citation, et le rang qui la protège

La condition citée commence désormais **au marqueur**, jamais au début de la ligne. Pour la porte 1 cela ne change rien (le marqueur y ouvre la ligne) ; les assertions de mika#2952 restent littéralement vertes. Pour la porte 2 cela donne une citation serrée : #1761 cite `Condition de réveil :** Vincent ou Prime écrit …` au lieu de reprendre `Dormant, ouvert.`.

**Le rang, et les deux mesures qui l'exigent.** Plusieurs lignes d'un même corps peuvent correspondre, et la plus informative n'est pas la première. Deux cas réels :

- #2592 : le titre l.60 (`## Condition de réveil (orchestrateur, 2026-10-17)`) précède la déclaration l.62 (`> **Condition de réveil :** quand la fenêtre S2 de 30 jours est close, au plus tôt le 2026-11-01. …`).
- #2871 : le titre l.11 (`## Dormant : condition de réveil`) précède la déclaration l.13 (`> **Condition de réveil :** 7 jours après le redémarrage du moteur …`).

« Premier match gagne » rendrait donc, sur deux des quatre tickets du rapport, un refus qui dit « dormant » sans dire derrière quoi — l'AC du vol parent mika#2952 exige l'inverse (`le motif doit citer la condition`). Le lecteur retient donc **deux rangs**, en un seul passage :

- **rang 0 (déclaration)** : porte 1 satisfaite **et** la ligne n'est pas un titre ATX (elle ne commence pas par `#` avant normalisation). Retour immédiat.
- **rang 1 (titre, ou ouvreur de dormance)** : tout autre match. Mémorisé, rendu seulement si aucun rang 0 n'existe dans le corps.

Vérification sur le corpus : #2854, #2472, #2117, #2912, #2913, #1761 l.7, #2592 l.62, #2871 l.13, #2194 l.5 sortent en rang 0 avec leur condition entière ; #2506 (titre seul), #2262 et #1721 sortent en rang 1. Aucun corps mesuré ne perd sa citation.

La discrimination « titre ATX » du rang 0 est une heuristique de forme, et elle ne peut rien perdre : un titre riche tombe en rang 1, qui sort dès qu'il est seul.

### 2.4 Pas d'octet découpé

Le marqueur est localisé par un index **en caractères**, et la citation est construite par `chars().skip(i).take(DORMANCY_CONDITION_BUDGET)`. Aucun offset d'octet n'apparaît, donc aucune frontière de caractère à risquer (mika#764) et le budget reste exprimé dans l'unité où il est déclaré. L'helper de localisation sert aux deux portes **et** à la citation : un seul concept, un seul site.

---

## 3. Ce que le plan ne touche pas

- **`well_formed_candidates` et l'ordre des bras.** La précédence `OperatorHeld` → `Dormant` → `PriorPlanOnly` → `NoGroomedPlan` est inchangée. #2262 et #1721, nouvellement lus dormants, restent refusés par `OperatorHeld` : c'est l'arbitrage déjà épinglé.
- **`RefusalReason::Dormant`, son `label()` et son `remediable_by_grooming()`.** Inchangés — c'est la lecture qui était fausse, pas le motif.
- **`relief::name_relief`.** Elle filtre déjà sur `remediable_by_grooming()`, et `Dormant` y rend `false`. Corriger le lecteur suffit à vider la relève de ses dormants ; aucune ligne de `relief.rs` ne bouge.
- **`DORMANCY_MARKER` et `DORMANCY_CONDITION_BUDGET`.** Valeurs inchangées. Le pluriel `conditions de réveil` n'est pas ajouté (§ 1.2).
- **`reader.rs`, `types.rs`, `pool_writer.rs`.** Les deux sites d'appel de production (`reader.rs:649`, `reader.rs:988`) passent le corps entier et ne changent pas de signature.
- **`#1727`.** Il reste `Awake` tant qu'aucune ligne de son corps n'ouvre une déclaration ; le lecteur ne devine pas. La ligne ajoutée à son corps le 2026-10-29 (`> **Condition de réveil :** quand le launch a eu lieu. …`) est lue par la porte 1, qui existe déjà — ce n'est pas ce vol qui l'attrape, et c'est bien ce que l'AC1 demande.

---

## 4. AC3, par écrit

Les trois orthographes déjà acceptées le restent, et par la même porte 1 :

| ticket | orthographe | porte | rang |
|---|---|---|---|
| #2854 | `> - **Condition de réveil :**` | 1 | 0 |
| #2472 | `> **Condition de réveil :**` | 1 | 0 |
| #2117 | `> **Condition de réveil du fil physique … :**` | 1 | 0 |
| #2912 / #2913 | `**Condition de réveil : …**` | 1 | 0 |

La citation de chacune est **inchangée au caractère près**, puisqu'elle commençait déjà au marqueur de fait. Les assertions existantes de `mika2952_le_corpus_dormant_mesure_rend_dormant_avec_sa_condition` sont reprises telles quelles, sans réécriture de la valeur attendue — c'est la preuve d'AC3 la plus serrée disponible.

---

## 5. Livrables

### 5.1 `crates/mika-agent/src/milestone_manager/pool.rs` — la grammaire

**(a) `DORMANCY_OPENERS`**, nouvelle constante voisine de `DORMANCY_MARKER` : les deux ouvreurs de dormance mesurés, `["dormant", "dormeur"]`, en minuscules, testés en préfixe casse-ignorée. Sa doc nomme les quatre corps qui les portent (#2871, #1761, #2262, #1721) et dit pourquoi ce n'est pas une allowlist de littéraux au sens de mika#2309 : le prédicat est « la ligne ouvre sur une déclaration de dormance », et ces deux mots sont la racine française de cette déclaration, non une liste de valeurs à maintenir ticket par ticket.

**(b) `marker_char_position(texte: &str) -> Option<usize>`**, helper module-privé : l'index **en caractères** de la première occurrence de `DORMANCY_MARKER`, comparaison caractère par caractère en casse ASCII ignorée. Rend `Some(0)` quand le marqueur ouvre la ligne — ce qui fait de la porte 1 un simple `position == Some(0)` et évite un second prédicat redondant.

**(c) `dormancy_reading` réécrite.** Un passage sur les lignes, et pour chacune :

```
est_un_titre  := la ligne (trim_start des espaces) commence par '#'
texte         := ligne, décorations de tête retirées — espaces, '>', '-', '*', '_', '#'
position      := marker_char_position(texte)
ouvre_dormance := DORMANCY_OPENERS.iter().any(|o| texte ouvre sur o, casse ignorée)

porte 1 : position == Some(0)
porte 2 : ouvre_dormance && position.is_some()

rang 0  : porte 1 && !est_un_titre      → retour immédiat
rang 1  : (porte 1 || porte 2)          → mémorisé dans un Option<String>, premier gagnant
```

La citation se construit à partir de `position` : `texte.chars().skip(position).take(DORMANCY_CONDITION_BUDGET)`, `trim_end`, plus le `…` quand la coupe a mordu. Retour : rang 0 s'il existe, sinon le rang 1 mémorisé, sinon `Awake`.

**(d) La doc de la fonction et celle de `DORMANCY_MARKER`** sont réécrites : les sept orthographes acceptées, les trois refus mesurés avec leur ticket, l'arbitrage du marqueur en milieu de ligne et sa raison (#2959 lui-même), le rang et les deux mesures qui l'exigent, et l'extension de la normalisation à `#`. La phrase actuelle « un titre de section ne la satisfait pas » devient fausse et doit disparaître, remplacée par la distinction exacte : un titre **dont le texte ouvre sur le marqueur** la satisfait ; un titre qui le mentionne ailleurs, non.

### 5.2 `crates/mika-agent/src/milestone_manager/pool.rs` — les corps mesurés et les tests

**(a) Cinq constantes de corps**, toutes relues sur la forge le 2026-10-29, dans le style des trois existantes (`CORPS_DORMANT_2854` et sœurs), chacune documentée par son ticket et la ligne qu'elle porte :

- `CORPS_DORMANT_2592` — titre `## Condition de réveil (orchestrateur, 2026-10-17)` **suivi** de la déclaration l.62, parce que c'est la paire qui mesure le rang ;
- `CORPS_DORMANT_2871` — titre `## Dormant : condition de réveil` suivi de sa déclaration, même raison ;
- `CORPS_DORMANT_1761` — `> **Dormant, ouvert. Condition de réveil :** …`, le marqueur en milieu de ligne derrière un ouvreur ;
- `CORPS_DORMANT_2262` — `> - **Dormeur visible** (règle ratifiée 2026-09-01) — **condition de réveil datée et concrète :** …`, l'ouvreur `Dormeur` et le marqueur en minuscules ;
- `CORPS_DORMANT_2506` — `### Condition de réveil (datée, concrète)` **seul** (sa condition sur les lignes suivantes), le cas où le rang 1 doit sortir.

**(b) Deux constantes de corps pour le contrôle négatif** (AC2), mesurées elles aussi :

- `CORPS_REVEILLE_2872` — `## Réveillé le 2026-10-22 (ancienne condition de réveil)`, le cas d'historique ;
- `CORPS_CITE_LA_DORMANCE_DAUTRUI_2959` — un extrait du corps de ce ticket : la phrase de prose du § 1.3(b) **et** deux lignes de sa table, le cas de citation d'un autre ticket.

**(c) `mika2959_les_orthographes_mesurees_de_dormance_sont_lues`** — test paramétré, patron de `plan_reading` dans mika#2952 : une table `(ticket, corps, condition_attendue)` sur les cinq nouveaux corps, l'assertion portant sur `DormancyReading::Dormant { condition }` entier. Pour #2592 et #2871 la condition attendue est celle de la **déclaration**, jamais celle du titre : c'est ce qui épingle le rang, et sans cette valeur précise le test resterait vert sur un lecteur qui cite le titre.

**(d) `mika2959_une_citation_de_condition_nest_pas_une_declaration_de_dormance`** — le contrôle négatif de l'AC2, dans le même test que la moitié positive ne serait pas lisible, donc en test voisin et nommé : `CORPS_REVEILLE_2872` et `CORPS_CITE_LA_DORMANCE_DAUTRUI_2959` rendent `Awake`, et l'assertion dit en message ce qu'un faux positif coûte (un actionnable retiré du bassin sous un mauvais motif). Le test reprend aussi, en troisième cas, le titre de #2472 **seul** — l'assertion d'AC3 que mika#2952 avait posée, relue ici depuis la nouvelle grammaire.

**(e) `mika2952_le_corpus_dormant_mesure_rend_dormant_avec_sa_condition`** — conservé, valeurs attendues **inchangées** (AC3, § 4). Sa dernière assertion sur le budget et la coupe sur frontière de caractère est conservée telle quelle.

**(f) `MIKA2952_DORMANCY_EXCEPTIONS`** et son assertion auto-nettoyante — conservées, et sa doc mise à jour : le recensement du 2026-10-29 porte sur treize corps ouverts et deux fermés, et les sept orthographes qu'ils emploient sont toutes couvertes. L'allowlist reste **vide**, et le § Fire-Disposition dit sous quelle condition une entrée s'y écrit.

---

## 6. Ordre de commit

Un seul commit de travail. La grammaire et ses corps mesurés ne sont pas séparables : un commit qui livrerait la fonction sans les corps laisserait le dépôt avec une doc qui décrit sept orthographes et un test qui n'en mesure que trois ; l'inverse ne compilerait pas vert.

1. `fix(2959): le lecteur de dormance lit la déclaration, titre et ouvreur compris (mika#2959)` — §§ 5.1 et 5.2 ensemble.

---

## 7. Vérification

```
cargo test -p mika-agent milestone_manager::pool
cargo test -p mika-agent milestone_manager
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
scripts/check-byte-slices.sh
scripts/verify-pipeline.sh origin/main
```

`check-byte-slices.sh` est nommé explicitement : la citation touche à la découpe d'une chaîne, et le § 2.4 affirme qu'aucun offset d'octet n'y apparaît. Cette affirmation doit être vérifiée par la garde, pas par relecture.

---

## 8. Sonde après déploiement (AC4)

La correction n'atteint aucun agent avant `make deploy` (motif mika#2340 : le binaire servi est la seule chose qui lit).

1. `make deploy`.
2. Au **premier** rapport du sink postérieur au déploiement, ouvrir la section « Relève à grommer ».
3. Pour **chaque** ligne de cette section, relire le corps du ticket nommé : `gh issue view <n> --repo senara-solutions/mika --json body --jq '.body' | grep -n -i 'réveil\|dormant\|dormeur'`.
4. **Lecture attendue :** aucune ligne de la relève ne nomme un ticket dont le corps porte une déclaration de dormance au sens du § 2.1. Les quatre tickets du rapport sous `74e7254f` — #1727, #2592, #2871, #1761 — doivent avoir quitté la relève et figurer, s'ils apparaissent, sous un refus `dormant` citant leur condition.
5. **Lecture d'un échec :** une ligne de la relève dont le corps porte une déclaration signale une **septième** orthographe non mesurée. La remédiation est une entrée dans `MIKA2952_DORMANCY_EXCEPTIONS` nommant le corps, plus un ticket de suivi — jamais un élargissement improvisé du prédicat.
6. **Lecture d'un faux positif :** un ticket actionnable refusé en `dormant` dans la section des refus, dont le corps ne porte aucune déclaration. Même remédiation, sens inverse : l'entrée nomme le corps et le ticket de suivi borne la forme.

L'**absence** de la section « Relève à grommer » dans le rapport n'est pas une relève vide : c'est un rapport dont la section n'a pas été produite. Les deux se distinguent en relisant la section des refus du même rapport.

---

## Definition of Done

- [ ] `dormancy_reading` porte les deux portes du § 2.1, le rang du § 2.3, et la citation en caractères du § 2.4.
- [ ] `DORMANCY_OPENERS` et `marker_char_position` existent, documentés, chacun à un seul site.
- [ ] Les sept corps mesurés dormants rendent `Dormant` avec la condition attendue ; les trois corps de contrôle rendent `Awake`.
- [ ] Les valeurs attendues des trois orthographes de mika#2952 sont inchangées au caractère près.
- [ ] La doc de `dormancy_reading` ne contient plus l'affirmation « un titre de section ne la satisfait pas », remplacée par la distinction exacte.
- [ ] `MIKA2952_DORMANCY_EXCEPTIONS` reste vide, sa doc portant le recensement du 2026-10-29.
- [ ] Aucune ligne de `relief.rs`, `types.rs`, `pool_writer.rs` ni de `well_formed_candidates` modifiée.
- [ ] Les sept commandes du § 7 passent.
- [ ] La sonde du § 8 est écrite dans le corps de la PR, avec la commande de relecture par ticket.

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2959.

- **AC1.** Les orthographes mesurées ci-dessus sont lues comme dormantes. Le titre `## Condition de réveil …` et `## Dormant : condition de réveil` le sont, ou le plan dit par écrit pourquoi l'un d'eux ne doit pas l'être. Pour le marqueur en milieu de ligne (`Dormant, ouvert. Condition de réveil :`), le plan tranche. Le cas sans marqueur (#1727) reste `Awake`, puisque le lecteur ne devine pas.

  *Réponse du plan :* les deux titres sont lus dormants — le premier par la porte 1 (§ 2.1), le second par la porte 2. Le marqueur en milieu de ligne est **tranché** : accepté seulement derrière un ouvreur de dormance, refusé en soi, parce qu'accepté en soi il endort #2959 lui-même (§ 1.3(b)). #1727 reste `Awake` tant qu'aucune ligne de son corps n'ouvre une déclaration (§ 3).

- **AC2.** Contrôle négatif dans le même test : une ligne qui cite une condition de réveil sans être une déclaration de dormance (au moins un cas d'historique et un cas de citation d'un autre ticket) n'est pas lue comme dormante. Les corps de test viennent de corps mesurés, pas de littéraux inventés. Le patron est celui de `plan_reading` dans mika#2952.

  *Réponse du plan :* § 5.2(b) et 5.2(d) — historique mesuré sur #2872 l.17, citation mesurée sur le corps de #2959, patron paramétré de `plan_reading`.

- **AC3.** Les trois orthographes déjà acceptées le restent : #2854, #2472 et #2117 (doc de `pool.rs`).

  *Réponse du plan :* § 4, avec les valeurs attendues reprises sans réécriture.

- **AC4.** Sonde après déploiement : au premier rapport du sink, aucun ticket de la relève à grommer ne porte une condition de réveil dans son corps. La lecture se fait en relisant chaque ligne de la relève contre le corps du ticket.

  *Réponse du plan :* § 8, avec la commande de relecture par ticket et les deux lectures d'échec.

## Fire-Disposition

Ce plan livre des détecteurs : deux tests dont le chemin de succès est « aucune violation trouvée » (`mika2959_les_orthographes_mesurees_de_dormance_sont_lues`, `mika2959_une_citation_de_condition_nest_pas_une_declaration_de_dormance`) et l'assertion auto-nettoyante de `MIKA2952_DORMANCY_EXCEPTIONS`.

**Option retenue : (a) exception nommée en allowlist — et elle est livrée VIDE.**

`MIKA2952_DORMANCY_EXCEPTIONS`, posée vide par mika#2952, est conservée vide. Le recensement complet du § 1.2 en est la justification mesurée : treize corps ouverts portent le marqueur, plus deux fermés (#2872, #2506) retenus comme contrôles, et les sept orthographes qu'ils emploient sont toutes couvertes par les deux portes du § 2.1 — vérifié ligne à ligne aux §§ 1.2 et 2.2. Il n'existe donc **aucune** violation existante à exempter, et inventer une entrée pour satisfaire la forme serait écrire une exception morte à la naissance.

L'assertion auto-nettoyante existante tient le contrat : une entrée dont le corps rend désormais `Dormant` fait rougir le test, donc le geste qui rend l'exception inutile est le geste qui la fait expirer. Sur une allowlist vide la boucle tourne zéro fois ; le test existe pour le jour où la sonde du § 8 en fait écrire une ligne. La doc de la constante est mise à jour pour porter la date et l'étendue du recensement (`§ 5.2(f)`), sans quoi « vide » se lirait comme « jamais mesuré ».

Le format d'une entrée future est inchangé : le corps précis, la raison, et le ticket de suivi — les deux remédiations du § 8 points 5 et 6 nomment laquelle écrire selon le sens de l'erreur.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/pool.rs` — `DORMANCY_OPENERS`, `marker_char_position`, `dormancy_reading` réécrite, docs (§ 5.1) | 110 |
| `crates/mika-agent/src/milestone_manager/pool.rs` — corps mesurés et tests (§ 5.2) | 220 |

Total estimé : 330 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et pourquoi il tient.** 330 est sous la borne par vol (429) et très sous le seuil par PR (1000). Le périmètre est un seul fichier et une seule fonction : aucun appelant ne change de signature, aucun type ne bouge, aucune migration. Le volume est dominé par les tests et par la doc de la grammaire — le corps exécutable de `dormancy_reading` reste sous trente lignes. Découper en deux vols séparerait la grammaire de ses corps mesurés, ce que le § 6 refuse : le premier vol laisserait une doc qui décrit sept orthographes contre un test qui en mesure trois, et un lecteur élargi sans contrôle négatif est précisément le défaut que ce ticket corrige.

## Hors périmètre

- **Le pluriel `conditions de réveil`** (#2194 l.1, #2506 l.1). Aucun corps mesuré ne le porte **seul** : les deux tickets qui l'emploient portent aussi une déclaration au singulier. Ajouter une seconde valeur au marqueur sans population qui l'exige serait une largeur non mesurée.
- **Le geste d'écriture sur un dormant** — poser un label, retirer `ready`, commenter. `pool_writer` ne pose que `ready` et le rattachement au milestone (mika#2699), et la dormance est un refus, jamais une écriture. Rien ici ne change cela.
- **La relève à grommer et son geste de groom** (mika#2958). Ce vol corrige ce que la relève **lit** ; il ne touche ni son filtre (`remediable_by_grooming`) ni l'interrupteur `MIKA_MANAGER_GROOM_WRITE`.
- **Les tickets dormants qui ne portent aucun marqueur** (#1727 avant le 2026-10-29). Le lecteur ne devine pas, et l'AC1 le dit. La remédiation est une ligne écrite dans le corps par un humain, pas une heuristique.
- **L'idempotence inter-cycle et le rapport de ce qui a été posé** — vol 5 de mika#2952, suivi sur mika#2699.
