# mika#2855 — vol 2/2 : le lecteur joint l'issue du tour

> Vol 1/2 est **mergé** : `dc96ad5e feat(server): attester l'issue du tour webhook (mika#2855, vol 1/2) (#2859)`.
> Ce vol-ci livre ce que le vol 1 a nommément renvoyé à sa suite — **AC3, AC4, et la
> seconde moitié d'AC6** — sous la ligne `Tracked in: senara-solutions/mika#2855 (vol 2)`
> de son plan (`docs/plans/2026-10-20-003-feat-2855-vol-dinstrument-attester-lissue-du-tour-plan.md`,
> § *Ce que ce vol ne touche pas*).

## État des lieux

Le registre du tour webhook a **deux termes** depuis le vol 1, et un seul lecteur :

| terme | site | moment | ce qu'il porte |
|---|---|---|---|
| entrée | `crates/mika-agent/src/server/webhook_prefilter.rs` (`WEBHOOK_TURN_CHARGED_EVENT`) | **avant** `run_agent` | `request_id`, `agent_id`, `trigger`, `text_touched`, `candidate`, `prefilter_enabled`, `release` |
| sortie | `crates/mika-agent/src/server/webhook_turn_outcome.rs` (`WEBHOOK_TURN_OUTCOME_EVENT`) | **après** `run_agent` | `request_id`, `agent_id`, `outcome` |

Le second est émis depuis un **site unique**, `crates/mika-agent/src/server/handlers.rs:2126-2154`,
placé entre `run_agent` et le `match run` — il voit donc les deux branches, et sur
`Err` il n'existe aucun `AgentOutput`, donc aucun texte, donc `outcome_absent`. Son
enum de fil porte huit valeurs : les six termes attestables (`stop_no_open_pr`,
`stop_out_of_scope`, `stop_already_reviewed_sha`, `stop_verdict_recorded`, `review`,
`review_failed`) plus les deux termes moteur `outcome_absent` et `outcome_unreadable`,
inatteignables depuis tout texte de modèle.

**Le lecteur ne connaît pas encore le second terme.** `scripts/measure-webhook-turn-cost`
(459 lignes) narrow trois événements — `webhook_turn_charged`, `webhook_prefilter_skipped`,
`turn_usage` — et son agrégat porte cinq seaux : `by_agent_trigger`,
`by_agent_trigger_release`, `skipped_by_class`, `skipped_by_agent_class`,
`charged_by_release`. L'axe de l'issue n'y est nulle part. Son harnais
`scripts/test-measure-webhook-turn-cost.sh` (264 lignes, § 1 à § 4, 3 positifs +
10 négatifs) est atteint par CI à `.github/workflows/ci.yml:112` via
`make test-measure-webhook-turn-cost` (`Makefile:213-214`).

Et le § 10 de `docs/eval/mika-2680/README.md` porte déjà la **spécification normative**
de ce vol, écrite par le vol 1 sous le titre *« Comment il se lira, une fois le vol 2
livré »* : seau `by_agent_trigger_release_outcome`, tours sans ligne de sortie comptés
à part sans être perdus ni fusionnés, corpus gelé mêlant `state_unreadable` et un arrêt
propre. Ce plan ne redécide rien de ce qui y est écrit ; il l'implémente.

---

## Décision 1 — un quatrième bras dans le passage en flux, jamais un second passage

`webhook_turn_outcome` rejoint le `grep -F` (`scripts/measure-webhook-turn-cost:176`),
la chaîne `elif` de la projection `jq -R` gagne un bras `O\t`, l'`awk` de fan-out gagne
une cible `$WORK/outcome`, et `aggregate()` gagne un quatrième `--slurpfile`. Un second
balayage du journal serait une seconde lecture de 19 Go pour un gain nul, et surtout
deux populations lues sous deux filtres de fenêtre qu'une édition ultérieure pourrait
désynchroniser.

La projection suit la discipline du fichier à la lettre :

```jq
elif $ev == "webhook_turn_outcome" then
  ("O\t" + ({
      request_id: field($o; "request_id"),
      agent_id:   field($o; "agent_id"),
      outcome:    field($o; "outcome"),
      timestamp:  $ts
    } | tojson))
```

`field($o; $n)` et non `//`, comme partout ailleurs dans ce fichier : le motif mika#2331
est une propriété du **lecteur**, pas des seuls champs booléens qui l'ont motivé. Aucun
des trois champs de la ligne de sortie ne peut valoir `false` aujourd'hui, mais un
lecteur dont la règle d'absence dépend du type lu est un lecteur dont la correction
expire au prochain champ ajouté.

**Deux choses que ce bras ne fait pas, et dont l'omission est intentionnelle :**

1. **Il n'alimente pas `field_epoch`.** Le bras `E` reste sur `webhook_turn_charged`
   seul. L'époque du champ est la plus ancienne ligne d'**entrée** du corpus, et c'est
   la borne qui décide `window_precedes_field_epoch` — une propriété du registre, pas
   de son axe le plus récent. Une ligne de sortie qui l'abaisserait ferait croire le
   registre plus vieux qu'il n'est.
2. **Il ne conjugue pas la décision « rien mesuré ».** `exit 2` reste décidé sur
   `CHARGED_LINES -eq 0` seul (`measure-webhook-turn-cost:259`). Un corpus de lignes de
   sortie sans aucune ligne d'entrée n'a **rien mesuré** : il n'y a aucun tour à
   chiffrer, et un `exit 0` y rendrait un agrégat vide qui se lit comme une fenêtre
   sans coût. C'est la même leçon mika#2205 que pour `unparseable_lines`, et elle est
   tenue par un rouge dédié (N16).

**Le filtre `--agent` s'applique à ce bras, et c'est pour cela que `agent_id` est sur
la ligne.** `measure-webhook-turn-cost:204` —
`elif ($agent != "" and (($o.agent_id? // "") != $agent)) then empty` — est dans la
chaîne `elif` **avant** les bras par événement, donc il filtre toute ligne du corpus,
y compris celle de sortie. Un `agent_id` absent du fil ferait silencieusement tomber
chaque ligne de sortie sous le seul appel que (g′) exige, `--agent mika-qa`, et les
36 tours de la cellule se liraient tous « sans ligne de sortie ». Le champ n'est donc
pas une redondance avec la ligne d'entrée : c'est une **condition de correction du
lecteur**, et elle reçoit son propre rouge (N15).

## Décision 2 — la jointure par `request_id`, et la dernière occurrence gagne

```jq
($outcome
 | map(select(((.request_id // "") != "") and ((.outcome // "") != "")))
 | map({ key: .request_id, value: .outcome })
 | from_entries)                                                  as $by_outcome
```

Même forme que `$by_trace` juste au-dessus — `from_entries` sur des paires issues du
fil. Deux propriétés à nommer :

- **`from_entries` garde la DERNIÈRE paire sur clé dupliquée**, et c'est le bon sens de
  dégradation : il est cohérent avec le « la dernière occurrence gagne » de
  `parse_attestation` côté Rust (vol 1), où un modèle qui cite le contrat avant de
  l'honorer ne doit pas voir son intention prise pour un fait. Rien ne produit de
  doublon aujourd'hui — un site d'émission unique, gardé par
  `original_github_text.is_some()` — donc cette propriété **est nommée, pas assérée** :
  un rouge sur un doublon que rien n'écrit serait un rouge sur une fiction, et il
  figerait un détail d'implémentation de `jq` au rang de contrat.
- **La ligne inutilisable est écartée de la jointure, pas de l'agrégat.** Une ligne de
  sortie sans `request_id` n'a rien à quoi se joindre ; une ligne sans `outcome` n'a
  rien à porter. Les deux sortent de `$by_outcome` par le `select`, et **restent
  comptées** par la décision 4. Les laisser entrer avec une clé `""` ou une valeur
  `null` ferait apparaître sur l'axe de l'issue une valeur que le contrat de huit
  termes ne connaît pas.

## Décision 3 — trois absences, trois noms, et aucun ne vaut pour un autre

C'est la décision qui porte tout le vol, et le § 10 du protocole l'écrit déjà :
confondre les deux premières *« écraserait deux populations de sens opposé »*.

| nom | qui l'écrit | ce qu'il dit |
|---|---|---|
| `outcome_absent` | le **moteur** (vol 1), sur le fil | le tour a eu lieu, la ligne de sortie existe, et **le modèle n'a rien attesté** |
| `outcome_line_absent` | le **lecteur** (ce vol), dans les seaux | la jointure n'a trouvé **aucune ligne de sortie** pour ce `request_id` |
| `absent_pre_field` | le lecteur (vol 4 de mika#2680) | le champ `release` manque sur une ligne d'entrée **présente** |

`outcome_line_absent` n'est pas une issue de tour : c'est le **motif mika#2340**,
l'absence de la LIGNE comme binaire sur le binaire servi. `skills/bundled/` est lu au
**build** (`build.rs` → `BUNDLED_SKILL_MANIFESTS`), puis semé
(`seed_bundled_skills_if_needed`), puis lu au tour suivant : la ligne de contrat du
prompt n'existe pour **aucun agent** avant `make deploy`. Compter les tours d'avant le
déploiement sous `outcome_absent` écraserait « le modèle n'a pas attesté » et « le
binaire servi ne sait pas le demander » — deux faits de signe opposé, dont l'un est une
mesure et l'autre un état de déploiement.

Sur la ligne du tour (`$turns`), le champ vaut donc :

```jq
outcome: ($by_outcome[$c.request_id] // null)
```

`null`, jamais `"none"`, jamais `"outcome_line_absent"` : le nommage du repli est une
affaire de **seau**, où il doit se lire à côté de ses pairs ; sur la ligne per-tour, le
`null` de JSON dit exactement « ce tour n'a pas de valeur sur cet axe ». Ici `//` est
sûr — `$by_outcome` ne porte que des chaînes non vides, par construction du `select` de
la décision 2 — et le fichier l'autorise explicitement dans ce cas
(`measure-webhook-turn-cost:430-433`).

## Décision 4 — trois compteurs de ligne, parce que « rien trouvé » et « rien à trouver » rendent les mêmes octets

L'axe de l'issue est une taxonomie de **tours**. Une ligne de sortie malformée ou
orpheline n'est pas un tour, et la poser sur cet axe y introduirait une valeur hors
contrat. Mais ne pas la compter du tout rendrait une jointure cassée indistinguable
d'une jointure qui n'avait rien à joindre — classe mika#2205, la même que les deux codes
de sortie du § 6 du protocole. Trois compteurs au niveau de la **ligne**, dans le
`summary`, à côté de `usage_lines` et `unparseable_lines` :

| clé | ce qu'elle compte |
|---|---|
| `outcome_lines` | lignes `webhook_turn_outcome` retenues par la fenêtre et `--agent` |
| `outcome_lines_unusable` | parmi elles, celles sans `request_id` **ou** sans `outcome` |
| `outcome_lines_orphan` | parmi les utilisables, celles dont le `request_id` n'a aucun tour payé |

**Un compte suffit, et aucune valeur de coût ne l'accompagne** — à l'inverse du seau
`unattributed`, qui porte `input_tokens`, `cache_read_tokens` et `llm_calls` parce que
les `turn_usage` orphelins portent du coût **réel** que le § 4 du protocole exige de
rendre *« visible et hors tableau »*. Une ligne de sortie orpheline ne porte aucun
token : lui en prêter un ferait entrer dans le tableau un coût que personne n'a payé.
Le compteur n'a donc pas besoin de changer de nom sur fenêtre indéterminée comme
`unattributed` le fait — il ne porte aucun chiffre qu'un lecteur pourrait prendre pour
une mesure de coût.

## Décision 5 — `by_agent_trigger_release_outcome` : quatre clés, deux replis, l'unité est le token

Le seau nommé par AC4, raffinement d'un cran de `by_agent_trigger_release` exactement
comme celui-ci raffinait `by_agent_trigger` :

```jq
by_agent_trigger_release_outcome:
  ($turns
   | group_by([(.agent_id // "null"), (.trigger // "null"),
               (.release  // "absent_pre_field"),
               (.outcome  // "outcome_line_absent")])
   | map({ agent_id:          .[0].agent_id,
           trigger:           .[0].trigger,
           release:           (.[0].release // "absent_pre_field"),
           outcome:           (.[0].outcome // "outcome_line_absent"),
           turns:             length,
           input_tokens:      sum_num(.input_tokens),
           cache_read_tokens: sum_num(.cache_read_tokens),
           llm_calls:         sum_num(.llm_calls) })
   | sort_by(- .input_tokens)),
```

Trois propriétés, chacune héritée d'une leçon déjà payée :

- **Le repli est dans la clé de groupement ET dans la projection.** Leçon N8 du § 4 :
  dans la clé seule, deux tours sans ligne de sortie grouperaient bien ensemble et la
  projection rendrait `outcome: null` — le bon compte sous un nom faux ; dans la
  projection seule, deux populations se mélangeraient. Les quatre clés portent donc
  **trois replis différents** : `"null"` pour `agent_id` et `trigger` (une ligne sans
  agent), `absent_pre_field` pour `release` (un binaire d'avant le vol 4),
  `outcome_line_absent` pour `outcome` (un binaire d'avant ce registre-ci). Un repli
  partagé les confondrait, et ce sont trois absences de nature différente.
- **L'unité est le TOKEN, pas le tour.** Les quatre mêmes valeurs de coût que le voisin,
  dans son ordre, et `sort_by(- .input_tokens)` comme lui. Tout ce ticket chiffre en
  tokens d'entrée (10 687 327 sur la cellule de (g)), et copier le `value: length` du
  seau plat rendrait un agrégat qui a l'air complet, qui passe toute assertion de
  comptes, et sur lequel (g′) reste indécidable : deux cellules à un tour chacune
  peuvent valoir 900 tokens et 120.
- **Il ne remplace rien.** Les cinq seaux antérieurs ne bougent pas d'un octet — c'est
  la règle N9 du § 4, *croiser n'est pas remplacer*, et elle reçoit son rouge (N13).

## Décision 6 — `charged_by_outcome`, seau plat — et ce n'est pas une redondance du croisé

```jq
charged_by_outcome:
  ($turns | group_by(.outcome // "outcome_line_absent")
          | map({ key:   (.[0].outcome // "outcome_line_absent"),
                  value: length })
          | from_entries),
```

Le § 4 du protocole énonce la distinction que ce seau respecte : *« un seau plat répond
à une question de fenêtre, un seau croisé répond à une question de décision, qui se pose
toujours au grain d'une cellule »*. Et il existe sur cet axe une question de fenêtre
nommée, dont le § 10 dit qu'elle décide une action :

> **Le taux d'`outcome_absent` est ce qui décidera s'il faut armer la garde
> `required_suffix_lines` (EndTurn #864)**, délibérément non armée par le vol 1 parce
> qu'une relance est un appel LLM de plus sur le tour même que l'instrument mesure.

Ce taux est un agrégat sur **toute la fenêtre**, pas sur une cellule : la question est
« ce prompt obtient-il l'attestation qu'il demande », et elle ne dépend ni du
déclencheur ni du terme de relâche. Le lire en sommant le seau croisé à la main serait
exactement la pièce rapportée humaine que le registre existe pour supprimer. Et le seau
plat porte un **compte de tours**, pas des tokens — c'est le bon chiffre ici : une
désobéissance du modèle se compte en tours, non en coût.

Par construction il sépare `outcome_absent` de `outcome_line_absent` : deux clés
distinctes, jamais sommables, et c'est le rouge N12.

## Décision 7 — deux corpus gelés, et un troisième pour le code de sortie

Les fixtures vont dans un répertoire **neuf**,
`crates/mika-agent/tests/fixtures/mika2855/`, servi par une seconde variable
`FIXTURES_2855` dans le harnais. Le répertoire `mika2680/` garde ses six corpus intacts :
la provenance d'une fixture est ce qui permet de savoir quelle question elle a été
construite pour poser, et verser les corpus de ce ticket-ci dans le répertoire du
précédent la perdrait sans rien gagner.

Leçon mika#2277 — **une entrée par confusion**, jamais un corpus qui neutralise
plusieurs gardes fail-safe d'un coup. Trois confusions distinctes, trois corpus :

### `outcome_corpus.jsonl` — le croisement, et ce qui ne doit pas y entrer

Quatre tours payés, cinq lignes de sortie. Sa **propriété** est la même que celle de
`cross_tab_corpus.jsonl` : les seaux plats y sont ambigus, le croisé ne l'est pas.

| `request_id` | agent | `trigger` | `release` | `outcome` | coût (in/cache/calls) |
|---|---|---|---|---|---|
| `req-o-qa-review` | `mika-qa` | `check_suite_pr_branch` | `state_unreadable` | `review` | 900 / 150 / 3 |
| `req-o-qa-sha` | `mika-qa` | `check_suite_pr_branch` | `head_not_previously_decided` | `stop_already_reviewed_sha` | 120 / 0 / 2 |
| `req-o-qa-nopr` | `mika-qa` | `check_suite_pr_branch` | `head_not_previously_decided` | `stop_no_open_pr` | 300 / 20 / 2 |
| `req-o-dev-noline` | `mika-dev` | `check_suite_pr_branch` | `head_not_previously_decided` | *aucune ligne de sortie* | 60 / 0 / 1 |

plus deux lignes de sortie qui ne doivent **pas** atteindre l'axe de l'issue :

- une **orpheline** — `request_id: "req-o-ghost"`, `agent_id: "mika-qa"`,
  `outcome: "stop_out_of_scope"`. Sa valeur est délibérément la seule du contrat
  qu'aucun tour payé ne porte : une fuite vers l'axe est alors rouge **par présence de
  clé**, pas par écart de compte, ce qui est le détecteur le plus net.
- une **inutilisable** — sans champ `request_id`, `agent_id: "mika-qa"`,
  `outcome: "review"`.

Ce que le corpus rend ambigu, et que le croisement lève :
`charged_by_release` → `{head_not_previously_decided: 3, state_unreadable: 1}` (muet sur
l'agent et sur l'issue) ; `charged_by_outcome` → quatre seaux de 1 (muet sur le terme de
relâche). **Et il porte la réserve de Prime (AC3)** : `req-o-qa-review` est relâché par
`state_unreadable` alors que `req-o-qa-sha` est un arrêt propre — deux cellules que seul
le croisement distingue, et c'est littéralement *« un corpus mêlant `state_unreadable`
et un arrêt propre »* que le § 10 exige.

Deux tours — `req-o-qa-sha` et `req-o-qa-nopr` — ne diffèrent que par leur `outcome`.
C'est voulu : un croisement resté sur trois clés les fusionnerait en une cellule de
2 tours / 420 tokens, et ce chiffre-là est exactement celui que le seau
`by_agent_trigger_release` doit continuer de rendre (N13). Les deux assertions se
tiennent sur la même paire, de part et d'autre.

### `outcome_absent_corpus.jsonl` — les deux absences, et rien d'autre

Deux tours payés, identiques sur **les trois autres clés** (`mika-qa` ×
`check_suite_pr_branch` × `head_not_previously_decided`), de sorte que l'axe de l'issue
soit la seule variable :

| `request_id` | ligne de sortie | `outcome` attendu sur le tour |
|---|---|---|
| `req-a-attested` | présente, `outcome: "outcome_absent"` | `"outcome_absent"` |
| `req-a-noline` | absente | `null` |

`charged_by_outcome` doit y rendre `{"outcome_absent": 1, "outcome_line_absent": 1}`. Un
lecteur qui confondrait les deux rendrait `{"outcome_absent": 2}` — un chiffre qui
additionne une désobéissance de modèle et un état de déploiement.

### `outcome_only_corpus.jsonl` — l'exit code, sur sa propre entrée

Deux lignes de sortie bien formées, **aucune** ligne `webhook_turn_charged`. Le lecteur
doit rendre `exit 2`. La faute que ce corpus tient est précise et facile à commettre :
ajouter `webhook_turn_outcome` au `grep -F` puis laisser la présence de *n'importe
quelle* ligne narrow autoriser un `exit 0`. Rien ne serait mesuré, et l'agrégat vide se
lirait comme une fenêtre sans coût.

## Décision 8 — AC6 : la commande, et une troisième règle de halte

Le § 10 du protocole perd son paragraphe au futur (*« Comment il se lira, une fois le
vol 2 livré »*) et gagne la commande réelle, en deux temps : la mesure, puis la lecture
de la cellule.

```bash
scripts/measure-webhook-turn-cost \
    --log-file "$MIKA_SPIRIT_LOG_FILE" \
    --agent    mika-qa \
    --since    <borne ≥ field_epoch de la ligne de sortie> \
    --until    <fin de fenêtre, ISO-8601 UTC> \
    --summary  docs/eval/mika-2680/summary-gprime-<fenêtre>.json
```

```bash
jq -r '
  [ .by_agent_trigger_release_outcome[]
    | select(.agent_id == "mika-qa" and .trigger == "check_suite_pr_branch") ] as $c
  | ($c | map(select(.outcome != "outcome_line_absent")))                      as $att
  | { attestés:   (($att | map(.turns) | add) // 0),
      sans_ligne: ((($c - $att) | map(.turns) | add) // 0),
      par_issue:  ($att | group_by(.outcome)
                        | map({ (.[0].outcome): { tours:        (map(.turns) | add),
                                                  input_tokens: (map(.input_tokens) | add) } })
                        | add) }
' docs/eval/mika-2680/summary-gprime-<fenêtre>.json
```

C'est la *« part des arrêts par numéro sur les 36 tours de la cellule »* que l'AC6
demande, et elle sort du seul agrégat.

**La troisième règle de halte.** Le § 5 du protocole en porte deux, toutes deux *dans
l'agrégat*. Celle-ci est de la famille du § 4 — le seau dit déjà la raison de sa propre
absence, donc elle n'ajoute **aucun drapeau** :

> **Une cellule dont `outcome_line_absent` est non nul mêle deux régimes binaires.** La
> ligne de sortie n'existe pour aucun agent avant le `make deploy` qui embarque le vol 1
> (motif mika#2340) : dans cette zone, « pas de ligne de sortie » ne distingue pas un
> tour qui n'a pas attesté d'un tour servi par un binaire qui ne sait pas le demander.
> La part des arrêts n'est donc calculable que sur la **sous-population attestée**, et
> **son dénominateur doit être écrit** : `attestés`, jamais `attestés + sans_ligne`.
> *Remède : remonter `--since` au premier `webhook_turn_outcome` du journal.*

Et la sonde qui tranche le régime, déjà écrite par le vol 1 au même § 10 :
`grep -F webhook_turn_outcome "$MIKA_SPIRIT_LOG_FILE" | head`. Zéro ligne sur la fenêtre
entière veut dire « le binaire servi précède le vol 1 », pas « aucun tour n'a attesté ».

---

## Phase 1 — les trois corpus gelés

1. `crates/mika-agent/tests/fixtures/mika2855/outcome_corpus.jsonl` — 4 lignes
   `webhook_turn_charged`, leurs `turn_usage` (dont une continuation `step: 4294967295`,
   comme les corpus du vol 5), 5 lignes `webhook_turn_outcome`. Forme de fil copiée de
   `mika2680/cross_tab_corpus.jsonl` au champ près ; la ligne de sortie porte
   `"target":"mika::otel"` et le message
   `"issue du tour webhook, attestée par le modèle (mika#2855)"`.
2. `crates/mika-agent/tests/fixtures/mika2855/outcome_absent_corpus.jsonl` — 2 tours,
   1 ligne de sortie.
3. `crates/mika-agent/tests/fixtures/mika2855/outcome_only_corpus.jsonl` — 2 lignes de
   sortie, zéro ligne d'entrée.

Les corpus d'abord : les assertions de la phase 3 ne valent que si elles lisent une
entrée que personne n'a ajustée après coup pour les rendre vertes.

## Phase 2 — le lecteur

4. `grep -F` : quatrième `-e 'webhook_turn_outcome'`.
5. Projection `jq -R` : bras `O\t` de la décision 1, placé **après** le bras
   `webhook_prefilter_skipped` et **avant** le `else` de `turn_usage` — le `else` est le
   fourre-tout du dernier événement narrow et doit le rester.
6. `awk` : `-v O="$WORK/outcome"` et la règle `$1 == "O" { sub(/^O\t/, ""); print > O; next }`.
7. `: >"$WORK/outcome"` dans le bloc d'initialisation, et `--slurpfile outcome` dans
   `aggregate()`.
8. `$by_outcome` (décision 2), le champ `outcome` sur `$turns` (décision 3), les trois
   compteurs (décision 4), les deux seaux (décisions 5 et 6).
9. Les commentaires de bloc, au débit du fichier (il tourne à ~2:1 commentaire:code) :
   chaque décision ci-dessus se relit sur le site qu'elle gouverne, parce que c'est là
   qu'une édition future la rencontrera.

Mise à jour du bloc d'en-tête du script (lignes 2-95, servi tel quel par `--help`) : le
quatrième événement lu, et les trois noms d'absence de la décision 3.

## Phase 3 — le § 5 du harnais

10. `FIXTURES_2855` et les trois variables de corpus. Le scratch reste
    `$REPO_ROOT/target/mika2680-test` — une seule suite, un seul répertoire de travail.
11. Un positif et six négatifs :

| id | ce qu'il tient | corpus |
|---|---|---|
| **P4** | le croisement à quatre clés est lisible, et **`state_unreadable` se distingue d'un arrêt propre** (AC3) : deux cellules de même agent × déclencheur, l'une relâchée par panne de lecture et l'autre proprement, à 900 et 120 tokens pour 1 tour chacune — un croisement qui ne porterait que `turns` est rouge ici | `outcome` |
| **N11** | **quatre clés, pas trois** : `req-o-qa-sha` et `req-o-qa-nopr` ne diffèrent que par l'issue ; un croisement resté sur `[agent, trigger, release]` les fusionne en une cellule de 2 tours / 420 tokens | `outcome` |
| **N12** | **`outcome_absent` ≠ `outcome_line_absent`**, et le repli est dans la clé **et** dans la projection : `charged_by_outcome` porte les deux clés à 1, la cellule nommée existe, aucune cellule de l'axe ne vaut `null`, et la ligne per-tour rend `"outcome_absent"` contre `null` | `outcome_absent` |
| **N13** | **croiser n'est pas remplacer** : `by_agent_trigger`, `by_agent_trigger_release` et `charged_by_release` rendent, sur le corpus neuf, exactement ce qu'ils rendaient sans l'axe de l'issue — dont la cellule de 2 tours / 420 que N11 refuse de voir sur le croisé | `outcome` |
| **N14** | les trois compteurs de ligne, **et l'étanchéité de l'axe** : `outcome_lines: 5`, `outcome_lines_unusable: 1`, `outcome_lines_orphan: 1`, et `charged_by_outcome` n'a **aucune** clé `stop_out_of_scope` — la valeur que seule l'orpheline porte | `outcome` |
| **N15** | **`agent_id` sur la ligne de sortie est une condition de correction** : sous `--agent mika-qa`, les trois cellules attestées gardent leur issue. Un émetteur sans ce champ ferait tomber chaque ligne de sortie au filtre `:204`, et les trois se liraient `outcome_line_absent` | `outcome` |
| **N16** | un corpus de **lignes de sortie seules** sort 2 : une ligne de sortie n'est pas une ligne d'entrée, et `exit 2` reste décidé sur `webhook_turn_charged` seul | `outcome_only` |

12. L'en-tête du harnais : sa discipline annoncée passe de « 3 positifs + 10 négatifs »
    à « 4 positifs + 16 négatifs », et le texte d'aide de `Makefile:213-214` avec elle.
    Un compte annoncé qui ne correspond plus est la première chose qu'un relecteur croit.

N13 ne re-pine pas la moitié écartée : `outcome_corpus.jsonl` ne porte aucune ligne
`webhook_prefilter_skipped`, et N3 / N10 du § 4 tiennent déjà `skipped_by_class` et
`skipped_by_agent_class` sur leurs propres corpus gelés, qui continuent de tourner dans
la même exécution.

## Phase 4 — le protocole

13. `docs/eval/mika-2680/README.md` § 10 : la sous-section au futur devient la commande
    de la décision 8, suivie du `jq` de lecture et de la troisième règle de halte. Le
    tableau des huit valeurs et le contrôle positif de la sonde restent tels quels.
14. `docs/eval/mika-2680/README.md` § 4 : deux lignes au tableau des moitiés —
    `by_agent_trigger_release_outcome` et `charged_by_outcome` — et le paragraphe du
    seau d'absence gagne la distinction des trois noms de la décision 3.
15. `crates/mika-agent/CLAUDE.md`, § HTTP Server : une phrase au paragraphe du registre
    à deux termes, disant que le lecteur joint désormais la sortie par `request_id` et
    que les deux absences ne se somment pas.

Hors compte de lignes : `docs/` et `CLAUDE.md` (règle mika#2636).

---

## Ce que ce vol ne touche pas

- **L'émetteur et son contrat.** `crates/mika-agent/src/server/webhook_turn_outcome.rs`,
  ses sept tests, le site d'émission de `handlers.rs`, le `system_prompt.md` de
  `qa-review-webhook-success` et son test de parité prompt ↔ enum : livrés et mergés par
  le vol 1. Ce vol **lit** le fil ; il ne change pas ce qui y est écrit. Si une valeur
  manquait au contrat, ce serait un troisième vol, pas une retouche de celui-ci.
- **La garde `required_suffix_lines` (EndTurn #864).** Délibérément non armée. Ce vol
  rend lisible le taux d'`outcome_absent` qui décidera de l'armer — c'est tout ce qu'il
  en fait, et l'armer ici serait décider avant de mesurer.
- **Le pré-filtre.** Aucune classe (a)–(h) ne bouge, aucun terme de `Release` n'est
  ajouté ni renommé, aucun tour n'est écarté (AC5, déjà tenu par le vol 1 et non
  régressé ici puisque ce vol ne touche aucun fichier Rust).
- **Le filtre de (g′) lui-même.** Il n'est pas dans ce ticket. La mesure sur une fenêtre
  comparable dira la taille de (g′) ; ensuite seulement se posent les trois arbitrages
  nommés sur #2680 — le numéro de PR sous le PAT du relecteur, les deux sites de vérité
  du dedup, le taux de `StateUnreadable`.
- **La mesure elle-même.** Produire `summary-gprime-<fenêtre>.json` sur un journal réel
  est un acte d'opérateur sur une fenêtre à choisir, pas un livrable de PR. Ce vol livre
  l'instrument et la procédure.

## Definition of Done

- `scripts/measure-webhook-turn-cost` narrow `webhook_turn_outcome`, le joint par
  `request_id`, et son agrégat porte `by_agent_trigger_release_outcome`,
  `charged_by_outcome`, `outcome_lines`, `outcome_lines_unusable`,
  `outcome_lines_orphan`.
- Chaque ligne de `.turns` porte un champ `outcome` : une valeur du contrat à huit
  termes, ou `null`.
- Les cinq seaux antérieurs sont inchangés, octet pour octet, sur un corpus qui les
  exerce.
- `scripts/test-measure-webhook-turn-cost.sh` porte un § 5 de 1 positif + 6 négatifs
  (P4, N11–N16) et rend 0 ; son en-tête et le texte d'aide du `Makefile` annoncent
  4 positifs + 16 négatifs.
- Les trois corpus gelés existent sous `crates/mika-agent/tests/fixtures/mika2855/`.
- `make test-measure-webhook-turn-cost` passe en local et sur CI
  (`.github/workflows/ci.yml:112`).
- `docs/eval/mika-2680/README.md` § 10 porte la commande AC6, son `jq` de lecture et la
  troisième règle de halte ; son § 4 porte les deux nouveaux seaux et les trois noms
  d'absence.
- `crates/mika-agent/CLAUDE.md` dit que le lecteur joint la sortie.
- `scripts/verify-pipeline.sh origin/main` passe ; aucune entrée ajoutée à
  `scripts/suite-wiring.tsv` (la suite est atteinte par un workflow, pas résiduelle).
- Aucun fichier sous `crates/mika-agent/src/` n'est modifié.

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2855, avec le vol qui les sert.

- **AC1** : chaque tour `qa-review-webhook-success` émet en fin de tour une ligne portant
  `request_id` et `outcome`. `outcome` vaut `stop_no_open_pr`, `stop_out_of_scope`,
  `stop_already_reviewed_sha`, `stop_verdict_recorded`, `review`, `review_failed`, ou
  `outcome_absent` (les noms sont à fixer au groom). Il y a un rouge pour chaque valeur.
  → **servi par le vol 1** (`webhook_turn_outcome.rs`, test
  `mika2855_les_six_termes_attestes_se_lisent`). Non régressé ici : ce vol ne touche
  aucun fichier Rust.
- **AC2** : l'issue est **attestée**. Un test montre qu'un tour qui n'atteste rien
  produit `outcome_absent`, et pas un arrêt deviné.
  → **servi par le vol 1** (tests `..._un_tour_qui_natteste_rien_rend_absent`,
  `..._un_tour_sans_texte_rend_absent`, `..._une_valeur_hors_contrat_nest_pas_une_absence`).
- **AC3, réserve de Prime** : un tour relâché par `Release::StateUnreadable` se distingue
  d'un arrêt propre dans la jointure. Le croisement `release` × `outcome` est lisible, et
  un test le pine avec un corpus mêlant les deux.
  → **servi par CE vol** : décisions 5 et 7, corpus `outcome_corpus.jsonl`
  (`state_unreadable` × `review` contre `head_not_previously_decided` ×
  `stop_already_reviewed_sha`), assertion **P4**.
- **AC4** : `scripts/measure-webhook-turn-cost` joint la ligne de sortie par
  `request_id`. L'agrégat gagne un seau `by_agent_trigger_release_outcome`. Les tours
  sans ligne de sortie sont comptés à part, sans être perdus ni fusionnés.
  `scripts/test-measure-webhook-turn-cost.sh` le couvre.
  → **servi par CE vol** : décisions 1 à 6 ; « comptés à part sans être perdus ni
  fusionnés » est le repli nommé `outcome_line_absent` de la décision 3, distinct
  d'`outcome_absent` **et** d'`absent_pre_field` ; couverture **P4, N11–N16**.
- **AC5** : aucun tour n'est écarté, et le comportement du skill est inchangé, hors
  l'attestation. Les classes (a) à (h) du pré-filtre restent intactes.
  → **servi par le vol 1**, et **structurellement préservé ici** : ce vol ne modifie que
  `scripts/`, `crates/mika-agent/tests/fixtures/`, `docs/` et `CLAUDE.md`. Un lecteur de
  journal ne peut pas écarter un tour.
- **AC6** : le protocole de mesure de #2680 dit comment la prochaine fenêtre chiffrera
  (g′) : la part des arrêts par numéro sur les 36 tours de la cellule.
  → **première moitié servie par le vol 1** (§ 10 de `docs/eval/mika-2680/README.md` :
  les deux termes, le contrat à huit valeurs, la sonde du binaire) ; **seconde moitié
  servie par CE vol** (décision 8 : la commande, le `jq` sur
  `by_agent_trigger_release_outcome`, et la troisième règle de halte qui nomme le
  dénominateur).

## Fire-Disposition

Ce vol livre des détecteurs : les sept assertions du § 5 de
`scripts/test-measure-webhook-turn-cost.sh`, dont le chemin de succès est « aucune
divergence trouvée ». **Option (a) — exception nommée en allowlist, et l'allowlist est
vide par construction.**

- **Aucune exception n'est requise, et aucune n'est possible.** La population que ces
  détecteurs lisent est **entièrement bâtie par ce vol** : trois corpus JSONL gelés,
  écrits dans la même PR que les assertions qui les lisent. Il n'existe aucune donnée
  préexistante qu'une entrée d'allowlist pourrait nommer. C'est une affirmation plus
  forte que « vide parce que rien ne viole » : il n'y a pas de population héritée du
  tout.
- **Les détecteurs atterrissent ARMÉS.** Aucun `#[ignore]`, aucun `#[cfg(skip)]`, aucun
  garde d'environnement : le § 5 tourne dans la même exécution que les § 1 à § 4, sous
  `make test-measure-webhook-turn-cost`, atteint par CI sur chaque PR à
  `.github/workflows/ci.yml:112`. Un échec rougit la PR qui l'a causé.
- **Le contrôle de bonne foi.** Leçon du vol 1 : *une allowlist vide sur un détecteur qui
  ne détecte rien est indiscernable d'un détecteur inerte.* Ici c'est le **positif** qui
  l'exclut. Un lecteur qui émettrait `by_agent_trigger_release_outcome: []` serait vert
  sur N11, N12, N13, N15 et N16 — et **rouge sur P4**, qui exige deux cellules nommées
  avec leurs quatre valeurs de coût, et **rouge sur N14**, qui exige
  `outcome_lines: 5`. Les deux contrôles positifs de l'axe neuf sont la preuve que le
  détecteur lit quelque chose.
- **Aucun ticket de suivi n'est ouvert par cette disposition**, puisqu'aucune exception
  n'est inscrite. Le seul report de ce vol est celui déjà nommé au § *Ce que ce vol ne
  touche pas* — l'armement de la garde EndTurn #864 — et il est conditionné à une
  **mesure**, pas à une violation : il n'a pas de Fire-Disposition à porter.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/measure-webhook-turn-cost` (bras `O`, jointure, 2 seaux, 3 compteurs, commentaires) | 90 |
| `scripts/test-measure-webhook-turn-cost.sh` (§ 5 : P4, N11–N16, en-tête) | 92 |
| `crates/mika-agent/tests/fixtures/mika2855/*.jsonl` (3 corpus) | 22 |
| `Makefile` (texte d'aide de la cible) | 2 |

Total estimé : 206 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 206 lignes, sous la borne par vol de 429 et
très loin du seuil de 1000 par PR — la plus serrée des deux mord, et elle est tenue avec
plus du double de marge.

Trois propriétés du périmètre expliquent le chiffre, et chacune porte sur les **tours**,
qui sont l'unité de la borne :

1. **Aucune ligne de Rust.** Le vol 1 a livré l'émetteur, ses sept tests inline, le site
   d'émission et le test sur le harnais de capture mika#2680. Ce vol ne touche que
   `scripts/`, des fixtures JSONL, `docs/` et `CLAUDE.md`. Il n'y a donc **aucune boucle
   compile-corrige** — exactement là où les trois morts du 2026-10-14 ont brûlé leurs
   tours (mika#2683, mika#2654, mika#2687).
2. **Tout est une extension d'une forme déjà présente.** Le bras `O` copie le bras `S`,
   `$by_outcome` copie `$by_trace`, `by_agent_trigger_release_outcome` copie
   `by_agent_trigger_release` d'une clé près, `charged_by_outcome` copie
   `charged_by_release`, et le § 5 du harnais copie la structure du § 4 jusqu'au helper
   `cellule()` (qui gagne un quatrième argument). Rien ici ne demande d'inventer une
   forme, donc rien ne demande d'itérer sur un choix de forme.
3. **La vérification est locale et rapide.** `bash scripts/test-measure-webhook-turn-cost.sh`
   lit trois corpus de quelques lignes et rend un verdict en moins d'une seconde : la
   boucle assertion → rouge → correction coûte un tour, pas une compilation de
   l'espace de travail.

La fin de pipeline — `cargo fmt`, `cargo clippy`, la suite, les lints de dépôt,
`scripts/verify-pipeline.sh origin/main`, commit, push, PR — coûte des tours et non des
lignes, et c'est le soustrahend de 30 de la dérivation qui la couvre.

## Fichiers touchés

| fichier | nature |
|---|---|
| `scripts/measure-webhook-turn-cost` | modifié — quatrième événement, jointure, deux seaux, trois compteurs |
| `scripts/test-measure-webhook-turn-cost.sh` | modifié — § 5, `FIXTURES_2855`, en-tête |
| `crates/mika-agent/tests/fixtures/mika2855/outcome_corpus.jsonl` | créé |
| `crates/mika-agent/tests/fixtures/mika2855/outcome_absent_corpus.jsonl` | créé |
| `crates/mika-agent/tests/fixtures/mika2855/outcome_only_corpus.jsonl` | créé |
| `Makefile` | modifié — texte d'aide de `test-measure-webhook-turn-cost` |
| `docs/eval/mika-2680/README.md` | modifié — § 10 (commande AC6, troisième halte), § 4 (deux seaux, trois absences) |
| `crates/mika-agent/CLAUDE.md` | modifié — une phrase au § HTTP Server |

Refs mika#2855, mika#2680, mika#2675, mika#2260, mika#2854, mika#2205, mika#2277,
mika#2331, mika#2340, EndTurn #864.
