# mika#2847 — La porte de section heartbeat est registration-only par construction (vol 4 de mika#2456)

**Ticket :** senara-solutions/mika#2847
**Amont :** mika#2456 (le défaut), mika#2841 (vol 2, livré par #2844), mika#2843 (vol 3, livré par #2848)
**Plan du vol 3 :** `docs/plans/2026-10-20-002-feat-2843-task-engine-la-surface-daudit-du-filet-plan.md`
**Branche :** `feat/2847/task-engine-la-porte-de-section`

---

## Contexte

mika#2456 se livre en quatre vols, et les trois premiers sont sur `main` :

| vol | PR | ce qu'il porte |
|---|---|---|
| 1 | #2835 | les sondes de la porte à l'**enregistrement** (`Identity::recurring_tasks_enabled`, blocage dans `ensure_recurring_task_with_policy`) |
| 2 | #2844 | le **filet au tir** — `refuse_recurring_fire_if_agent_disabled`, en tête de `dispatch_run_skill` |
| 3 | #2848 | la **surface d'audit** de ce filet (`agent_recurring_gate`, déduplication 24 h, scan SOLE-WRITER) |
| 4 | *ce vol* | la porte de **section** `heartbeat` est **registration-only** par construction |

Deux portes coexistent et ne se remplacent pas. La porte **niveau-agent** (`enabled`
à la racine d'`identity.toml`, lue par `agent_recurring_tasks_enabled`) coupe
**toutes** les récurrentes de l'agent. La porte de **section** (`[heartbeat] enabled
= false`, lue par `heartbeat_enabled_for_agent`, `task_engine/mod.rs:898`) en coupe
**une**. Le doc-comment de la première dit déjà cette distinction ; rien ne tient la
seconde.

Ce que la seconde a d'inhabituel, et qui est l'objet de ce vol : **elle n'agit qu'à
l'enregistrement.** `dispatch_heartbeat` (`dispatcher.rs:1768`) ne la consulte jamais.
La décision vit en tête de boucle de démarrage, où la branche `else` appelle
`cancel_recurring_task_by_label("heartbeat")` — annuler la ligne, et non filtrer le
tir. Donc « une récurrente `heartbeat` effectivement non tirée sous `[heartbeat]
enabled = false` » n'est pas un fait du **tir** : c'est un fait de
l'**enregistrement**, et la forme qui le démontre est **structurelle**.

### Ce que le report coûte aujourd'hui

Le vol 2 ne tient la séparation « le filet au tir ne décide pas de la porte de
section » que par l'assertion statique de sa sonde V6
(`mika2456_le_filet_laisse_la_porte_de_section_decider`, `dispatcher.rs:5717-5743` :
`heartbeat_enabled_for_agent` rend `false` et le filet ne le consulte pas). Cette
sonde atteste le comportement du **filet**, pas la topologie des lecteurs : un
lecteur ajouté **au tir** la laisserait verte. Jusqu'à ce vol, rien ne rougit si la
porte de section cesse d'être registration-only — et le jour où elle cesse de
l'être, aucune décision ne devient fausse immédiatement, seule la propriété se perd
en silence (classe `grooming_marker`, mika#2158).

### La prémisse d'origine était fausse, et ce vol en corrige une moitié de plus

Le corps de mika#2843 rangeait ce suivi derrière *« la conjonction au tir réel
demande de piloter un `TaskEngine` complet sur une boucle de ticks »*. Le corps de
mika#2847 a déjà réfuté les deux moitiés de cette phrase, et ce plan ne les rouvre
pas : aucun `TaskEngine`, aucune boucle de ticks.

**En revanche le corps de mika#2847 porte à son tour une erreur de fait, mesurée en
préparant ce plan, et c'est la décision de conception de ce vol.** Il affirme que
`heartbeat_enabled_for_agent` a *« exactement un lecteur de production,
`server/mod.rs:1906` »*. Le grep en rend **deux** :

| site | ce qu'il est | la branche `else` |
|---|---|---|
| `crates/mika-agent/src/server/mod.rs:1906` | démarrage du démon mika-spirit, par agent | `cancel_recurring_task_by_label("heartbeat")` (`:1915`) |
| `crates/mika-cli/src/commands/chat.rs:229` | démarrage du TUI CLI, agent courant | `cancel_recurring_task_by_label("heartbeat")` (`:240`) |

Le troisième hit (`dispatcher.rs:5743`) est la sonde V6 du vol 2, sous `mod tests` —
écarté par la troncature de `production_sources()` (premier `#[cfg(test)]` à
`dispatcher.rs:5195`). Le quatrième (`task_engine/mod.rs:898`) est la **définition**.

Conséquence directe sur le périmètre : **un scan « lecteur unique » écrit tel que le
corps le décrit serait rouge à son premier run**, sur `chat.rs`, et son allowlist
naîtrait à une entrée — c'est-à-dire exactement ce que mika#2323 refuse, et un lint
rouge à la naissance se fait désarmer. Le **titre** du ticket, lui, est juste :
*registration-only*. Les deux lecteurs agissent tous deux à l'enregistrement, et
c'est **cette** proposition que ce vol atteste.

La forme livrée est donc celle de mika#2627 : un **recensement** à deux entrées (un
périmètre, pas une allowlist d'exemption — sens de mika#2536), chacune nommée avec
sa raison, **plus** une allowlist née vide pour les non-recensés. La doctrine
mika#2323 porte sur l'**allowlist** ; le recensement est le périmètre que le scan
mesure. Le précédent est explicite : `TESTIMONY_PREDICATE_READERS` (trois entrées) +
`TESTIMONY_PREDICATE_READERS_ALLOWED` (vide).

Un corollaire que l'on ne mesurait pas et que l'on obtient : le corpus de
`production_sources()` marche sur `crates/` **en entier**, donc il voit la CLI — ce
que mika#2670 a dû graver une fois après avoir supposé le contraire. Ici cette borne
n'est pas supposée : `chat.rs` est une entrée **nommée** du recensement, donc un walk
qui se restreindrait à `mika-agent` ferait rougir l'anti-vacuité du scan. La borne
s'auto-atteste, et aucune assertion de périmètre séparée n'est due.

---

## Périmètre — ce que CE vol livre

### D1 — le recensement des lecteurs, dans `canonical_tokens.rs`

`HEARTBEAT_GATE_REGISTRATION_SITES: &[&str]` — les deux chemins ci-dessus, chacun
avec sa raison en doc-comment (« démarrage du démon, par agent » / « démarrage du
TUI, agent courant »). C'est un **périmètre**, et le doc-comment le dit en ces
termes pour que personne ne le lise comme une liste d'exemptions.

`HEARTBEAT_GATE_READER_ALLOWED: &[&str] = &[]` — l'allowlist, **née vide**, avec son
assertion auto-nettoyante (D5). Son doc-comment nomme la résolution attendue : router
la lecture par un site d'enregistrement existant, **ne pas** ajouter d'entrée
(doctrine mika#2201).

### D2 — le scan : aucun lecteur hors du recensement

`heartbeat_gate_reader_sites()` — walk de `production_sources()`, commentaires
retirés par `crate::source_scan::strip_comment_lines`, aiguille
`heartbeat_enabled_for{}agent` **composée à l'exécution** pour que le fichier de scan
ne se dénonce pas lui-même (mika#2496). La ligne de **définition** est écartée par
`!line.contains("fn heartbeat_enabled_for_agent(")` — elle est dans la partie
production de `task_engine/mod.rs` (premier `#[cfg(test)]` à `:944`, définition à
`:898`), donc ce filtre est **nécessaire** et non décoratif.

`mika2847_la_porte_de_section_na_que_des_lecteurs_denregistrement` :

- **anti-vacuité d'abord** — chacune des deux entrées du recensement *est* trouvée,
  sinon le scan vise un nom mort et il est vert pour la mauvaise raison (mika#2103 /
  mika#2205) ;
- **puis les étrangers** — tout lecteur hors recensement et hors allowlist fait
  échouer, avec un message de résolution qui nomme la gestion attendue.

C'est ce second volet qui porte la propriété que la sonde V6 du vol 2 ne tient pas :
`dispatcher.rs` n'est pas au recensement, donc **un lecteur ajouté au tir rougit
ici**. Aucune assertion séparée sur `dispatcher.rs` n'est livrée — elle serait
exactement redondante avec le volet « étrangers », et une garde redondante se fait
supprimer par le premier qui la relit.

### D3 — le volet qui donne son sens au recensement : chaque lecteur annule

Un recensement de **fichiers** ne dit rien du **geste**. Un lecteur déplacé dans la
boucle de tick de `server/mod.rs` resterait vert en D2 : même fichier, même entrée.
Ce que « registration-only » affirme est une **conjonction locale**, et c'est elle
que D3 mesure.

`heartbeat_gate_reads_at_registration(content, needle) -> bool` — pour chaque hit de
l'aiguille, une fenêtre de 25 lignes **sur le texte décommenté** doit porter les
trois termes du geste d'enregistrement :

1. `ensure_recurring_task` — la branche qui enregistre ;
2. `cancel_recurring_task_by_label(` — la branche `else` qui annule ;
3. le littéral `heartbeat`, composé à l'exécution.

Mesure des distances réelles sur l'arbre : 9 lignes (`server/mod.rs` 1906→1915) et
11 lignes (`chat.rs` 229→240). La fenêtre de 25 est large sans être molle ; sur le
texte décommenté elle est plus serrée encore, puisque `strip_comment_lines`
**supprime** les lignes de commentaire.

`mika2847_chaque_lecteur_recense_annule_a_lenregistrement` — les deux sites du
recensement satisfont le prédicat. Son échec dit : la lecture a quitté le site
d'enregistrement, la porte de section n'est plus registration-only.

### D4 — le contrôle de bonne foi du prédicat de D3, et il est dû

`heartbeat_gate_reads_at_registration` est un prédicat **neuf** : contrairement aux
scans SOLE-WRITER du vol 3, il ne peut pas emprunter l'attestation d'un prédicat
partagé (`audit_name_literal_present`, attesté par
`mika2634_the_sole_writer_scan_still_catches_a_second_site`). Un prédicat
d'adjacence non attesté est la forme qui devient inerte en silence, et l'anti-vacuité
de D3 ne le dit pas : elle vérifie que les **propriétaires** passent, pas qu'un
**étranger** serait vu.

`mika2847_le_predicat_dadjacence_voit_une_lecture_hors_enregistrement` — deux
assertions sur des fragments synthétiques, composés dans le test :

- **négative** : une lecture dans une boucle de tick, sans aucune des deux branches
  dans la fenêtre → le prédicat rend `false` ;
- **positive** : la forme réelle, lecture puis `else` d'annulation → `true`.

### D5 — l'assertion auto-nettoyante de l'allowlist

`mika2847_lallowlist_des_lecteurs_est_livree_vide` — le pendant exact de
`mika2456_lallowlist_du_scan_dagent_recurring_gate_est_livree_vide` du vol 3.

### D6 — la sonde comportementale, dans le fichier eval du ticket amont

La décision vit dans la boucle de démarrage de `server/mod.rs`, une fonction qui n'est
pas appelable depuis un test — c'est pourquoi `test_qa_review_reconcile_2334.rs:151`
lit déjà la composition de ce même fichier **en source** plutôt qu'en l'exécutant.
D2 et D3 tiennent cette moitié-là. Ce que la source ne dit pas est ce que la
composition **fait**, et c'est D6.

Dans `crates/mika-agent/tests/eval/test_agent_disabled_recurring_gate_2456.rs`, sur
le harnais déjà livré par les vols 1 et 2 (`test_db`, `agent_home`, `statuses_for`) :

- un helper `home_with_heartbeat(bool)` sur le modèle exact de `home_with_enabled` —
  les deux foyers produits par la **même** fonction, pour que l'écart mesuré soit
  celui de la clé de section et non celui de deux montages divergents ;
- **V8** — sous `[heartbeat] enabled = false` : `heartbeat_enabled_for_agent` rend
  `false`, et `cancel_recurring_task_by_label("heartbeat")` fait passer la ligne
  `recurring_active` préexistante à `cancelled`. Les deux moitiés comptent : refuser
  l'enregistrement sans annuler la ligne existante laisse tirer exactement ce que
  mika#2456 a mesuré ;
- **V8-bis, contrôle négatif** — `identity.toml` sans section `[heartbeat]` :
  résolution `true` (le défaut de `default_heartbeat_enabled`, `prompt.rs:437`), et
  `ensure_recurring_task` enregistre normalement. Sans ce volet, un défaut qui
  casserait tout enregistrement rendrait V8 vert. **C'est la paire qui mesure.**

L'en-tête de module du fichier est mis à jour : il affirme aujourd'hui *« ces sondes
portent sur l'enregistrement ; le filet au tir est le vol 2 ; aucun test ici ne le
couvre »*. Après ce vol, le fichier porte aussi la porte de **section** à
l'enregistrement, et V8 y est nommée comme les sept autres. Laisser l'en-tête en
place en ferait une phrase fausse sur le fichier qu'elle décrit.

**Ni D2/D3 ni D6 ne suffit seul, et c'est dit plutôt que tu.** D6 rejoue la
composition à la main : il atteste que la paire *lecture → annulation* produit bien
l'annulation, mais il ne mesure pas `server/mod.rs`. D2/D3 atteste que
`server/mod.rs` compose bien cette paire, mais pas ce qu'elle produit. Les prendre
l'un pour l'autre serait la classe mika#2205 — deux populations non soustractibles.
C'est la paire qui est l'attestation.

## Périmètre — ce que CE vol ne livre PAS

- **La porte de section `reflection`.** `reflection_cron_for_agent`
  (`task_engine/mod.rs:908`) a la **même** structure dupliquée aux **mêmes** deux
  sites (`server/mod.rs:1917-1929`, `chat.rs:243-257`), branche `else` d'annulation
  comprise. Elle n'est pas dans le périmètre du ticket, et l'y ajouter doublerait le
  volume d'un vol déjà à 91 % de sa borne. Ce n'est pas une omission silencieuse :
  `Tracked in:` — ticket de suivi à ouvrir à la livraison de ce vol, *« la porte de
  section reflection est registration-only par construction »*, qui transpose D1–D3
  à `reflection_cron_for_agent` et pourra réutiliser tel quel le prédicat
  d'adjacence livré ici (en passant le label en paramètre, comme mika#2653 l'a fait
  de `audit_name_literal_present`).
- **L'extraction d'un helper `register_or_cancel_heartbeat(db, home_dir)`** qui
  ramènerait les deux lecteurs à un seul et rendrait la propriété
  comportementalement testable. Tentant — il réaliserait la prémisse littérale du
  corps — mais c'est un refactor d'un chemin de démarrage que le ticket ne demande
  pas, qui laisserait `reflection` asymétrique à côté, et dont la régression se
  paierait au boot de la flotte. Le scan atteste la propriété **sans** y toucher. Si
  le suivi `reflection` est pris, ce helper est la bonne conversation à y avoir —
  pour les deux labels à la fois.
- **Toute extension de la porte de section aux autres `trigger_type`**, et **tout
  remplacement du filet du vol 2 par une mutation de ligne** — inchangés depuis les
  vols 2 et 3 (doctrine mika#2156 : un filet qui mute efface sa propre preuve).
- **Aucune nouvelle requête opérateur, donc aucune ligne dans
  `crates/mika-agent/CLAUDE.md` § *Audit Log*.** Ce vol n'écrit aucune ligne
  `audit_events` : il atteste une topologie de source. Documenter une requête sur une
  population que rien n'écrit est l'instrument qui mente que le vol 3 a refusé
  d'ouvrir avant son heure.

---

## Ordre de commit

Un seul vol, un seul commit de travail — les deux livrables sont indépendants et
aucun ne gate l'autre, mais ils attestent la même proposition et se relisent
ensemble.

1. `crates/mika-agent/src/canonical_tokens.rs` — D1 à D5.
2. `crates/mika-agent/tests/eval/test_agent_disabled_recurring_gate_2456.rs` — D6 +
   l'en-tête de module.

**Plancher de ce vol :** D2 (le scan des étrangers) avec son anti-vacuité, et son
allowlist vide (D5). C'est ce qui rougit le jour où un lecteur apparaît au tir, et
c'est la seule chose que rien ne tient aujourd'hui. D3/D4 resserrent la propriété de
« dans ces fichiers » à « à l'enregistrement » ; D6 l'ancre dans le comportement.

---

## Contrat de vérification

| # | ce qui est mesuré | comment |
|---|---|---|
| V1 | les deux lecteurs de production sont trouvés par le scan | anti-vacuité de `mika2847_la_porte_de_section_na_que_des_lecteurs_denregistrement` |
| V2 | aucun lecteur hors recensement | volet « étrangers » du même test |
| V3 | la lecture est adjacente au geste d'enregistrement, sur les deux sites | `mika2847_chaque_lecteur_recense_annule_a_lenregistrement` |
| V4 | le prédicat d'adjacence mord encore | `mika2847_le_predicat_dadjacence_voit_une_lecture_hors_enregistrement` (une négative, une positive) |
| V5 | l'allowlist est née vide et le reste | `mika2847_lallowlist_des_lecteurs_est_livree_vide` |
| V6 | `[heartbeat] enabled = false` ⇒ la ligne existante passe à `cancelled` | V8 dans `test_agent_disabled_recurring_gate_2456.rs` |
| V7 | contrôle négatif : pas de section `[heartbeat]` ⇒ résolution `true`, enregistrement nominal | V8-bis |
| V8 | rien ne régresse sur les vols 1–3 | `cargo test -p mika-agent` entier, `cargo clippy`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` |

---

## Fire-Disposition

Ce vol livre **cinq détecteurs** : D2, D3, D4, D5 (scans et assertions structurelles
dans `canonical_tokens.rs`) et D6 (sondes comportementales). Chemin de succès pour
les quatre premiers : *aucune violation trouvée*.

**Option retenue : (a) exception nommée en allowlist — née vide.**

- `HEARTBEAT_GATE_READER_ALLOWED: &[&str] = &[]`, avec son assertion auto-nettoyante
  `mika2847_lallowlist_des_lecteurs_est_livree_vide` (D5), qui rougit si une entrée y
  est déposée sans que le scan soit retiré.
- **Violations préexistantes : zéro, et c'est mesuré — pas espéré.** Le grep de
  préparation rend quatre hits de `heartbeat_enabled_for_agent` dans l'arbre, dont
  exactement deux lecteurs de production, tous deux au recensement : `server/mod.rs`
  et `mika-cli/src/commands/chat.rs`. Les deux autres sont la **définition**
  (`task_engine/mod.rs:898`, écartée par le filtre `fn …(`) et la **sonde V6 du vol
  2** (`dispatcher.rs:5743`, écartée par la troncature au premier `#[cfg(test)]`,
  `:5195`). Le prédicat d'adjacence de D3 est satisfait par les deux sites recensés,
  à 9 et 11 lignes de distance pour une fenêtre de 25.
- **Le recensement n'est pas l'allowlist, et leur confusion est la seule façon de
  mal lire cette disposition.** `HEARTBEAT_GATE_REGISTRATION_SITES` naît à **deux**
  entrées parce que la proposition attestée est *« tous les lecteurs sont à
  l'enregistrement »*, pas *« il y a un lecteur »* — c'est un périmètre (sens de
  mika#2536), précédent `TESTIMONY_PREDICATE_READERS` (mika#2627, trois entrées).
  mika#2323 porte sur la case où déposer la prochaine infraction : c'est
  `HEARTBEAT_GATE_READER_ALLOWED`, et elle est vide.
- **Message de résolution, sur les deux scans.** Un lecteur non recensé se résout en
  **routant la lecture par un site d'enregistrement existant**, jamais par une entrée
  d'allowlist (doctrine mika#2201). Et le message le dit en toutes lettres : un
  lecteur au **tir** retournerait précisément la propriété que ce scan atteste — donc
  l'allowlister serait déclarer fausse la proposition pour laquelle le scan existe.
  Un lecteur d'enregistrement **nouveau et légitime** (une troisième porte d'entrée
  du substrat) se déclare dans le **recensement**, avec sa raison, et doit satisfaire
  D3.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/canonical_tokens.rs` (D1 recensement + allowlist, D2 helper + scan, D3 prédicat + scan, D4 bonne foi, D5 assertion, doc-comments compris) | 178 |
| `crates/mika-agent/tests/eval/test_agent_disabled_recurring_gate_2456.rs` (D6 : en-tête de module, `home_with_heartbeat`, V8, V8-bis) | 77 |

Total estimé : 255 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 255 < 279, à 91 % de la borne — serré, et c'est
assumé plutôt que subi : la marge a été dégagée par une **suppression de livrable**,
pas par une sous-estimation. Un sixième détecteur était rédigé (`le tir ne lit pas la
porte de section`, une assertion ciblée sur `dispatcher.rs`, ≈ 20 lignes) ; il est
retiré parce qu'il est exactement redondant avec le volet « étrangers » de D2, qui
rougit déjà sur `dispatcher.rs`. Le ticket amont mesure la même chose depuis l'autre
bord : le corps de mika#2847 chiffre ce vol à *« ≈ 40 lignes au-dessus de la borne »*
s'il avait été soudé au vol 3, dont 248 lignes livrées — 248 + 255 = 503, bien
au-dessus de 279, ce qui confirme la découpe en vols plutôt qu'en phases.

Aucune dépendance technique sur le vol 3 : le harnais eval et `production_sources()`
sont sur `main` depuis les vols 1 et 2, et ce vol n'emprunte au vol 3 que la **forme**
de son scan, pas son code. Le seul livrable qui aurait pu pousser au-dessus de la
borne — la transposition à `reflection`, structurellement identique aux deux mêmes
sites — est renvoyé par la ligne `Tracked in:` du § *Périmètre — ce que CE vol ne
livre PAS*.

---

## Definition of Done

- [ ] D1 — `HEARTBEAT_GATE_REGISTRATION_SITES` (deux entrées, chacune avec sa raison)
      et `HEARTBEAT_GATE_READER_ALLOWED` (vide) livrées dans `canonical_tokens.rs`,
      avec le doc-comment qui nomme la distinction périmètre / allowlist.
- [ ] D2 — `heartbeat_gate_reader_sites()` et
      `mika2847_la_porte_de_section_na_que_des_lecteurs_denregistrement` : aiguille
      composée à l'exécution, définition écartée, anti-vacuité **avant** le volet
      « étrangers », message de résolution nommant mika#2201.
- [ ] D3 — `heartbeat_gate_reads_at_registration` et
      `mika2847_chaque_lecteur_recense_annule_a_lenregistrement` : les trois termes
      du geste dans une fenêtre de 25 lignes décommentées, sur les deux sites.
- [ ] D4 — `mika2847_le_predicat_dadjacence_voit_une_lecture_hors_enregistrement` :
      une assertion négative (lecture au tir) et une positive (forme réelle).
- [ ] D5 — `mika2847_lallowlist_des_lecteurs_est_livree_vide`.
- [ ] D6 — `home_with_heartbeat`, V8 et V8-bis dans
      `test_agent_disabled_recurring_gate_2456.rs`, et son en-tête de module mis à
      jour (V8 nommée, la phrase « aucun test ici ne couvre … » rectifiée).
- [ ] `cargo test -p mika-agent` vert, `cargo clippy` sans avertissement nouveau,
      `cargo fmt --check` propre.
- [ ] `scripts/verify-pipeline.sh origin/main` vert (dont U2 : le plan porte
      `## Acceptance criteria`).
- [ ] Le corps de la PR nomme l'écart de fait avec le corps du ticket (deux lecteurs
      de production, pas un) et la forme retenue en conséquence — un relecteur qui
      compare le corps au diff doit trouver l'explication dans la PR, pas la
      reconstruire.

---

## Acceptance criteria

Le corps de mika#2847 ne porte pas de section `## Acceptance criteria`. Les
critères ci-dessous sont dérivés de son § *Périmètre* et du contrat de vérification
ci-dessus.

1. **AC1 — un lecteur de production non recensé de `heartbeat_enabled_for_agent`
   fait échouer un test.** Le scan parcourt `production_sources()` (tout
   `crates/*/src`, CLI comprise), écarte la définition et le code de test, et nomme
   les étrangers trouvés.
2. **AC2 — le scan n'est pas vert par vacuité.** Il échoue si l'un des deux sites
   recensés — `crates/mika-agent/src/server/mod.rs` et
   `crates/mika-cli/src/commands/chat.rs` — cesse d'être trouvé.
3. **AC3 — l'aiguille est composée à l'exécution**, de sorte que
   `canonical_tokens.rs` ne se compte pas lui-même comme lecteur.
4. **AC4 — chacun des deux sites recensés lit la porte au voisinage immédiat du geste
   d'enregistrement**, attesté par la présence conjointe de `ensure_recurring_task`,
   de `cancel_recurring_task_by_label(` et du littéral `heartbeat` dans une fenêtre
   bornée de lignes décommentées.
5. **AC5 — le prédicat d'adjacence porte son propre contrôle de bonne foi** : il rend
   `false` sur une lecture plantée hors d'un site d'enregistrement, et `true` sur la
   forme réelle.
6. **AC6 — l'allowlist des lecteurs est livrée vide**, et une assertion dédiée rougit
   si une entrée y est ajoutée.
7. **AC7 — le message d'échec de chaque scan nomme la résolution attendue** : router
   la lecture par un site d'enregistrement, ne pas ajouter d'entrée d'allowlist
   (mika#2201).
8. **AC8 — sous `[heartbeat] enabled = false`, l'enregistrement annule la ligne
   `heartbeat` existante** : une ligne `recurring_active` préexistante se retrouve en
   `cancelled` après le passage de la paire lecture → annulation.
9. **AC9 — contrôle négatif** : sans section `[heartbeat]` dans `identity.toml`, la
   porte résout `true` et `ensure_recurring_task` enregistre la récurrente
   normalement.
10. **AC10 — l'en-tête de module du fichier eval ne contient plus de phrase fausse**
    sur son propre périmètre, et V8 y est nommée comme les autres sondes.
11. **AC11 — aucun fichier de production n'est modifié par ce vol.** La propriété est
    attestée, pas construite : si un changement de production devient nécessaire,
    c'est un écart au plan qui se remonte plutôt qu'il ne se décide.

---

## Risques et contre-mesures

| risque | contre-mesure |
|---|---|
| La troncature de `production_sources()` au **premier** `#[cfg(test)]` masque un vrai lecteur situé après un attribut `#[cfg(test)]` précoce. | Mesuré sur les quatre fichiers concernés : premier `#[cfg(test)]` à `server/mod.rs:2177` (> 1906), `task_engine/mod.rs:944` (> 898), `dispatcher.rs:5195` (< 5743, troncature **voulue**), et **aucun** dans `chat.rs`. Aucun des deux sites recensés n'est tronqué. |
| La fenêtre d'adjacence de 25 lignes devient trop serrée si un commit insère du code entre la lecture et l'annulation. | L'échec est **bruyant** et dit la bonne chose (« la lecture a quitté le site d'enregistrement ») ; la résolution est de relire le site, pas d'élargir la fenêtre par réflexe. La mesure actuelle est à 9 et 11 lignes, décommentées. |
| Le scan paraît redondant avec la sonde V6 du vol 2. | Le doc-comment du scan dit l'écart exactement : V6 atteste que le **filet** ne consulte pas la porte ; D2 atteste qu'**aucun site de production** ne la consulte hors enregistrement. V6 resterait verte sur un lecteur ajouté ailleurs au tir. |
| Le recensement à deux entrées se lit comme une allowlist née non vide, contre mika#2323. | Dit trois fois, aux trois endroits où un relecteur le rencontre : doc-comment de la constante, § *Fire-Disposition* de ce plan, corps de la PR. Précédent cité : `TESTIMONY_PREDICATE_READERS` (mika#2627). |
| D6 est pris pour une mesure de `server/mod.rs`. | Le doc-comment de V8 nomme sa borne : il rejoue la composition, il ne l'exécute pas depuis le serveur ; c'est D2/D3 qui attestent que `server/mod.rs` la porte. |

---

## Références

- mika#2456 — le défaut d'origine (agent dormant portant des récurrentes actives).
- mika#2841 / #2844 — vol 2, le filet au tir et sa sonde V6.
- mika#2843 / #2848 — vol 3, la surface d'audit et le scan SOLE-WRITER modèle
  (`mika2456_the_gate_audit_name_has_exactly_one_writer`).
- mika#2627 — le précédent du recensement à N entrées + allowlist vide
  (`TESTIMONY_PREDICATE_READERS`).
- mika#2653 — un prédicat partagé plutôt qu'une copie ; `composes_live_pilot_question`
  comme modèle de prédicat à plusieurs termes.
- mika#2670 — le corpus de `production_sources()` contient la CLI ; une borne
  documentée que le walk ne porte plus est un mensonge.
- mika#2496 — composer l'aiguille à l'exécution pour que le scan ne se dénonce pas.
- mika#2103 / mika#2205 — un scan qui ne trouve personne se lit comme un scan propre.
- mika#2201 / mika#2323 — on route le site, on ne l'allowliste pas ; une allowlist
  née vide.
- mika#2536 — un périmètre n'est pas une allowlist d'exemption.
- mika#2158 — la classe `grooming_marker` : la couverture se perd en silence.
- mika#2334 — le précédent de la lecture en source d'une composition de
  `server/mod.rs` non appelable.
