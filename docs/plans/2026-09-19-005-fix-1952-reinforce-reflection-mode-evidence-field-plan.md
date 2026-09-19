---
title: Le contrat déclaré des trois outils gardés cesse de mentir en mode réflexion — Plan
type: fix
date: 2026-09-19
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
issue: senara-solutions/mika#1952
---

# Le contrat déclaré des trois outils gardés cesse de mentir en mode réflexion — Plan

## Goal Capsule

- **Objectif :** en mode réflexion, le schéma JSON servi au modèle pour `update_fact`, `store_fact` et `update_core_memory` déclare `evidence` comme **requis** — ce que la garde d'exécution `check_reflection_evidence` exige déjà depuis toujours. Les descriptions de champ et le bloc `## Available tools` du prompt de réflexion disent la même chose, au point de décision.
- **Moyens :** un filtre nommé appliqué au tableau d'outils du tour silencieux quand `is_reflection` (U1), qui est **lecteur unique** de la liste des outils gardés ; l'alignement des trois descriptions (U2) ; le renfort du prompt demandé par le ticket (U3) ; la couverture de test qui n'existe pas aujourd'hui (U4) ; la correction de la sonde de l'AC3, qui ne mesure pas ce qu'elle prétend (U5).
- **Autorité :** le corps de mika#1952 > le doc d'investigation mika#1770. Le commentaire opérateur du 19/09 porte le motif du dé-parquage, pas de correction de trajectoire technique. Le ticket ordonne Option 1 d'abord, Option 2 (dédoublement des outils) hors périmètre — **ce plan respecte les deux**, voir M4 : la correction du schéma n'est pas l'Option 2, elle ne crée aucun outil.
- **Profil d'exécution :** Rust, `crates/mika-agent` seul. Aucun changement de schéma DB, aucune migration, aucune variable d'environnement nouvelle, aucun appel réseau nouveau, aucun outil ajouté ni retiré de la surface.
- **Finish/ship :** le pipeline `/mika` sur la branche `fix/1952/reinforce-reflection-mode-evidence-field` ouvre la PR qui clôt mika#1952.

---

## Product Contract

### Summary

En mode réflexion, trois outils refusent un appel sans champ `evidence` non vide. Le refus vient d'une garde d'exécution partagée, `tools/mod.rs:533`. Le **schéma JSON** que ces trois outils déclarent au modèle ne mentionne `evidence` dans aucun de ses tableaux `required` : le contrat déclaré dit « facultatif », le moteur répond « obligatoire ».

Mesure (mika#1770, `tool_calls` du tenant `mika`, 2026-07-28 → 2026-08-17, N=17) : **8 échecs de première tentative sur 17**, tous portant la même sortie mot pour mot — `Reflection mode requires an evidence field citing specific conversation content.` Sept se réparent par un retry dans les dix secondes. Le huitième — commitment id=52, session `reflection-2026-08-17` — ne se répare pas : la session se termine, et l'annulation n'atterrit jamais.

Ce n'est pas un trou de documentation. Le doc d'investigation écarte explicitement la classe C : le prompt de réflexion **dit déjà** « The evidence field MUST cite a specific conversation timestamp and quote ». La règle est présente et le modèle la rate malgré tout, au moment précis où il émet un lot parallèle d'appels et remplit les champs que le schéma lui demande. **C'est le schéma qu'il lit pour remplir, et le schéma ment.**

### Problem Frame

#### M1 — Le site de la contradiction, exactement

La garde est partagée, et elle a exactement trois appelants :

| fichier | ligne de l'appel | `required` déclaré | description du champ `evidence` |
|---|---|---|---|
| `tools/update_fact.rs` | 60 | `["id","category","updates"]` | « Required in reflection mode: cite a specific conversation timestamp and quote as justification for this change » |
| `tools/store_fact.rs` | 72 | `["category"]` | *identique à ci-dessus* |
| `tools/update_core_memory.rs` | 126 | `["section","action","reasoning"]` | « **Only** required in reflection mode. Cite a specific conversation timestamp and quote as justification for this change. » |

`tools/mod.rs:533` — `check_reflection_evidence` — renvoie l'erreur quand `ctx.is_reflection` et que `input["evidence"]` est vide après `trim()`. Le prédicat est le même pour les trois ; le contrat déclaré ne l'est pas, et aucun des trois ne le porte.

Noter la troisième ligne : `update_core_memory` commence par « **Only** required », une formulation qui minimise là où les deux autres affirment. Trois sites, trois textes voisins, un déjà dérivé — la forme de dérive que `docs/solutions/prompt-engineering/2026-09-06-un-prompt-qui-reimplemente-une-garde-executable-derive.md` documente, vue depuis le côté déclaratif.

#### M2 — `definition()` ne connaît pas le mode, mais le site d'assemblage, si

`Tool::definition(&self) -> ToolDefinition` (`tools/mod.rs:199`) ne prend pas de `ToolContext`. C'est ce qui a fait proposer au ticket l'Option 2 (enregistrer des outils `*_reflection` jumeaux) : si l'outil ne peut pas connaître le mode, dédoublons-le.

La lecture du code donne une troisième voie que le ticket n'avait pas :

- `run_silent_agent` (`agent_loop/mod.rs`) obtient `skill_tool_defs: Vec<ToolDefinition>` en **5484**, calcule `let is_reflection = matches!(&params.trigger, SilentTrigger::Reflection)` en **5558**, et ne convertit en `Vec<LlmToolDefinition>` qu'en **5677**.
- La conversion est un simple déplacement de champ : `impl From<ToolDefinition> for LlmToolDefinition` (`mika-common/src/llm/types.rs:268`) pose `parameters: td.input_schema` sans le lire.
- Le précédent existe déjà à cet endroit : `apply_agent_tool_visibility` (`agent_loop/mod.rs:7282`) est décrit dans son propre doc-comment comme le **hook nommé** appliqué « à la couche de présentation LLM, avant que le tableau atteigne l'appel API ».

Il y a donc une fenêtre de ~190 lignes, dans une seule fonction, où le mode est connu et le schéma encore mutable. Le contrat peut être rendu exact **sans toucher au trait, sans dédoubler un seul outil, sans changer la surface d'outils** — donc sans le coût qui a fait écarter l'Option 2.

#### M3 — Les deux autres modes sont hors population, par construction

`is_reflection` est écrit `false` en dur aux deux autres sites de construction de `ToolContext` (`agent_loop/mod.rs:4578` conversation, `6271` équipe) et n'est calculé qu'en 5558. La mutation ne peut donc atteindre qu'un tour `SilentTrigger::Reflection` ; les autres déclencheurs silencieux (heartbeat, callback, reminder…) servent le schéma inchangé. **Le contrôle négatif est une obligation de test, pas une évidence** (U4-c).

#### M4 — Ce que ce plan livre n'est PAS l'Option 2, et la distinction est vérifiable

Le ticket pose l'Option 2 hors périmètre et en donne le motif : « doubles the tool surface area ». Le critère est donc la **surface**, pas le mécanisme.

| | Option 2 (hors périmètre) | U1 de ce plan |
|---|---|---|
| Outils enregistrés | +3 (`update_fact_reflection`, …) | inchangé |
| Noms visibles du modèle | +3 | inchangé |
| Dispatch d'exécution | nouvelle branche par outil | inchangé |
| Mode conversation | inchangé | inchangé |
| Garde d'exécution | remplacée par le schéma | **conservée** (voir M5) |

U1 ajoute **zéro** entrée à la surface. Il corrige la valeur d'un champ d'un schéma existant. L'assertion « le tableau d'outils du tour de réflexion a exactement la même liste de noms qu'avant » est épinglée par un test (U4-c).

#### M5 — `required` n'est pas une garantie dure, et c'est pour ça que la garde reste

À écrire sans l'adoucir : Mika n'émet pas `strict: true` sur ses définitions d'outils, et ni l'API Anthropic ni les rails OpenAI-compatibles ne refusent côté serveur un appel dont il manque une clé `required`. Le `required` **oriente** le modèle ; il ne le contraint pas.

Deux conséquences, toutes deux portantes :

1. **La garde d'exécution n'est pas remplacée, elle reste la seule barrière dure.** Elle attrape en plus ce que `required` ne verra jamais : `"evidence": ""` satisfait `required` et échoue la garde (`trim().is_empty()`). U4-d épingle ce cas précisément, pour qu'un futur lecteur ne retire pas la garde en la croyant redondante.
2. **Le gain attendu est probabiliste, comme celui de l'Option 1.** Ce que U1 achète par rapport à une simple réécriture de description, ce n'est pas une garantie : c'est de faire passer l'information du **corps d'un texte** au **contrat structuré** que le mécanisme de tool-calling remplit champ par champ — c'est-à-dire précisément là où le doc d'investigation situe le ratage (« il émet les champs requis par le schéma et oublie le champ conditionnel »). Et accessoirement, ça supprime une contradiction interne dont le coût ne se mesure pas en taux de miss mais en temps de lecture du prochain mainteneur.

#### M6 — La sonde de l'AC3 ne mesure pas ce qu'elle annonce

La requête que l'AC3 demande de rejouer est, dans le doc d'investigation :

```sql
SELECT COUNT(*) as n, SUM(CASE WHEN success=0 THEN 1 ELSE 0 END) as failed, ...
FROM tool_calls WHERE agent_id='mika' AND tool_name='update_fact';
```

Trois défauts, chacun mesurable avant de déployer quoi que ce soit :

1. **Elle ne filtre pas le mode réflexion.** Elle compte tous les `update_fact` du tenant, mode conversation compris, où la garde ne s'applique pas. Le discriminant existe et est gratuit : `dispatch_reflection` (`task_engine/dispatcher.rs:1970`) écrit `session_id = format!("reflection-{today_str}")`, et `tool_calls` porte `session_id` (DDL `db/migrations.rs:2193`). Il manque un `AND session_id LIKE 'reflection-%'`.
2. **Elle n'a aucune puissance statistique.** N=17 sur 30 jours. Un seuil « < 5 % » sur N≈17 ne distingue pas 0/17 de 1/17 : un unique miss post-correctif rend 5,9 % et ferait « échouer » un correctif qui marche. Ce n'est pas un critère, c'est un tirage.
3. **Elle peut porter sur une population vide, sans le dire.** `[reflection].enabled` vaut `false` par défaut (`prompt.rs:267`) et les identités well-known ont **interdiction** de porter une section `[reflection]` (assertion `well_known_agents.rs:3570`). La réflexion est opt-in par tenant, `dispatch_reflection` saute encore si l'utilisateur a parlé dans les 30 minutes ou s'il n'y a eu aucune conversation du jour. **Zéro échec est compatible avec zéro réflexion**, et la requête rend le même chiffre dans les deux cas — la classe mika#2205 exactement.

U5 corrige la requête et remplace le seuil par un protocole qui dit ce que son silence vaut. La moitié déterministe du contrat passe aux tests (U4), qui, eux, ne dépendent d'aucune population.

#### M7 — Il n'existe aujourd'hui aucun test sur ce chemin

`grep -rn "Reflection mode requires" crates/` rend **un seul** résultat : le site d'émission. Aucun test, unitaire ou eval, ne couvre `check_reflection_evidence`, et `grep -rn "reflection" crates/mika-agent/tests/` ne rend que trois occurrences sans rapport (`reflection: None` dans des fixtures d'identité).

Le motif de test nécessaire existe et est directement réutilisable : `tests/eval/test_qa_build_callback_verdict_2355.rs` pilote `run_silent_agent` avec un `SilentTrigger` choisi, et `MockLlmProvider::captured_requests()` (`mika-common/src/llm/mock.rs:100`) rend les `LlmRequest` servies — donc le `tools` réellement envoyé. **Le contrat « le schéma servi porte `evidence` dans `required` » est assertable sur le chemin de production, sans réseau.**

### Non-goals

- **Option 2 (dédoublement des outils).** Hors périmètre par décision du ticket, et U1 la rend sans objet pour le défaut mesuré. Si la sonde U5 montre un résidu, c'est le ticket de suivi qui la tranchera.
- **Restructuration du prompt de réflexion** (déplacer `## Rules` au-dessus de `## What to do`). Explicitement hors périmètre du ticket, et le doc d'investigation a écarté la classe « layout ».
- **Étendre la garde à d'autres outils ou à d'autres modes.** La population est les trois appelants existants de `check_reflection_evidence` ; U4-e la fige et fait rougir l'ajout d'un quatrième non traité, mais n'en ajoute aucun.
- **Activer la réflexion sur un tenant qui ne l'a pas.** C'est une décision produit par tenant, pas un effet de bord de ce correctif. Elle est nommée en U5 comme précondition de la sonde, pas comme livrable.
- **Retirer la garde d'exécution.** Voir M5 : elle reste la seule barrière dure, et U4-d existe pour empêcher qu'on la croie redondante.

---

## Planning Contract

### Decisions

- **D1 — La correction vit au site d'assemblage, pas dans `definition()`.** Le trait ne porte pas de contexte (M2) et l'élargir pour de l'observabilité de mode aurait un rayon d'impact sur tous les outils. Le filtre est une fonction libre voisine de `apply_agent_tool_visibility`, appelée depuis `run_silent_agent` seulement.
- **D2 — Lecteur unique de la liste des outils gardés.** Le nom des trois outils est écrit **une fois**, dans une constante du module qui porte le filtre. Aucun autre site ne le réécrit. Motif maison : `grooming_marker` (mika#2158), `auto_pull_stop` (mika#2329) — un prédicat écrit deux fois est un prédicat qui peut diverger, et la divergence ne rend aucune décision fausse le jour où elle est écrite.
- **D3 — La mutation est additive et idempotente.** Le filtre **ajoute** `"evidence"` au tableau `required` s'il n'y est pas et remplace la `description` du champ ; il ne retire rien, ne réordonne rien, ne touche à aucun autre champ. Un schéma qui déclarerait déjà `evidence` requis est laissé tel quel. Un outil de la liste absent du tableau (évincé par denylist d'identité, ou par le filtre compact) est ignoré silencieusement — absence n'est pas erreur.
- **D4 — Fail-soft sur un schéma inattendu.** Si `input_schema` n'a pas la forme attendue (`properties.evidence` absent, `required` non-tableau), le filtre **laisse le schéma intact** et émet un `warn!` nommant l'outil. Servir un schéma à moitié muté serait pire que servir l'actuel ; et la garde d'exécution couvre toujours le cas. L'événement de journal est ce qui empêche ce fail-soft d'être un silence.
- **D5 — Les trois descriptions deviennent identiques, au mot.** Texte imposé par le corps du ticket (AC1), y compris pour `update_core_memory`, dont la formulation « Only required » disparaît. La description est celle du schéma **de base** ; en mode réflexion le filtre la sert telle quelle (elle est déjà écrite pour ce mode), ce qui évite un quatrième texte à maintenir.
- **D6 — Le renfort de prompt est livré comme moitié d'intention, et dit comme telle.** L'AC2 le demande, il coûte quatre lignes, et il place la règle au point de décision (`docs/solutions/prompt-engineering/2026-09-07-une-garde-de-prompt-doit-vivre-sur-le-canal-que-lexecutant-lit.md`). Mais le doc d'investigation a **mesuré** l'insuffisance de la forme actuelle de cette même règle : aucune mesure ne lui sera attribuée. La moitié structurelle est U1.
- **D7 — L'AC3 est reformulé, pas abandonné.** Le seuil « > 95 % sur 30 jours » est remplacé par : requête corrigée + capture de baseline **avant** déploiement + critère de lecture explicite avec ses haltes. Voir U5. Un critère qu'on ne peut pas satisfaire honnêtement est un critère qu'on finit par déclarer satisfait.
- **D8 — L'AC4 (commitment id=52) est un geste opérateur, pas du code.** Il porte sur `~/.mika/data/mika.db` du tenant `mika`, inatteignable depuis ce worktree. Livré comme procédure nommée (U6), avec sa précondition (le correctif déployé) et sa vérification.

### Risks

| # | Risque | Traitement |
|---|---|---|
| R1 | Un quatrième outil appelle `check_reflection_evidence` sans être ajouté à la liste → son schéma redevient menteur, en silence. | **U4-e** : scan de source comparant la liste des appelants de la garde à la constante du filtre. Aucun test comportemental ne peut voir cette classe : le nouvel outil marcherait, il mentirait simplement. |
| R2 | Le filtre est supprimé ou déplacé après la conversion → plus aucun effet, et **aucune assertion existante ne rougit**. | **U4-b** épingle le schéma **servi** (via `captured_requests()`), pas le retour du filtre. Un filtre appelé trop tard fait rougir ce test. |
| R3 | Le modèle, voyant `evidence` requis, fabrique un evidence vide ou bidon pour satisfaire le schéma. | Le vide est attrapé par la garde (U4-d). Le « bidon » est hors de portée de tout mécanisme structurel ici — c'est la classe `assert_grounded` (mika#1331), nommée et non couverte. |
| R4 | Un rail provider tolère mal un `required` sur un champ absent de son idée du schéma. | Les trois schémas déclarent déjà `evidence` dans `properties` ; `update_core_memory` a `additionalProperties: false` et le déclare aussi. Aucun champ nouveau n'apparaît : seul le tableau `required` change. |
| R5 | La sonde U5 ne trouve aucune réflexion sur la fenêtre → lue comme « zéro miss, correctif validé ». | U5 impose de lire **N d'abord**. `N = 0` est écrit comme « rien mesuré », jamais comme un succès. Halte explicite. |
| R6 | La rétention purge les `tool_calls` pré-correctif avant la comparaison. | U5 impose la capture de baseline **avant** déploiement, dans le corps de PR. |

### Open Questions

- **(non bloquante)** Le résidu de 13 % mesuré est *un* cas sur *une* session. Si U1 le ramène à zéro observable mais que N reste ≈ 17/30 j, il n'y aura jamais de quoi trancher entre « corrigé » et « pas assez de tirages ». Un eval real-provider sur le tour de réflexion (famille `MIKA_EVAL_REAL_PROVIDERS`) serait la seule mesure répétable. **Ticket de suivi**, non ouvert ici : son préalable est la sonde U5, dont le résultat décide s'il y a quelque chose à mesurer.
- **(non bloquante)** `store_fact` déclare `required: ["category"]` seulement, et route ensuite vers quatre sous-catégories dont les champs obligatoires sont vérifiés à l'exécution. La même classe de contradiction déclaratif/runtime que celle-ci, sur un autre axe. Hors périmètre ; noté pour qui passera après.

---

## Implementation Units

### U1 — Le schéma servi en mode réflexion déclare `evidence` requis

- **Fichier :** `crates/mika-agent/src/agent_loop/mod.rs` (voisin de `apply_agent_tool_visibility`, ~7282).
- Constante `REFLECTION_EVIDENCE_GATED_TOOLS: &[&str] = &["update_fact", "store_fact", "update_core_memory"]` — **seul site** où cette liste est écrite (D2), avec un doc-comment pointant `tools::check_reflection_evidence` comme la garde dont elle est le miroir.
- `pub(crate) fn apply_reflection_evidence_contract(tool_defs: &mut [ToolDefinition])` : pour chaque définition dont le nom est dans la liste, ajoute `"evidence"` à `input_schema["required"]` si absent. Additif, idempotent (D3), fail-soft avec `warn!` sur forme inattendue (D4). Émet un `debug!` par outil muté et un `info!` d'agrégat (`mutated_count`) quand `> 0`.
- **Appel :** dans `run_silent_agent`, après l'obtention de `skill_tool_defs` (~5484) et **avant** la conversion en `LlmToolDefinition` (~5677), sous `if is_reflection`. Déplacer le calcul de `is_reflection` (~5558) au-dessus de l'appel — il ne dépend que de `params.trigger`. Rendre `skill_tool_defs` mutable.
- **Ne touche pas :** les deux autres appels à `inject_skills_and_resolve_tools` (conversation ~4361, équipe ~6199).

### U2 — Les trois descriptions `evidence` deviennent identiques

- `crates/mika-agent/src/tools/update_fact.rs` (~48-52), `store_fact.rs` (~57-60), `update_core_memory.rs` (~64-67).
- Texte unique, celui de l'AC1 du ticket : `REQUIRED IN REFLECTION MODE. Format: "[YYYY-MM-DDTHH:MM:SSZ] <one-sentence citation of the conversation content that justifies this change>". Missing or empty evidence in reflection mode ALWAYS returns an error — no exceptions. Example: "[2026-07-28T13:00:00Z] Reflection search found id=22 duplicate of id=25, both pending, no actionable meaning."`
- Le « Only required » de `update_core_memory` disparaît (M1). Aucun autre champ n'est modifié — en particulier l'alias non documenté `reason` de `update_core_memory` (#488) est laissé intact.

### U3 — Le prompt de réflexion nomme l'exigence au point de décision

- `crates/mika-agent/src/agent_loop/mod.rs`, bloc `SilentTrigger::Reflection` (~5312-5336), section `## Available tools` (~5316-5320).
- Les trois lignes d'outils gardés portent le suffixe `MUST include `evidence` field in reflection mode — no exceptions.` (texte AC2). `search_memory` est inchangé : il ne porte pas la garde.
- La ligne existante de `## Rules` (« The evidence field MUST cite a specific conversation timestamp and quote ») est **conservée** : elle dit le *format*, les nouvelles disent le *caractère obligatoire* et *où*. La retirer transformerait un renfort en déplacement.

### U4 — La couverture qui n'existait pas

- **(a) Unitaires du filtre** (`agent_loop/mod.rs` `mod tests`) : mutation des trois outils ; idempotence (double application) ; outil hors liste intact ; tableau partiel (un seul des trois présent) ; fail-soft sur `properties.evidence` absent et sur `required` non-tableau, sans panique.
- **(b) Eval, chemin de production** — `crates/mika-agent/tests/eval/test_reflection_evidence_contract_1952.rs`, sur le motif de `test_qa_build_callback_verdict_2355.rs` : `run_silent_agent` + `SilentTrigger::Reflection`, puis assertion sur `MockLlmProvider::captured_requests()[0].tools` — les trois outils y portent `evidence` dans `required`. C'est le schéma **servi**, ce qui couvre R2.
- **(c) Contrôle négatif** : même harnais avec `SilentTrigger::Heartbeat` → les trois schémas servis sont inchangés, **et** la liste des noms d'outils servis est identique entre les deux tours (preuve que la surface n'a pas bougé, M4).
- **(d) La garde reste la barrière dure** : appel de `update_fact` en mode réflexion avec `"evidence": ""` → `ToolOutput::error` portant le message de `check_reflection_evidence`. Le doc-comment du test dit pourquoi il existe : `required` n'interdit pas la chaîne vide (M5).
- **(e) Garde structurelle** (`tools/mod.rs` `mod tests`) : scan de source sur `crates/mika-agent/src/tools/*.rs` recensant les appelants de `check_reflection_evidence` et comparant l'ensemble à `REFLECTION_EVIDENCE_GATED_TOOLS`. Divergence **dans les deux sens** = rouge. Message d'échec nommant le geste (ajouter l'outil à la constante, ou retirer l'appel). Couvre R1 ; aucun test comportemental ne peut le faire.

### U5 — La sonde de l'AC3, corrigée et bornée

Documentée dans le corps de PR et dans le doc d'investigation (`docs/solutions/prompt-engineering/2026-08-22-...md`, section « Follow-up fix »), qui gagne une note datée.

- **Requête corrigée** (le `session_id LIKE` est le discriminant manquant, M6) :
  ```sql
  SELECT tool_name,
         COUNT(*) AS n,
         SUM(CASE WHEN success=0 THEN 1 ELSE 0 END) AS failed
  FROM tool_calls
  WHERE agent_id='mika'
    AND session_id LIKE 'reflection-%'
    AND tool_name IN ('update_fact','store_fact','update_core_memory')
    AND created_at >= strftime('%Y-%m-%dT%H:%M:%SZ','now','-30 days')
  GROUP BY tool_name;
  ```
  Et le contrôle qualitatif, qui vaut plus que le taux à ce N : `SELECT created_at, session_id, tool_name, output FROM tool_calls WHERE success=0 AND output LIKE 'Reflection mode requires%' ORDER BY created_at DESC;` — **régime attendu : zéro ligne postérieure au déploiement.**
- **Baseline capturée AVANT déploiement** et reportée dans le corps de PR (R6).
- **Protocole de lecture, avec ses haltes.**
  - **Lire `n` d'abord.** `n = 0` ⇒ *rien mesuré* : vérifier que `[reflection].enabled = true` sur le tenant et que des sessions `reflection-*` existent sur la fenêtre. **Ne pas lire zéro échec comme un succès** (R5, classe mika#2205).
  - **`n > 0`, zéro ligne `Reflection mode requires%`** ⇒ résultat conforme, énoncé comme « aucun miss observé sur N=<n> », jamais comme « < 5 % ».
  - **Halte — le miss persiste au même rythme** ⇒ le schéma servi n'est pas celui qu'on croit. Lire `system_prompt_bytes` / la requête servie **avant** de retoucher un texte : soit le binaire déployé précède le correctif (classe mika#2340), soit le tour passe par un chemin que U1 ne traverse pas. Établir lequel d'abord.
  - **Halte — le miss disparaît mais un `"evidence": ""` apparaît** ⇒ le modèle satisfait le schéma sans le contrat. C'est R3, et le remède n'est ni le schéma ni le prompt : c'est la famille `assert_grounded`. Ticket de suivi.

### U6 — AC4 : le commitment résiduel (geste opérateur, hors code)

- **Précondition :** U1–U3 déployés (`make deploy`), sinon la tentative rejoue le défaut.
- **Cible :** `agent_id='mika'`, commitment `id=52`, « Vincent has minimum 5 months of runway… », `status='pending'` depuis le 2026-08-17.
- **Geste :** retry supervisé via le tour de réflexion ou un appel direct de `update_fact` portant un `evidence` conforme au format de U2 et citant la session `reflection-2026-08-17`. Le choix entre `completed` et `cancelled` appartient à l'opérateur et n'est pas tranché ici — le ticket lui-même laisse les deux ouverts.
- **Vérification :** `SELECT id, status FROM commitments WHERE id=52 AND agent_id='mika';` ne rend plus `pending`.
- **Ce que le geste ne prouve pas :** qu'il réussisse du premier coup ne valide pas U1 (un retry manuel réussissait déjà avant le correctif). La sonde est U5.

---

## Verification Contract

| Vérification | Commande / geste | Critère |
|---|---|---|
| Compilation + lint | `cargo build && cargo clippy -- -D warnings` | zéro erreur, zéro warning |
| Format | `cargo fmt --check` | propre |
| Unitaires du filtre (U4-a) | `cargo test -p mika-agent apply_reflection_evidence_contract` | vert |
| Garde structurelle (U4-e) | `cargo test -p mika-agent mika1952_gated_tools` | vert ; **et rouge** si un des trois noms est retiré de la constante (vérifié à la main une fois) |
| Chemin de production (U4-b/c/d) | `cargo test -p mika-agent --test eval reflection_evidence` | vert ; le test (b) vérifié rouge sans l'appel à U1 |
| Non-régression globale | `cargo test` | aucune régression |
| Surface d'outils inchangée (M4) | assertion portée par U4-c | listes de noms identiques réflexion / heartbeat |
| Baseline sonde (U5) | requête corrigée, avant déploiement | chiffres reportés dans le corps de PR |

**Contrôle négatif exigé, et il n'est pas facultatif :** le test U4-b doit être constaté **rouge** avec l'appel à `apply_reflection_evidence_contract` commenté. Sans cette vérification, rien ne distingue « le filtre marche » de « le schéma portait déjà `evidence` ».

---

## Definition of Done

- [ ] `apply_reflection_evidence_contract` existe, est appelé depuis `run_silent_agent` sous `is_reflection`, avant la conversion en `LlmToolDefinition`.
- [ ] `REFLECTION_EVIDENCE_GATED_TOOLS` est le seul site où la liste des trois outils est écrite.
- [ ] Les trois descriptions `evidence` portent le texte unique de l'AC1.
- [ ] Le bloc `## Available tools` du prompt de réflexion porte l'exigence sur les trois lignes concernées ; la ligne de `## Rules` est conservée.
- [ ] U4-a/b/c/d/e écrits et verts ; U4-b vérifié rouge sans l'appel.
- [ ] `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check` verts.
- [ ] Baseline de la sonde capturée et reportée dans le corps de PR.
- [ ] Le doc d'investigation mika#1770 porte une note datée renvoyant à la requête corrigée.
- [ ] Le corps de PR nomme explicitement : la surface d'outils est inchangée (ce n'est pas l'Option 2), la garde d'exécution est conservée, l'AC3 est reformulé avec son motif, l'AC4 est un geste post-déploiement.

---

## Acceptance criteria

Transcrits du corps de mika#1952. Les écarts sont nommés, jamais silencieux.

- [ ] **AC1** — Les trois outils (`update_fact`, `store_fact`, `update_core_memory`) portent la description étendue du champ `evidence`. *(U2, verbatim.)*
- [ ] **AC2** — Le prompt de réflexion liste l'exigence `evidence` en ligne à côté de chacun des trois noms d'outils dans le bloc `## Available tools`. *(U3, verbatim.)*
- [ ] **AC3 — reformulé (D7), motif en M6.** Le critère original (« taux de succès de première tentative > 95 % sur 30 jours ») n'est pas mesurable tel qu'écrit : la requête citée ne filtre pas le mode réflexion, N≈17 ne donne aucune puissance à un seuil de 5 %, et une population vide rend le même chiffre qu'une population saine. Remplacé par : **(a)** la sonde corrigée de U5 est documentée, **(b)** sa baseline est capturée avant déploiement, **(c)** sur une fenêtre de 30 jours post-déploiement avec `n > 0`, aucune ligne `output LIKE 'Reflection mode requires%'` n'est postérieure au déploiement, **(d)** `n = 0` est rapporté comme « rien mesuré » et déclenche la vérification d'activation de la réflexion, jamais comme un succès.
- [ ] **AC4** — Le commitment résiduel id=52 sur `agent_id='mika'` est résolu (annulé ou complété) par un retry supervisé avec evidence. *(U6 — geste opérateur post-déploiement, hors code ; la procédure et sa vérification sont livrées ici.)*
- [ ] **AC5 (ajouté par ce plan, motif en M1/M5)** — En mode réflexion, le schéma servi au modèle pour les trois outils déclare `evidence` dans `required` ; hors mode réflexion il est inchangé ; la surface d'outils (liste des noms) est identique dans les deux modes ; la garde d'exécution `check_reflection_evidence` continue de refuser un `evidence` vide.

---

## Sources

- `senara-solutions/mika#1952` — corps + commentaire opérateur du 2026-09-19.
- `docs/solutions/prompt-engineering/2026-08-22-reflection-update-fact-evidence-first-attempt-miss.md` — trace de mesure (N=17, 8 échecs, retry 7/8, résidu id=52), verdicts de classe A→E.
- `crates/mika-agent/src/tools/mod.rs:531-546` — `check_reflection_evidence`, la garde partagée.
- `crates/mika-agent/src/tools/update_fact.rs:48-62`, `store_fact.rs:57-73`, `update_core_memory.rs:64-127` — les trois schémas et les trois appels.
- `crates/mika-agent/src/agent_loop/mod.rs:5312-5336` — prompt de réflexion ; `:5484,5558,5677` — fenêtre d'assemblage ; `:7268-7310` — `apply_agent_tool_visibility`, le précédent.
- `crates/mika-common/src/llm/types.rs:268-276` — `From<ToolDefinition> for LlmToolDefinition` (le schéma passe tel quel).
- `crates/mika-common/src/llm/mock.rs:85-125` — `captured_requests()`.
- `crates/mika-agent/tests/eval/test_qa_build_callback_verdict_2355.rs` — motif de test `run_silent_agent`.
- `crates/mika-agent/src/task_engine/dispatcher.rs:1892-1990` — `dispatch_reflection`, ses quatre conditions de saut et `session_id = reflection-<date>`.
- `crates/mika-agent/src/task_engine/mod.rs:140-173` — `reflection_cron_for_agent` (opt-in) ; `crates/mika-agent/src/prompt.rs:249-269` — `ReflectionConfig`, défaut `enabled = false`.
- `crates/mika-agent/src/db/migrations.rs:2190-2211` — DDL `tool_calls` (présence de `session_id`).
- `docs/solutions/prompt-engineering/2026-09-06-un-prompt-qui-reimplemente-une-garde-executable-derive.md` — classe de la dérive déclaratif/exécutable.
- `docs/solutions/prompt-engineering/2026-09-07-une-garde-de-prompt-doit-vivre-sur-le-canal-que-lexecutant-lit.md` — motif du point de décision (U3).
- `CLAUDE.md` § mika#2205 (un scan silencieusement inactif se lit comme un scan oisif) — motif de la halte U5.
