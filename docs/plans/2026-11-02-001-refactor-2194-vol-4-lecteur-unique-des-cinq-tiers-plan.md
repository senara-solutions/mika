# mika#2194 — vol 4 : le lecteur unique des cinq tiers, et la parité dorée *pre-switch*

Phase 3 du programme, ouverte dans mika#2194 même par le GO de Vincent du
2026-11-02 (relayé par samidarko). Ce plan borne **le premier des deux vols** de
cette phase. La bascule des deux enveloppes bash est le vol 5, nommé au § 6 et
suivi par la ligne `Tracked in:` du § *Taille estimée* — et ce découpage n'est
pas un accommodement de budget : c'est une **condition de validité de la preuve**
(§ 1, *Pourquoi la preuve et la bascule ne peuvent pas tenir dans le même vol*).

---

## 1. Où en est le programme, re-mesuré aujourd'hui

Toutes les valeurs ci-dessous sont mesurées sur le worktree du 2026-11-02, pas
reprises d'un plan antérieur.

| mesure | phase 1 (2026-09-30) | vol 3 (2026-10-19) | **aujourd'hui (2026-11-02)** |
|---|---|---|---|
| `dispatch-lib.sh`, octets | 551 330 | 741 823 | **819 220** |
| `dispatch-lib.sh`, lignes | 9 757 | 12 844 | **14 147** |
| `dispatch-lib.sh`, fonctions (`^_nom()`) | 119 | 146 | **158** |

Le fichier a encore crû de **~10 % en 14 jours**. La mesure du § 7 de la doctrine
(« le fichier grandit pendant que la migration avance ») n'est pas un accident de
fenêtre : elle se reproduit sur chaque intervalle mesuré.

État des phases, re-lu sur la doctrine (`docs/architecture/dispatch-lib-migration.md`
§ 6) :

- **Phase 1** livrée (#2607), **phase 2** livrée (#2608), **vol 3** livré (#2830 —
  corpus doré du tier 2 et `arch_paraphrase.rs` **sans appelant de production**).
- **Phase 3** : sa précondition — *« décider ce que devient le canal de retour par
  fichier `$_DISPOSITION_FUZZY_FILE` »* — est **remplie par le corps du ticket** :
  le fichier reste, et le shell l'écrit à partir de la sortie du moteur. C'est ce
  qui ouvre ce vol.
- **Phase 4** : précondition inchangée, non remplie.

### Le maillon, re-localisé à la ligne

| élément | lignes mesurées |
|---|---|
| `_DISPOSITION_FUZZY=0` / `_DISPOSITION_FUZZY_FILE=` | 9 787–9 788 (nettoyage au trap l. 3 372) |
| `_disposition_was_fuzzy()` | 9 790–9 794 |
| `_parse_disposition_fuzzy()` | 9 796–9 845 |
| `_parse_verdict_fuzzy()` | 9 847–9 883 |
| `_engine_escalation_line()` | 9 898–9 901 — **second appelant hors des deux parseurs** (l. 10 821/10 824), donc il ne part pas avec les fonctions fuzzy |
| `_parse_disposition()` | 9 903–9 995 — tier 0 l. 9 942–9 948, tier 0b l. 9 955–9 959, tier 1a l. 9 965–9 971, tier 1b l. 9 974–9 988, tier 2 l. 9 990–9 994 |
| `_parse_verdict()` | 9 997–10 088 — tier 0 l. 10 034–10 040, tier 0b l. 10 042–10 046, tier 1a l. 10 047–10 053, tier 1b l. 10 063–10 081, tier 2 l. 10 083–10 087 |
| les trois appelants du suffixe de trail | 11 288, 11 381, 11 525 |

**Une asymétrie mesurée entre les deux parseurs, à porter telle quelle.** Les
greps des tiers 1a et 1b de `_parse_disposition` portent `|| true` (son commentaire
l. 9 961–9 964 dit pourquoi : sous `errexit` + `pipefail`, un non-match tue
l'appelant quand la fonction tourne hors d'un `$(...)`). Les greps homologues de
`_parse_verdict` — l. 10 047–10 049 et 10 063–10 066 — **ne le portent pas**. Ce
n'est pas une différence de comportement observable aujourd'hui (les trois
appelants l'invoquent en substitution de commande), mais c'est une différence de
robustesse, et les deux enveloppes du vol 5 l'**effacent** en donnant une seule
forme aux deux. À dire dans le vol 5 plutôt qu'à découvrir.

### Pourquoi ce vol n'est pas la phase 3 entière

Le ticket décrit un geste en 5 points et six critères d'acceptation. Mesuré
livrable par livrable (§ *Taille estimée*), le geste entier vaut **≈ 555 lignes de
code hors `docs/`**, soit **deux fois** la borne par vol de 279. La règle de
grooming prescrit alors un découpage en vols mergeables seuls. Ce vol prend la
moitié « lecteur + preuve » ; le vol 5 prend la moitié « canal + bascule ».

### Pourquoi la preuve et la bascule ne peuvent pas tenir dans le même vol

C'est l'argument décisif, et il n'est pas budgétaire. La doctrine § 4 l'écrit
déjà, et le commentaire d'en-tête du lecteur bash de vol 3
(`test-dispatch-lib.sh:14944`) le répète mot pour mot :

> c'est la seule fenêtre où la parité est une preuve — après la bascule du vol
> suivant, le bash appellera le Rust, et un lecteur comparé à lui-même est
> toujours d'accord avec lui-même.

Une PR est jugée sur **l'état final de son arbre**. Si la bascule et la parité
tenaient dans la même PR, le test de parité, à l'état final, comparerait
`_parse_disposition` (devenue une enveloppe sur la sous-commande) à
`arch_disposition::decide` — c'est-à-dire `decide` à lui-même. Il serait **vert
et vide**, et l'AC1 serait satisfaite par une tautologie. La preuve exige donc un
commit *pre-switch* **mergé**, et c'est ce vol.

Corollaire à écrire dans le vol 5 : la même exécution du même test y est une
**non-régression**, jamais une preuve. Son en-tête devra le dire, sans quoi un
futur lecteur croira la parité toujours vérifiée.

---

## 2. Le maillon, confronté aux trois critères de sélection

Le maillon de ce vol est **l'expression de décision des cinq tiers**, pas les
fonctions `_parse_disposition` / `_parse_verdict` (doctrine § 6 : « le maillon est
l'EXPRESSION, jamais la fonction »).

### C1 — Pureté

**Satisfait pour les cinq tiers, et c'est mesurable ligne à ligne.** Chacun des
cinq tiers est `texte → (jeton, tier, fragment)` : un `case` de motif (tier 0), un
`grep -m1 -E` ancré (tier 0b), deux `grep -oE | head -1` (tiers 1a et 1b), deux
boucles `grep -oi` sur table (tier 2). Aucun n'ouvre de fichier, n'appelle `git`,
`gh` ou `bwrap`, ne lit l'environnement.

**L'impureté du maillon courant est HORS du maillon**, et c'est exactement ce que
la décision du ticket exploite : les deux écritures `printf '0'`/`printf '1' >
"$_DISPOSITION_FUZZY_FILE"` (l. 9 922, 9 992, 10 014, 10 085) sont de la glue de
franchissement de `$(...)`, pas une décision. Elles **restent au shell** — donc
le maillon migré est pur, et le maillon non migré reste au seul endroit qui le
connaît (C3, lu dans l'autre sens).

### C2 — Un seul appelant

**Satisfait, et pour la même raison qu'en phase 2.** Lu sur les fonctions, C2
écarterait le maillon : `_parse_disposition` et `_parse_verdict` ont trois
appelants de production (l. 11 286, 11 379, 11 523) et de l'ordre de la dizaine
d'appels de test. Lu sur l'expression, chaque tier a **un** appelant : son propre
parseur. Et la doctrine § 6 a déjà tranché le reste : *« C2 ne contraint pas le
fan-in d'un site de délégation déjà migré »* — chaque appelant de plus d'un
lecteur unique est une copie de moins.

### C3 — Le Rust doit déjà être, ou devoir être, un lecteur

**Satisfait deux fois, et l'un des deux est contraignant.**

- Le tier 2 **est déjà** en Rust : `arch_paraphrase::paraphrase` porte les deux
  tables depuis le vol 3, avec son corpus doré et **aucun appelant de
  production**. Ce vol lui en donne un (interne : `arch_disposition::decide`).
- Le tier 0 **doit** être en Rust : son littéral est
  `DISPOSITION_WITHHELD_MARKER` (`crates/mika-agent/src/agent_loop/mod.rs:9698`),
  que **le moteur écrit**. Le bash en porte aujourd'hui une copie, et
  `test-dispatch-lib.sh:6439-6449` existe précisément pour détecter la dérive
  entre les deux. Un tier 0 qui *lit* la constante au lieu d'en recopier le
  littéral supprime la classe de panne entière, et c'est le seul tier dont on
  puisse dire ça.

### Le critère de canal (§ 2 de la doctrine) : la fréquence d'appel décide

**La fréquence est de deux appels par ticket groomé** — un par passe
d'architecte — et la doctrine admet un démarrage de `mika` à cette fréquence
(c'est celle de `plan-callout`). Ce vol **n'ajoute aucun démarrage** : il ne
livre pas de canal. Le vol 5 en ajoute deux par groom, et devra le dire. La sonde
S3 reste hors tranche, avec son remède écrit par le ticket : au-delà d'environ
500 ms, le remède est **le placement dans `main.rs`** (le chemin court de tête, à
côté de `Token` et `CredentialHelper`), **jamais un cache**.

---

## 3. Les trois divergences mesurées entre le texte du ticket et le dépôt

Elles ne sont pas des erreurs du ticket : ce sont des mesures qui ont bougé. Les
rejouer telles quelles coûterait un vol.

### D1 — `scripts/canonical-tokens.tsv` : ce qui part n'est pas ce que le ticket nomme

Le ticket écrit que le TSV « perd ses lignes 85, 86 et 185 ». Mesuré aujourd'hui :

| lignes | contenu | sort réel |
|---|---|---|
| 85–86 | `Disposition:` / `Verdict:` → `::_parse_disposition_fuzzy` / `::_parse_verdict_fuzzy`, classe `paraphrase` | **partent au vol 5**, avec les deux fonctions |
| 185–188 | quatre jetons → `::_parse_disposition` | **survivent** : la fonction devient une enveloppe, pas un disparu |
| 189–192 | quatre jetons → `::_parse_verdict` | **survivent**, idem |
| 196–199 | bloc de prose nommant les deux fonctions fuzzy comme duplication *temporaire et déclarée* | **à réécrire au vol 5** : la duplication cesse |
| 214–217 | quatre jetons → `arch_paraphrase.rs::paraphrase` | **changent de statut** : le site cesse d'être sans appelant |
| 367 | `REVIEW-ANCHOR-MISSING` → `::DISPOSITION_WITHHELD_MARKER`, classe B | inchangée |

Retirer « la ligne 185 » seule laisserait 186–188 orphelines. **Et ce vol
*ajoute* au TSV** plutôt qu'il n'en retire : `arch_disposition.rs::decide`
portera les littéraux `READY` / `ITERATE` / `ESCALATE` / `GROOMED` ainsi que
`Disposition:` et `Verdict:`, donc un **troisième** site de match que
`canonical-tokens-survey.sh --check` exige déclaré. Non déclaré, la garde
mika#2201 rougit — c'est un livrable de ce vol, pas du suivant.

`scripts/canonical-tokens-survey.sh:41` nomme aussi `_parse_disposition_fuzzy`,
mais **dans un commentaire** : ce n'est pas un lecteur, et le vol 5 n'a qu'à le
mettre à jour.

### D2 — « les dix appels directs » : dix occurrences, deux appels

Le ticket parle de « dix appels directs de `_parse_disposition_fuzzy` et
`_parse_verdict_fuzzy` dans les tests (mesurés le 2026-11-02) ». Recomptés par
nature :

| nature | nombre | sites |
|---|---|---|
| **appels réels** | **2** | `test-dispatch-lib.sh:15011`, `:15014` — le lecteur de corpus du vol 3 |
| gardes d'existence `declare -f` | 4 | la sentinelle de sourcing et la garde de boucle |
| commentaires | 4 | dont `tests/test_parse_disposition.sh:251` |

Conséquence pour l'AC2, à écrire : « chacun devient un cas du corpus doré » porte
sur **deux** sites d'appel, pas sur dix cas indépendants ; et les quatre gardes
`declare -f` doivent être **re-pointées** sur les enveloppes (vol 5), pas
supprimées — ce sont elles qui empêchent le test d'être vert sur un
`dispatch-lib.sh` qui ne se source plus (`source "$DISPATCH_LIB" 2>/dev/null ||
true`, l. 720, **avale** son échec).

Bonne nouvelle mesurée : `skills/bundled/_shared/tests/test_parse_disposition.sh`
(361 lignes) appelle **exclusivement les enveloppes** — jamais les fonctions
fuzzy. L'« attentes inchangées » de l'AC2 est donc structurellement atteignable
pour ce fichier.

### D3 — la garde de dérive du marqueur meurt de la bascule, et doit s'INVERSER

Le ticket admet dans `agent_loop/mod.rs` un changement d'environ 11 octets :
`const` → `pub(crate) const`. Deux mesures sur cette ligne.

1. **La regex de la garde survit à `pub(crate)`.** `test-dispatch-lib.sh:6439`
   fait `grep -oE 'const DISPOSITION_WITHHELD_MARKER: &str = "[^"]+"'`, non
   ancré : `pub(crate) const …` contient cette sous-chaîne. Rien à faire.
2. **Mais la garde entière devient fausse au vol 5**, et pas pour cette
   raison-là. Les lignes 6 445–6 449 exigent que `dispatch-lib.sh` **porte le
   littéral** — et l'AC4 le retire. La prémisse de la garde (*« le littéral
   existe dans les deux langues »*) meurt avec la bascule. Elle doit donc
   **s'inverser** plutôt que disparaître, exactement comme la phase 2 a inversé
   les deux assertions de la phase 1 (`test-dispatch-lib.sh:11186-11198`
   l'écrit) : *le littéral n'est plus dans le bash*, **et** *la constante Rust a
   un lecteur unique*. Supprimer la garde rendrait vert un dépôt où le tier 0
   aurait été recopié ailleurs.

La marge de `agent_loop/mod.rs` est re-mesurée aujourd'hui : **1 046 924 octets
sur 1 048 576**, soit **1 652 octets** (`crates/mika-common/tests/claude_md_size.rs`
et `scripts/check-secrets.sh` plafonnent à 1 Mio). Les ~11 octets de
`pub(crate) ` tiennent. **Tout autre ajout à ce fichier est hors tranche** : s'il
devient nécessaire, c'est une halte, pas un arbitrage de marge.

---

## 4. Ce que le corpus va mesurer

Le corpus existant (`crates/mika-agent/tests/fixtures/arch_paraphrase_texts/`,
17 lignes d'attendus sur 16 corps) ne couvre que le **tier 2**. Ce vol l'étend
aux tiers 0, 0b, 1a et 1b, chacun avec au moins un positif et un négatif, plus
les deux cas que le ticket nomme explicitement.

### D1 — le marqueur cité au milieu d'une ligne ne supprime pas la disposition

C'est le cas de bord que le commentaire bash l. 9 937–9 939 décrit : le `case`
n'a **que deux bras** (début de texte, ou précédé d'un `\n`), donc un marqueur
cité en milieu de ligne ne déclenche pas le tier 0. Un portage Rust par
`text.contains(MARKER)` — la forme la plus naturelle — le ferait déclencher et
**supprimerait un ITERATE légitime**. Une ligne négative de classe
`tier-shadowed` l'épingle.

### D2 — un `READY` cité en ligne, avant une escalade du moteur : l'escalade gagne

C'est la raison d'être de l'ordre tier 0b **avant** tier 1a (commentaire
l. 9 950–9 954) : le tier 1a grepe le *premier* `Disposition:` **n'importe où**,
non ancré, donc un `READY` que le modèle a cité serait lu avant l'`ESCALATE` que
le moteur a ajouté à la fin. Une ligne positive tier 0b sur un corps qui porte
les deux l'épingle. Un portage qui réordonne les tiers rougit.

### D3 — le tier 1b de seconde passe n'a pas de bras `ITERATE`, délibérément

`_parse_verdict` n'aliase pas `Disposition: ITERATE` (commentaire l. 10 078–10 080 :
la spec interdit une troisième passe). Un corps portant `Disposition: ITERATE`
lu en **seconde** passe doit donc **tomber au tier 2**, pas rendre `ITERATE`.
C'est le négatif du tier 1b, et la forme d'erreur qu'un portage « symétrique et
serviable » produirait.

### D4 — le tier 1a est insensible à la langue, le tier 2 ne l'est pas

La classe `french-inert` du vol 3 mesure que le tier 2 est **inerte** sur les
réponses françaises (ses deux tables sont en anglais), ce qui est le cas **modal**
en production. Le tier 1a, lui, matche `Disposition:` littéral, donc une réponse
française *avec* sa ligne canonique parse au tier 1a. Les nouvelles lignes ne
doivent pas revendiquer une inertie française sur le tier 1a, et la classe
`french-inert` reste bornée au tier 2. B1 interdit d'y toucher : le tier 2
**reste anglais et reste inerte**, et le corriger serait un autre vol.

### D5 — le fragment : qui l'asserte, et qui ne peut pas

Le tier 2 écrit son fragment décisif sur stderr sous une forme lue par les deux
lecteurs (`(matched: '…')`). Les tiers 0 à 1b écrivent une **prose différente par
tier** (« tier 0 — disposition withheld… », « tier 1b accepted Verdict: GROOMED
→ READY… ») et pas de fragment sous cette forme. La colonne `fragment` des
nouvelles lignes vaut donc `-`, et le lecteur bash **saute** la comparaison de
fragment quand le tier n'est pas `2`. Dit ici plutôt que découvert par un
lecteur qui croirait la colonne cassée.

---

## 5. Requirements

### R1 — `crates/mika-agent/src/arch_disposition.rs` : les tiers 0 à 1b et l'entrée unique

Un **module voisin**, pas une extension de `arch_paraphrase.rs` : B1 interdit de
toucher aux deux tables du tier 2, et les garder dans un fichier que ce vol
n'édite pas est la façon la moins ambiguë de le tenir. `arch_paraphrase` reste
le tier 2 ; `arch_disposition` porte les quatre autres tiers et l'entrée.

Surface publique :

```rust
pub enum ArchPass { First, Second }          // ré-exporté de arch_paraphrase

pub enum Tier { Withheld, EngineEscalation, Literal, CarryOver, Paraphrase, None }
impl Tier { pub fn as_str(&self) -> &'static str }  // "0" | "0b" | "1a" | "1b" | "2" | "-"

pub struct Decision<'a> {
    pub token: Option<&'static str>,   // READY | ITERATE | ESCALATE | GROOMED
    pub tier: Tier,
    pub fragment: Option<&'a str>,     // tier 2 uniquement, et c'est une TRANCHE de l'entrée
}

pub fn decide(text: &str, pass: ArchPass) -> Decision<'_>;
```

Contraintes de portage, chacune mesurée sur le bash :

1. **L'ordre des tiers est celui du bash** : 0, 0b, 1a, 1b, 2. L'ordre est
   porteur (§ 4 D2), pas stylistique.
2. **Tier 0 LIT la constante** : `agent_loop::DISPOSITION_WITHHELD_MARKER` passée
   en `pub(crate)`. Aucune recopie du littéral. Le prédicat est « en tête de
   ligne », soit exactement les deux bras du `case` : `text` commence par le
   marqueur, ou contient `"\n" + marqueur`.
3. **Tier 0 rend `token: None` avec `tier: Withheld`** — pas `tier: None`. Le
   bash y fait `return` sans rien émettre, et la distinction « le moteur a retiré
   la disposition » / « rien n'a matché » est exactement celle que le canal du
   vol 5 doit porter sur sa ligne 2 (classe mika#2205 : deux populations sous un
   nom ne sont pas soustractibles).
4. **Tier 0b reproduit l'ancrage en entier** : `^[[:space:]]*F[0-9]+:
   \(BLOCKING\) \[mika-engine\] review-anchor:`. Le commentaire bash
   l. 9 891–9 897 dit pourquoi `[mika-engine]` seul n'est **pas** le signal : tout
   re-prompt correctif commence par là, le modèle le relaie en session et les
   prompts d'architecte l'enseignent.
5. **Tiers 1a/1b reproduisent la sémantique de `grep -oE … | head -1`** :
   première **ligne** portant un match, premier match **sur cette ligne**. Pas
   « premier match du document » si le portage scanne autrement.
6. **Le pliage de casse reste ASCII** (`to_ascii_lowercase`) partout où le tier 2
   en fait, parce que la longueur en octets doit être préservée pour le
   tranchage du fragment — propriété déjà tenue par `arch_paraphrase`, à ne pas
   défaire par le point d'entrée.
7. **Le fragment est une tranche de l'entrée, jamais du motif.** Propriété déjà
   prouvée structurellement côté parité par un contrôle d'adresse de pointeur
   (`as_ptr().addr()`), à préserver à travers `Decision`.

### R2 — `agent_loop/mod.rs` : `pub(crate)`, et rien d'autre

Un seul changement : `const DISPOSITION_WITHHELD_MARKER` → `pub(crate) const
DISPOSITION_WITHHELD_MARKER`. ≈ 11 octets sur 1 652 de marge. La forme de la
ligne est préservée, donc la garde `test-dispatch-lib.sh:6439` continue de
matcher (§ 3 D3 point 1). Si un autre ajout à ce fichier devient nécessaire :
**halte**, ce n'est pas un arbitrage de marge.

### R3 — le corpus doré étendu aux cinq tiers

`crates/mika-agent/tests/fixtures/arch_paraphrase_texts/` gagne **huit corps** et
**douze lignes d'attendus**. Les corps couvrent, au minimum :

| tier | positif | négatif |
|---|---|---|
| 0 | le marqueur **en tête de ligne**, seul → aucun jeton, tier `0` | le marqueur **cité en milieu de ligne**, plus un `Disposition: ITERATE` → `ITERATE` au tier `1a` (§ 4 D1) |
| 0b | la ligne d'escalade du moteur **après** un `Disposition: READY` cité → `ESCALATE` au tier `0b` (§ 4 D2) | un corps qui cite `[mika-engine]` en prose sans la forme complète → le tier 0b ne tire pas |
| 1a | `Disposition: READY` (1re passe), `Verdict: GROOMED` (2de) | `Disposition: undefined` → le tier 1a ne tire pas (le cas de `tests/test_parse_disposition.sh` l. 71–106 porté au corpus) |
| 1b | `Verdict: GROOMED` en **1re** passe → `READY` ; `Disposition: READY` en **2de** passe → `GROOMED` | `Disposition: ITERATE` en **2de** passe → **tombe au tier 2** (§ 4 D3) |

Le fichier d'attendus gagne une **sixième colonne**, `tier`. La règle fondatrice
du fichier s'y applique sans exception : **une colonne vide porte `-`, jamais
rien** — `IFS=$'\t' read` fusionne deux tabulations consécutives et décale en
silence toutes les colonnes suivantes, là où le `split('\t')` du Rust ne le fait
pas. L'en-tête du TSV documente la nouvelle colonne et les deux nouvelles classes
(R5).

### R4 — le lecteur Rust : la colonne `tier`, et le routage vers `decide`

`crates/mika-agent/tests/arch_paraphrase_parity.rs` :

- lit la sixième colonne et la valide contre un vocabulaire **fermé**
  (`0` | `0b` | `1a` | `1b` | `2` | `-`), comme `PASS_VALUES` / `TOKEN_VALUES` /
  `CLASS_VALUES` le sont déjà ;
- route **tout** le corpus vers `arch_disposition::decide` (et non plus
  `arch_paraphrase::paraphrase`) : c'est l'entrée unique, et un corpus qui
  contourne l'entrée ne mesure pas ce que le canal appellera ;
- asserte le **jeton**, le **tier** et, quand `tier = 2`, le **fragment** avec sa
  preuve de tranche ;
- conserve ses six tests et ses trois termes d'anti-vacuité, et **relève son
  plancher** : `cases >= 15` devient `cases >= 27` (17 + 12 − 2 lignes réécrites),
  faute de quoi un TSV tronqué passerait sous l'ancien plancher.

### R5 — le lecteur bash : la sixième colonne, les enveloppes, la sentinelle *pre-switch*

`skills/bundled/_shared/test-dispatch-lib.sh`, bloc l. 14 944–15 055 :

- **lit six colonnes** (sans quoi `class` recevrait `class<TAB>tier` et la
  vérification de classe serait silencieusement fausse) ;
- **route vers les enveloppes** `_parse_disposition` / `_parse_verdict` au lieu
  de `_parse_disposition_fuzzy` / `_parse_verdict_fuzzy`. C'est le changement
  substantiel : ce sont les enveloppes qui portent les cinq tiers, et c'est leur
  sortie que la sous-commande du vol 5 devra reproduire. Les deux appels aux
  fonctions fuzzy (D2 : il n'y en a que deux) disparaissent **ici**, avec leurs
  attentes conservées ;
- **n'asserte PAS la colonne `tier`**, et le dit : *pre-switch*, le bash n'a
  aucun canal de sortie de tier — sa prose de stderr diffère par tier et n'est
  pas un contrat. La colonne `tier` est assertée par le lecteur Rust seul ; le
  bash asserte le **jeton**, et le fragment quand `tier = 2` ;
- **saute la comparaison de fragment quand `tier ≠ 2`** (§ 4 D5) ;
- conserve ses quatre termes d'anti-vacuité, dont le quatrième propre au bash —
  la **sentinelle de sourcing** — en la re-pointant sur les enveloppes :
  `declare -f _parse_disposition` et `declare -f _parse_verdict`. La ligne 720
  fait `source "$DISPATCH_LIB" 2>/dev/null || true` et **avale** son échec : sans
  la sentinelle, un `dispatch-lib.sh` qui ne se source plus rendrait tout le
  corpus vert sur rien. C'est le terme que l'AC1 exige nommément ;
- **ajoute la sentinelle *pre-switch*** : les enveloppes **ne doivent pas encore**
  appeler la sous-commande. Elle rougit le jour où le vol 5 bascule, et c'est ce
  qui force la mise à jour de l'en-tête (« ceci est une non-régression, plus une
  preuve ») au lieu de laisser une tautologie verte. Voir § *Fire-Disposition*.

**Rien à ajouter à `scripts/suite-wiring.tsv`** : ce fichier est atteint par
`make test-dispatch-lib`, que `ci.yml:85` nomme. Le `suite-wiring-lint` est
bidirectionnel — une entrée pour une suite que la CI atteint le fait **échouer**.

### R6 — `scripts/canonical-tokens.tsv` : le troisième site, déclaré

`arch_disposition.rs::decide` porte les littéraux `READY`, `ITERATE`,
`ESCALATE`, `GROOMED` (tiers 1a/1b) ainsi que `Disposition:` et `Verdict:`.
Quatre lignes de jeton classe A + deux lignes de marqueur, plus une ligne de
prose disant **pourquoi** trois sites coexistent et lequel est transitoire. Non
déclaré, `canonical-tokens-survey.sh --check` rougit — et le ticket a raison de
viser « un site unique », mais c'est le vol 5 qui l'atteint, pas celui-ci.

### R7 — ce que ce vol NE fait pas, nommément

- **Aucune ligne de `dispatch-lib.sh` n'est éditée.** Le fichier n'est touché ni
  pour les enveloppes, ni pour le retrait des fonctions fuzzy, ni pour les
  tables. Même forme que le vol 3, et pour une raison plus forte (§ 1).
- **Aucune sous-commande `mika`** n'est ajoutée : ni `cli.rs`, ni `main.rs`, ni
  `commands/`. Donc aucun démarrage de `mika` ajouté, donc S3 reste sans objet
  pour ce vol.
- **Aucun scan anti-copie étendu** (AC4), **aucun test de code `≥2`** (AC3),
  **aucune mise à jour de doctrine sur la bascule** (AC6) : vol 5.
- **Aucune harmonisation de tolérance** (B1), tier 2 compris.

### R8 — la décision d'AC5, écrite ici et portée à la doctrine au vol 5

Elle est tranchée au § 7 de ce plan. Elle est écrite **dans ce vol** parce que
l'AC5 demande au *plan* de trancher, et parce que le nom du module Rust
(`arch_disposition`) et celui de la future sous-commande en dépendent. Son
inscription aux § 6 et 7 de `docs/architecture/dispatch-lib-migration.md`
accompagne la bascule, au vol 5, avec le passage de la phase 3 à « livrée ».

---

## 6. Le contrat du canal, tranché ici pour que le vol 5 n'ait pas à le redériver

Le vol 5 livre le canal. Son contrat est fixé maintenant, parce que la forme de
`Decision` (R1) doit le porter sans retouche.

**Nom et forme.** `mika arch-disposition --pass <first|second> --response-file
<chemin>`. Le corps passe par un **fichier**, jamais par un argument : un argv
ré-introduirait la classe de panne de portée de guillemets (cpp#157) dans le
geste même qui prétend la fermer. Le refus est **structurel** — aucune variante
positionnelle n'existe.

**Trois codes de sortie**, sur le modèle de `plan_callout.rs` :

| code | signification | stdout | stderr |
|---|---|---|---|
| `0` | un jeton a été lu | ligne 1 = le jeton, ligne 2 = le tier | le fragment décisif |
| `1` | aucun jeton (UNPARSED, tier 0 compris) | ligne 1 = **vide**, ligne 2 = le tier (`0` ou `-`) | la prose du tier |
| `≥2` | « je n'ai pas pu regarder » | — | `REFUSED (arch-disposition, mika#2194) <motif>: <détail>` |

**La sortie a TOUJOURS deux lignes**, y compris sous le code `1`. C'est ce qui
rend les deux enveloppes identiques et sans cas particulier : chacune rend la
ligne 1 (vide ⇒ elle n'émet rien, exactement comme aujourd'hui) et écrit `1` ou
`0` dans `$_DISPOSITION_FUZZY_FILE` selon que la ligne 2 vaut `2`. La règle
`-`-jamais-vide s'applique à la **ligne 2** ; elle ne s'applique **pas** à la
ligne 1, dont la vacuité *est* le signal que les trois appelants traitent déjà.

**Le refus doit atteindre une surface qu'on lit.** Le stderr d'avant-pilote est
structurellement perdu sur un dispatch qui réussit (classe mika#2050 : il hérite
du `Stdio::piped()` de l'exécuteur, que l'exécuteur ne lit que dans sa branche
`if !status.success()`). Le refus voyage donc jusqu'au `RESULT` du callback,
comme `_PLAN_CALLOUT_REFUSAL` le fait déjà.

**Pas de repli bash**, et l'asymétrie de déploiement est le pourquoi : *ancien
dispatch-lib + nouveau `mika`* est sûr par construction (le nouveau n'est pas
appelé) ; *nouveau dispatch-lib + ancien `mika`* sort non-zéro sur une
sous-commande inconnue ⇒ code `≥2` ⇒ refus **bruyant et nommé**
(`subcommand_error`), jamais un chemin vide. Un repli serait une seconde
implémentation — c'est-à-dire la chose que ce ticket existe pour retirer.

**Le canal vit dans `mika-cli`, le lecteur dans `mika-agent`** — le même partage
que `plan_callout` (lecteur `crates/mika-agent/src/plan_callout.rs`, canal
`crates/mika-cli/src/commands/plan_callout.rs`). Ce vol livre le lecteur ; le
canal est une enveloppe de lecture de fichier.

---

## 7. AC5 — la famille de sous-commandes, tranchée par écrit

Le ticket rappelle que le critère de bascule du § 6 de la doctrine — *« la
seconde sous-commande de prédicat distincte »* — **est atteint** par cette
tranche, et demande au plan de trancher entre une famille nommée
(`mika predicate <nom>`) et le maintien de la famille plate.

### Décision : la famille reste PLATE, et le critère du § 6 est REMPLACÉ, pas simplement appliqué

Trois mesures, dont la troisième est décisive.

**1. Le namespace de tête porte déjà cette classe, et la groupe déjà — ailleurs.**
`crates/mika-cli/src/cli.rs` déclare 31 sous-commandes. Trois d'entre elles sont
appelées par une machine et exécutables à la main : `Token`,
`CredentialHelper`, `PlanCallout`. Et elles sont **déjà groupées**, non par un
namespace mais par le chemin qui les distingue : `main.rs:470`,
`Some(Commands::Token(_) | Commands::CredentialHelper(_) | Commands::PlanCallout(_))`,
le court-circuit de tête qui saute `dotenv`, `Settings`, `home`, la DB et le
réseau. `arch-disposition` rejoint ce `match`. Le regroupement que la famille
nommée apporterait **existe déjà, au seul endroit où il a une conséquence**.

**2. La prémisse du critère — « ceci n'est pas pour vous » — est fausse pour
cette classe.** Elle n'est pas gratuite à acheter (`clap` a `hide = true`), elle
est **indésirable** : la façon dont un refus `≥2` de `plan-callout` se diagnostique
est de rejouer le prédicat à la main sur le fichier de corps exact. La
découvrabilité de ces sous-commandes est une propriété *utile*, pas du bruit. Un
namespace qui les cache aggrave le diagnostic de la seule classe d'erreur qu'elles
produisent.

**3. Le renommage serait un changement de contrat CLI à l'intérieur d'un vol de
parité.** Déplacer `plan-callout` sous `mika predicate` est un changement de
comportement observable d'un canal **livré et dépendu** (trois appelants de
production depuis la phase 2, et une aiguille de test à
`test-dispatch-lib.sh:11256`). Or la méthode de ce ticket est le strangler, dont
la preuve est la parité à comportement identique. Faire rouler un changement de
contrat dans un vol dont la validité repose sur « rien n'a changé » mélange deux
gestes de nature différente, sur du substrat de boucle dont le merge passe par le
mot de Vincent PR par PR. Et un alias — garder les deux orthographes — est
précisément le geste que ce dépôt refuse par écrit (deux orthographes pour une
intention, dont une qui dérive).

### Le critère de remplacement, pour qu'un futur lecteur ne croie pas le § 6 ignoré

> **Grouper quand les prédicats cessent d'être des instruments de diagnostic.**
> Tant que chaque sous-commande de prédicat est aussi quelque chose qu'un
> opérateur rejoue à la main pour diagnostiquer un refus `≥2`, elle appartient au
> namespace de tête, à côté de `token` et `credential-helper`, qui sont la même
> espèce. Le jour où un prédicat existe qu'un opérateur n'a aucune raison
> d'invoquer, la famille a quelque chose à grouper — et c'est ce jour-là, pas au
> deuxième membre, que `mika predicate <nom>` se paie.

L'alternative reste **refusée avec sa raison écrite**, pour ne pas être
re-proposée à l'aveugle au troisième prédicat. Le coût du maintien est celui que
la doctrine avait déjà borné : un renommage ultérieur produit la fenêtre
« dispatch-lib neuf + `mika` ancien », où le code `≥2` rend un refus nommé — et
`make deploy` ne produit pas cet état (le binaire qui seede est celui qui est
installé).

---

## 8. Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | `arch_disposition::decide` porte les cinq tiers dans l'ordre du bash | tests inline de `arch_disposition.rs` : un test par tier, plus un test d'ordre (tier 0b gagne sur un tier 1a présent) |
| V2 | le tier 0 **lit** la constante du moteur, il ne la recopie pas | `grep -c 'Disposition-Withheld' crates/mika-agent/src/arch_disposition.rs` rend `0` ; le module référence `agent_loop::DISPOSITION_WITHHELD_MARKER` |
| V3 | le marqueur cité en milieu de ligne ne supprime pas la disposition | ligne `tier-shadowed` du corpus, lue par les deux lecteurs |
| V4 | un `READY` cité avant une escalade du moteur perd | ligne `tier-nominal` tier `0b` du corpus, lue par les deux lecteurs |
| V5 | `Disposition: ITERATE` en seconde passe tombe au tier 2 | ligne `tier-shadowed` du corpus, passe `second` |
| V6 | **parité dorée** : le bash *pre-switch* et `decide` rendent les mêmes jetons sur tout le corpus | `make test-dispatch-lib` (lecteur bash, R5) **et** `cargo test -p mika-agent --test arch_paraphrase_parity` (lecteur Rust, R4), sur le même fichier d'attendus |
| V7 | anti-vacuité, quatre termes | TSV absent ⇒ rouge ; TSV vide ⇒ rouge ; une ligne nommant un corps absent ⇒ **échec**, pas saut ; `dispatch-lib.sh` qui ne se source plus ⇒ rouge via la sentinelle `declare -f` sur les **enveloppes** |
| V8 | le plancher de cas est relevé | `cases >= 27` dans `arch_paraphrase_parity.rs` ; `M3_DECLARED == M3_CASES` côté bash |
| V9 | le troisième site de match est déclaré | `scripts/canonical-tokens-survey.sh --check` vert ; `mika2201_every_declared_symbol_still_exists` vert |
| V10 | `agent_loop/mod.rs` tient sous le plafond, et sa garde de dérive survit | `wc -c crates/mika-agent/src/agent_loop/mod.rs` < 1 048 576 ; `cargo test -p mika-common --test claude_md_size` ; l'assertion `test-dispatch-lib.sh:6441` reste verte (regex non ancrée) |
| V11 | ce vol ne touche pas `dispatch-lib.sh` | `git diff --stat origin/main -- skills/bundled/_shared/dispatch-lib.sh` est **vide** |
| V12 | pipeline complet | `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`, `make test-dispatch-lib`, `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main` |

---

## Fire-Disposition

Ce vol livre des détecteurs : le corpus doré étendu et ses deux lecteurs, les
quatre termes d'anti-vacuité, les assertions auto-nettoyantes des classes, et la
sentinelle *pre-switch*. Leur chemin de succès est « aucun écart trouvé », donc
la section est requise.

**Option retenue : (a) exception nommée en allowlist.** La forme est celle que le
vol 3 a déjà posée et qui a tenu : la **colonne `classe` du fichier d'attendus
EST l'allowlist**, grep-visible ligne à ligne, et chaque classe porte dans
l'en-tête du TSV une assertion « ÉCHOUE si » qui rougit le jour où l'exception
cesse. Aucun `#[ignore]`, aucun détecteur désarmé, aucune halte.

Les quatre classes existantes sont **conservées telles quelles** (B1 : les
tolérances du tier 2 ne sont pas touchées). Ce vol en ajoute **deux**, et
seulement deux :

| classe / entrée | la donnée précise | le suivi | ce qui la rend auto-nettoyante |
|---|---|---|---|
| `tier-nominal` | le tier nommé par la colonne `tier` **tire** sur ce corps, et c'est le contrôle positif de ce tier | — (c'est le comportement porté, pas une exception) | le lecteur Rust ÉCHOUE si la ligne cesse de rendre son jeton **ou** change de tier : un portage qui réordonne les tiers rougit sur la colonne, pas seulement sur le jeton |
| `tier-shadowed` | le littéral du tier **est présent** et pourtant un autre tier (ou aucun) décide : marqueur cité en milieu de ligne, `READY` cité avant l'escalade du moteur, `Disposition: ITERATE` en seconde passe | mika#2194 (vol 5 : l'inventaire redevient un site unique) | le lecteur Rust ÉCHOUE si l'une de ces lignes se met à rendre le tier dont le littéral est présent — c'est-à-dire le jour où quelqu'un « répare » l'ancrage ou l'ordre sans toucher au bash |
| sentinelle *pre-switch* (ligne d'assertion, pas de classe) | `_parse_disposition` et `_parse_verdict` **n'appellent pas encore** `mika arch-disposition` | mika#2194 (vol 5) | elle rougit **le jour de la bascule**, et force la réécriture de l'en-tête du bloc (« ceci est désormais une non-régression, plus une preuve ») au lieu de laisser une parité tautologique passer pour une parité satisfaite |

**Contrôles négatifs mesurés, pas supposés.** Chaque « ÉCHOUE si » ci-dessus est
vérifié une fois à la main pendant l'implémentation, en cassant délibérément le
lecteur Rust, et le résultat est rapporté dans le corps de PR — le vol 3 a établi
cette discipline, et c'est ce qui sépare « l'assertion est auto-nettoyante » de
« l'assertion est écrite comme si elle l'était ».

**Et le refus, pour qu'il ne soit pas re-proposé :** aucune exception au-delà des
classes déclarées, et **aucune classe introduite vide**. Une allowlist livrée
vide est un geste que ce dépôt a déjà refusé par écrit (mika#2201), et les deux
classes ci-dessus naissent peuplées — `tier-nominal` par les quatre positifs,
`tier-shadowed` par les trois cas de l'AC1 et du § 4.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/arch_disposition.rs` (tiers 0, 0b, 1a, 1b + entrée unique) | 105 |
| tests inline de `arch_disposition.rs` (un par tier + ordre + fragment) | 55 |
| `crates/mika-agent/src/agent_loop/mod.rs` (`pub(crate)`, ≈ 11 octets) | 1 |
| corpus `tests/fixtures/arch_paraphrase_texts/` (8 corps + 12 lignes d'attendus + en-tête) | 46 |
| `crates/mika-agent/tests/arch_paraphrase_parity.rs` (colonne `tier`, routage vers `decide`, plancher) | 35 |
| `skills/bundled/_shared/test-dispatch-lib.sh` (6e colonne, routage vers les enveloppes, sentinelles) | 25 |
| `scripts/canonical-tokens.tsv` (le 3e site des quatre jetons + les deux marqueurs, déclarés) | 10 |

Total estimé : 277 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

### Pourquoi ce vol tient, et ce qui est renvoyé

277 ≤ 279, avec 2 lignes de marge. La marge est mince et c'est assumé : les deux
postes qui pourraient déraper sont bornés par leur nature. Le module Rust est un
portage **ligne à ligne** de cinq expressions dont les lignes bash sont comptées
au § 1 (93 + 92 lignes de parseurs, commentaires compris) ; le corpus est fait de
réponses d'architecte **courtes** (les 16 corps existants le sont, dont le
fondateur de 85 octets de mika#771). Si le portage dépasse, le poste qui cède est
`tests inline de arch_disposition.rs` — les cas qu'il perdrait sont déjà couverts
par le corpus doré, qui est la vraie mesure.

**Ce qui est renvoyé au vol 5, chiffré** (≈ 278 lignes, donc tenant aussi sous la
borne, et mergeable seul) :

| livrable du vol 5 | lignes estimées |
|---|---|
| `crates/mika-cli/src/commands/arch_disposition.rs` (le canal, trois codes) + ses tests | 105 |
| `crates/mika-cli/src/cli.rs` + `main.rs` (6 points de câblage, dont les 3 bras de chemin court) | 25 |
| `skills/bundled/_shared/dispatch-lib.sh` (les deux enveloppes écrites ; retrait de ≈ 250 lignes) | 45 |
| `test-dispatch-lib.sh` (scan anti-copie étendu aux tiers 1/2, garde de dérive **inversée**, test `≥2` à deux polarités, re-pointage des gardes `declare -f`) | 95 |
| `scripts/canonical-tokens.tsv` + `canonical-tokens-survey.sh` (commentaire l. 41) | 8 |

**Ordre de commit, et le plancher de chaque vol.** Vol 4 d'abord, et c'est
obligatoire : sans un commit *pre-switch* mergé, la parité du vol 5 est une
tautologie (§ 1). Plancher du vol 4 : `arch_disposition.rs` + le corpus + les
deux lecteurs — retirer l'un des deux lecteurs retire la parité, qui est **tout**
le livrable. Plancher du vol 5 : le canal + les deux enveloppes + le scan
anti-copie — livrer les enveloppes sans le scan laisserait les tables mortes dans
le fichier sans rien pour le dire.

Total des deux vols : ≈ 555 lignes, soit ≈ 2× la borne par vol — c'est la mesure
qui impose le découpage, et le § 1 donne la raison qui le rendait de toute façon
nécessaire.

Tracked in: senara-solutions/mika#2194

Le ticket parent est le porteur légitime du reste : la décision opérateur du
2026-09-30T19:03:44Z dit en propres termes que *« ce ticket parent reste ouvert
et les porte »*, et le réveil du 2026-11-02 a ouvert la phase 3 **dans ce ticket
même, sans nouveau ticket** (gel total jusqu'au 2026-12-01).

---

## Definition of Done

- [ ] `crates/mika-agent/src/arch_disposition.rs` existe, porte les tiers 0, 0b,
      1a et 1b et l'entrée unique `decide`, et son tier 0 **lit**
      `agent_loop::DISPOSITION_WITHHELD_MARKER` sans recopier le littéral.
- [ ] `agent_loop/mod.rs` ne porte **qu'un** changement : `const` → `pub(crate)
      const`. `wc -c` reste sous 1 048 576, et l'assertion de dérive
      `test-dispatch-lib.sh:6441` reste verte.
- [ ] Le corpus porte 8 nouveaux corps et sa sixième colonne `tier`, avec les
      quatre tiers couverts positivement et négativement, le marqueur cité en
      milieu de ligne, et le `READY` cité avant l'escalade du moteur.
- [ ] Les **deux** lecteurs lisent le même fichier d'attendus ; le lecteur bash
      route vers les **enveloppes** et non vers les fonctions fuzzy ; les deux
      rendent les mêmes jetons sur tout le corpus, **zéro écart**.
- [ ] Les quatre termes d'anti-vacuité sont en place, dont la sentinelle de
      sourcing re-pointée sur les enveloppes, et la sentinelle *pre-switch*.
- [ ] Les deux nouvelles classes du TSV sont peuplées à la naissance, chacune
      avec son « ÉCHOUE si », et **chaque contrôle négatif a été exécuté à la
      main** : son résultat est rapporté dans le corps de PR.
- [ ] `scripts/canonical-tokens.tsv` déclare le troisième site de match, et
      `canonical-tokens-survey.sh --check` est vert.
- [ ] `git diff --stat origin/main -- skills/bundled/_shared/dispatch-lib.sh`
      est **vide** : ce vol ne touche pas le fichier migré.
- [ ] Pipeline vert : `cargo test`, `cargo clippy --all-targets --all-features
      -- -D warnings`, `cargo fmt --check`, `make test-dispatch-lib`,
      `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`.
- [ ] Le corps de PR ne porte **ni `Closes #2194`, ni `Fixes`, ni `Resolves`** —
      la fermeture par effet de bord de #2607 a déjà coûté une réouverture
      manuelle de ce ticket. Forme prescrite :
      `Refs mika#2194 (vol 4 ; le ticket parent reste ouvert et porte le vol 5).`
- [ ] Le corps de PR porte `Code review: delegated to mika-qa (mika#2694)`, et
      la session n'invoque ni `ce-simplify-code` ni `ce-code-review`.
- [ ] Le corps de PR dit que la **parité est une preuve dans ce vol et cessera de
      l'être au vol 5**, et nomme le vol 5 par sa ligne `Tracked in:`.
- [ ] Le corps de PR nomme la décision d'AC5 (§ 7 : famille plate, critère du
      § 6 remplacé) pour que la revue la voie sans ouvrir le plan.
- [ ] `dispatch-lib.sh` est du substrat de boucle : le merge passe par le mot de
      Vincent.

---

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2194, section
*« Critères d'acceptation »*. La colonne *couverture* dit quel vol les porte ;
elle est un ajout du plan, pas du ticket.

- **AC1 : parité dorée sur les cinq tiers.** Le corpus
  `tests/fixtures/arch_paraphrase_texts/` (17 cas) est étendu aux tiers 0, 0b, 1a
  et 1b, chacun avec au moins un cas positif et un cas négatif. Le corpus inclut
  le marqueur cité au milieu d'une ligne, qui ne doit pas supprimer la
  disposition, et un `READY` cité en ligne avant une escalade du moteur, où
  l'escalade doit gagner. Le bash d'avant la bascule (`pre-switch`) et la
  sous-commande rendent les mêmes jetons sur tout le corpus, avec un terme
  d'anti-vacuité (§ 4) : un `dispatch-lib.sh` qui ne se source plus rougit.
  → **vol 4** (ce vol), à une nuance près, dite plutôt que glissée : la parité
  est établie contre **le lecteur** `arch_disposition::decide`, pas contre la
  sous-commande, qui n'existe pas encore. C'est la forme déjà en place au vol 3
  (le test de parité appelle la fonction de bibliothèque) ; la conformité
  *canal = lecteur* est un test distinct, sur le modèle de
  `mika2194_le_canal_repond_comme_le_lecteur_sur_le_corpus_dore`, et elle est
  livrée au vol 5 avec le canal.
- **AC2 : non-régression `post-switch`.** Les tests existants de
  `test-dispatch-lib.sh` et de `_shared/tests/` qui appellent
  `_parse_disposition`, `_parse_verdict` ou `_disposition_was_fuzzy` passent sans
  modification de leurs attentes. Les dix appels directs de
  `_parse_disposition_fuzzy` et `_parse_verdict_fuzzy` dans les tests (mesurés le
  2026-11-02) ne disparaissent pas avec ces fonctions : chacun devient un cas du
  corpus doré, avec son attente inchangée.
  → **vol 5** pour la non-régression *post-switch*. La seconde phrase est
  **réalisée dès le vol 4** : le lecteur de corpus cesse d'appeler les fonctions
  fuzzy et route vers les enveloppes. Mesure à substituer aux « dix appels » :
  deux appels réels, quatre gardes `declare -f` à re-pointer, quatre
  commentaires (§ 3 D2).
- **AC3 : code `≥2`.** Une sous-commande absente ou un fichier illisible produit
  un refus nommé dans le `RESULT`, jamais un UNPARSED silencieux. Un test
  épingle les deux polarités dans le même corps : le refus sort quand le moteur
  est absent, et rien ne sort sur une réponse saine.
  → **vol 5** (le canal). Son contrat est fixé au § 6 de ce plan.
- **AC4 : le scan anti-copie (§ 5).** Aucun motif de tier 1 ou 2 ne subsiste dans
  `dispatch-lib.sh`. Le scan est rouge si une table revient.
  → **vol 5**, augmenté d'une mesure de ce plan : la garde de dérive du marqueur
  (`test-dispatch-lib.sh:6445-6449`) exige aujourd'hui que le bash **porte** le
  littéral du tier 0. Elle doit **s'inverser** avec l'AC4, pas disparaître
  (§ 3 D3).
- **AC5 : la famille de sous-commandes.** C'est la **seconde** sous-commande de
  prédicat. Le critère de bascule écrit au § 6 est donc atteint. Le plan tranche
  par écrit entre une famille nommée et le maintien de la famille plate, et la
  doctrine reçoit la décision.
  → **tranché au § 7 de ce plan** (famille plate ; le critère du § 6 est
  remplacé, avec son remplaçant écrit et l'alternative maintenue refusée). La
  doctrine reçoit la décision au vol 5, avec le passage de la phase 3 à
  « livrée ».
- **AC6 : la doctrine est mise à jour.** Les § 6 et 7 de
  `dispatch-lib-migration.md` passent la phase 3 à « livrée » et notent la
  décision sur le canal ci-dessus.
  → **vol 5**. La phase 3 n'est pas livrée tant que la bascule n'est pas faite ;
  l'écrire au vol 4 serait déclarer livrée une phase dont le bash ne bouge pas.
