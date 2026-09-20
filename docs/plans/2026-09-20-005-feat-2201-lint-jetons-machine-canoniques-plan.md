# mika#2201 — Le lint porte sur les jetons dont le lecteur est strict, et la liste canonique porte la tolérance de chacun

- **Ticket :** senara-solutions/mika#2201
- **Type :** feat (garde CI)
- **Date :** 2026-09-20

---

## M0 — La rectification, écrite avant le reste

Le ticket demande un lint qui refuse « toute variante non canonique d'un jeton
machine (traduction, casse, sous-chaîne ambiguë) ». Confrontée au code qui
matche — l'exigence Prime elle-même — cette règle **accuserait deux formes que
le dépôt lit délibérément bien**, et son AC3 prescrit l'une des deux comme
fixture de non-vacuité. La rectification est donc le premier livrable, et elle
repose sur une lecture, pas sur une préférence.

### R1 — « seconde passe » n'est pas une variante non canonique : c'est une forme canonique

`crates/mika-agent/src/grooming_marker.rs`, lecteur **unique** du marqueur de
verdict depuis mika#2158, est bilingue **par décision écrite** :

```rust
static LATER_PASS_RE = r"(?i)(second-pass|seconde passe|deuxième passe|deuxieme passe)"
static FIRST_PASS_READY_RE = r"(?i:first-pass|première passe|premiere passe)\s*\(\s*(READY)"
```

Et son doc-comment tranche le sens de l'alignement :

> C'est le **prédicat** qui s'aligne sur la spec, pas l'inverse. […] Corollaire
> assumé : la spec n'a pas à imposer l'anglais pour être lisible par la machine,
> dans un dépôt qui écrit ses tickets et ses plans en français.

Le ticket cite lui-même le bearing Prime qui dit la même chose : « **Le français
n'a pas mordu — une frontière textuelle a mordu, et elle est maintenant
structurelle.** » La morsure de #1772 a été fermée **en élargissant le lecteur**,
pas en resserrant le texte. Un lint qui rougit sur « seconde passe » rougirait
donc sur une forme que la machine lit, et rougirait sur `grooming_marker.rs`
lui-même, sur ses 24 tests, et sur tout corps de ticket français du dépôt.

**Conséquence sur AC3 : la fixture « seconde passe » change de signe.** Elle
reste une fixture — mais de **non-régression** : elle doit rester **verte**. Un
jour où elle rougit, quelqu'un a resserré le lint sur une population que le
lecteur accepte, et c'est cette garde qui le dira.

### R2 — « ESCALATE-divergence » reste un défaut réel, et le lint y est légitime

mika#2188 a **écarté explicitement** le resserrage de `VERDICT_TOKEN_RE` (« elle
ferait dépendre le verdict de l'orthographe d'un mot composé plutôt que de la
chronologie ») et a résolu par la position : un marqueur abouti postérieur
surclasse l'escalade. Mais cette résolution est **conditionnelle à l'existence
d'une passe postérieure**. Un callout qui porte
`(ESCALATE-divergence, résolu par l'opérateur)` **et rien après** se lit
`Escalated` — la prose dit « résolu », la machine dit « escaladé ». Le défaut
n'est pas fermé : il est **déplacé sur le cas sans passe suivante**, et la
doctrine positionnelle ne prétend pas le couvrir (elle l'écrit : « prétendre
qu'elle distingue une escalade résolue d'une escalade ouverte serait lui prêter
une lecture sémantique qu'elle n'a pas »).

C'est exactement ce qu'un lint textuel peut attraper là où un prédicat
positionnel ne le peut pas. **AC3 est donc ratifiée pour cette moitié** :
`ESCALATE-divergence` dans une ligne de callout doit faire rougir.

### R3 — L'asymétrie qui gouverne tout le lint

Les deux cas ci-dessus ne diffèrent pas par la langue ni par la casse. Ils
diffèrent par **la tolérance du lecteur** :

| | lecteur | une variante est… | le lint doit |
|---|---|---|---|
| `seconde passe` | `(?i)` + FR | **lue** | se taire |
| `ESCALATE-divergence` | `\b…\b`, sous-chaîne | **mal lue** | rougir |

**La règle du lint n'est donc pas « la forme canonique est l'anglais ». C'est :
une forme qu'un lecteur strict ne voit pas est refusée ; une forme qu'un lecteur
tolérant voit est admise.** La liste canonique doit par conséquent porter, pour
chaque jeton, **la tolérance de son lecteur** — sans quoi elle ne peut pas être
« confrontée aux sites de match réels » comme AC2 l'exige, elle ne peut que les
contredire.

### R4 — Le mode de panne que le remède naïf atteint

`scripts/check-a2a-timeout-literals.sh` (mika#2309) a déjà dû écrire cette
leçon, et elle s'applique mot pour mot :

> Demander qu'une durée de vie de JWT soit single-sourcée sur une variable d'env
> ne veut rien dire — donc la garde se fait désarmer, ou son allowlist devient le
> tiroir fourre-tout dans lequel la régression qu'elle existe pour attraper passe
> inaperçue. **C'est le mode de panne que le ticket existe pour prévenir, atteint
> par le remède.**

Un lint qui accuse chaque « seconde passe » du dépôt produit des dizaines de
faux positifs le jour de sa naissance. Il finit désarmé, ou allowlisté en masse
— et le jour où un vrai jeton dérive, il passe dans le bruit. La borne de
fermeture de Prime (« un jeton oublié, et "seconde passe" revient sous un autre
nom ») a un symétrique que ce plan pose : **un jeton accusé à tort, et le lint
n'est plus lu.**

---

## M1 — L'inventaire exhaustif des sites de match (exigence Prime)

### M1.0 — Le relevé est produit par une commande, jamais tenu à la main

**La première version de ce plan présentait la table ci-dessous comme « relevé
sur l'arbre à `17f42a6b` », sans livrer le geste qui la produit. C'était une
promesse, pas une preuve — et la révision l'a démontré sur elle-même : le relevé
était *exact* sur chacune de ses lignes et *incomplet*, ratant
`dispatch-lib.sh::_extract_plan_path` (`grep -oP '^> - \*\*Plan:\*\* `\K…'`,
ligne 6581), qui est le lecteur le plus strict du callout `Plan` de tout le
dépôt.** Un relevé manuel peut être juste et manquer un site ; c'est précisément
la classe que AC4 existe pour fermer, et il serait incohérent de fonder AC2 sur
le mécanisme que AC4 déclare insuffisant.

Le plan livre donc **`scripts/canonical-tokens-survey.sh`** — le producteur du
relevé, exécutable par un tiers, dont la sortie est l'entrée de la colonne
`site de match` du TSV :

```sh
# Énumère tout site de match candidat : littéral de callout, token de verdict,
# clé de marqueur, préfixe webhook — dans les cinq formes de lecture strictes
# que le dépôt emploie (Regex::new, starts_with, strip_prefix, match_indices,
# grep -E/-oE/-oP, sed -n 's/^…/').
scripts/canonical-tokens-survey.sh            # tableau lisible (fichier, symbole, forme, littéral)
scripts/canonical-tokens-survey.sh --tsv      # colonnes 1 et 3 du TSV, prêtes à diff
scripts/canonical-tokens-survey.sh --check    # diff contre le TSV livré ; rouge sur divergence
```

Le mode `--check` **est** le scan M5 dans sa moitié shell : le relevé et la garde
d'exhaustivité partagent un lecteur, pour la raison que `grooming_marker.rs` a
déjà dû écrire — deux lecteurs d'une même question divergent, et la divergence
est silencieuse (mika#2158 : deux regex de marqueur ont répondu différemment
pendant des mois sans que rien ne casse).

**Le TSV désigne un site par son symbole, jamais par un numéro de ligne.** Un
numéro pourrit au premier commit qui insère une ligne au-dessus, et il pourrit
*en silence* : la colonne resterait syntaxiquement valide en désignant autre
chose. Le format est `chemin::symbole` (`crates/mika-agent/src/auto_pull.rs::PLAN_CALLOUT_RE`,
`skills/bundled/_shared/dispatch-lib.sh::_extract_plan_path`). Les numéros de
ligne des tables ci-dessous sont donnés **à titre de repère de lecture à
`17f42a6b`** et ne sont pas ce que le TSV porte.

### M1.1 — La classification, seule part qui reste au plan

Ce que la commande ne peut pas produire, c'est la colonne `classe` : décider
qu'un lecteur est tolérant ou strict est une lecture de son code, pas un motif
textuel. **C'est la seule part de M1 que ce plan revendique**, et c'est celle qui
porte R1. **C'est la table que le lint doit refléter**, et AC2 exige qu'elle vive
dans un fichier unique plutôt que dans ce plan.

### Classe A — lecteur TOLÉRANT (hors périmètre du lint, par décision)

| jeton | site de match | tolérance mesurée |
|---|---|---|
| `second-pass` | `grooming_marker.rs:LATER_PASS_RE` | `(?i)` + `seconde passe` / `deuxième passe` / `deuxieme passe` |
| `first-pass` | `grooming_marker.rs:FIRST_PASS_READY_RE` | `(?i:)` sur le préfixe + `première passe` / `premiere passe` |
| dispositions paraphrasées | `dispatch-lib.sh:_parse_disposition_fuzzy` | tier 2 paraphrase, `ESCALATE > ITERATE > READY` |
| verdicts paraphrasés | `dispatch-lib.sh:_parse_verdict_fuzzy` | tier 2 paraphrase, `ESCALATE > GROOMED` |
| valeur de `rescue-pipeline-verified` | `wip_rescue.rs:pipeline_verified` | casse + espaces (`Yes`, `:yes`, ` : Yes `) |
| `Closes #N` / `Tracked in:` | `check-pr-body-consistency.sh:122,148` | `grep -qiE` — insensible à la casse |

**Ces six lignes sont la raison d'être de la liste.** Sans elles, un futur
auteur du lint qui part de la seule intuition « jeton machine = anglais exact »
les accuse toutes les six.

### Classe B — lecteur STRICT (périmètre du lint)

| jeton canonique | site de match | rigueur |
|---|---|---|
| `> - **Grooming history:**` | `grooming_marker.rs:CALLOUT_LINE_RE` | `(?m)^` ancré, littéral |
| `> - **Plan:** \`…docs/plans/…\`` | `auto_pull.rs:562` | `(?m)^` ancré ; un segment de dépôt optionnel (mika#2120) |
| `> - **Plan:**` | `dispatch-lib.sh:1870,2284` | `sed -n 's/^…/` et `grep -qE '^…'` ancrés |
| `> - **Plan:** \`…docs/plans/…\`` | `dispatch-lib.sh:6581` (`_extract_plan_path`) | `grep -oP '^…\K'` ancré — **le lecteur le plus strict du dépôt**, et celui que le relevé manuel avait raté (M1.0) |
| `> - **Branch:**` | `auto_pull.rs:865` | `(?m)^` ancré, littéral |
| `GROOMED`, `ESCALATE[DS]?` | `grooming_marker.rs:VERDICT_TOKEN_RE` | `\b…\b`, **casse stricte** |
| `READY` (1ʳᵉ passe) | `grooming_marker.rs:FIRST_PASS_READY_RE` grp 1 | **casse stricte** |
| `Disposition: READY\|ITERATE\|ESCALATE` | `dispatch-lib.sh:5168,5267` | `grep -oE` littéral, casse stricte |
| `Verdict: GROOMED\|ESCALATE` | `dispatch-lib.sh:5177,5250` | `grep -oE` littéral, casse stricte |
| `Outcome: PR_OPENED\|PLAN_COMMITTED\|PLAN_GROOMED\|ESCALATE` | `dispatch-lib.sh:2973` | `grep -m1 -E '^Outcome: '` ancré |
| `<!-- rescue-pipeline-verified: … -->` (**clé**) | `wip_rescue.rs:315` | clé stricte, valeur tolérante |
| `RECOVERY_PENDING: true` | `dispatch-lib.sh:3819` | littéral |
| `[GitHub] …` (préfixes webhook) | `webhook_dispatch.rs:44,51,55` | `starts_with`, littéral |
| `READY_LABEL_DISPATCH_MARKER` | `ready_label_handler.rs:395,1152` | `starts_with` / `strip_prefix` |
| `## Acceptance criteria` | `scripts/verify-pipeline.sh` | littéral |
| labels : `ready`, `blocked`, `operator-review`, `operator-gated`, `wip-rescue`, `human-review-required`, `stale-against-main`, `origin:loop`, `dispatch:*` | `auto_pull.rs`, `wip_rescue.rs`, `webhook_dispatch.rs`, `.github/labels.yml` | égalité exacte |

### Ce que l'inventaire a trouvé en chemin, et qui n'est pas de ce ticket

- **`post-launch` n'est déclaré nulle part et n'est lu par aucun code.** Le
  commentaire opérateur de mika#2201 l'emploie (« dé-parqué »), et le workflow de
  sync tourne en `delete-other-labels: true` : c'est la quatrième occurrence de la
  classe `un-label-denforcement-non-declare-echoue-en-silence`, à ceci près qu'il
  n'est **pas** un label d'enforcement — sa perte ne casse aucune garde, elle
  perd un parking. **Ticket de suivi**, pas une extension de périmètre ici : ce
  ticket porte sur les jetons de *texte*, la déclaration des labels a déjà son
  mécanisme (`check-dispatch-seats-declared.sh`).

---

## M2 — Le périmètre du lint : trois surfaces, trois statuts, et l'une n'est pas un gate

AC1 dit « refuse dans les callouts/**corps de tickets** ». Un lint CI ne voit pas
un corps de ticket GitHub, et **GitHub n'offre aucun gate sur une issue** : un
workflow `issues:` peut annoter, jamais refuser. Prétendre livrer un gate là
serait annoncer une protection qui n'existe pas. Les trois surfaces sont donc
livrées avec leur statut réel.

| surface | qui l'écrit | statut | pourquoi |
|---|---|---|---|
| **S1 — prescripteurs du dépôt** (`.claude/commands/*.md`, `skills/bundled/**/system_prompt.md`, `dispatch-lib.sh`) | humains + revue | **gate CI bloquant** | **levier structurel** : un prompt qui prescrit une forme fausse la reproduit sur *tous* les tickets qu'il produit. C'est là que « seconde passe » serait redevenu systématique. |
| **S2 — corps de PR** | pilote + opérateur | **gate CI bloquant** (extension de `pr-body-validation.yml`) | Un gate existe déjà sur cet objet ; l'étendre coûte une passe, pas un mécanisme. |
| **S3 — corps de tickets** | `_write_canonical_callout` (machine) sur le chemin autonome ; pilote/opérateur sinon | **annotation non bloquante** (`issues: [opened, edited]`) | Aucun gate n'existe sur l'objet. Et le producteur canonique est **déjà une machine** : `_write_canonical_callout` compose les trois formes depuis un `case` fermé. La marge d'erreur restante est le chemin opérateur, qui mérite un signal, pas un refus qu'on ne peut pas rendre. |

**S3 est borné par une époque.** Le bearing Prime interdit la migration
rétroactive (« coût pur contre bénéfice nul ») ; le workflow n'annote donc que
les issues `opened`/`edited` **après** son déploiement, jamais l'historique.
L'annotation est un commentaire nommant le jeton, sa forme canonique et le site
de match qui ne le verra pas — pas un label (un label non déclaré serait
supprimé en silence, cf. la classe ci-dessus).

---

## M3 — AC2 : la liste canonique, et pourquoi elle porte quatre colonnes

`scripts/canonical-tokens.tsv` — un fichier, deux lecteurs (le lint shell et le
test Rust d'exhaustivité), format aligné sur `scripts/a2a-timeout-allowlist.txt`.

```
# jeton	classe	site de match	tolérance
second-pass	A	crates/mika-agent/src/grooming_marker.rs:LATER_PASS_RE	ci+fr:seconde passe,deuxième passe,deuxieme passe
GROOMED	B	crates/mika-agent/src/grooming_marker.rs:VERDICT_TOKEN_RE	exact:word-boundary
> - **Plan:**	B	crates/mika-agent/src/auto_pull.rs:562	exact:line-anchored
…
```

- **`classe`** décide si le lint accuse (`B`) ou se tait (`A`).
- **`site de match`** est ce qui rend la liste *confrontable* : AC2 demande
  « générée ou confrontée aux sites de match réels », et une liste sans pointeur
  ne peut qu'être crue.
- **`tolérance`** est la colonne que l'intuition naïve omet, et c'est celle qui
  empêche R1 de se reproduire.

**Le TSV plutôt que le TOML** : le lint est du shell (le patron du dépôt :
huit `check-*.sh`), et parser du TOML en shell est un mécanisme de plus pour un
fichier de trente lignes. Le TSV se lit en `awk` d'un côté et en `split('\t')`
de l'autre.

---

## M4 — Les règles du lint (surfaces S1+S2+S3)

Chaque règle est **négative et nommée**. Aucune ne dit « ce texte doit contenir
X » : un lint qui exige une forme sur un corps libre accuse toute prose.

- **L1 — sous-chaîne ambiguë dans une ligne de callout.** Dans une ligne
  `^> - \*\*Grooming history:\*\*`, un token de classe B suivi ou précédé d'un
  caractère de mot composé (`ESCALATE-divergence`, `GROOMED-partiel`) est refusé.
  C'est la moitié d'AC3 que R2 ratifie. **Bornée à la ligne de callout** : la même
  chaîne en prose est hors périmètre, exactement comme `CALLOUT_LINE_RE` l'est.
- **L2 — casse d'un token de verdict dans une ligne de callout.** `groomed`,
  `Escalate`, `ready` (comme disposition) dans un callout sont refusés : le
  lecteur est sensible à la casse, et le doc-comment l'écrit (« c'est un token
  produit par le pipeline, pas un mot de prose »).
- **L3 — préfixe de callout non canonique.** `> - **Plan :**` (espace avant les
  deux-points, réflexe typographique français), `> - **Branche:**`,
  `> - **Historique de grooming:**` sont refusés : les trois regex sont ancrées et
  littérales, une seule de ces formes rend le ticket invisible. **C'est la
  variante la plus probable dans un dépôt qui écrit en français, et aucune des
  deux morsures citées par le ticket ne l'a encore produite** — c'est la valeur
  prospective du lint.
- **L4 — jeton de classe B traduit.** `Verdict :`, `Disposition :`,
  `Résultat:` à la place de `Outcome:` — même raison que L3.
- **L5 — label non déclaré** cité comme instruction dans un prescripteur (S1) :
  confronté à `.github/labels.yml`. Complète `check-dispatch-seats-declared.sh`,
  qui ne couvre que la famille `dispatch:`.

**Ce que le lint ne fait pas :** il ne lit aucun jeton de classe A, il ne lit
aucune prose hors ligne de callout, et il n'exige jamais la présence d'un jeton.

---

## M5 — AC4 : le scan de garde d'exhaustivité

Le patron existe : `grooming_marker::tests::no_grooming_regex_outside_this_module`
refuse déjà une regex de marqueur de passe hors de son module, sur
`crates/mika-agent/src`. AC4 demande la généralisation.

`canonical_tokens::tests::mika2201_every_match_site_is_declared` — scan de source
sur `crates/*/src`, `scripts/*.sh`, `skills/bundled/**/*.sh` :

- tout littéral appartenant à la classe B trouvé dans un `Regex::new`, un
  `starts_with`, un `strip_prefix`, un `contains` ou un `grep -E` doit voir son
  fichier cité dans la colonne `site de match` du TSV ;
- **allowlist livrée vide**, et la doctrine est écrite au-dessus : quand le scan
  tire, on **déclare le site dans le TSV**, on ne l'allowliste pas. C'est la
  discipline de `mika2323_no_gate_predicate_reads_the_actor` et de
  `mika1883_run_usage_accumulates_only_via_the_one_helper`.

**Pourquoi un scan de source et pas un test comportemental.** La régression que
AC4 vise ne rend aucune décision fausse : elle rend un site **invisible à la
liste**. Toutes les assertions de comportement resteraient vertes pendant que
l'exhaustivité — la borne de fermeture de Prime — se perd. C'est la raison que
mika#2131 a déjà dû écrire pour sa propre garde.

---

## M6 — Author ≠ outside-check

L'exigence Prime sépare l'écriture du lint de la vérification d'exhaustivité.
La table M1 de ce plan est le relevé de l'auteur ; elle n'est **pas** la preuve
d'exhaustivité. La passe distincte est **le scan M5 exécuté contre le TSV**, qui
ne peut pas être vert par accord avec son auteur : il lit l'arbre, pas le plan.

---

## Fire-Disposition

Ce plan livre **six détecteurs** qui peuvent tirer sur des données préexistantes.
Le gate mika#1574 exige que chacun nomme sa disposition — (a) exception
d'allowlist nommée, (b) landing désarmé, (c) halt-and-surface — plutôt que de la
laisser au pilote. Ce que la table tranche n'est pas « que se passe-t-il si tout
va bien » mais **« que fait l'implémenteur le jour où V5 rougit »**, qui est une
décision de design et non une contingence.

| # | détecteur | surface | tire-t-il à HEAD ? | disposition |
|---|---|---|---|---|
| D1 | L1–L4 (sous-chaîne, casse, préfixe de callout, jeton traduit) | S1 — prescripteurs | **à mesurer** (V5) | **(a)** exception nommée par site, avec ticket de suivi et assertion auto-nettoyante |
| D2 | L5 (label non déclaré) | S1 — prescripteurs | **à mesurer** (V5b) | **(a)**, même forme que D1 |
| D3 | L1–L4 | S2 — corps de PR | **à mesurer** (V5c, F3) | **(b)** landing désarmé si l'échantillon rougit — le lint est livré en annotation sur S2 et armé par un commit suivant |
| D4 | L1–L4 | S3 — corps de tickets | **oui, par construction** | **(b)** désarmé *structurellement* et définitivement : S3 est une annotation, jamais un gate (M2) |
| D5 | `mika2201_every_match_site_is_declared` | arbre entier | **oui, avec certitude** | **(c)** halt-and-surface pendant l'implémentation, résolu en **déclarant le site dans le TSV** |
| D6 | `canonical-tokens-survey.sh --check` | arbre entier | **oui, avec certitude** | **(c)**, identique à D5 — même lecteur, même résolution |

### Résultat mesuré à l'implémentation (2026-09-20)

Le *Definition of Done* exige que V5/V5b/V5c soient mesurés et leur résultat
reporté ici avant le merge. Voici ce qu'ils ont donné.

| # | mesure | résultat | disposition appliquée |
|---|---|---|---|
| D1 (V5) | L1–L4 sur S1 | **zéro accusation** | rien à faire ; le fichier d'exceptions reste **vide** |
| D2 (V5b) | L5 sur S1 | **une accusation** — `needs-multi-agent-review`, écrit par `_finalize_pr_gate` en `--add-label … \|\| true`, déclaré nulle part | **non** la disposition (a) : le remède juste est la **réparation**, faite dans le même commit — le label est déclaré dans `.github/labels.yml`, comme mika#2199 l'a fait pour `human-review-required`. Une exception aurait pérennisé un label d'enforcement supprimable en silence, c'est-à-dire la panne que la règle existe pour voir |
| D3 (V5c) | L1–L4 sur un échantillon S2 | **non mesurable** — `gh` n'est pas authentifié dans le bac à sable de dispatch, l'échantillon n'a pas pu être récupéré | **(b) désarmé**. Le plan prévoyait le cas « rouge », pas le cas « non mesurable » ; la lecture sûre est la même — *ne pas armer ce qu'on n'a pas mesuré*. La condition d'armement et le geste (retirer une ligne `continue-on-error`) sont écrits dans `pr-body-validation.yml` |
| D4 | — | conforme | annotation, jamais un gate |
| D5 | scan Rust | **a tiré**, 1 site : `executor.rs::check_grooming_markers`, lecteur par `contains` que le survey ne voit pas | **(c)** appliqué : site **déclaré** dans le TSV, allowlist toujours vide |
| D6 | `--check` | **a tiré** au premier jet, puis vert | **(c)** appliqué : le TSV est **produit** depuis la sortie du survey |

**Un sixième détecteur non prévu par la table, et c'est le plus instructif.** Le
lint accusait le **plan de ce ticket même** (2 × L4) : son § M4 décrit la règle
en citant les formes fautives entre backticks. Le corps de PR l'aurait fait
aussi, et tout ticket futur discutant de la règle. C'est R4 sous sa forme la plus
pure — *un document qui explique le lint fait rougir le lint, et le lint se fait
désarmer*. Remède **structurel** et non une exception : un jeton cité dans un
span de code inline ou un bloc clôturé est une **mention**, jamais une
instruction. C'est le geste qu'`auto_pull::is_groomed` fait déjà pour la même
raison (mika#2120), et la classe du faux positif du Signal S (mika#2050). Non
appliqué à L5, où l'instruction `gh` d'un prescripteur markdown vit
nécessairement dans un bloc.

### D1/D2 — la forme exacte d'une exception, si V5 rougit

Une exception n'est jamais un motif large. Elle est une ligne de
`scripts/canonical-tokens-exceptions.tsv` portant **quatre champs obligatoires** :
`fichier`, `jeton`, `ticket de suivi`, `date`. Et elle porte son **assertion
auto-nettoyante** : le lint refuse une exception dont le fichier ne contient plus
le jeton accusé. Une exception qui survit à sa cause est la manière dont une
allowlist devient le tiroir fourre-tout de R4 ; l'assertion est ce qui l'empêche,
et elle rougit **le jour de la réparation**, pas des mois après.

**Le fichier d'exceptions est livré vide.** S'il est non vide au moment du merge,
son contenu est la mesure du défaut préexistant et chaque ligne a son ticket.

### D3 — pourquoi S2 est la seule surface qui peut lander désarmée

D1 et D3 exécutent les **mêmes règles**, mais pas sur la même population :
S1 est un ensemble **fini, versionné et relu** (une trentaine de prescripteurs),
tandis que S2 est un flux dont l'échantillon mesurable est un passé qu'on ne peut
pas corriger — une PR mergée ne se ré-écrit pas. Sur S1, une accusation est un
fichier à corriger dans le même commit ; sur S2, une accusation sur les PR
ouvertes au moment du déploiement bloquerait un travail en vol pour une forme que
personne ne lui avait demandée. D'où la disposition (b) **conditionnelle** : si
V5c est propre, S2 land armé avec S1 ; s'il rougit, S2 land en annotation et
l'armement est un commit distinct, après correction des PR concernées.

**Ce n'est pas une réduction d'AC1** : AC1 est ratifiée sur S2 comme gate
bloquant, et la disposition (b) porte sur la *fenêtre de déploiement*, pas sur la
cible. La condition d'armement est écrite et vérifiable (V5c vert), donc elle ne
peut pas devenir un désarmement permanent qu'on aurait oublié d'armer.

### D5/D6 — pourquoi la certitude de tirer n'est pas un problème

Ces deux-là **tirent à coup sûr**, et c'est voulu : M1.0 a déjà mesuré qu'un
relevé manuel manque des sites. Leur premier tir *est* le complément de M1. La
doctrine est écrite en M5 et elle ne souffre aucune exception : **on déclare dans
le TSV, on n'allowliste pas.** Il n'existe donc pas de fichier d'exceptions pour
D5/D6 — l'allowlist est vide et le reste, et le jour où le scan tire, la
résolution est une ligne de TSV. Un site de match qu'on ne veut pas déclarer est
un site de match qu'il faut supprimer.

**Conséquence sur l'ordre d'implémentation** : D6 s'exécute **avant** que le TSV
soit figé (il le produit), D5 **après** (il le vérifie). Les inverser ferait
passer la garde d'exhaustivité pour verte par accord avec son propre producteur —
c'est exactement ce que M6 (Author ≠ outside-check) interdit.

---

## Verification Contract

| # | Quoi | Comment |
|---|---|---|
| V1 | `ESCALATE-divergence` dans un callout rougit | fixture `scripts/fixtures/canonical-tokens/escalate-divergence.md`, attendue **rouge** |
| V2 | « seconde passe » dans un callout **passe** | fixture `…/seconde-passe.md`, attendue **verte** — garde de non-régression de R1 |
| V3 | `> - **Plan :**` (espace typographique FR) rougit | fixture `…/plan-espace-fr.md` |
| V4 | `groomed` minuscule dans un callout rougit | fixture `…/verdict-minuscule.md` |
| V5 | **L1–L4 à HEAD sur S1** | `scripts/check-canonical-tokens.sh` sur l'arbre — **zéro accusation** |
| V5b | **L5 à HEAD sur S1** | même script, règle L5 seule, confrontée à `.github/labels.yml` — **zéro accusation** |
| V5c | **L1–L4 sur un échantillon S2** | même script sur les corps des **PR ouvertes + les 50 dernières PR mergées** (`gh pr list --state all --limit 50 --json body`) — **zéro accusation** ; conditionne l'armement de D3 |
| V6 | Le scan d'exhaustivité voit chaque site de M1 | `cargo test -p mika-agent mika2201_every_match_site_is_declared` |
| V6b | Le relevé et le TSV concordent | `scripts/canonical-tokens-survey.sh --check` — **zéro divergence** (F2) |
| V7 | Un nouveau site de classe B non déclaré fait rougir | test négatif : fichier temporaire portant `Regex::new(r"\bGROOMED\b")` |
| V7b | Une exception périmée fait rougir | test négatif : ligne d'exception dont le fichier ne porte plus le jeton — assertion auto-nettoyante de D1 |
| V8 | Le lint a son test | `scripts/test-check-canonical-tokens.sh`, patron des huit `test-check-*.sh` |
| V9 | CI le fait tourner | job `canonical-tokens-lint` dans `ci.yml` |

**V5, V5b et V5c sont le contrôle de non-vacuité inverse, et ils sont porteurs** :
un lint qui accuse l'arbre à sa naissance est un lint qui sera désarmé (R4). Leur
séparation en trois est la réponse à F3 — la première version ne mesurait que S1
alors que S2 est un gate bloquant lui aussi, ce qui laissait la surface S2 exposée
au mode de panne de R4 sans qu'aucun chiffre ne le dise. **Le résultat de chacun
alimente la table Fire-Disposition** : V5/V5b décident si D1/D2 ont besoin
d'exceptions nommées, V5c décide si D3 land armé ou désarmé.

**Pourquoi l'échantillon S2 est « ouvertes + 50 dernières mergées » et pas
l'historique.** Les PR ouvertes sont la population que le gate bloquerait *le jour
même* — c'est la mesure du coût immédiat. Les 50 dernières mergées mesurent la
**tendance du producteur** (pilote + opérateur) : si la forme fautive y est
absente, elle n'apparaîtra pas demain. Remonter plus loin mesurerait un producteur
qui n'existe plus, et le bearing Prime interdit déjà la migration rétroactive.

---

## Definition of Done

- `scripts/canonical-tokens-survey.sh` livré (`--tsv`, `--check`) — **le
  producteur du relevé**, sans lequel AC2 reste une promesse (F2/M1.0).
- `scripts/canonical-tokens.tsv` livré, quatre colonnes, classes A et B peuplées
  **depuis la sortie du survey**, colonne `site de match` au format
  `chemin::symbole` (jamais un numéro de ligne).
- `scripts/canonical-tokens-exceptions.tsv` livré **vide**, quatre champs
  obligatoires, assertion auto-nettoyante (Fire-Disposition D1).
- `scripts/check-canonical-tokens.sh` + `scripts/test-check-canonical-tokens.sh`
  livrés, patron `check-*.sh` du dépôt.
- Job `canonical-tokens-lint` dans `.github/workflows/ci.yml` (S1) et passe
  ajoutée à `pr-body-validation.yml` (S2) — **armée si V5c est vert, en
  annotation sinon** (Fire-Disposition D3).
- Workflow `issue-token-annotate.yml` (S3), **non bloquant**, borné à l'époque de
  déploiement.
- Scan `mika2201_every_match_site_is_declared`, allowlist vide.
- Quatre fixtures V1–V4, dont **une verte** (V2).
- `CLAUDE.md` : entrée nommant le lint, sa **classe A**, la doctrine « on
  déclare, on n'allowliste pas », et **le statut d'armement réel de S2**.
- V1–V9 verts, V5/V5b/V5c mesurés et leur résultat reporté dans la table
  Fire-Disposition du plan avant le merge.

---

## Acceptance criteria

Transcrites du ticket, avec leur disposition. Les rectifications sont motivées
en M0 et ne sont pas des réductions de périmètre : AC1 gagne une surface que le
ticket ne nommait pas (les prescripteurs), AC3 garde ses deux fixtures et en
inverse une.

1. **AC1 — Lint refusant les variantes non canoniques dans les callouts/corps de
   tickets.** *Ratifiée, avec son statut par surface (M2)* : bloquant sur les
   prescripteurs du dépôt (S1) et sur les corps de PR (S2) ; **annotation non
   bloquante** sur les corps de tickets (S3), parce que GitHub n'expose aucun
   gate sur une issue. Le refus porte sur la **classe B** — les jetons dont le
   lecteur est strict ; la classe A est hors périmètre **par mesure** (M0/R1).
2. **AC2 — Liste canonique = un fichier source de vérité, confronté aux sites de
   match réels.** *Ratifiée* : `scripts/canonical-tokens.tsv`, quatre colonnes,
   dont `site de match` (ce qui la rend confrontable) et `tolérance` (ce qui
   l'empêche de contredire ses lecteurs).
3. **AC3 — Anti-vacuité : fixtures « seconde passe » et « ESCALATE-divergence »
   font rougir.** *Ratifiée pour `ESCALATE-divergence` (V1, motif R2) ;
   **inversée** pour « seconde passe » (V2, motif R1)* — la fixture est livrée et
   doit rester **verte**, parce que `grooming_marker.rs` lit cette forme
   délibérément depuis mika#2158. Une fixture rouge y serait une régression, pas
   une garde. La non-vacuité est portée par V1, V3 et V4.
4. **AC4 — Tout nouveau site de match référence la liste (grep de garde).**
   *Ratifiée* : `mika2201_every_match_site_is_declared`, allowlist livrée vide,
   patron `no_grooming_regex_outside_this_module`.

---

## Hors périmètre, délibérément

- **La classe A.** Six jetons dont le lecteur est mesurément tolérant (M1). Les
  linter serait refuser ce que la machine lit.
- **Migration rétroactive des corps existants.** Interdite par le bearing
  (« coût pur contre bénéfice nul ») ; S3 est borné à son époque.
- **Resserrer `VERDICT_TOKEN_RE`** pour que `ESCALATE-divergence` cesse de
  matcher : écarté par mika#2188 avec sa raison, et ce ticket ne le rouvre pas —
  il traite la forme en amont, au lieu de rendre le verdict dépendant de
  l'orthographe d'un mot composé.
- **`post-launch` non déclaré** (M1) : réel, adjacent, **ticket de suivi**.
- **Unifier les prédicats `Branch`/`Plan` d'`auto_pull` et d'`executor`** :
  asymétrie délibérée, documentée dans `grooming_marker.rs` (« les deux lisent le
  même callout mais n'engagent pas la même dépense »).

---

## Ce que ce travail n'achète pas

Aucun compteur, aucun événement de journal. Le lint est un gate : son signal est
son propre rouge, et **son silence sur S3 ne prouve rien** — une annotation que
personne ne lit est un silence avec une ligne de plus. La seule mesure
disponible est le taux d'accusation sur S1+S2 après déploiement : **régime
attendu zéro**. Une accusation non nulle et soutenue sur un prescripteur signifie
que ce prescripteur prescrit une forme fausse depuis un moment — c'est un
résultat, pas une panne du lint. Une accusation soutenue sur les **corps de PR**
sans qu'aucun prescripteur ne rougisse signifie au contraire que la forme fautive
naît du pilote et non du prompt : **halte** — ne pas élargir le lint, établir
quel chemin l'écrit.

---

## Revision history

- **rev 2 (2026-09-20)** — révision suite au premier passage architecte
  (`Disposition: ITERATE`, findings F1–F3).
  - **F1 (bloquant) — adressée** par la nouvelle section `## Fire-Disposition` :
    six détecteurs nommés (D1–D6), chacun avec sa surface, son statut « tire-t-il
    à HEAD » et sa disposition au sens mika#1574. La forme exacte d'une exception
    est spécifiée (quatre champs obligatoires, fichier livré vide, assertion
    auto-nettoyante), et l'ordre d'implémentation D6→TSV→D5 est contraint pour ne
    pas faire passer la garde d'exhaustivité pour verte par accord avec son propre
    producteur (M6, *Author ≠ outside-check*).
  - **F2 (bloquant) — adressée** par M1.0 : le plan livre
    `scripts/canonical-tokens-survey.sh` (`--tsv`, `--check`) comme **producteur**
    du relevé, et ne revendique plus que la colonne `classe` (M1.1), seule part
    qu'une commande ne peut pas produire. Le TSV désigne un site par
    `chemin::symbole` et non par un numéro de ligne, qui pourrirait en silence.
    **La révision a validé le finding sur pièce** : le relevé manuel de rev 1
    était exact ligne à ligne mais **incomplet**, ratant
    `dispatch-lib.sh::_extract_plan_path` (ligne 6581), lecteur le plus strict du
    callout `Plan` — site désormais ajouté à la table de classe B. Le mode
    `--check` est la moitié shell du scan M5, partagée à dessein (un lecteur, pas
    deux : leçon mika#2158).
  - **F3 (sharpening) — adressée** par la scission de V5 en V5/V5b/V5c : S1 règles
    L1–L4, S1 règle L5, et **S2 sur un échantillon** (PR ouvertes + 50 dernières
    mergées). Le choix de l'échantillon est motivé (coût immédiat + tendance du
    producteur ; l'historique lointain mesurerait un producteur qui n'existe
    plus). S2 n'est **pas** déclaré structurellement exempt : la disposition D3
    conditionne son armement au résultat de V5c, avec landing en annotation si
    l'échantillon rougit — et la condition d'armement est écrite et vérifiable,
    donc elle ne peut pas devenir un désarmement permanent oublié.
  - Ajouts au *Definition of Done* correspondants (survey, fichier d'exceptions,
    statut d'armement de S2 reporté dans `CLAUDE.md`), et V6b/V7b au *Verification
    Contract*.
  - **Aucun AC affaibli.** AC1 reste bloquante sur S1 et S2 ; AC2 gagne son
    mécanisme de confrontation ; AC3 et AC4 sont inchangées.
