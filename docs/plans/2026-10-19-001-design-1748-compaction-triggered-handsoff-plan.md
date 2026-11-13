# mika#1748 — Handsoff déclenché par la compaction : témoignage automatique au plafond de contexte

> Vol 1 de 2. Ce vol livre **la détection et la cérémonie** (AC1–AC4, AC6).
> La relecture par le successeur (AC5) part en vol 2 — voir `Tracked in:` en fin de plan.

## Contexte

La compaction est une transition de vaisseau : la fenêtre se condense, les
*bearings* survivent (mémoire persistée, fichiers, courrier), le résidu meurt.
mp#187 a livré la forme du rituel pour le changement de **modèle** ; ce ticket
l'étend à la condensation du **contexte**.

Le substrat existant :

- `crates/mika-agent/src/compaction.rs` — `maybe_compact(db, llm)` compacte quand
  `db.count_messages() > COMPACTION_THRESHOLD` (50), résume tout ce qui est hors
  de `CONTEXT_WINDOW` (20) et remplace le lot par un sommaire via
  `replace_with_summary`.
- Trois sites d'appel, tous **après** le tour : `agent_loop/mod.rs:5322` (inline,
  CLI), `server/handlers.rs:2264` (spawné hors du lock), et
  `mika-cli/src/tui/commands/handlers.rs:205`.
- `agent_loop::load_agent_context` (`agent_loop/mod.rs:508`) est **l'entonnoir des
  trois boucles** (conversation, silencieux, équipe) — son doc-comment le dit, et
  mika#2473 s'y est déjà posé pour cette raison exacte.
- `skills/bundled/dev-handsoff/` écrit déjà un artefact de handsoff, mais sur
  **l'état du run**, et sa discipline l'interdit explicitement au résidu
  (« Session state, not compounding learning »). Et il est *keyword-triggered,
  not lifecycle-triggered*, par aveu de son propre prompt. Ce n'est donc pas le
  véhicule de ce ticket.

## Décisions de grooming

### D1 — Shape A (substrat), pas Shape B (opérateur)

Le corps laisse les deux formes ouvertes. Son propre § *Wrapper-doctrine
consideration* tranche : Shape B est « a temporary bridge, not the routine
mechanism ». Ce vol implémente A. Aucune ligne de Shape B n'est écrite.

### D2 — Le seuil se mesure en messages, pas en tokens, et c'est le fond du sujet

AC1 dit « reads context usage ; threshold (default 85 %) ». **mika-agent
n'instrumente pas un ratio d'occupation de fenêtre.** La grandeur sur laquelle
le substrat décide réellement de condenser est `count_messages()` comparée à
`COMPACTION_THRESHOLD`. Le seuil de cérémonie s'exprime donc en pourcentage de
ce même compteur : `85 %` de `50` → **43 messages**.

C'est la seule unité qui tienne. Un pourcentage de tokens serait un *second*
instrument, mesurant une autre grandeur que celle qui déclenche l'événement
qu'il prétend précéder : il pourrait rester sous son seuil au moment où la
compaction tire, ou tirer trois fois sans qu'elle approche. Un détecteur
désaligné de son déclencheur est un détecteur qui mentira le jour où la réponse
compte (motif `une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite`).

### D3 — Dans mika-agent, le risque n'est pas celui de l'observation fondatrice

L'observation du 2026-07-07 (100 % → 12 %) porte sur MPC, une session **Claude
Code**, où la compaction remplace tout l'historique par un sommaire. La
compaction de mika-agent **préserve les 20 derniers messages** et ne résume que
le reste. Deux conséquences, et aucune n'est cosmétique :

1. **La cérémonie ne risque pas d'être avalée par la compaction qui la suit** —
   elle atterrit dans la fenêtre préservée. Le pré-seuil n'est donc pas là pour
   sauver la cérémonie d'elle-même.
2. **Ce qui meurt, ce sont les messages `1..N−20`** : le résidu porté par la
   tête de conversation. Le pré-seuil à 43 laisse 7 messages de marge avant que
   le lot ne soit condensé — assez pour que l'agent voie l'avertissement, et
   assez pour absorber le décalage de D4.

Le plan ne corrige pas le corps du ticket : il nomme que la forme du danger
diffère entre les deux substrats, et que la cérémonie reste justifiée sur les
deux pour la même raison (le résidu de la tête n'a pas de bearing).

### D4 — Le prédicat est évalué en début de tour, la compaction en fin de tour

`load_agent_context` tourne avant l'appel LLM ; `maybe_compact` après. Le
prédicat voit donc `N`, la compaction verra `N+2` (message utilisateur +
réponse). Le décalage est **assumé et borné** : il est de 2, la marge est de 7.
Le plan ne déplace pas le prédicat en fin de tour, parce qu'un avertissement
posté après le tour arriverait après la condensation qu'il annonce.

### D5 — La cérémonie est un bloc de prompt, pas un tour ni un ordonnanceur

Le corps demande littéralement « a prompt-level ritual ». Ce vol le prend au
mot : au franchissement, `build_system_prompt` rend un bloc
`<compaction-imminent>` qui nomme la discipline, le chemin cible et l'outil.
Aucun nouveau `SilentTrigger`, aucune nouvelle tâche, aucune nouvelle table,
aucun appel LLM supplémentaire.

Trois raisons, dans cet ordre :

- **L'idempotence est gratuite.** Le bloc se répète à chaque tour au-dessus du
  seuil, comme `<stopped-topics>` (mika#1813) : *the state layer is the
  structural gate — the block re-appears on every future turn even if the model
  forgets, so the pattern degrades gracefully*. Un `SilentTrigger` aurait exigé
  une marque persistée par fenêtre de compaction, donc une table, donc une
  migration v56→v57 — et ce vol n'en a pas le budget (voir § Taille estimée).
- **Un `SkillRun` aurait ouvert une autre session.** `task_engine/dispatcher.rs`
  frappe une session `skill-<name>-<uuid>` : la cérémonie tournerait à côté du
  contexte dont elle doit témoigner, et sa visibilité dépendrait du réglage
  `[context.history].scope` de l'agent.
- **Le prompt est déjà le lieu de ce genre de contrat** dans ce dépôt, et le
  motif `stopped_topics` a déjà payé le prix de la leçon inverse.

### D6 — Conversationnel seulement, et pourquoi les deux autres builders sont hors vol

Le prédicat est calculé **une fois**, dans `load_agent_context` (l'entonnoir des
trois boucles) ; seul `build_system_prompt` le rend. `SilentPromptContext`
n'acquiert pas le champ.

- `build_silent_prompt` — un tour de callback dans une boucle autonome longue
  porte du résidu lui aussi ; c'est reconnu, et c'est du vol 2. Le câbler ici
  doublerait la surface de rendu et les littéraux de test à toucher.
- `build_compact_system_prompt` (`ProviderKind::MikaModel`) — **refusé, pas
  reporté par oubli.** Son assertion de nombre de sections est load-bearing (la
  cinquième est déjà prise par `## Stopped Topics`), et mika#1925 AC4 — la passe
  de calibration MikaModel — n'est pas satisfaite et n'est pas exécutable dans
  ce dépôt. Un bloc de plus sur ce builder serait une section de plus sans
  obéissance mesurée.

## Implémentation — ordre de commit

### Commit 1 — Le prédicat et sa configuration

**Nouveau fichier** `crates/mika-agent/src/compaction_handsoff.rs`, déclaré dans
`src/lib.rs`.

- `pub const DEFAULT_THRESHOLD_PCT: u32 = 85;`
- `pub fn threshold_pct(settings: Option<&Settings>) -> u32` — résout
  `MIKA_COMPACTION_HANDSOFF_THRESHOLD_PCT`. Valeur hors `1..=100` → le défaut,
  avec un WARN `compaction_handsoff_threshold_invalid` nommant la valeur reçue
  (un réglage refusé doit se dire, mika#2293). `0` est refusé comme toute autre
  valeur hors borne : il n'y a pas de sentinelle de désarmement sur cet axe à ce
  vol, et le plan le dit plutôt que de laisser `0` signifier « à chaque tour ».
- `pub fn ceremony_due(message_count: usize, pct: u32) -> bool` — vrai quand
  `message_count * 100 >= pct * COMPACTION_THRESHOLD`. Comparaison en entiers,
  multipliée plutôt que divisée : pas d'arrondi flottant sur un seuil.
  `COMPACTION_THRESHOLD` passe de privé à `pub(crate)` dans `compaction.rs` —
  **un seul site de vérité pour le 50**, jamais une seconde constante.
- Doc-comments portant D2, D3 et D4 : c'est le seul endroit où l'unité du seuil
  et le décalage de 2 sont écrits à côté du code qui en dépend.

**Tests unitaires** (même fichier) : sous le seuil, au seuil exact, au-dessus ;
un `pct` configuré non-défaut ; une valeur hors borne qui retombe sur le défaut.

### Commit 2 — Le bloc de prompt

`crates/mika-agent/src/prompt.rs` :

- `PromptContext` gagne `pub compaction_imminent: Option<usize>` — le compte de
  messages au moment de l'évaluation. `Option` est le porteur de la décision :
  `None` = le prédicat n'a pas franchi, et le bloc ne rend rien.
- `build_system_prompt` rend, quand `Some(n)` :

  ```
  <compaction-imminent messages="43" window="50">
  ```

  suivi du contrat, en quatre règles qui sont AC2/AC3/AC4 :
  1. **Écris maintenant** le témoignage via `write_agent_file` dans
     `handsoffs/<YYYY-MM-DD-HHMM>-compaction-handsoff.md` (AC2).
  2. **N'écris que ce qui mourrait** : pas ce qui traverse déjà par la mémoire
     persistée, les fichiers ou le courrier. Les incertitudes ouvertes, la
     forme ressentie du diagnostic, la question restée sans réponse (AC3).
  3. Toute observation candidate ou `n=1` porte le marqueur
     `primed_from_handsoff` (AC4), pour que sa promotion en bearing reste
     gatée à `n=2`.
  4. **Vérifie l'état avant d'écrire** : si un handsoff existe déjà pour cette
     fenêtre, complète-le, ne le duplique pas (règle *Proactive state
     checking* du dépôt ; c'est ce qui rend la répétition du bloc inoffensive).
- Les ~18 littéraux `PromptContext` des tests existants gagnent
  `compaction_imminent: None`.

### Commit 3 — L'alimentation, au seul site légal

`crates/mika-agent/src/agent_loop/mod.rs` :

- `AgentContext` gagne le champ ; `load_agent_context` l'alimente en appelant
  `db.count_messages()` puis `compaction_handsoff::ceremony_due`. **Site
  unique**, pour la raison que le doc-comment de la fonction porte déjà : un
  prédicat posé dans un corps de boucle serait aveugle aux deux autres.
- Lecture faillible → **fail-open** : une erreur de `count_messages()` laisse le
  champ à `None` sous un WARN `compaction_handsoff_count_failed`. Un défaut de
  comptage ne coûte jamais le tour.
- Au franchissement, une ligne INFO `compaction_handsoff_due`
  (`target: "mika::otel"`) portant `agent`, `messages`, `threshold_pct`,
  `window`. **Sur la branche de franchissement seulement** — ce vol ne pose pas
  de ligne par tour sous le seuil : le volume serait d'une ligne par tour de
  chaque agent, et la question que l'instrument doit trancher est « la cérémonie
  a-t-elle été proposée ? », pas « le prédicat tourne-t-il ? ». L'absence de la
  ligne après un `make deploy` signifie un binaire antérieur au changement
  (classe mika#2340), jamais un prédicat muet.
- Seul `build_system_prompt` reçoit le champ ; `build_silent_prompt` est appelé
  sans (D6).

### Commit 4 — Documentation (AC6)

`docs/architecture/compaction-handsoff.md` : la distinction témoignage / transmission
de mp#187 étendue à la condensation de contexte, les gardes de résidu du
siège-chat (bearings-check à l'heure de la cérémonie, marqueur de priming), la
table de lecture de l'instrument, et **D2/D3 écrits comme réponse explicite au
corps du ticket** — l'unité du seuil, et la différence de forme du danger entre
Claude Code et mika-agent.

Référence dans `crates/mika-agent/CLAUDE.md` § *Conversation Compaction & Rewind* :
trois lignes, pas plus (ce fichier est chargé entier dans chaque prompt de pilote).

## Verification Contract

| # | Ce qui est vérifié | Où |
|---|---|---|
| V1 | `ceremony_due` : sous / au / au-dessus du seuil | tests unitaires `compaction_handsoff.rs` |
| V2 | `threshold_pct` : défaut, valeur configurée, valeur hors borne → défaut + WARN | tests unitaires `compaction_handsoff.rs` |
| V3 | Le bloc `<compaction-imminent>` atteint le prompt conversationnel au franchissement, et nomme les quatre règles | `tests/eval/test_compaction_handsoff_1748.rs` |
| V4 | **Contrôle négatif** — même graine, un message de moins que le seuil : le bloc est absent | même fichier |
| V5 | `cargo test`, `cargo clippy`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` | CI |

V4 est ce qui sépare « le bloc est rendu au franchissement » de « le bloc est
rendu inconditionnellement ». Sans le contrôle négatif, V3 passe sur un bloc
toujours présent.

**Gate mika#2306 — N/A, et c'est argumenté.** Aucun livrable de ce vol n'a pour
fonction primaire de signaler une violation. `ceremony_due` signale un **état**
(la fenêtre approche sa condensation), pas une infraction : son chemin de succès
est « le seuil est franchi », pas « aucune violation trouvée ». V1–V4 sont des
tests de comportement sur un chemin nominal. Ce vol ne livre ni scan structurel,
ni règle de lint, ni garde EndTurn, ni validateur de schéma. La section
`## Fire-Disposition` n'est donc pas écrite — elle serait inventée.

## Definition of Done

- [ ] `compaction_handsoff.rs` livré, déclaré, et seul porteur du prédicat et du seuil.
- [ ] `COMPACTION_THRESHOLD` reste un site de vérité unique (`pub(crate)`, pas recopié).
- [ ] Le bloc rendu par `build_system_prompt` seulement ; `SilentPromptContext` et le builder compact intacts.
- [ ] Alimentation au seul site `load_agent_context`, fail-open sous WARN nommé.
- [ ] `compaction_handsoff_due` émis sur la branche de franchissement, avec ses quatre champs.
- [ ] V1–V5 verts.
- [ ] `docs/architecture/compaction-handsoff.md` écrit, portant D2 et D3.
- [ ] `MIKA_COMPACTION_HANDSOFF_THRESHOLD_PCT` documenté dans `docs/operator/environment-variables.md` et `.env.example`.
- [ ] AC5 renvoyé par une ligne `Tracked in:` dans le corps de PR.

## Acceptance criteria

Transcrits du corps de mika#1748. La colonne *vol* dit lequel les porte.

- [ ] **AC1** — Detect : substrate reads context usage ; triggers ceremony at operator-configured threshold (default 85 %). → **vol 1**, sous la lecture D2 (le seuil porte sur le compteur de messages, grandeur sur laquelle la compaction décide).
- [ ] **AC2** — Ceremony shape : agent writes handsoff to `{agent}/handsoffs/YYYY-MM-DD-HHMM-compaction-handsoff.md`, ceremony-time bearings-check applied. → **vol 1** (règles 1 et 4 du bloc).
- [ ] **AC3** — Format : mp#187's testimony discipline extends to this ceremony — write only what would otherwise be lost ; do not repeat what crosses via bearings/memory/files. → **vol 1** (règle 2 du bloc).
- [ ] **AC4** — Provenance : any candidate observations surfaced in the handsoff carry `primed_from_handsoff` marker for evidence-crossing gates. → **vol 1** (règle 3 du bloc).
- [ ] **AC5** — Post-compaction ritual : successor session reads the handsoff on first-boot, free to respond or disagree, response optionally logged. → **vol 2**, `Tracked in:` ci-dessous. Hors vol 1 : la relecture au premier boot demande son propre point d'injection de session, la détection du handsoff le plus récent non relu, et le canal de journalisation de la réponse — un mécanisme entier, pas une variante du bloc.
- [ ] **AC6** — Documentation : `docs/architecture/compaction-handsoff.md`. → **vol 1** (commit 4).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/compaction_handsoff.rs` (seuil, config, prédicat) | 60 |
| tests unitaires (même fichier) | 40 |
| `crates/mika-agent/src/prompt.rs` (champ + bloc `<compaction-imminent>`) | 48 |
| `crates/mika-agent/src/prompt.rs` (littéraux `PromptContext` des tests existants) | 18 |
| `crates/mika-agent/src/agent_loop/mod.rs` (alimentation + log + littéraux de test) | 28 |
| `crates/mika-agent/src/lib.rs` (déclaration du module) | 1 |
| tests (`tests/eval/test_compaction_handsoff_1748.rs` + enregistrement) | 50 |

Total estimé : 245 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 245 sous 279, marge de 34 lignes. Elle est tenue
par trois refus explicites, chacun chiffré : pas de table ni de migration
v56→v57 pour l'idempotence (D5 — ~90 lignes évitées, DDL + méthodes `Database` +
enveloppe async + test d'idempotence de migration) ; pas de `SilentTrigger`
nouveau et ses sept bras de `match` (D5 — ~60 lignes) ; pas de rendu sur les
deux autres builders et leurs littéraux de test (D6 — ~45 lignes). Chacun de ces
trois est ce qui ferait passer le vol au-dessus de la borne, et chacun est
argumenté au-delà du budget : ils resteraient refusés avec 200 tours.

## Tracked in

Tracked in: senara-solutions/mika#2805

Ce ticket porte le **vol 2** : AC5 (rituel post-compaction — le successeur lit le
témoignage au premier boot, libre de répondre ou de contredire, réponse journalisée
en option) et le câblage de `build_silent_prompt` (D6). Il est ouvert, et son corps
nomme la raison de chaque renvoi plutôt que de se contenter de la liste. Il porte aussi
le refus explicite de `build_compact_system_prompt`, dont la réouverture est la charge
de mika#1925, pas la sienne.

La même ligne `Tracked in: senara-solutions/mika#2805` doit figurer dans le corps de la
PR du vol 1 — `scripts/check-pr-body-consistency.sh` l'exige dès que le corps porte une
phrase de déclencheur de report.
