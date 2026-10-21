# Mesure 3 — ce que coûtent les tours webhook, après le vol 1 (mika#2680)

Ce répertoire porte le **protocole** de la troisième mesure du § *Critère
d'acceptation de la mesure* de mika#2680, et accueillera son **artefact**. Le
vol 2 livre l'instrument (`scripts/measure-webhook-turn-cost`) et ce protocole ;
**le vol 3 exécute la mesure, écrit l'artefact, puis implémente ce qu'elle
confirme.** Aucune classe de pré-filtre — ni (e), ni (f), ni (g), ni (h) — n'est
implémentée par le vol 2 : chiffrer d'abord, décider ensuite.

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

## 9. L'état des quatre classes, après les vols 3 à 5

Le § 7 dit ce que le vol 3 *fait* de l'artefact. Le § 2 bis dit pourquoi
l'artefact n'est pas à sa portée. Ce qui suit est donc l'état réel, classe par
classe — et **les quatre raisons ne sont pas la même raison** : c'est tout
l'intérêt du tableau.

| classe | déclencheur | état après les vols 3 à 5 | ce qui décide de cet état |
|---|---|---|---|
| **(e)** `closed`/`merged` d'une PR sans tâche de boucle | `pr_closed` | **implémentée** (`pr_closed_no_loop_task`) | la mesure 3 ne pouvait pas changer son chiffre |
| **(f)** `pr_review` sur une PR fermée au texte réécrit par le `verdict_handler` | `pr_review` + `charged_by_release` | **couverte par (c) pour les deux formes citées** ; résidu = une décision AC2 et trois chemins non-filtres | le code servi, relu — pas une mesure |
| **(g)** `check_suite success` sur branche de PR, côté mika-qa | `check_suite_pr_branch` | **bloquée par la mesure** ; sa soustraction est lisible sur l'agrégat depuis le vol 5 | la mesure 3 est sa condition d'existence |
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

**(g) — la mesure n'est pas un confort, c'est sa condition.** Son plafond se
superpose à ce que (b) écarte déjà : la mesure 2 le dit en toutes lettres,
aucune `check_suite` de branche de PR n'avait été vue depuis le déploiement de
`e14ace7f`, donc un chiffrage pris là aurait compté comme à filtrer ce que (b)
écarte. Et la question du § 7.2 — *que fait mika-qa de ce tour ?* — porte sur un
agent dont le fan-out est délibéré (mika#1711). Les deux termes manquent, et les
deux sont dans l'artefact.

**Ce que le vol 4 change pour (g) : la soustraction devient nommable.** Avant lui,
`by_agent_trigger` donnait le plafond de (g) — les `check_suite_pr_branch` payées
côté mika-qa — et rien ne disait lesquelles (b) aurait écartées si elle avait vu
la tête. Le terme de relâche le nomme, parce que c'est exactement ce que (b)
décide : un `check_suite` vert de branche de PR relâché sous
`head_not_previously_decided` est un **premier** traitement de cette tête, que
(b) ne prétend pas couvrir ; un relâché sous `no_decision_attested` ou
`text_touched` est passé devant (b) pour une autre raison, nommée. Le chiffre
de (g) après retrait n'est donc plus une estimation mais une soustraction :
plafond moins les seaux que (b) revendique.

**Ce que le vol 5 change : elle devient lisible au grain où elle se décide.** Le
vol 4 avait livré le terme ; il ne l'avait pas livré *au bon grain*, et ce
document a écrit à tort que « `charged_by_release` le dit ». Il le dit au grain
de la **fenêtre** — `{head_not_previously_decided: 7}`, tous agents et tous
déclencheurs confondus, en comptes de tours et sans coût. Or (g) se décide au
grain d'une **cellule** : un agent, un déclencheur, un terme. Les deux opérandes
de sa soustraction vivaient donc sur deux axes qui ne se rencontraient pas, et
les croiser demandait un `jq` à la main sur le fichier de tours. Depuis le
vol 5, `by_agent_trigger_release` est cette cellule, avec son coût en tokens :
la ligne `mika-qa × check_suite_pr_branch × head_not_previously_decided` **est**
la réponse, lue sur le seul agrégat. La mesure 3 reste la condition de (g) — les
vols 4 et 5 lui fournissent l'opérande qui manquait et le grain où le lire, ils
ne la remplacent pas.

**(h) — le plus gros volume, et le seul blocage qui ne soit pas un chiffre.**
1,30 M tokens, 79 % de la fenêtre de la mesure 2 : le volume est établi. Ce qui
manque n'est pas une mesure mais une **réponse** : le tour met à jour la mémoire
et le fil de priorités, et savoir si ce travail survit sans tour LLM est un
jugement sur ce que le tour fait, pas sur ce qu'il coûte. Le § 7 le tranche déjà
en une phrase — *une classe dont le chiffre est significatif mais dont la
question reste ouverte ne s'implémente pas : elle se pose à l'opérateur.* Elle
est posée dans le corps de PR du vol 3.

## 10. (g′) — l'axe `outcome`, et ce que la prochaine fenêtre chiffrera (mika#2855)

Le § 9 laisse (g) **bloquée par la mesure**, et nomme les deux termes qui
manquent. Le vol 5 a livré le premier : la cellule `mika-qa ×
check_suite_pr_branch × head_not_previously_decided` donne le plafond, au grain
où (g) se décide. Le second — *que fait mika-qa de ce tour ?* — ne se lisait
nulle part : le registre dit qu'un tour a eu lieu, ce qu'il a coûté et sous quel
terme la porte l'a relâché, jamais **ce qu'il a fait**. Or dans (g) seuls les
arrêts précoces du skill `qa-review-webhook-success` seraient filtrables, et leur
nombre était inconnu.

**mika#2855 est le vol d'instrument qui ouvre cet axe. Il n'écarte aucun tour.**

### L'axe existe : une seconde ligne, jointe par `request_id`

`webhook_turn_charged` est émise **avant** le tour — son message dit « le tour a
lieu » — donc elle ne peut structurellement pas porter une issue qui n'existe pas
encore. Le vol 1 de mika#2855 ajoute un **second terme du registre**, émis en fin
de tour :

| | événement | émis | porte |
|---|---|---|---|
| entrée | `webhook_turn_charged` | avant `run_agent` | `agent_id`, `request_id`, `trigger`, `release`, `candidate`, `text_touched`, `prefilter_enabled` |
| sortie | `webhook_turn_outcome` | après `run_agent` | `agent_id`, `request_id`, `outcome` |

Les deux moitiés portent le **même périmètre** — tout tour webhook GitHub
facturé — et le même `request_id`, qui est la clé de jointure. `release` et
`outcome` sont par conséquent orthogonaux par construction : un tour relâché sous
`state_unreadable` porte aussi son issue, et le croisement des deux axes est ce
que la réserve de Mika Prime (AC3 de mika#2855) demande de pouvoir lire.

**`outcome` est attesté par le modèle, jamais déduit de la trace d'outils.** Le
prompt du skill impose une ligne finale `Webhook-Outcome: <valeur>` sur chaque
issue, les arrêts compris ; le moteur lit la **dernière** occurrence du marqueur
et la valide contre un contrat fermé de six termes. Une déduction était
impossible : les arrêts 1 et 2 laissent la même trace d'outils, et 3 et 4 aussi
— deux populations sous un nom, non soustractibles (classe mika#2205). Deux
termes supplémentaires nomment les défauts de l'attestation elle-même :

| valeur | ce qu'elle dit |
|---|---|
| `stop_no_open_pr` | arrêt 1 — aucune PR ouverte sur la tête du `check_suite` |
| `stop_out_of_scope` | arrêt 2 — repo, auteur ou état de PR hors périmètre |
| `stop_already_reviewed_sha` | arrêt 3 — une revue de mika-qa porte déjà ce `commit_id` |
| `stop_verdict_recorded` | arrêt 4 — un verdict d'autorité est déjà sur la tête |
| `review` | le skill `qa-review` a été appelé |
| `review_failed` | la revue n'a pas pu se faire ; opérateur notifié, tâche `blocked` |
| `outcome_absent` | **le tour n'a rien attesté** — jamais un arrêt deviné |
| `outcome_unreadable` | une ligne était là, sa valeur est hors contrat |

`outcome_absent` et `outcome_unreadable` ne sont atteignables par **aucun** texte
du modèle : ce sont des termes du moteur, et c'est une propriété du type (deux
énumérations, une pour ce que le modèle peut écrire, une pour ce qui part sur le
fil). Un tour qui n'atteste rien coûte quand même ses tokens ; il ne dit
simplement plus ce qu'ils ont acheté.

### Comment il se lira, une fois le vol 2 livré

Le vol 1 met la ligne dans le journal. `scripts/measure-webhook-turn-cost` ne la
joint **pas encore** : aujourd'hui elle se lit au `grep`, et la jointure
automatique est le vol 2 (seau `by_agent_trigger_release_outcome`, tours sans
ligne de sortie comptés à part sans être perdus ni fusionnés, corpus gelé mêlant
`state_unreadable` et un arrêt propre). C'est la même séquence que mika#2680 a
elle-même suivie — vol 1 le registre, vol 2 le lecteur.

**Ce que la prochaine fenêtre chiffrera (AC6 de mika#2855) :** sur la cellule
`mika-qa × check_suite_pr_branch` — 36 tours, 10 687 327 tokens d'entrée, 16,31 %
de la fenêtre mesurée — la **part des arrêts par numéro**. Soit, pour chacune des
quatre valeurs `stop_*` : son compte de tours et son coût en tokens, à retrancher
du plafond que le vol 5 a rendu lisible. Les tours sous `review` ne sont pas des
candidats au filtrage : ils sont l'unique entrée autonome de la revue QA sur CI
vert, et la porte mika#2260 les préserve exprès. Les tours sous `outcome_absent`
ne sont candidats à **rien** : ils sont la mesure de la fiabilité de
l'attestation, et c'est leur taux qui décidera s'il faut armer la garde
`[output].required_suffix_lines` (EndTurn #864) — délibérément non armée par le
vol 1, parce qu'une re-sollicitation est un appel LLM supplémentaire sur le tour
même que l'instrument mesure.

### Contrôle positif — l'absence de la ligne est un binaire, pas une issue

**Une fenêtre antérieure au déploiement ne porte aucune ligne
`webhook_turn_outcome`, et cela ne veut pas dire que ses tours n'ont pas eu
d'issue.** C'est le motif mika#2340, et il est plus tranchant ici que pour
`release` : le champ `release` manquant se lit sous `absent_pre_field` parce que
la ligne existe et qu'il suffit de regarder dedans ; ici c'est la **ligne entière**
qui manque. Un lecteur qui compterait ces tours sous `outcome_absent` écraserait
deux populations de sens opposé — *« le modèle n'a pas attesté »* et *« le binaire
servi ne sait pas demander »*. D'où le compte à part du vol 2, et d'où cette
sonde :

```sh
grep -F webhook_turn_outcome "$MIKA_SPIRIT_LOG_FILE" | head
```

Elle doit rendre une ligne après le premier `check_suite` vert servi à mika-qa
**par un binaire postérieur à `make deploy`**. `skills/bundled/` est lu au
**build** (`build.rs` → `BUNDLED_SKILL_MANIFESTS`), puis ensemencé
(`seed_bundled_skills_if_needed`), puis lu au tour suivant : la ligne du prompt
n'existe pour aucun agent avant ce rebuild. Un silence de la sonde se lit donc
dans cet ordre — d'abord *quel binaire tourne*, ensuite seulement *ce que les
tours ont attesté*.
