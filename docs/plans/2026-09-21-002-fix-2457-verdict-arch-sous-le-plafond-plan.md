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
- **DeepSeek n'a aucune calibration mika-arch. Ni kimi.** `docs/eval/calibration/` contient exactement six entrées — `2264`, `mika-dev-1221`, `mika-dev-1633`, `mika-orchestrator-1641`, `mika-qa-1632`, `mika-qa-2328` — et **rien pour mika-arch**, alors que la suite `crates/mika-agent/src/calibration/roles/mika_arch.rs` et ses fixtures existent. Plus net encore : le `CLAUDE.md` racine désigne `docs/eval/calibration/baselines/` comme le lieu des baselines (« Baselines live at `docs/eval/calibration/baselines/` ») et **ce répertoire n'existe pas dans l'arbre**. Il n'existe donc aucune baseline contre laquelle mesurer quoi que ce soit sur cet agent — ni pour k2.5, ni pour un modèle de repli. Le pilote n'a pas à le redécouvrir : les deux chemins sont nommés ici.
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
pub struct ResolvedBudgetRecord {
    /* les champs de l'événement, tels quels */
    pub resolved_at: String, // RFC 3339 UTC — l'instant de la résolution (F3)
}
pub fn resolve_llm_budget_record(agent_id, global_home, agent_home) -> ResolvedBudgetRecord;
pub fn log_llm_budget_resolved(...)  // appelle le premier, garde la dédup et le WARN mika#2362
```

Refactor **pur** : mêmes champs, même dédup, même `llm_budget_retry_unreachable`,
même ligne INFO. Un second résolveur serait un résolveur libre de diverger du
premier — la classe que `grooming_marker` (mika#2158) a dû fermer une fois.

**`log_llm_budget_resolved` a deux appelants, pas un**, et le refactor doit les
garder tous deux verts : `crates/mika-agent/src/server/mod.rs:476` (`init_agent`)
et `crates/mika-agent/src/teams/engine.rs:222` (run d'équipe). Seul le premier
prend le chemin U2 — un run d'équipe ne construit pas d'`AgentState` et continue
d'appeler la fonction de journal telle quelle. Le chemin per-skill
(`agent_loop`'s `make_provider_for`) reste hors périmètre et n'émet rien, comme
les commentaires des deux sites le déclarent déjà.

`resolved_at` est le **seul** champ ajouté, et c'est un fait sur le record, pas
un second avis sur le budget (F3) : il date la résolution, il ne la refait pas.
Il n'entre **pas** dans la signature de déduplication — deux résolutions
identiques à deux instants différents doivent rester une seule ligne, sans quoi
la dédup de mika#2293 serait annulée par le champ censé la documenter.

### U2 — `AgentState` garde ce que son `init_agent` a résolu

`crates/mika-agent/src/server/mod.rs:476` appelle déjà
`log_llm_budget_resolved(agent_name, global_home, agent_home)`. Remplacer par un
appel à `resolve_llm_budget_record`, stocker le record sur `AgentState`, puis
émettre. Le champ est un `Arc<ResolvedBudgetRecord>` posé à l'init et **jamais
recalculé** — même contrat *not hot-swappable* que `AgentState.tier` (mika#1962) et
`AgentState.deployment` (mika#2290), et pour la même raison : c'est l'état sous
lequel cet agent **tourne**, pas celui que le disque porte maintenant.

**Le gel est la propriété voulue, et il a un coût qu'il faut rendre lisible
plutôt que taire (F3).** Un record figé au boot répond à *« sous quoi cet agent
tourne-t-il »* et **jamais** à *« que porte le disque en ce moment »*. Les deux
questions se confondent tant que rien ne date la réponse : un opérateur lisant
`model = moonshotai/kimi-k2.5` ne peut pas distinguer « k3 est absent du disque »
de « le disque a changé depuis le boot ». D'où `resolved_at`, posé au même
instant que le record et rendu partout où le record l'est. Il ne lève pas
l'ambiguïté tout seul — il la rend **décidable** : la conduite de lecture est
écrite au § 6.

### U3 — Une route read-only

`GET /api/v1/agents/{name}/budget`, auth dashboard-ou-interne comme ses voisines
(`/api/v1/agents/{name}/sessions`, `/api/v1/agents/{name}/audit` existent déjà —
le motif est en place). Rend le record de l'`AgentState`, tel quel, `resolved_at`
compris. **404 si l'agent n'est pas résolu** — et surtout : ne recalcule rien, ne
relit pas le disque. Un agent non servi n'a pas de budget « en vigueur » à
rapporter, et en inventer un serait le faux vert que U1/U2 existent pour
empêcher. Le refus de relire le disque n'est pas une économie : c'est ce qui rend
la route et le disque **deux faits distincts**, dont la différence *est* la
mesure de dérive que #2457 attend (§ 6).

### U4 — `mika agents budget [--agent <name>]`

Nouvelle variante de `AgentsCommand` (`crates/mika-cli/src/cli.rs:389`, à côté de
`Reprovision`). Interroge `MIKA_SPIRIT_URL`, rend texte (défaut) ou `--format json`.
Sortie texte, une ligne par fait, chacune avec sa provenance :

```
mika-arch                        (résolu le 2026-09-21T06:12:44Z)
  provider   openrouter          (agent_config)
  model      moonshotai/kimi-k2.5 (agent_config, clé: openrouter_model)
  plafond    240 s               (agent_config)
  enveloppe  900 s               (agent_config)
  max_tokens 32768               (agent_config)
  atteignable 9000 tokens  @ 50 tok/s
  tentatives 3 nominales / 3 atteignables
```

La première ligne porte `resolved_at` (F3) : la sortie dit **quand** elle a été
vraie, jamais seulement ce qu'elle vaut.

**Si spirit ne répond pas, ou répond 404, le CLI ne calcule PAS de repli local.**
Il dit *« ce serveur n'a rien attesté »* — population que mika#2304 a dû nommer
pour exactement cette raison, et dont l'ambiguïté (binaire antérieur au correctif
vs agent non servi) est préférable à une valeur fausse.

**Le sixième mot de provenance existe déjà ; U4 ne l'invente pas (F2).**
`MODEL_SOURCE_UNKNOWN_PROVIDER = "unknown_provider"` est déclaré à
`crates/mika-common/src/llm/budget_provenance.rs:164`, produit par
`ModelProvenance::model_source_name` (`:346`) quand une porte de la cascade porte
un `llm_provider` que le lecteur ne sait pas parser, et **déjà épinglé** par deux
tests en place : `mika2293_source_names_are_a_wire_format` (`:1288`, format de
fil) et `mika2328_an_unreadable_provider_is_never_reported_as_a_default_model`
(`:1498`, qui asserte en plus que `Settings::load_for_agent` refuse ce fichier,
donc que l'état est inatteignable en production). Le doc-comment du site énonce
la raison : rapporter `default` affirmerait « aucune porte ne portait le
modèle », ce qui est inconnu et possiblement faux.

**Conséquence de périmètre, dite explicitement :** ce plan **n'introduit aucun
nouveau comportement de refus** dans `BudgetProvenance` / `ModelProvenance`. La
ligne U4 du § 5 est donc une **non-régression** — elle atteste que le passage par
le record et par la route ne perd pas cette provenance en chemin — et non une
garde nouvelle. Sur ce croisement, `effective_model()` rend `None` (`:334`) : la
sortie texte doit rendre `model  (non résolu — provider illisible : "<brut>")`
plutôt qu'une chaîne vide, et `provider_name()` (`:356`) fournit déjà le brut qui
a échoué à parser.

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
| U4 (non-régression) | Un `provider` illisible rend `unknown_provider` **à travers le record et la route**, pas `default` — comportement en place (`budget_provenance.rs:164` / `:346`), déjà épinglé par `mika2328_an_unreadable_provider_is_never_reported_as_a_default_model` (`:1498`) | une provenance perdue en traversant le record |
| U2/U3/U4 | Le record rend `resolved_at`, et le CLI l'affiche sur toute sortie attestée | une mesure non datable (F3) |
| U1 | `teams::engine` (`engine.rs:222`), second appelant, reste vert et continue d'émettre la même ligne | un refactor qui n'adapte que `server/mod.rs` |
| Non-régression | `test_mika_arch_config_toml_is_valid_toml` et `mika2280_the_three_shipped_geometries_and_their_verdict` **verts sans modification** | toute valeur touchée |

Le contrôle négatif d'U4 est celui qui porte : sans lui, « le CLI lit le serveur »
et « le CLI calcule localement » produisent exactement la même sortie sur un poste
où les deux processus partagent l'environnement — c'est-à-dire sur le poste de
développement où le test serait écrit.

---

## Fire-Disposition

Option retenue : **(a) named allowlist exception, allowlist livrée VIDE** — pour
le seul détecteur de ce plan qui ait une population pré-existante. Les autres
n'en ont aucune, et cette asymétrie est le cœur de la réponse, donc elle est
écrite détecteur par détecteur plutôt que globalement.

**Deux familles, et une seule a un sujet.**

| Détecteur | Population « données existantes » | Disposition |
|---|---|---|
| U1 équivalence (quatre positions de cascade) + contrôle négatif dédup/WARN | **aucune** — hermétique : le test construit ses propres `global_home`/`agent_home` en `tempdir`, sur le gabarit de `mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position` | N/A |
| U2 fraîcheur (muter le `config.toml` post-boot ne change pas le record) | **aucune** — hermétique : le test provisionne, initialise, mute, asserte | N/A |
| U3 200/404 | **aucune** — hermétique | N/A |
| U4 contrôle négatif *« non attesté »* | **aucune** — hermétique (serveur simulé injoignable / 404) | N/A |
| Non-régressions de valeur (`test_mika_arch_config_toml_is_valid_toml`, `mika2280_…`) | tests **déjà verts** à HEAD, non modifiés | N/A |
| **Scan structurel** : le record a **un** site de construction, et `mika-cli` ne résout **aucun** budget localement | **le code source présent** — sujet réel | **(a), allowlist vide** |

**Le scan structurel, et pourquoi il est nécessaire.** Le contrôle négatif d'U4
est comportemental : il atteste que *cette* sortie vient du serveur. Il ne peut
pas voir un second lecteur ajouté six mois plus tard sur un autre chemin du CLI —
la régression ne rendrait aucune décision fausse, elle rendrait la garantie
inopérante en silence, et toutes les assertions comportementales resteraient
vertes. C'est la classe que `mika1883_run_usage_accumulates_only_via_the_one_helper`
et `mika2205_periodic_scans_do_not_read_the_pat_field_directly` ont dû fermer par
un scan de source, pour exactement ce motif.

**Population pré-existante, mesurée à HEAD (`93931d40`) :**

- `grep -rn "BudgetProvenance\|budget_provenance\|effective_budget\|http_timeout_secs()" crates/mika-cli/` ⇒ **0 ligne**. Le CLI ne résout aucun budget aujourd'hui : l'allowlist du scan part vide et doit le rester.
- `log_llm_budget_resolved` a **deux** sites d'appel en production (`server/mod.rs:476`, `teams/engine.rs:222`) et **un** site de définition. Après U1, `resolve_llm_budget_record` devient l'unique constructeur ; le scan compte les constructeurs, pas les appelants.

**Zéro violation existante ⇒ aucune exception à nommer, donc aucun tracker de
suivi ni assertion auto-nettoyante à écrire** — les sous-conditions (1)(2)(3) de
l'option (a) portent sur les entrées de l'allowlist, et il n'y en a pas. C'est
le cas sain de l'option (a), pas son contournement : un plan qui aurait trouvé
des violations devrait les nommer une par une.

**Conduite quand le scan tire : on retire le second lecteur, on n'allowliste
pas.** Écrit dans le doc-comment du test, comme `mika2323_no_gate_predicate_reads_the_actor`
et le scan de mika#2201 l'écrivent déjà chacun pour le leur. Une allowlist qui
cesse d'être vide sur ce scan **est** le faux vert mika#2304 réintroduit : le
CLI afficherait une valeur locale avec l'autorité d'une attestation.

**Le cas concret soulevé par la revue, et sa réponse directe.** *« Au premier
boot après déploiement, si l'`AgentState` d'un agent déjà provisionné porte un
record qui diverge du `config.toml` sur disque, U2 passe — mais sur une donnée
déjà fausse. »* Deux moitiés :

1. **U2 ne lit jamais cet état.** Il est hermétique (tableau ci-dessus) : il
   construit son propre home, donc aucune donnée de production n'entre dans son
   verdict. Il n'y a pas de population à exempter.
2. **La divergence décrite n'est pas une violation à supprimer, c'est le
   livrable.** Ce qui lit l'état de production est la **route**, et une route
   n'est pas un détecteur : elle rapporte, et rapporter cette divergence est
   précisément ce que #2457 attend. Il n'y a donc rien à allowlister — il y a une
   **datation** à fournir pour que l'opérateur sache de quand date ce qu'il lit.
   C'est `resolved_at` (U1/U2/U3/U4) et la table de lecture du § 6.

---

## 6. Sonde post-déploiement, et ses quatre haltes

Après déploiement, sur le vrai serveur :

```bash
mika agents budget --agent mika-arch
```

Elle tranche A1–A4 en une commande — **à l'instant qu'elle affiche**, et cette
restriction n'est pas une précaution de style.

### Ce que la sonde mesure, et ce qu'elle ne mesure pas (F3)

Le record est figé à l'`init_agent` (U2, contrat *not hot-swappable* mika#1962) :
il rapporte **l'état sous lequel l'agent tourne**, daté par `resolved_at`. Il ne
rapporte pas l'état du disque à l'instant de la lecture. Les deux coïncident sauf
si quelqu'un a édité le `config.toml` depuis le boot — c'est-à-dire précisément
dans la branche « provisionnement gelé » que le § R4 juge la plus probable, où
une édition hors dépôt est le mécanisme même de la dérive soupçonnée.

**La lecture est donc en deux temps, et le second n'est facultatif que si les
dates le permettent :**

```bash
mika agents budget --agent mika-arch              # ce sous quoi l'agent TOURNE, daté
stat -c '%y  %n' ~/.mika/agents/mika-arch/config.toml   # ce que le disque PORTE, daté
```

| `resolved_at` vs mtime du `config.toml` | Ce qui est établi |
|---|---|
| `resolved_at` **postérieur** au mtime | Le record reflète le disque courant : la sonde tranche A1–A4 **sans réserve** |
| `resolved_at` **antérieur** au mtime | Le disque a bougé depuis le boot. Le record reste vrai de ce qui **tourne** — donc toujours décisif pour diagnostiquer un tour coupé — mais il ne dit **rien** de ce que le disque porte. Lire le fichier avant toute conclusion sur la dérive, et se rappeler que la valeur du disque **n'entrera en service qu'au prochain redémarrage** : c'est le contrat, pas un défaut |

Écrire `model = moonshotai/kimi-k2.5` sans cette datation laissait l'opérateur
incapable de distinguer « k3 est absent du disque » de « le record est
antérieur à l'arrivée de k3 ». Les deux dates rendent la distinction décidable
par construction, ce qui est ce qu'on demande à un livrable d'observation
(review-guide § Orthogonality : il rapporte l'état qu'il mesure, pas un état
postulé).

### Table de lecture

Chaque ligne se lit **sous la réserve de datation ci-dessus**.

| Observation | Lecture | Conduite |
|---|---|---|
| `model` ≠ `moonshotai/kimi-k2.5` | **une dérive hors dépôt est confirmée et mesurée** : à `resolved_at`, ce modèle-là était en service — donc il vit (ou vivait) sur disque, hors du dépôt | **Halte 1.** Noter la valeur, sa provenance **et `resolved_at`** avant de toucher au disque, puis lire le `config.toml` et son mtime pour savoir si le disque porte encore cette valeur. Toute édition de `MIKA_ARCH_CONFIG` démote le modèle : le préalable est un ticket de réconciliation modèle + calibration (mika#1190), pas une baisse de `max_tokens` |
| `model = moonshotai/kimi-k2.5` **et** mtime du `config.toml` postérieur à `resolved_at` | **Indéterminé sur la dérive présente**, décisif sur la dérive au boot | Ne **pas** conclure « k3 est absent ». Lire le `config.toml` : s'il porte k3, la dérive existe et n'est pas encore en service — c'est un redémarrage qui la mettra en vigueur, et c'est le moment de décider si on le veut |
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
3. **AC3 — Un seul site de résolution.** `resolve_llm_budget_record` est l'unique constructeur du record ; `log_llm_budget_resolved` l'appelle. La ligne `llm_budget_resolved`, sa déduplication et le WARN `llm_budget_retry_unreachable` sont inchangés à champ constant, et **les deux appelants** (`server/mod.rs:476`, `teams/engine.rs:222`) restent verts. `resolved_at` n'entre pas dans la signature de déduplication.
4. **AC4 — Le record est celui de l'init, et il est daté.** `AgentState` porte le record résolu à `init_agent` ; muter le `config.toml` d'un agent servi ne change pas ce que la route rend, jusqu'au redémarrage. Le record porte `resolved_at` (RFC 3339 UTC), rendu par la route et par le CLI sur toute sortie attestée.
5. **AC5 — La route atteste ou se tait.** `GET /api/v1/agents/{name}/budget` rend 200 + le record pour un agent servi, 404 pour tout autre. Elle ne relit pas le disque et ne calcule aucun repli.
6. **AC6 — Le CLI n'invente rien.** `mika agents budget` rend le record du serveur, avec la provenance de chaque champ. Serveur injoignable ou 404 ⇒ *« non attesté »* ; aucune valeur résolue localement n'est jamais affichée. Un contrôle négatif comportemental **et** un scan structurel (aucune résolution de budget dans `mika-cli`, allowlist vide) l'attestent.
6bis. **AC6bis — Aucun comportement de refus nouveau.** `BudgetProvenance` / `ModelProvenance` sont inchangés : `unknown_provider` (`budget_provenance.rs:164`/`:346`) est un comportement en place, et la ligne U4 correspondante du § 5 est une non-régression attestant qu'il survit au passage par le record et par la route.
7. **AC7 — L'Étape 2 n'est pas livrée, et son chemin est écrit.** Aucun mécanisme de repli modèle n'est introduit. Le § 3 énonce ses trois préconditions (baseline mika-arch absente, calibration du modèle de repli, mika#1190) et la PR ouvre le ticket de suivi correspondant.
8. **AC8 — La sonde est exécutable, et sa portée est écrite.** Le § 6 fournit la commande qui tranche A1–A4, ses quatre haltes nommant chacune une conduite distincte, **et** la règle de datation (`resolved_at` vs mtime du `config.toml`) qui dit quand la sonde tranche sans réserve et quand elle ne tranche que sur l'état au boot.
9. **AC9 — Fire-Disposition renseignée.** La section `## Fire-Disposition` nomme l'option retenue pour chaque détecteur livré : N/A pour les détecteurs hermétiques (population de données existantes vide par construction), **(a) named allowlist exception avec allowlist vide** pour le scan structurel, dont la population pré-existante est mesurée à zéro à HEAD et dont la conduite au déclenchement (retirer le second lecteur, ne pas allowlister) est écrite dans son doc-comment.

## Definition of Done

- [ ] `resolve_llm_budget_record` extrait, `log_llm_budget_resolved` délégué, champs et dédup inchangés ; `teams/engine.rs:222` vérifié vert
- [ ] `resolved_at` porté par le record, hors signature de déduplication
- [ ] `AgentState` porte le record ; `server/mod.rs:476` adapté
- [ ] `GET /api/v1/agents/{name}/budget` livrée avec ses deux cas (200 / 404)
- [ ] `mika agents budget [--agent] [--format json]` livrée, avec la branche *non attesté*, la ligne `resolved_at` et le rendu du croisement `unknown_provider`
- [ ] Scan structurel livré (constructeur unique du record + aucune résolution de budget dans `mika-cli`), **allowlist vide**, conduite au déclenchement écrite dans son doc-comment
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
- **Tout nouveau comportement de refus dans `BudgetProvenance` / `ModelProvenance`** — `unknown_provider` existe déjà et n'est pas retouché (AC6bis).
- **Une route ou une commande qui relirait le disque pour « rafraîchir » le record** — ce serait annuler U2. La fraîcheur se lit par datation (§ 6), jamais par recalcul.

## Revision history

- rev 2 (2026-09-21) : adressé **F1** en ajoutant une section `## Fire-Disposition` qui statue détecteur par détecteur — N/A pour les quatre détecteurs hermétiques (population de données existantes vide par construction, gabarit `mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`), option **(a) allowlist vide** pour le scan structurel dont la population pré-existante est **mesurée** à zéro à HEAD (`grep` sur `crates/mika-cli/` ⇒ 0 ligne), avec la conduite au déclenchement écrite (« on retire le second lecteur, on n'allowliste pas ») ; le cas concret soulevé par la revue y reçoit sa réponse en deux moitiés (U2 est hermétique et ne lit pas cet état ; la route n'est pas un détecteur, et la divergence qu'elle rapporte *est* le livrable). Adressé **F2** en ancrant `unknown_provider` à `crates/mika-common/src/llm/budget_provenance.rs:164` (constante), `:346` (`model_source_name`), `:1288` et `:1498` (tests en place) : le comportement **existe**, la ligne U4 du § 5 est requalifiée en **non-régression**, un AC6bis déclare qu'aucun refus nouveau n'est introduit, et le rendu du croisement (`effective_model()` ⇒ `None`, `:334`) est spécifié. Adressé **F3** en choisissant la branche (b) *et* (a) : le record porte `resolved_at` (hors signature de dédup), la route et le CLI le rendent, et le § 6 gagne une règle de datation explicite (`resolved_at` vs mtime du `config.toml`) plus une ligne de table pour le cas « record antérieur à l'édition du disque » — la sonde dit désormais de quand date ce qu'elle affirme au lieu de le postuler. Sharpening non bloquant intégré : `docs/eval/calibration/baselines/`, nommé par le `CLAUDE.md` racine comme le lieu des baselines, **n'existe pas dans l'arbre**. Corrigé au passage un manque trouvé en chemin : `log_llm_budget_resolved` a **deux** appelants (`server/mod.rs:476`, `teams/engine.rs:222`) et le plan n'en nommait qu'un.
