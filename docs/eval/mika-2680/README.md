# Mesure 3 — ce que coûtent les tours webhook, après le vol 1 (mika#2680)

Ce répertoire porte le **protocole** de la troisième mesure du § *Critère
d'acceptation de la mesure* de mika#2680, et son **artefact** :
`summary-2026-10-19T0602Z-1630Z.json`, pris le 2026-10-19 par l'orchestrateur
hors bac à sable — le § 2 bis dit pourquoi ce ne pouvait pas être un vol
dispatché — et commité par le vol 6. L'instrument qui le produit
(`scripts/measure-webhook-turn-cost`) et ce protocole viennent du vol 2.

Ni ce protocole ni cet instrument n'implémentent de classe de pré-filtre :
chiffrer d'abord, décider ensuite. L'état classe par classe — ce que la mesure a
tranché et ce qui reste à l'opérateur — est au § 9.

## 1. Pourquoi une troisième mesure

Les deux premières (commentaires du 2026-10-08 et du 2026-10-14) ont chiffré par
`llm_calls` × premier message `user` de chaque session, en rattachant à la main
chaque session à son événement déclencheur. Deux limites, et c'est le vol 1 qui
les lève :

- **Le rattachement était humain.** Rien dans la donnée ne disait « ce tour-ci
  vient d'un `check_suite` sur une branche de PR » ; il fallait lire le premier
  message et décider. Depuis le vol 1, `webhook_turn_charged` porte `trigger`,
  `text_touched`, `candidate` et `prefilter_enabled` **à l'instant du tour**.
- **La classe (b) n'avait jamais été exercée.** La mesure 2 le dit en toutes
  lettres : aucune `check_suite` de branche de PR depuis le déploiement de
  `e14ace7f`. Un chiffrage de (e)+(f)+(g) pris là aurait compté comme à filtrer
  ce que (b) écarte déjà.

## 2. La fenêtre

**Au moins 4 h 30 de données servies**, pour être comparable à la fenêtre de
référence (2026-10-07 16:00Z–20:30Z).

Le registre est servi depuis le **2026-10-16 13:01Z** (merge de #2705 en
`185362cc`). La borne basse de toute mesure est donc `2026-10-16T13:01:00Z` —
**jamais avant**, et l'instrument le dit lui-même : quand `--since` précède
`field_epoch`, le seau des `turn_usage` sans ligne payée change de nom (§5).

Une fenêtre qui ne contient **aucune activité PR** ne vaut rien pour ce
chiffrage : (g) porte sur les `check_suite` de branche de PR, (f) sur les
`pr_review`. C'est précisément ce qui a invalidé la mesure 2.

## 2 bis. Qui exécute cette commande — pas un pilote dispatché

**Le bac à sable d'un pilote ne monte pas le journal.** Le § 3 lit
`$MIKA_SPIRIT_LOG_FILE`, c'est-à-dire `/var/log/mika/server.log`
(`packaging/systemd/mika-spirit.service`). Dans le namespace d'une session
dispatchée, `/var/log/mika` **n'existe pas** : constaté le 2026-10-17 depuis le
vol 3, `ls /var/log/mika/` rend `No such file or directory`.

**La cause est une décision, pas un oubli.** Le jeu de binds de
`skills/bundled/_shared/dispatch-lib.sh` monte `/var/log/claude-pilot`
(`_pilot_log_bind_args`, mika#2165) et rien d'autre sous `/var/log`. Le
commentaire de ce helper le dit en toutes lettres : « *this directory, never
/var/log. /var/log/mika holds the egress-proxy log, an incident-diagnosis
surface (mika#2041), and the pilot has no business writing there.* » Élargir le
bind pour rendre la mesure 3 prenable depuis un pilote reviendrait à donner à
chaque session dispatchée l'écriture sur une surface de diagnostic d'incident.
Ce n'est pas un arbitrage que la mesure justifie.

**La conséquence, et le seul endroit où elle ne se lit pas comme un résultat.**
Lancée là, la commande ne se trompe pas : sans fichier lisible, l'instrument
sort en `2`, et le § 6 dit que `2` signifie *rien n'a été mesuré* — jamais
« zéro tour payé ». Le refus est donc explicite, et c'est tout ce que
l'instrument peut faire. La mesure 3 est par construction **un geste
opérateur**, ou une session hors bac à sable (hôte, ou tout contexte où
`/var/log/mika/server.log` est lisible). Un vol dispatché ne peut ni l'exécuter
ni la contourner ; il peut seulement porter la commande prête à coller, ce que
fait le corps de PR du vol 3.

**Et c'est ainsi qu'elle a été prise — ce qui précède est désormais un
historique.** La mesure 3 existe : le **2026-10-19**, par **l'orchestrateur**,
hors bac à sable, sur le journal servi, avec la commande du § 3 appliquée à la
fenêtre `06:02Z–16:30Z`. Son agrégat est dans ce répertoire,
`summary-2026-10-19T0602Z-1630Z.json`, et le § 9 en tire l'état de la classe (g).
Le raisonnement ci-dessus ne devient pas caduc pour autant : il dit pourquoi
aucun des vols dispatchés ne pouvait la prendre — trois d'entre eux l'ont
redécouvert à leurs frais les 2026-10-19 00:26Z, 00:52Z et 01:03Z — et pourquoi
le prochain ne le pourra pas davantage. Le mur est double : au bind manquant
s'ajoute la permission-policy de la session dispatchée, qui refuse
`scripts/measure-webhook-turn-cost` en amont (`no matching policy rule — denied
by default`). Élargir le bind ne lèverait que le premier, et la mesure ne
justifie toujours pas de l'élargir.

## 3. La commande

```bash
scripts/measure-webhook-turn-cost \
    --log-file "$MIKA_SPIRIT_LOG_FILE" \
    --since 2026-10-16T13:01:00Z \
    --until <fin de fenêtre, ISO-8601 UTC> \
    --out    docs/eval/mika-2680/turns-<fenêtre>.jsonl \
    --summary docs/eval/mika-2680/summary-<fenêtre>.json
```

Le corpus est le **journal**, jamais la base : la moitié « payé » du registre
n'écrit aucune ligne d'audit (choix délibéré du vol 1), et `turn_usage` est
dégaté là où `llm_calls` dépend de `MIKA_STORE_LLM_CALLS`. Sans `--agent`, les
deux agents sont dans le même tableau — ce qu'il faut, puisqu'un `check_suite`
vert coûte un tour **aux deux**.

Les deux fichiers produits sont l'artefact du vol 3 ; ils se committent ici.

## 4. Lire les deux moitiés

L'agrégat porte deux populations qui ne se soustraient pas l'une de l'autre :

| moitié | champ | ce qu'elle dit |
|---|---|---|
| payée | `by_agent_trigger` | les tours qui ont **eu lieu**, par agent × déclencheur, avec leur coût |
| payée | `charged_by_release` | **pourquoi** chacun a eu lieu, par terme de relâche (dix valeurs, vol 4) |
| payée | `by_agent_trigger_release` | les deux à la fois : agent × déclencheur × terme de relâche, **avec le coût en tokens** (vol 5) |
| écartée | `skipped_by_class` | les tours que le pré-filtre a **évités**, par classe |
| écartée | `skipped_by_agent_class` | la même population, **par agent** × classe (vol 5) |

**Les deux taxonomies ne se dérivent pas l'une de l'autre**, et c'est pour cela
qu'il y a deux seaux. `skipped_by_class` nomme ce qu'une porte a reconnu ;
`charged_by_release` nomme la porte qui a relâché. Un même `trigger` se répartit
sur plusieurs termes de relâche, et un même terme traverse plusieurs
déclencheurs.

**Elles ne se dérivent pas, mais elles se croisent — et c'est le croisement qui
porte la décision.** Jusqu'au vol 4, les deux opérandes de la soustraction de la
classe (g) vivaient sur deux axes qui ne se rencontraient pas :
`charged_by_release` donnait le terme de relâche sans l'agent et sans le coût,
`by_agent_trigger` donnait l'agent et le coût sans le terme. Répondre à *« que
coûtent les `check_suite` verts de branche de PR, côté `mika-qa`, dont la tête
n'avait pas encore été décidée ? »* demandait un `jq` ad hoc sur le fichier de
tours — exactement la pièce rapportée humaine que le registre du vol 1 existait
pour supprimer. Les deux seaux croisés du vol 5 la rendent lisible sur le seul
agrégat : `by_agent_trigger_release` pour la moitié payée,
`skipped_by_agent_class` pour la moitié écartée.

Ils **ne remplacent ni l'un ni l'autre** des deux seaux plats, et ne les
modifient pas d'un octet. Un seau plat répond à une question de fenêtre — *« sur
quels termes cette fenêtre a-t-elle relâché ? »* — et c'est la lecture des § 5 et
§ 6. Un seau croisé répond à une question de décision, qui se pose toujours au
grain d'une cellule.

**L'unité du seau payé est le token, pas le tour.** `charged_by_release` compte
des tours ; `by_agent_trigger_release` porte les trois valeurs de coût de son
voisin `by_agent_trigger` — `input_tokens`, `cache_read_tokens`, `llm_calls` —
et il est trié comme lui, par `input_tokens` décroissant. Un croisement qui ne
porterait que `turns` aurait l'air complet et laisserait (g) indécidable : deux
cellules à un tour chacune peuvent valoir 900 tokens et 120. Le seau écarté, lui,
n'a pas de clé de coût du tout et porte `skipped` — un tour évité n'a consommé
aucun token, et lui en prêter un serait inventer un chiffre.

**Ce que le registre décide.** La moitié payée chiffre chaque classe candidate
par son `trigger` : (e) est `pr_closed`, (g) est `check_suite_pr_branch` côté
`mika-qa`, (h) est `issue_comment` et `issues_closed`. Ces chiffres-là sont des
mesures.

**(f) ne se lit PAS sur `text_touched`, et ce document l'a dit à tort** jusqu'au
vol 4. `text_touched` est `Touched::by_any_handler`, c'est-à-dire
`req.text != original` : il est vrai pour une revue sans ligne `VERDICT:` comme
pour une revue qu'un handler ultérieur a touchée, et vrai aussi dès que le
`verdict_handler` a réécrit sans qu'aucun handler ultérieur n'intervienne. Le
champ n'y sépare pas les chemins, il les agrège — un chiffre pris là ne
désignerait aucune décision. C'est `charged_by_release` qui les sépare : voir la
ligne **(f)** du § 9.

**Le seau d'absence.** Une ligne `webhook_turn_charged` écrite par un binaire
antérieur au vol 4 ne porte pas `release` ; elle est comptée sous
`absent_pre_field`, jamais sous `null` ni `unknown`. Une fenêtre entièrement
antérieure au déploiement se lit donc à 100 % dans ce seau, et c'est pourquoi le
§ 5 ne gagne **pas** une troisième condition de halte pour ce champ-ci : le seau
dit déjà la raison de sa propre absence. `by_agent_trigger_release` applique la
même règle, et il l'applique **aux deux endroits** : le repli
`absent_pre_field` est dans la clé de groupement *et* dans la projection de la
cellule. Dans la clé seule, la cellule aurait le bon compte sous un faux nom
(`release: null`) ; dans la projection seule, deux populations se
mélangeraient. C'est le motif mika#2331 — *absent n'est ni zéro ni « none »* — et
l'assertion **N8** du § 4 de `scripts/test-measure-webhook-turn-cost.sh` le
tient en vérifiant les deux moitiés à la fois.

**Ce que le registre ne fait que majorer.** Un tour payé n'est pas un tour
*inutile*. `by_agent_trigger` donne le **plafond** de ce qu'une classe pourrait
économiser, jamais l'économie elle-même : décider qu'un `issue_comment` sans
tâche de boucle ne mérite pas un tour complet est un jugement sur ce que le tour
fait, et la question est posée telle quelle dans le commentaire du 2026-10-08.
L'instrument ne la tranche pas.

**Ce qu'`unattributed*` n'est pas.** Les `turn_usage` sans ligne payée de même
`trace_id` ne sont **pas** des tours webhook non comptés : ce sont, pour
l'essentiel, les tours Telegram et CLI, qui ne passent pas par le registre. Leur
seau existe pour que leur coût soit **visible et hors tableau**, pas pour être
ajouté.

## 5. La halte — deux raisons de refuser de conclure

Un chiffre pris dans l'une de ces deux conditions ressemble à un résultat et n'en
est pas. Les deux sont **dans l'agrégat**, pas dans une note :

1. **`window_precedes_field_epoch: true`.** La borne basse précède l'époque du
   champ (ou il n'y a pas de borne basse : la fenêtre est alors ouverte vers le
   passé). Dans cette zone, « pas de ligne payée » ne distingue pas un tour
   Telegram d'un tour servi par un binaire antérieur au registre. Le seau est
   alors rendu sous le nom `unattributed_undetermined` : **le seau change de nom,
   pas de place** — un lecteur qui cherche `unattributed` ne doit pas trouver un
   chiffre qui ne veut pas dire ce qu'il croit. *Remède : remonter `--since` à
   `field_epoch` ou après.*
2. **`prefilter_enabled: false` sur des tours de la fenêtre.** Le pré-filtre
   était désarmé : ces tours-là auraient peut-être été écartés par (a)–(d), et
   les compter comme candidats à (e)/(f)/(g) attribuerait à une classe nouvelle
   ce qu'une classe existante couvrait déjà. *Remède : restreindre la fenêtre à
   la période armée, ou chiffrer les deux populations séparément.*

`field_epoch` est levé sur le **corpus entier**, avant `--since`, `--until` et
`--agent` : c'est une propriété du binaire déployé, pas de la fenêtre qu'on lui
demande. Levé après les filtres, il vaudrait toujours au moins `--since` et le
drapeau ne saurait dire que « oui ».

## 6. Les deux codes de sortie

| code | sens |
|---|---|
| `0` | le corpus a été **lu**. Zéro tour payé est alors un **résultat**. |
| `2` | **rien n'a été mesuré** : pas de fichier, illisible, `jq` absent, borne mal formée, aucune ligne `webhook_turn_charged`, `--agent` sans appariement. |

La distinction est la leçon mika#2205 : un scan inerte en silence se lit
exactement comme un scan qui n'a rien trouvé. `unparseable_lines` est compté et
rapporté, mais **ne conjugue pas** la décision « rien mesuré » — une seule ligne
tronquée, quasi certaine sur un journal de 19 Go, rendrait sinon `exit 2`
inatteignable pour toujours.

## 7. Ce que le vol 3 fait de l'artefact

1. Chiffrer (e), (f), (g) et (h) sur la fenêtre, **après** retrait de ce que
   `skipped_by_class` montre déjà écarté par (a)–(d).
2. Pour chaque classe, répondre à la question du § *Ce que le registre ne fait
   que majorer* — que fait le tour, et ce travail survit-il autrement ?
3. N'implémenter que les classes dont la réponse est « rien, ou rien qui
   nécessite un tour LLM », une classe par phase dans `webhook_prefilter.rs`.

Une classe dont le chiffre est significatif mais dont la question reste ouverte
ne s'implémente pas : elle se pose à l'opérateur.

## 8. L'instrument lui-même

- `scripts/measure-webhook-turn-cost` — le lecteur. `--help` imprime son en-tête.
- `scripts/test-measure-webhook-turn-cost.sh` — son harnais : treize assertions,
  trois contrôles positifs et dix négatifs, chacun sur sa propre entrée. Le
  premier positif tient l'unité (le tour, et la valeur est une **somme** sur
  toutes les lignes `turn_usage` de la trace, continuation `step = u32::MAX`
  comprise) ; les deux autres tiennent le terme de relâche (§ 3 du harnais,
  vol 4) et le croisement des deux taxonomies (§ 4, vol 5). `make
  test-measure-webhook-turn-cost`. **L'étape CI qui appelle cette cible est
  posée** — `.github/workflows/ci.yml` l. 111-112, *« Webhook-turn register
  reader — the turn is a SUM (mika#2680) »*, par `8ac93d62` (mika#2752) ; le
  jeton d'un pilote dispatché n'a pas le scope `workflow`, donc elle a été posée
  hors de la boucle, et c'est ce qui en faisait un hors-périmètre des vols 2 et
  3. **Et c'est aussi pourquoi cette cible n'est PAS dans
  `scripts/suite-wiring.tsv`** : ce registre ne porte que le résidu — les suites
  qu'aucun workflow n'atteint — et sa comparaison est bidirectionnelle depuis
  mika#2761, donc une entrée pour une suite que la CI atteint **fait rougir**
  `check-suite-wiring`.
- `crates/mika-agent/tests/fixtures/mika2680/` — six corpus gelés et
  synthétiques (trois du vol 2, deux ajoutés par le vol 4 pour le terme de
  relâche, un par le vol 5 pour le croisement). Ils ne sont pas échantillonnés
  sur le journal vivant : une suite qui le lirait changerait de verdict à mesure
  qu'il grossit. Le corpus du vol 5, `cross_tab_corpus.jsonl`, est construit pour
  que **les deux seaux plats y soient ambigus** — un `{head_not_previously_decided:
  2}` et un `{green_check_suite_head_decided: 2}` qui ne disent ni de quelle
  cellule ni de quel agent ils parlent — et pour que les deux seaux croisés
  lèvent chacune de ces ambiguïtés : c'est cette propriété, et non une économie
  de fichiers, qui autorise ses quatre assertions à partager une entrée.

## 9. L'état des quatre classes, après les vols 3 à 6

Le § 7 dit ce qu'un vol *fait* de l'artefact. Le § 2 bis dit pourquoi aucun vol
dispatché ne pouvait le produire, et qui l'a produit. Ce qui suit est l'état
réel, classe par classe — et **les quatre raisons ne sont pas la même raison** :
(e) implémentée, parce qu'aucune mesure ne conditionnait son chiffre ; (f)
tranchée par le code servi seul, sa prémisse étant fausse ; (g) **chiffrée puis
réfutée** — le seul cas où la mesure a parlé et où cela n'a pas suffi ; (h)
posée sans qu'aucune mesure ne la tranche. C'est tout l'intérêt du tableau.

| classe | déclencheur | état après les vols 3 à 6 | ce qui décide de cet état |
|---|---|---|---|
| **(e)** `closed`/`merged` d'une PR sans tâche de boucle | `pr_closed` | **implémentée** (`pr_closed_no_loop_task`) | la mesure 3 ne pouvait pas changer son chiffre |
| **(f)** `pr_review` sur une PR fermée au texte réécrit par le `verdict_handler` | `pr_review` + `charged_by_release` | **couverte par (c) pour les deux formes citées** ; résidu = une décision AC2 et trois chemins non-filtres | le code servi, relu — pas une mesure |
| **(g)** `check_suite success` sur branche de PR, côté mika-qa | `check_suite_pr_branch` | **tranchée** — chiffrée (36 tours, 10 687 327 tokens : le plafond entier, la soustraction de (b) étant vide) et sa prémisse réfutée ; **aucun filtre écrit**, successeur (g′) posé à l'opérateur | la mesure 3 a donné le chiffre ; le code servi, relu, a réfuté la question |
| **(h)** `issue_comment` / `issues_closed` sans tâche de boucle | `issue_comment`, `issues_closed` | **posée à l'opérateur** | aucune mesure ne la tranche |

**(e) — pourquoi elle part sans l'artefact.** Le retrait que le § 7.1 prescrit
(« après retrait de ce que `skipped_by_class` montre déjà écarté par (a)–(d) »)
est, pour (e), **un seau vide par construction** : aucune des classes (a) à (d)
ne regarde le déclencheur `pr_closed`. (a) et (b) sont des `check_suite`, (c) un
`pr_review`, (d) un `issues labeled`. Le chiffre de (e) après retrait est donc
égal à son chiffre avant retrait, et ce chiffre est déjà pris : n=2, 193 k puis
279 k tokens d'entrée, sur deux fenêtres distinctes (mesures 1 et 2), par
l'opérateur, à la main. La mesure 3 l'aurait confirmé ; elle ne le conditionnait
pas. C'est la seule des quatre dont on peut dire cela.

**(f) — sa prémisse était fausse, et le vol 4 l'a vérifiée sur le code servi.**
Le ticket pose (f) sur l'idée que *« la règle « texte intact après handler » de
la phase 2 exclut ce cas »*. Elle ne l'exclut pas. `after_verdict_handler` — la
fenêtre que `Touched::applies_to` consulte pour un `Candidate::ReviewVerdict` —
est calculée au site d'appel comme `text_after_verdict.as_ref().is_none_or(|t|
req.text != *t)`, et `text_after_verdict` est pris **après** que le
`verdict_handler` a rendu. La réécriture du `verdict_handler` est donc **dans**
la fenêtre : seul un handler **ultérieur** garde le tour. Les deux formes que le
constat cite nommément sont par conséquent déjà écartées par la classe (c) —
`CIRCUIT BREAKER TRIPPED` par `verdict_pr_closed`, `verdict_classification_failed`
par `verdict_unreadable` — et les quatre assertions de
`mika2680_f_le_pr_review_reecrit_est_deja_ecarte` gèlent ce constat dans la
suite, de sorte qu'un futur handler intercalé le fasse rougir au lieu de le
rouvrir.

Ce qui reste n'est pas un résidu mesurable mais **quatre chemins de nature
différente**, que `charged_by_release` sépare et qu'aucune conjonction de champs
ne séparait :

| terme de relâche | ce que c'est | ce qu'il faudrait pour agir |
|---|---|---|
| `not_classified` | une revue **hors population** : pas de ligne `VERDICT:`, donc hors du périmètre de (c) — typiquement une consigne opérateur en prose | une **décision AC2** (mika#2675) : élargir la population au `pr_review` sans verdict, ou pas |
| `pr_task_active` | une tâche de boucle vit sur la PR | rien : le tour est légitime |
| `state_unreadable` | la forge ou la base n'a pas répondu | rien : fail-safe vers le LLM, par construction |
| `text_touched` | un handler **ultérieur** a réécrit | rien : la règle de la phase 2 fait son travail |

Trois de ces quatre chemins **ne sont pas des filtres** : les toucher reviendrait
à retirer un garde fail-safe. Le quatrième est une question, pas un chiffre — et
c'est pourquoi (f) ne passe pas de « renvoyée » à « implémentée » mais de
« renvoyée » à **tranchée** : il n'y a pas de classe (f) à écrire.

**(g) — chiffrée, puis réfutée : la mesure a parlé, et elle n'a pas suffi.** Les
deux termes qui lui manquaient sont arrivés, mais de deux sources différentes —
le chiffre de l'agrégat, la question du code servi — et ils ne pointent pas dans
le même sens.

*Le chiffre d'abord, et pourquoi c'en est un.* La mesure 3 porte sur la fenêtre
`2026-10-19 06:02Z–16:30Z` (10 h 28, activité PR présente). Le § 6 est attesté
par l'agrégat lui-même — `paid_turns: 244`, `paid_turns_without_usage: 0`,
`unparseable_lines: 0` : le corpus a été lu, et ses seaux sont des résultats.

Les deux haltes du § 5 ne se lèvent pas de la même façon, et la seconde demande
une précision que ce document doit à son lecteur. La première est **dans**
l'agrégat : `window_precedes_field_epoch: false`, la borne basse suivant le
premier `webhook_turn_charged` porteur d'un terme de relâche. La seconde ne
l'est pas — `prefilter_enabled` est écrit **par tour**, dans le `turns-*.jsonl`
que ce vol ne commite pas, et l'agrégat n'en porte aucun seau. Ce que l'agrégat
établit est plus faible, et c'est tout ce qu'on peut lui faire dire : le
pré-filtre était **armé et actif** sur la fenêtre, puisqu'il y a écarté 150
tours répartis sur sept classes (`skipped_by_class`). Un pré-filtre désarmé
n'écarte rien. Cela ne prouve pas qu'aucun tour isolé n'ait été servi désarmé ;
affirmer le contraire demanderait le fichier de tours, dont la commande du § 3
régénère à l'identique les 244 lignes. La cellule `mika-qa ×
check_suite_pr_branch` vaut
**36 tours, 10 687 327 tokens d'entrée** (7 163 735 en cache) et 202 appels LLM,
soit **16,31 % des 65 507 602 tokens d'entrée de la fenêtre**.

*La soustraction que le § 7.1 prescrit est vide, et ce n'est pas un hasard de
fenêtre.* La ligne `mika-qa × check_suite_pr_branch × head_not_previously_decided`
**n'existe pas** dans `by_agent_trigger_release` : la cellule est à 36/36 en
`no_decision_attested`. Côté écarté, `skipped_by_agent_class` n'attribue le seul
`green_check_suite_head_decided` de la fenêtre qu'à mika-dev, pour un tour. (b)
ne revendique donc rien côté relecteur, et le plafond de (g) tient entier. La
cause est structurelle, et se relit en trois pas sur l'arbre servi :

1. `decide_head_decided`
   (`crates/mika-agent/src/server/webhook_prefilter.rs:818-840`) a besoin d'une
   ligne d'audit `DECISION_CORE_HOLD_TOOL` **pour cet événement** afin d'obtenir
   le préfixe de PR ; sans elle, elle rend
   `Decision::Llm(Release::NoDecisionAttested)` (l. 826-829) sans même lire les
   attestations de la PR.
2. Cette ligne ne s'écrit qu'à
   `crates/mika-agent/src/server/ci_success_handler.rs:628`, soit **après** la
   porte d'entrée de mika#2260, dont le commentaire ne laisse pas de place au
   doute : *« la porte précède donc **tout** : le `evaluating` ci-dessous,
   l'exigence de token, le dedup, et toute ligne d'audit »*.
3. mika-qa n'est pas l'acteur de merge — la porte refuse sur
   `!owns_merge_transition(agent_id)` — donc il n'atteint jamais ce site.
   **(b) ne peut pas même regarder sa tête.**

Le terme de relâche ne décrit donc pas ici un cas limite : il décrit le seul
chemin possible.

*La prémisse ensuite, et elle est fausse.* Le ticket pose (g) sur l'idée que le
relecteur reçoit le fan-out *« sans qu'aucun verdict n'en dépende directement »*.
Le code servi dit l'inverse, et il le dit par écrit. La porte rend
`Passthrough { enrichment: None }` **et non** `Handled`, en nommant le
consommateur qu'elle protège : *« le tour du relecteur doit voir l'**événement
brut**, que `qa-review-webhook-success` attend pour corréler la PR »*
(`ci_success_handler.rs:209-213`). Cette skill est bundled et allowlistée sur
mika-qa (`crates/mika-agent/src/well_known_agents.rs:323`, recensée en `:2731`) ;
son prompt porte l'ordre inverse d'un tour vide (*« CRITICAL: DO NOT end your
turn without acting. This is a CI-green notification that unblocks review. »*) ;
et son incident fondateur est mesuré, pas supposé — avant mika#1711, mika-qa ne
recevait aucun signal de CI verte, d'où *« a 14-hour dead qa window on
2026-07-01 during which multiple autonomous-loop PRs landed READY+GREEN with
zero qa attempts »*. Ce tour est **l'unique porte d'entrée autonome de
`qa-review` sur CI vert** : l'écarter rouvrirait exactement cette fenêtre morte.
Le ticket met d'ailleurs ce travail hors périmètre — *« Le tour de revue QA
lui-même : légitime. »*

*Ce que l'agrégat ne peut pas dire, et c'est là que ça s'arrête.* Le registre
porte `trigger`, `release`, `candidate`, `text_touched`, `prefilter_enabled` et
le coût ; il **ne porte pas ce que le tour a fait**. Or le chemin de
`qa-review-webhook-success` compte six étapes, dont quatre sont des arrêts —
aucune PR ouverte sur la tête, hors périmètre (auteur non-boucle, brouillon),
déjà revue à ce SHA, verdict déjà enregistré — et une est la revue elle-même.
Les 36 tours mélangent les deux populations, et l'agrégat ne les sépare pas. Son
seul indice est le coût moyen : **5,6 appels LLM par tour**, contre 12,5 pour
`mika-qa × pr_opened` et 13,8 pour `mika-qa × pr_other`. Une décomposition tient
l'arithmétique — treize revues complètes à ~13 appels plus vingt-trois arrêts
précoces à ~1,5 donne 203,5 pour 202 observés — mais elle repose sur un coût par
forme que le registre n'atteste pas, et une autre la tient aussi bien. C'est une
vraisemblance, pas un compte. Le § 7.3 n'admet que les classes dont la réponse
au § 7.2 est « rien, ou rien qui nécessite un tour LLM » ; ici la réponse est
**« parfois tout, parfois rien »**, dans une proportion inconnue. La phrase du
§ 7 s'applique telle quelle : *une classe dont le chiffre est significatif mais
dont la question reste ouverte ne s'implémente pas : elle se pose à
l'opérateur.* **Aucun filtre de classe (g) n'est écrit.**

*Le successeur (g′) est nommé et borné.* Ce qui serait filtrable, ce ne sont pas
les 36 tours : ce sont les **arrêts précoces**. Leur prédicat est à portée
technique — le pré-filtre appelle déjà la forge, `PrefilterState::pr_state` fait
un `gh pr view <n> --json state` pour la classe (c). L'obstacle est ailleurs, en
trois arbitrages dont aucun n'est à la main d'un pilote :

1. **Le coût que la porte de mika#2260 venait de retirer.** Elle chiffre ce
   qu'elle a supprimé — *« cinq appels `gh` sous le PAT du relecteur »* — et le
   commentaire de son unique ligne d'audit ajoute que connaître le numéro de PR
   *« coûterait le premier appel `gh`, c'est-à-dire l'effet de bord que cette
   porte retire »*. Un filtre (g′) les remettrait, au même endroit, sous le même
   jeton.
2. **Un garde de prompt qui descend dans le Rust.** Les arrêts 2 à 4 vivent dans
   `skills/bundled/qa-review-webhook-success/system_prompt.md`, dont l'étape 4
   désigne le `commit_id` de la revue GitHub comme source de vérité du dedup
   (*« Do not add a separate task-metadata flag »*). Les porter en code servi
   crée deux sites de vérité à tenir alignés.
3. **La posture fail-open (AC3 de mika#2675).** Une lecture de forge en erreur
   doit laisser passer le tour, et le taux de `Release::StateUnreadable` sur ce
   chemin n'est pas mesuré : un filtre dont la moitié des passages est un
   fail-open paie ses appels `gh` sans rien écarter.

Et sa taille reste inconnue : l'agrégat donne le plafond (36 tours), jamais la
part qui serait effectivement écartée. **Mesurer (g′) demande un champ que le
registre ne porte pas — ce que le tour a fait — donc un vol d'instrument avant
tout vol de filtre.** (g) rejoint ainsi (h) : chiffre significatif, question
ouverte, posée à l'opérateur. Mais sa question est désormais nommée, et c'est la
différence.

**Ce que les vols 4 et 5 avaient livré reste exact**, et c'est ce qui a rendu la
soustraction ci-dessus lisible plutôt que devinée.

*Le vol 4 : la soustraction devient nommable.* Avant lui,
`by_agent_trigger` donnait le plafond de (g) — les `check_suite_pr_branch` payées
côté mika-qa — et rien ne disait lesquelles (b) aurait écartées si elle avait vu
la tête. Le terme de relâche le nomme, parce que c'est exactement ce que (b)
décide : un `check_suite` vert de branche de PR relâché sous
`head_not_previously_decided` est un **premier** traitement de cette tête, que
(b) ne prétend pas couvrir ; un relâché sous `no_decision_attested` ou
`text_touched` est passé devant (b) pour une autre raison, nommée. Le chiffre
de (g) après retrait n'est donc plus une estimation mais une soustraction :
plafond moins les seaux que (b) revendique.

*Le vol 5 : elle devient lisible au grain où elle se décide.* Le
vol 4 avait livré le terme ; il ne l'avait pas livré *au bon grain*, et ce
document a écrit à tort que « `charged_by_release` le dit ». Il le dit au grain
de la **fenêtre** — `{head_not_previously_decided: 7}`, tous agents et tous
déclencheurs confondus, en comptes de tours et sans coût. Or (g) se décide au
grain d'une **cellule** : un agent, un déclencheur, un terme. Les deux opérandes
de sa soustraction vivaient donc sur deux axes qui ne se rencontraient pas, et
les croiser demandait un `jq` à la main sur le fichier de tours. Depuis le
vol 5, `by_agent_trigger_release` est cette cellule, avec son coût en tokens :
la ligne `mika-qa × check_suite_pr_branch × head_not_previously_decided` **est**
la réponse, lue sur le seul agrégat.

Et c'est précisément ce qui a permis au vol 6 de conclure : sur l'agrégat de la
mesure 3, **cette ligne n'existe pas**. Sans le grain du vol 5, l'absence
n'aurait pas été lisible comme une absence — `charged_by_release` au grain de la
fenêtre porte bien un `head_not_previously_decided`, mais il est à mika-dev. Les
deux vols ne remplaçaient pas la mesure ; ils ont rendu son silence
interprétable.

**(h) — le plus gros volume, et le seul blocage qui ne soit pas un chiffre.**
1,30 M tokens, 79 % de la fenêtre de la mesure 2 : le volume est établi. Ce qui
manque n'est pas une mesure mais une **réponse** : le tour met à jour la mémoire
et le fil de priorités, et savoir si ce travail survit sans tour LLM est un
jugement sur ce que le tour fait, pas sur ce qu'il coûte. Le § 7 le tranche déjà
en une phrase — *une classe dont le chiffre est significatif mais dont la
question reste ouverte ne s'implémente pas : elle se pose à l'opérateur.* Elle
est posée dans le corps de PR du vol 3.
