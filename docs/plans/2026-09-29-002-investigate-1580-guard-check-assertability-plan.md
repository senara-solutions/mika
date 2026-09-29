---
title: "mika#1580 — Assertabilité de la vérification de garde à la promotion"
type: investigate
issue: senara-solutions/mika#1580
branch: investigate/1580/skills-guard-aware-promotion-check
date: 2026-09-29
status: planned
---

# mika#1580 — Quelle fraction de « ne fragilise pas la chaîne de gardes » est mécaniquement assertable

## Issue

https://github.com/senara-solutions/mika/issues/1580

## Résumé

Spike. Le livrable est **un document**, jamais une implémentation de gate de
promotion. Le document partitionne la question « un skill écrit par l'agent
fragilise-t-il la chaîne de gardes ? » en Q1 (mécaniquement vérifiable) /
Q2 (structurellement invérifiable) / Q3 (revue de contenu jugée par LLM), puis
tranche un go/no-go sur l'ouverture du nudge mika#1583 aux sièges de la boucle
autonome (mika-dev, mika-qa, mika-arch).

Le plan ci-dessous ne rédige pas le finding : il fixe **ce qui doit être établi
par lecture de code**, **dans quel ordre**, et **quelles haltes** s'appliquent
quand une prémisse du ticket se révèle fausse.

---

## Ce que la lecture du code déplace dans le ticket

C'est le premier livrable du spike, et il conditionne tout le reste. Le ticket a
été rédigé et groomé le **2026-06-26** ; ses trois sous-issues substrat
(#1582/#1583/#1584) ont livré depuis. Quatre constats de lecture, à
**re-vérifier** au moment de la rédaction plutôt qu'à recopier d'ici :

**C1 — Le ticket pose Q1 sur la surface du *manifeste*, pas sur celle du
*producteur*.** `skill_manage` (`crates/mika-agent/src/tools/skill_manage.rs`)
n'écrit pas un `skill.toml` fourni par le modèle : il en **génère** un depuis un
gabarit fermé, `write_skill_files` (l. 471). Ce gabarit émet `[skill]`
(`name`, `description`, `version`, `always_on`), `[triggers] keywords`,
`[dependencies]`, plus un `system_prompt.md`. Rien d'autre. Le schéma d'entrée de
l'outil (l. 37–71) est lui aussi fermé : six champs.

**C2 — Trois des quatre formes énumérées en Q1 sont donc inatteignables par
construction**, et non « à vérifier » :

| forme Q1 du ticket | champ de manifeste visé | atteignable par `skill_manage` ? |
|---|---|---|
| a. rétrécissement de `required_tools` | `[constraints]` | **non** — la section n'est jamais émise |
| b. contrat `[output]` plus étroit | `[output]` | **non** — idem |
| c. `keywords` qui supplante une skill de grounding always-on | `[triggers] keywords` | **oui** |
| d. collision de nom d'outil via `tools.json` (classe mika#1326) | `tools.json` | **non** — aucun `tools.json` n'est écrit |

**C3 — Deux surfaces réellement atteignables ne figurent pas dans l'énumération
du ticket**, et une troisième est un chemin plutôt qu'un champ :

- `always_on = true` — le modèle peut se déclarer toujours actif, ce qui change
  la composition du prompt de **chaque** tour, pas seulement des tours appariés
  par mot-clé.
- `dependencies` — écrites `{ source = "sibling" }`, elles tirent d'autres skills
  dans la résolution. Leur effet sur la surface résolue est à établir.
- `action = "update"` sur un nom **bundled**. `handle_update` (l. 262) fait
  `remove_dir_all(&skill_dir)` puis `rename` (l. 339) sur
  `<home>/skills/<name>`, **sans discriminer** bundled et authored. Or les skills
  bundled autorisés y sont des **symlinks** posés par
  `materialize_agent_skill_links` (`bundled_skills.rs:788`), et le mode
  `--copy-managed` (`is_copy_managed`, l. 1027) en fait au contraire de vrais
  répertoires. Les deux cas n'ont aucune raison de se comporter pareil sous
  `rename`, et la ligne `skill_overrides … lifecycle_state = 'staged'` qui suit
  **évince** le skill au prochain `apply_overrides` (`skills/mod.rs:747`). Un
  skill de garde évincé est un affaiblissement de chaîne d'une nature que le
  ticket n'anticipe pas.

**C4 — Le chiffre « 11 EndTurn guards » du ticket est périmé.** L'AC2 exige
l'énumération « at finding-time » : le compte se rétablit **par lecture** de
`agent_loop/mod.rs` et des `[output]` déclarés dans `skills/bundled/*/skill.toml`,
jamais par recopie du ticket ni de `CLAUDE.md` (qui en annonce 12 puis décrit
des gardes 5d/5e/5f/5g ajoutées après).

**Conséquence sur la forme de la réponse.** Si C1–C2 tiennent à la
re-vérification, la réponse au ticket n'est pas « telle fraction reste à
automatiser » mais : *la fraction mécaniquement assertable l'est déjà, et pas par
un vérificateur de promotion — par le générateur qui refuse d'émettre les
sections dangereuses.* Le poids bascule alors sur Q2 et sur les formes C3, et la
recommandation d'AC4/AC5 doit être écrite depuis **ce** constat, pas depuis celui
du ticket.

**Halte H0.** Si la re-vérification **infirme** C1 ou C2 — une section
`[constraints]` ou `[output]` émise par un chemin que cette lecture n'a pas vu,
un second producteur de skills authored —, le finding se rédige sur la surface
réellement observée et le dit explicitement. Aucune ligne de ce plan n'autorise
à recopier C1–C3 sans les avoir revus à HEAD.

---

## Requirements

- **R1 — Surface du producteur.** Établir par lecture le jeu **fermé** des champs
  qu'un agent peut peupler via `skill_manage` (`create` et `update`), et le jeu
  des fichiers réellement écrits sur disque. Citer fichier et ligne.
- **R2 — Chaîne de gardes au finding-time.** Énumérer les gardes EndTurn et les
  contraintes de manifeste en vigueur à HEAD : le site d'évaluation dans
  `agent_loop/mod.rs`, les champs de `skills/manifest.rs`
  (`Constraints`, `Output`), et les `[constraints]`/`[output]` effectivement
  déclarés par les skills bundled. Le compte est un résultat de lecture.
- **R3 — Q1, ensemble fermé.** Croiser R1 × R2 : pour chaque forme
  d'affaiblissement concevable, dire si elle est **atteignable** par le
  producteur, et si oui, si elle est **détectable** mécaniquement. Une forme
  inatteignable est nommée comme telle avec le site qui la rend inatteignable —
  elle ne disparaît pas de l'énumération, sans quoi la fermeture de l'ensemble
  n'est pas lisible.
- **R4 — Conception de test de régression, par forme.** Pour chaque forme
  *atteignable* de R3, décrire le test qui la détecterait : son site, sa donnée
  d'entrée, son contrôle négatif. **Conception, pas implémentation** (hors
  périmètre du ticket) — à l'exception du prototype désarmé du § Fire-Disposition.
- **R5 — Q2, le résidu.** Nommer ce que le gate ne voit pas, avec des exemples
  concrets tirés de la surface réelle (`system_prompt.md` et `description`, seuls
  champs en prose libre). Ancrer sur
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` et sur la
  mesure du dépôt (neuf récurrences sous enforcement par prompt contre zéro écrit
  à la main, mika#2120). Dire où le jugement opérateur reste porteur.
- **R6 — Q3, la couche intermédiaire.** Relever le langage de critère existant
  dans `skills/bundled/mika-arch-second-review/system_prompt.md` qui recoupe
  « ce skill fragilise une garde », et trancher oui / non / conditionnel. Un
  conditionnel **doit** nommer ce qui le lève : plafond de coût, budget de
  latence, ou seuil de taux de détection — avec la surface qui le mesurerait.
- **R7 — Population réelle du nudge.** Établir quels chemins d'exécution font
  tourner le nudge. Lecture de départ : `agent_loop/skill_nudge.rs` (« silent/team
  turns do not nudge »), branché sur `server/handlers.rs:1611` et
  `server/a2a.rs:343`. Les callbacks (`run_silent_agent`) et les runs d'équipe en
  sont dehors. Dire quelle fraction des tours de mika-dev / mika-qa / mika-arch
  traverse chacun — c'est ce qui décide si « ouvrir le nudge » a une population.
- **R8 — Go / no-go.** Trancher AC5, depuis R3 et R7. Un *go* nomme la forme de
  vérification qui partirait en ticket feat. Un *no-go* nomme l'hypothèse dont le
  changement rouvrirait la question.

---

## Plan d'exécution

### Phase 1 — Re-vérifier la surface du producteur (R1, C1–C3)

1. Lire `crates/mika-agent/src/tools/skill_manage.rs` en entier : schéma
   d'entrée, `handle_create`, `handle_update`, `write_skill_files`, et les
   validateurs importés de `tools/create_skill.rs`
   (`validate_skill_name`, `validate_keywords`, `validate_system_prompt`,
   `validate_dependencies`, `verify_skill_path`).
2. Lire `crates/mika-agent/src/skills/index.rs::validate_skill` — quelles règles
   le pré-valideur applique déjà, et à quel niveau (`Fail` vs `Warn`). Un
   diagnostic `Warn` ne bloque pas : la distinction est porteuse pour Q1.
3. Chercher un **second producteur** de skills authored (`create_skill`,
   chemins CLI/HTTP d'écriture sous `<home>/skills/`). L'absence est un résultat
   à écrire ; sa présence invalide C1.
4. Établir le comportement du chemin `update` sur un nom bundled, dans **les
   deux** topologies (symlink et `--copy-managed`). Par lecture, pas par
   exécution contre l'installation de l'hôte.

**Halte H1 — ne rien exécuter contre `~/.mika`.** Le spike lit du code ; il ne
crée, ne met à jour et ne promeut aucun skill sur l'installation réelle. Si une
vérification empirique paraît nécessaire, elle est **nommée dans le finding comme
sonde à faire tourner par l'opérateur**, avec sa commande — jamais exécutée ici.

### Phase 2 — Relever la chaîne de gardes à HEAD (R2, C4)

5. Énumérer les gardes EndTurn depuis `agent_loop/mod.rs` (les commentaires
   `// Guard(s) …` autour de l. 1415–1447 sont une entrée, pas la liste). Recouper
   avec les noms d'événements `guard.*` réellement émis.
6. Énumérer les champs de contrainte depuis `skills/manifest.rs` : `Constraints`
   (`required_tools`, `required_fetches_for_quoted_resources`) et `Output`
   (`required_suffix_lines`, `required_finding_list_prefixes`,
   `required_review_anchor_prefixes` + ses trois seuils,
   `required_tool_arg_suffixes`).
7. Relever quels skills bundled déclarent effectivement ces sections. C'est ce
   qui distingue « contrainte existante » de « contrainte qu'un skill authored
   pourrait rétrécir » — on ne rétrécit que ce qui existe.
8. Relever les deux autres étages nommés par le ticket : allowlist d'identité
   (`well_known_agents.rs`, `[skills].allowlist`) et `skill_overrides`
   (éviction lifecycle, `skills/mod.rs:747`).

### Phase 3 — Rédiger Q1 (R3, R4)

9. Écrire le tableau croisé R1 × R2. Trois colonnes : forme, atteignable,
   détectable. Chaque « non » porte le site qui le rend vrai.
10. Pour chaque forme atteignable, écrire la conception de test de R4.
11. Écrire explicitement la propriété de fermeture : *pourquoi cet ensemble est
    fermé* — l'argument est que la surface d'entrée de l'outil est finie et
    énumérée, pas que l'auteur a été exhaustif dans son imagination.

### Phase 4 — Rédiger Q2 (R5)

12. Borner : la prose atteignable est `system_prompt.md` et `description`.
13. Construire deux ou trois exemples d'évasion **sur cette surface-là**, pas sur
    celle qu'un `[constraints]` éditable aurait offerte.
14. Nommer la frontière : ce que le jugement opérateur seul couvre, et pourquoi
    aucun seuil ne la déplace.

### Phase 5 — Rédiger Q3 (R6)

15. Lire `skills/bundled/mika-arch-second-review/system_prompt.md` et relever le
    langage de critère qui recoupe déjà « fragilise une garde ».
16. Poser la question de la duplication : ce que la seconde passe attrape
    **au moment de la PR** contre ce qu'un classifieur attraperait **au moment de
    la promotion** — et noter que le skill authored par `skill_manage` **ne passe
    par aucune PR**, ce qui décide peut-être seul la question.
17. Trancher, avec la condition de levée si conditionnel.

### Phase 6 — Population et go/no-go (R7, R8)

18. Établir la population du nudge par lecture des trois sites (`skill_nudge.rs`,
    `handlers.rs`, `a2a.rs`), puis nommer la **sonde opérateur** qui mesurerait la
    répartition réelle des tours par mode pour les trois agents autonomes
    (`turn_usage`, champ `mode` — Signal O).
19. Écrire le go/no-go. **Halte H2 :** si la population s'avère quasi vide (les
    agents autonomes tournant majoritairement en tour silencieux), c'est un
    *résultat* et il doit être écrit comme tel — « ouvrir le nudge ne fait rien
    d'observable » est une réponse, pas un échec du spike, et c'est la classe
    mika#2205 (un mécanisme silencieusement inerte se lit exactement comme un
    mécanisme sain).

### Phase 7 — Écriture et dépôt

20. Écrire `docs/brainstorms/2026-09-29-guard-check-assertability.md`.
    **Le chemin est fixé par AC1 (`docs/brainstorms/<date>-guard-check-assertability.md`)
    et ne se renomme pas.**
21. Volume : 2000–4000 mots (borne du ticket). En **français**, conformément à la
    trajectoire du dépôt sur les findings substrat récents ; les identifiants, noms
    de symboles, chemins et citations de code restent inchangés.
22. En-tête au format maison des brainstorms existants : titre, `**Date:**`,
    `**Status:**`, puis un *headline* qui pose la réponse avant de l'argumenter
    (modèle : `docs/brainstorms/2026-05-05-session-continuity-across-scope-types-brainstorm.md`).
23. Toute affirmation sur le code porte `fichier:ligne`. Une affirmation qui n'a
    pas été vérifiée à HEAD est ou bien supprimée, ou bien marquée comme
    hypothèse à vérifier — jamais servie au ton de la mesure.

---

## Fire-Disposition

Le plan livre **deux** détecteurs. Chacun porte sa disposition.

### D1 — L'assertion de surface du producteur : option (a), allowlist livrée VIDE

Tout le raisonnement de Q1 repose sur une prémisse d'un seul site : le gabarit
`write_skill_files` n'émet ni `[constraints]`, ni `[output]`, ni `tools.json`.
Le jour où quelqu'un ajoute une section à ce gabarit, **le finding devient faux en
silence** — aucun test existant ne rougit, et le document continue de se lire
comme une mesure.

Donc : un test unitaire dans `crates/mika-agent/src/tools/skill_manage.rs` qui
**fige** le jeu de sections émises par `write_skill_files` — appelle la fonction
sur une entrée représentative et assert que le `skill.toml` produit ne contient
**aucune** des sections de garde, ni le fichier `tools.json`.

- **Disposition (a)** : allowlist nommée des sections de garde tolérées dans la
  sortie du gabarit, **livrée vide**, avec le commentaire qui dit la résolution :
  *quand ce test rougit, on ré-ouvre le finding mika#1580 — on n'ajoute pas
  d'entrée à l'allowlist.* Motif maison : `ACTOR_READING_PREDICATES_ALLOWED`
  (mika#2323), l'allowlist vide de mika#2496 U3.
- **Assertion auto-nettoyante** : le test échoue aussi si l'allowlist cesse d'être
  vide sans qu'une ligne en nomme la raison et le ticket de suivi.
- **Anti-vacuité** : le test assert d'abord que la sortie **contient** `[skill]` et
  `[triggers]`. Sans ce contrôle positif, un gabarit devenu inerte (ou un
  renommage de fonction) rendrait le test vert en ne regardant rien — la classe
  mika#2205 appliquée au détecteur lui-même.
- **Livré armé** : il passe à HEAD, il n'y a aucune violation existante à couvrir.

### D2 — Le prototype de vérification de non-régression Q1 : option (b), DÉSARMÉ

Le ticket autorise « a small Rust prototype … Ephemeral, lives in
`crates/mika-agent/tests/eval/` if used » et met l'implémentation du gate hors
périmètre. Un prototype armé serait cette implémentation par la porte de service.

- **Disposition (b)** : s'il est livré, il l'est sous `#[ignore]`, avec un
  doc-comment qui nomme (i) mika#1580 comme sa provenance, (ii) qu'il est une
  *illustration de conception* et non un gate, (iii) le ticket feat que le
  finding recommande comme condition de son armement.
- **Facultatif, et l'ordre compte** : il n'est écrit que si la Phase 3 conclut
  qu'au moins une forme atteignable est mécaniquement détectable. Si Q1 se referme
  sur « rien à vérifier au-delà de ce que le producteur interdit déjà », le
  prototype ne se construit pas — un détecteur sur une population vide est un
  détecteur dont le silence ne prouve rien, et c'est précisément ce que le finding
  aurait établi.

---

## Verification Contract

| # | Vérification | Comment |
|---|---|---|
| V1 | Le brainstorm existe au chemin exact d'AC1 | `ls docs/brainstorms/2026-09-29-guard-check-assertability.md` |
| V2 | Les trois partitions Q1/Q2/Q3 sont des sections nommées | lecture |
| V3 | Q1 porte le tableau croisé avec la colonne « atteignable » et un site par « non » | lecture |
| V4 | Chaque forme *atteignable* porte sa conception de test | lecture |
| V5 | Q2 nomme la frontière du jugement opérateur | lecture |
| V6 | Q3 rend oui / non / conditionnel, et un conditionnel nomme sa levée | lecture |
| V7 | Le doc se ferme sur un go/no-go explicite, avec sa suite nommée | lecture |
| V8 | Volume dans 2000–4000 mots | `wc -w` |
| V9 | D1 passe, et son allowlist est vide | `cargo test -p mika-agent skill_manage` |
| V10 | D2, s'il existe, est `#[ignore]` | `cargo test -p mika-agent --test eval` ne l'exécute pas |
| V11 | Aucun fichier hors `docs/brainstorms/`, `docs/plans/` et (si D1/D2) leurs sites de test | `git diff --stat main...HEAD` |
| V12 | `cargo fmt --check` et `cargo clippy` propres | commandes |

**V11 est le contrôle négatif du périmètre** : le ticket interdit toute
modification des gardes, des contraintes de manifeste et du schéma
`skill_overrides`. Un diff qui les touche est une violation de périmètre, pas une
amélioration.

---

## Definition of Done

- Le brainstorm est écrit, au chemin d'AC1, dans la borne de volume.
- Les huit requirements R1–R8 sont traités, chacun repérable dans le document.
- Chaque affirmation sur le code porte `fichier:ligne`, vérifié à HEAD.
- D1 est livré armé, allowlist vide, avec son anti-vacuité.
- D2 est livré désarmé ou n'est pas livré, selon la conclusion de Phase 3.
- Aucune garde, contrainte de manifeste ou colonne de schéma n'est modifiée.
- `cargo fmt --check`, `cargo clippy`, `cargo test -p mika-agent` passent.

---

## Acceptance criteria

Transcrites verbatim depuis le corps du ticket.

- **AC1.** `docs/brainstorms/<date>-guard-check-assertability.md` exists and
  explicitly partitions the "doesn't weaken the chain" question into Q1
  (checkable) / Q2 (unchecked / impossible) / Q3 (LLM-judged middle).
- **AC2.** Q1 enumerates the **closed set** of structurally checkable
  guard-weakening shapes given the current guard chain at finding-time. Includes a
  regression test design (not implementation) covering each shape.
- **AC3.** Q2 documents the residual — what the gate *cannot* assert — explicitly.
  Names it as the boundary at which operator judgment remains load-bearing.
- **AC4.** Q3 produces a recommendation: yes/no/conditional on the LLM-judged
  middle layer. If conditional, names what would resolve the conditional (cost
  ceiling, latency budget, hit rate threshold).
- **AC5.** Brainstorm closes with a written go/no-go on opening the nudge to
  autonomous-loop agents (mika-dev/qa/arch). If go, names the assertable check
  shape that would land as a feat ticket. If no-go, names what changed assumption
  would re-open the question.

---

## Hors périmètre, délibérément

- **L'implémentation du gate de promotion.** Le ticket feat se cadre depuis le
  finding, pas ici. D2 en est une illustration désarmée, pas une amorce.
- **Toute modification des gardes EndTurn, des contraintes de manifeste ou du
  schéma `skill_overrides`.** Interdit par le ticket, contrôlé par V11.
- **Le chemin de promotion manuel** (opérateur-comme-vérification, sous-issue 1
  de la milestone). Il ship indépendamment et reste le défaut de Phase 1 quelle
  que soit la conclusion de ce spike.
- **La réparation de ce que le spike découvre.** Si la Phase 1 établit que le
  chemin `update` atteint un skill bundled, c'est un **finding**, avec son ticket
  de suivi nommé dans le document — jamais un correctif glissé dans cette PR. Un
  spike qui corrige en passant ne rend plus le périmètre lisible, et son PR cesse
  d'être revuable comme un document.
- **Toute exécution contre `~/.mika`** (création, mise à jour, promotion de
  skill). Halte H1.

---

## Risques et notes

- **R-a — La rédaction dérive vers la conception du gate.** C'est le risque
  principal d'un spike dont la question est « que pourrait-on vérifier ». La
  discipline est R4 : *conception de test*, jamais code de gate, sauf D2 désarmé.
- **R-b — Recopier C1–C3 sans re-vérifier.** Ce plan est écrit le jour du
  dispatch ; le finding se rédige sur la lecture, pas sur ce plan. H0 nomme la
  conduite si la re-vérification infirme.
- **R-c — Consigne opérateur du grooming de juin.** Le commentaire de première
  passe du 2026-06-26 se ferme sur « **Do NOT label ready** — spike requires
  operator-driven research work (Vincent's seat) ». Le ticket porte aujourd'hui
  `ready` et `dispatch:loop`. Ce plan est écrit pour être exécutable par un
  pilote — le livrable est un document ancré sur du code lisible, ce qu'une
  session dispatchée sait produire. Mais la tension est réelle et **nommée plutôt
  que tue** : si l'architecte ou l'opérateur juge que le siège est le bon
  discriminant, le remède est de retirer `ready` et de reprendre le spike à la
  main, pas d'élargir le plan.
- **R-d — Le finding vieillit.** Sa prémisse centrale est un gabarit d'un seul
  site. D1 est exactement le mécanisme qui rend ce vieillissement bruyant au lieu
  de silencieux, et c'est la raison pour laquelle ce spike livre un détecteur
  alors que son done-condition est un document.
