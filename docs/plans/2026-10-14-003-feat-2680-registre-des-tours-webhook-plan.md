---
title: "feat(webhooks): pré-filtre phase 4 — le registre des tours webhook, vol 1 : le moteur dit ce qui a déclenché le tour qu'il paie (mika#2680)"
type: feat
status: active
date: 2026-10-14
issue: senara-solutions/mika#2680
---

# Phase 4, vol 1 — `webhook_turn_charged` : la moitié manquante du registre

## Pourquoi

mika#2680 **chiffre, il n'implémente pas** : il nomme quatre classes candidates
— (e) `closed`/`merged` sans tâche, (f) `pr_review` sur PR fermée au texte
réécrit, (g) `check_suite` verte de branche côté mika-qa, (h) `issue_comment` /
`issue closed` sans tâche — et pose un critère d'acceptation unique :

> Avant toute implémentation, refaire cette mesure sur une fenêtre comparable
> **après** le déploiement de #2679, pour retirer ce que (b) couvre déjà.

Le commentaire du 2026-10-08 constate que ce critère **reste ouvert** : « il faut
une fenêtre comparable avec de l'activité PR après `e14ace7f` ». Ce plan ne
choisit pas de classe à implémenter. Il livre ce qui manque pour que la mesure 3
soit *prenable* — et il la rend comparable par construction, ce que ni la
mesure 1 ni la mesure 2 ne sont.

## Ce que le groom a établi

**1. Le coût est déjà exact ; le déclencheur, non.** `turn_usage` (mika#1889) est
dégatée de `MIKA_STORE_LLM_CALLS`, porte `input_tokens` et `agent_id`, et est
jointe par `trace_id`. Sur le chemin webhook, `handlers.rs` pose
`trace_id: Some(req.request_id.clone())` dans `AgentParams` — la jointure
coût ↔ événement est donc **exacte, déjà déployée, et gratuite**. Ce qui manque
est de l'autre côté : le seul texte que le moteur persiste est le `req.text`
**d'après handlers**, et les handlers le réécrivent précisément sur la population
étudiée — la classe (f) est littéralement « texte réécrit par le
`verdict_handler` ». Le texte d'origine existe en **un** endroit et **un**
instant : `original_github_text`, au site du pré-filtre (`handlers.rs`, où la
décision de saut est déjà connue).

**2. Les mesures 1 et 2 ne sont pas comparables entre elles.** Mesure 1 : deux
agents, 4 h 30, agrégée par session. Mesure 2 : mika-dev seul, 30 min, appels
puis sessions, rapprochée à la main de `audit_events`. La comparabilité est une
propriété de l'**instrument**, et il n'y a pas d'instrument — c'est la raison de
fond pour laquelle le critère d'acceptation est encore ouvert, davantage que
l'absence d'activité PR.

**3. Aucun pilote dispatché ne peut produire la mesure.** Vérifié dans ce
worktree : `~/.mika/data/mika.db` absent, `~/.mika/logs` absent,
`MIKA_SPIRIT_LOG_FILE` non défini, aucun `/var/log/mika*`. Un plan qui demande
des chiffres à un implement le fait mourir ou fabriquer. **La mesure est un geste
d'orchestrateur sur l'hôte ; le dépôt ne peut livrer que l'instrument.**

Conséquence : le registre est la paire
`webhook_prefilter_skipped` (déjà écrit par `skip_turn`, porte `class`) +
`webhook_turn_charged` (**manquant**, doit porter `trigger`), tous deux clés sur
`request_id`. Toute phase future obtient son avant/après de cette paire au lieu
d'un rapprochement manuel qui ne se compare à rien.

## Décision

1. **Une taxonomie pure du texte d'ORIGINE**, dans le module existant
   `server/webhook_prefilter.rs` : `enum TriggerClass` + `fn as_str` (la forme
   de `PrefilterClass`), et `pub(crate) fn trigger_class(text: &str) ->
   TriggerClass`. Termes couvrant la totalité des formats que
   `mika-gateway::github::format_event_text` émet : `check_suite_default_branch`,
   `check_suite_pr_branch`, `pr_review`, `pr_opened`, `pr_closed`, `pr_other`,
   `issue_labeled`, `issue_closed`, `issue_other`, `issue_comment`,
   `push`, `other`. **Classer, pas décider** : aucune consultation d'état, aucun
   appel GitHub, aucune lecture de base — la fonction est pure et testable sans
   bouchon.
2. **Zéro nouveau littéral de grammaire.** Les préfixes manquants (`Issue
   opened:`, `Issue closed:`, `PR opened:`, `PR closed:`, `New comment on `)
   sont déclarés dans `mika-common::github_event_format`, dont l'en-tête dit
   déjà être la source unique et dont le test cross-crate dans
   `mika-gateway::github::tests` refuse la dérive producteur/consommateur. La
   taxonomie les lit par constante ; la garde canonical-tokens (mika#2201) ne
   voit donc aucun site nouveau, puisqu'un `starts_with(CONST)` ne porte pas de
   littéral. Les constantes locales déjà présentes
   (`upstream_close_handler`, `draft_pr_opened_handler`,
   `milestone_context_handler`, `webhook_dispatch`) sont **laissées en place** :
   les rerouter est un refactor qui n'appartient pas à ce vol.
3. **Une ligne INFO, au seul endroit qui connaît les deux faits.** Au site du
   pré-filtre, quand le tour part : `info!(event = WEBHOOK_TURN_CHARGED_EVENT,
   agent_id, request_id, trigger, text_touched, candidate, prefilter_enabled)`.
   - `trigger` : le terme de la taxonomie, calculé sur `original_github_text` —
     **le texte d'avant handlers**, seule forme qui classe (f) correctement ;
   - `text_touched` : `Touched::by_any_handler`, le fait qui distingue
     « intact » de « réécrit » ;
   - `candidate` : le variant de `Candidate` quand `classify` en a rendu un,
     **absent** sinon. Absent veut dire « l'événement n'est pas dans la
     population du pré-filtre », ce qui n'est pas la même affirmation que « il y
     est et a gardé son tour » — motif mika#2331, jamais `0` ni `"none"` ;
   - `prefilter_enabled` : l'état de l'interrupteur, parce qu'une fenêtre où il
     est désarmé est exactement une fenêtre qu'on veut mesurer.
   La ligne est **dégatée** de `MIKA_STORE_LLM_CALLS` comme `turn_usage`,
   `target: "mika::otel"`, et n'écrit **aucune** ligne d'audit : elle décrit un
   tour qui a lieu, pas une décision du moteur — `audit_events` reste réservé
   aux décisions (`webhook_prefilter_skipped` garde son monopole).
4. **Constante, écrivain unique.** `pub(crate) const WEBHOOK_TURN_CHARGED_EVENT:
   &str = "webhook_turn_charged";`, dans le module, avec le commentaire
   « SEUL ÉCRIVAIN » qu'y porte déjà `PREFILTER_SKIPPED_TOOL`.
5. **Le moteur ne change pas de comportement.** Le site actuel est
   `if let Some(original) = &original_github_text && skip_turn(..).await
   { return; }` ; le `&&` court-circuite, donc la branche « payé » n'existe pas
   encore comme bloc. Restructuration minimale : lier le booléen, émettre quand
   il est faux, `return` quand il est vrai. Aucune classe de pré-filtre ajoutée,
   aucun `Touched` modifié, aucun handler touché. **C'est ce qui respecte le
   « hors périmètre : implémenter » du ticket** : le seul code moteur de ce vol
   est une ligne d'observabilité, et chiffrer l'exige.
6. **Périmètre de la ligne : les tours webhook.** Elle n'est émise que là où
   `original_github_text` est `Some`, c'est-à-dire `req.channel == "github"`.
   Les tours Telegram et les tours silencieux ne sont pas des tours webhook et
   restent hors registre, comme dans les mesures 1 et 2.
7. **Contrôle positif (mika#2205 / mika#2340).** Pour un `trace_id` de canal
   `github`, `turn_usage` présent **sans** `webhook_turn_charged` signifie
   « binaire déployé antérieur au champ », jamais « aucun tour webhook ». Cette
   lecture est consignée dans la doc compound du module ; le vol 2 en fait un
   refus explicite de l'instrument.

## Découpage en vols

Le dispositif complet (taxonomie + ligne + lecteur + mesure 3) est estimé à
≈ 765 lignes, au-dessus de la borne par vol. Il se coupe en deux, et **l'ordre
est contraint, pas arbitraire** : un instrument n'a rien à lire tant qu'aucun
binaire déployé n'écrit le champ (chaîne `rebuild → seed → read`, mika#2340 — et
la mesure 2 le constate déjà : « le déploiement de `e14ace7f` date de 20:26:07Z,
aucun appel LLM depuis »).

- **Vol 1 — le champ (CE plan).** Taxonomie, constantes de préfixe, ligne
  `webhook_turn_charged`, tests unitaires et un test d'intégration. Mergeable
  seul, déployable seul, utile seul : dès le déploiement, chaque tour webhook dit
  ce qui l'a déclenché.
- **Vol 2 — le lecteur.** `scripts/measure-webhook-turn-cost` (forme canonique de
  `scripts/measure-empty-turns` : prédicat ordonné documenté, fail-safe toujours
  vers `undetermined`, exit 0 = corpus lu / exit 2 = rien mesuré, bornes refusées
  plutôt que mal répondues), son test shell, `docs/eval/mika-2680/README.md`,
  puis la mesure 3 exécutée par l'orchestrateur sur l'hôte. C'est elle qui ferme
  le critère d'acceptation du ticket.

Le vol 1 porte `Refs #2680`, **pas** `Closes` : le ticket reste ouvert sur son
critère de mesure.

Tracked in: senara-solutions/mika#2680 — vol 2 (« le lecteur »)

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-common/src/github_event_format.rs` — préfixes manquants + extension du test cross-crate | 45 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` — `TriggerClass`, `as_str`, `trigger_class`, constante, émission | 100 |
| `crates/mika-agent/src/server/handlers.rs` — liaison du booléen de saut, appel de l'émission | 25 |
| tests unitaires de la taxonomie (`webhook_prefilter.rs` § tests) | 100 |
| test d'intégration (`crates/mika-agent/src/server/mod.rs`) | 75 |

Total estimé : 345 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

## Fichiers

- `crates/mika-common/src/github_event_format.rs` — cinq préfixes manquants ;
  contrat producteur/consommateur étendu.
- `crates/mika-gateway/src/github.rs` — aucune modification de format ; seul le
  test cross-crate existant gagne des cas.
- `crates/mika-agent/src/server/webhook_prefilter.rs` — `TriggerClass`,
  `trigger_class`, `WEBHOOK_TURN_CHARGED_EVENT`, l'émission, les tests unitaires.
- `crates/mika-agent/src/server/handlers.rs` — restructuration minimale du site
  de saut (point 5).
- `crates/mika-agent/src/server/mod.rs` — test d'intégration.
- `docs/solutions/best-practices/un-pre-filtre-de-webhook-se-place-apres-les-handlers-2026-10-07.md`
  — le registre (les deux lignes, leur jointure, la lecture du point 7) ajouté à
  la doc compound du module ; la sonde AC5 y gagne son volet « tours payés ».

## Fire-Disposition

**Option (a) — exception nommée en allowlist, et l'allowlist est vide.**

Les détecteurs livrés sont les tests unitaires de la taxonomie et le test
d'intégration. Ils portent **exclusivement sur du code créé par ce vol**
(`trigger_class`, la ligne d'émission) : il n'existe donc, par construction,
aucune violation préexistante à excepter, et l'ensemble d'exceptions est **vide
à la livraison**.

Il est vide, pas absent : c'est l'emplacement où la prochaine infraction se
déposera nommée plutôt que tolérée (mika#2323 — « quand cette garde tire, on
déclare le site ; on ne l'allowliste pas »). Deux conséquences d'implémentation :

- **Exhaustivité, pas allowlist.** `trigger_class` n'a pas de liste
  d'exclusion ; son terme de repli est `other`, et un test énumère les six
  formats que `format_event_text` émet pour exiger qu'aucun ne retombe sur
  `other`. Un format ajouté côté gateway sans terme ici rougit ce test — c'est
  la déclaration du site, pas son exemption.
- **Les ajouts à `scripts/canonical-tokens.tsv` sont attendus nuls** (point 2 :
  lecture par constante). S'il s'en avère un nécessaire, c'est une
  **déclaration** dans ce fichier, jamais une entrée dans
  `ALLOWED_UNDECLARED_SITES`, qui reste vide.

## Definition of Done

- Tout tour webhook payé émet exactement une ligne `webhook_turn_charged`
  portant `trigger`, `text_touched`, `prefilter_enabled`, et `candidate` quand
  il y en a un ; un tour écarté n'en émet aucune (c'est
  `webhook_prefilter_skipped` qui parle).
- `trigger` est calculé sur le texte **d'avant handlers** : un `pr_review`
  réécrit par le `verdict_handler` est classé `pr_review`, pas `other` — la
  mutation qui le calcule sur `req.text` est vue rouge et consignée dans le
  corps de la PR.
- Aucun changement de comportement du moteur : mêmes décisions de saut, mêmes
  `Touched`, aucune classe ajoutée, aucune ligne d'audit nouvelle.
- `cargo test -p mika-agent`, `cargo test -p mika-common`,
  `cargo test -p mika-gateway`, `cargo clippy`, les lints du dépôt et
  `scripts/verify-pipeline.sh origin/main` verts.
- Doc compound du module étendue (registre + lecture du point 7).
- PR : `Refs #2680`, vol 2 nommé par la ligne `Tracked in:`,
  `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

- [ ] AC1 — un tour webhook payé émet une ligne `webhook_turn_charged` dont le
  `request_id` est **le même** que le `trace_id` de la ligne `turn_usage` du
  même tour, de sorte que coût et déclencheur se joignent sans rapprochement
  manuel. Vérifié par le test d'intégration.
- [ ] AC2 — `trigger` est dérivé du texte d'origine : un `pr_review` dont le
  texte a été réécrit par le `verdict_handler` (classe (f)) et une
  `check_suite` enrichie par `ci_success_handler` sont classées par leur
  événement réel, et `text_touched` vaut `true`.
- [ ] AC3 — la taxonomie est exhaustive sur les six formats émis par
  `mika-gateway::github::format_event_text` : aucun ne retombe sur `other`.
- [ ] AC4 — un tour écarté par le pré-filtre n'émet **pas**
  `webhook_turn_charged` ; `webhook_prefilter_skipped` reste sa seule trace.
- [ ] AC5 — `MIKA_WEBHOOK_PREFILTER` désarmé : le tour a lieu, la ligne est
  émise, `prefilter_enabled` vaut `false`. Le registre ne dépend pas de
  l'interrupteur.
- [ ] AC6 — `candidate` est **absent** quand `classify` ne rend rien, et jamais
  `0` ni `"none"` (mika#2331).
- [ ] AC7 — la lecture « `turn_usage` sans `webhook_turn_charged` pour un
  `trace_id` de canal `github` ⇒ binaire antérieur au champ » est écrite dans la
  doc compound du module.
- [ ] AC8 *(vol 2, suivi — transcrit du ticket)* — refaire la mesure sur une
  fenêtre comparable **avec activité PR** après `e14ace7f`, pour retirer ce que
  la classe (b) couvre déjà. Produite par
  `scripts/measure-webhook-turn-cost` sur l'hôte, pas par un pilote dispatché
  (constat 3). **Hors périmètre du vol 1.**

## Constats de grooming sur (e), (f), (g), (h)

Consignés ici pour que la phase qui implémentera n'ait pas à les retrouver. Ce
vol n'en implémente aucun.

- **(f) est moins cher que les 250–400 lignes estimées par le ticket.** La
  variante `verdict_pr_closed` existe déjà dans `PrefilterClass` ; (f) est la
  **même** classe avec le terme `Touched::after_verdict_handler` relâché quand
  la forge dit la PR fermée ou mergée. C'est un terme, pas une classe.
- **(g) ne peut pas être couvert par (b), structurellement.** Le terme
  « décision attestée » de (b) exige que l'événement ait écrit une attestation
  DECISION-CORE ; mika-qa n'en écrit jamais (étape 1b du handler sort tôt,
  l'agent n'est pas le dispatcher). (g) a donc besoin de son propre terme — et
  le registre répond d'abord à la question ouverte du ticket : « que fait
  mika-qa de ce tour ? ».
- **(e) a besoin d'un fait de forge ou d'une trace de handler.** Le format
  gateway `[GitHub] PR closed:` **ne distingue pas** mergée de fermée ; la
  taxonomie de ce vol ne le prétend pas et rend `pr_closed` dans les deux cas.
- **(h) est 79 % de la mesure 2 et porte une question non tranchée** — « que
  doit faire le tour quand un commentaire d'opérateur arrive sur un ticket sans
  tâche ? ». C'est une question de produit, pas de filtre, et le registre la
  cadre sans y répondre : `issue_comment` et `issue_closed` sont des termes
  distincts, donc le coût de (h) sera lisible classe par classe.

## Hors périmètre

- **Implémenter une classe de pré-filtre**, quelle qu'elle soit. Le ticket
  chiffre ; ce vol outille le chiffrage.
- **La mesure elle-même** (vol 2 + geste d'orchestrateur, constat 3).
- **Le tour de revue QA**, légitime (dit par le ticket).
- **La renotification opérateur à chaque retraitement DECISION-CORE**, nommée
  dans #2679, candidate à un ticket à part.
- **Rerouter les constantes de préfixe locales** des quatre handlers vers
  `github_event_format` (refactor sans rapport avec la mesure).
- **Les tours non-webhook** (Telegram, silencieux, heartbeat) : hors registre.
