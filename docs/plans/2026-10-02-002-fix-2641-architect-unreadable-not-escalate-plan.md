# Une réponse d'architecte sans disposition lisible est relancée, jamais escaladée

**Ticket :** mika issue#2641
**Type :** fix (substrat de la boucle — `loop-substrate`)
**Branche :** `fix/2641/groom-une-r-ponse-d-architecte-sans`
**Lié :** mika#2280 (géométrie de coupure), mika#2545 (gel du re-dispatch après ESCALATE),
mika#1823 (le retry UNPARSED de **première** passe), mika#2037 (garde d'ancrage de revue),
mika#2050 (le stderr de dispatch-lib qui n'a pas de puits)

---

## 1. Ce que la lecture du code déplace dans le ticket — et c'est le premier livrable

Le ticket pose un défaut réel et mesuré. Cinq faits lus dans le code en déplacent
le remède, et chacun change ce qu'il faut écrire.

### R1 — Le retry demandé par l'AC1 EXISTE DÉJÀ, mais seulement sur la première passe

`_iterate_groom_loop` (`skills/bundled/_shared/dispatch-lib.sh:7819`) contient
une boucle `for attempt in 1 2` autour de la **première** passe : un UNPARSED
déclenche un re-`_arch_ask` avec un prompt correctif, session portée. C'est
mika#1823, livré le 2026-07-25. Et son terminal de double-UNPARSED ne pose
**aucun** ESCALATE — il rend `return 1` nu, que l'appelant convertit en
`Outcome: PIPELINE_INCOMPLETE` plus un `PIPELINE FAILURE:`, c'est-à-dire la
population **retryable** de `self-dev-callback`.

La **seconde** passe n'a rien de tout cela. Les deux sites sont :

| site | ligne | arme |
|---|---|---|
| `case "$verdict"` (après un premier passage READY) | ~7965 | `*)` → `_escalate_groom "second-pass-after-ready"` |
| `case "$verdict_iter"` (après ITERATE + revise) | ~8010 | `*)` → `_escalate_groom "second-pass-after-iterate"` |

Aucun retry, et le `*)` attrape l'UNPARSED **au même titre** qu'un verdict
ESCALATE — parce que `_parse_verdict` rend `GROOMED`, `ESCALATE`, **ou rien**, et
que le `case` n'a que `GROOMED)` et `*)`.

**Donc AC1 n'est pas « inventer un retry », c'est « porter le patron mika#1823
sur la seconde passe ».** Ce qui garde le changement petit, et ce qui donne le
gabarit exact à suivre (prompt correctif en `@`-fichier, session portée, marqueur
`-after-retry` au trail, borne à deux tentatives).

### R2 — Le fuzzy ne matche rien sur le verbatim : c'est bien le `*)` qui convertit

Avant de toucher au `case`, il fallait établir que `_parse_verdict` rend
réellement **rien** sur le verbatim du ticket. Si le tier 2 fuzzy avait rendu
`ESCALATE`, le remède aurait été tout autre (resserrer le fuzzy), et le retry
n'aurait jamais été atteint.

Verbatim : « Je relis le plan révisé mika#2617 pour second passe. Analyse en
cours des contrats de sortie et de la résolution des 12 rectifications. »

`_parse_verdict_fuzzy` (`dispatch-lib.sh:7035`) cherche, en insensible à la
casse :

- ESCALATE : `escalate`, `cannot approve`, `human review needed`,
  `fundamental issues remain` — **aucun** (le texte est en français) ;
- GROOMED : `groomed`, `approved`, `plan is ready`, `ship it`,
  `no remaining concerns` — **aucun**.

Tier 0 (marqueur de disposition retirée) et tier 0b (ligne d'escalade moteur) ne
s'appliquent pas : ni l'un ni l'autre littéral n'est présent. Tier 1 (`Verdict:`)
et tier 1b (`Disposition:`) non plus. **`_parse_verdict` rend donc une chaîne
vide, `verdict` est vide, et le `*)` arm escalade.** Confirmé.

### R3 — `dispatch-lib` n'écrit AUCUNE ligne d'audit, et son stderr est jeté

L'AC2 demande « plus une ligne d'audit ». `grep -n "audit_event\|log_audit"` sur
`dispatch-lib.sh` rend **zéro ligne** : c'est du shell, sans accès à la base.

Pire, et c'est le fait qui décide de la forme du livrable : **tout le stderr de
`_iterate_groom_loop` est jeté sur un dispatch de groom.** Le `CLAUDE.md` racine
l'écrit déjà noir sur blanc (§ mika#2474, *Hors périmètre*) — la boucle est
appelée **après** `_run_claude_pilot`, donc hors de la redirection
`2>"$STDERR_FILE"`, et son stderr est le `Stdio::piped()` que l'exécuteur ne lit
que dans la branche `if !status.success()` ; or un dispatch de groom sort
**toujours en 0**, donc le tuyau est lâché sans être lu. Classe mika#2050,
Signaux M et Q.

**Conséquence directe : toute instrumentation de ce ticket qui passerait par
`echo … >&2` serait inerte.** Les seules surfaces durables du loop sont :

1. `RESULT` → `tasks.result` (requêtable en SQL — c'est déjà la surface que
   mika#2545 lit pour geler, et celle que mika#2536 a choisie pour ses refus) ;
2. les fichiers sous `$WORKTREE_DIR/.iterate/` (préservés sur ESCALATE, et le
   worktree d'un groom sans PR est `pr_unknown`, donc conservé par mika#2420) ;
3. le trail `$WORKTREE_DIR/.claude/groom-verdict-trail.log`, qui meurt avec le
   worktree.

**La ligne `audit_events` demandée par l'AC2 exige donc deux moitiés** : le
producteur shell pose un marqueur dans `RESULT`, et un **lecteur moteur** l'y lit
et écrit la ligne. Le patron existe et il est à un appel de distance du site :
`try_report_pilot_cost_overrun` (`crates/mika-agent/src/task_engine/dispatcher.rs:4457`),
appelé dans `deliver_callback` juste après `try_extract_callback_metadata`, lit
`task.result` et écrit un `audit_events` SOLE WRITER sans rien arrêter.

### R4 — Collision de vocabulaire : `unreadable` est DÉJÀ pris, à un autre sens

mika#2545 a livré, dans `executor.rs:1994` :

```rust
pub(crate) const GROOM_ESCALATE_VERDICT_UNREADABLE: &str = "unreadable";
```

Il désigne « **la preuve en base** n'a pas pu être lue » (colonne `result` NULL
sur un callback terminal) — pas « la réponse de l'architecte est illisible ». Deux
sens, deux couches, un mot. L'AC2 nomme heureusement `architect_unreadable` et
non `unreadable` nu : **c'est ce qui évite de couper deux populations sous un même
mot**, et le plan tient ce nom-là.

### R5 — Le piège majeur : un prompt de retry trop long retourne le fix contre lui-même

`skills/bundled/mika-arch-second-review/skill.toml` déclare
`review_anchor_min_brief_chars = 2000`, et son commentaire dit **explicitement**
pourquoi ce seuil existe :

> Les trois skills arch sont `always_on`, donc le seul match de skill armerait la
> garde sur TOUS les tours de mika-arch — questions ad-hoc via `/mika-ask-arch`,
> et le re-prompt correctif de la récupération UNPARSED (mika#1823), dont le
> message utilisateur fait ~480 caractères. **Aucun des deux ne peut porter la
> preuve exigée.**

Donc : un prompt de retry de seconde passe dépassant 2000 caractères **armerait**
la garde d'ancrage mika#2037. L'architecte, qui n'a pas le brief sous les yeux
dans un re-prompt, ne peut produire aucune ancre ; le moteur retire alors la
disposition (tier 0) ou réécrit la réponse en escalade (tier 0b, mika#2338) ; et
`_parse_verdict` rend **ESCALATE**. **Le correctif produirait exactement le
terminal qu'il existe pour empêcher.**

Le prompt de retry est donc court **par contrat, et c'est asserté** (V4), pas
espéré.

### R6 — La mesure demandée par le commentaire opérateur existe déjà à deux tiers

Le commentaire du 2026-10-02 demande « appels architecte du jour : aboutis,
coupés et tronqués ». Deux tiers sont déjà instrumentés côté **moteur**, sur des
surfaces ungated :

- **coupés** : `llm_call_attempt` (mika#2331) porte `outcome` ∈
  `{success, retrying, exhausted, deadline_abort}`, `provider`, `model`,
  `elapsed_ms`, `error_class` ;
- **coupés au niveau tour** : `a2a_turn_failed` (mika#2522, SOLE WRITER) porte
  `agent_id`, `model`, `error_class`, et une ligne `audit_events` groupable.

Ce qui manque est le **troisième** tiers — « tronquées » — et c'est précisément
ce que la ligne d'audit de ce ticket crée. **Mais rapporter les trois ensemble,
par agent et par jour, est un livrable de reporting distinct** : il demande de
joindre deux `tool_name` et un journal, et sa précondition est que la population
neuve soit non vide. **Ticket de suivi**, nommé au § 9, pas livré ici — et le
commentaire le pose lui-même comme « au-delà du cas unreadable de ce ticket ».

---

## 2. La chaîne du défaut, maillon par maillon

| # | site | ce qui se passe |
|---|---|---|
| 1 | `mika-arch-second-review` | le modèle rend un préambule seul (troncature côté modèle) |
| 2 | garde moteur `required_suffix_line` | re-prompte **une fois** ; la réponse reste sans `Verdict:` |
| 3 | `_parse_verdict` | tiers 0, 0b, 1, 1b, 2 : aucun ne matche ⇒ **chaîne vide** (R2) |
| 4 | `_trail_append` | écrit `UNPARSED` au trail — **l'information existe déjà à cet instant** |
| 5 | `case "$verdict"` arm `*)` | `_escalate_groom "second-pass-after-ready"` — **le maillon décisif** |
| 6 | `_escalate_groom` | pose `GROOM ESCALATED (terminal)`, `Verdict: ESCALATE — human review required`, `Outcome: ESCALATE` |
| 7 | appelant (`dispatch_claude_pilot`, ~10025) | branche `elif grep -qE '^Outcome: ESCALATE'` ⇒ préserve, **pas** de `PIPELINE FAILURE:` ⇒ **hors de la population retryable** |
| 8 | garde mika#2545 (`executor.rs`) | `Outcome: ESCALATE` ancré ligne ⇒ **tout re-dispatch de groom est refusé** |

Le maillon 4 est ce qui rend le défaut petit à réparer : **la boucle sait déjà
que la réponse est illisible**, elle l'écrit même au trail, et le maillon 5 jette
cette distinction en la fondant dans le même bras qu'un verdict.

Le maillon 8 est ce qui rend le défaut coûteux : le gel n'est pas une
conséquence secondaire, c'est le mécanisme nominal de mika#2545 appliqué à une
non-décision. Le ré-armement est son geste opérateur documenté (`remove` → `add`
du label `ready`), et il demande un humain.

**Et le maillon 6 porte une affirmation fausse** : « `Verdict: ESCALATE — human
review required` » affirme un verdict que l'architecte n'a pas rendu. C'est
littéralement ce que l'AC2 nomme — « un motif distinct, **et non un verdict** ».

---

## 3. Décisions

### D1 — Le périmètre est la SECONDE passe ; la première reste retryable, et c'est raisonné

AC2 dit « sur la même passe ». La règle est appliquée là où le défaut vit, et
l'asymétrie avec la première passe est **nommée plutôt que découverte** :

- La première passe satisfait **déjà** AC1 : elle relance une fois (mika#1823) et
  ne convertit **jamais** en ESCALATE terminal (son `*)` rend `return 1` nu).
- Son double-UNPARSED ne pose pas `Outcome: ESCALATE`, donc mika#2545 **ne gèle
  pas**, donc `self-dev-callback` rejoue — et le rejeu part sur une session
  architecte **neuve** (le retour de dispatch ne porte pas le `session_id`), donc
  il n'est **pas** déterministe, donc **pas stérile**.
- La convertir en ESCALATE terminal serait un **durcissement non demandé** qui
  gèlerait des tickets que le rejeu récupère aujourd'hui.

**Coût nommé :** un double-UNPARSED de première passe continue de produire un
`PIPELINE_INCOMPLETE` retryable au lieu d'un ESCALATE motivé. Sa population et
son motif restent lisibles par `GROOM_LOOP_FAILURE_REASON`, qui nomme déjà
« disposition UNPARSED after N attempts ». La sonde S4 mesure si ce rejeu
converge ; si la mesure montre qu'il ne converge jamais, l'uniformisation devient
un **ticket de suivi avec un compte**, pas une intuition.

### D2 — Le terminal du double-unreadable garde `Outcome: ESCALATE` comme ligne de disposition

Tentant de poser un autre mot pour éviter le gel de mika#2545. **Refusé**, et la
raison est mesurable : `_measure_cycle_output` P4 (`dispatch-lib.sh:4101`) énumère
exactement `^Outcome: (PR_OPENED|PLAN_COMMITTED|PLAN_GROOMED|ESCALATE)`. Un mot
hors de cette liste ferait tomber le cycle en `empty`, c'est-à-dire un **faux
rouge** sur un run qui a bel et bien produit une décision — et un faux rouge
entraîne les gens à ignorer le rouge (la phrase est du site lui-même).

Donc le gel mika#2545 **subsiste**, et c'est voulu par AC2 : deux réponses
illisibles d'affilée sont une non-convergence réelle. Ce que le ticket achète
n'est pas l'absence de gel, c'est **une tentative de plus avant lui** (AC1) et
**un motif lisible quand il arrive** (AC2).

### D3 — Sur un unreadable, la ligne `Verdict:` n'est PAS écrite

C'est le cœur d'AC2. `_escalate_groom` gagne un quatrième paramètre, la **cause**,
avec un `case` **exhaustif sans bras par défaut muet** (motif
`dispatch_substrate_diagnostic`) :

| cause | ce qui est écrit |
|---|---|
| `verdict` (défaut, comportement d'aujourd'hui **byte pour byte**) | `mika-arch escalated at <stage>.` + `Verdict: ESCALATE — human review required.` |
| `architect_unreadable` | `mika-arch answered at <stage> without a parsable verdict line.` + `Groom-halt-cause: architect_unreadable — …` ; **aucune ligne `Verdict:`** |

Le défaut `verdict` est ce qui garantit AC3 : un ESCALATE explicite reste
terminal, et son `RESULT` est identique à l'octet près.

`GROOM ESCALATED (terminal):` reste le préfixe dans les deux cas. Vérifié : ce
littéral n'est lu par **aucun** code de production (seule occurrence hors
dispatch-lib : une fixture de test dans `executor.rs:11111`). Le discriminant
retryable est `PIPELINE FAILURE:`, lu par le prompt de `self-dev-callback` ; le
discriminant de gel est `Outcome: ESCALATE`, lu en Rust. Les deux sont préservés.

### D4 — Le motif est un format de fil, à site unique, synchronisé shell ↔ Rust

`architect_unreadable` atterrit dans `tasks.result` **et** dans
`audit_events.after_value`, et l'opérateur en fera des `GROUP BY` : deux
orthographes couperaient une population en deux sans le dire. Donc une constante
nommée de chaque côté, et un **scan de synchronisation** (patron
`GROOM_ESCALATE_MARKER` ↔ son écrivain shell, déjà en place pour mika#2545).

La ligne porteuse est `Groom-halt-cause: architect_unreadable`, **ancrée en début
de ligne** et lue ancrée — jamais un `contains` : le `result` d'un callback porte
la prose du pilote, qui peut citer ce mécanisme même, et c'est le faux positif que
mika#2050 a mesuré sur le Signal S et que mika#2545 a dû éviter sur le sien.

### D5 — Un échec de transport n'est JAMAIS un unreadable

`_arch_ask_with_retry` rend `75` sur une classe transport, et la boucle fait déjà
`return 1` dans ce cas. Ce chemin est **inchangé** : il produit un
`PIPELINE_INCOMPLETE` retryable, ce qui est correct — une passe qui n'a pas
répondu n'a rien rendu d'illisible.

**« N'a pas répondu » et « a répondu sans disposition » restent deux populations,
et le fail-safe va dans le sens du rejeu dans les deux cas** : la première par le
`return 1` existant, la seconde par la relance de l'AC1. C'est la doctrine
mika#2277 appliquée dans le bon sens — *un signal qu'on ne peut pas lire n'est
jamais un terme satisfait*.

Même traitement pour un `.content` vide : `_groom_warn_empty_content` +
`return 1`, inchangé. mika#2296 a établi que c'est un défaut de **budget**
(`llm_max_tokens` épuisé par le raisonnement) avec son propre remède ; le fondre
dans l'unreadable effacerait un diagnostic qui a coûté trois tentatives et quatre
tickets à établir.

### D6 — Le retry porte la session

Comme mika#1823 : l'architecte doit voir **son propre préambule** pour le
compléter. C'est aussi ce que le contrat de continuité de
`mika-arch-second-review` exige, et ce que mika#2305 a épinglé pour le retry de
première passe (« le retry UNPARSED continue la session (D6) »).

### D7 — Le prompt de retry est court, et c'est asserté

Par R5. Cible : du même ordre que celui de la première passe (~480 caractères),
et **strictement sous 2000**. Asserté par V4, qui lit le seuil **depuis le
manifeste** plutôt qu'en le recopiant : un seuil durci dans le `skill.toml` sans
que le prompt rétrécisse doit faire rougir le test, pas passer.

### D8 — Deux moitiés pour l'audit, et chacune tombe seule

- **Moitié shell** (le producteur) : la ligne `Groom-halt-cause:` dans `RESULT`.
  Elle suffit à elle seule pour la lecture SQL sur `tasks.result`.
- **Moitié moteur** (le lecteur) : `try_report_groom_architect_unreadable` dans
  `dispatcher.rs`, patron `try_report_pilot_cost_overrun` — lit `task.result`,
  **n'arrête rien**, écrit un WARN et une ligne `audit_events` SOLE WRITER.

Ordre non contraint : la moitié shell livrée seule donne une surface SQL ; la
moitié moteur livrée seule n'a rien à lire et reste muette. Aucune des deux ne
peut produire un faux positif en l'absence de l'autre.

---

## 4. Requirements

**R-a — Retry UNPARSED sur la seconde passe, aux deux sites.**
Les deux `case` de verdict passent par une boucle `for attempt in 1 2` du même
gabarit que la première passe : sur chaîne vide, re-`_arch_ask_with_retry`
avec un prompt correctif court en `@`-fichier, `session_id` porté, puis re-parse.
Le trail reçoit `UNPARSED` puis `<verdict>-after-retry` ou `UNPARSED-after-retry`.

**R-b — Une seule définition du prompt correctif de seconde passe.**
Un helper (`_second_pass_retry_prompt`) écrit le fichier ; les deux sites
l'appellent. Deux copies divergeraient, et c'est la leçon que
`grooming_marker.rs` a dû engraver une fois (mika#2158) : un prédicat recopié
prend du retard sans que rien ne rougisse.

**R-c — `_escalate_groom` prend une cause, `case` exhaustif, défaut `verdict`.**
Les trois sites d'appel existants passent explicitement `verdict` (jamais par
omission : un appel qui oublie l'argument doit être visible au scan, pas tomber
dans un défaut silencieux). Les deux sites neufs passent `architect_unreadable`.

**R-d — La ligne `Verdict:` est conditionnée à la cause.** D3.

**R-e — La ligne `Groom-halt-cause:` est posée sur la cause `architect_unreadable`,
et sur elle seule.** Ancrée, format de fil, constante unique.

**R-f — `GROOM_LOOP_FAILURE_REASON` nomme la cause.**
Elle voyage dans `tasks.result` et c'est tout ce que l'opérateur voit (PR#2028).
Elle doit dire « l'architecte n'a pas émis de ligne de verdict lisible, deux
fois » et **pas** « l'architecte a refusé », qui est la phrase actuelle et qui
envoie lire une objection qui n'existe pas. C'est la classe mika#1772 exactement
— *le terminal conçu rapporté comme un bug*.

**R-g — Lecteur moteur + ligne d'audit.** D8, patron
`try_report_pilot_cost_overrun`. `tool_name` neuf, SOLE WRITER, `target_key` =
`<repo>#<issue>` quand il est résolvable, dégradé plutôt que supprimé sinon
(motif `repo=unknown` de mika#2496).

**R-h — Les trois scans.** § Fire-Disposition.

**R-i — Aucun autre comportement ne bouge.** Ni `_parse_verdict`, ni les tiers
fuzzy, ni la géométrie de retry transport, ni mika#2545, ni le prompt de
`mika-arch-second-review`, ni le chemin première passe, ni le chemin
`.content` vide, ni le chemin transport.

---

## 5. Hors périmètre, délibérément

- **Le durcissement de la première passe** en ESCALATE motivé — D1, avec son coût
  et sa précondition de suivi (sonde S4).
- **Le taux de coupure architecte par agent et par jour** — R6. Les coupures sont
  déjà mesurées (`llm_call_attempt`, `a2a_turn_failed`) ; ce qui manque est le
  **rapport joint**, et c'est un livrable de reporting. **Ticket de suivi**,
  précondition : que la population `architect_unreadable` soit non vide.
- **La cause de la troncature côté modèle** (kimi-k2.5 sur un brief lourd). Si la
  sonde S3 montre que les unreadable corrèlent avec la taille du brief, le levier
  est mika#2474 (le seuil `brief_size_alert_bytes`, déjà livré) et la géométrie
  de l'agent — pas ce prédicat. Ce travail rend la perte **rattrapable et
  comptable**, il ne fait pas cesser la troncature.
- **Le puits du stderr de `_iterate_groom_loop`** (R3). Réel, nommé, adjacent —
  et le fermer demande de décider où dispatch-lib écrit après `_run_claude_pilot`,
  ce qui est un changement de substrat d'exécution. **Ticket de suivi** ; c'est
  aussi pourquoi aucune instrumentation de ce plan ne passe par stderr.
- **Un durcissement de `_parse_verdict`** — rien n'est ajouté ni retiré à ses cinq
  tiers. Élargir le fuzzy pour attraper un préambule français serait l'inverse du
  remède : il faudrait deviner un verdict là où il n'y en a pas.
- **La garde mika#2545** — inchangée, non exemptée, non contournée. Elle cesse
  simplement d'avoir une population que la seconde passe lui fabriquait au
  premier essai.
- **Une variable d'environnement de désarmement.** Précédent le plus proche,
  mika#2627 : un désarmement par variable sur un chemin de convergence serait un
  désarmement par coquille. Le geste de désarmement est un **revert**, et le coût
  d'un faux positif (une relance de trop, bornée à une) le supporte largement.

---

## 6. Verification Contract

| # | ce qui est vérifié | forme | vu rouge ? |
|---|---|---|---|
| **V1** | le verbatim du ticket, en seconde passe après READY, **relance** au lieu d'escalader | probe shell sur le **vrai** `_iterate_groom_loop`, `_arch_ask` stubbé au bord de processus (gabarit `_groom_signature_probe_1772`) | **OUI — obligatoire** (AC4) |
| **V2** | contrôle positif : `Verdict: ESCALATE` explicite reste terminal, `RESULT` identique à l'octet près | même probe, réponse explicite | non (doit rester vert) |
| **V3** | double-unreadable ⇒ `Outcome: ESCALATE`, `Groom-halt-cause: architect_unreadable`, **aucune** ligne `Verdict:`, `GROOM_LOOP_FAILURE_REASON` ne dit pas « refusé » | même probe | **OUI** |
| **V4** | le prompt de retry est strictement sous `review_anchor_min_brief_chars` **lu depuis le manifeste** | assertion shell | non |
| **V5** | un échec transport (`exit 75` épuisé) et un `.content` vide ne produisent **pas** d'unreadable | même probe, deux cas | non |
| **V6** | le même traitement s'applique au site **after-iterate** | même probe, chemin ITERATE+revise | **OUI** |
| **V7** | le lecteur moteur écrit la ligne d'audit sur `Groom-halt-cause:` ancré, et **pas** sur un ESCALATE explicite ni sur une prose qui le cite | test Rust aux bornes (fonction pure + site) | **OUI** |
| **V8** | les trois scans tirent sur une violation fabriquée, et passent sur l'arbre | tests Rust / shell | **OUI** pour chaque scan |
| **V9** | anti-vacuité : chaque probe assert d'abord `arch_calls=<n>` attendu | assertion en tête de chaque cas | — |

**V9 n'est pas décoratif.** Sans lui, chacune des assertions ci-dessus passerait
sur une chaîne vide si le probe cessait de tourner — et c'est la précaution que
le probe mika#1772 a déjà dû écrire (« anti-vacuity anchor, first »).

**Ce qui n'est PAS testable ici, écrit plutôt que découvert :** « le modèle ne
tronque plus » est exécuté par un fournisseur, dans un autre processus. Le contrat
côté mika est *une réponse sans verdict lisible produit une relance puis un
terminal motivé*, et V1/V3/V6 l'attestent déterministement. La moitié
comportementale est la sonde S1.

Lancement : `make test-dispatch-lib` (CI l'exécute déjà) et `cargo test -p mika-agent`.

---

## 7. Fire-Disposition

Ce plan livre **trois détecteurs**. Disposition retenue :
**(a) exception nommée en allowlist — et les trois allowlists sont livrées VIDES.**

| détecteur | ce qu'il refuse | allowlist | état |
|---|---|---|---|
| **S-a** scan SOLE WRITER du `tool_name` d'audit neuf | un second écrivain du nom | `ARCHITECT_UNREADABLE_SOLE_WRITER_EXCEPTIONS` | **vide** |
| **S-b** scan de synchronisation du format de fil | le littéral `Groom-halt-cause:` / `architect_unreadable` divergeant entre `dispatch-lib.sh` et le Rust | `ARCHITECT_UNREADABLE_WIRE_EXCEPTIONS` | **vide** |
| **S-c** scan de cardinalité des appels à `_escalate_groom` | un site qui ne passe pas de cause explicite | *(assertion de compte, pas d'allowlist)* | — |

**Rien à excepter à la livraison, et c'est vérifiable** : les deux noms sont
**neufs**, donc aucune violation préexistante ne peut exister. Les allowlists
portent chacune leur **test auto-nettoyant** (`assert!(…is_empty())`) avec la
phrase de la doctrine mika#2201 : *quand le scan tire, on retire le second
écrivain, on ne l'excepte pas* — une allowlist née vide est un emplacement où
déposer la prochaine infraction (mika#2323).

Patron exact : `GROOM_ESCALATE_SOLE_WRITER_EXCEPTIONS` et
`mika2545_the_escalate_refusal_name_has_a_single_writer`
(`crates/mika-agent/src/canonical_tokens.rs:2868`), plus son frère
`mika2506_l_allowlist_du_nom_daudit_est_vide`.

**Pourquoi S-c est une assertion de compte et pas une allowlist** : la population
est de **cinq** sites (trois existants + deux neufs), connue et petite. Un scan
qui compterait sans asserter la cardinalité passerait en ne regardant rien le jour
où le nom de la fonction change — c'est la classe mika#2205, et c'est la même
raison qui a fait asserter `cardinality == 3` dans le scan de mika#2496.

**Aucun des trois ne peut être vu par un test comportemental** : un second
écrivain du nom ne rend aucune décision fausse le jour où il est écrit, il rend le
compte inexact **en silence** ; et un site d'appel qui oublie la cause produirait
un `RESULT` plausible.

---

## 8. Definition of Done

- [ ] Le retry UNPARSED de seconde passe est livré aux **deux** sites, par un
      helper unique, session portée, prompt court.
- [ ] `_escalate_groom` prend une cause, `case` exhaustif, défaut `verdict` ;
      les trois sites existants la passent explicitement.
- [ ] Sur `architect_unreadable` : pas de ligne `Verdict:`, une ligne
      `Groom-halt-cause:` ancrée, `GROOM_LOOP_FAILURE_REASON` nommant la cause.
- [ ] `Outcome: ESCALATE` reste la ligne de disposition (D2), donc P4 et
      mika#2545 lisent ce qu'ils ont toujours lu.
- [ ] Le lecteur moteur écrit la ligne `audit_events`, SOLE WRITER, sans rien
      arrêter.
- [ ] V1, V3, V6, V7 et les trois scans **vus rouges** avant d'être verts.
- [ ] V2 et V5 restent verts sans modification (contrôles positifs / négatifs).
- [ ] `make test-dispatch-lib`, `cargo test -p mika-agent`, `cargo clippy`,
      `cargo fmt` passent.
- [ ] `CLAUDE.md` racine : une section pour ce mécanisme — surfaces opérateur,
      sondes, haltes, et « ce que ce travail n'achète PAS ».
- [ ] Les trois tickets de suivi du § 5 sont nommés dans le corps de PR avec
      leur précondition (règle `Tracked in:` de `check-pr-body-consistency.sh`).

---

## 9. Acceptance criteria

Transcrits verbatim du ticket mika#2641.

- [ ] **AC1.** Une réponse d'architecte **sans ligne `Disposition:` reconnue**
      (ni READY, ni ITERATE, ni ESCALATE explicite) est classée `unreadable` :
      elle est **relancée une fois** comme une coupure de transport, et jamais
      convertie en ESCALATE terminal.
- [ ] **AC2.** Deux `unreadable` consécutives sur la même passe donnent un
      ESCALATE **avec un motif distinct** (`architect_unreadable`, et non un
      verdict), plus une ligne d'audit, pour que l'opérateur sache qu'il ne
      s'agit pas d'une objection.
- [ ] **AC3.** Un ESCALATE **explicite** (ligne `Disposition: ESCALATE` avec
      constats) reste terminal comme aujourd'hui.
- [ ] **AC4.** Test : le verbatim ci-dessus (préambule seul) donne une relance
      (**vu rouge** sur le code actuel, qui escalade) ; un ESCALATE explicite
      reste terminal (contrôle positif).

**Note de lecture sur AC1 :** le ticket écrit « ligne `Disposition:` ». Sur la
**seconde** passe la ligne canonique est `Verdict:` ; `_parse_verdict` accepte les
deux (tier 1b, tolérance de report de session). Le prédicat implémenté est donc
« `_parse_verdict` rend une chaîne vide », ce qui couvre l'intention de l'AC1 sans
la restreindre à une seule orthographe — et c'est le même prédicat que le
maillon 3 du § 2.

---

## 10. Surfaces opérateur

```bash
# 1. Une réponse illisible a-t-elle été relancée, puis escaladée ?
grep groom_architect_unreadable "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{repo, issue, stage, task_id}'

# 2. CONTRÔLE POSITIF — des grooms tournent-ils seulement ?
grep -c 'Outcome: PLAN_GROOMED\|Outcome: ESCALATE' "$MIKA_SPIRIT_LOG_FILE"
```

```sql
-- La population neuve, par étape
SELECT target_key, count(*) FROM audit_events
 WHERE tool_name = 'groom_architect_unreadable'
 GROUP BY 1 ORDER BY 2 DESC;

-- Le producteur, lu directement sur `tasks.result` (la surface de la moitié shell)
SELECT id, created_at, substr(result, 1, 300) FROM tasks
 WHERE result LIKE '%Groom-halt-cause: architect_unreadable%'
 ORDER BY created_at DESC;

-- CONTRÔLE NÉGATIF — les escalades EXPLICITES restent-elles la population dominante ?
SELECT count(*) FROM tasks
 WHERE result LIKE '%Verdict: ESCALATE — human review required%';
```

| surface | niveau | régime attendu | lecture |
|---|---|---|---|
| `groom_architect_unreadable` | WARN | **non vide, faible** | chaque ligne est une passe qui a échoué deux fois à rendre un verdict **après** avoir été relancée. Ce n'est pas une objection de l'architecte |
| `Groom-halt-cause:` dans `tasks.result` | — | même population | la moitié shell, lisible même si le lecteur moteur n'est pas déployé |
| `Verdict: ESCALATE — human review required` | — | **doit rester dominant** | le contrôle négatif : si cette population s'effondre au profit de la neuve, le prédicat mord trop large |
| une relance qui aboutit | — | **non vide** | le trail porte `<verdict>-after-retry` ; c'est la mesure que la relance **sert** à quelque chose |
| `groom_architect_unreadable_audit_failed` | WARN | **vide** | le WARN est passé, la ligne d'audit non — le `GROUP BY` sous-compte alors |

**Coût daté, nommé plutôt que découvert :** les escalades **antérieures** à ce
déploiement portent toutes `Verdict: ESCALATE — human review required`, y compris
celles qui étaient en réalité des unreadable — dont celle du 2026-10-02 sur
mika#2617. **Elles ne sont pas réécrites** : les réécrire rendrait faux ce
qu'elles ont dit quand elles ont été écrites (motif mika#2361). Un `GROUP BY` qui
enjambe le déploiement compare donc deux vocabulaires, et la population
`architect_unreadable` démarre à zéro **par construction**, jamais parce que le
défaut a cessé.

---

## 11. Sondes post-déploiement, et leurs cinq haltes

> **Préalable.** `skills/bundled/` est une projection du **binaire**, pas du
> checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer` doit porter le sha
> qu'on vient de bâtir — **sans cette vérification, chacune des sondes ci-dessous
> décrit le binaire d'hier.** Et ce sont des gestes d'**opérateur** sur l'hôte :
> la base n'est pas montée dans le bac à sable de dispatch.

**S1 — le défaut fondateur ne se rejoue pas** (première troncature réelle de
seconde passe). Attendu : le trail porte `UNPARSED` puis un verdict
`-after-retry`, le groom **converge**, et **aucune** ligne
`groom_architect_unreadable`.
*Halte 1 — un ESCALATE terminal part quand même alors que la relance n'apparaît
pas au trail :* **ne pas élargir le prédicat par réflexe.** Établir d'abord le
déploiement (préalable ci-dessus), puis lire **quel** des deux sites de seconde
passe a servi — le chemin after-iterate et le chemin after-ready sont deux
`case` distincts, et un seul corrigé se lit exactement comme deux.

**S2 — la relance sert (30 jours).** Le compte des `-after-retry` au trail doit
être **supérieur** à celui de `groom_architect_unreadable`.
*Halte 2 — ils sont égaux :* la relance ne récupère **jamais**, donc le prompt
correctif n'est pas lu, ou la garde d'ancrage l'intercepte (R5). **Ne pas ajouter
une seconde relance** — la famille des budgets d'un coup est délibérée. Vérifier
d'abord V4 sur le binaire servi, puis lire si le moteur a retiré la disposition
(`grep Disposition-Withheld` et la ligne d'escalade moteur).

**S3 — attribution (30 jours).** Croiser `groom_architect_unreadable` avec
`brief_size_overrun` (mika#2474) : si les unreadable tombent majoritairement sur
les briefs du décile supérieur, la cause est la **taille**, et c'est un ticket de
suivi sur la géométrie — **pas** sur ce prédicat.
*Halte 3 — répartition indépendante de la taille :* c'est un **résultat**, pas un
échec. La cause est le modèle ou le transport, et ce travail se referme sur sa
mesure.

**S4 — contrôle négatif de bruit (7 jours).** Aucun `groom_architect_unreadable`
sur une passe dont l'architecte a rendu un verdict explicite.
*Halte 4 — une occurrence :* faux positif, et son coût est une relance de trop
plus un motif faux dans `tasks.result`. **Désarmer d'abord** (revert du terme),
diagnostiquer ensuite.

**S5 — la population de D1 (30 jours).** Les double-UNPARSED de **première**
passe convergent-ils au rejeu ? `GROOM_LOOP_FAILURE_REASON` les nomme
(« disposition UNPARSED after 2 attempts »).
*Halte 5 — ils ne convergent jamais :* la prémisse de D1 est fausse, le rejeu est
stérile, et l'uniformisation devient un ticket de suivi **avec ce compte**.

**Halte transverse — les deux sondes muettes.** Zéro `groom_architect_unreadable`
**et** zéro `-after-retry` ne prouve **rien** : il faut qu'une seconde passe ait
tourné depuis le déploiement. Vérifier le contrôle positif avant toute conclusion.
*Une garde que personne n'a exercée se lit exactement comme une garde qui marche*
(mika#2205).

---

## 12. Ce que ce travail n'achète PAS

- **Il ne fait pas cesser la troncature.** Il rend la perte **rattrapable** (une
  relance) et **comptable** (un motif distinct). La cause vit chez le modèle et
  dans la taille du brief, et S3 est ce qui la dimensionne.
- **Il ne supprime pas le gel de mika#2545** sur un double-unreadable. AC2 veut un
  ESCALATE, donc le gel reste — simplement après deux tentatives au lieu d'une, et
  avec un motif que l'opérateur peut lire. Le ré-armement est le geste documenté
  de mika#2545 (`remove` → `add` du label `ready`).
- **Il ne rattrape pas l'incident du 2026-10-02 sur mika#2617.** Son `tasks.result`
  porte `Verdict: ESCALATE` et **rien ne rétro-estampille** : fabriquer une ligne
  décrivant une relance qui n'a pas eu lieu est l'inverse de ce que ce travail
  défend. La sonde est la **prochaine** occurrence.
- **Il ne couvre pas la première passe** — D1, avec son coût et sa précondition.
- **Il ne livre pas le rapport du commentaire opérateur** — R6, ticket de suivi.
  Les coupures sont déjà mesurées ; le **joint** manque.
- **Il ne donne aucun puits au stderr de `_iterate_groom_loop`** — R3, ticket de
  suivi. C'est aussi pourquoi aucune instrumentation de ce plan n'y passe : une
  ligne de journal qui n'atterrit nulle part est la classe mika#2050.
- **Il ne rend pas le champ surveillé, seulement lisible.** Les seuls instruments
  sont les greps et les requêtes du § 10, et **leur silence ne prouve rien tant
  que personne ne les exécute** — d'où le contrôle positif, sans lequel « aucune
  troncature » et « le lecteur regarde ailleurs » rendent les mêmes octets.
