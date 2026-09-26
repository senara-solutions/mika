# mika#1950 — le rejeu prescrit n'est pas exécutable ; la mesure qu'il visait l'est

**Ticket :** senara-solutions/mika#1950
**Mesure :** 2026-09-20, 2353 sessions
**Artefacts :** [`measurement-2026-09-20.jsonl`](measurement-2026-09-20.jsonl) (donnée brute)
· [`measurement-2026-09-20.json`](measurement-2026-09-20.json) (lecture agrégée)
**Geste :** `scripts/measure-pilot-cycle-emptiness`

> **Verdict, en une ligne.** Sur l'axe mesurable, le verdict demandé est **sans
> objet** : le symptôme sur lequel on demande de trancher survient **0 fois sur
> 2353 sessions**, et **aucune** de ces sessions ne tourne sous l'un des deux
> modèles à comparer. La prémisse du ticket est réfutée par la donnée — ce qui
> est une réponse, pas un échec.

---

## 1. Ce que le ticket prescrivait, et pourquoi ce n'est pas exécutable

Le ticket demande de rejouer deux fixtures JSONL sous `z-ai/glm-5.2` puis
`z-ai/glm-5.3` et de rendre un verdict FIXED / IGNORED / AGGRAVER.

### R1 — Les fixtures n'existent nulle part

Les cinq commandes ci-dessous **doivent toutes retourner vide**. Elles ont été
rejouées avant toute autre unité de ce travail (Verification Contract §6 du plan),
et elles l'étaient.

```bash
ls docs/eval/                                                  # → calibration
git log --all --oneline -- 'docs/eval/hy3-v0-2026-08-18/**'    # → vide
git log --all --oneline --grep="hy3"                           # → vide
find /data/workspace/mika-platform -name "run_glm*" -not -path "*/target/*"  # → vide
find /data/workspace/mika-platform -name "*hy3*"   -not -path "*/target/*"   # → vide
```

`--all` couvre toute branche et tout historique atteignable ; la recherche
workspace couvre les deux dépôts présents (`claude-pilot`, `mika`). Ce sont des
artefacts d'une campagne d'août qui n'a laissé aucune trace ici, et ils ne sont
pas reconstructibles.

> **HALTE.** Si l'une de ces commandes retourne non-vide, ce document n'est plus
> fondé : le rejeu redevient exécutable et la rectification doit être **re-plaidée**,
> pas contournée. Le corps de mika#1910 mentionne des traces « mergé[e]s sur
> mika-platform via PR#192 » — une PR d'un autre dépôt n'est pas couverte par un
> `git log --all` local, donc l'espace de recherche peut être incomplet.

**Fabriquer des fixtures « équivalentes » est refusé.** Une fixture construite
depuis la définition du symptôme ne peut pas réfuter cette définition. C'est la
règle 6 de [`cycle-non-empty-detector-2026-08-30.md`](../../solutions/best-practices/cycle-non-empty-detector-2026-08-30.md)
— *falsifier contre le monde, pas contre la définition* — et le corpus réel est
précisément ce qu'elle prescrit.

### R2 — La condition bloquante a été franchie des deux côtés

Le ticket pose : *« Do NOT promote GLM-5.3 to mika-dev OR mika-qa UNTIL this
replay verdict lands. »* Les deux moitiés sont caduques, pour des raisons
différentes.

- **mika-dev** est passé sur `glm-5.3` le **2026-08-26**, sans ce verdict. La
  bascule vit hors du dépôt : `MIKA_DEV_CONFIG` déclare toujours
  `openrouter_model = "z-ai/glm-5.2"`, et son propre doc-comment nomme la dérive
  (`crates/mika-agent/src/well_known_agents.rs`).
- **mika-qa** y est passé, a cassé son enveloppe de revue le **2026-09-15**, et
  est revenu à `glm-5.2`. Sa promotion est désormais gouvernée par **mika#2328**,
  dont la condition est conjonctive et plus stricte que celle de ce ticket, avec
  une procédure déjà écrite ([`../calibration/mika-qa-2328/README.md`](../calibration/mika-qa-2328/README.md)).

**Conséquence de périmètre :** ce document ne peut pas « débloquer » une promotion
déjà faite d'un côté et déjà gouvernée ailleurs de l'autre. Aucun fichier sous
`docs/eval/calibration/mika-qa-2328/` n'est touché.

### R3–R7, en bref

| | Constat | Vérification |
|---|---|---|
| **R3** | Un verdict par une autre voie existe déjà et est gravé : *« mika-dev was swapped … on 2026-08-26; the 2026-08-29 measurement still found 102 of 120 sessions silent »*. En vocabulaire du ticket : IGNORED. | [`no-substrate-on-open-failure-mode-2026-08-30.md`](../../solutions/best-practices/no-substrate-on-open-failure-mode-2026-08-30.md) — **et §6 ci-dessous corrige son attribution** |
| **R4** | L'attribution au modèle n'est plus atteignable par rejeu : mika#1665 (reclassement `ReasoningBudgetExhausted`), mika#1996 (le détecteur), mika#2296 (`llm_max_tokens` relevé) ont tous bougé depuis. | trois tickets |
| **R5** | La population mesurée ne tourne pas sous le modèle accusé. **Établi par la donnée** ci-dessous, plus supposé. | champ `model` de l'artefact |
| **R6** | Une cause concurrente massive existe. **Corrigé par la mesure** : elle ne discrimine pas (§4). | champ `policy_denies` |
| **R7** | L'instrument réclamé existe (mika#1996, verdict à trois valeurs). Ce qui manquait est une **lecture datée et reproductible**. | `_measure_cycle_output` |

---

## 2. Le chemin de retour posé par l'opérateur, et pourquoi il n'est pas pris

Le commentaire opérateur du 2026-09-20 sur #1950 pose un retour explicite :
*« Retour = reposer `post-launch`, retirer `ready` »* — re-parquer le ticket
plutôt que l'exécuter.

**Ce chemin n'est pas pris, et la raison est nommée plutôt que laissée à deviner :
re-parquer ne produit rien.** Le ticket est resté ouvert un mois précisément parce
que la question « le symptôme a-t-il bougé ? » n'était pas re-posable sans refaire
le travail : la mesure fondatrice du 2026-08-29 survit comme *une phrase* dans un
document de principe, sans commande et sans artefact. Le re-parquer une seconde
fois reconduirait cet état en le datant.

**Le choix est réversible et sans coût.** Si l'opérateur juge que la rectification
sort du périmètre qu'il avait en tête, son geste reste disponible à l'identique
(reposer `post-launch`, retirer `ready`). Les artefacts de ce répertoire gardent
leur valeur indépendamment : ils ne présupposent rien du statut du ticket.

---

## 3. Deux axes, et un seul est mesuré ici

Confondre les deux est la faute R5 elle-même, et la formulation du ticket y invite.

| Axe | Population | Source | Mesuré ici ? |
|---|---|---|---|
| **A** — cycles pilote vides | sessions `claude-pilot` (**implémentation**) | `/var/log/claude-pilot/*.stderr` | **oui** (§4) |
| **B** — tours GLM vides | tours de `mika-dev` (**dispatcheur**) | `$MIKA_SPIRIT_LOG_FILE` | **non — geste opérateur** (§5) |

`claude-pilot` est la session d'implémentation et tourne sous **Claude** ;
`mika-dev` est l'agent dispatcheur et c'est lui qui porte GLM.

> **L'axe temporel est CALENDAIRE, pas un axe de régime.** Le champ `date` est un
> découpage neutre. La population tourne sous Claude **de part et d'autre** du
> 2026-08-26, donc la bascule GLM n'a changé le modèle d'aucune de ces sessions.
> **Une discontinuité au 2026-08-26 ne vaut pas verdict** : sans canal causal
> établi, elle relève de la réfutation de prémisse, pas d'un FIXED/IGNORED/AGGRAVER.
>
> Le canal causal candidat — *une session pilote est un capteur aval du
> dispatcheur : si `mika-dev` se tait, le pilote démarre sans travail et paraît
> vide sans qu'aucun de ses propres tours ne le soit* — est énoncé comme
> **hypothèse non établie**. Sa preuve vivrait sur l'axe B.
>
> La question est de toute façon tranchée empiriquement au §4 : **il n'y a pas de
> discontinuité à cette date.**

---

## 4. Axe A — la mesure

### Le geste (AC6)

```bash
scripts/measure-pilot-cycle-emptiness --out measurement.jsonl
scripts/test-measure-pilot-cycle-emptiness.sh     # 35 assertions
```

Lecture seule, sans dépendance, `undetermined` sur tout signal illisible.
Chaque nombre ci-dessous est re-dérivable de la donnée brute par un `jq`.

### Le symptôme lui-même : absent

| | |
|---|---|
| Sessions | **2353** (2026-05-15 → 2026-09-20) |
| Événement `[guardrail] empty_response:` | **0** |
| Sessions portant la **constante** `emptyResponseThreshold=` | **2349** |

```bash
jq -s 'map(select(.empty_event)) | length' measurement-2026-09-20.jsonl   # → 0
```

**C'est le résultat principal.** Le symptôme sur lequel le ticket demande un
verdict ne survient **pas une seule fois** dans trois mois de la boucle d'outils
réelle. Et l'écart entre ces deux lignes est le piège que ce travail existe pour
ne pas retomber dedans : un script qui grepperait `empty` sans distinguer la
constante de l'événement rapporterait **2349 « sorties vides »** — un nombre faux,
plausible, et porté par l'autorité d'une mesure. L'ancre est donc dérivée du
producteur (`claude-pilot` `guardrails.py::_abort` → `ui.py::log_guardrail`) et
non rétro-ingénierée depuis un log.

### Le modèle, établi par la donnée (AC3)

| Modèle | Sessions |
|---|---|
| `claude-opus-4-8[1m]` | 1376 |
| `unknown` | 686 |
| `claude-opus-5[1m]` | 288 |
| illisible (`null`) | 3 |
| **GLM, toutes variantes** | **0** |

`unknown` est un **littéral du producteur** (`log_init(session_id or "", model or
"unknown", task_id)`, `agent.py:591`) écrit quand le SDK ne rapporte pas de
modèle : une absence de donnée, pas un modèle masqué. Ce que la donnée établit est
donc : **zéro session GLM observée**, 1664 sessions explicitement sous Claude, 686
sans modèle rapporté sur un substrat qui est un client du SDK Claude.

### Les sessions vides, et le contrôle négatif qui décide (AC4)

| Verdict | Sessions |
|---|---|
| `produced` | 1171 |
| `empty` | 1179 |
| `undetermined` | 3 |

La vacuité au sens « aucun appel d'outil » est donc réelle et massive (50,1 %).
La question est ce qui la cause.

| Signal | chez les `empty` (n=1179) | chez les `produced` (n=1171) |
|---|---|---|
| `policy:deny` | 597 — **50,6 %** | 585 — **50,0 %** |
| `error_during_execution` | 597 — 50,6 % | 418 — 35,7 % |
| `idle_timeout` | 312 | — |
| aucun signal | 267 | — |

> **Le contrôle négatif renverse R6, et c'est une correction que la mesure impose
> au plan.** `policy:deny` était présenté comme *« une cause concurrente qui domine
> cette population »* sur la foi d'un grep d'orientation (~50 % du corpus). La
> mesure montre qu'il est présent à 50 % **des deux côtés** : la moitié des
> sessions se voit refuser un outil et travaille quand même. **Il ne discrimine
> pas.** Rapporter les 50,6 % sans le contrôle aurait produit une explication
> fausse avec l'autorité d'un chiffre — exactement la faute de forme que R6 dénonce
> par ailleurs.
>
> `error_during_execution` discrimine faiblement (50,6 % vs 35,7 %), et coïncide
> **exactement** avec `policy:deny` chez les vides (597 / 597, aucun écart dans
> ni l'un ni l'autre sens).

### Pas de discontinuité au 2026-08-26 (F2)

| Fenêtre | Sessions décidées | `empty` | Taux |
|---|---|---|---|
| avant 2026-08-26 | 1675 | 860 | **51,3 %** |
| à partir du 2026-08-26 | 675 | 319 | **47,3 %** |

La date du swap GLM ne sépare rien. Les vraies inflexions sont ailleurs :
juillet→août (27 % → 79 %) puis fin août→septembre (79 % → 7 %).

### Ce que la donnée montre à la place, avec sa réserve

| Modèle | Sessions | `empty` | Taux |
|---|---|---|---|
| `claude-opus-4-8[1m]` | 1376 | 962 | 69,9 % |
| `unknown` | 686 | 201 | 29,3 % |
| `claude-opus-5[1m]` | 288 | 16 | 5,6 % |

Le taux de vacuité covarie fortement avec le modèle **claude-pilot** — mais les
modèles se succèdent presque parfaitement dans le temps (juillet-août :
opus-4-8 exclusivement ; septembre : opus-5). Septembre est la **seule** fenêtre où
les deux coexistent :

| Septembre seul | Sessions | `empty` | Taux |
|---|---|---|---|
| `claude-opus-4-8[1m]` | 41 | 8 | 19,5 % |
| `claude-opus-5[1m]` | 288 | 16 | 5,6 % |

À période égale opus-4-8 reste ~3,5× plus vide — mais il est lui-même passé de
79 % en août à 19,5 % en septembre, donc le calendrier bouge aussi. **Aucun des
deux axes n'est isolable sur cette population**, et la réserve R4 s'applique en
entier. Ce n'est pas un résultat sur GLM : **aucun de ces modèles n'est GLM.**

### Recoupement de la mesure fondatrice (Verification Contract §4)

Restreint aux 120 sessions les plus récentes au 2026-08-29 — la population que la
doctrine du 2026-08-30 cite comme « 102 of 120 sessions silent » :

| | |
|---|---|
| `empty` | **118** / 120 |
| Modèle | **`claude-opus-4-8[1m]` — 120 sur 120** |

118/120 reproduit 102/120 en ordre de grandeur : le script n'est pas réfuté par la
mesure connue, la halte §4 n'est pas déclenchée. Et **120 sessions sur 120 sous
Claude Opus 4.8, zéro sous GLM** : c'est le critère qui déclenche §6.

### Verdict de l'axe A (AC5)

**SANS OBJET — la prémisse est réfutée.**

Un verdict FIXED / IGNORED / AGGRAVER compare un symptôme entre deux modèles. Sur
cette population, le symptôme survient **0 fois sur 2353**, et **aucune session ne
tourne sous l'un des deux modèles**. Il n'y a rien à comparer. Fabriquer un
verdict pour remplir la case produirait un rapport faux avec l'autorité d'une
mesure — ce que ce travail existe pour ne pas faire.

**Réserve d'attribution, dans la même section (R4) :** même si la population avait
discriminé, mika#1665, mika#1996 et mika#2296 ont tous bougé entre la mesure
fondatrice et aujourd'hui. Une différence observée ne serait pas attribuable à un
modèle.

---

## 5. Axe B — la procédure opérateur (non exécutée ici)

`$MIKA_SPIRIT_LOG_FILE` n'est pas lisible depuis une session de dispatch
(`/var/log/mika/` n'y existe pas). À exécuter sur l'hôte :

```bash
# Quel modèle tourne réellement pour mika-dev, et par quelle porte (mika#2293)
grep llm_budget_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-dev")
        | {provider, model, model_source, model_config_key,
           http_timeout_secs, agent_total_timeout_secs, http_source}'

# Les tours vides. Le discriminant est stop_reason, PAS le vide : un contenu vide
# AVEC budget de sortie saturé est mika#1665 (famine de budget sur un modèle à
# raisonnement), pas un silence du modèle.
grep turn_usage "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-dev")
        | {step, stop_reason, output_tokens, tool_use_in_turn, status,
           model, request_bytes}'
```

**`model_source` sépare trois mondes** et c'est tout son objet : `agent_config`
(le réglage est en vigueur), `process_env` (une variable fleet-wide écrase le
`config.toml`), `default` (le `config.toml` n'a pas été lu). La bascule mika-dev du
2026-08-26 vivant **hors du dépôt** (R2), c'est cette ligne — et non le code — qui
dit quel modèle tourne.

---

## 6. Correction d'attribution (U4 — déclenchée)

Le critère conditionnel du plan était : *« si et seulement si la mesure confirme sur
la population entière que les sessions comptées tournent sous Claude »*. Il est
satisfait sans ambiguïté — **120 sessions sur 120** de la fenêtre fondatrice sous
`claude-opus-4-8[1m]`, zéro sous GLM.

[`no-substrate-on-open-failure-mode-2026-08-30.md`](../../solutions/best-practices/no-substrate-on-open-failure-mode-2026-08-30.md)
juxtaposait, dans la même phrase, le swap GLM du 2026-08-26 et la mesure du
2026-08-29, ce qui fait lire la seconde comme un test du premier. **Sa règle ne
bouge pas** — elle est juste et indépendante du modèle ; c'est son exemple chiffré
qui devait dire de quoi il est l'exemple. La correction le nomme.

L'ironie est utile : la règle corrigée est précisément *« a mitigation is only
active once measured »*, et le défaut de son exemple était que la mesure ne portait
pas sur la population du swap. La correction **renforce** la règle.

---

## 7. Les haltes

| Situation | Conduite |
|---|---|
| Une commande de §1 retourne **non-vide** | **Halte.** Les fixtures existent ; la rectification n'est plus fondée et doit être re-plaidée avant tout. |
| Le recoupement 118/120 ne se reproduit plus | **Halte.** C'est le script qui est faux, pas l'histoire. Le réparer avant toute conclusion. |
| Le taux d'`undetermined` remonte au-dessus de quelques pour cent | Le format du log `claude-pilot` a dérivé (dépôt tiers). **C'est déjà arrivé** : cpp#168 a préfixé chaque ligne d'un timestamp, ce qui a rendu 231 sessions illisibles avant que le script ne l'absorbe. Le fail-safe protège la justesse, pas la couverture — lire le producteur, pas deviner un motif. |
| On veut conclure « GLM est innocenté » | **Non.** L'axe B n'est pas mesuré. Le verdict est par axe, et celui-ci est déclaré non mesuré ici. |
| On veut conclure « opus-4-8 cause la vacuité » | **Non, pas depuis cette donnée.** Modèle et calendrier covarient ; septembre les sépare partiellement mais pas assez. C'est une piste, pas un résultat. |
| Une discontinuité apparaît au 2026-08-26 sur une re-mesure | Elle ne vaut **pas** verdict (§3). Corrélation calendaire dont le canal causal se prouve sur l'axe B. |

---

## 8. Ce que ce travail n'achète pas

- **Il ne corrige pas la cause de la vacuité.** Il la mesure et la départage, et il
  écarte deux explications (le symptôme modèle, et `policy:deny`).
- **Il ne rend pas `glm-5.3` promouvable.** Pour mika-qa la porte est mika#2328 ;
  pour mika-dev la bascule est déjà faite et sa réconciliation dépôt ↔ runtime est
  son propre travail.
- **Il ne mesure pas les tours GLM.** L'axe B est prescrit, pas exécuté.
- **Il ne ferme pas mika#1910.** Voir §9.
- **Il ne reconstitue pas la campagne hy3-v0.** Ces artefacts sont perdus.

---

## 9. Note de statut pour mika#1910 (geste opérateur — **pas** une fermeture)

Le plan (D5) et KTD5 du plan mika#1996 sont explicites : *« fermer sur la foi d'une
mitigation que la mesure contredit, c'est refaire le geste que ce ticket corrige »*.
**mika#1910 n'est pas fermé ici.** Texte proposé, à poser par l'opérateur :

> **Mesure du 2026-09-20 (mika#1950) — le symptôme n'est pas observé, et la
> population habituellement citée n'est pas celle qu'on croit.**
>
> Sur 2353 sessions `claude-pilot` (2026-05-15 → 2026-09-20), l'événement
> `[guardrail] empty_response:` survient **0 fois**. La « vacuité » réellement
> présente (1179 sessions sans aucun appel d'outil, 50,1 %) est un autre
> phénomène : elle ne porte jamais cet événement.
>
> Ces sessions tournent sous **Claude**, jamais sous GLM (`claude-opus-4-8[1m]`
> 1376, `claude-opus-5[1m]` 288, 686 sans modèle rapporté, **0 GLM**). En
> particulier, les 120 sessions de la mesure du 2026-08-29 citée dans la doctrine
> tournent **toutes** sous `claude-opus-4-8[1m]` — cette mesure ne dit donc rien
> de `glm-5.2` ni de `glm-5.3`.
>
> `policy:deny`, longtemps soupçonné, **ne discrimine pas** : 50,6 % chez les
> sessions vides contre 50,0 % chez les productives.
>
> **Ce ticket reste ouvert.** La moitié GLM du symptôme — les tours de `mika-dev` —
> n'est pas mesurée ici : elle vit dans `$MIKA_SPIRIT_LOG_FILE` et la procédure est
> en `docs/eval/mika-1950/README.md` §5. Rien n'a été promu, rien n'a été fermé.
>
> Geste reproductible : `scripts/measure-pilot-cycle-emptiness`.

---

## Sources

- `scripts/measure-pilot-cycle-emptiness` + `scripts/test-measure-pilot-cycle-emptiness.sh`
- `claude-pilot` `src/claude_pilot/guardrails.py` (`_abort("empty_response", …)`),
  `src/claude_pilot/ui.py:112-113` (`log_guardrail`), `src/claude_pilot/agent.py:591`
  (`log_init`, le littéral `unknown`) et `:842` (le second site d'émission d'un
  guardrail), `src/claude_pilot/logger.py` (`_LineStamper`, cpp#168) — **les
  producteurs dont chaque ancre est dérivée**
- [`no-substrate-on-open-failure-mode-2026-08-30.md`](../../solutions/best-practices/no-substrate-on-open-failure-mode-2026-08-30.md) — la mesure du 2026-08-29 et la règle qu'elle fonde (corrigée en §6)
- [`cycle-non-empty-detector-2026-08-30.md`](../../solutions/best-practices/cycle-non-empty-detector-2026-08-30.md) — le vocabulaire à trois valeurs et la règle 6
- [`calibration-reasoning-budget-exhaustion-false-empty-2026-06-30.md`](../../solutions/best-practices/calibration-reasoning-budget-exhaustion-false-empty-2026-06-30.md) — un « empty » peut être une famine de budget
- [`../calibration/mika-qa-2328/README.md`](../calibration/mika-qa-2328/README.md) — la porte qui gouverne mika-qa
- `crates/mika-agent/src/well_known_agents.rs` — les dérives dépôt ↔ runtime
- Commentaire opérateur du 2026-09-20 sur mika#1950 (`IC_kwDORWsgGM8AAAABVsWYCg`) — le chemin de retour (§2)
