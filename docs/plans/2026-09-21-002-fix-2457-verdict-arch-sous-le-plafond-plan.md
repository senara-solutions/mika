# mika#2457 — le verdict arch sous le plafond : établir avant de régler

> **Ce que ce plan livre, en une phrase.** Il **refuse les deux étapes du ticket
> telles qu'écrites** — preuves à l'appui, tirées du dépôt — et livre à la place
> la pièce sans laquelle ni l'une ni l'autre n'est décidable : **mika-spirit
> atteste le budget et le modèle qu'il a réellement résolus pour un agent**, et
> `mika agents budget` les lit. Aucune valeur de réglage ne bouge.

---

## 1. Rectification : cinq affirmations du ticket contredites par le dépôt

Le ticket décrit une configuration qui n'est celle d'aucune ligne de ce dépôt.
Relevé à HEAD (`93931d40`), `crates/mika-agent/src/well_known_agents.rs:1524`
(`MIKA_ARCH_CONFIG`) :

| # | Affirmation du ticket | Ce que le dépôt déclare |
|---|---|---|
| A1 | modèle **`kimi-k3`** | `openrouter_model = "moonshotai/kimi-k2.5"` — et `grep -rn "kimi-k3"` sur `crates/` + `docs/` rend **zéro ligne** : k3 est absent du dépôt entier |
| A2 | plafond par appel **300 s** | `llm_http_timeout_secs = 240` |
| A3 | `llm_max_tokens` = **16384** | `llm_max_tokens = 32768` (16384 est la valeur de **mika-qa**, l. 256) |
| A4 | « ~11250 tokens atteignables **mesurés** au cut » | `reachable_output_tokens` = `plafond × 3/4 × plancher` (`llm/budget.rs:474`). À 240 s : **9000**. À 300 s : **11250**. Ce n'est pas une mesure de tokens produits, c'est la formule évaluée à **300 s** |
| A5 | « baisser à ~10000, **sous les 11250** » | 10000 est sous l'atteignable à 300 s, mais **au-dessus** de l'atteignable à 240 s (9000) — la marge dépend du plafond réellement en vigueur, qui n'est pas établi |

**A4 est le fait le plus utile du ticket, et il dit autre chose que ce qu'il
croit dire.** `11250 = 300 × 3/4 × 50` exactement. Le chiffre n'a pas été mesuré :
il a été lu sur une ligne (`llm_budget_resolved` ou `llm_call_cap_exhausted`) dont
le `http_timeout_secs` valait **300**. Il atteste donc que le plafond en service
n'est **pas** le 240 du `config.toml` — c'est-à-dire qu'une porte de la cascade
au-dessus du fichier per-agent (variable d'environnement de service) gagne.

C'est la question que mika#2342 a déjà posée et laissée ouverte, en toutes
lettres : *« Le ticket affirme 420/600 sur le process ; si c'est exact, le
`240/900` posé à mika-arch par mika#2189 est écrasé par une variable de service.
Lire `llm_budget_resolved` et regarder `http_source`. »* Trois valeurs de plafond
ont maintenant été affirmées pour le même agent — **240** (dépôt), **420**
(mika#2342), **300** (mika#2457) — et **aucune** n'a jamais été établie par
lecture d'instrument.

---

## 2. Pourquoi l'Étape 1 ne peut pas être livrée

Quatre raisons indépendantes. Une seule suffirait.

### R1 — La coupure observée est TEMPORELLE ; `max_tokens` ne la gouverne pas

`llm_call_cap_exhausted` est émis dans `crates/mika-common/src/llm/openai.rs:341`,
sur la seule branche où *les en-têtes sont arrivés et le corps n'a pas fini*. Le
commentaire du site (l. 217) définit le drapeau :

> *the body stopped arriving at ≈ the per-call plafond, i.e. **the model was still
> generating**, as opposed to stopping at an arbitrary instant*

L'appel a été tué par `reqwest` **au plafond de temps**, pendant que le modèle
produisait. `max_tokens` figure sur la ligne comme **contexte rapporté**
(`max_tokens = request.max_tokens`, l. 344), jamais comme cause. Abaisser un
plafond de sortie ne fait pas produire un modèle plus vite : ça le **coupe plus
tôt**. « Raccourcir le verdict » et « tronquer le verdict » sont deux choses, et
le levier proposé fait la seconde.

### R2 — La baisse réintroduit la panne que mika#2296 a corrigée il y a quatre jours

La dérivation est écrite dans la constante elle-même (`well_known_agents.rs:1530`) :

> *A reasoning model counts its thinking in the OUTPUT budget. On a heavy brief the
> thinking alone exhausted 8192 before the verdict was ever emitted: the arch pass
> `8b623724` ran 123 s — well inside its time budget — and ended on
> `stop_reason=MaxTokens`, `output_tokens=8192`, with an **EMPTY content**.
> dispatch-lib found no `Disposition:` line to parse and reported PIPELINE FAILURE.*

Le mécanisme est exactement celui que 10000 rouvre : un modèle de raisonnement
dépense son budget de sortie en *thinking* avant que le texte visible ne commence.
À 50 tok/s, 10000 tokens sont atteints vers 200 s ; au débit mesuré de 66 tok/s,
vers 151 s. L'appel « tiendrait sous 300 s » — littéralement vrai — **et rendrait
un contenu vide**. L'objectif réel est *un verdict avant le plafond*, pas *un appel
qui finit avant le plafond* ; la baisse satisfait le second en détruisant le
premier.

Le ticket anticipe à moitié l'objection (« pas plus bas que 8192 »), mais 8192
n'est pas un plancher magique : c'est la valeur **qui a échoué**. mika#2296 ne pose
pas un minimum, il pose un principe — *le budget de sortie doit être non
contraignant, le temps est le frein* — et 32768 est décrit comme « a ceiling made
non-binding », avec un facteur 2 de marge délibéré.

### R3 — Deux tests figent la valeur, et l'un porte le refus dans son message

```
crates/mika-agent/src/well_known_agents.rs:2790
  assert_eq!(config["llm_max_tokens"].as_integer(), Some(32768),
    "mika#2296: mika-arch's output budget must stay non-binding for a
     reasoning model — see the derivation in MIKA_ARCH_CONFIG");
```

et `mika2280_the_three_shipped_geometries_and_their_verdict` (l. 2862), dont la
ligne d'allowlist mika-arch porte `http_timeout_secs: 240, max_tokens: 32_768` avec
le motif `"decided (mika#2296): a non-binding output plafond, time is the brake —
**terminal**"`.

Ce ne sont pas des tests à mettre à jour. Ce sont des **décisions écrites**, datées
du 16/09 (`09bc473c`, PR #2332), qui refusent nommément ce que le ticket demande.
Le test frère `mika2296_no_well_known_config_declares_an_output_budget_below_8192`
énonce la conduite : *« Do NOT exempt this agent and do NOT raise the value here —
its budget is solidary with its model; surface the decision to the operator. »*

### R4 — Éditer `MIKA_ARCH_CONFIG` peut annuler le bearing qu'il sert

`reconcile_well_known_config` (l. 759) compare le fichier **entier**
(`on_disk == expected`) et, s'il diffère, réécrit `expected` **en entier**. Le
doc-comment de `MIKA_DEV_CONFIG` (l. 168) nomme le piège sur un agent voisin :

> *this constant's source has DRIFTED from its runtime. … `reconcile_well_known_config`
> rewrites the WHOLE file on the next provisioning pass, so touching one field here
> silently demotes the model … without a calibration run, and without anyone asking
> for it.*

Deux branches, toutes deux mauvaises :

- **Provisionnement actif** → l'édition réécrit le fichier entier et **démote
  k3 → k2.5**, sans calibration, en contradiction directe avec le bearing Prime
  (« garder k3 »).
- **Provisionnement gelé** (`MIKA_DISABLE_AGENT_PROVISIONING`) → l'édition
  **n'atterrit jamais**. C'est le sort qu'a connu mika#2327, inerte en production
  pour cette raison exacte.

La seconde branche est la plus probable : le `CLAUDE.md` racine décrit ce drapeau
comme protégeant précisément « a hand-picked `llm_provider` / `openrouter_model`
(Z.AI direct, **kimi**) ». Le mot est là. Et si k3 tourne vraiment, il vit hors
dépôt — même dérive que le `glm-5.3` de mika-qa mesuré par mika#2328.

**Laquelle des deux s'applique n'est pas établie.** C'est l'objet du § 4.

---

## 3. Pourquoi l'Étape 2 ne peut pas être livrée telle quelle

- **Aucun mécanisme de repli modèle n'existe.** `grep -rn "model_fallback|fallback_model|secondary_model|on_cap_exhausted"` sur `crates/` rend un seul résultat, dans un test de coût KG sans rapport. C'est une brique entière, pas un réglage.
- **Elle viole mika#1190 par construction.** *« Never swap an agent's base model … without a passing `make calibrate-<role>` run. »* Un repli **automatique** est un swap de modèle sur un chemin que personne ne voit, au moment le plus défavorable.
- **DeepSeek n'a aucune calibration mika-arch. Ni kimi.** `docs/eval/calibration/` contient `mika-dev-1221`, `mika-dev-1633`, `mika-orchestrator-1641`, `mika-qa-1632`, `mika-qa-2328`, `2264` — et **rien pour mika-arch**, alors que la suite (`calibration/roles/mika_arch.rs`) et ses fixtures existent. Il n'existe donc **aucune baseline** contre laquelle mesurer quoi que ce soit sur cet agent.
- **Le ticket la conditionne lui-même** : « Après la mesure ». Elle n'est pas à livrer ici.

Le chemin correct, quand la mesure du § 6 l'aura justifiée : un ticket propre
portant (a) `make calibrate-mika-arch` sur k2.5 pour créer la baseline manquante,
(b) la même sur le modèle de repli, (c) le mécanisme. Trois choses, dans cet ordre.

---

## 4. Ce que ce plan livre : spirit atteste ce qu'il a résolu

Le blocage de #2457 n'est pas un manque de code, c'est un **manque de fait** — et
le fait est illisible. `llm_budget_resolved` le porte déjà (agent, plafond,
enveloppe, `llm_max_tokens`, `reachable_output_tokens`, provider, modèle, et la
**provenance** de chaque moitié), mais il est émis une fois par agent et par couple
résolu, dédupliqué, dans ~19 Go de journal. L'opérateur qui veut savoir *« quel
modèle et quel budget tournent pour mika-arch, et par quelle porte »* n'a aucune
surface directe. C'est ce trou qui a laissé le `240/900` du 06/09 échouer en
silence jusqu'au 11/09, et c'est lui qui avale #2457 aujourd'hui.

### Le piège central de la conception : ne pas calculer côté CLI

`BudgetProvenance::resolve` lit `process_env` **du process qui appelle**. Une
sous-commande qui recalculerait localement rendrait `http_source = agent_config`
à 240, pendant que spirit tourne à 300 par variable de service : un champ qui
affirme avec autorité un réglage qui n'a pas lieu. C'est mot pour mot le défaut
mika#2304 (`--verbose` affichant le modèle demandé alors que le tour tournait sous
un autre) et la leçon mika#2270 (le serveur tenait la réponse et la jetait).

**Donc : spirit atteste, le CLI rend. Jamais l'inverse.**

### U1 — Un seul site de résolution, et il rend ce qu'il émet

`crates/mika-common/src/llm/budget_provenance.rs` : extraire de
`log_llm_budget_resolved` (l. 673) la construction du **record** résolu, sans
changer un champ ni une valeur :

```rust
pub struct ResolvedBudgetRecord { /* les champs de l'événement, tels quels */ }
pub fn resolve_llm_budget_record(agent_id, global_home, agent_home) -> ResolvedBudgetRecord;
pub fn log_llm_budget_resolved(...)  // appelle le premier, garde la dédup et le WARN mika#2362
```

Refactor **pur** : mêmes champs, même dédup, même `llm_budget_retry_unreachable`,
même ligne INFO. Un second résolveur serait un résolveur libre de diverger du
premier — la classe que `grooming_marker` (mika#2158) a dû fermer une fois.

### U2 — `AgentState` garde ce que son `init_agent` a résolu

`crates/mika-agent/src/server/mod.rs:476` appelle déjà
`log_llm_budget_resolved(agent_name, global_home, agent_home)`. Remplacer par un
appel à `resolve_llm_budget_record`, stocker le record sur `AgentState`, puis
émettre. Le champ est un `Arc<ResolvedBudgetRecord>` posé à l'init et **jamais
recalculé** — même contrat *not hot-swappable* que `AgentState.tier` (mika#1962) et
`AgentState.deployment` (mika#2290), et pour la même raison : c'est l'état sous
lequel cet agent **tourne**, pas celui que le disque porte maintenant.

### U3 — Une route read-only

`GET /api/v1/agents/{name}/budget`, auth dashboard-ou-interne comme ses voisines
(`/api/v1/agents/{name}/sessions`, `/api/v1/agents/{name}/audit` existent déjà —
le motif est en place). Rend le record de l'`AgentState`, tel quel. **404 si
l'agent n'est pas résolu** — et surtout : ne recalcule rien, ne relit pas le
disque. Un agent non servi n'a pas de budget « en vigueur » à rapporter, et en
inventer un serait le faux vert que U1/U2 existent pour empêcher.

### U4 — `mika agents budget [--agent <name>]`

Nouvelle variante de `AgentsCommand` (`crates/mika-cli/src/cli.rs:389`, à côté de
`Reprovision`). Interroge `MIKA_SPIRIT_URL`, rend texte (défaut) ou `--format json`.
Sortie texte, une ligne par fait, chacune avec sa provenance :

```
mika-arch
  provider   openrouter          (agent_config)
  model      moonshotai/kimi-k2.5 (agent_config, clé: openrouter_model)
  plafond    240 s               (agent_config)
  enveloppe  900 s               (agent_config)
  max_tokens 32768               (agent_config)
  atteignable 9000 tokens  @ 50 tok/s
  tentatives 3 nominales / 3 atteignables
```

**Si spirit ne répond pas, ou répond 404, le CLI ne calcule PAS de repli local.**
Il dit *« ce serveur n'a rien attesté »* — population que mika#2304 a dû nommer
pour exactement cette raison, et dont l'ambiguïté (binaire antérieur au correctif
vs agent non servi) est préférable à une valeur fausse.

### Ce que ce livrable n'est PAS

- **Pas une garde.** Il *mesure* la dérive code↔runtime ; il ne la refuse pas. La garde est le suivi que mika#2328 s'est écrit (« Cet événement mesure la dérive ; rien ne l'empêche »), et l'ouvrir ici serait changer de ticket.
- **Pas un changement de valeur.** Plafond, enveloppe, `max_tokens`, modèle : aucun ne bouge. Les deux tests du § R3 restent verts, sans modification.
- **Pas un filet de verdict.** `_arch_ask_with_retry` (mika#2278, `dispatch-lib.sh:5016`) borne déjà le retry transport et dit lui-même sa limite : `"the single retry was spent and the pass is lost anyway"`. Sur une coupure au plafond, la cause est la longueur du raisonnement, pas un aléa réseau — un second retry recouperait au même endroit.

---

## 5. Contrat de vérification

| Unité | Ce qui est asserté | Ce qui rougit si on se trompe |
|---|---|---|
| U1 | `resolve_llm_budget_record` et `log_llm_budget_resolved` rendent le **même** couple sur les quatre positions de la cascade | un second résolveur qui diverge |
| U1 | **Contrôle négatif** : la dédup et le WARN `llm_budget_retry_unreachable` sont inchangés (mêmes émissions sur la même séquence d'appels) | un refactor qui déplace la dédup |
| U2 | Le record servi est celui de l'init : muter le `config.toml` après démarrage **ne change pas** ce que la route rend | un recalcul par requête (le faux vert mika#2304) |
| U3 | Agent servi → 200 + record ; agent inconnu → **404**, jamais un record calculé | un repli qui invente une provenance |
| U4 | Spirit injoignable ou 404 → *« non attesté »*, **aucune** valeur locale affichée | un `BudgetProvenance::resolve` appelé dans le process CLI |
| U4 | Un `provider` dont le modèle est illisible rend `unknown_provider`, pas `default` | la fausse provenance que `budget_provenance.rs` refuse déjà |
| Non-régression | `test_mika_arch_config_toml_is_valid_toml` et `mika2280_the_three_shipped_geometries_and_their_verdict` **verts sans modification** | toute valeur touchée |

Le contrôle négatif d'U4 est celui qui porte : sans lui, « le CLI lit le serveur »
et « le CLI calcule localement » produisent exactement la même sortie sur un poste
où les deux processus partagent l'environnement — c'est-à-dire sur le poste de
développement où le test serait écrit.

---

## 6. Sonde post-déploiement, et ses quatre haltes

Après déploiement, sur le vrai serveur :

```bash
mika agents budget --agent mika-arch
```

Elle tranche A1–A4 en une commande. Table de lecture :

| Observation | Lecture | Conduite |
|---|---|---|
| `model` ≠ `moonshotai/kimi-k2.5` | **la dérive hors dépôt est confirmée et mesurée** — k3 vit sur disque | **Halte 1.** Noter la valeur, sa provenance et la date **avant** de toucher au disque. Toute édition de `MIKA_ARCH_CONFIG` démote le modèle : le préalable est un ticket de réconciliation modèle + calibration (mika#1190), pas une baisse de `max_tokens` |
| `plafond` ≠ 240 avec `http_source = process_env` | une variable de service écrase le `config.toml` | Le remède est de **retirer la variable de l'environnement du service**, pas de toucher une constante. Referme aussi la question ouverte de mika#2342 |
| `plafond = 240`, `http_source = agent_config` | le réglage est bien en vigueur | Alors A2 et A4 du ticket sont faux, et les coupures à 300 s viennent d'ailleurs — **ne pas conclure sur `max_tokens`** |
| `max_tokens_source = default` | le `config.toml` n'a pas été lu ou ne porte pas la clé | Provisionnement gelé : l'Étape 1 aurait été **inerte**. Geste de provisionnement, pas de code |
| **Aucune ligne, agent inconnu** | **Halte 2.** Le binaire servi est antérieur au correctif (classe mika#2340), ou l'agent n'est pas servi. Établir le déploiement **avant** toute conclusion sur le texte |

**Halte 3 — la charge, avant le budget.** Cinq corrections ont déjà porté sur ce
chemin entre le 06 et le 20/09 : mika#2189 (géométrie), mika#2278 (retry
`_arch_ask`), mika#2295+#2330 (fenêtre d'historique), mika#2363 (`--only-skill`,
−23,5 à −30,6 Ko de prompt arch), mika#2296 (budget de sortie). Avant de conclure
que le budget est le levier, lire ce que mika#2331 a livré exactement pour ça :

```bash
grep turn_usage "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-arch") | {status, latency_ms, request_bytes, system_prompt_bytes}'
```

Si `request_bytes` des tours en échec est nettement supérieur à celui des tours
sains, le levier est **la taille du brief**, que mika#2189 a nommée hors périmètre
et laissée à son ticket (« shrinking mika-arch's system prompt, which grew 54 KB →
59.8 KB on 2026-09-01 and is the proximate reason this agent crossed the line »).
C'est un ticket de suivi, pas un réglage.

**Halte 4 — si les coupures cessent.** Les cinq corrections ci-dessus ont pu déjà
refermer le symptôme. Mesurer avant de construire :

```bash
grep llm_call_cap_exhausted "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.model | test("kimi")) | {model, max_tokens, http_timeout_secs, elapsed_ms}'
```

Zéro ligne sur 48 h ⇒ **#2457 se referme sur la mesure**, et l'Étape 2 n'a pas
d'objet. C'est un résultat, pas un échec.

---

## Acceptance criteria

1. **AC1 — Rectification livrée.** Le corps de PR et le § 1 de ce plan énoncent les cinq divergences A1–A5 avec leur référence de fichier et de ligne, et établissent que `11250 = 300 × 3/4 × 50` est `reachable_output_tokens` à 300 s, non une mesure de tokens produits.
2. **AC2 — Aucune valeur de réglage ne bouge.** `MIKA_ARCH_CONFIG` est inchangé : `openrouter_model`, `llm_max_tokens`, `llm_http_timeout_secs`, `agent_total_timeout_secs` conservent leurs valeurs. `test_mika_arch_config_toml_is_valid_toml` et `mika2280_the_three_shipped_geometries_and_their_verdict` passent **sans modification**.
3. **AC3 — Un seul site de résolution.** `resolve_llm_budget_record` est l'unique constructeur du record ; `log_llm_budget_resolved` l'appelle. La ligne `llm_budget_resolved`, sa déduplication et le WARN `llm_budget_retry_unreachable` sont inchangés à champ constant.
4. **AC4 — Le record est celui de l'init.** `AgentState` porte le record résolu à `init_agent` ; muter le `config.toml` d'un agent servi ne change pas ce que la route rend, jusqu'au redémarrage.
5. **AC5 — La route atteste ou se tait.** `GET /api/v1/agents/{name}/budget` rend 200 + le record pour un agent servi, 404 pour tout autre. Elle ne relit pas le disque et ne calcule aucun repli.
6. **AC6 — Le CLI n'invente rien.** `mika agents budget` rend le record du serveur, avec la provenance de chaque champ. Serveur injoignable ou 404 ⇒ *« non attesté »* ; aucune valeur résolue localement n'est jamais affichée. Un test de contrôle négatif l'atteste.
7. **AC7 — L'Étape 2 n'est pas livrée, et son chemin est écrit.** Aucun mécanisme de repli modèle n'est introduit. Le § 3 énonce ses trois préconditions (baseline mika-arch absente, calibration du modèle de repli, mika#1190) et la PR ouvre le ticket de suivi correspondant.
8. **AC8 — La sonde est exécutable.** Le § 6 fournit la commande qui tranche A1–A4 et ses quatre haltes, chacune nommant une conduite distincte.

## Definition of Done

- [ ] `resolve_llm_budget_record` extrait, `log_llm_budget_resolved` délégué, champs et dédup inchangés
- [ ] `AgentState` porte le record ; `server/mod.rs:476` adapté
- [ ] `GET /api/v1/agents/{name}/budget` livrée avec ses deux cas (200 / 404)
- [ ] `mika agents budget [--agent] [--format json]` livrée, avec la branche *non attesté*
- [ ] Tests du § 5 verts, **contrôle négatif d'U4 vérifié rouge** avant d'être vert
- [ ] `cargo test`, `cargo clippy`, `cargo fmt` propres
- [ ] `crates/mika-agent/CLAUDE.md` : la route et le champ d'`AgentState` documentés à côté de la section *Observability + boot guard (mika#2293)*
- [ ] `crates/mika-common/CLAUDE.md` : `resolve_llm_budget_record` documenté sous *Budget provenance*
- [ ] `CLAUDE.md` racine : la commande ajoutée à la liste des commandes, et le § 6 résumé sous *Observabilité du budget effectif*
- [ ] Corps de PR portant AC1 (les cinq divergences) et AC7 (le refus motivé de l'Étape 2)
- [ ] Tickets de suivi ouverts : (a) réconciliation modèle mika-arch + baseline `calibrate-mika-arch`, (b) garde de dérive code↔runtime (suivi nommé par mika#2328), (c) taille du brief arch (suivi nommé par mika#2189)

## Hors périmètre, délibérément

- **La baisse de `llm_max_tokens`** — refusée au § 2, quatre raisons indépendantes.
- **Le repli automatique DeepSeek** — refusé au § 3 ; trois préconditions, aucune satisfaite.
- **La garde de dérive code↔runtime** — suivi explicitement nommé par mika#2328.
- **La réduction du prompt arch** — suivi explicitement nommé par mika#2189, conditionné à la mesure `request_bytes` de la Halte 3.
- **La cause fournisseur des coupures** — ce travail rend le réglage lisible ; il ne rend pas le modèle plus rapide.
- **Un second retry `_arch_ask`** — mika#2278 a livré le premier et écrit pourquoi le budget est de un.
