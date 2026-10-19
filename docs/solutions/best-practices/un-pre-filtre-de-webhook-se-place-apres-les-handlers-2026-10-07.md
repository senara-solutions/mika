---
title: Un pré-filtre de webhook se place après les handlers déterministes, pas avant
date: 2026-10-07
category: best-practices
module: crates/mika-agent/src/server
problem_type: best_practice
component: agent-core
severity: medium
tags: [webhooks, llm-cost, prefilter, observability, mika-2675, mika-2680]
applies_when:
  - "Vouloir écarter un événement webhook sans tour LLM"
  - "Ajouter une garde qui court-circuite run_agent_for_message"
  - "Mesurer les tours LLM « à vide » par classe d'événement"
  - "Chiffrer ce que coûte une classe d'événement AVANT d'écrire son filtre"
---

# Un pré-filtre de webhook se place après les handlers déterministes, pas avant

## Le problème

Mesuré le 2026-10-07 sur `messages` × `llm_calls` (30 h) : chaque `check_suite`
verte sur `main` coûtait deux tours LLM (mika-qa ≈ 44 k tokens d'entrée,
mika-dev ≈ 55 k), seize sessions sur la fenêtre, toutes concluant « CI green on
main. No action ». Les labels `bug`, `p1-important`, `p2-normal`,
`loop-substrate` coûtaient chacun un tour mika-dev (≈ 65 k) qui concluait « pas
de `ready`, pas de dispatch » — ce que le prompt `self-dev` prescrit déjà.

## La décision qui ne va pas de soi : APRÈS les handlers

Le réflexe est de filtrer en tête de `run_agent_for_message`, comme la garde de
tête périmée de mika#2671 B2. C'est faux ici : B2 annule un tour **entier**,
alors qu'un événement « sans suite » pour le modèle peut encore avoir une suite
pour le moteur. Les handlers structurels (`ci_success_handler` évalue le merge,
`verdict_handler` notifie l'opérateur d'un `hold[review]`, `upstream_close`
nettoie des lignes) doivent tourner. La porte vit donc entre le dernier handler
et `agent::run_agent` : **le filtre ne change pas ce que le moteur fait, il
change seulement si l'on demande ensuite au modèle.**

Corollaire : les classes qui ne dépendent pas d'un handler exigent un **texte
intact**. Si un handler a remplacé ou enrichi `req.text`, il a quelque chose à
dire au modèle, et le tour a lieu. Sur `main`, l'évaluateur de mika-dev rend
`Passthrough { enrichment: None }` (pas de PR ouverte) et celui de mika-qa sort
à la porte mika#2260 sans toucher au texte : les deux restent filtrables.

## Ce qui reste fail-safe

Interrupteur `MIKA_WEBHOOK_PREFILTER` désarmé (trois paliers, via
`qa_head_supersession::parse_switch`), texte touché, recherche de tâche en
erreur : le tour a lieu. Un faux « écarter » perd un événement ; un faux
« laisser passer » coûte un tour. L'asymétrie décide.

## L'ensemble des labels lus est composé, pas recopié

`webhook_dispatch::label_read_by_loop` compose `READY_LABEL`,
`OPERATOR_HELD_LABELS`, le préfixe de siège `DISPATCH_SEAT_LABEL_PREFIX`, le
préfixe `operator-` et les labels d'automatisation. Large à dessein : un lecteur
qui fait grandir sa liste fait grandir l'ensemble dans la même édition.

Le critère est l'**événement d'ajout**, pas l'existence du label comme état :
`p1-important` ou `bug` sont lus comme état par le feeder et dispatch-lib, mais
leur ajout ne met rien en mouvement — d'où leur filtrage. Écrire « un label lu
n'est jamais filtré » serait faux d'après le code ; la revue l'a relevé.

## La sonde (AC5)

Les écarts, par classe et par agent :

```sql
SELECT agent_id, after_value AS classe, count(*)
  FROM audit_events
 WHERE tool_name = 'webhook_prefilter_skipped'
   AND created_at > strftime('%Y-%m-%dT%H:%M:%SZ','now','-18 hours')
 GROUP BY 1, 2;
```

Les tours à vide RESTANTS, par type d'événement (mêmes classes, même fenêtre),
à comparer à la mesure fondatrice :

```sql
WITH ev AS (
  SELECT m.session_id, s.agent_id, substr(m.content, 1, 60) AS head
    FROM messages m JOIN sessions s ON s.id = m.session_id
   WHERE m.role = 'user'
     AND m.created_at > strftime('%Y-%m-%dT%H:%M:%SZ','now','-18 hours')
     AND (m.content LIKE '[GitHub] Check suite success on % (branch: main)%'
          OR m.content LIKE '[GitHub] Issue labeled %')
)
SELECT ev.agent_id, ev.head, count(DISTINCT l.session_id) AS sessions,
       sum(l.input_tokens) AS tokens_entree
  FROM ev JOIN llm_calls l ON l.session_id = ev.session_id
 GROUP BY 1, 2 ORDER BY tokens_entree DESC;
```

Cible : ≥ 80 % de baisse des tokens d'entrée des classes filtrées. Un label lu
par la boucle (`ready`, `blocked`, `operator-*`, …) doit, lui, continuer
d'apparaître dans la seconde requête : c'est le contrôle négatif.

## Phase 2 : « texte intact » est relatif au handler qui a déjà parlé

La règle de la phase 1 excluait la classe (c) par construction : sur un
`pr_review` sans tâche, le `verdict_handler` **enrichit ou remplace toujours**
le texte (`hold_review_without_task`, `refuse_pass_from_non_reviewer`, le
pré-digest du verdict illisible). Ce n'est pas un signal « il a quelque chose à
dire au modèle » : ce qu'il dit est précisément « rien d'actionnable ». La porte
compare donc, pour (c), le texte final au texte **juste après** le
`verdict_handler` — un handler ultérieur qui y touche garde le tour. Une seule
règle, paramétrée par le handler responsable de la classe ; pas une exception.

Trois décisions qui ne vont pas de soi :

- **Une revue sans ligne `VERDICT:` n'est pas un verdict.** `Verdict::Missing`
  confond « ligne illisible » et « pas de ligne » ; seule la première est dans
  (c). La seconde est la forme d'une consigne écrite en revue, et l'identité
  GitHub de l'opérateur est partagée (AC2). `verdict_raw_value` sépare les deux.
- **« Sans tâche active » = `find_active_task_by_pr_url` rend `None`**, tout
  statut non terminal comptant comme actif — plus large que
  `find_task_for_verdict` (`in_progress` seul). Un faux « actif » coûte un tour,
  un faux « inactif » perdrait un événement.
- **La forge en dernier.** Les trois termes lus dans le texte (`hold[review]`
  déjà remis à l'opérateur, auteur ≠ relecteur QA, valeur illisible) décident
  sans `gh` ; `gh pr view --json state` n'est payé que si aucun ne tient.
- **« Non actionnable » = « déjà refermé par quelqu'un ».** Le terme
  d'identité ne couvre que le `pass` d'un non-relecteur, parce que c'est le
  seul que le `verdict_handler` refuse et audite. Un `block[*]` d'une autre
  identité ne déclenche aucune notification ; l'écarter aussi aurait fait
  disparaître sans trace une consigne de l'opérateur (identité partagée).
  Avant d'écarter, vérifier qu'un autre site a déjà dit à quelqu'un ce qui se
  passe — la revue l'a attrapé, pas les tests, qui épinglaient la version large. État
  inconnu, pas de jeton, délai dépassé : le tour a lieu (AC3).

La couture d'état devient un trait (`PrefilterState`) : trois recherches,
bouchonnées en test, aucun appel GitHub réel.

Sonde de la classe (c), par terme :

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'webhook_prefilter_skipped'
   AND after_value LIKE 'verdict_%'
 GROUP BY 1;
```

Contrôle négatif : `grep webhook_prefilter_state_unreadable` avec
`what = "pr_state"` — non vide en continu signifie que la forge n'est jamais
lisible (jeton), donc que le terme « PR fermée » est inerte et ne compte rien.

**Ce que cette règle N'exclut PAS, et qui a été lu à l'envers trois fois**
(mika#2680, vol 4). Le corps de mika#2680 pose une classe (f) — `pr_review` sur
une PR fermée dont le `verdict_handler` a réécrit le texte — en supposant que
*« la règle « texte intact après handler » de la phase 2 exclut ce cas »*. Elle
ne l'exclut pas, et c'est exactement ce que le paragraphe ci-dessus dit :
`text_after_verdict` est pris **après** que le `verdict_handler` a rendu, donc sa
réécriture est **dans** la fenêtre et n'entre pas dans la comparaison. Seul un
handler **ultérieur** garde le tour. Les deux formes que le ticket cite —
`CIRCUIT BREAKER TRIPPED` et `verdict_classification_failed` à valeur illisible —
sont par conséquent déjà écartées par (c), sous `verdict_pr_closed` et
`verdict_unreadable`. Il n'y a pas de classe (f) à écrire. Le constat est gelé
par `mika2680_f_le_pr_review_reecrit_est_deja_ecarte` (quatre assertions, dont un
contrôle négatif de même graine) plutôt que par cette prose, parce qu'un handler
intercalé entre le `verdict_handler` et le pré-filtre le rendrait faux en
silence : le test rougit, le paragraphe ne saurait pas.

## Découpage

Phase 1 : (a) `check_suite` verte sur `main`, (d) label inerte.
Phase 2 (livrée) : (c) `pr_review` non actionnable sans tâche. Phase 3 : (b) `check_suite` verte sur
une PR déjà décidée et notifiée pour cette tête — le SHA n'est pas dans le texte
du gateway ; la décision vit dans `ci_success_handler` après `find_open_pr`.
Phase 4 (mika#2680) : vol 1 l'instrument, vol 2 le lecteur, vol 3 la classe
**(e)** `closed`/`merged` d'une PR sans tâche de boucle, vol 4 le **terme de
relâche** — pourquoi un tour a eu lieu — vol 5 le **croisement** des deux
taxonomies, qui met ce terme au grain où une classe se décide.

Les cinq classes en service, le terme d'état que chacune lit, et le terme de
relâche qu'elle rend quand elle **ne** filtre pas :

| classe | nom de fil (`skipped_by_class`) | déclencheur | le terme d'état lu | terme de relâche (`charged_by_release`) |
|---|---|---|---|---|
| (a) | `green_check_suite_default_branch` | `check_suite` verte sur `main` | `branch_has_active_task` (une tâche de boucle sur la branche) | `branch_task_active`, `state_unreadable` |
| (b) | `green_check_suite_head_decided` | `check_suite` verte sur branche de PR | l'attestation `ci_success_handler_decision_core_hold` (4 termes) | `no_decision_attested`, `head_not_previously_decided`, `state_unreadable` |
| (c) | `verdict_*`, `ci_failure_*` | `pr_review` | `find_active_task_by_pr_url`, puis la forge en dernier | `pr_task_active`, `pr_open`, `no_terminal_trace`, `state_unreadable` |
| (d) | `inert_label` | `issues labeled` | aucun — l'ensemble des labels lus est composé en code | **aucun** — un label inerte est toujours écarté |
| **(e)** | **`pr_closed_no_loop_task`** | **`PR closed` / `merged`** | **`pr_has_active_task`, et rien d'autre** | **`pr_task_active`, `state_unreadable`** |

**La cinquième colonne est la lecture réciproque de la deuxième, pas sa
paraphrase.** `skipped_by_class` nomme ce qu'une porte a reconnu ;
`charged_by_release` nomme la porte qui a relâché. Les deux taxonomies ne se
dérivent pas l'une de l'autre, et c'est pour cela qu'il y a deux seaux dans
l'agrégat : une classe peut relâcher par plusieurs termes, et un même terme —
`state_unreadable` au premier chef — traverse quatre classes sur cinq.

**Mais une ligne de ce tableau ne se décide pas au grain où les deux seaux plats
la chiffrent.** Les deux comptent sur une fenêtre entière, tous agents confondus :
`{head_not_previously_decided: 7}` ne dit pas si ces sept tours sont de `mika-qa`
ou de `mika-dev`, ni ce qu'ils ont coûté — et la ligne (b) ci-dessus se décide
par agent, un fan-out délibéré (mika#1711) n'ayant pas la même valeur des deux
côtés. Le vol 5 de mika#2680 ajoute donc les deux seaux **croisés**,
`by_agent_trigger_release` (avec le coût en tokens, pas seulement le compte de
tours) et `skipped_by_agent_class`, sans toucher d'un octet les deux plats :
`docs/eval/mika-2680/README.md` § 4.

Trois termes n'ont **pas** de ligne dans ce tableau, parce qu'ils sont rendus
avant qu'aucune classe soit décidée : `switch_disarmed` (le coupe-circuit est
baissé), `not_classified` (l'événement est hors population) et `text_touched`
(un handler a réécrit le texte, donc le tour lui appartient). La case vide de
(d) n'est pas un oubli : `decide` ne rend jamais de relâche pour un
`Candidate::InertLabel`, et c'est l'absence de bras `Decision::Llm` sur ce
chemin qui le garantit — pas une convention.

Ces noms sont des clés de `GROUP BY` dans toutes les sondes ci-dessous. Les
renommer casse une population, jamais un test ; `mika2675_valeurs_daudit_format_de_fil`
les épingle pour cette raison. Le côté relâche est épinglé par
`mika2680_la_relache_atteint_chacun_de_ses_dix_termes`, et son exhaustivité est
tenue par un `match` sans bras `_` : un onzième terme ne compile pas tant qu'il
n'a pas son cas.

## Phase 3 : une décision n'est filtrable que si une trace l'atteste, tête et notification comprises

La classe (b) (`check_suite` verte sur une tête déjà décidée et notifiée)
semblait vivre dans les deux couches de dédup par tête de
`ci_success_handler` (2b mémoire, 2c audit). **La mesure l'a déplacée.** Sur
3 jours de `audit_events` × `llm_calls` joints par `trace_id`, les traitements
d'une même tête sont espacés de **plusieurs minutes**, donc au-delà de la
fenêtre de 60 s. Le cas réel est le **retraitement complet** qui redécide
DECISION-CORE, renotifie et rend `Handled`. Exemples : mika#2672@4ab024d à
16:54 puis 17:06, et mika#2674@a634a46 à 20:54 puis 21:05. Le tour LLM qui suit
conclut « Already notified ».

Les dédups 2b/2c **gardent leur tour**. Leur marqueur `processed` est écrit
*avant* l'évaluation, donc il atteste « traitement commencé », jamais l'issue.
Et un `hold` n'est atteint qu'une fois tous les checks terminés, ce qui rend
une dédup < 60 s *après* un hold structurellement improbable.

Ce qui manquait n'était pas un filtre mais une **trace**. La ligne
`human_gate_required` cible `pr:{repo}#{n}` sans SHA et précède l'envoi : elle
ne peut attester ni la tête ni la notification. La phase 3 ajoute donc une
attestation additive, `ci_success_handler_decision_core_hold`. Elle cible
`pr:{repo}#{n}@{sha}`, porte `notified` / `not_notified` et est écrite
**après** l'envoi. Les décisions, la notification et le retour du handler sont
inchangés.

La porte écarte le tour quand quatre termes tiennent, chacun avec sa mutation
vue rouge :

- **décision attestée** : *cet* événement a écrit l'attestation ;
- **même tête** : un *autre* événement porte la même cible exacte ;
- **notification attestée** : cette attestation antérieure dit `notified` ;
- **antériorité** : son `id` est strictement plus petit.

Ce dernier terme vient de la revue. Sans lui, un premier traitement lent
(plus de 60 s de `gh`) pouvait être devancé dans l'audit par un traitement plus
tardif, et sauté, ce qui viole AC2.

Leçon générale : **avant de filtrer « déjà fait », chercher la trace qui dit
*quoi* a été fait, *sur quoi*, et *si c'est arrivé*.** Un marqueur de passage
n'est pas une attestation d'issue.

Sonde de la classe (b), avec les tours qu'elle n'attrape pas encore :

```sql
-- Les écarts (b)
SELECT count(*) FROM audit_events
 WHERE tool_name = 'webhook_prefilter_skipped'
   AND after_value = 'green_check_suite_head_decided';

-- Les attestations posées, par issue de notification
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'ci_success_handler_decision_core_hold' GROUP BY 1;

-- Tours LLM restants sur une tête dont une attestation antérieure notifiée
-- existe (doit tendre vers zéro)
SELECT a.trace_id, sum(l.input_tokens) FROM audit_events a
  JOIN llm_calls l ON l.trace_id = a.trace_id AND l.agent_id = a.agent_id
 WHERE a.tool_name = 'ci_success_handler_decision_core_hold'
   AND EXISTS (SELECT 1 FROM audit_events p
                WHERE p.tool_name = a.tool_name AND p.target_key = a.target_key
                  AND p.after_value = 'notified' AND p.id < a.id)
 GROUP BY 1;
```

**Halte.** Zéro écart **et** zéro attestation ne prouve rien : il faut qu'une
PR DECISION-CORE ait reçu au moins deux `check_suite` vertes sur la même tête
depuis le déploiement. **Hors périmètre, nommé :** la renotification de
l'opérateur à chaque retraitement est inchangée, puisque ce module ne change
pas ce que le moteur fait. La dédupliquer est un ticket séparé.

## Phase 4, vol 1 : le pré-filtre compte ce qu'il écarte, pas ce qu'il laisse passer

Les trois phases ci-dessus ont chacune leur sonde, et toutes lisent la **même**
population : `audit_events` où `tool_name = 'webhook_prefilter_skipped'`. Ce que
le pré-filtre **laisse passer** n'était inscrit nulle part. Le chiffrage de
mika#2680 a donc dû se refaire deux fois à la main — `llm_calls` joint au premier
message `user` de chaque session, puis `substr(content, 1, 60)` taxonomisé à
l'œil — et c'est cette reconstruction, pas le filtre, qui est le coût récurrent :
la mesure 1 et la mesure 2 ont produit des découpages différents (la classe (h)
n'existait pas dans la première) parce que la taxonomie vivait dans la tête de
celui qui lisait.

**Le symptôme à reconnaître : une porte dont on ne sait mesurer qu'un côté.**
Tant qu'il est là, « quelles classes reste-t-il à filtrer ? » n'est pas une
requête, c'est une enquête — et le critère d'acceptation qui en dépend reste
ouvert, ce qu'il est encore à ce jour.

### Une ligne de log, pas une ligne d'audit

`webhook_turn_charged` est un `info!` sur `target: "mika::otel"`, dégaté de
`MIKA_STORE_LLM_CALLS` comme `turn_usage`, et **n'écrit aucune ligne d'audit**.
Ce n'est pas une économie : `audit_events` est réservé aux **décisions** du
moteur, et `webhook_prefilter_skipped` y garde son monopole. Un tour qui a lieu
n'est pas une décision, c'est un fait. Lui donner une ligne d'audit créerait une
seconde population à côté de celle qui nomme les écarts, sur un axe disjoint, et
tout `GROUP BY tool_name` sur la table deviendrait ambigu.

### La jointure : `request_id` ici, `trace_id` là

```
webhook_turn_charged.request_id  ==  turn_usage.trace_id
```

Les deux champs ne portent pas le même nom et portent la même valeur
(`req.request_id`). C'est ce qui rend le chiffrage **une requête** : le registre
dit *de quelle classe* était le tour, `turn_usage` dit *ce qu'il a coûté*, et
rien n'oblige plus à retaxonomiser un `substr(content, 1, 60)`.

Quatre champs, chacun pour une question qui s'est posée :

- **`trigger`** — le terme de taxonomie, calculé sur `original_github_text`,
  c'est-à-dire **le texte d'avant les handlers**. Les deux handlers réécrivants
  remplacent le texte par un pré-digest dont la première ligne est la balise nue
  (`<verdict_handler>`, `<ci_success_handler>`) ; calculé sur `req.text`, le
  `trigger` d'un `pr_review` réécrit vaudrait `other` — exactement la classe (f)
  du ticket, rendue invisible par l'instrument censé la compter. C'est la
  mutation rouge d'AC2.
- **`text_touched`** — `Touched::by_any_handler`. Le fait qui sépare « intact »
  de « réécrit », donc le seul qui dise si la règle de la phase 2 a joué.
- **`candidate`** — la variante de `Candidate` quand `classify` en a rendu une,
  **absent** sinon. Absent veut dire « l'événement n'est pas dans la population
  du pré-filtre » ; ce n'est pas la même affirmation que « il y est et a gardé
  son tour », que le lecteur lit sur le couple `candidate` présent +
  `webhook_turn_charged` émis. Jamais `0`, jamais `"none"` (motif mika#2331).
- **`prefilter_enabled`** — l'état de l'interrupteur. Une fenêtre où il est
  désarmé est précisément une fenêtre qu'on veut pouvoir mesurer : sans ce
  champ, une flambée de tours ne se distingue pas d'une flambée de trafic.

### Le contrôle positif, et ce que son absence veut dire

Pour un `trace_id` de canal `github`, `turn_usage` **sans** `webhook_turn_charged`
signifie « binaire déployé antérieur au champ », jamais « aucun tour webhook »
(classe mika#2205, chaîne mika#2340 `rebuild → deploy`). Le registre ne couvre
que les tours webhook : il n'est émis que là où `original_github_text` est
`Some`, donc `req.channel == "github"`. Telegram et les tours silencieux restent
hors registre **par construction**, et leur silence ne se lit pas comme un
manque.

### La sonde des tours payés

```bash
# Une commande, les deux moitiés (mika#2680 vol 2).
scripts/measure-webhook-turn-cost --since 2026-10-16T13:01:00Z --until <fin>
```

`by_agent_trigger` est la moitié coût (tours × `input_tokens`, par agent ×
déclencheur) et `skipped_by_class` la moitié écarts. La somme des deux est la
population des événements webhook, et c'est la première fois qu'elle est close.

**Cette sonde a remplacé trois tuyaux disjoints, et l'un d'eux était inerte.**
Il lisait le journal sous un nom de variable amputé de son `SPIRIT_`, qui
n'existe nulle part dans ce dépôt. `grep` sur une chaîne vide lit l'entrée
standard, donc la sonde attendait indéfiniment ou rendait zéro ligne, et « zéro
tour payé » se lisait comme un résultat. Deux des trois autres défauts qu'elle
portait sont du même ordre : la moitié coût interrogeait `llm_calls`, dégaté par
`MIKA_STORE_LLM_CALLS` là où `turn_usage` ne l'est pas, et le `//` de jq rend son
alternative sur `false` autant que sur `null`, donc chaque `text_touched: false`
— la valeur qui porte la mesure — ressortait absent. Le protocole complet,
la fenêtre et la halte : `docs/eval/mika-2680/README.md`.

**Halte.** `webhook_turn_charged` absent de tous les journaux alors que des
`turn_usage` de canal `github` existent : **établir le déploiement avant de
conclure quoi que ce soit** sur le volume des tours webhook. Et
`prefilter_enabled=false` sur les lignes d'une fenêtre : la mesure de cette
fenêtre ne dit rien du filtre en service, elle dit ce que coûterait son absence.

### Ce que vol 1 ne fait pas

Aucune classe de pré-filtre n'est ajoutée — ni (e), ni (f), ni (g), ni (h).
Aucun handler n'est touché, aucun `Touched` modifié, aucune décision du moteur
changée. Le ticket chiffre, il n'implémente pas ; vol 1 livre **l'instrument qui
rend le chiffrage prenable**, et vol 2 (« le lecteur ») en fera un refus
explicite si l'instrument manque.

### Observer le pré-filtre coûte deux états process-globaux

Mesurer un tour, en test, demande deux choses que le processus partage : la
variable qui désarme le pré-filtre, et l'abonné `tracing` qui lit la ligne.
L'une et l'autre débordent du test qui les pose, et le débordement **ne
ressemble pas à une casse** : le voisin lésé voit un tour avoir lieu là où il
attendait un écart, ou une capture rendre zéro. Les deux lectures plausibles
sont « le filtre ne filtre plus » et « la ligne n'est pas émise » — les deux
fausses.

- `MIKA_WEBHOOK_PREFILTER` est lu dans l'environnement par `prefilter_enabled()`.
  Le test qui la pose le temps d'un tour désarme le pré-filtre de **tous** les
  tours qui tournent en parallèle.
- L'`Interest` d'un callsite `tracing` est figé par le dispatcher du thread qui
  l'enregistre le **premier**. Un voisin qui l'atteint sans abonné l'éteint
  globalement, capture déjà installée comprise.

La parade est la même pour les deux, et c'est celle de
`install_capturing_subscriber` : `#[serial_test::serial]` doit couvrir les
**poisonneurs** autant que les lecteurs. Un lecteur seul à être annoté laisse la
course entière ouverte. Ici, la frontière est « tout test qui passe par
`run_agent_for_message` » ; les tests qui exercent la porte (`skip_turn`)
reçoivent `enabled` en paramètre et sortent de la contrainte.

Pour le second état, une frontière tenable n'existe pas : `turn_usage` est
atteint par n'importe quel tour d'agent du crate. L'ordre **« tour à blanc,
puis capture »** la rend inutile — l'enregistrement a lieu sans abonné, donc
éteint, et le `rebuild_interest_cache()` de l'installation le rallume sous le
nôtre. Le rebuild réévalue les callsites déjà enregistrés, et ne peut rien pour
ceux qui le seront après : c'est pour cela que l'ordre compte, et pas seulement
la sérialisation.

Mesuré le 2026-10-16 sur le job `Check` de la PR #2705 : quatre échecs, les deux
formes présentes, et **zéro** en local sur la suite complète — les 6 246 tests
étalent l'ordonnancement, un `cargo test -- mika2675 mika2680 mika2683` le
concentre et sort les quatre à coup sûr. Une suite filtrée est donc un
instrument, pas un raccourci.

## Phase 4, vol 3 : la classe (e), et la propriété qui l'autorise

AC2 de mika#2675 excluait `closed`/`merged` du filtre **par principe**. Le vol 3
l'implémente comme classe **(e)** `pr_closed_no_loop_task`. Ce qui a changé n'est
pas le chiffre — n=2, 193 k puis 279 k tokens d'entrée pour un tour déclenché par
le merge d'une PR sans tâche de boucle — c'est la lecture de ce principe.

**Le principe disait l'inverse de la propriété de ce module.** « Un `closed` peut
avoir une suite » est vrai, et c'est précisément pourquoi écarter le **tour** ne
coûte rien : la suite en question est le travail d'un handler, et le filtre vit
*après* les handlers. Sur un `PR closed`, ce travail a un seul site —
`upstream_close_handler::try_handle_upstream_close` (mika#1934 AC4), qui
termine les lignes de suivi laissées `blocked`/`in_progress`. Il est
**side-effect-only** par contrat : les trois autres bras de son `VerdictAction`
sont des `unreachable!`, il n'enrichit jamais le texte, ne rend jamais `Handled`
ni `Dispatched`. Quand la porte décide, ce nettoyage **a déjà eu lieu**.

Généralisation, et c'est la phrase à retenir du module entier : **une classe
exclue « parce que l'événement a une suite » demande qu'on nomme le site de cette
suite, puis qu'on regarde s'il tourne avant la porte.** S'il tourne avant,
l'exclusion protège un tour LLM, pas la suite. AC2 protégeait un tour.

**Un terme de texte, un terme d'état, la fenêtre la plus stricte.** (e) est la
jumelle de (a) par sa forme :

- *texte* — l'URL de la PR, et elle seule. Lue par
  `upstream_close_handler::extract_pr_url`, promu `pub(crate)` plutôt que
  recopié : cette URL est la **clé** de `pr_has_active_task`, donc deux lectures
  qui dérivent sont deux populations de tours qui ne se soustraient plus. C'est
  le motif du § *La jointure* ci-dessus, appliqué à un extracteur.
- *état* — `pr_has_active_task`, et rien d'autre. Pas la ligne `Merged:` que le
  gateway ajoute, pas l'état forge, pas les refs de clôture : la vérité du merge
  et la clôture du ticket appartiennent aux handlers structurels. Pour (e),
  `closed` et `merged` sont le même événement. Relire ce booléen serait redériver
  une vérité qui ne lui appartient pas.
- *fenêtre* — `by_any_handler`, la plus stricte des quatre. Non par prudence
  rituelle : la seule chose que (e) lit dans le texte est l'URL, donc **tout**
  handler qui a réécrit ce texte a pu réécrire la ligne qui porte la clé.
- *fail-safe (AC3)* — URL inextractible ou lecture en erreur ⇒ le tour a lieu,
  sous `webhook_prefilter_state_unreadable` avec `what = "pr_has_active_task"`.
  Ce `what` diffère du `pr_task` de (c) à dessein : deux lectures d'état, deux
  noms, deux greps séparables.

**Deux prémisses de tests existants ont été amendées, pas exceptées.** Les deux
`PR closed` de `mika2675_evenements_legitimes_le_tour_a_lieu` encodaient AC2 et
sont sortis de cette liste ; ils ont désormais leurs propres tests, de part et
d'autre de la seule chose qui les sépare — une tâche de boucle active. Un test
qui rougit parce qu'une décision a changé se réécrit avec la décision ; l'excepter
aurait laissé dans la suite une assertion qui dit le contraire du code.

Contrôle négatif aux deux niveaux (leçon mika#2277) : même texte, même bouchon,
seul l'état diffère — en unitaire sur `decide`, et de bout en bout sur
`LiveState`. Une batterie purement positive est satisfaite par un filtre qui
écarte tout.

Sonde de la classe (e) :

```sql
SELECT count(*) FROM audit_events
 WHERE tool_name = 'webhook_prefilter_skipped'
   AND after_value = 'pr_closed_no_loop_task';
```

**Halte.** Zéro écart ne prouve rien tant qu'une PR a été fermée ou mergée
**sans** tâche de boucle active depuis le déploiement — un merge de PR est rare,
et c'est le coût accepté de cette classe : la population est petite, donc la
sonde est lente à parler. Le contrôle négatif est `grep
webhook_prefilter_state_unreadable` avec `what = "pr_has_active_task"` : non vide
en continu signifie que l'état n'est jamais lisible, donc que la classe est
inerte et ne compte rien.

**Ce que le vol 3 ne fait pas, et pourquoi chaque raison est différente.** (f) est
un résidu renvoyé à la mesure, (g) est bloquée par elle, (h) est posée à
l'opérateur. Le tableau des quatre raisons est au § 9 de
`docs/eval/mika-2680/README.md` ; le § 2 bis du même fichier dit pourquoi aucune
session dispatchée ne peut exécuter cette mesure — le journal n'est pas dans le
jeu de binds du bac à sable, et c'est une décision de mika#2165, pas un oubli.
