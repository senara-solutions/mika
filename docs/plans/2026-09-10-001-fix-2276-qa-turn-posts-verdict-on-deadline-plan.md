# fix mika#2276 — le tour QA POSTE un verdict même quand sa deadline tombe

**Issue:** senara-solutions/mika#2276 (p1-important, bug)
**Branche:** `bug/2276/qa-review-le-tour-qa-aboutit-notifie`
**Type sémantique:** `fix` (le slug de branche porte le type dérivé du label `bug` ; les
deux peuvent diverger — cf. `/mika-groom-ticket` § slug-immutability)
**Distinct de:** mika#2212 (le TRIGGER ne re-dispatche pas) — ici le tour EST dispatché,
il tourne, et il meurt sans poster. Deux maillons différents de la même chaîne.

## Le défaut, mesuré

Trace `921f11f0-acd4-11f1-8bc6-90c3b908c45a`, `/var/log/mika/server.log`, tour mika-qa
sur PR#2275, 2026-09-10 05:00:55Z → 05:09:21Z (07:00–07:09 local). Chronologie exacte,
horodatages du log :

| Instant (UTC) | Événement |
|---|---|
| 05:00:55.351 | `preparing LLM request` — `provider=openrouter`, `model=z-ai/glm-5.2`, `mode=conversation`, 59 outils |
| 05:01:23.79 | step 2 — le tour a lu le callout de plan et le diff complet |
| 05:01:29.63 | `run_shell` → `./target/release/mika test test_pilot_silent_stall_reaper` |
| **05:01:32.776** | `run_shell` → `cargo test --release --features telemetry -p mika-agent --test eval -- test_reaper_reaps_live_pending_pilot_2272` |
| **05:05:30.635** | l'appel précédent rend la main — **237.9 s** — et un second `cargo test --release` part |
| **05:09:21.765** | `agent deadline exceeded — exiting loop gracefully`, `steps_completed=5` — le second appel a consommé **231.1 s** sans finir |

469 s des ~506 s d'enveloppe du tour ont été mangés par deux appels `run_shell`. Aucun
verdict n'a été rédigé, donc aucun n'a été posté.

Les trois hypothèses du corps du ticket sont **infirmées** pour ce tour : le verdict
n'est jamais produit (donc ni mauvaise identité, ni post avalé, ni format non reconnu).
Le commentaire de Vincent — « le tour QA recompile dans son budget » — est exact mais
s'arrête un cran avant la cause. Il y a **deux maillons**, et chacun est un défaut
indépendant.

### M1 — le budget d'outil que la doctrine QA croit avoir n'existe pas

`max_skill_timeout` (`crates/mika-agent/src/agent_loop/mod.rs:5415`) calcule **un seul**
budget d'exécution d'outil pour tout le tour :

```rust
matched.iter()
    .map(|e| e.effective_timeout(provider_name, model_name))
    .max()
    .unwrap_or(crate::planning::policy::TOOL_TIMEOUT_SECS)
```

Ce maximum est ensuite appliqué **uniformément à chaque appel d'outil skill** —
`dispatch.skill_timeout` en `crates/mika-agent/src/tool_execution/dispatch.rs:505`, passé
tel quel à `execute_skill_tool`, qui en fait son `tokio::time::timeout`
(`crates/mika-agent/src/skills/executor.rs:556-557`).

Conséquence : le budget d'un outil n'est pas celui de **son** skill, mais celui du skill
le plus généreux chargé dans le tour. Timeouts déclarés chez `mika-qa`
(`~/.mika/agents/mika-qa/skills/*/skill.toml`) :

```
shell-exec:            30      ← le skill qui possède run_shell
build-mika:           300      ← dépendance déclarée de qa-review (skill.toml)
address-pr-comments:  600
dev-pilot:            600
resolve-pr-conflicts: 600
```

`run_shell` déclare 30 s. Le prompt QA écrit littéralement, à
`skills/bundled/qa-review/system_prompt.md:194` : *« Measured cost on `mika` (3323
tracked files): ~0.6s, against `run_shell`'s 30s budget. »* **Le moteur ne tient pas ce
plancher.** Mesuré : 237.9 s, puis 231.1 s. La doctrine QA raisonne sur une garantie que
le moteur n'a jamais offerte, et c'est ce qui rend `cargo test --release` survivable
assez longtemps pour vider l'enveloppe.

C'est la classe exacte de `feedback_prompt_enforcement_fragile` : une règle portée par le
prompt, contredite par le substrat, qui tient jusqu'au jour où un modèle prend la porte
que le moteur a laissée ouverte.

### M2 — le dépassement de deadline est un silence sur le canal GitHub

En mode conversation, `LoopResult::DeadlineExceeded` tombe dans
`persist_deadline_fallback` (`agent_loop/mod.rs:3781-3814`) : un message assistant
générique — *« I'm sorry, that took too long. Let me try a simpler approach next time. »*
— est persisté en base et rendu comme `AgentOutput.text`.

Le caller (`crates/mika-agent/src/server/handlers.rs:1440-1456`) ne distingue pas ce
texte d'une vraie réponse : il l'envoie sur le canal de réponse via `sender_arc.send()`.

**C'est le mécanisme exact du symptôme du ticket.** Telegram est notifié parce que le
fallback part sur le canal ; la PR ne reçoit rien parce que le seul chemin qui poste un
verdict est `run_gh pr review`, appelé par le LLM — qui n'a jamais atteint cette étape.
« Le tour a abouti » (Telegram) et « le tour a conclu » (verdict) sont deux faits
différents, et rien dans le code ne les sépare aujourd'hui.

`AgentOutput` (`agent_loop/mod.rs:297-301`) ne porte aucun champ disant *pourquoi* le
tour s'est terminé — c'est l'absence structurelle qui rend le filet impossible à écrire
au call-site aujourd'hui.

## Les voies, et ce que la mesure en dit

Le ticket propose trois voies et confie l'arbitrage à l'architecte. La mesure ci-dessus
en disqualifie une, en contraint une autre, et en révèle une quatrième que le ticket ne
nomme pas.

**(a) Lire la CI au lieu de recompiler — collision frontale avec une règle existante.**
`skills/bundled/qa-review/system_prompt.md:45` interdit explicitement à la QA de lire le
statut CI par quelque outil que ce soit (`gh pr checks`, `check-runs`,
`statusCheckRollup`), et `qa_pr_view` **strippe** les champs CI par construction
(`skills/bundled/qa-review/tools.json`). La voie (a) n'est donc pas un ajustement de
prompt : c'est le renversement d'une règle de Data-Integrity que d'autres décisions
portent (mika#2264 ferme même la classe « CI-deferred » sur le périmètre 2.5.4b,
`system_prompt.md:292`, précisément parce que déférer à la CI coche une case que personne
n'a posée). **Recommandation : hors scope de ce ticket.** L'ouvrir ici mélangerait une
correction de substrat p1 avec une révision de doctrine de revue.

**(b) Deadline dédié au tour QA — ne ferme aucune classe.** Allonger l'enveloppe déplace
le dépassement, il ne le supprime pas : un `cargo test --release` à froid sur `mika`
n'a pas de plafond que 600 s ou 900 s garantirait. Utile au mieux comme réglage, jamais
comme fix. **Recommandation : non retenu seul.**

**(c) Verdict explicite au dépassement — structurellement obligatoire ici.** AC2 exige un
test qui asserte « deadline dépassé ⇒ verdict posté ». Aucune autre voie ne peut
satisfaire cette assertion : (a) et (b) rendent le dépassement plus rare, seule (c) rend
le silence impossible. La directive de groom a donc déjà tranché : **(c) fait partie du
livrable.**

**(d) Rendre le budget d'outil per-outil — la cause à sa source.** Ce n'est pas une
extension hors du champ tracé par le ticket : la piste (a) de Vincent contient **deux**
volets — *« interdire les builds/tests longs dans la revue QA »* ET *« le tour lit les
logs CI au lieu de recompiler »*. Le plan écarte le second (collision de doctrine,
ci-dessus) et réalise le premier — par voie structurelle plutôt que par prompt, parce
qu'une interdiction écrite dans le prompt est précisément ce qui a échoué ici : le prompt
QA *dit déjà* que `run_shell` a 30 s.
Si `run_shell` était coupé aux 30 s de son propre manifeste, le premier
`cargo test` rendait la main à 30 s et le tour gardait ~470 s pour rédiger et poster.
Cette voie ferme M1 ; elle rend aussi la phrase du prompt QA (`:194`) vraie, au lieu de
la laisser être une croyance.

**Position portée à l'architecte : (c) + (d).** (c) est le filet — il garantit qu'un
dépassement produit un signal et jamais un silence, quelle que soit sa cause future.
(d) est la cause — il retire au tour QA le moyen de se suicider par recompilation. Les
deux sont nécessaires : (d) sans (c) laisse la prochaine cause de dépassement muette ;
(c) sans (d) laisse la QA échouer bruyamment à chaque revue de PR substrat.

### Tranchages de l'architecte — premier passage, `Disposition: READY`

Session mika-arch `4f793619-a706-4e1c-883d-cf7db01180e6`, 2026-09-10. Les quatre
questions posées sont tranchées ; ce qui suit est la décision, pas une option.

**Q1 — forme du verdict de secours : `hold[review]`.** Surface existante
(`qa-review/skill.toml`), déjà comprise par `verdict_handler.rs`, **CODEOWNERS épargné**.
L'architecte confirme l'écartement de (a) dans les mêmes termes : *« lire la CI viole la
doctrine "pas de CI-deferred" (mika#2264 2.5.4b) ; un p1 ne renverse pas une doctrine de
revue, même si Vincent la nomme préférée. »*

**Q2 — (d) reste dans CE ticket.** *« la sémantique max-skill-timeout est la cause racine
(M1). Scinder créerait un ticket bloquant pour un p1. »* Le fan-out de 3 sites est accepté
comme dette temporaire, à condition que la table de disposition au feu prévoie
HALT-et-investiguer si un autre agent casse — elle le prévoit (ligne 3).

**Q3 — signal de dépassement : un champ sur `AgentOutput`.** *« suffisant car
`handlers.rs:1440` consomme déjà cette struct. Propager `LoopResult` jusqu'aux handlers
toucherait l'interface agent-core/orchestrateur ; pour un p1, préférez la surface
minimale. »* La variante « remonter `LoopResult` » est donc écartée.

**Q4 — `pr_reviews_posted` validé, avec une exigence ajoutée.** *« Ajoutez un contrôle
défensif : si POST GitHub retourne 422 (déjà posté), l'agent doit l'interpréter comme
succès idempotent, non comme échec de verdict. »* Reportée en AC3 ci-dessous — c'est une
exigence de l'architecte, pas une suggestion.

## Acceptance criteria

- **AC1 — le tour QA POSTE un verdict même quand son travail dépasse la deadline.**
  Quand le tour mika-qa se termine par dépassement d'enveloppe (`LoopResult::DeadlineExceeded`)
  alors qu'il traite un événement PR et qu'aucune review n'a été postée pour cette PR
  dans la session, le moteur poste lui-même un verdict sur la PR — **la ligne canonique
  `VERDICT: hold[review]`** (forme tranchée en Q1 ; ni `block[timeout]`, ni un verdict
  neuf), un corps portant le motif (deadline dépassée), le nombre de steps accomplis, et
  le `trace_id` du tour. Le fallback conversationnel actuel
  (`persist_deadline_fallback`) reste ce qui part sur le canal de notification ; il ne
  remplace pas le verdict, et le verdict ne le remplace pas.
- **AC2 (PORTE — test négatif OBLIGATOIRE) : `deadline dépassé ⇒ verdict posté`.** Un
  test qui construit un tour QA à deadline volontairement courte
  (`run_agent_with_deadline`, `agent_loop/mod.rs:3212`, l'entrée de test prévue pour ça),
  le force au dépassement, et **asserte qu'un verdict EST posté sur la PR** — pas
  seulement qu'un message de fallback existe. Le test doit **échouer sur `main`** : le
  pilote colle dans le corps de PR la sortie du test avant et après le fix, et le rouge
  d'avant est ce qui prouve que le test pine bien la classe. Sans lui, la revue bloque.
- **AC3 — pas de double-post.** Un tour qui a déjà posté sa review puis dépasse la
  deadline ne poste pas un second verdict. Le contrôle est le registre existant
  `pr_reviews_posted` (`ToolContext`, `tools/mod.rs:166`), peuplé sur succès de
  `gh pr review` en `skills/builtin_handlers.rs:3055-3062` et déjà keyed
  `session_id → {clé PR}`. Test : même scénario que AC2, mais avec une review déjà
  enregistrée dans le registre ⇒ **zéro** post supplémentaire.
  **Contrôle défensif exigé par l'architecte (Q4) :** si le POST GitHub répond **422**
  (review déjà postée), le moteur l'interprète comme un **succès idempotent**, jamais
  comme un échec de verdict. Le registre en mémoire ne survit pas à un redémarrage ; 422
  est le filet quand il a été perdu. Test : réponse 422 simulée ⇒ le chemin ne journalise
  pas d'échec et ne réessaie pas.
- **AC4 — `run_shell` s'exécute sous le budget de son propre skill.** Un appel
  `run_shell` dans un tour qui charge aussi `build-mika` (300 s) est coupé au budget
  déclaré par `shell-exec` (30 s), pas à 300 s. Test : deux skills aux timeouts
  différents chargés dans le même tour, l'outil du skill le plus court est coupé à son
  propre budget. Ce test aussi doit échouer sur `main` — c'est la mesure M1 rendue
  exécutable.
- **AC5 — la doctrine QA et le moteur disent la même chose.** Après le fix, l'affirmation
  de `skills/bundled/qa-review/system_prompt.md:194` (« run_shell's 30s budget ») est
  vraie. Si l'architecte retient une valeur différente de 30 s pour `shell-exec` chez
  mika-qa, le prompt est corrigé dans le même diff — un plancher documenté qui diverge du
  moteur est ce qui a produit ce ticket.

## Étapes d'implémentation

1. **Reproduire M1 en test avant de toucher au moteur.** Écrire le test d'AC4 contre
   `max_skill_timeout` / le chemin `dispatch.skill_timeout`, le voir rouge sur `main`,
   coller la sortie. C'est le contrôle négatif exigé par
   `feedback_verify_pipeline_passes_without_the_fix`.
2. **Fermer M1 — budget per-outil.** Faire porter à chaque outil skill le
   `effective_timeout` du skill qui le définit, au lieu du maximum du tour. Le point de
   jonction est `ResolvedSkillTool` (qui connaît déjà son skill d'origine) et
   `dispatch.rs:505`. `max_skill_timeout` garde un rôle de plafond de sécurité ou
   disparaît — l'architecte tranche, mais la valeur passée à `execute_skill_tool` doit
   devenir celle du skill propriétaire.
   Vérifier le fan-out : `agent_loop/mod.rs:3382`, `:4291`, `:4956` sont les trois sites
   qui calculent `skill_timeout` (conversation / silent / team) — les trois doivent
   suivre, per `feedback_structural_gate_audit_grep_all_callsites`. Un fix qui n'en
   couvre que deux est un fix qui ment.
3. **Reproduire M2 en test.** Test d'AC2, rouge sur `main`, sortie collée.
4. **Fermer M2 — le filet.** Ajouter à `AgentOutput` (`agent_loop/mod.rs:297-301`) un
   champ disant que le tour s'est terminé par dépassement — **forme tranchée en Q3** ;
   ne PAS remonter `LoopResult` jusqu'aux handlers (interface agent-core/orchestrateur,
   surface trop large pour un p1). Puis, dans `handlers.rs` autour de `:1440` : si
   dépassement **et**
   l'événement d'origine porte une URL de PR **et** `pr_reviews_posted` ne contient pas
   déjà cette PR pour la session ⇒ poster le verdict de secours.
   Ne pas dupliquer l'extraction d'URL de PR : `crates/mika-agent/src/server/verdict.rs`
   parse déjà les événements PR, réutiliser cette surface.
5. **Test d'AC3** (double-post) sur le même harnais que AC2.
6. **Aligner la doctrine (AC5)** — corriger `system_prompt.md:194` si la valeur retenue
   diffère de 30 s.
7. **Compound** — la leçon durable n'est pas « la QA recompilait » mais « un budget
   déclaré par un manifeste n'était pas celui appliqué à l'outil ». Écrire l'entrée sous
   `docs/solutions/best-practices/`.

## Table de disposition au feu

| Ce qui casse | Disposition |
|---|---|
| Le test d'AC4 passe déjà sur `main` | **HALT** — le test ne pine pas M1 ; le rouge-avant est la preuve, pas une formalité |
| Le test d'AC2 passe déjà sur `main` | **HALT** — même raison |
| Un autre test de timeout de skill casse après l'étape 2 | **Investiguer, ne pas neutraliser** — un test qui dépendait du maximum-du-tour encode le défaut ; le corriger explicitement en nommant pourquoi |
| Le filet poste sur une PR déjà reviewée en test | **HALT** — AC3 non tenu, le double-post est une régression pire que le silence |
| Le POST du filet reçoit 422 et le code le traite en échec | **Corriger, ne pas contourner** — AC3 exige l'interprétation « succès idempotent » (exigence architecte Q4) |
| La voie retenue touche `verdict_handler.rs` / `perimeter/` / `pr_merge_with_gate.rs` | **Attendu, pas un bug** — la PR gate sur revue @samidarko (`.github/CODEOWNERS`), Vincent merge à sa revue |

## Hors scope

- **mika#2212** — le trigger qui ne re-dispatche pas sur `synchronize` /
  `ready_for_review`. Maillon voisin, ticket distinct, déjà groomé.
- **La voie (a)** — autoriser la QA à lire la CI. Renversement de doctrine de revue
  (`system_prompt.md:45`, mika#2264) ; si elle doit être ouverte, elle mérite son propre
  ticket et son propre arbitrage.
- **Le réglage de l'enveloppe agent** (`MIKA_AGENT_TOTAL_TIMEOUT_SECS`, mika#2189). Ce
  plan ne change aucune valeur d'enveloppe ; il fait tenir aux outils le budget qu'ils
  déclarent, et fait parler le dépassement quand il survient.
