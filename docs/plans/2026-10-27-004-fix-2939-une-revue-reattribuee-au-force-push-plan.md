# mika#2939 — une revue réattribuée au force-push ne statue pas sur la nouvelle tête

- Ticket : senara-solutions/mika#2939 (`bug`, `p1-important`, `dispatch:loop`, `loop-substrate`)
- Branche : `fix/2939/qa-une-revue-r-attribu-e-au-force-push`
- Prédécesseur direct : mika#2934 (`docs/plans/2026-10-27-002-fix-2934-letape-4-juge-la-tete-sur-les-revues-de-cette-tete-plan.md`)
- Bloque : mika#2920 (vol 3 de mika#2917), toujours en draft
- Le merge revient à Vincent (`loop-substrate`).

## Contexte

mika#2934 a retiré au modèle la capacité de juger une tête sur un état de PR, en
lui donnant l'outil `qa_head_reviews` : les revues dont `commit_id == head`, et
rien d'autre. Le handler l'écrit en toutes lettres, ligne 18 :

> A review's `commit_id` is the only field that names the head it ruled on.

**Sur GitHub, c'est faux.** Au force-push, la forge peut réattribuer une revue à
la nouvelle tête. Le champ ne nomme pas la tête sur laquelle la revue a statué ;
il nomme la tête à laquelle la forge l'**attribue aujourd'hui**, ce qui n'est pas
la même chose et ne l'est plus dès qu'un force-push est passé.

### Le cas mesuré (n=1), PR #2920

| Instant (UTC) | Fait | Source |
|---|---|---|
| 07:16:49Z | mika-qa approuve #2920. Revue `5663460195`, faite sur `f7542d43` | corps de mika#2934 |
| 12:14:25Z | `5e4918cf` est créé par un rebase sur main `967e1f1f` (date de committer) | `git log -1 --format=%cI` |
| 12:44:36Z | premier `check_suite` sur `5e4918cf` — la forge voit le SHA | `GET /commits/5e4918cf/check-suites` |
| 12:44:37Z | force-push de `f7542d43` vers `5e4918cf` | timeline, `head_ref_force_pushed` |
| 12:45:10Z | `qa_head_reviews(head_sha: 5e4918cf…)` rend `[{"state":"APPROVED","submitted_at":"2026-10-27T07:16:49Z","carries_verdict_line":true}]` | `tool_calls` |
| 12:45:13Z | `record_reviewed_head` écrit `stop_verdict_recorded` sur `pr:senara-solutions/mika#2920@5e4918cf…` | `audit_events` |

La revue `5663460195` porte aujourd'hui `"commit_id": "5e4918cf992d576e1098015c12fb50d9d94de6cc"` et
`"submitted_at": "2026-10-27T07:16:49Z"` : **elle est datée de cinq heures avant la
création du commit qu'elle est censée juger.** Aucune revue de mika-qa n'a été
faite sur `5e4918cf`.

### Ce que la garde de mika#2934 n'a pas pu voir

Le tour a lu les revues de la tête, avec succès, avec la bonne tête :
`preuve_du_tour_manquante` a rendu `None`, **à juste titre**. Le prompt a appliqué
l'étape 4 à la lettre. C'est la donnée qui est fausse, pas la lecture. Aucune
règle n'a été enfreinte : le défaut est dans la **définition de la preuve**, et
c'est là qu'il se corrige.

Trois conséquences, les trois du ticket :

1. la garde de mika#2934 est contournée sans qu'aucune règle soit enfreinte ;
2. la sonde temporelle d'AC4/AC5 de mika#2934 passe à tort — elle compare
   `created_at` (12:45:13Z) à la première revue dont `commit_id == head`, soit
   07:16:49Z, et ne trouve donc rien ;
3. mika#2920 est le lecteur : toute PR rebasée sans changement après une
   approbation serait écartée du pré-filtre sur la foi d'une revue faite contre
   une autre base.

## Ce qui est refusé (bearing, non négociable)

Hérité de mika#2917, mika#2927 et mika#2934, intact :

- **Pas de table, pas de migration, pas de DDL.** Le registre reste des lignes
  `audit_events` (`tool_name = "qa_reviewed_head"`).
- **Écrivain unique.** `crates/mika-agent/src/tools/record_reviewed_head.rs`
  reste le seul site qui écrit.
- **Vocabulaire fermé de trois valeurs.** `REVIEWED_HEAD_OUTCOMES` ne change pas.
- **Les six valeurs de `Webhook-Outcome:` (mika#2855) ne bougent pas.**
  `stop_verdict_recorded` reste une issue possible de l'étape 4 : ce plan change
  la **preuve** qu'elle exige, pas son existence — exactement comme mika#2934.
- **Aucune suppression de ligne en base.** Voir § *Le legs*.
- **`record_reviewed_head` ne fait aucun appel réseau.** Le filtre temporel vit
  dans `qa_head_reviews`, qui est déjà le site qui parle à la forge.
- **Hors périmètre, nommé :** le lecteur de mika#2920 et son interrupteur
  `MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD` ; la question de produit « une revue de
  patch équivalent vaut-elle pour la nouvelle base ? ». Ce plan refuse seulement
  qu'un champ réattribué par la forge en décide **en silence**.

## Mesurer la règle de réattribution, ou s'en passer — on s'en passe, avec un instrument

Le ticket laisse le choix : « Avec un seul cas de chaque côté, je ne conclus pas
sur la règle exacte que GitHub applique. Le plan devra la mesurer, ou s'en
passer. »

**Ce plan s'en passe, et c'est un choix argumenté.** Le filtre livré est
**purement temporel** : il refuse une revue soumise avant l'arrivée de la tête,
quelle que soit la raison pour laquelle la forge la lui a attribuée. Sa
correction ne dépend donc d'aucune hypothèse sur la règle de réattribution — s'en
passer n'est pas un report, c'est une indépendance.

Mesurer la règle coûterait des force-push expérimentaux sur une PR réelle, pour
une caractérisation d'un comportement non documenté que GitHub peut changer sans
préavis : une mesure dont la péremption n'est pas sous notre contrôle, et sur
laquelle ce ticket ne doit surtout pas s'appuyer.

**Mais l'inconnue devient mesurable.** Le handler rend un compteur
`dropped_predating_head` : le nombre de revues que le filtre a écartées. Cette
valeur part dans `tool_calls.output` à chaque tour, donc la population
« réattributions observées » s'accumule d'elle-même, et une question future
(« à quelle fréquence, sur quelle forme de rebase ? ») se répondra sur des
données au lieu de deux cas. On ne devine pas la règle ; on instrumente son
effet.

## AC1 — le repère : `head_first_seen_at = MIN(check_suites[].created_at)`

Le ticket nomme trois candidats et en refuse un quatrième. Les trois ont été
mesurés sur `#2920@5e4918cf` avant d'en choisir un.

| candidat | mesure | verdict |
|---|---|---|
| date de committer (`git log -1 --format=%cI`) | `2026-10-27T12:14:25Z` | **refusé par le ticket** — fixée localement, donc falsifiable et sans rapport avec l'arrivée sur la PR |
| timeline `committed` | ne porte que `committer.date`, soit la même valeur locale | **refusé** — même défaut, un emballage de plus |
| timeline `head_ref_force_pushed` | porte `commit_id` + `created_at` serveur : `f7542d43` @07:05:28Z, `5e4918cf` @12:44:37Z | **rejeté** — voir ci-dessous |
| `MIN(check_suites[].created_at)` sur le SHA | `2026-10-27T12:44:36Z` (5 suites) | **retenu** |

### Pourquoi le check_suite et pas la timeline

`head_ref_force_pushed` est correct quand il existe, et son `created_at` est bien
serveur. Trois raisons le font perdre :

1. **Il est absent d'un push ordinaire.** Une tête arrivée par push non forcé —
   le cas majoritaire — ne produit aucun `head_ref_force_pushed`. Le repère
   n'existerait alors pas, et un repère qui manque sur la population normale
   force à inventer un défaut, c'est-à-dire à rendre le filtre inerte là où il
   doit mordre.
2. **Il coûte une pagination.** La timeline d'une PR vivante compte des dizaines
   d'événements, et le force-push cherché peut être arbitrairement loin.
3. **Il répond à la mauvaise question.** « Quand cette tête est-elle arrivée sur
   cette PR ? » est plus faible que ce qu'on veut savoir : **« cette revue a-t-elle
   pu voir ce SHA ? »** Une revue soumise avant que la forge ait vu le SHA n'a pas
   pu statuer dessus, quel que soit le chemin par lequel le SHA est ensuite
   devenu la tête.

À l'inverse, `MIN(check_suites[].created_at)` :

- est **serveur**, donc hors de portée d'une horloge locale (le refus du ticket) ;
- **existe par construction** : le tour est déclenché *par* un `check_suite` sur
  cette tête, donc il y a au moins une suite, donc le `MIN` est défini ;
- est **sémantiquement exact** : la première suite est le premier instant où la
  forge atteste l'existence du SHA.

Une seule lecture, déjà vérifiée en session :

```bash
gh api "repos/$REPO/commits/$HEAD_SHA/check-suites?per_page=100" \
  --jq '[.check_suites[].created_at] | min // empty'
# sur 5e4918cf992d576e1098015c12fb50d9d94de6cc → 2026-10-27T12:44:36Z
```

### Comparaison et dégradations, nommées

- **Prédicat :** `submitted_at >= head_first_seen_at`, **inclusif**. Une revue
  soumise dans la même seconde que la première suite est conservée : le repère
  borne ce que la revue a pu voir, il ne départage pas des sous-secondes.
- **Comparaison lexicographique**, et c'est correct : les deux côtés sont de l'ISO
  8601 UTC à largeur fixe suffixé `Z`, comme toute la base (§ *Timestamps* de
  `crates/mika-agent/CLAUDE.md`).
- **Plus de 100 suites :** le `MIN` porte sur un sous-ensemble, donc il est
  **≥** le vrai `MIN`, donc le repère est **plus tardif**, donc le filtre est
  **plus strict** — il écarte au plus des revues légitimes, et une revue écartée
  à tort coûte une revue de plus, jamais une revue manquée. La dégradation est
  fail-safe, et le handler ne pagine pas pour cette raison précise.
- **Liste de suites vide, champ absent, date malformée :** **ERROR, jamais un
  filtre neutre et jamais un tableau vide.** Un tableau vide est la réponse
  « aucune revue sur cette tête » ; confondre les deux est exactement la classe
  mika#2205, et le handler refuse déjà cette confusion pour ses trois entrées.
- **Le prompt ne refait pas le filtre.** AC1 l'exige : l'outil l'applique
  lui-même. Le modèle ne reçoit jamais la revue écartée, donc il ne peut pas la
  réintroduire.

## Les livrables

### L1 — `qa_head_reviews.sh` rend un objet, et applique le filtre

Le handler cesse de rendre un tableau nu. Nouvelle forme de sortie :

```json
{
  "head_sha": "5e4918cf992d576e1098015c12fb50d9d94de6cc",
  "head_first_seen_at": "2026-10-27T12:44:36Z",
  "reviews": [ { "author": …, "state": …, "submitted_at": …, "carries_verdict_line": … } ],
  "dropped_predating_head": 1
}
```

Quatre champs, et chacun est nécessaire : `reviews` est la preuve, `head_sha` et
`head_first_seen_at` rendent la décision **lisible dans la trace** (sans eux,
`reviews: []` ne dit pas *pourquoi* il est vide), `dropped_predating_head` est
l'instrument du § précédent.

Séquence, dans cet ordre :

1. validation inchangée de `repo` / `pr` / `head_sha` **avant** la ligne de
   commande (injection d'option, traversée de chemin) — les trois regex restent
   telles quelles ;
2. `head_first_seen_at` par l'appel `check-suites` ci-dessus. Vide ou non
   conforme à `^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$` ⇒
   `ERROR` sur stderr, `exit 1` ;
3. l'appel `pulls/$PR/reviews --paginate` inchangé, puis le `jq -s` final qui
   reçoit **deux** `--arg` (`head_sha`, `first_seen`) et construit l'objet. Les
   deux valeurs arrivent à `jq` en **données**, jamais en texte de programme
   interpolé — la propriété que le handler tient déjà.

Forme du `jq` final (le filtre partitionne, il ne sélectionne pas : les deux
moitiés sont comptées) :

```jq
[ .[] | select(.commit_id == $head_sha) ] as $sur_la_tete
| ($sur_la_tete | map(select((.submitted_at // "") >= $first_seen))) as $gardees
| {
    head_sha: $head_sha,
    head_first_seen_at: $first_seen,
    reviews: ($gardees | map({ author: (.user.login // ""), state: (.state // ""),
                               submitted_at: (.submitted_at // ""),
                               carries_verdict_line: ((.body // "") | test("VERDICT:[ \t*]*(pass|block)")) })),
    dropped_predating_head: (($sur_la_tete | length) - ($gardees | length))
  }
```

Le commentaire d'en-tête est corrigé — c'est lui qui portait la fausse
affirmation. La phrase ligne 18 devient :

> `commit_id` names the head the forge attributes a review to **today**, which a
> force-push can rewrite (mika#2939, measured on #2920: a review submitted at
> 07:16:49Z carries a `commit_id` created at 12:14Z). The head a review *ruled
> on* is therefore `commit_id` **and** a submission not predating the head.

Le corps de la revue reste non rendu, et la doctrine du fichier reste
*remove capability, don't prohibit it* : le modèle ne reçoit pas la revue écartée.

### L2 — `tools.json`

La `description` de `qa_head_reviews` décrit le nouvel objet : les quatre champs,
le filtre temporel et son repère, le fait qu'un champ absent est une erreur et
jamais un tableau vide, et — explicitement — que `dropped_predating_head` est un
**instrument** et non une raison de sauter. Le schéma d'entrée est **inchangé**
(`repo`, `pr`, `head_sha`, les trois requis) : c'est ce qui laisse intacte la
garde de mika#2934, voir L5.

### L3 — `system_prompt.md`, étapes 3 et 4

Trois amendements, aucune étape nouvelle, aucune issue nouvelle.

**Étape 3** — « It returns only the reviews whose `commit_id` is exactly this
head … **Keep that array** » devient une lecture de l'objet : le champ `reviews`
porte les revues de cette tête **qui ont pu la voir** ; un `reviews` vide veut
dire qu'aucune revue n'a jamais statué sur elle, et c'est une réponse, pas un
échec. « Keep that object : steps 3 and 4 both read its `reviews` field. »

**Étape 4** — « In the **same array already in hand** » devient « in the
`reviews` field of the object already in hand ». Le refus des états de PR est
conservé mot pour mot — il tient le défaut 1 de mika#2934 et son détecteur
dépend de sa lettre. La phrase finale de ce bullet, qui portait la fausse
affirmation du handler, est corrigée : `commit_id` **seul** ne nomme pas la tête
jugée, parce qu'un force-push peut le réécrire ; ce qui la nomme est `commit_id`
**plus** une soumission postérieure à l'arrivée de la tête, et c'est
`qa_head_reviews` qui applique les deux.

**Une ligne nouvelle, et une seule** — sous l'étape 4 :

> **`dropped_predating_head` is an instrument, never a reason to skip.** It counts
> the reviews the tool refused because they predate this head. A non-zero count
> is the force-push case (mika#2939): it means the evidence is *absent*, so you
> do **not** skip — you review.

Elle est nécessaire : sans elle, un modèle qui voit `dropped_predating_head: 1`
peut relire ce compteur comme « il y a bien une revue » et re-dériver le défaut
exact que le filtre vient de supprimer.

Les tableaux des six `Webhook-Outcome:` (lignes 49–58) et du
`record_reviewed_head` (lignes 71–78) sont **inchangés**.

### L4 — `scripts/test-qa-head-reviews-handler.sh` (AC2)

Première suite de tests d'un handler de skill bundlé. Idiome copié de
`scripts/test-verify-dependabot-topology.sh` : `TMPROOT=$(mktemp -d)` +
`trap cleanup EXIT`, un faux `gh` en tête de `PATH`, aiguillage par slug de
fixture, compteurs `PASS`/`FAIL`, et sa doctrine en en-tête — *« Supprime ce que
le test protège ; vérifie qu'il rougit. »*

Le faux `gh` sert **deux** chemins, `pulls/*/reviews` et
`commits/*/check-suites`, parce que le handler en appelle deux désormais.

Pourquoi un faux `gh` plutôt qu'une mesure : la propriété s'étend sur **deux
états du monde** (avant et après le force-push), donc elle n'est pas
établissable une fois à la main ; et elle n'est pas établissable depuis un bac à
sable de pilote, qui n'a ni jeton ni egress arbitraire. C'est le raisonnement que
porte déjà l'en-tête de la suite dependabot.

Cas, tous dans le même fichier :

| cas | fixture | attendu |
|---|---|---|
| **le cas #2920** | revue `5663460195`, `APPROVED`, `commit_id = 5e4918cf…`, `submitted_at = 2026-10-27T07:16:49Z` ; suites ⇒ `first_seen = 2026-10-27T12:44:36Z` | `reviews == []`, `dropped_predating_head == 1`, `head_first_seen_at` rendu |
| **contrôle positif** (même test, AC2 l'exige) | revue `5667360863`, `submitted_at = 2026-10-27T12:55:57Z`, même `commit_id` | la revue est **rendue**, `dropped_predating_head == 0` |
| limite inclusive | `submitted_at == first_seen` | rendue (le `>=` est délibéré) |
| autre tête | `commit_id != head_sha` | ni dans `reviews`, ni dans `dropped_predating_head` — le compteur ne compte que les revues de *cette* tête |
| aucune suite | `check_suites: []` | `exit 1`, `ERROR` sur stderr, **rien** sur stdout |
| `gh` en échec sur les suites | slug `.err` | `exit 1`, pas de sortie partielle |
| entrées invalides | `repo` avec `../`, `pr` non numérique, `head_sha` tronqué | `exit 1`, les trois messages inchangés |
| contrôle de rougeur | le même cas #2920 passé à une copie du handler **privée du terme `>= $first_seen`** | le test **échoue** — c'est ce qui prouve qu'il teste le filtre et pas la forme de l'objet |

Le dernier cas est ce qui sépare un test du filtre d'un test de sérialisation : la
copie désarmée est écrite par le test lui-même sous `$TMPROOT` (un `sed` sur le
handler lu depuis l'arbre), jamais commitée.

### L5 — câblage CI (`.github/workflows/ci.yml`)

Une étape `run: bash scripts/test-qa-head-reviews-handler.sh`, dans le job des
suites shell (voisines : `smoke-webhook-chain`, `verify-dependabot-topology`),
précédée du commentaire d'usage nommant l'invariant et son ticket.

**Et donc aucune entrée dans `scripts/suite-wiring.tsv`.** La comparaison du lint
est bidirectionnelle : une entrée pour une suite que la CI atteint **fait rougir**
`suite-wiring-lint`. La preuve de câblage est le workflow qui la nomme, jamais une
ligne de registre (mika#2761).

### L6 — `crates/mika-agent/src/qa_reviewed_head.rs`

**Aucun changement de production n'est nécessaire, et c'est une propriété, pas un
oubli.** `lecture_de_tete_dans_le_tour` reconnaît la lecture sur
`ToolCallSummary.input_summary` — l'**entrée** de l'outil — et le schéma d'entrée
ne change pas (L2). Le site de refus de mika#2934 continue donc de fonctionner
intact, sans une ligne touchée. La preuve du tour reste « la tête a été lue par
l'outil » ; ce ticket change ce que l'outil répond, pas la façon dont le moteur
atteste qu'on l'a interrogé.

Côté détecteurs, deux clauses s'ajoutent à l'existant :

- `CLAUSES_DE_LA_TETE_JUGEE` gagne `"dropped_predating_head"` et
  `"head_first_seen_at"`. Les deux sont nécessaires et aucune ne couvre l'autre :
  la première est la ligne « instrument, jamais une raison de sauter » sans
  laquelle le modèle re-dérive le défaut depuis le compteur ; la seconde est le
  repère, dont l'absence du prompt signifierait que l'étape 3 décrit encore une
  lecture non filtrée. Les trois clauses existantes restent — `commit_id` en
  particulier, parce que le prédicat en a toujours besoin, il n'est simplement
  plus suffisant.
- le contrôle négatif du test `mika2934_letape_4_nomme_la_lecture_et_son_outil`
  est étendu : sa fabrique (le prompt d'avant mika#2934) ne porte évidemment
  aucune des deux nouvelles clauses, donc le `assert_eq!` sur les absences
  attendues passe de deux à quatre éléments. Sans cette extension le contrôle
  négatif deviendrait faux et le test rougirait **pour la mauvaise raison**.

`ETATS_DE_PR_REFUSES_A_LETAPE_4` et
`mika2934_le_prompt_du_webhook_ne_cite_plus_aucun_etat_de_pr_revocable` sont
inchangés : le prompt ne regagne aucun état de PR.
`CONFORMITE_TETE_JUGEE_ALLOWLIST` reste `&[]` — voir § *Fire-Disposition*.

## AC3 — la sonde temporelle corrigée

La Lecture 2 du plan mika#2934 calcule le premier verdict sur une tête ainsi :

```bash
first=$(gh api "repos/$repo/pulls/$pr/reviews" --paginate \
  --jq "[.[] | select(.user.login == \"mika-platform-qa\" and .commit_id == \"$sha\") | .submitted_at] | sort | first // empty")
```

Elle est existentiellement correcte et **temporellement aveugle** : c'est
exactement le défaut de ce ticket, dans l'instrument au lieu du moteur.
Correction — un terme de plus, et un appel de plus :

```bash
first_seen=$(gh api "repos/$repo/commits/$sha/check-suites?per_page=100" \
  --jq '[.check_suites[].created_at] | min // empty')
[ -n "$first_seen" ] || { echo "INDÉTERMINÉ $repo#$pr@$sha — aucune check_suite"; continue; }
first=$(gh api "repos/$repo/pulls/$pr/reviews" --paginate \
  --jq "[.[] | select(.user.login == \"mika-platform-qa\" and .commit_id == \"$sha\" and .submitted_at >= \"$first_seen\") | .submitted_at] | sort | first // empty")
```

Trois propriétés voulues :

- le prédicat de mika#2934 est **conservé** (`user.login` + `commit_id`) et
  seulement **resserré** par `>= first_seen`. C'est ce resserrement, et lui seul,
  qui rend la ligne de #2920 en violation ;
- un `first_seen` absent rend **`INDÉTERMINÉ`**, jamais « pas de violation » : une
  mesure qui ne peut pas conclure doit le dire (mika#2331 — absent ≠ zéro) ;
- la Lecture 4 est inchangée :
  `grep -c 'reviewed_head_write_without_evidence' ~/.mika/logs/mika-spirit.log`.

**Vérification à la main, faite avant d'écrire ce plan.** Sur
`#2920@5e4918cf` : `first_seen = 12:44:36Z`, la revue de 07:16:49Z est écartée, le
premier verdict retenu devient **12:55:57Z** (revue `5667360863`), et la ligne
`stop_verdict_recorded` écrite à **12:45:13Z** lui est donc antérieure. La lecture
de bonne foi la rend `VIOLATION avant-verdict` — ce qu'AC3 exige. La Lecture 3
non bornée du plan mika#2934 rendait quatre lignes ; elle en rendra **cinq**.

## AC4 — la sonde post-déploiement

À lancer après `make deploy` (mika#2340 : la correction du prompt et du handler
n'atteint aucun agent avant le rebuild, et sans lui la sonde mesure l'ancien
binaire). `$DEPLOY_AT` est l'instant du déploiement, en ISO 8601 UTC.

**Lecture 0 — le binaire servi porte bien la correction.**

```bash
cat ~/.mika/skills/.manifest-writer     # git_hash == le SHA déployé
grep -c 'dropped_predating_head' ~/.mika/skills/qa-review-webhook-success/system_prompt.md   # ≥ 1
```

Sans elle, les lectures suivantes sont vertes pour la mauvaise raison : rien ne
distinguerait « aucune violation » de « l'ancien prompt tourne encore ».

**Lecture 1 — population.** Au moins une PR rebasée sans changement après une
approbation, c'est-à-dire : un `head_ref_force_pushed` postérieur à une revue
`APPROVED` de `mika-platform-qa`, et postérieur à `$DEPLOY_AT`. Si la population
est vide, la sonde est **INDÉTERMINÉE** et doit être rejouée — une sonde sans
sujet ne dit rien, et ne doit pas pouvoir se lire comme un succès. Le cas se
provoque au besoin : rebase sans changement d'une PR approuvée de la boucle.

**Lecture 2 — AC4 à la lettre.** Pour chaque tête de cette population, aucune
ligne `stop_*` sur `pr:{repo}#{n}@{sha}` dans `audit_events`
(`tool_name = 'qa_reviewed_head'`, `created_at >= $DEPLOY_AT`) ne précède une
revue soumise après le push de la tête. Même forme que la Lecture 2 corrigée
d'AC3, bornée par `$DEPLOY_AT`, et portant sur **les deux** valeurs `stop_*` —
`stop_already_reviewed_sha` comme `stop_verdict_recorded` — parce que les deux
déclarent une tête conclue.

**Lecture 3 — contrôle positif, sans lequel la sonde est muette.** La boucle
tourne : sur la même fenêtre, au moins une ligne `qa_reviewed_head` écrite, ou au
moins un appel `qa_head_reviews` réussi dans `tool_calls`. Zéro partout veut dire
que rien n'a été mesuré, pas que tout va bien.

**Lecture 4 — l'instrument.** Distribution de `dropped_predating_head` sur les
sorties de `qa_head_reviews` depuis `$DEPLOY_AT`. C'est la première mesure de la
fréquence réelle des réattributions, celle que le § *Mesurer ou s'en passer* met
en place.

## Le legs

**La ligne de 12:45:13Z reste en base. Aucun SQL, aucune suppression.**

Elle rejoint la cohorte de legs que mika#2934 libère déjà par la fenêtre de
mika#2920 : le pré-filtre ne lit que les lignes postérieures à son déploiement
(après le 2026-11-26, ou `MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD=off` jusque-là).
Cette ligne est antérieure à cette borne, donc elle n'écartera jamais aucune
revue.

Le traitement est donc : **laisser**, et pour trois raisons.
(a) Elle est déjà hors de portée du seul lecteur à venir.
(b) La neutraliser demanderait un `DELETE` ciblé sur `audit_events`, alors que le
registre a un écrivain unique et aucun supprimeur — ajouter un chemin de
suppression pour une ligne coûterait plus que la ligne.
(c) **Elle est un sujet d'expérience** : c'est le seul cas mesuré de
réattribution, et la sonde d'AC3, non bornée, doit la rendre en violation. La
supprimer rendrait AC3 invérifiable.

La sonde d'AC4, bornée par `$DEPLOY_AT`, ne la signale pas. Celle d'AC3, non
bornée, la signale — et c'est l'asymétrie voulue : l'une mesure le présent,
l'autre atteste la correction sur le passé.

## Ordre de commit, et le plancher

Un seul vol. Trois commits, dans cet ordre, et l'ordre porte la Fire-Disposition.

1. **Le correctif.** L1 + L2 + L3 — handler, `tools.json`, prompt. Après ce
   commit le défaut n'existe plus, et le prompt porte les clauses que les
   détecteurs exigeront.
2. **Les détecteurs.** L4 (suite bash) + L5 (câblage CI) + L6 (clauses Rust et
   contrôle négatif étendu). Ils atterrissent **après** le correctif, donc ils
   sont verts à l'atterrissage sans aucune exception — c'est ce qui permet à
   l'allowlist d'être livrée vide, et c'est la même séquence que mika#2934.
3. **La sonde d'AC3 corrigée**, écrite dans ce plan, plus la mention de la
   cinquième ligne attendue par la lecture non bornée.

**Plancher (ce qui doit être vrai pour qu'un commit soit mergeable seul) :**
`cargo test -p mika-agent qa_reviewed_head` vert, `bash
scripts/test-qa-head-reviews-handler.sh` vert à partir du commit 2,
`make verify-bundled-skills` vert (il valide la cohérence de `tools.json` et la
résolution des handlers), `cargo clippy` et `cargo fmt --check` propres, et
`scripts/verify-pipeline.sh origin/main` vert.

Le commit 1 est mergeable seul (il corrige le défaut, sans détecteur). Le commit
2 n'est **pas** mergeable sans le 1 — il rougirait sur un prompt non corrigé —
et c'est délibéré : un détecteur qui atterrit avant sa correction doit s'excuser
d'un antécédent, et l'ordre inverse supprime le besoin d'exception.

## Definition of Done

- `qa_head_reviews` rend l'objet à quatre champs, applique le filtre temporel
  lui-même, et rend `ERROR` sur une liste de suites vide ou une date illisible.
- Le commentaire d'en-tête du handler ne porte plus l'affirmation que ce ticket
  réfute.
- `tools.json` décrit la nouvelle sortie, et dit que `dropped_predating_head`
  n'est jamais une raison de sauter.
- Les étapes 3 et 4 du prompt lisent `reviews`, portent la ligne sur
  l'instrument, et ne citent aucun état de PR révocable.
- `scripts/test-qa-head-reviews-handler.sh` existe, rejoue le cas #2920 et son
  contrôle positif dans le même test, porte son contrôle de rougeur, et est
  **câblé dans `ci.yml`** sans entrée dans `suite-wiring.tsv`.
- Les clauses Rust de mika#2934 couvrent les deux littéraux nouveaux, et le
  contrôle négatif du test est étendu en conséquence.
- Les sondes d'AC3 et d'AC4 sont écrites dans ce plan, avec leur lecture 0 de
  déploiement, leur contrôle positif et leur issue `INDÉTERMINÉ`.
- Le traitement du legs est tranché dans ce plan : laisser, sans SQL.
- Le plancher ci-dessus est vert.

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2939.

- **AC1, une revue ne statue que sur une tête qui existait quand elle a été
  soumise.** La preuve « verdict déjà enregistré sur cette tête » exclut toute
  revue soumise avant l'arrivée de la tête sur la PR. Le plan choisit et justifie
  le repère : l'instant du push de la tête (événement `head_ref_force_pushed` ou
  `committed` de la timeline, ou `created_at` du check_suite). La date de
  committer n'est pas un repère sûr, puisqu'elle se fixe localement. L'outil
  `qa_head_reviews`, ou ce qui le remplace, applique ce filtre lui-même, et le
  prompt ne le refait pas.
- **AC2, le cas de #2920 est rejoué dans un test.** Fixture : une revue APPROVED
  dont le `commit_id` vaut la tête, soumise avant le push de cette tête. Le test
  atteste qu'aucune preuve n'est rendue. Le contrôle positif est dans le même
  test : une revue soumise après le push de la tête est rendue.
- **AC3, la sonde temporelle est corrigée.** Le premier verdict « sur cette
  tête » est le premier verdict soumis **après l'arrivée de la tête**. La lecture
  de bonne foi rejoue la ligne de 12:45:13Z sur `#2920@5e4918cf` et doit la
  rendre en violation.
- **AC4, la sonde post-déploiement**, écrite dans le plan : après `make deploy`,
  sur au moins une PR rebasée sans changement après une approbation, aucune ligne
  `stop_*` n'est écrite avant une revue soumise après le push.

## Fire-Disposition

**Option (a) — exception nommée en allowlist, et l'allowlist est LIVRÉE VIDE.**

Détecteurs livrés : `scripts/test-qa-head-reviews-handler.sh` (L4), son câblage
CI (L5), et les deux clauses ajoutées à `CLAUSES_DE_LA_TETE_JUGEE` (L6).

Les trois atterrissent au commit 2, **après** le correctif du commit 1. Au moment
où ils arrivent, le prompt porte les clauses et le handler porte le filtre :
**il n'existe aucune violation antécédente à excuser**, donc
`CONFORMITE_TETE_JUGEE_ALLOWLIST` reste `&[]`, exactement comme mika#2934 l'a
livrée.

L'auto-nettoyage est **déjà en place** et n'a pas à être réécrit : la boucle
finale de `mika2934_letape_4_nomme_la_lecture_et_son_outil` assert que la clause
de toute exception est **absente** de son prompt, donc une exception dont la
clause est revenue fait rougir le test — l'entrée expire par le geste même qui la
rend inutile. C'est ce qui empêche cette allowlist de devenir celle que mika#2309
condamne.

Si une exception devait néanmoins être ajoutée en cours d'implémentation, elle
porterait : le couple `(fichier, clause)` exact (grep-visible), un ticket de
suivi nommé dans le commentaire au-dessus de l'entrée, et l'assertion
auto-nettoyante ci-dessus — qui la couvre déjà par construction.

Rien n'est livré désarmé : aucun `#[ignore]`, aucun `#[cfg(skip)]`, et la suite
bash tourne en CI dès le commit 2.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/qa-review-webhook-success/handlers/qa_head_reviews.sh` | 55 |
| `skills/bundled/qa-review-webhook-success/tools.json` | 3 |
| `skills/bundled/qa-review-webhook-success/system_prompt.md` | 10 |
| `scripts/test-qa-head-reviews-handler.sh` (nouveau) | 180 |
| `.github/workflows/ci.yml` | 6 |
| `crates/mika-agent/src/qa_reviewed_head.rs` (clauses + contrôle négatif) | 25 |

Total estimé : 279 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 279 est sous la borne par vol (429)
et très loin du seuil par PR (1000) ; la plus serrée des deux, 429, laisse 150
lignes de marge. Le poste dominant est la suite bash (180), dont l'idiome est
**copié** d'une suite existante (`test-verify-dependabot-topology.sh`) plutôt
qu'inventé : le faux `gh`, le `trap`, l'aiguillage par slug et les compteurs sont
du report, pas de la conception. Le moteur Rust ne reçoit **aucun** changement de
production (L6) — ses 25 lignes sont deux littéraux et l'extension d'un
`assert_eq!`. Aucun DDL, aucune migration, aucun nouveau site de lecture. Le
découpage en vols serait donc du découpage d'un travail qui n'en demande pas.
