---
title: "mika#2786 — un frein sur les retenues d'ancrage, et la tolérance du préfixe gras"
type: fix
status: active
date: 2026-10-18
issue: senara-solutions/mika#2786
---

# mika#2786 — un frein sur les retenues d'ancrage, et la tolérance du préfixe gras

## Pourquoi

La garde d'ancrage de mika#2037 retient, en fail-closed, une passe d'architecte dont
les lignes `A1:`…`A3:` ne citent pas le brief du tour. Le 2026-10-18, elle a retenu
trois passes de suite sur mika#2762 et deux sur mika#2780. Chaque retenue coûte un
pilote de groom (1,24 $ à 3,35 $ mesurés dans `tasks.result`) et un tour d'architecte,
et rien n'empêche la suivante.

### Ce que la base dit du rebouclage (`~/.mika/data/mika.db`, 2026-10-18)

| heure (Z) | qui a dispatché le groom | passe d'arch | issue |
|---|---|---|---|
| 18:03:57 | webhook `ready` (`ready_label_outcome=dispatched`) | `f167ff53` | retenue `QuoteNotInBrief` |
| 18:30:55 | **mika-dev**, dans un tour de la tâche `1936fc26` (`tool_calls`) | `49ffa230` | retenue `QuoteNotInBrief` |
| 18:49:57 | **mika-dev**, `forced_dispatch_consent=allowed_consented` | `8c9da828` | retenue `NoAnchorLine` |
| 19:00:06 | Phase 2 de l'auto-pull (`stuck_ready_direct_dispatch`) | `1fa87f8b` | `Outcome: PLAN_GROOMED` |

Deux des trois passes retenues ne viennent **pas** de l'auto-pull : mika-dev relit le
callback d'ESCALATE comme un succès (« grooming COMPLETE », 18:15:11 ; « Re-dispatching
grooming to push branch », 18:36:54) et relance le groom lui-même. Le budget de
re-drive de mika#2020 ne les compte pas : il n'incrémente qu'au re-drive de la
Phase 2 (`auto_pull_stats.redrive_count` = 3 à 19:00:11). Le gate mika#2545, qui
refuse un re-dispatch de groom après ESCALATE, ne les a pas tenues non plus : il ne
mord que sans `originating_message`, et le dernier callback de groom lu à 18:30 était
le lanceur différé `long_running:run_claude_pilot:deferred` de 18:20:02 (« deferred
dispatch slot freed »), qui ne porte aucun `Outcome: ESCALATE`.

**Le seul site que tous ces chemins traversent est la frontière d'outil du dispatch de
groom**, dans `crates/mika-agent/src/skills/executor.rs`, là où mika#2545 et le gate de
consentement mika#2772 se tiennent déjà. Un frein placé dans la Phase 2 ne voit pas
les relances de mika-dev ; c'est pour cela qu'il ne va pas là.

### Ce que le rejeu dit de la convergence

Le matcher de `review_anchor.rs`, rejoué hors ligne sur les briefs et réponses réelles
(`messages`, `llm_calls`) :

| passe | réponse | ancres trouvées / valides | avec `**A1:**` toléré |
|---|---|---|---|
| `8c9da828` (#2762) | 1re | 0 / 0 | 4 / 2 |
| `8c9da828` (#2762) | après re-prompt | 0 / 0 | **5 / 3 → Satisfied** |
| `49ffa230` (#2762) | les deux | 3 / 2 | inchangé |
| `a7f1ccad` (#2780) | les trois | 3 / 2 | inchangé |
| `f167ff53` (#2762) | 1re / 2e | 3 / 1, 3 / 0 | inchangé |

La troisième passe de #2762 citait le brief verbatim, à trois endroits disjoints ; elle
a été retenue parce que ses préfixes étaient en gras. Dans trois des quatre autres
réponses, il manque **une** ancre, toujours la première, qui cite la phrase d'ouverture
du ticket lu par `gh_read`.

## Décisions

### D1 — Le frein : un gate de plus à la frontière d'outil

Après **N retenues d'ancrage consécutives** sur le même ticket, tout dispatch de groom
qui n'est pas une demande explicite de l'opérateur est refusé, sur une ligne nommée.

- **Ce qui compte comme une retenue** : un callback de groom terminal dont le `result`
  porte une ligne commençant par `Engine reason: ` et contenant le marqueur
  `REVIEW_ANCHOR_ENGINE_FINDING_MARKER` (`agent_loop/mod.rs`). `dispatch-lib.sh`
  (`_escalate_groom`) écrit exactement cette ligne quand l'ESCALATE vient du moteur.
  Le marqueur est partagé par constante, pas recopié.
- **Ce qui casse la série** : un callback dont le `result` porte une ligne
  `Outcome: …` et pas de retenue (un `PLAN_GROOMED`, un vrai ESCALATE d'architecte, un
  `PIPELINE_INCOMPLETE`).
- **Ce qui est neutre** : un callback sans ligne `Outcome:` — le lanceur différé
  (« deferred dispatch slot freed ») en est le cas mesuré. Le compter comme une rupture
  aurait ramené la série de #2762 à 2 au lieu de 3, et c'est la même confusion qui a
  aveuglé mika#2545 à 18:30.
- **La levée** : une `originating_message` qui est une intention de grooming
  explicite (`webhook_dispatch::is_grooming_intent_message` sur le texte masqué par
  `mask_cited_branch`, le prédicat déjà utilisé par mika#2772). C'est le geste
  `mika ask --agent mika-dev "groom <owner/repo>#<n>"`. Reposer `ready` ne lève pas
  le frein : le moteur ne distingue pas un `ready` reposé par l'opérateur d'un
  `ready` reposé par mika-dev, et c'est mika-dev qui a relancé deux fois sur trois.
  Un groom qui converge (`PLAN_GROOMED`) remet la série à zéro sans geste.
- **N = 3**, réglable par `MIKA_GROOM_ANCHOR_BRAKE_MAX`, `0` désarme (même contrat
  que `MIKA_AUTO_PULL_MAX_REDRIVES`). Mesure sur toute la base : séries maximales de
  1 (#2194, #2516, #2575, #2617, #2716), 2 (#2752 puis convergence ; #2780) et 3
  (#2762, convergence à la 4e). Trois borne la dépense à trois pilotes, comme
  `MAX_REDRIVES_DEFAULT` et `CIRCUIT_BREAKER_THRESHOLD`. **Coût nommé** : sur
  l'historique, ce frein aurait arrêté #2762 avant sa 4e passe, qui a convergé ; un
  geste `groom` de l'opérateur l'aurait relancée. D2 rend ce cas moins probable : la
  3e retenue de #2762 aurait été acceptée.
- **Fail-open sur base illisible**, avec une ligne WARN nommée. Le frein borne une
  dépense ; ce n'est pas un gate de sûreté. Un terme qu'on ne peut pas lire n'est
  pas satisfait (motif mika#2277), donc il ne mord pas.
- **La levée est écrite et borne la série** (ajout de la revue). Une demande
  explicite qui traverse un frein armé écrit `groom_anchor_brake_lifted` ; le frein
  ne compte que les retenues postérieures à la dernière levée. Sans cela, la levée
  se perdrait dans la file différée : le rejeu d'un dispatch garé part d'un tour
  silencieux sans `originating_message`. Les lanceurs différés sont exclus du
  lecteur à la source, pour qu'une pile de wrappers ne pousse pas les retenues
  hors de la fenêtre lue.
- **Pas de label, pas de commentaire posé par ce gate.** Le ticket garde `ready` ; la
  Phase 2 continue de le re-driver, chaque re-drive est refusé ici sans pilote, et au
  troisième le budget de mika#2020 l'abandonne avec `operator-review`, `ready` retiré
  et un commentaire. Le chemin visible par l'opérateur existe déjà ; le dupliquer
  demanderait un jeton d'écriture de labels (mika#2228) que l'exécuteur n'a pas.
- **Nom unique** `groom_anchor_withheld_brake`, journal WARN + ligne d'audit
  (`tool_name`), écrit à un seul site.

### D2 — La convergence : tolérer `**A1:**`, sans toucher au verbatim

`verify_review_anchors` retire les `*` de tête d'une ligne avant de chercher le préfixe
déclaré. Le contenu, lui, passe par `normalize_for_anchor_match` comme avant, qui
retirait déjà `*` des deux côtés. La comparaison verbatim, la longueur minimale, la
dispersion et le dédoublonnage sont inchangés.

Pistes écartées :

- **Re-prompt correctif ciblé** (nommer l'ancre qui échoue et le fragment le plus
  proche). C'est la piste qui vise les trois autres cas, mais c'est un pari sur
  l'obéissance du modèle, qu'aucun test hors ligne ne mesure : le re-prompt actuel
  nomme déjà le `miss_reason`, et `a7f1ccad` a rendu trois fois la même ancre
  fautive. Elle demande une mesure avant/après sur des passes réelles. Ticket de
  suite, avec la mesure « une ancre sur trois, toujours l'ouverture du ticket ».
- **Consigne au prompt de `mika-arch-groom-ticket`.** La consigne existe (*What does
  NOT satisfy the contract*, mika#2786) et le modèle ne la tient pas ; une consigne de
  plus est le remède que `feedback_prompt_enforcement_fragile` déconseille.
- **Changer de modèle** : interdit par le brief (mika#1190).

## Unités

### U1 — Le marqueur et le lecteur de série

- `agent_loop/mod.rs` : `REVIEW_ANCHOR_ENGINE_FINDING_MARKER` passe en `pub(crate)`.
- `db/tasks.rs` : `recent_groom_results_for_issue(agent_id, issue_url, limit)` —
  même jointure et même filtre que `latest_groom_verdict_for_issue`, `LIMIT ?`, ordre
  décroissant. Wrapper dans `async_db.rs`.
- `skills/executor.rs` : fonction pure `anchor_withheld_streak(&[Option<String>]) ->
  usize` (règles de D1), parse pur `parse_groom_anchor_brake_max`.

### U2 — Le gate

- `skills/executor.rs`, juste après le gate mika#2545 : classe `groom`, sujet résolu
  depuis le prompt, pas d'intention explicite, `max > 0`, série `>= max` → refus JSON
  `dispatch_groom_anchor_brake` avec la levée nommée, WARN + audit
  `groom_anchor_withheld_brake`, `record_dispatch_rejection`.

### U3 — La tolérance de forme

- `agent_loop/review_anchor.rs` : `trim_start_matches('*')` avant `strip_prefix`.

### U4 — Documentation

- `docs/operator/environment-variables.md` et `.env.example` :
  `MIKA_GROOM_ANCHOR_BRAKE_MAX`.
- Le doc-comment de `review_anchor.rs` nomme la tolérance et sa mesure.

## Vérification

- U1 : série = 3 sur `[W, neutre, W, W]` ; = 0 quand le plus récent est `PLAN_GROOMED` ;
  un `PLAN_GROOMED` sous trois `W` récents n'est pas atteint ; `NULL` est neutre.
- U2 : **contrôle positif** — trois callbacks retenus, dispatch de groom sans
  intention explicite → refusé avec `dispatch_groom_anchor_brake` et une ligne
  d'audit. **Contrôles négatifs** — deux retenues seulement → pas de refus du frein ;
  trois retenues mais `originating_message = "groom mika#N"` → pas de refus du frein ;
  `max = 0` → pas de refus.
- U3 : `**A1:** "<quote>"` ×3 sur trois régions → `Satisfied` ; `**A1:** "<paraphrase>"`
  → `QuoteNotInBrief` (la tolérance ne relâche pas le verbatim) ; la forme sans gras
  reste `Satisfied`.
- `cargo test -p mika-agent`, `cargo test -p mika-common mika2398`, `cargo clippy`,
  les lints du dépôt, `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

Le frein et la tolérance sont livrés avec leurs tests, la variable est documentée, le
ticket de suite pour le re-prompt ciblé est ouvert avant la PR.

## Acceptance criteria

- [ ] Après 3 retenues d'ancrage consécutives sur un ticket, un dispatch de groom sans demande explicite est refusé sous le nom `groom_anchor_withheld_brake` (journal + audit).
- [ ] Une demande `groom <ref>` explicite traverse le frein ; deux retenues ne le déclenchent pas ; `MIKA_GROOM_ANCHOR_BRAKE_MAX=0` le désarme.
- [ ] Un callback sans ligne `Outcome:` ne casse pas la série ; un `PLAN_GROOMED` la remet à zéro.
- [ ] `**A1:**` est reconnu comme ligne d'ancrage, et une paraphrase en gras reste refusée.
- [ ] La garde de mika#2037 reste fail-closed et verbatim ; aucun modèle n'est changé.
