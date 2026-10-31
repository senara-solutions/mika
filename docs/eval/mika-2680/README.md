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
| payée | `charged_by_release` | **pourquoi** chacun a eu lieu, par terme de relâche (dix-huit valeurs, vol 4 puis mika#2854 et mika#2917 — table au § *Taxonomie servie*) |
| payée | `by_agent_trigger_release` | les deux à la fois : agent × déclencheur × terme de relâche, **avec le coût en tokens** (vol 5) |
| payée | `charged_by_outcome` | **ce que** chaque tour a fait, par issue attestée (huit valeurs, mika#2855) |
| payée | `by_agent_trigger_release_outcome` | les quatre clés : agent × déclencheur × relâche × issue, **avec le coût en tokens** (mika#2855) |
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

**Trois absences, trois noms, et aucune ne s'ajoute aux autres** (mika#2855).
L'axe de l'issue en apporte une seconde, et elle ne dit pas la même chose que
la première :

| nom | qui le dit | ce qu'il dit |
|---|---|---|
| `absent_pre_field` | le lecteur | la ligne d'**entrée** n'avait pas de `release` — binaire d'avant le vol 4 |
| `outcome_absent` | le **moteur** | la ligne de sortie est là, et le modèle n'y a rien attesté — c'est une valeur du contrat |
| `outcome_line_absent` | le lecteur | aucune ligne de **sortie** ne porte ce `request_id` — binaire d'avant mika#2855 (motif mika#2340) |

Les deux dernières se ressemblent et mesurent des choses opposées :
`outcome_absent` est une mesure du **comportement du modèle** — son taux décide
s'il faut armer `[output].required_suffix_lines` — tandis que
`outcome_line_absent` est un fait de **déploiement**, vrai pour tous les tours
d'un binaire à la fois. Les sommer donnerait deux populations sous un nom
(classe mika#2205) et ferait lire « le modèle n'atteste presque jamais » sur
une fenêtre à cheval sur un `make deploy`. Elles ont chacune leur clé dans
`charged_by_outcome`, et l'assertion **N12** du § 5 du harnais les tient
séparées sur un corpus où les deux tours sont par ailleurs identiques. Sur la
ligne par tour, l'absence de jointure reste `null` : nommer le seau est le
travail de l'agrégat, pas celui de la ligne. Trois compteurs de lignes
accompagnent l'axe — `outcome_lines`, `outcome_lines_unusable` (ni
`request_id` ni `outcome` exploitable) et `outcome_lines_orphan` (un
`request_id` qu'aucun tour payé de la fenêtre ne porte) : ils disent la santé
du **journal**, pas celle des tours, et ne portent donc aucune valeur de coût.

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

### Taxonomie servie

Les deux tables ci-dessous déclarent **toute** la taxonomie que le moteur sert
dans `skipped_by_class` et `charged_by_release`. Elles ne sont pas une
commodité de lecture : un détecteur les lit à l'exécution de la suite
(`webhook_prefilter.rs § mod tests`, vol 7) et confronte chaque table à l'`enum`
correspondante **dans les deux sens**. Une clé servie et non déclarée fait
rougir la suite ; une ligne déclarée que le moteur ne sert plus la fait rougir
aussi. C'est ce second sens qui fait que ces tables se nettoient d'elles-mêmes
au lieu de devenir un inventaire périmé (mika#2309).

Les marqueurs HTML qui encadrent chaque table sont l'ancrage du détecteur. Ne
les retire pas, et ne pose pas de clé hors de leur intervalle : le document cite
ailleurs des termes côté lecture qui ressemblent à des clés sans en être
(`absent_pre_field`, `outcome_line_absent`), et le § 10 énumère les huit valeurs
d'un **troisième** axe, `charged_by_outcome`, que ces tables ne couvrent pas.

**Axe `skipped_by_class` — les quatorze classes d'écart (`PrefilterClass`).**

<!-- taxonomie-servie:debut axe=classe -->

| clé de fil | axe | ce que la classe écarte |
|---|---|---|
| `green_check_suite_default_branch` | `skipped_by_class` | (a) `check_suite` verte sur la branche par défaut, aucune tâche active sur cette branche. |
| `inert_label` | `skipped_by_class` | (d) un label qu'aucune partie de la boucle ne lit. |
| `verdict_hold_tracked` | `skipped_by_class` | (c) `hold[review]` sans tâche : `hold_review_without_task` a déjà notifié l'opérateur et écrit sa ligne d'audit. |
| `verdict_non_reviewer` | `skipped_by_class` | (c) un `pass` posé par une identité qui n'est pas le relecteur QA. |
| `verdict_unreadable` | `skipped_by_class` | (c) la ligne `VERDICT:` est présente mais illisible. |
| `verdict_pr_closed` | `skipped_by_class` | (c) la PR est fermée ou mergée côté forge : il n'y a plus rien à décider. |
| `green_check_suite_head_decided` | `skipped_by_class` | (b) `check_suite` verte sur une tête déjà décidée par DECISION-CORE et notifiée. **Se somme avec (g′)** — voir § 11. |
| `red_check_suite_default_branch` | `skipped_by_class` | (a rouge) `check_suite` en échec ou `timed_out` sur la branche par défaut, sans tâche active (mika#2683). |
| `ci_failure_no_open_pr` | `skipped_by_class` | (c) CI rouge d'une branche qui n'a pas de PR ouverte. |
| `ci_failure_surfaced_for_adoption` | `skipped_by_class` | (c) CI rouge d'une PR sans tâche : `surface_for_adoption` a déjà notifié (mika#1745). |
| `ci_failure_task_not_in_progress` | `skipped_by_class` | (c) CI rouge d'une PR dont la tâche n'est pas `in_progress`. |
| `pr_closed_no_loop_task` | `skipped_by_class` | (e) `closed`/`merged` d'une PR sans tâche de boucle active (mesurée à n=2 : 193 k puis 279 k tokens d'entrée). |
| `comment_triaged_note` | `skipped_by_class` | (h) `issue_comment` ou `issues closed` sur un ticket sans tâche de boucle, dont le triage n'a trouvé **aucune décision actionnable**. Seule classe dont la décision passe par un modèle ; livrée **désarmée** (mika#2854). |
| `green_check_suite_head_already_reviewed` | `skipped_by_class` | (g′) `check_suite` verte sur une tête que le skill `qa-review-webhook-success` a **déjà traitée**, lue sur le registre que ce skill écrit (mika#2917). **Se somme avec (b)** — voir § 11. |

<!-- taxonomie-servie:fin axe=classe -->

**Axe `charged_by_release` — les dix-huit termes de relâche (`Release`).**

<!-- taxonomie-servie:debut axe=relache -->

| clé de fil | axe | pourquoi le tour a lieu |
|---|---|---|
| `switch_disarmed` | `charged_by_release` | L'interrupteur du pré-filtre est désarmé : `skip_turn` rend la main sans rien classer. |
| `not_classified` | `charged_by_release` | `classify` n'a reconnu aucune population candidate. Le gros du trafic, et le dénominateur de toute mesure de couverture. |
| `text_touched` | `charged_by_release` | Un handler a réécrit le texte dans la fenêtre que la classe exige : le tour a lieu parce que le texte n'est plus celui que la classe sait lire. |
| `branch_task_active` | `charged_by_release` | (a) une tâche vit sur la branche par défaut : son état est à prendre en compte. |
| `pr_task_active` | `charged_by_release` | (c) revue et (e) `PR closed` : une tâche vit sur la PR. Deux sites, un seul terme — même prédicat, même clé. |
| `pr_open` | `charged_by_release` | (c) revue : la PR est `Open` et aucun des termes lus dans le texte n'a mordu. |
| `no_terminal_trace` | `charged_by_release` | (c) CI rouge : l'audit ne porte aucune trace terminale reconnue. |
| `no_decision_attested` | `charged_by_release` | (b) aucune ligne DECISION-CORE dans l'audit : il n'y a aucune tête à comparer. |
| `head_not_previously_decided` | `charged_by_release` | (b) la tête n'a pas d'attestation antérieure : c'est son premier traitement. |
| `state_unreadable` | `charged_by_release` | Un état est illisible et le tour a lieu (AC3, fail-open). **Un seul** terme pour les cinq sites d'appel : le champ `what` du `warn!` nomme déjà le prédicat. |
| `triage_escalated` | `charged_by_release` | (h) le triage a lu une décision actionnable. **Numérateur du taux d'escalade** : `triage_escalated / (triage_escalated + comment_triaged_note)`. |
| `triage_unreadable` | `charged_by_release` | (h) le triage n'a rendu aucun verdict exploitable (transport, délai, hors grammaire, aucun fournisseur) : fail-open. |
| `issue_task_active` | `charged_by_release` | (h) un vol est déjà en cours sur ce ticket : aucun triage n'est appelé. |
| `triage_disarmed` | `charged_by_release` | (h) `MIKA_WEBHOOK_COMMENT_TRIAGE` absent ou `off` : la classe est livrée désarmée (mika#2854). |
| `closure_snapshot_absent` | `charged_by_release` | (h) la moitié fermeture est armée et pourtant aucun instantané n'existe : le site de capture n'a pas tourné. **Zéro attendu** — voir § 11. |
| `reviewed_head_absent` | `charged_by_release` | (g′) le payload nomme une PR et sa tête, et le registre du skill ne porte aucune ligne pour cette tête. **Terme nominal du régime**, pas une panne — voir § 11. |
| `reviewed_head_out_of_contract` | `charged_by_release` | (g′) le registre a répondu hors contrat : le défaut est chez l'écrivain, pas dans la base. **Zéro attendu** — voir § 11. |
| `reviewed_head_switch_disarmed` | `charged_by_release` | (g′) l'interrupteur de la classe est à `off` : (b) décide seule. **Zéro attendu** — voir § 11. |

<!-- taxonomie-servie:fin axe=relache -->

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

## 9. L'état des quatre classes, après les vols 3 à 7

Le § 7 dit ce que le vol 3 *fait* de l'artefact. Le § 2 bis dit pourquoi
l'artefact n'est pas à sa portée. Ce qui suit est donc l'état réel, classe par
classe — et **les quatre raisons ne sont pas la même raison** : c'est tout
l'intérêt du tableau.

| classe | déclencheur | état après les vols 3 à 7 | ce qui décide de cet état |
|---|---|---|---|
| **(e)** `closed`/`merged` d'une PR sans tâche de boucle | `pr_closed` | **implémentée** (`pr_closed_no_loop_task`) | la mesure 3 ne pouvait pas changer son chiffre |
| **(f)** `pr_review` sur une PR fermée au texte réécrit par le `verdict_handler` | `pr_review` + `charged_by_release` | **couverte par (c) pour les deux formes citées** ; résidu = une décision AC2 et trois chemins non-filtres | le code servi, relu — pas une mesure |
| **(g′)** `check_suite success` sur branche de PR, côté mika-qa | `check_suite_pr_branch` | **implémentée** sous la forme (a) (`green_check_suite_head_already_reviewed`, mika#2917) ; il reste son **AC4**, qui est une mesure et non du code | la forme (a) a été chiffrée faisable, puis livrée ; l'AC4 est un geste opérateur |
| **(h)** `issue_comment` / `issues_closed` sans tâche de boucle | `issue_comment`, `issues_closed` | **implémentée désarmée** (`comment_triaged_note`, mika#2854) : `MIKA_WEBHOOK_COMMENT_TRIAGE` est `off` par défaut | la question posée à l'opérateur a été tranchée par un triage modèle, et l'armement reste sa décision |

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

**(g) — la mesure n'était pas un confort, c'était sa condition.** Son plafond se
superposait à ce que (b) écarte déjà : la mesure 2 le dit en toutes lettres,
aucune `check_suite` de branche de PR n'avait été vue depuis le déploiement de
`e14ace7f`, donc un chiffrage pris là aurait compté comme à filtrer ce que (b)
écarte. Et la question du § 7.2 — *que fait mika-qa de ce tour ?* — porte sur un
agent dont le fan-out est délibéré (mika#1711). Les deux termes manquaient, et
les deux étaient dans l'artefact. Ils y sont depuis le vol 6.

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
la réponse, lue sur le seul agrégat. La mesure 3 est restée la condition de (g) —
les vols 4 et 5 lui ont fourni l'opérande qui manquait et le grain où le lire,
ils ne l'ont pas remplacée.

**Ce que les vols 6 et 7 changent : (g′) est livrée, et ce qui reste est une
mesure.** La mesure 3 a eu lieu le 2026-10-19, hors bac à sable, avec la commande
exacte du § 3 : la cellule `mika-qa × check_suite_pr_branch` est entièrement en
`no_decision_attested`, 36 tours pour 10,69 M tokens d'entrée, soit 16,3 % de la
fenêtre. Bearing Prime a ratifié le filtrage le 2026-10-26 et arbitré la forme :
la forme **(a)** — un pré-filtre qui n'utilise que le payload `check_suite` et le
compare, **en lecture seule**, à l'état de dedup que le skill écrit — a été
chiffrée faisable, et c'est elle que mika#2917 a livrée sous la classe
`green_check_suite_head_already_reviewed`. La classe est donc **du code servi**,
plus une attente de mesure. Ce qui reste d'elle est son **AC4** : lire, sur une
fenêtre servie, que la classe écarte réellement la population visée. C'est une
mesure, pas une implémentation, et le § 11 dit les trois choses qu'il faut savoir
avant de conclure sur elle — au premier rang desquelles que **(b) et (g′) se
somment** : lire (b) seule montrerait un transfert de population comme une
régression (mika#2205).

**(h) — le plus gros volume, et le seul blocage qui n'était pas un chiffre.**
1,30 M tokens, 79 % de la fenêtre de la mesure 2 : le volume était établi. Ce qui
manquait n'était pas une mesure mais une **réponse** : le tour met à jour la
mémoire et le fil de priorités, et savoir si ce travail survit sans tour LLM est
un jugement sur ce que le tour fait, pas sur ce qu'il coûte. Le § 7 le tranchait
en une phrase — *une classe dont le chiffre est significatif mais dont la
question reste ouverte ne s'implémente pas : elle se pose à l'opérateur* — et
elle a été posée dans le corps de PR du vol 3.

**La réponse est venue sous la forme d'un triage, et la classe est livrée
désarmée.** mika#2854 a livré `comment_triaged_note` : sur un `issue_comment` ou
un `issues closed` sans tâche de boucle, un **modèle** lit le fil et dit s'il
porte une décision actionnable. C'est la seule classe du pré-filtre dont la
décision passe par un modèle — d'où `triage_escalated` quand il en trouve une,
`triage_unreadable` quand il ne rend rien d'exploitable (fail-open), et un taux
d'escalade `triage_escalated / (triage_escalated + comment_triaged_note)` qui est
le chiffre à surveiller. Mesure du vol : 13 tours, 1 007 748 tokens d'entrée sur
la fenêtre du 2026-10-19. **`MIKA_WEBHOOK_COMMENT_TRIAGE` est `off` par
défaut** : l'armement reste la décision de l'opérateur, et `triage_disarmed` est
ce qui le rend lisible plutôt que silencieux.

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

### Comment il se lit (vol 2 livré)

`scripts/measure-webhook-turn-cost` joint la ligne de sortie par `request_id`.
Deux temps : l'agrégat, puis la lecture de la cellule.

```sh
scripts/measure-webhook-turn-cost \
    --log-file "$MIKA_SPIRIT_LOG_FILE" \
    --agent mika-qa \
    --since 2026-MM-DD --until 2026-MM-DD \
    --summary docs/eval/mika-2680/summary-gprime-<fenêtre>.json
```

```sh
jq '[.by_agent_trigger_release_outcome[]
     | select(.agent_id == "mika-qa" and .trigger == "check_suite_pr_branch")]
    | { "attestés":   (map(select(.outcome != "outcome_line_absent")) | map(.turns) | add // 0),
        "sans_ligne": (map(select(.outcome == "outcome_line_absent")) | map(.turns) | add // 0),
        "par_issue":  (map(select(.outcome != "outcome_line_absent"))
                       | map({ (.outcome): { tours: .turns, tokens: .input_tokens } })
                       | add) }' \
   docs/eval/mika-2680/summary-gprime-<fenêtre>.json
```

L'axe de l'issue ne remplace rien : `by_agent_trigger_release` garde le plafond
de la cellule au grain du vol 5, et `charged_by_outcome` donne le plat — un
compte de tours par issue, sur toute la fenêtre et tous les agents.

**Troisième règle de halte, propre à cet axe.** Les deux premières sont au § 4.
Celle-ci : **une cellule dont `outcome_line_absent` est non nul mêle deux
régimes binaires** — des tours servis par un binaire qui demande l'attestation,
et des tours servis par un binaire qui ne la demande pas. La part des arrêts
n'y est pas calculable : le dénominateur est `attestés`, **jamais**
`attestés + sans_ligne`, et une part écrite sur le second serait diluée par un
fait de déploiement qui n'a rien à voir avec ce que le modèle fait. Le remède
n'est pas une correction, c'est une **fenêtre** : remonter `--since` au premier
`webhook_turn_outcome` du journal, et refaire la mesure. Tant que `sans_ligne`
n'est pas nul, on écrit la valeur brute et on ne conclut pas.

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

## 11. Lire l'AC4 de (g′) — trois choses à savoir avant de conclure

L'AC4 de (g′) est une **mesure**, pas du code : la classe
`green_check_suite_head_already_reviewed` est servie depuis mika#2917, et ce qui
reste est de lire, sur une fenêtre servie, qu'elle écarte bien la population
visée. Les trois lectures ci-dessous ne sont pas des précautions de style : sans
elles, l'agrégat rend des chiffres qui ressemblent à des résultats et disent le
contraire de ce qui se passe.

### 11.1 (b) et (g′) se somment — la quantité conservée est leur somme

(g′) est consultée **avant** (b). Un `check_suite` vert sur une tête que les deux
classes sauraient écarter s'inscrit donc désormais sous
`green_check_suite_head_already_reviewed`, et plus sous
`green_check_suite_head_decided`. **Le transfert de population est attendu, et il
n'est pas une régression.**

La quantité conservée est :

```
green_check_suite_head_decided + green_check_suite_head_already_reviewed
```

jamais l'une des deux. Un lecteur qui suit (b) seule à travers le déploiement de
mika#2917 verra son compte baisser et lira une régression du filtre là où il n'y
a qu'un déplacement entre deux seaux — exactement la classe d'erreur de
mika#2205 : deux populations sous un même nom ne sont pas soustractibles, et deux
noms dont la somme est l'invariant ne se lisent pas séparément.

Conséquence pratique pour la mesure : si la fenêtre chevauche le déploiement,
`charged_by_release` n'est pas comparable terme à terme de part et d'autre. Prends
une fenêtre **entièrement postérieure**, ou lis la somme.

### 11.2 Trois termes dont le régime attendu est zéro — et ce qu'un non-zéro nomme

Ces trois-là valent **zéro** en régime nominal, et c'est ce zéro qui atteste le
câblage. Un non-zéro ne dit pas « la classe filtre mal » : il nomme un défaut
précis, et à chaque fois **ailleurs** que là où on le chercherait d'instinct.

| terme | ce qu'un non-zéro nomme | où va le diagnostic |
|---|---|---|
| `reviewed_head_out_of_contract` | l'écrivain a écrit hors contrat — le registre a répondu quelque chose que cette liste ne prévoit pas | dans le **prompt du skill**, pas dans SQLite. Les fusionner avec un autre terme enverrait le diagnostic vers la base quand le défaut est dans le prompt (mika#2205) |
| `reviewed_head_switch_disarmed` | un **armement oublié** : l'interrupteur de la classe est à `off` alors que la mesure la suppose active | dans la configuration servie. Sans ce terme, un désarmement oublié ressemblerait à un registre muet |
| `closure_snapshot_absent` | le **site de capture** de (h) n'a pas tourné, alors que la moitié fermeture est armée | dans le câblage de (h), pas dans son triage |

Un zéro sur ces trois termes est donc une **information positive**, pas une
absence de donnée : il dit que la classe tourne, que l'outil et cette liste
s'accordent, et que le site de capture a bien tourné.

### 11.3 `reviewed_head_absent` est le terme nominal — son effondrement serait le symptôme

`reviewed_head_absent` se lit à l'envers des trois précédents. Il dit : le payload
nommait une PR et sa tête, et le registre du skill ne portait **aucune ligne**
pour cette tête. C'est le cas d'une tête vue pour la première fois — donc **le
régime, et non une panne**. Un volume franc sur ce terme est le signe que la
classe est consultée sur la bonne population.

**C'est son effondrement à zéro qui serait le symptôme à craindre, pas sa
présence.** Un `reviewed_head_absent` qui tombe à zéro sans que
`green_check_suite_head_already_reviewed` monte d'autant veut dire que la classe
n'est plus consultée du tout — et c'est la seule lecture de cet axe où zéro est
une mauvaise nouvelle. Ne la confonds pas avec les trois termes du § 11.2, où
zéro est l'attestation.
