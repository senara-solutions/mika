# Vol 2 de mika#2077 — un `PIPELINE FAILURE` périmé est démoté, jamais décapité

> Ticket : senara-solutions/mika#2077 (parapluie p3 du substrat de dispatch).
> Ligne traitée : « `sed` retire une ligne d'un `PIPELINE FAILURE` multi-ligne » (reprise de #2032).
> Vol 1 : `docs/plans/2026-10-19-005-fix-2077-entrypoint-phase-2b-argv-positionnel-plan.md`, livré par #2827.

## État des cinq lignes du parapluie, à la date de ce plan

| ligne | source | état pour ce vol |
|---|---|---|
| Six suites de `_shared/tests/` ne s'exécutent pas | #2058 | **hors périmètre** — recoupe senara-solutions/mika#2826, mesurée OPEN et non mergée le 2026-10-19. Le corps du parapluie le dit : « Ne pas les refaire ici. » |
| L'entrypoint Phase 2b re-cite l'argv avec `printf '%q'` | #2057 | **cochée** — vol 1, #2827 |
| `_launch_revise_pilot` est hors du chemin de terminaison | #2033 | **renvoyée au vol 3** — voir § Découpage en vols |
| `sed` retire une ligne d'un `PIPELINE FAILURE` multi-ligne | #2032 | **ce vol** |
| `test_sandbox_log_dir_bound.sh` échoue en rouge là où elle devrait sauter | vol #2827 | **hors périmètre** — même recoupement avec #2826 |

Mesure du recoupement : `gh issue view 2826 --repo senara-solutions/mika --json state` → `OPEN`,
`gh pr view 2826 --json mergedAt` → `null`. Les deux lignes restent donc décochées et
**ne peuvent pas être cochées par ce vol** : leur preuve est le merge de #2826, pas un commit d'ici.

## Le défaut, et pourquoi la correction proposée par #2032 ne marche pas

### Le site

`dispatch-lib.sh:12447`, sur le chemin où `_iterate_groom_loop` a convergé sur GROOMED :

```bash
RESULT=$(printf '%s' "$RESULT" | sed '/^PIPELINE FAILURE:/d')
```

`sed` supprime les lignes qui *commencent* par le marqueur. Les messages d'échec de ce
fichier sont des blocs de plusieurs lignes dont une seule porte le marqueur. Le reste
survit, orphelin — un `Halt event:` sans échec au-dessus, un `Halt class:` sans halte, un
`Session:` sans session morte. C'est le défaut nommé par #2032, et c'est exactement la classe
que mika#1772 a fermée ailleurs : du texte de `RESULT` qui induit en erreur.

### Pourquoi la frontière « prochaine ligne vide » est fausse

#2032 propose, en premier remède : *delete from `^PIPELINE FAILURE:` to the next blank-line
boundary*. **Mesuré sur les messages réels du fichier : cette frontière ne tient pas.** Dans
tous les blocs concernés, la première ligne vide suit **immédiatement** le marqueur :

```
PIPELINE FAILURE: the claude-pilot session was terminated before it produced any work. …
                                        ← la première ligne vide est ICI
Session: …
Turns: …
Halt class: …
```

(`_classify_terminated_session`, `dispatch-lib.sh:6256` et `:6267` ; même forme au
policy-deny `:7193` et `:7432`, et au message de #2032.) Un strip jusqu'à la ligne vide
retirerait donc le marqueur **et la ligne vide**, en laissant `Session:`, `Turns:` et
`Halt class:` collés au texte précédent : strictement pire que le `sed` actuel.

Le second remède proposé — *compose `RESULT` so failure markers are individually removable*
— demande d'encadrer de sentinelles les ~15 sites d'écriture de `PIPELINE FAILURE:`. Hors
borne par vol, et il fait porter à tous les producteurs le coût d'un seul consommateur.

### Le remède retenu : démotion plutôt que suppression

Le `sed` existe pour une raison précise, et elle n'est pas « faire disparaître la prose » :
le marqueur `PIPELINE FAILURE:` met le callback dans la **population retryable**, lue par
`milestone_manager::evaluator::dead_cause_of` (`evaluator.rs:305`, `result.contains("PIPELINE
FAILURE:")`). Sur un groom qui a convergé, ce marqueur est faux. Ce qui doit disparaître,
c'est donc **le marqueur**, pas la prose qu'il introduit.

Le remède est de remplacer la décapitation par une **démotion** : la ligne

```
PIPELINE FAILURE: dev-groom: _find_issue_plan returned empty for mika#2077 …
```

devient

```
Superseded (grooming converged, mika#2077): dev-groom: _find_issue_plan returned empty for mika#2077 …
```

Trois propriétés, chacune mesurable :

1. **La sous-chaîne `PIPELINE FAILURE:` disparaît de la ligne.** Nécessaire, et pas
   seulement l'ancrage : `evaluator.rs:305` lit en `contains`, non ancré. Une démotion qui
   garderait le marqueur en milieu de ligne reconduirait le piège que mika#2590 a fermé sur
   `Outcome:` — la forme « ancrée à l'écriture, non ancrée à la lecture ».
2. **La prose garde une tête.** Plus rien n'est orphelin : le lecteur voit qu'un échec a été
   posé **puis dépassé**, ce qui est la vérité de ce chemin, là où la suppression effaçait
   l'événement.
3. **Le texte de démotion est inerte pour tous les lecteurs de `tasks.result`.** `Superseded`
   n'est le préfixe d'aucune ligne lue : mesuré contre `PIPELINE FAILURE:`,
   `STRUCTURAL VIOLATION:`, `HANDLER CRASH`, `STATUS=CANCELLED`, `Outcome:` (dont
   `GROOM_SUCCESS_MARKER = "Outcome: PLAN_GROOMED"`, `task_state/tasks.rs:45`), `PR:`,
   `NO_PR:`, `RECOVERY_PENDING:`, `Halt class:`, `Retry hint:`.

La démotion n'est pas une forme inventée pour l'occasion : `dispatch-lib.sh:7464` porte déjà
« Demoted from PIPELINE FAILURE to advisory note (mika#1303) » sur le même chemin dev-groom,
et pour le même motif — un fait réel dont le marqueur était faux.

### Ce que la démotion ne change pas

`_set_outcome_line` (`:3070`) strippe **toutes** les lignes `^Outcome: ` et en appose une
seule ; elle est appelée immédiatement après le site 12447. L'`Outcome: PIPELINE_INCOMPLETE —
…` que porte le bloc de `_classify_terminated_session` est donc déjà traité, et la démotion
n'a ni à le connaître ni à y toucher. Ce point est épinglé par une assertion plutôt que
supposé : si `_set_outcome_line` cessait de le faire, le détecteur de ce vol le dirait.

## Requirements

- **R1.** Une fonction `_demote_pipeline_failure_markers` dans `dispatch-lib.sh`, filtre
  pur stdin → stdout : chaque ligne **ancrée** sur `PIPELINE FAILURE: ` voit son marqueur
  remplacé par `Superseded (grooming converged, mika#2077): ` ; toute autre ligne passe
  octet pour octet.
- **R2.** L'ancrage est conservé. Une ligne portant `PIPELINE FAILURE:` ailleurs qu'en
  tête — la prose d'un plan, un extrait de log cité dans un diagnostic — n'est pas touchée.
  C'est le comportement actuel du `sed`, et le périmètre de ce vol est le résidu orphelin,
  pas l'ancrage.
- **R3.** Tous les marqueurs ancrés sont démotés, pas seulement le premier : un `RESULT` peut
  porter deux blocs d'échec (le post-flight de `_run_claude_pilot` puis la validation de plan).
- **R4.** Le site `dispatch-lib.sh:12447` appelle R1 au lieu du `sed`.
- **R5.** Un détecteur mesure R1–R4 sur les **messages réels** du fichier, et porte une partie
  anti-vacuité qui rejoue ses assertions contre le `sed` d'origine — elles doivent échouer.
- **R6.** Le détecteur est atteint par le pipeline : cible `make` dédiée, cible `test`, et un
  pas du job `Check` de `ci.yml`. Un détecteur que rien n'exécute ne refuse rien
  (`check-suite-wiring.sh`, mika#2761) ; comme il est câblé, il n'a **aucune** entrée à
  prendre dans `scripts/suite-wiring.tsv`.

## Non-requirements

- Pas de sentinelles aux ~15 sites d'écriture de `PIPELINE FAILURE:` (second remède de #2032).
  Hors borne, et il déplace le coût sur les producteurs.
- Pas de changement d'ancrage de `evaluator.rs:305`. Le `contains` y est volontaire (il lit
  des `RESULT` de provenances multiples) ; ce vol lui retire la chaîne au lieu de lui demander
  de changer d'avis.
- Pas de retouche aux messages d'échec eux-mêmes.
- Pas de second site de démotion. Le `sed` n'a qu'une occurrence, mesurée
  (`grep -c "/\^PIPELINE FAILURE:/d" dispatch-lib.sh` → 1).

## Verification contract

| # | ce qui est mesuré | comment |
|---|---|---|
| V1 | un bloc `_classify_terminated_session` mode `full` traverse la démotion sans laisser `Session:`/`Turns:`/`Halt class:` sans tête | fixture = le message réel, assertion sur la ligne qui précède `Session:` |
| V2 | la sous-chaîne `PIPELINE FAILURE:` est absente de la sortie | `grep -cF` → 0, non ancré, comme `evaluator.rs:305` |
| V3 | la prose de chaque bloc est intégralement préservée | diff octet pour octet entrée/sortie, hors les marqueurs remplacés |
| V4 | deux blocs sont démotés, pas un | fixture à deux marqueurs, compte des `Superseded (` → 2 |
| V5 | une mention non ancrée de `PIPELINE FAILURE:` survit intacte | fixture avec le marqueur en milieu de ligne |
| V6 | le texte de démotion ne matche aucun motif retryable | boucle sur la table des motifs lue depuis `dispatch-lib.sh:5976`, jamais réécrite dans le test |
| V7 | `GROOM_SUCCESS_MARKER` reste atteignable après démotion + `_set_outcome_line` | la sortie porte exactement une ligne `^Outcome: PLAN_GROOMED` |
| V8 | anti-vacuité | V1, V2 et V4 rejouées sur une copie régressée au `sed` d'origine : elles doivent **échouer** |
| V9 | préconditions | ni bwrap, ni réseau, ni token — la suite est wirable sur runner hébergé par construction |

V6 lit la table des motifs depuis le fichier plutôt que de la recopier : une liste recopiée
dans le test est précisément l'allowlist que mika#2309 condamne — elle vieillit en silence le
jour où un motif est ajouté au site de vérité.

## Definition of Done

- `_demote_pipeline_failure_markers` existe et est appelée au site 12447 ; le `sed` a disparu.
- `skills/bundled/_shared/tests/test_demote_pipeline_failure.sh` passe, V1–V9 vertes.
- La suite est câblée : cible `make test-demote-pipeline-failure`, cible `test`, pas `Check` de `ci.yml`.
- `make check-suite-wiring` passe sans nouvelle entrée dans `scripts/suite-wiring.tsv`.
- `bash skills/bundled/_shared/test-dispatch-lib.sh` et les suites voisines du substrat restent vertes.
- `scripts/verify-pipeline.sh origin/main` passe.
- Le corps de mika#2077 coche la ligne `sed` avec la PR de ce vol pour preuve, et **laisse
  décochées** les deux lignes qui recoupent #2826.

## Acceptance criteria

Le ticket mika#2077 n'a pas de section `## Acceptance criteria` ; les critères ci-dessous sont
dérivés des Requirements et du Verification contract.

- **AC1.** Sur le chemin GROOMED, un `RESULT` portant un bloc `PIPELINE FAILURE` multi-ligne
  périmé est délivré sans aucune ligne de prose orpheline : chaque ligne de corps a toujours sa
  tête, démotée. Mesuré par V1 et V3.
- **AC2.** Le même `RESULT` ne contient plus la sous-chaîne `PIPELINE FAILURE:`, ancrée ou non,
  donc `milestone_manager::evaluator::dead_cause_of` ne le classe plus `PipelineFailure`.
  Mesuré par V2.
- **AC3.** Plusieurs blocs périmés dans un même `RESULT` sont tous démotés. Mesuré par V4.
- **AC4.** Une mention non ancrée de `PIPELINE FAILURE:` n'est pas altérée — l'ancrage du
  comportement d'origine est conservé. Mesuré par V5.
- **AC5.** Le texte de démotion n'est lu par aucun des motifs retryables du substrat, et la
  table de ces motifs est lue depuis son site de vérité et non recopiée. Mesuré par V6.
- **AC6.** Un groom convergé continue de porter exactement une ligne `Outcome: PLAN_GROOMED`
  en position de verdict, donc `task_state::tasks::groom_result_convergence` rend `Converged`.
  Mesuré par V7.
- **AC7.** Le détecteur sait dire non : ses assertions centrales échouent contre le `sed`
  d'origine. Mesuré par V8.
- **AC8.** Le détecteur est atteint par le pipeline sur toute PR, et `check-suite-wiring`
  passe sans exception déclarée. Mesuré par le job `Check` et `make check-suite-wiring`.

## Fire-Disposition

Ce plan livre un détecteur : `test_demote_pipeline_failure.sh`, dont le chemin de succès est
« aucun orphelin, aucun marqueur survivant ».

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est vide.**

- **L'ensemble des exceptions est vide par construction, pas par omission.** Le détecteur et
  le correctif atterrissent dans le même commit, et le détecteur ne mesure que la fonction
  neuve et son unique site d'appel. Il n'existe aucune violation préexistante à excepter : le
  site `12447` est le seul (`grep -c "/\^PIPELINE FAILURE:/d"` → 1), et il est réparé dans ce
  vol. Le détecteur atterrit donc **armé et vert**, et non désarmé (option b).
- **Ce qui empêche « vide » de se lire comme « vacant ».** Un détecteur sans exception et sans
  violation est indistinguable d'un détecteur qui ne sait pas dire non — c'est la classe que
  mika#1772 a trouvée en câblant `test-dispatch-lib.sh`. V8 est la réponse : les trois
  assertions centrales sont rejouées sur une copie de `dispatch-lib.sh` régressée au `sed`
  d'origine (écrite sous `.pilot-scratch/`), et la suite **échoue** si elles passent.
- **Ce qui ferait rougir l'exception si elle naissait.** Le jour où un second site de
  démotion apparaît, il n'a pas d'entrée d'allowlist à prendre : il sera soit un appel à la
  fonction, soit une régression que V2 attrape sur son propre fixture. L'auto-nettoyage est
  donc structurel — il n'y a pas d'entrée à devenir stale.
- Option (c) écartée : rien ici ne demande de cadrage opérateur.

## Découpage en vols

Le parapluie porte deux lignes traitables (`sed`, `_launch_revise_pilot`). Les deux ensemble
dépassent la borne par vol ; elles partent donc en deux vols, chacun mergeable seul.

**Ce vol (vol 2) : la ligne `sed`.** Elle passe d'abord parce que son rayon est le plus large :
le `sed` est sur le **chemin de succès** du grooming, donc tout groom convergé aujourd'hui peut
délivrer des fragments orphelins à mika-dev. La ligne `_launch_revise_pilot` est sur un chemin
d'échec plus rare (pilote de revise tué par un guardrail).

**Vol 3 : `_launch_revise_pilot` hors de la garantie d'honnêteté (#2033).** Le pilote de revise
est lancé par `_run_pilot_sandboxed` (`dispatch-lib.sh:9186`) sans que `status` / `.subtype` /
`.termination_reason` ne soient jamais extraits de sa sortie ; une session tuée par le même
`idle_timeout` que mika#1772 a rendu mesurable pour le pilote principal est rapportée
`plan unchanged after revise pilot` — c'est-à-dire comme un modèle qui a refusé de réviser.
Deux contraintes déjà mesurées, à porter dans le plan du vol 3 : (i) `_classify_terminated_session`
lit les globales `STATUS`/`SUBTYPE`/`SESSION_ID`/`TURNS`/`DURATION`, que `_launch_revise_pilot`
ne peut pas écraser sans corrompre l'état que le callback du pilote principal lira ensuite ;
(ii) son appelant (`:10221`) ne pose pas `GROOM_LOOP_FAILURE_REASON`, donc l'exit final dit
« no reason recorded » — la seconde moitié de la même fausse attribution.

Tracked in: senara-solutions/mika#2077

## Ordre de commit

1. `_demote_pipeline_failure_markers` + sa doctrine dans `dispatch-lib.sh` (R1–R3).
2. Le site 12447 bascule sur la fonction ; le `sed` disparaît (R4).
3. `test_demote_pipeline_failure.sh`, V1–V9 (R5).
4. Câblage : `Makefile` (cible dédiée + cible `test`), `ci.yml` job `Check` (R6).

Plancher mergeable : les quatre pas. Un correctif sans son détecteur câblé est précisément ce
que ce parapluie reproche aux six suites non exécutées de sa première ligne.

## Risques

- **Un lecteur de `tasks.result` non recensé lirait `Superseded`.** Mesuré par `grep -rni
  superseded` sur `crates/`, `scripts/` et `skills/bundled/` : aucune occurrence en préfixe de
  ligne de `RESULT` (les occurrences Rust sont des variantes de `qa_head_supersession` et de
  `cancel_task_superseded`, sans rapport avec `tasks.result`). V6 épingle la table des motifs.
- **La prose démotée reste longue dans le callback.** Acceptée : c'est l'intention — un échec
  dépassé est un fait, et le `sed` l'effaçait. Le marqueur retryable, lui, disparaît bien.
- **Le fixture se désynchronise du message réel.** Borné : les fixtures sont les messages du
  fichier, et V3 est un diff octet pour octet, donc une dérive se lit comme un échec de V3 et
  non comme un faux vert.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — `_demote_pipeline_failure_markers` + doctrine | 55 |
| `skills/bundled/_shared/dispatch-lib.sh` — site 12447 (bascule + commentaire) | 12 |
| tests (`skills/bundled/_shared/tests/test_demote_pipeline_failure.sh`) | 175 |
| `Makefile` (cible dédiée + ligne dans `test`) | 3 |
| `.github/workflows/ci.yml` (pas du job `Check`) | 4 |

Total estimé : 249 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

249 < 279, et 249 < 1000 : un seul vol. **Pourquoi il tient** — le correctif est une fonction
d'une douzaine de lignes utiles sur un site d'appel unique et déjà localisé ; le volume est aux
deux tiers dans le détecteur et dans la doctrine, qui ne demandent ni exploration ni itération.
Aucun code Rust, donc pas de cycle `cargo build`. La seconde ligne traitable du parapluie part
au vol 3 précisément pour que ce total reste sous la borne.
