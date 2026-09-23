# mika#2492 — La PR nominale de la boucle autonome n'est pas une épave

**Ticket :** mika issue#2492
**Type :** fix (substrat boucle — classement de la PR produite par `dispatch-lib`)
**Date :** 2026-09-23
**Branche :** `fix/2492/loop-substrate-ouvrir-la-pr-draft-push`
**Parent :** #2491 (umbrella loop-substrate), défaut 1

---

## Problème

### Ce que le ticket affirme

> Le pipeline `/mika` (plan → work → review → resolve-TODOs → doc-audit →
> compound → PR) épuise son budget de tours […] et se termine **avant**
> `git push` + `gh pr create`. Résultat : les commits sont faits mais **aucune
> PR** n'est ouverte — le travail est orphelin dans le worktree, récupéré à la
> main par wip_rescue.

Remède proposé : insérer `git push` + `gh pr create --draft` dans `/mika` juste
après la phase `work`.

### Ce que la mesure déplace — et c'est le premier livrable

Trois faits, lus sur l'arbre et sur les journaux pilote, changent le diagnostic
et donc le remède.

**(M1) Les quatre dispatches mesurés n'ont pas tourné sous `/mika`.** Un ticket
groomé porte un callout `> - **Plan:** \`docs/plans/…\``, et
`dispatch-lib.sh::_detect_plan_on_branch` (mika#1074, `dispatch-lib.sh:7307`)
**surcharge** alors `ENTRY_COMMAND` de `/mika` vers `/ce-work <plan>`. Les
journaux des quatre sessions de la nuit du 2026-09-22 le portent noir sur blanc :

| ticket | log pilote | entry command mesurée |
|---|---|---|
| #2051 | `08f8fe27-…` | `/ce-work docs/plans/2026-09-21-003-fix-2051-egress-mort-avant-bind-plan.md` |
| #2484 | `153c2044-…` | `/ce-work docs/plans/2026-09-22-002-fix-2484-feeder-dispatch-callouts-de-corps-sans-plan.md` |
| #2425 | `d8aa26ee-…` | `/ce-work docs/plans/2026-09-22-001-feat-2425-context-history-per-tenant-plan.md` |
| #2471 | `73573501-…`, `a0886164-…`, `a2c109a6-…` | `/ce-work docs/plans/2026-09-22-002-test-2471-mika-agent-six-bare-bootstrap-agent-plan.md` |

Commande de re-vérification :
`grep -o "ce-work docs/plans/[a-z0-9./-]*" /var/log/claude-pilot/<id>.log`.

**(M2) `/ce-work` n'ouvre pas de PR — par construction, pas par troncature.** Sa
description l'énonce : *« Use when an outer orchestrator needs implementation and
local verification only, **without the shipping tail** »*. dispatch-lib le sait
et l'écrit depuis mika#1383 (`dispatch-lib.sh:4216`) : *« Mode 1 = bare `/ce-work`
launch never had commit→PR in scope »*. **Tout ticket groomé — c'est-à-dire tout
dispatch d'implémentation de la boucle autonome — tourne donc sous un pipeline
qui n'ouvre jamais de PR.** Ce n'est pas un budget épuisé : c'est un périmètre.

Conséquence directe et dirimante sur le remède proposé : **éditer `/mika`
n'atteint aucun des quatre cas mesurés.** Le correctif serait inerte sur sa
propre population de preuve — la classe mika#2340 (« un correctif qu'on n'a pas
déployé se lit exactement comme une flotte saine »), ici sous sa forme « un
correctif déployé sur un chemin que personne n'emprunte ».

**(M3) Le travail n'est pas orphelin.** Le rescue `commit-pushed-no-pr`
(mika#1396, `dispatch-lib.sh:7893`) ouvre une PR draft dès que le pilote a
commité sans PR. Preuve dans l'historique du dépôt :
`git log --all --grep=2051` rend `5c3e6867 wip(mika#1383): auto-PR-create rescue
for mika#2051` et `6e7bc598` — deux passages du rescue. Les PR #2487/#2488/#2489/
#2490 **sont** ces PR de rescue. Rien n'a été perdu ; le geste manuel de Vincent
n'a pas été de retrouver du travail, il a été de le **débloquer**.

### Le défaut réel : le chemin nominal est classé comme une épave

Le rescue fait son travail, mais il le fait sous une étiquette fausse. Une PR de
classe `commit-pushed-no-pr` reçoit quatre marqueurs, et chacun a un consommateur
qui agit :

1. **`RECOVERY_PENDING: true`** dans le RESULT → `self-dev-callback` écrit
   `unpushed_recovery_pending: true` dans `tasks.metadata`
   (`self-dev-callback/system_prompt.md:104`) → **Guard 1** du qa-webhook
   (`self-dev-webhook-qa/system_prompt.md:248`) fait **sauter la revue autonome**
   et escalade à l'opérateur.
2. **Un commit vide `wip(mika#1383)`** posé en tête (`dispatch-lib.sh:7935`) →
   **Guard 2** (`isDraft AND ^wip\(`) fait sauter la revue de son côté aussi.
3. **Le label `wip-rescue`** → le guard moteur mika#1682 **refuse** l'appel
   `gh pr ready` à tout agent (`self-dev-webhook-qa/system_prompt.md:325`).
4. **`Outcome: PIPELINE_INCOMPLETE`** (`dispatch-lib.sh:4428`, dont le texte
   affirme *« Pipeline truncated before git push + gh pr create »*) → mika-dev
   reçoit un échec là où le pilote a fait exactement ce que son périmètre
   prévoyait.

**C'est le geste manuel, et il est structurel, pas accidentel.** La boucle
autonome produit, en régime nominal, une PR que trois gardes indépendantes
refusent de faire avancer et qu'aucun agent n'a le droit de sortir du draft. Le
coût mesuré est celui que le ticket rapporte : quatre PR reprises à la main dans
la même nuit.

### Reformulation

Le ticket a raison sur le symptôme (« il faut une PR reviewable sans geste
humain ») et se trompe sur la cause (« le pipeline `/mika` tronque ») et donc sur
le lieu du remède (« éditer `/mika` »). Le remède est là où la PR est réellement
produite : **dans `dispatch-lib`, qui possède déjà la queue git/PR par le contrat
mika#1271** — et il consiste à **distinguer deux populations que le code
confond** : le pilote tronqué avant sa queue d'expédition (une épave, à traiter
comme aujourd'hui) et le pilote qui n'avait pas de queue d'expédition (le chemin
nominal, à ne plus traiter comme une épave).

---

## Requirements

- **R1.** Le périmètre d'expédition du pilote est un **fait estampillé par son
  producteur**, jamais reconstruit après coup. `_detect_plan_on_branch` sait,
  *avant* le lancement, qu'il pose une entry command sans queue d'expédition :
  c'est lui qui l'écrit.
- **R2.** Une session de classe « sans queue d'expédition » **qui a conclu
  proprement** produit une PR draft qui ne porte **ni** `RECOVERY_PENDING: true`,
  **ni** le commit marqueur `wip(mika#1383)`, **ni** `Outcome:
  PIPELINE_INCOMPLETE`.
- **R3.** Les trois autres croisements (queue présente × conclu, queue présente ×
  tronqué, queue absente × tronqué) sont **byte-identiques** à aujourd'hui.
- **R4.** Tout terme illisible (périmètre non estampillé, statut de session
  indéterminé) **sort de la classe nouvelle** et retombe sur le comportement
  actuel. Un signal illisible n'est jamais un terme satisfait.
- **R5.** La PR de la classe nouvelle reste dans le champ du filet qui sait la
  faire avancer (`wip_rescue.rs` : rebase → clippy → classification de périmètre
  → un-draft). Aucune PR ne devient invisible à tous les filets.
- **R6.** Aucune garde n'est ajoutée, retirée, élargie ou resserrée. Aucune
  valeur de réglage ne bouge.
- **R7.** Le prédicat de complétude de session est **mesuré avant d'être écrit**,
  pas supposé.

---

## Décisions

### D1 — Le remède ne vit pas dans `/mika`, et les trois lieux possibles sont nommés

| lieu | atteint la population mesurée ? | verdict |
|---|---|---|
| `.claude/commands/mika.md` | **non** — elle tourne sous `/ce-work` (M1) | écarté, inerte |
| le prompt de `/ce-work` | hors dépôt (plugin `compound-engineering`) | inatteignable |
| `dispatch-lib.sh` | **oui** — c'est lui qui ouvre la PR aujourd'hui (M3) | **retenu** |

S'ajoute la doctrine maison : `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
(mesurée par mika#2120 — neuf récurrences sous prompt contre zéro écrit à la
main). Un pas d'expédition déplacé dans un prompt est une instruction de plus sur
un chemin qui n'en manquait pas ; ce que le substrat fait, il le fait.

### D2 — Deux axes déclarés, croisement exhaustif, une seule case change

| A — queue d'expédition | B — session conclue | classe | changement |
|---|---|---|---|
| présente (`/mika`) | oui | le pilote a ouvert sa PR | aucun |
| présente | non | `commit-pushed-no-pr` (épave) | **aucun** |
| **absente (`/ce-work`)** | **oui** | **`no-shipping-tail` (nominal)** | **la seule case qui bouge** |
| absente | non | `commit-pushed-no-pr` (épave) | **aucun** |

L'axe B est indispensable, et **la population mesurée porte ses deux états** —
c'est ce que rev 2 a établi en nommant les témoins (U0, finding F3) : #2484 a
fini en `[guardrail] error_max_turns` (`153c2044-….log`) quand #2425 a fini en
`[done] Success | 142 turns` (`d8aa26ee-….log`), tous deux sous `/ce-work`. Un
travail tronqué ne doit pas être présenté comme complet, même quand son périmètre
n'avait pas de queue. **Les deux phénomènes coexistent et sont orthogonaux** ; le
ticket les confond, le croisement les sépare — et la troisième ligne du tableau
n'est pas une hypothèse, elle a déjà un cas.

### D3 — Le label `wip-rescue` est CONSERVÉ, et c'est un arbitrage, pas un oubli

Le réflexe est de retirer le label puisque la PR n'est pas une épave. **Il est
écarté, et son coût est mesuré :** `wip_rescue.rs` n'élit que les drafts portant
ce label (`WIP_RESCUE_LABEL`, `wip_rescue.rs`). Le retirer sort la PR du seul
mécanisme qui la rebase, la passe à clippy, la classifie par périmètre et la
sort du draft — et `qa_review_reconcile` (mika#2334) ne la rattrape pas davantage,
son filtre exigeant `isDraft == false`. La PR deviendrait **invisible à tous les
filets** : un défaut créé par le remède, strictement pire que celui qu'il répare.

Ce que ce plan répare est donc ce qui **bloque** (les trois gardes), pas ce qui
**achemine** (le label). Le label a cessé de signifier « épave » pour signifier
« cette PR a besoin du chemin rebase → clippy → un-draft » ; le renommer ou le
scinder est un travail de vocabulaire avec sa propre population à mesurer —
**suivi nommé**, pas préempté ici.

### D4 — La fenêtre draft est conservée, l'un-draft reste au filet existant

Le DoD du ticket demande « draft d'abord, ready à la fin ». La PR de la classe
nouvelle **s'ouvre draft** : cela préserve l'invariant mika#1941 (« aucun PR ne
quitte draft tant que la revue multi-agent n'est pas postée »), et une
`gh pr create --draft` qui réussit alors qu'un appel ultérieur échoue laisse une
PR récupérable plutôt que rien.

Le passage draft→ready **n'est pas ajouté par ce ticket** : il existe déjà, dans
`wip_rescue.rs`, avec des garanties que dispatch-lib n'a pas au moment où il
ouvre la PR (rebase sur `main` à jour, clippy post-rebase, classification de
périmètre, et pour un DECISION-CORE l'exigence `rescue-pipeline-verified: yes`
de mika#2286). **Le dupliquer dans dispatch-lib serait réimplémenter un filet
pour éviter de le laisser faire son travail.** Ce que ce plan change est que la
PR arrive à ce filet **sans les trois marqueurs qui faisaient sauter sa revue**
une fois un-draftée.

### D5 — `_measure_pipeline_verified` est appelé pour la classe nouvelle aussi

Le producteur de `rescue-pipeline-verified` (mika#2354) tourne déjà juste avant
`gh pr create` et mesure « l'état exact que cette PR est sur le point de
publier ». Rien ne change : la classe nouvelle passe par le même appel. C'est ce
marqueur qui décidera, en aval, si un DECISION-CORE peut être un-drafté — et
c'est précisément la garantie qu'il ne faut pas contourner.

### D6 — Le prédicat de complétude est établi par lecture du dépôt (rev 2)

**Rev 1 de ce plan affirmait que ce prédicat n'était pas établissable par lecture
du code seul, claude-pilot étant hors dépôt, et le renvoyait à une mesure sur
journaux. La lecture faite en rev 2 (F3) réfute les deux moitiés**, et la
correction va dans le sens du durcissement, pas de l'assouplissement :

1. **Le mapping est écrit et consommé dans ce dépôt.** `dispatch-lib.sh:3071`
   pose `STATUS` depuis le JSON du pilote ; `:3074-3087` énonce que
   `status: terminated` couvre **les deux** populations d'arrêt (abort guardrail
   et limite SDK) ; `:3169` embranche dessus ; et `_halt_family` (`:3520`) nomme
   `error_max_turns` comme un `SDK_TERMINATION_SUBTYPES`, table atteinte depuis
   `_classify_terminated_session`, elle-même appelée **uniquement** sous
   `STATUS = "terminated"` (`:3170`, `:3184`). Une session `error_max_turns`
   porte donc `STATUS = "terminated"`, jamais `"success"`. Le prédicat
   `STATUS = "success"` de U2 est **figé** et n'attend plus rien.
2. **La mesure que rev 1 prescrivait était inexécutable.** `STATUS`/`SUBTYPE`
   sont lus du **stdout** du pilote (`:3071`, `:3087`), capturé dans
   `$STDOUT_FILE`, un `mktemp` **supprimé ligne 3068**. Le JSON n'atterrit ni
   dans `<id>.log` ni dans `<id>.stderr` — vérifié sur le cas mesuré #2484 :
   `grep -o '"subtype":"[a-z_]*"'` et `grep -o '"is_error":[a-z]*'` (les deux
   commandes que rev 1 publiait) rendent **zéro ligne** sur les deux fichiers.
   Un plan qui prescrit une mesure impossible ne livre pas une prudence, il
   livre une étape qui sera sautée ou bricolée.

Ce que les journaux archivés portent est un **marqueur de prose**, pas le couple
JSON : `[done] Success | N turns | $C | Ts` d'un côté, `[guardrail] <subtype>: …`
de l'autre. C'est sur lui que porte le contrôle U0 ci-dessous, et il suffit à
nommer les deux populations témoins.

Le fail-safe (R4) reste la protection de fond, et son sens est inchangé : un
doute classe « épave », c'est-à-dire le comportement d'aujourd'hui.

---

## Scope Boundaries

**Dans le périmètre**
- `skills/bundled/_shared/dispatch-lib.sh` : estampille du périmètre, classe
  `no-shipping-tail`, corps de PR, ligne `Outcome:` (bras dédié dans le bloc
  Unit 3 + nouvelle fonction `_set_outcome_line`, sœur de `_set_pr_status_line`),
  bloc mika#940 Unit 1.
- `skills/bundled/_shared/test-dispatch-lib.sh` : tests + scans structurels.
- `.github/labels.yml` : **vérification** que tout label écrit par le chemin
  modifié y est déclaré (aucun label nouveau n'est introduit).

**Hors périmètre, délibérément**
- `.claude/commands/mika.md` — **non modifié.** Il n'est pas sur le chemin de la
  population mesurée (M1), et le modifier ajouterait une instruction non mesurée
  sur un chemin sain.
- Le prompt `/ce-work` — hors dépôt.
- `wip_rescue.rs` — **non modifié.** Son filtre, son rebase, son clippy, sa
  classification et son un-draft sont inchangés ; ce plan change ce qui lui
  arrive, pas ce qu'il en fait.
- Le budget de tours du pilote et l'« alternative complémentaire » du ticket
  (budget par étape avec push forcé) — autre défaut, autre mesure, **suivi**.
- `CLAUDE_PILOT_REQUIRE_PR=1`, exporté pour dev-pilot **avant** l'override
  d'entry command (`dispatch-lib.sh:7732`) et donc actif sur un `/ce-work` qui
  n'ouvrira jamais de PR. Réelle incohérence, trouvée en chemin, **suivi nommé** :
  son consommateur est dans claude-pilot, hors dépôt, et y toucher changerait la
  classification d'échec de toutes les sessions dev-pilot.
- Le renommage / la scission du label `wip-rescue` (D3) — **suivi**.
- Le faux-étiquetage rescue-class côté `qa-review` Step 1.5, que mika#2334 a déjà
  nommé comme suivi à ouvrir — adjacent, distinct, **suivi**.

---

## Implementation Units

### U0 — Contrôle des deux populations témoins sur journal (précède tout code)

Aucune ligne de code. Le prédicat lui-même est **déjà établi** par lecture du
dépôt (D6) ; ce qui reste est le contrôle qu'il a bien **deux** populations dans
la vraie vie, et que la classe nouvelle n'en absorbe pas une. **Les deux témoins
sont nommés** — c'est la rectification F3 de rev 2, rev 1 se bornant à affirmer
que « les quatre cas mesurés ont tous tronqué », ce qui est faux :

| population | témoin | marqueur terminal mesuré |
|---|---|---|
| **session conclue proprement** | `d8aa26ee-322f-4946-94c9-9d2bfd2565ca.log` (#2425) | `[done] Success \| 142 turns \| $48.50 \| 2950s` |
| **session tronquée** | `153c2044-9d38-4458-abfa-322c3c174020.log` (#2484) | `[guardrail] error_max_turns: SDK limit reached after 201 turns` |

```bash
tail -2 /var/log/claude-pilot/d8aa26ee-322f-4946-94c9-9d2bfd2565ca.log   # [done] Success
tail -2 /var/log/claude-pilot/153c2044-9d38-4458-abfa-322c3c174020.log   # [guardrail] error_max_turns
```

Le témoin « conclu proprement » **existe donc dans la population mesurée
elle-même**, et il porte l'axe A (`/ce-work`, M1) : #2425 est, à la lettre, un
cas de la classe `no-shipping-tail` que ce plan crée. Les deux autres journaux de
M1 (`08f8fe27` #2051, `a0886164` #2471) portent un marqueur `[guardrail]` et
tombent dans la seconde ligne du tableau.

**Le couple JSON `(STATUS, SUBTYPE)` n'est PAS relisible sur ces fichiers** (D6,
point 2) : ne pas le chercher, ne pas en inventer un proxy. Le marqueur de prose
ci-dessus est ce que les journaux portent, et le mapping vers `STATUS` est établi
par le code, pas par eux.

**Halte U0 — trois issues, dont la troisième est celle que rev 1 omettait.**
(i) Le témoin « conclu » manque (journal purgé) → en produire un avec
`grep -l '\[done\] Success' /var/log/claude-pilot/*.log | head`, qui balaie les
~2 800 journaux du répertoire ; ne pas conclure à son absence sans cette commande.
(ii) Les deux témoins rendent le même marqueur terminal → le croisement D2 n'a
pas deux états observables : **ne pas livrer la classe nouvelle sur le seul
axe A**, ce qui ferait traiter une session tronquée comme nominale ; le travail
s'arrête et remonte à l'opérateur (`## Fire-Disposition`, branche c).
(iii) Le mapping de D6 est contredit par la lecture (un `[done] Success` dont la
session serait classée `terminated`, ou l'inverse) → **halte** : le prédicat de
U2 est faux et c'est lui qu'il faut refaire, jamais le contourner par un proxy.

Écrire le résultat de U0 dans la description de PR, quel qu'il soit.

### U1 — L'axe A est estampillé par son producteur

Dans `dispatch_claude_pilot`, bras `dev-pilot)` du `case "$SKILL"`
(`dispatch-lib.sh:7724`), initialiser explicitement :

```bash
PILOT_SHIPPING_TAIL="present"   # /mika porte commit→PR dans son périmètre
```

Dans `_detect_plan_on_branch` (`dispatch-lib.sh:7346`), **à la ligne même qui
pose l'override** et nulle part ailleurs :

```bash
ENTRY_COMMAND="/ce-work $PLAN_PATH"
PILOT_SHIPPING_TAIL="absent"    # mika#2492 : /ce-work est « implementation and
                                # local verification only, without the shipping
                                # tail » — ce pilote n'ouvrira aucune PR, et
                                # c'est son périmètre, pas une troncature.
```

Deux sites d'écriture, un par valeur, chacun adjacent à la décision qu'il
décrit. Motif maison : un fait estampillé par son producteur, jamais reconstruit
(mika#2026 `origin:loop`, mika#2242 `closing_pr_closed_unmerged`, mika#2368
`qa_review_pr_target`). La branche `else` de `_detect_plan_on_branch`
(`:7348-7350`, callout présent, fichier absent) **n'écrit rien** : elle retombe
sur `/mika`, donc sur la valeur `present` déjà posée.

**L'unicité du chemin d'entrée est établie par lecture (finding F5).** Le site
qui pose `present` doit dominer *tout* chemin atteignant la sélection de classe
avec `SKILL = dev-pilot`, sans quoi une entrée non estampillée lirait une
variable vide. Quatre lectures le donnent, et elles composent :

1. **`SKILL` a un seul site d'écriture** — `:1716`, dans `_parse_input_json`,
   depuis le JSON d'entrée. Aucune autre affectation dans le fichier.
2. **Le `case` est exhaustif et fatal** — `:7723-7750` n'a que trois bras,
   `dev-pilot)`, `dev-groom)` et `*) echo "Unknown skill" >&2; exit 1`. Il n'y a
   donc pas de valeur de `SKILL` qui traverse le `case` sans l'un des deux bras
   nommés, et `dev-pilot` n'en a qu'un.
3. **La suite est linéaire** — de `:7751` (`_setup_gh_auth`) à `:7893` (la
   sélection de classe), `dispatch_claude_pilot` est une séquence sans boucle,
   sans `goto` et sans second point d'entrée ; les sorties anticipées
   (`_handle_dry_run`, les `_deliver_callback` + `exit` de `_set_up_worktree` aux
   `:2535`/`:2586`, le `return` du push guard `:7771`) **quittent** la fonction,
   elles ne la ré-entrent pas.
4. **`dispatch_claude_pilot` n'a que deux appelants**, tous deux sans argument :
   `skills/bundled/dev-pilot/handlers/run.sh:7` et
   `skills/bundled/dev-groom/handlers/run.sh:7`.

Conséquence : tout dispatch qui atteint `:7893` avec `SKILL = dev-pilot` a
traversé le bras `dev-pilot)` et porte donc `present` ou `absent`, jamais une
variable vide. **T5 pin cette propriété du côté où elle peut régresser** (un
second site d'écriture) ; la lecture ci-dessus couvre l'autre côté (un site
d'entrée qui n'écrirait rien), qu'aucun scan ne peut atteindre.

`dev-groom` ne pose pas la variable (`_detect_plan_on_branch` retourne à `:7317`
sur la garde `SKILL = dev-pilot`), et la classe nouvelle est gardée par
`SKILL = dev-pilot` : une variable vide sort de la classe (R4).

### U2 — Le prédicat de la classe nouvelle

Une fonction, un seul lecteur décisionnel de l'estampille :

```bash
# _pilot_had_no_shipping_tail — vrai quand ce dispatch a lancé un pilote dont le
# périmètre n'incluait pas l'ouverture de PR ET dont la session a conclu.
# Fail-safe (mika#2492 R4) : toute indétermination rend faux, ce qui renvoie au
# comportement d'avant ce ticket (classe `commit-pushed-no-pr`).
_pilot_had_no_shipping_tail() {
    [ "${SKILL:-}" = "dev-pilot" ]                || return 1
    [ "${PILOT_SHIPPING_TAIL:-}" = "absent" ]     || return 1
    [ "${STATUS:-}" = "success" ]                 || return 1   # cf. D6
    return 0
}
```

Le troisième terme est **figé** (D6, rev 2) : une session arrêtée par un
guardrail ou une limite SDK porte `STATUS = "terminated"` et non `"success"`,
ce qui est établi par `dispatch-lib.sh:3074-3087`, `:3169-3170` et `_halt_family`
`:3520`. Le précédent de ce terme exact existe dans le fichier : le bloc mika#940
Unit 1 (`dispatch-lib.sh:4428`, commenté `:4421-4422`) l'emploie déjà pour « ne
pas double-classer une session déjà en échec ». Ce n'est donc pas un prédicat
inventé pour ce ticket — c'est le prédicat de complétude que ce fichier emploie
déjà, lu au même endroit.

### U3 — La classe `no-shipping-tail`, et ses quatre différences

**(a) Bloc mika#940 Unit 1 (`dispatch-lib.sh:4428`)** — ajouter
`&& ! _pilot_had_no_shipping_tail` à la conjonction. Son texte affirme
« Pipeline truncated before git push + gh pr create », ce qui est **faux** pour
cette classe : le pipeline n'avait pas cette étape. Sans ce site, le RESULT porte
un `PIPELINE FAILURE:` et la ligne `Outcome:` retombe mécaniquement sur
`PIPELINE_INCOMPLETE` (`dispatch-lib.sh:4438`).

**(b) Sélection de classe (`dispatch-lib.sh:7890-7896`)** — un bras **avant**
`commit-pushed-no-pr` (`:7893-7895`), la garde `RESCUED_DIRTY_WORKTREE` (`:7891`)
restant première :

```bash
elif [ -z "$PR_URL" ] && [ -n "$PRE_RUN_HEAD" ] && [ -n "$POST_RUN_HEAD" ] \
     && [ "$PRE_RUN_HEAD" != "$POST_RUN_HEAD" ] && _pilot_had_no_shipping_tail; then
    RECOVERY_CLASS="no-shipping-tail"
```

**(c) Corps et marqueurs** — `_rescue_class_fact` dit la vérité de cette classe :
le périmètre du pilote (`/ce-work <plan>`) n'incluait pas l'ouverture de PR, le
travail est celui du plan, et la PR est draft parce que sa revue n'a pas encore
eu lieu — **jamais** parce que son contenu serait douteux. Le commit marqueur
`wip(mika#1383)` est déjà exclu par l'égalité stricte de `dispatch-lib.sh:7930`
(`[ "$RECOVERY_CLASS" = "commit-pushed-no-pr" ]`) : **vérifier, ne rien ajouter.**
La ligne `RECOVERY_PENDING: true` (`dispatch-lib.sh:8004`) est posée pour les
seules classes `dirty-worktree` et `commit-pushed-no-pr`.

**(d) Ligne `Outcome:` — les deux sites, la fenêtre entre eux, et son
arbitrage.** Rev 1 énonçait la valeur transitoire `UNKNOWN` sans citer les sites
ni établir que personne ne lit entre les deux, et invoquait `_set_pr_status_line`
de mémoire. Les trois points sont repris ici par lecture (finding F2).

*Site de pose* — bloc **mika#940 Unit 3**, `dispatch-lib.sh:4434-4459`, à
l'intérieur de `_post_flight_recovery` (`:4088`), elle-même appelée depuis
`_run_claude_pilot` (`:3177`), donc **pendant** `_run_claude_pilot "$ENTRY_COMMAND"`
à `:7756`. C'est une cascade à quatre bras, et la classe nouvelle en traverse
trois faux : `PIPELINE FAILURE:` absent (c'est (a) qui le retire), `PR_URL` vide
(la PR n'existe pas encore), `SKILL = dev-groom` faux. Elle tombe donc sur le
`else` `:4455-4458`, `Outcome: UNKNOWN — inspect worktree manually.`

*Site de réécriture* — bloc **mika#1282/#1396 Unit 2**, `:7879-8046` (« Path B »),
après `_push_branch` (`:7867`) et `_signal_rescue_into_open_pr` (`:7877`) ; le
`gh pr create --draft` est à `:7975` et la valeur finale est posée dans le bras
`[ -n "$RESCUED_PR_URL" ]` (`:7983`).

*L'anti-fenêtre, établie* — les seuls lecteurs de `^Outcome: ` dans le fichier
sont `_measure_cycle_output` (`:3338`) et `_gate_non_empty_cycle` (`:3427`,
`:3441`), et ces deux fonctions n'ont que **deux** points d'appel :
`_deliver_callback` (`:7245`), qui s'exécute à `:8047` donc **après** Path B, et
le trap EXIT (`:1673`). Les réécritures `sed 's/Outcome: .*/…/'` de `:7799` et
`:7841` sont dans la branche dev-groom et sortent de la classe par
`SKILL = dev-pilot`. **Sur le chemin nominal, personne ne lit entre la pose et la
réécriture.**

*La fenêtre résiduelle est réelle, et elle est nommée plutôt que niée* — le trap
EXIT est un lecteur intermédiaire pour la seule population « dispatch-lib meurt
entre `:4459` et `:8046` ». Rev 1 y aurait laissé `UNKNOWN — inspect worktree
manually`, **moins actionnable qu'aujourd'hui** (`PIPELINE_INCOMPLETE — manual
recovery needed`) : une régression silencieuse sur une population de crash. Le
remède est donc de rendre la valeur de la fenêtre **vraie à son instant** plutôt
que d'essayer de supprimer la fenêtre. Un bras dédié est ajouté au bloc Unit 3,
**avant** le `else`, gardé par `_pilot_had_no_shipping_tail` :

```
Outcome: PIPELINE_INCOMPLETE — no_shipping_tail: dispatch-lib did not reach PR creation.
```

C'est exact : si le process meurt là, la PR n'a effectivement pas été ouverte, et
le travail du pilote attend dans le worktree. L'actionnabilité est identique à
celle d'aujourd'hui sur cette population, et le motif nommé vaut mieux que le
`manual recovery needed` générique qu'elle reçoit actuellement.

*Le mécanisme de réécriture est une fonction NOUVELLE* — `_set_outcome_line`,
calquée sur `_set_pr_status_line` (`:1567-1570`) **sans la réutiliser** : celle-ci
ne connaît que `PR:`/`NO_PR:` (`sed '/^PR: /d; /^NO_PR: /d'`) et n'a jamais
touché `Outcome:`. La sœur fait `sed '/^Outcome: /d'` puis append, ce qui rend le
contrat « exactement une ligne `Outcome:` » vrai **par construction** et non par
coïncidence d'ordre — la propriété même que le commentaire de `:1561-1566`
revendique pour son aînée. Elle n'est appelée que dans le bras `no-shipping-tail`
de Path B ; les autres classes ne la traversent pas (R3).

*Et le test existe quand même* — l'anti-fenêtre est établie **aujourd'hui**, par
une lecture qu'un futur appel de `_gate_non_empty_cycle` entre les deux sites
invaliderait sans rien casser de visible. T1 assert donc la valeur finale **et**
l'unicité (`grep -c '^Outcome: ' = 1`), et T8 (ci-dessous) pin la fenêtre
elle-même.

`_stamp_pr_origin … loop` (`:7987`), le label `wip-rescue` (`:7989`, D3), l'appel
à `_measure_pipeline_verified` (`:7964`, D5) et la ligne canonique `PR:`
(`:8003`) sont **inchangés**.

### U4 — Les labels écrits par le chemin modifié sont déclarés

Vérifier que `wip-rescue` figure dans `.github/labels.yml`. Aucun label nouveau
n'est introduit par ce plan ; cette unité est une **vérification**, pas une
écriture — la classe « un label d'enforcement non déclaré échoue en silence »
compte cinq occurrences dans ce dépôt (`delete-other-labels: true` le supprime du
dépôt *et de chaque PR qui le porte*, sans événement ni journal).

### U5 — Tests

Dans `skills/bundled/_shared/test-dispatch-lib.sh`, à côté des tests 15 / 15b qui
couvrent déjà mika#1383 et mika#1396 :

- **T1 (comportemental, positif).** Périmètre absent + session conclue → classe
  `no-shipping-tail`, corps sans `RECOVERY_PENDING: true`, sans commit marqueur,
  `Outcome: PR_OPENED`, une seule ligne `Outcome:`, une seule ligne `PR:`.
- **T2 (contrôle négatif — le test porteur).** Périmètre absent + session
  **tronquée** → classe `commit-pushed-no-pr`, `RECOVERY_PENDING: true` présent,
  commit marqueur présent. Sans lui, T1 ne distingue pas « le croisement
  fonctionne » de « la classe nouvelle absorbe tout ».
- **T3 (non-régression, R3).** Périmètre présent (pas d'override `/ce-work`),
  dans ses deux états → comportement byte-identique à aujourd'hui.
- **T4 (fail-safe, R4).** `PILOT_SHIPPING_TAIL` vide → classe
  `commit-pushed-no-pr`.
- **T5 (structurel, D-detector).** `PILOT_SHIPPING_TAIL="absent"` n'est écrit
  qu'à **un** site du fichier — le site d'override de `_detect_plan_on_branch`.
- **T6 (structurel, D-detector).** Le bloc conditionné par
  `RECOVERY_CLASS = "no-shipping-tail"` n'émet jamais `RECOVERY_PENDING: true`.
- **T7 (prise du guard U3a) — deux moitiés, parce qu'un scan de présence
  surestime sa force (finding F4).** Rev 1 se contentait de vérifier que le
  jeton `_pilot_had_no_shipping_tail` *apparaît* dans le bloc mika#940 Unit 1 —
  ce qui reste vrai si quelqu'un écrit la conjonction **sans la négation**, cas
  où la garde s'inverse et où la classe nouvelle reçoit un `PIPELINE FAILURE:`
  sur chaque session qu'elle est censée épargner. D'où :
  - **T7a (structurel).** Le jeton cherché est `! _pilot_had_no_shipping_tail`
    **complet, négation incluse**, et il est cherché dans la **ligne de la
    conjonction** (`dispatch-lib.sh:4428`), pas dans le fichier entier — un
    scan à l'échelle du fichier serait satisfait par l'appel que (b) fait à la
    sélection de classe, où le jeton apparaît légitimement **sans** `!`.
  - **T7b (comportemental).** Périmètre absent + session conclue → le RESULT
    **ne contient pas** `PIPELINE FAILURE:`. C'est la moitié qu'aucun scan ne
    donne : elle atteste que la garde *a pris*, là où T7a atteste seulement
    qu'elle est *écrite*.
- **T8 (fenêtre `Outcome:`, finding F2).** Sur la classe nouvelle, la valeur
  posée par `_post_flight_recovery` **avant** Path B est
  `PIPELINE_INCOMPLETE — no_shipping_tail: …` (jamais `UNKNOWN`), et la valeur
  après Path B est `PR_OPENED — <url>`, en une seule ligne `Outcome:` dans les
  deux états. Ce test pin la fenêtre que la lecture d'U3(d) établit comme sans
  lecteur : si un lecteur y apparaît un jour, c'est cette assertion qui dira ce
  qu'il aura lu.

---

## Verification Contract

| # | Quoi | Comment | Attendu |
|---|---|---|---|
| V1 | U0 est faite et écrite | lecture des deux journaux | le couple `(STATUS, SUBTYPE)` des deux populations, reporté dans la PR |
| V2 | Harness vert | `bash skills/bundled/_shared/test-dispatch-lib.sh` | 0 échec, T1–T8 présents |
| V3a | Contrôle négatif **vu rouge** — l'axe A | retirer le terme `PILOT_SHIPPING_TAIL` de `_pilot_had_no_shipping_tail`, relancer | T2 et T4 rougissent ; restaurer |
| V3b | Contrôle négatif **vu rouge** — la négation du guard (F4) | retirer le `!` de `! _pilot_had_no_shipping_tail` en `dispatch-lib.sh:4428`, relancer | **T7a et T7b rougissent** ; restaurer |
| V3c | Contrôle négatif **vu rouge** — l'axe B | forcer `STATUS = "success"` dans le scénario tronqué de T2, relancer | T2 rougit ; restaurer |
| V4 | Structure des bundles | `make verify-bundled-skills` | vert |
| V5 | Shell propre | `shellcheck` sur les deux fichiers touchés | pas de régression |
| V6 | R3 tenu | `git diff` relu bras par bras | les trois croisements inchangés ne traversent aucune ligne modifiée |
| V7 | Rust intact | `cargo test -p mika-agent` | vert (aucun fichier Rust touché — contrôle de non-effet) |

V3 est la vérification porteuse : un scan et deux tests comportementaux qui n'ont
jamais été vus rouges n'attestent pas qu'ils mordent. **V3b est celle que rev 1
n'avait pas** (finding F4) : elle est la seule à distinguer « le guard est écrit »
de « le guard est écrit dans le bon sens », et sa cible est une inversion d'un
seul caractère — exactement la classe de régression qu'un scan de présence laisse
passer en restant vert.

---

## Definition of Done

**Item 0 — geste opérateur, ordonnancé AVANT toute implémentation.**

- [ ] **Le corps de #2492 est rectifié par l'opérateur.** Ce plan réfute le
      diagnostic du ticket sur trois points mesurés (M1 entry command `/ce-work`,
      M2 absence de queue d'expédition par construction, M3 travail non
      orphelin) et **reformule AC1 et AC3**. Le corps du ticket, lui, porte
      toujours sa lettre d'origine. La convention mika#2169/#2158 impose que
      cette divergence soit portée **sur le ticket** : un encadré daté nommant
      M1/M2/M3 et la reformulation des deux AC, plus un commentaire d'avis
      d'édition. C'est un **geste opérateur** — ni l'implémenteur ni l'architecte
      ne ratifient unilatéralement une divergence entre un plan et la
      spécification qu'il corrige, et un plan qui se contente de la consigner
      dans une description de PR laisse le ticket dire le faux à tout lecteur
      ultérieur. **Tant que cet item n'est pas fait, le reste du DoD n'est pas
      engagé** : implémenter d'abord reviendrait à livrer contre une
      spécification que personne n'a accepté de corriger.

**Le reste, dans l'ordre.**

- [ ] U0 est faite, son résultat est écrit dans la description de PR, et le
      contrôle des deux témoins confirme D6 (ou l'une des trois issues de la
      halte U0 est déclenchée et remontée).
- [ ] Le périmètre d'expédition est estampillé à deux sites, un par valeur.
- [ ] Une session sans queue d'expédition qui conclut produit une PR draft sans
      `RECOVERY_PENDING: true`, sans commit marqueur, avec `Outcome: PR_OPENED`.
- [ ] Les trois autres croisements sont inchangés, et un test le pin.
- [ ] Tout terme illisible retombe sur le comportement actuel, et un test le pin.
- [ ] La ligne `Outcome:` vaut `PR_OPENED` en fin de chemin nominal et
      `PIPELINE_INCOMPLETE — no_shipping_tail: …` dans la fenêtre de crash,
      en un seul exemplaire dans les deux états (T8).
- [ ] V1–V7 passent, V3a/V3b/V3c inclus (les trois contrôles négatifs vus rouges).
- [ ] La description de PR nomme la rectification (M1/M2/M3), ce que le plan
      **ne** livre **pas** de la lettre du ticket, et **renvoie à l'encadré du
      ticket** plutôt que de s'y substituer (item 0).

## Acceptance criteria

Le ticket porte trois critères. Deux sont **reformulés** parce que leur lettre
suppose un chemin que la population mesurée n'emprunte pas (M1/M2) ; la
reformulation est nommée ligne à ligne plutôt que silencieuse.

**La reformulation n'est pas acquise par le fait d'être écrite ici** (finding
F1). Elle doit être portée sur le corps de #2492 par l'opérateur, sous la
convention mika#2169/#2158, et c'est l'**item 0 du DoD** — ordonnancé avant toute
implémentation. Aucun AC ci-dessous n'est tenu par un plan qui reformule seul ce
qu'un ticket continue d'affirmer.

- [ ] **AC1 — reformulé.** *Lettre du ticket :* « Le pipeline `/mika` ouvre une
      PR draft juste après `work` (avant review), vérifié sur un implement. »
      *Inapplicable :* les quatre implements de la preuve tournent sous
      `/ce-work` (M1), où il n'existe ni phase `review` ni queue d'expédition à
      devancer (M2). *Tenu à la place :* un implement de la boucle autonome dont
      la session conclut produit une **PR draft reviewable sans geste humain** —
      ni `RECOVERY_PENDING`, ni commit marqueur, ni `PIPELINE_INCOMPLETE` — et
      la revue QA autonome n'est plus sautée sur elle.
- [ ] **AC2 — tenu, et il l'était déjà.** « Un implement tronqué après `work`
      laisse une PR draft ouverte (pas de commits orphelins sans PR), testé. »
      Assuré par le rescue mika#1396 **avant** ce ticket (M3) ; ce plan le
      **préserve** et le pin explicitement (T2, T3, V6). Le livrable est ici la
      non-régression, pas la fonction.
- [ ] **AC3 — reformulé.** *Lettre :* « Le passage draft→ready se fait à la fin
      nominale du pipeline. » *Inapplicable :* sous `/ce-work` il n'existe pas de
      « fin nominale » incluant review/compound. *Tenu à la place :* le passage
      draft→ready reste au mécanisme qui sait le faire en sûreté
      (`wip_rescue.rs` : rebase → clippy → périmètre → un-draft, avec l'exigence
      `rescue-pipeline-verified: yes` sur DECISION-CORE), et ce plan lui livre
      une PR dont la revue ne sera plus sautée une fois un-draftée. **Aucun
      un-draft nouveau n'est ajouté par ce ticket** (D4).

---

## Fire-Disposition

Ce plan livre des détecteurs : **T5** (un seul site écrit l'estampille), **T6**
(la classe nouvelle n'émet jamais `RECOVERY_PENDING: true`) et **T7a** (la
conjonction du bloc Unit 1 porte le jeton `! _pilot_had_no_shipping_tail`,
négation incluse) sont des scans de source dont le chemin de succès est « aucune
violation trouvée ». **T7b et T8 ne sont pas des détecteurs** mais des tests
comportementaux, et c'est délibéré : F4 a établi qu'un scan de présence seul
surestime sa force sur T7, donc la moitié « la garde a pris » est portée par un
comportement, jamais par un grep.

**Option retenue : (a) exception nommée en allowlist — allowlist livrée VIDE.**

- Chaque scan porte une constante d'allowlist explicite
  (`SHIPPING_TAIL_WRITERS_ALLOWED`, `NO_TAIL_RECOVERY_PENDING_ALLOWED`,
  `UNIT1_GUARD_EXEMPT`), déclarée, grep-visible, et **livrée vide**.
- **Population préexistante : zéro par construction** — les trois scans portent
  sur des jetons (`PILOT_SHIPPING_TAIL`, `RECOVERY_CLASS = "no-shipping-tail"`,
  `_pilot_had_no_shipping_tail`) que ce ticket crée. À mesurer avant de livrer
  armé, et à reporter dans la PR :
  ```bash
  grep -c "PILOT_SHIPPING_TAIL"        skills/bundled/_shared/dispatch-lib.sh   # attendu 0 avant U1
  grep -c "no-shipping-tail"           skills/bundled/_shared/dispatch-lib.sh   # attendu 0 avant U3
  grep -c "_pilot_had_no_shipping_tail" skills/bundled/_shared/dispatch-lib.sh  # attendu 0 avant U2
  grep -c "_set_outcome_line"          skills/bundled/_shared/dispatch-lib.sh   # attendu 0 avant U3(d)
  grep -c "no_shipping_tail"           skills/bundled/_shared/dispatch-lib.sh   # attendu 0 avant U3(d)
  ```
  Les deux derniers sont ajoutés en rev 2 : U3(d) introduit une fonction et un
  motif d'`Outcome:` qui n'existaient pas au plan initial, et un homonyme sur
  l'un des deux ferait mentir la même halte que sur les trois autres.
  **Halte :** un compte non nul signifie qu'un jeton homonyme existe déjà —
  établir lequel **avant** d'écrire le scan, jamais l'allowlister.
- **Assertion auto-nettoyante :** un test refuse que l'une des trois allowlists
  cesse d'être vide — quand un scan tire, la résolution est de **retirer le
  second site**, pas d'y ajouter une entrée. Motif : `canonical-tokens`
  (mika#2201, « on déclare, on n'allowliste pas ») et
  `mika1883_run_usage_accumulates_only_via_the_one_helper`.
- **Branche (c) — halte-et-remontée — est armée sur U0 seul** : si la mesure ne
  rend aucun prédicat de complétude lisible (halte U0), l'implémentation
  s'arrête et remonte à l'opérateur plutôt que de livrer la classe nouvelle sur
  le seul axe A.

Aucun détecteur n'est livré désarmé : aucun ne porte de population préexistante,
donc l'option (b) n'aurait rien à différer.

---

## Sondes post-déploiement, et leurs haltes

Le déploiement passe par `make deploy` (`skills/bundled/` n'atteint un agent que
par ce rebuild — `~/.mika/skills/` est une projection du binaire, mika#2340).

**S1 — contrôle positif, sur le premier implement autonome qui suit.** Le
callback porte `Outcome: PR_OPENED`, la PR ouverte ne porte pas
`RECOVERY_PENDING: true`, et son commit de tête est le vrai commit
d'implémentation, pas un marqueur vide.

```bash
grep -c "RECOVERY_PENDING: true" /var/log/claude-pilot/<id>.stderr   # 0 attendu
git log -1 --format='%s' <branche>                                    # pas de `wip(mika#1383)`
```

**Halte S1a — la PR porte encore les marqueurs.** Établir **le déploiement**
avant de toucher au code : `cat ~/.mika/skills/.manifest-writer` doit porter le
sha qu'on vient de construire. Une garde qu'on n'a pas déployée se lit exactement
comme un correctif qui ne prend pas (mika#2340).

**Halte S1b — la PR sort propre mais la classe est `commit-pushed-no-pr`.**
L'estampille n'atteint pas le prédicat : lire d'abord si l'override `/ce-work` a
bien eu lieu (`Plan-on-branch detected:` dans le stderr du dispatch) — un ticket
sans callout `Plan:` tourne légitimement sous `/mika` et n'est pas dans la
population. **Ne pas élargir le prédicat avant d'avoir établi lequel des deux.**

**S2 — la revue QA n'est plus sautée.** Sur la PR de S1, une revue de
`mika-platform-qa` est postée sans geste humain. **Halte :** si la revue est
toujours sautée, lire `unpushed_recovery_pending` sur la tâche parente
(`mika tasks get`) — s'il est absent et que la revue saute quand même, c'est
**Guard 2** qui fire, donc un commit marqueur subsiste : le site à lire est
`dispatch-lib.sh:7930`, pas le prédicat.

**S3 — contrôle négatif, la population épave existe toujours.** Sur un dispatch
tronqué (`error_max_turns` ou halte), `RECOVERY_PENDING: true` et le commit
marqueur **doivent** réapparaître. **Halte :** leur disparition totale signifie
que la classe nouvelle absorbe les deux populations — désarmer en restaurant le
bras `commit-pushed-no-pr` en tête, puis réparer l'axe B. C'est la sonde dont
l'absence rendrait S1 ininterprétable : un croisement qui ne dit jamais « épave »
n'est pas un croisement.

**S4 — mesurer ce que ce plan n'a pas mesuré (7 jours).** Quelle part des
dispatches d'implémentation autonomes tombe dans la classe `no-shipping-tail` ?
M1/M2 prédisent **la quasi-totalité** (tout ticket groomé tourne sous
`/ce-work`), mais cela n'a pas été compté. Une part faible réfuterait la lecture
et **rouvrirait le diagnostic** — c'est un résultat, pas une panne.

**Limite commune aux quatre sondes :** aucun compteur ni événement de journal
n'est livré (voir ci-dessous). Leur silence ne prouve rien tant qu'un dispatch
autonome n'a pas tourné.

---

## Suivi (hors périmètre, nommé)

1. **Le vocabulaire du label `wip-rescue`** (D3) — il signifie aujourd'hui deux
   choses. *Préalable :* S4, qui donne la taille de chaque population.
2. **`CLAUDE_PILOT_REQUIRE_PR=1` sur un pilote sans queue d'expédition**
   (`dispatch-lib.sh:7732`, exporté avant l'override) — incohérence réelle,
   consommateur hors dépôt.
3. **Le budget de tours** — l'« alternative complémentaire » du ticket, et le
   `error_max_turns` de #2484. Défaut réel, orthogonal à celui-ci.
4. **Le faux-étiquetage rescue-class côté `qa-review` Step 1.5**, déjà nommé
   comme suivi à ouvrir par mika#2334.
5. **Un événement d'attribution pour la classe** — voir ci-dessous.

---

## Ce que ce travail n'achète PAS

- **Il n'ouvre pas la PR plus tôt.** La PR reste ouverte à la fin du dispatch,
  par dispatch-lib. « Plus tôt » n'est atteignable que depuis la session du
  pilote, et la session de la population mesurée tourne sous un prompt hors
  dépôt. Ce qui change est le **classement** de cette PR, pas son instant.
- **Il ne protège pas d'une mort par signal.** Un pilote tué par le reaper
  (mika#2249/#2277) ou par une supersession (mika#2335) emporte dispatch-lib avec
  lui — même pgid — donc aucun rescue ne tourne et le travail reste dans le
  worktree. Cette population existe, n'est pas celle du ticket (les quatre cas
  mesurés ont tous vu leur rescue tourner), et **seule une PR ouverte pendant la
  session la couvrirait**. Nommée, non couverte.
- **Il n'ajoute aucun un-draft** (D4), aucune garde, aucune valeur de réglage
  (R6).
- **Il n'ajoute ni compteur ni ligne de journal ni `audit_events`.** Le défaut
  n'est pas un silence à instrumenter mais un classement à corriger, et sa
  vérification tient dans l'état de la PR produite. Le coût est réel et nommé :
  répondre à « quelle part des dispatches passe par cette classe ? » (S4) demande
  de lire les journaux pilote un par un. Un événement d'attribution est le
  **suivi 5**, à ouvrir si S4 se révèle coûteuse à produire.

---

## Références

- `skills/bundled/_shared/dispatch-lib.sh` — **estampille et entrée :**
  `_parse_input_json` pose `SKILL` (1716, site unique) ; `case "$SKILL"` exhaustif
  avec `*) exit 1` (7723-7750) ; bras `dev-pilot)` (7724-7733) dont l'export
  `CLAUDE_PILOT_REQUIRE_PR` (7732) ; `_detect_plan_on_branch` (7307), garde
  `SKILL = dev-pilot` (7317), **site de l'override `/ce-work`** (7346), branche
  `else` sans écriture (7348-7350) ; appelants externes
  `dev-pilot/handlers/run.sh:7`, `dev-groom/handlers/run.sh:7`.
  **Prédicat de complétude (D6) :** `STATUS` posé du stdout JSON (3071), `SUBTYPE`
  (3087), commentaire sur les deux populations de `terminated` (3074-3087),
  embranchement (3169-3170, 3184), `_halt_family` et `error_max_turns`
  (3501-3529, spéc. 3520), garde `STATUS = success` déjà employée (4421-4422).
  **Ligne `Outcome:` — pose, lecture, réécriture (U3d) :** `_post_flight_recovery`
  (4088), appelée depuis `_run_claude_pilot` (3177) ; bloc mika#940 Unit 1 (4428) ;
  bloc Unit 3, cascade de pose (4434-4459) dont le `else` `UNKNOWN` (4455-4458) ;
  **seuls lecteurs** `_measure_cycle_output` (3338) et `_gate_non_empty_cycle`
  (3427, 3441), atteints depuis le trap EXIT (1673) et `_deliver_callback` (7245) ;
  réécritures dev-groom hors classe (7799, 7841) ; `_set_pr_status_line` et son
  contrat d'unicité (1561-1570).
  **Path B (bloc Unit 2) :** 7879-8046 ; sélection de classe (7890-7896), bras
  `commit-pushed-no-pr` (7893-7895) ; commit marqueur (7930-7947) ;
  `_measure_pipeline_verified` (7964) ; `gh pr create --draft` (7975) ;
  `_stamp_pr_origin` (7987) ; label `wip-rescue` (7989) ; ligne `PR:` (8003) ;
  `RECOVERY_PENDING: true` (8004) ; `_deliver_callback` (8047).
- `skills/bundled/self-dev-webhook-qa/system_prompt.md` — Guard 1 (248),
  Guard 2 (251-254), interdiction `gh pr ready` / guard mika#1682 (325).
- `skills/bundled/self-dev-callback/system_prompt.md` — consommateur de
  `RECOVERY_PENDING: true` (104-108).
- `crates/mika-agent/src/wip_rescue.rs` — filtre `wip-rescue`, chaîne rebase →
  clippy → périmètre → un-draft, exigence mika#2286.
- Journaux de la preuve : `/var/log/claude-pilot/{08f8fe27,153c2044,d8aa26ee,73573501}-….log`.
- Lignée : mika#1074 (override d'entry command), mika#1271 (partage
  contenu/workflow), mika#1282, mika#1383, mika#1396, mika#1613, mika#1679,
  mika#1852, mika#1941, mika#2286, mika#2334, mika#2354.
- Doctrines appliquées :
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
  (mika#2120) ; « un fait estampillé par son producteur » (mika#2026, #2242,
  #2368) ; « un filet, pas un chemin » (mika#2334) ; « on déclare, on
  n'allowliste pas » (mika#2201) ; mika#2340 (établir le déploiement avant de
  conclure).

## Revision history

- **2026-09-23 — v1.** Plan initial. Rectifie le diagnostic du ticket sur trois
  points mesurés (M1 entry command `/ce-work`, M2 absence de queue d'expédition
  par construction, M3 travail non orphelin), déplace le remède de
  `.claude/commands/mika.md` vers `dispatch-lib.sh`, et reformule AC1 et AC3 dont
  la lettre suppose un chemin que la population mesurée n'emprunte pas.

- **2026-09-23 — rev 2.** Adresse les cinq findings de la première passe
  architecte. **Deux d'entre eux corrigent le plan par la mesure, pas seulement
  par l'ajout.**

  - **F1 (BLOCKING) — rectification du corps du ticket.** Ajout d'un **item 0**
    au DoD, ordonnancé **avant toute implémentation** : l'opérateur porte sur le
    corps de #2492 un encadré daté nommant M1/M2/M3 et la reformulation d'AC1/AC3,
    plus un commentaire d'avis d'édition (convention mika#2169/#2158). L'intro de
    `## Acceptance criteria` dit désormais qu'aucun AC n'est tenu par un plan qui
    reformule seul ce qu'un ticket continue d'affirmer, et le dernier item du DoD
    renvoie à l'encadré au lieu de s'y substituer.

  - **F2 (BLOCKING) — U3(d), la réécriture d'`Outcome:`.** Les deux sites sont
    nommés par lecture (pose : bloc mika#940 Unit 3, `:4434-4459`, dans
    `_post_flight_recovery` `:4088` appelée depuis `_run_claude_pilot` `:3177` ;
    réécriture : bloc Unit 2 « Path B », `:7879-8046`, valeur finale `:7983`).
    L'anti-fenêtre est **établie** : les seuls lecteurs de `^Outcome: ` sont
    `_measure_cycle_output` (`:3338`) et `_gate_non_empty_cycle` (`:3427`, `:3441`),
    atteints uniquement depuis `_deliver_callback` (`:7245`, donc après Path B)
    et le trap EXIT (`:1673`). La **fenêtre résiduelle de crash** est nommée
    plutôt que niée, et son arbitrage corrige une régression silencieuse de rev 1 :
    laisser `UNKNOWN — inspect worktree manually` y aurait été **moins
    actionnable qu'aujourd'hui**, d'où un bras dédié posant
    `PIPELINE_INCOMPLETE — no_shipping_tail: …`, exact à son instant. Rev 1
    invoquait `_set_pr_status_line` de mémoire : la lecture montre qu'elle ne
    connaît que `PR:`/`NO_PR:` (`:1567-1570`), donc `_set_outcome_line` est
    déclarée **nouvelle** et calquée, pas réutilisée. Le test d'unicité demandé
    en repli est livré **en plus** de l'anti-fenêtre (T8), parce qu'un futur
    lecteur intermédiaire invaliderait la lecture sans rien casser de visible.

  - **F3 (sharpening) — le témoin « session conclue proprement ».** Nommé :
    `d8aa26ee-…log` (#2425), marqueur `[done] Success | 142 turns | $48.50 | 2950s`,
    contre `153c2044-…log` (#2484), `[guardrail] error_max_turns`. Le témoin
    **existe dans la population mesurée elle-même** et porte l'axe A — #2425 est
    déjà un cas de la classe `no-shipping-tail`. La halte U0 passe de deux à
    trois issues (témoin purgé / marqueurs identiques / mapping contredit).
    **Deux corrections de fond en découlent.** (i) Les deux commandes de U0 en
    rev 1 étaient **inexécutables** : `STATUS`/`SUBTYPE` viennent du stdout du
    pilote (`:3071`, `:3087`), capturé dans un `mktemp` supprimé `:3068`, et ne
    sont ni dans `<id>.log` ni dans `<id>.stderr` — vérifié, zéro ligne rendue
    sur les deux fichiers. (ii) D6 affirmait que le prédicat n'était pas
    établissable par lecture, claude-pilot étant hors dépôt : réfuté, le mapping
    est écrit et consommé **ici** (`:3074-3087`, `:3169-3170`, `_halt_family`
    `:3520`), donc le prédicat `STATUS = "success"` est **figé** et U0 devient un
    contrôle sur marqueur de prose, non une mesure dont dépendrait le code.

  - **F4 (sharpening) — T7 surestimait sa force.** Scindé : **T7a** cherche le
    jeton `! _pilot_had_no_shipping_tail` **négation incluse**, et dans la ligne
    de la conjonction (`:4428`) plutôt que dans le fichier entier — un scan à
    l'échelle du fichier serait satisfait par l'appel légitime, sans `!`, de la
    sélection de classe. **T7b** est comportemental : la classe nouvelle produit
    un RESULT **sans** `PIPELINE FAILURE:`. V3 devient V3a/V3b/V3c, dont **V3b**
    est le contrôle négatif manquant : retirer le `!` doit faire rougir T7a *et*
    T7b, une inversion d'un seul caractère étant exactement ce qu'un scan de
    présence laisse passer en restant vert.

  - **F5 (sharpening) — exhaustivité des chemins d'entrée.** Établie par lecture
    et écrite dans U1 : `SKILL` a un site d'écriture unique (`:1716`) ; le `case`
    est exhaustif et fatal (`:7723-7750`, `*) exit 1`) ; la suite jusqu'à la
    sélection de classe (`:7893`) est linéaire, ses sorties anticipées quittant
    la fonction sans la ré-entrer ; et `dispatch_claude_pilot` n'a que deux
    appelants (`dev-pilot/handlers/run.sh:7`, `dev-groom/handlers/run.sh:7`).
    La note dit quel côté T5 couvre (un second site d'écriture) et quel côté
    seule la lecture couvre (un site d'entrée qui n'écrirait rien).

  Corrections de `file:line` faites au passage, chacune vérifiée sur l'arbre :
  override `/ce-work` `:7346` (et non `:7340`), sélection de classe `:7890-7896`
  avec le bras `commit-pushed-no-pr` en `:7893-7895` (et non `:7891`). La section
  `## Références` est réécrite par domaine — entrée/estampille, prédicat de
  complétude, ligne `Outcome:`, Path B — F2 ayant relevé qu'elle ne citait ni la
  pose d'`Outcome:` ni Path B.

  **Aucun AC n'est affaibli par cette révision.** AC1/AC2/AC3 sont inchangés dans
  leur substance ; ce qui est ajouté est la condition — item 0 — sans laquelle
  leur reformulation n'engage personne.
