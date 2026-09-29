# mika#2474 — La taille du brief arch est mesurée par composant, et sa queue devient une population

> Ticket : `senara-solutions/mika#2474` (agent-core, `dispatch:loop`).
> Lignée : suivi nommé par la DoD de **mika#2457** (AC7, ligne 481 de son plan :
> « taille du brief arch (suivi nommé par mika#2189) »), lui-même conditionné à
> la **Halte 3** de ce plan. Famille de mika#2189 (géométrie 240/900),
> mika#2179 (bornes de livraison), mika#2296 (budget de sortie),
> mika#2295+#2330 (fenêtre d'historique), mika#2305 (portée de la fenêtre),
> mika#2363 (`--only-skill`), mika#2331 (`request_bytes` sur le bras d'erreur),
> mika#2280 (`reachable_output_tokens`), mika#2342 (filet sur l'appel LLM).

---

## 1. Ce que la lecture du code déplace dans le ticket

C'est le premier livrable. Le ticket demande de **mesurer** et de **borner** la
taille du brief arch. Les deux moitiés se heurtent à des faits que seule la
lecture du code établit, et chacun change le remède.

### R1 — La mesure existe, composant par composant, et elle est déjà ungated

Le brief d'un tour arch a **quatre** composants. Chacun porte déjà un champ, sur
un événement qui atterrit dans `$MIKA_SPIRIT_LOG_FILE` et qui **ne se tait pas**
quand `MIKA_STORE_LLM_CALLS` est désarmé :

| composant | champ | événement | livré par | borné par |
|---|---|---|---|---|
| prompt système | `system_prompt_bytes`, `per_skill_bytes` | `system_prompt_assembled` | mika#1217 | `--only-skill` (mika#2363) — une **sélection**, pas un plafond |
| historique | `history_bytes` | `context_window_assembled` | mika#2295 | `[context.history].max_tokens` (= 8000 pour arch) — **appliqué** |
| **le plan** | **`user_message_bytes`** | `context_window_assembled` | mika#2295 | **rien, et structurellement** |
| définitions d'outils | `tool_defs_bytes` | `context_window_assembled` | mika#2295 | rien |
| **total** | `request_bytes` | `turn_usage` | mika#2331 | rien |

`ContextWindowFields::user_message_bytes` porte le commentaire *« Bytes of the
last message: the user message this turn just saved »*
(`agent_loop/mod.rs:8623-8624`). Pour un tour arch, **ce dernier message EST le
plan** : `_arch_ask` fait `mika ask … - < "$plan_path"`
(`dispatch-lib.sh:6459`), donc le contenu du fichier est le message utilisateur
du tour. Il n'y a **rien à instrumenter** pour connaître la taille du brief arch.

**Conséquence sur la conduite :** ajouter un cinquième instrument qui redirait ces
mêmes nombres créerait deux sources de vérité pour une même valeur — exactement
ce que le doc-comment d'`emit_context_window_assembled` met en garde contre
(`:8820-8822` : *« two independent surfaces that must move together. If only one
moves, it is the measurement that is in question, not the system »*). Ce plan
n'en ajoute aucun.

### R2 — Le seul composant non borné est celui dont le plafond est structurellement faux

`truncate_history_to_token_budget` (`agent_loop/mod.rs:8764`) porte, ligne 8775 :

```rust
let history_end = history.len() - 1; // exclusive: the turn's user message.
```

La boucle de découpe s'arrête à `history_end` et n'atteint **jamais** le dernier
message. Le plan est donc exempt du plafond par construction, et cette exemption
est **correcte** : tronquer la question posée rend un verdict qui porte sur un
plan mutilé. Un tel verdict est **vert et faux**, ce qui est strictement pire
qu'un timeout — lequel est rouge et visible (`ESCALATE`, `PIPELINE FAILURE`).

C'est le même arbitrage que mika#2368 a dû écrire en refusant de poser `pass`
depuis un filet : *« un filet qui poserait `pass` au motif que le build a réussi
mergerait une PR dont aucun diff n'a été revu, ce qui est strictement pire que le
silence qu'il remplace »*.

### R3 — Le stderr du site qui tient le plan n'atterrit dans AUCUN fichier

Le réflexe serait d'annoncer la taille depuis le shell, dans `_arch_ask`, qui
tient `$plan_path`. **Ce signal serait mort-né**, et la lecture le prouve :

1. `_iterate_groom_loop` est appelé depuis `dispatch_claude_pilot`
   (`dispatch-lib.sh:9359`), **après** `_run_claude_pilot`, donc **hors** de la
   redirection `2>"$STDERR_FILE"` du lancement du pilote (`:3698`).
2. Son stderr est le handle `Stdio::piped()` de `spawn_long_running_exec`
   (`executor.rs:4413`), que l'exécuteur ne lit que dans la branche
   `if !status.success()` (`:4578`).
3. Un dispatch de groom sort **toujours en 0** — `_deliver_callback` + `exit 0`
   est le chemin canonique, convergence **comme** `PIPELINE FAILURE`. Le tuyau
   est donc lâché sans être lu et les lignes n'atterrissent nulle part.

C'est la classe mika#2050, mesurée deux fois sur ce même fichier (Signal M pour
`pilot_push_guard`, Signal Q pour le prologue de secrets). **Publier un signal
là, en annonçant « régime attendu : une ligne par appel », donnerait un
instrument dont le silence ne prouve rien** — la panne que mika#2205 nomme : *une
garde qu'on n'a pas déployée se lit exactement comme une flotte saine.*

Le corollaire est ce qui fixe la forme du livrable : **la mesure doit être
côté serveur**, où elle est déjà, et où son sink est établi.

### R4 — La géométrie que le ticket suppose n'est pas celle du dépôt

`MIKA_ARCH_CONFIG` (`well_known_agents.rs:1551-1597`) déclare
`openrouter_model = "moonshotai/kimi-k2.5"`, `llm_max_tokens = 32768`,
`llm_http_timeout_secs = 240`, `agent_total_timeout_secs = 900`. Le corps de
mika#2457 affirme un runtime en **kimi-k3**, `llm_max_tokens = 16384` et un
plafond de **300 s**. Les deux ne peuvent pas être vrais ensemble : c'est la
dérive code↔runtime que mika#2473 a livré D1/D2 pour **mesurer**.

**Aucune ligne de ce plan ne touche ces quatre valeurs**, et aucune conclusion de
ce plan ne repose sur laquelle des deux géométries tourne. C'est délibéré : le
préalable écrit par mika#2457 (§ 2, quatre raisons indépendantes) est un ticket
de réconciliation modèle + calibration (mika#1190), pas un réglage pris en
passant.

### R5 — La prémisse « gros brief ⇒ verdict sous le plafond » est partiellement réfutable, et le plan le dit

Le mécanisme que le ticket postule est : *gros brief → le modèle est poussé vers
le plafond de tokens*. Or `llm_max_tokens` est un budget de **sortie**
(`MIKA_ARCH_CONFIG:1557-1558` : *« A reasoning model counts its thinking in the
OUTPUT budget »*), et `reachable_output_tokens = plafond × 3/4 × plancher`
(mika#2280) ne dépend pas de la taille de l'entrée.

Le couplage réel est indirect et il a deux branches : (a) le temps jusqu'au
premier octet croît avec l'entrée et mange le plafond **temporel** ; (b) **un
plan plus gros demande légitimement un verdict plus long** — plus d'ACs à
énumérer, plus de findings à ancrer, et les contrats de sortie de
`mika-arch-groom-ticket` (ligne `Disposition:`, liste `F1:`…`F10:`) sont
proportionnels au plan.

(b) est ce qui rend « borner le brief » ambigu : **réduire l'entrée réduit la
sortie en réduisant la revue**. C'est la même famille que R2 vue depuis l'autre
bout. Ce plan ne tranche pas (a) contre (b) — il livre la mesure qui le permet,
et nomme ci-dessous ce que chaque issue conditionne.

---

## 2. La distribution, mesurée

Sur les **198** plans `docs/plans/2026-09-*-plan.md` de l'arbre à HEAD
`b6c95955` (la population que l'architecte a réellement lue ce mois-ci) :

| statistique | octets | ≈ tokens (÷ 4) |
|---|---|---|
| min | 1 260 | 315 |
| p25 | 21 807 | 5 450 |
| **p50** | **29 414** | **7 350** |
| p75 | 37 382 | 9 350 |
| **p90** | **48 378** | **12 100** |
| p95 | 54 340 | 13 600 |
| p99 | 72 349 | 18 100 |
| max | 75 871 | 19 000 |
| moyenne | 30 211 | 7 550 |

Sur les **971** plans de tout l'arbre, le maximum est **89 167 B**
(`2026-04-25-001-feat-kg-multi-corpus-per-agent-plan.md`).

Pour situer : les trois prompts arch pèsent 16 562 B (`groom-ticket`),
14 583 B (`second-review`) et 9 239 B (`groom-milestone`), soit 40 384 B pour les
trois — cohérent avec les 39 798 B que mika#2363 a mesurés avant d'en évincer
23,5 Ko. **Depuis mika#2363, un plan médian (29 Ko) pèse deux fois le prompt de
la passe (16,5 Ko) : le prompt système n'est plus le levier, le plan l'est.**

**Ce que cette mesure ne dit pas, et c'est important :** elle porte sur les
plans **écrits**, pas sur les briefs **échoués**. La corrélation que la Halte 3
de mika#2457 demande — `request_bytes` des tours en échec contre celui des tours
sains — vit dans `$MIKA_SPIRIT_LOG_FILE` et dans `~/.mika/data/mika.db`, dont le
bac à sable de dispatch ne monte ni l'un ni l'autre. C'est un geste d'opérateur
sur l'hôte, pas quelque chose que l'implémenteur peut fournir. Le § 7 la rend
exécutable en une ligne.

---

## 3. Les quatre transformations demandées, et pourquoi chacune est refusée

Le ticket propose « la borner (ou la découper/résumer au-delà d'un seuil) ». Les
quatre variantes concevables sont refusées, chacune sur son motif propre. Ce
refus **est** un livrable : sans lui, la prochaine lecture du ticket
re-proposerait la première variante.

| # | transformation | motif du refus |
|---|---|---|
| T1 | **tronquer** le plan au-delà d'un seuil | le verdict porte alors sur un plan mutilé — vert et faux (R2). Et le site qui tronquerait est celui qui exempte déjà le dernier message **par construction** : le changement consisterait à retirer l'exemption |
| T2 | **résumer** le plan par un appel LLM | l'architecte reviendrait un **résumé**, pas le plan. Même classe que T1, une couche plus loin — et le résumé est lui-même un brief, donc le coût n'est pas supprimé mais déplacé |
| T3 | **découper** le plan en sections revues séparément | les findings de `docs/architecture/review-guide.md` (SOLID / DRY / Orthogonalité) sont **transverses par nature** : une revue par section perd très exactement ce que la revue existe pour trouver |
| T4 | **ne pas ré-envoyer** le plan en seconde passe, en s'appuyant sur l'historique | l'historique est **tronquable** (`[context.history].max_tokens` = 8000 ⇒ budget ~32 Ko), et la troncature élide des messages entiers par l'avant. Un plan de 72 Ko serait **entièrement élidé** de la fenêtre : la seconde passe reviendrait *rien*. Le ré-envoi est le choix défensif correct, pas une redondance à supprimer |

**T4 mérite son détail, parce que la redondance est réelle et qu'elle est
petite.** `_arch_ask_with_retry "mika-arch-second-review" "$plan_path"
"$session_id"` (`dispatch-lib.sh:7603`, `:7652`) continue la session **et**
ré-envoie le plan. Pour un plan de 15 Ko : l'historique porte [plan 15 Ko +
réponse ~10 Ko] = 25 Ko < 32 Ko de budget, donc rien n'est tronqué, et le plan
voyage **deux fois** (≈ 40 Ko là où 25 suffiraient). Pour un plan de 72 Ko :
l'historique dépasse le budget, le message du plan est élidé en entier, et le
plan ne voyage qu'une fois. **La redondance ne touche donc que les petits plans,
c'est-à-dire pas la population du problème** — et la supprimer casserait la
seconde passe des gros, qui sont la population du problème. Refus net.

---

## 4. Ce qui est livré

Une seule chose neuve, et c'est un **seuil**, pas un plafond.

### D1 — La queue de la distribution devient une population greppable et comptable

`brief_size_overrun` — un événement **INFO**, émis au site unique
d'`emit_context_window_assembled` (`agent_loop/mod.rs:5271`), **uniquement** quand
`user_message_bytes` dépasse le seuil. Champs :

```
agent_id, session_id, trace_id, mode,
user_message_bytes, history_bytes, system_prompt_bytes, tool_defs_bytes,
threshold_bytes, distinct_sessions, truncated_messages
```

**Pourquoi un événement distinct et non un champ sur `context_window_assembled`.**
Un booléen sur l'événement existant n'achèterait rien : il faudrait toujours
`jq 'select(.oversized)'` sur **tous** les tours, ce qu'on peut déjà faire avec
`select(.user_message_bytes > 48000)`. Ce qu'un nom distinct achète est le
**grep** — sur 19 Go, la différence entre une question et une analyse. C'est
exactement le raisonnement de mika#2293 pour `llm_budget_resolved` (*« il doit
grepper seul »*) et de mika#2131 (agrégat par tick au journal, détail par
anomalie ailleurs).

**Pourquoi INFO et non WARN.** Le régime attendu est **non vide** : le seuil est
posé sur le p90 d'une distribution **saine**, donc le décile supérieur tire par
construction. Un WARN sur le décile supérieur nominal est précisément ce qui
finit museler. L'anomalie n'est pas la ligne : c'est la **corrélation** entre
cette ligne et un `turn_usage` en `status = "error"`, établie par le join du § 7.
`pilot_cost_overrun` (mika#2496) est WARN parce que son seuil est une **règle**
posée (40 USD) dont le franchissement est une faute ; ici le seuil est un
**percentile mesuré**, dont le franchissement est nominal pour un plan sur dix.

**Le champ `system_prompt_bytes` est repris, pas recalculé.** Le prompt assemblé
est déjà en portée à ce site — `emit_system_prompt_assembled(&system, …)` le
consomme **par référence** 149 lignes plus haut (`:5122`), donc il n'est pas
déplacé et `system.len()` est disponible ligne 5271 sans rien élargir. Le porter
sur la ligne rend celle-ci
**auto-suffisante** pour l'attribution : sans lui, lire « le brief était gros »
demande un second grep pour savoir *de quoi* il était gros. Ce n'est pas une
seconde source de vérité — c'est la même valeur, à la même instant, sur la ligne
qui en a besoin. La règle de R1 (ne pas redire ces nombres **ailleurs qu'au
franchissement**) tient : `context_window_assembled` reste l'unique surface
par-tour.

### D2 — Une ligne `audit_events` par franchissement

`tool_name = 'brief_size_overrun'`, `target_key = "agent:<agent_id>"`,
`after_value` = `user_message_bytes` **et rien d'autre** (c'est ce qu'on moyenne
— motif `pilot_cost_overrun`), `reasoning` = `trace_id=… history=… system=…
tools=… threshold=…` en texte libre.

**Pas de déduplication, à dessein.** Chaque franchissement est un fait daté
distinct qu'on veut **compter** — motif `ready_label_outcome` (mika#2323) : la
population est de quelques unités par jour, pas une centaine par tick, donc la
doctrine mika#2131 ne s'applique pas. Le `GROUP BY target_key` répond
directement à « quel agent envoie de gros briefs ».

**Fire-and-forget**, comme ses cinq voisins : un échec d'écriture d'audit
`warn!` et rend la main. Mesurer une taille ne doit pas pouvoir casser un tour.

### D3 — Le seuil, et ses trois paliers

`brief_size_alert_bytes` (`config.toml`) / `MIKA_BRIEF_SIZE_ALERT_BYTES`, défaut
`DEFAULT_BRIEF_SIZE_ALERT_BYTES = 48_000`. Accesseur
`Settings::effective_brief_size_alert_bytes()`, exactement le motif de
`effective_pilot_cost_alert_usd` (`config.rs:2090`).

- absent / vide → défaut ;
- illisible → erreur `Settings::load` (comme tout frère numérique) ;
- `0` ou négatif → défaut **avec un WARN nommant la valeur** : un seuil d'alerte
  à zéro rapporterait *chaque* tour et noierait la population que la mesure
  existe pour dimensionner (le raisonnement que `pilot_cost_alert_usd` a déjà
  écrit).

**48 000 est un percentile mesuré, pas une rondeur** : c'est le p90 des 198 plans
de septembre, donc la population est le décile supérieur — ~2 lignes par jour à
la cadence actuelle. p95 (54 340) halverait une population déjà petite et
rendrait la corrélation du § 7 plus difficile à établir ; p75 (37 382) ferait
entrer le quart supérieur, ce qui n'est plus une queue. **Le préfixe `MIKA_` est
correct ici** — la clé est lue par **mika-spirit**, par la cascade config-rs,
dont rien ne nettoie l'environnement du process (à l'inverse de `PILOT_MAX_TURNS`,
nu parce que l'enfant de dispatch efface son env, mika#2508).

**Pas de seuil par agent, et c'est nommé.** `pilot_cost_alert_usd` est global et
ce seuil l'est aussi. La population de fait est celle d'arch (§ 5), et un
réglage par agent serait un `config.toml` de plus à réconcilier pour un besoin
que personne n'a mesuré.

### D4 — Correction trouvée en chemin : un doc-comment périmé

`ContextWindowFields::truncated_messages` / `truncated_bytes`
(`agent_loop/mod.rs:8652-8657`) portent : *« Always `0` today: the ceiling is
brique 2 of the mika#2295 plan and is deliberately gated on the verdict this very
event produces. »* **C'est faux depuis que la brique 2 a atterri** :
`truncate_history_to_token_budget` est appelée ligne 5257 dès que
`resolved_history.max_tokens` est `Some`, et mika-arch déclare
`max_tokens = 8000`. Le commentaire d'en face le dit déjà (`:5265` : *« and,
since the two bounds above landed, how much they took back out »*) — les deux se
contredisent dans le même fichier.

Corrigé en deux lignes de doc. Ça compte pour ce ticket : un opérateur qui lit
« toujours 0 » ne lira pas `truncated_messages`, et c'est **le** champ qui dit si
la fenêtre de la seconde passe a élidé le plan (§ 7, sonde S3).

---

## 5. Périmètre effectif : pourquoi le site agnostique est le bon

`emit_context_window_assembled` est appelé à **un seul site de production**
(`:5271`, dans le chemin conversation ; `rewind.rs` est administratif, pas un
tour). Le seuil y vit donc aussi, avec **un seul lecteur**.

Ce site est **agnostique de l'agent**, et c'est la propriété qui rend
l'instrument complet plutôt qu'arch-spécifique : il couvre **toutes** les portes
d'entrée d'un tour arch — l'A2A de `_arch_ask`, un `mika ask` à la main, le flux
opérateur `/mika-groom-ticket` — sans qu'aucune ne doive être énumérée. Un seuil
posé dans `_arch_ask` n'aurait couvert que la première (et son signal serait mort,
R3).

**La population de fait est celle d'arch, et c'est vérifiable.** Le message
utilisateur des autres tours est petit par construction :

| tour | ce que porte le message utilisateur | ordre de grandeur |
|---|---|---|
| conversation Telegram | le texte de l'utilisateur | < 4 Ko (limite Telegram) |
| webhook GitHub | le texte formaté par le gateway | corps de PR tronqué à 2 000 c. |
| callback | le `RESULT` du handler | `CALLBACK_RESULT_MAX_BYTES` = 10 240 |
| revue QA | **le diff ne passe pas par là** — `[context.gh_pr_diff]` l'injecte dans le prompt de skill, donc il compte dans `system_prompt_bytes` | — |
| **passe arch** | **le plan, `mika ask … - < "$plan_path"`** | **p50 29 Ko, p99 72 Ko** |

Les tours silencieux (callback, heartbeat) ne passent pas par ce site du tout.
**Corollaire à surveiller, et c'est la halte 3 du § 7 :** si la population porte
un agent autre qu'arch, le seuil mesure autre chose que ce qu'on croit.

---

## 6. Fire-Disposition

Ce plan livre **deux détecteurs** au sens de la règle mika#2306 — deux scans de
source dont le chemin de succès est « aucune violation trouvée ». Disposition
retenue pour les deux : **(a) exception nommée en allowlist, allowlist livrée
vide**, avec la conduite au déclenchement écrite dans le doc-comment.

`brief_size_overrun` et `brief_size_alert_bytes` **ne sont pas** des détecteurs :
leur régime attendu est non vide (§ 4 D1), donc leur chemin de succès n'est pas
« aucune violation ». Gate N/A pour eux, et l'inventer serait faux.

### Détecteur 1 — `mika2474_the_overrun_name_has_a_single_writer`

Scan de source refusant un second écrivain de `brief_size_overrun`, dans le
journal **et** dans `audit_events` — motif
`mika2496_the_cost_overrun_name_has_a_single_writer`.

- **Population pré-existante : zéro, mesurée.** Le nom est neuf ;
  `grep -rn brief_size_overrun crates/` rend 0 ligne à HEAD `b6c95955`.
- **Allowlist livrée vide**, et un test frère la fige vide (une allowlist née
  vide est un tiroir où déposer le prochain manquement — mika#2323).
- **Assertion auto-nettoyante :** le scan **échoue si le nom n'est écrit nulle
  part**. Un scan qui vise un nom mort ne vérifie rien et se lit exactement comme
  un arbre propre (classe mika#2103 / mika#2205).
- **Conduite au déclenchement : on retire le second site, on n'allowliste pas**
  (doctrine mika#2201). C'est cette propriété qui rend exact le `GROUP BY` du
  § 7, et non un nombre sur lequel deux sites peuvent divergier.
- **Aucun test comportemental ne peut voir cette classe** : un second écrivain ne
  rend aucune décision fausse le jour où il est écrit ; il rend deux populations
  inséparables, plus tard, en silence.

### Détecteur 2 — `mika2474_the_threshold_has_a_single_reader`

Scan de source refusant un second lecteur de `effective_brief_size_alert_bytes`
en code de production.

- **La classe est réelle et nommée :** un second lecteur serait, en pratique, le
  **refus** que ce plan décline de livrer sans sa précondition (§ 8) — une garde
  dans `validate_dispatch_readiness` ou dans `_arch_ask`. Il couperait un groom
  sur un seuil calibré pour une **alerte**, alors que l'asymétrie est écrite :
  *un faux positif d'alerte coûte une ligne de journal ; un faux positif de
  refus coûte une passe d'architecte et un point du budget de re-drive*
  (mika#2020 : trois abandonnent un ticket sain).
- **Population pré-existante : zéro** (l'accesseur est neuf).
- **Allowlist livrée vide**, même conduite, même contrôle de bonne foi
  (le scan doit rougir si on ajoute un second lecteur — test frère).

---

## 7. Surfaces opérateur, sondes et haltes

### SQL

```sql
-- La population, par agent. `brief_size_overrun` est SOLE WRITER, donc ce compte
-- est exact plutôt qu'un nombre sur lequel deux sites peuvent diverger.
SELECT target_key,
       count(*)                                  AS n,
       round(avg(CAST(after_value AS REAL)))     AS moy_octets,
       max(CAST(after_value AS INTEGER))         AS max_octets
  FROM audit_events
 WHERE tool_name = 'brief_size_overrun'
 GROUP BY 1 ORDER BY 2 DESC;
```

### Journal (`$MIKA_SPIRIT_LOG_FILE`)

```bash
# 1. Quels briefs ont franchi le seuil, et de quoi étaient-ils faits ?
grep brief_size_overrun "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{agent_id, trace_id, user_message_bytes, history_bytes,
            system_prompt_bytes, tool_defs_bytes, threshold_bytes}'

# 2. CONTRÔLE POSITIF — le site tourne-t-il seulement ?
grep -c context_window_assembled "$MIKA_SPIRIT_LOG_FILE"

# 3. LA CORRÉLATION — la Halte 3 de mika#2457, rendue exécutable.
#    Les tours arch en échec, avec la taille du brief qui a tenu.
grep turn_usage "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.agent_id == "mika-arch" and .status == "error")
           | {trace_id, latency_ms, request_bytes, system_prompt_bytes}'
#    …puis joindre sur trace_id avec la commande 1. La population de la
#    commande 1 étant petite (~2/j), le join est une lecture, pas une analyse.

# 4. La fenêtre de la seconde passe a-t-elle élidé le plan ? (champ D4)
grep context_window_assembled "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.agent_id == "mika-arch" and .truncated_messages > 0)
           | {trace_id, history_bytes, truncated_messages, truncated_bytes}'
```

| surface | niveau | régime attendu | lecture |
|---|---|---|---|
| `brief_size_overrun` | INFO | **non vide, ~2/jour** | le décile supérieur, par construction. Ce n'est **pas** une anomalie |
| `brief_size_overrun` sur un agent ≠ `mika-arch` | INFO | **vide** | le seuil mesure autre chose que ce qu'on croit — halte 3 |
| `brief_size_alert_invalid` | WARN | **vide** | une coquille dans la variable, nommée entre guillemets |
| `brief_size_overrun_audit_failed` | WARN | **vide** | la ligne INFO est passée, l'audit non — le `GROUP BY` est alors incomplet |

### Sondes, et leurs quatre haltes

> **Préalable.** La valeur est lue par **mika-spirit**, donc la sonde décrit le
> binaire servi. Établir le déploiement avant toute conclusion.

**S1 — le seuil mord (première passe arch sur un plan du décile supérieur).**
Une ligne `brief_size_overrun` portant `agent_id = "mika-arch"` et un
`user_message_bytes` cohérent avec `wc -c` du plan.
**Halte 1 — aucune ligne alors qu'un gros plan est passé.** Ne pas baisser le
seuil par réflexe. Lire d'abord le **contrôle positif** (commande 2) : zéro
`context_window_assembled` signifie que le site ne tourne pas — ou que le binaire
servi est antérieur au correctif. *Zéro franchissement avec zéro
`context_window_assembled` ne prouve rien du tout* (classe mika#2205).

**S2 — la corrélation, 30 jours.** C'est **la** sonde du ticket, et son résultat
décide du suivi. Croiser les commandes 1 et 3 :
- les tours en échec sont **majoritairement** dans la population de la
  commande 1 ⇒ la Halte 3 de mika#2457 est **confirmée**, le levier est la taille
  du brief, et le suivi § 8 (a) s'ouvre **avec un compte** ;
- les tours en échec sont **répartis** indépendamment de la taille ⇒ la Halte 3
  est **réfutée**, et c'est un **résultat** : la cause est ailleurs (le modèle, le
  transport — voisinage mika#2522/#2342), la piste devient l'Étape 2 de
  mika#2457, et ce ticket se referme sur sa mesure.

**Halte 2 — la population de la commande 1 est vide sur 30 jours alors que des
grooms ont tourné.** Deux causes opposées, à séparer avant de conclure : les
plans de la fenêtre étaient tous sous 48 Ko (résultat honnête — la distribution
du § 2 a bougé, et c'est à noter), ou le seuil n'est pas lu (vérifier
`llm_budget_resolved` pour établir qu'un tour arch a bien eu lieu).

**S3 — l'élision de la seconde passe (30 jours).** La commande 4 dit si la
fenêtre de la seconde passe a élidé le plan de la première. Régime attendu :
**non vide sur les gros plans** — c'est le comportement correct décrit en T4, pas
un défaut. **Halte — si elle est non vide sur des plans de 15 Ko**, le budget
d'historique résolu n'est pas celui qu'on croit : lire `context_history_resolved`
(mika#2425) **avant** de toucher au plafond.

**Halte 3 — la population porte un agent autre que `mika-arch`.** Le tableau du
§ 5 est faux quelque part : établir **quel** tour porte un message utilisateur de
cette taille avant de régler le seuil. Un seuil calibré sur la distribution des
plans et mesurant autre chose est un instrument qui mentira sur les deux
populations.

**Halte 4 — `brief_size_overrun_audit_failed` non vide.** Le `GROUP BY` SQL est
incomplet et sous-compte. Réparer l'écriture ; ne pas lire le compte comme une
mesure entre-temps.

---

## 8. Ce que ce travail n'achète PAS

- **Il ne fait tenir aucun verdict sous le plafond.** Il ne coupe rien, ne
  résume rien, ne découpe rien (§ 3). Un brief de 72 Ko part exactement comme
  avant ; ce qui change est qu'il est **nommé** et **compté**.
- **Il ne réduit aucune taille.** Ni le plan, ni le prompt système, ni la
  fenêtre, ni les définitions d'outils.
- **Il ne tranche pas (a) contre (b) du § R5.** Il livre la mesure qui le
  permettra.
- **Il ne rétro-remplit rien.** Les briefs déjà envoyés n'auront jamais leur
  ligne : fabriquer une ligne d'audit datée d'un franchissement qu'on n'a pas
  observé est l'inverse de ce que ce travail défend. La sonde est le **prochain**
  gros brief.
- **Il ne rend pas le brief surveillé, il le rend lisible.** Le seul instrument
  neuf est un seuil, et **son silence ne prouve rien tant que personne n'exécute
  les sondes du § 7** — sur une population de deux lignes par jour, l'absence
  d'occurrence peut simplement vouloir dire qu'aucun gros plan n'a été groomé
  cette semaine.

---

## 9. Hors périmètre, délibérément

- **Les quatre transformations T1–T4** (§ 3), chacune avec son motif.
- **Un refus a priori au-delà d'une taille** — c'est la forme la plus utile que
  le ticket puisse prendre, et elle est **conditionnée à sa propre précondition** :
  refuser une passe au-dessus d'une taille suppose de connaître cette taille,
  c'est-à-dire la mesure que ce ticket livre. Un refus ne peut pas atterrir dans
  la même PR que sa précondition. **Suivi nommé**, précondition écrite : la sonde
  S2 sur 30 jours établissant que les tours en échec sont majoritairement dans la
  population du seuil.
- **La géométrie de mika-arch** (`llm_max_tokens`, plafond, enveloppe, modèle) —
  refusée au § 2 de mika#2457 par quatre raisons indépendantes, et R4 ci-dessus
  montre qu'on ne sait même pas laquelle des deux géométries tourne.
  `test_mika_arch_config_toml_is_valid_toml` et
  `mika2280_the_three_shipped_geometries_and_their_verdict` passent **sans
  modification**.
- **Le repli automatique DeepSeek** — Étape 2 de mika#2457, trois préconditions
  écrites, aucune satisfaite.
- **La réduction du prompt arch** — mika#2363 en a évincé 23,5 Ko ; le reste
  (16,5 Ko pour la passe servie) est deux fois plus petit qu'un plan médian, donc
  ce n'est plus le levier. Le rouvrir demanderait une mesure montrant l'inverse.
- **La réduction de `tool_defs_bytes`** — une réduction **non destructive** existe
  peut-être là (le même raisonnement que `--only-skill`, appliqué aux outils),
  mais restreindre la surface d'outils change **ce que l'architecte peut faire** :
  c'est une décision de capacité, pas de taille. **Suivi**, précondition : que
  `tool_defs_bytes` soit mesuré non négligeable devant `user_message_bytes` sur
  la population du § 7 — le champ existe déjà, la mesure est une ligne de `jq`.
- **Un signal côté shell dans `_arch_ask`** — refusé par R3 : son sink n'existe
  pas. Le rendre possible demanderait de changer ce que
  `spawn_long_running_exec` lit sur un dispatch qui **réussit**, ce qui est le
  périmètre du parent mika#2532 et pas celui d'un ticket d'observabilité.
- **La cause fournisseur des coupures** — voisinage mika#2522 / #2342 ; ce
  travail rend la charge lisible, il ne rend pas le modèle plus rapide.
- **Le plafonnement du message utilisateur pour les autres agents** — la
  population est arch (§ 5) ; poser une garde sur une population vide produit un
  détecteur dont le silence ne prouve rien.

---

## Acceptance criteria

1. **AC1 — Rectification livrée.** Le § 1 établit par référence de fichier et de
   ligne : (R1) que les quatre composants du brief sont déjà mesurés, avec leur
   champ et leur événement ; (R2) que le seul composant non borné l'est
   structurellement (`agent_loop/mod.rs:8775`) et que l'exemption est correcte ;
   (R3) que le stderr de `_iterate_groom_loop` n'atterrit dans aucun fichier sur
   un dispatch qui sort en 0 (`executor.rs:4578`) ; (R4) que la géométrie arch du
   dépôt diverge de celle que le ticket suppose ; (R5) que la prémisse
   « gros brief ⇒ plafond de sortie » est indirecte et partiellement réfutable.
2. **AC2 — La distribution est mesurée et publiée.** Le § 2 donne min / p25 /
   p50 / p75 / p90 / p95 / p99 / max / moyenne sur les 198 plans de
   septembre 2026, avec la commande qui les produit, et nomme explicitement ce
   que cette mesure **ne** dit pas (elle porte sur les plans écrits, pas sur les
   briefs échoués).
3. **AC3 — Les quatre transformations sont refusées avec leur motif.** Le § 3
   nomme T1–T4 et, pour chacune, la raison propre du refus. T4 porte son
   arithmétique (la redondance ne touche que les petits plans).
4. **AC4 — Un seuil, et il ne coupe rien.** `brief_size_overrun` est émis au site
   unique d'`emit_context_window_assembled`, **seulement** au franchissement, en
   INFO, avec les quatre composants sur la ligne. Aucun tour n'est refusé, aucun
   texte n'est tronqué, aucune valeur de géométrie ne bouge. Un contrôle négatif
   atteste qu'un brief **sous** le seuil n'émet rien.
5. **AC5 — Le seuil est configurable et ses trois paliers sont tenus.**
   `brief_size_alert_bytes` / `MIKA_BRIEF_SIZE_ALERT_BYTES`, défaut 48 000 (p90
   mesuré) ; absent ou vide → défaut ; `0` ou négatif → défaut **plus** un WARN
   nommant la valeur. Le défaut est justifié par le percentile, non par une
   rondeur.
6. **AC6 — La population est comptable en SQL.** Une ligne `audit_events` par
   franchissement, `tool_name = 'brief_size_overrun'`,
   `target_key = "agent:<id>"`, `after_value` = les octets et rien d'autre.
   Écriture fire-and-forget ; `brief_size_overrun_audit_failed` marque une ligne
   INFO passée dont l'audit a échoué.
7. **AC7 — SOLE WRITER, et le scan le tient.**
   `mika2474_the_overrun_name_has_a_single_writer` refuse un second écrivain du
   nom (journal et audit), allowlist **livrée vide**, figée vide par un test
   frère, avec son assertion auto-nettoyante (il rougit si le nom est écrit
   nulle part) et la conduite au déclenchement écrite dans son doc-comment.
8. **AC8 — Le seuil a un lecteur unique.**
   `mika2474_the_threshold_has_a_single_reader` refuse un second lecteur de
   `effective_brief_size_alert_bytes` en production, allowlist **livrée vide**,
   avec son contrôle de bonne foi. Le doc-comment nomme la classe : un second
   lecteur serait le refus que le § 9 décline de livrer sans sa précondition.
9. **AC9 — La corrélation de mika#2457 Halte 3 est exécutable.** Le § 7 fournit
   les quatre commandes, le contrôle positif, et les quatre haltes nommant
   chacune une conduite distincte — dont celle qui dit que la réfutation de la
   Halte 3 est un **résultat**, pas un échec.
10. **AC10 — Le doc-comment périmé est corrigé.**
    `ContextWindowFields::truncated_messages` / `truncated_bytes` ne prétendent
    plus valoir « toujours 0 » : la brique 2 de mika#2295 est appliquée
    (`agent_loop/mod.rs:5256-5259`) et le commentaire voisin le disait déjà.
11. **AC11 — Fire-Disposition renseignée.** Le § 6 statue détecteur par
    détecteur : **(a) allowlist nommée, livrée vide** pour les deux scans, avec
    leur population pré-existante **mesurée à zéro** et la conduite au
    déclenchement ; **N/A** pour `brief_size_overrun` et son seuil, dont le
    régime attendu est non vide et qui ne sont donc pas des détecteurs.
12. **AC12 — Rien ne bouge d'autre.** Aucune valeur de géométrie, aucun plafond,
    aucun prompt, aucune ligne de `dispatch-lib.sh`, aucun refus nouveau. Les
    tests figeant la géométrie arch et la fenêtre d'historique passent sans
    modification.

## Definition of Done

- [ ] `DEFAULT_BRIEF_SIZE_ALERT_BYTES`, le champ `brief_size_alert_bytes` et
      `Settings::effective_brief_size_alert_bytes()` livrés dans
      `crates/mika-common/src/config.rs`, sur le motif de
      `effective_pilot_cost_alert_usd`
- [ ] `MIKA_BRIEF_SIZE_ALERT_BYTES` déclarée dans `.env.example`
- [ ] Le franchissement émis au site unique `agent_loop/mod.rs:5271`, avec les
      quatre composants, le `trace_id` et le seuil sur la ligne
- [ ] La ligne `audit_events` écrite, fire-and-forget, avec son événement
      d'échec nommé
- [ ] `mika2474_the_overrun_name_has_a_single_writer` livré, allowlist vide,
      test frère figeant la vacuité, assertion auto-nettoyante
- [ ] `mika2474_the_threshold_has_a_single_reader` livré, allowlist vide, avec
      son contrôle de bonne foi vérifié **rouge** avant d'être vert
- [ ] Contrôle négatif : un brief **sous** le seuil n'émet ni ligne ni audit
      (sans lui, « le seuil décide » est indistinguable de « le seuil émet
      toujours »)
- [ ] Contrôles de palier : `0`, négatif, absent — chacun avec son attendu, le
      WARN inclus
- [ ] Doc-comment de `truncated_messages` / `truncated_bytes` corrigé (AC10)
- [ ] `cargo test`, `cargo clippy`, `cargo fmt` propres
- [ ] `crates/mika-agent/CLAUDE.md` : le seuil documenté sous § *Observability*,
      à côté de `context_window_assembled`, avec sa table de lecture
- [ ] `crates/mika-common/CLAUDE.md` : le seuil documenté à côté de
      `pilot_cost_alert_usd`
- [ ] `CLAUDE.md` racine : une section portant les quatre commandes, les régimes
      attendus et les quatre haltes du § 7
- [ ] Corps de PR portant AC1 (les cinq rectifications, avec leurs références de
      ligne), AC3 (les quatre refus) et la précondition écrite du suivi § 9
- [ ] Tickets de suivi ouverts : (a) **refus a priori** au-delà d'une taille,
      précondition = sonde S2 sur 30 jours ; (b) **réduction de
      `tool_defs_bytes`**, précondition = une mesure `jq` montrant sa part non
      négligeable
