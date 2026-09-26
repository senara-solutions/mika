---
title: Un cycle réussi doit signifier une sortie non vide - Plan
type: feat
date: 2026-08-30
issue: senara-solutions/mika#1996
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# feat(loop-substrate): un cycle réussi doit signifier une sortie non vide

**Ticket:** mika issue#1996 — milestone « Substrat de boucle », p1-important

---

## Goal Capsule

- **Objective.** Un cycle de pilote qui n'a rien produit ne peut plus rendre « réussi ». Un cycle qui a produit quelque chose continue de rendre « réussi » sans une ligne de bruit en plus. La différence entre les deux est une mesure écrite, pas une impression.
- **Means.** Une mesure unique (`_measure_cycle_output`) et une porte unique (`_gate_non_empty_cycle`) placées sur **le** goulot par lequel tout cycle sort — la livraison du callback —, plus une garde statique qui empêche un futur site de livraison de contourner la porte. C'est CONTROL-MUST-BE-UNAVOIDABLE appliqué à la structure du fichier : la garantie n'existe que si tout chemin qui livre un verdict traverse le point de contrôle.
- **Authority.** Le corps de mika#1996 fixe la direction et six AC. L'intent de dispatch ajoute trois exigences qui priment sur la lettre des AC en cas de conflit : définition vérifiable et défendable de « non vide », anti-vacuité dans les deux sens, pas de faux échec. Là où AC1 (« ≥1 tool_use ET ≥1 commit ») heurte l'exigence 3, l'exigence 3 l'emporte et l'écart est écrit dans la PR (voir KTD2).
- **Stop conditions.** Ne pas toucher les filets `#1282` / `#1383` — ils restent, la porte se place **après** eux. Ne pas inventer de jeton d'Outcome que les consommateurs ne connaissent pas (KTD3). Ne pas ajouter de code Rust : la classification vit là où le cycle se termine, en bash. Ne pas fermer mika#1910 sans mesure fraîche (KTD5). S'arrêter et remonter si la correction demande de modifier le contrat de callback lu par `dispatcher.rs`.
- **Execution profile.** Bash uniquement (`skills/bundled/_shared/dispatch-lib.sh` + sa suite d'assertions déjà gatée en CI). Aucune migration, aucun redéploiement requis pour prouver le comportement — mais une vérification contre des journaux de sessions réelles du disque, pas seulement contre des fixtures.
- **Tail ownership.** PR sur `feat/1996/loop-detector-cycle-r-ussi-output-non`, **`Closes #1996`**, reviewer `mika-platform-qa`.

---

## Product Contract

### Summary

Aujourd'hui la boucle appelle « réussi » un cycle dont elle n'a jamais regardé la sortie. Ce plan lui donne le regard : une mesure de production réelle, exécutée sur tout chemin de sortie, qui rend l'un de trois verdicts — *produit*, *vide*, *indécidable* — et qui n'a le droit de changer le callback que dans le second cas.

### Problem Frame

Mesure du 2026-08-29 : sur les **120 dernières sessions de pilote**, **102 ont fait zéro appel d'outil** et **aucune n'a dépassé 2**. La dernière session au-dessus de 10 appels date du **2026-07-29**. Pendant tout ce temps, le cycle se déclarait réussi.

Le trou est localisable à la ligne près. `_pilot_left_no_work` (`dispatch-lib.sh:1517`) mesure déjà exactement la bonne chose — HEAD inchangé **et** worktree propre — mais elle n'est consultée que sous `if [ "$STATUS" = "terminated" ]` (`:1470`). Les trois autres populations sortent sans mesure :

| Chemin de sortie | Mesure de production aujourd'hui |
|---|---|
| `status: terminated` | **oui** (`_pilot_left_no_work`) |
| `status: success` avec 0 `tool_use` | **aucune** — c'est la population de 102/120 |
| exit 0, sortie non structurée | **aucune** |
| exit non nul | **aucune** |
| crash du handler (trap EXIT) | **aucune** — et ce chemin livre son propre callback, sans passer par `_deliver_callback` |

En aval, l'engine ne rattrape pas : le reaper `callback_delivered_without_pr_url` (`engine.rs:1308`) passe un parent *implement* sans `pr_url` à `failed`, mais il porte un **filtre groom-class explicite** — un cycle de grooming vide n'est donc jamais marqué en échec. Et un cycle *implement* vide que le filet `#1383` a rattrapé fournit un `pr_url` : le reaper y lit un succès.

Les filets `#1282` (rescue worktree sale) et `#1383` (auto-PR-create) rattrapent le travail **après coup**. Ce sont des filets. Le défaut n'est pas qu'ils existent, c'est que la boucle **découvre** la panne au lieu de **la détecter** — la version la plus coûteuse du signal qui répond quelque chose plutôt que rien.

### Définition de NON-VIDE

Un cycle est **non vide** s'il a laissé au moins une trace de production **observable en dehors de son propre processus**. Quatre preuves, la première qui répond « oui » suffit :

| # | Preuve | Mesure |
|---|---|---|
| **P1** | Une PR lui appartient | `PR_URL` non vide (inclut la PR ouverte par un filet à partir de son contenu) |
| **P2** | La branche a avancé avec du contenu | `PRE_RUN_HEAD != POST_RUN_HEAD` **et** `git diff --quiet $PRE $POST` échoue (le diff est non vide) |
| **P3** | Le worktree porte des fichiers écrits | `git status --porcelain` non vide |
| **P4** | Une **disposition terminale motivée** | le callback porte `Outcome:` avec un jeton conclusif (`PR_OPENED`, `PLAN_COMMITTED`, `PLAN_GROOMED`, `ESCALATE*`) **et** le cycle a émis **≥1 appel d'outil** mesuré |

Ce que la définition **exclut** explicitement — et c'est la moitié qui porte :

- **Les signaux de processus** : `exit code 0`, `status: success`, « callback delivered », `task_id` retourné. Ce sont les trois signaux sur lesquels la boucle s'appuyait ; aucun ne dit qu'un travail a eu lieu.
- **Le volume de sortie du modèle** : le nombre de tours, la longueur du texte, le coût, la durée. Un cycle peut parler pendant deux tours et 600 secondes sans rien produire — c'est la forme exacte des 102 sessions.
- **Les fichiers hors dépôt** : journaux, `/tmp`, artefacts de trace. Écrire dans son propre journal n'est pas produire.
- **Les commits sans contenu** : un `--allow-empty` (le marqueur `wip(mika#1383)` en est un par construction) fait bouger HEAD sans rien produire. P2 exige un diff non vide, pas un déplacement de HEAD.
- **La disposition seule** : une ligne `Outcome:` sans un seul appel d'outil ne vaut rien. P4 est une conjonction, jamais une alternative — sinon la porte se satisferait du texte qu'elle est censée juger.
- **Lire n'est pas produire** : un cycle à 40 appels d'outils tous en lecture, sans P1–P3 et sans disposition conclusive, est vide.

Symétriquement, **le compte d'appels d'outils n'est pas le critère de non-vacuité**. Il qualifie P4 et enrichit le message ; il ne peut ni sauver un cycle qui n'a rien produit, ni condamner un cycle qui a produit. Cette asymétrie est délibérée : le compte se lit dans un fichier qui peut manquer, et un critère qui dépend d'un fichier absent fabrique des faux rouges.

### Les trois issues, et pourquoi il en faut trois

`produced` / `empty` / **`undetermined`**. Le troisième verdict existe parce qu'un détecteur qui doit choisir entre « vert » et « rouge » quand il n'a pas de terrain à mesurer choisira toujours mal : fail-closed fabrique des faux rouges (et un faux rouge entraîne à ignorer le rouge), fail-open reproduit le silence d'origine. `undetermined` ne touche pas l'Outcome — il ajoute une ligne `Measurement:` qui dit que la mesure n'a pas pu se faire et pourquoi. Il est comptable et grep-able, donc un `undetermined` fréquent est lui-même un défaut visible.

### Protection contre les faux échecs (exigence 3)

Quatre protections, chacune adossée à un test :

1. **Le cycle légitimement court passe par P4.** Un grooming qui conclut ESCALATE avec justification, ou un cycle qui constate « déjà fait » et le dit avec un jeton conclusif, est **non vide** dès lors qu'il a réellement agi (≥1 appel d'outil).
2. **Un cycle déjà rouge ne reçoit pas un second bandeau.** Si le callback porte déjà `PIPELINE FAILURE:` (session terminée, violation de push, crash), la porte mesure, journalise, et laisse le texte intact. Empiler des diagnostics est la manière la plus sûre de rendre le rouge illisible.
3. **Terrain non mesurable ⇒ `undetermined`, jamais `empty`.**
4. **Le sens positif est un invariant testé** : sur `produced`, `RESULT` est identique **octet pour octet** avant et après la porte.

### Non-Goals

- Corriger la cause amont du silence (le modèle qui n'émet aucun `tool_use`) — c'est mika#1910 / #1901.
- Réparer automatiquement un cycle vide : détecter et nommer, pas relancer.
- Remplacer les filets `#1282` / `#1383`, ni le reaper Rust : la porte se place devant eux et ne change pas leur contrat.
- Ajouter un statut de tâche `empty_completion` côté SQLite/Rust (voir KTD3).

---

## Planning Contract

### KTD1 — La porte va dans la livraison, pas avant elle

**Décision.** `_gate_non_empty_cycle` est appelée **en tête de `_deliver_callback`** et **dans `_dispatch_lib_exit_trap` juste avant son propre envoi**.

**Pourquoi.** `_deliver_callback` a deux appelants (sortie nominale et retour anticipé sur violation de push) et le trap EXIT **duplique l'envoi** au lieu de l'appeler (`dispatch-lib.sh:745-752`). Placer la porte « avant `_deliver_callback` » dans `dispatch_claude_pilot` laisserait donc deux chemins de sortie non gardés. Le sigle est explicite : une garantie n'existe que si **tout** chemin produisant l'effet gardé traverse le point de contrôle. L'effet gardé ici, c'est « un verdict de cycle atteint mika-dev ».

### KTD2 — La production directe suffit ; le compte d'outils qualifie la disposition

**Décision.** AC1 demande la conjonction « ≥1 tool_use **et** ≥1 commit ». Le plan rend une conjonction plus faible mais plus juste : P1/P2/P3 suffisent seules ; le compte d'outils n'entre qu'en P4.

**Pourquoi.** La conjonction stricte fabrique deux faux rouges mesurables : (a) un cycle qui a committé mais dont le `.stderr` a été rogné ou n'est pas lisible serait déclaré vide alors que son travail est sur la branche ; (b) un grooming qui escalade légitimement n'a aucun commit et serait déclaré vide. L'exigence 3 de l'intent prime, et l'écart est écrit dans la PR.

### KTD3 — Réutiliser le vocabulaire de sortie, nommer la classe dans le texte

**Décision.** Un cycle vide rend `PIPELINE FAILURE: empty_completion — <preuve>` en tête de `RESULT` et `Outcome: PIPELINE_INCOMPLETE — empty_completion: <preuve>`. Pas de nouveau jeton d'Outcome, pas de nouveau statut de tâche.

**Pourquoi.** `PIPELINE FAILURE:` et `Outcome: PIPELINE_INCOMPLETE` sont déjà compris par les consommateurs (prompt self-dev-callback, tests d'engine, tableaux de bord). Un jeton inédit — `PIPELINE_EMPTY` — serait invisible pour tout consommateur qui filtre sur les jetons connus : la porte crierait dans une langue que personne ne parle, ce qui est indistinguable du silence qu'elle corrige. Le nom de classe `empty_completion` demandé par AC1 reste greppable, porté par le texte, sans dépendre d'une migration.

### KTD4 — Une garde statique, pas une convention

**Décision.** Un test statique découpe `dispatch-lib.sh` par fonction et exige que **toute** fonction contenant un `mika ask … --task-complete` contienne aussi un appel à `_gate_non_empty_cycle`.

**Pourquoi.** L'invariant « tout chemin de sortie est mesuré » est exactement le genre de règle qu'un futur correctif casse sans le savoir en ajoutant un troisième site de livraison. La discipline de prompt ne tient pas au substrat ; une garde qui reste, si.

### KTD5 — mika#1910 reçoit une note de statut, pas une fermeture

**Décision.** AC5 demande de fermer mika#1910 avec une note de contournement. Le plan poste la note de statut et **laisse la fermeture à l'opérateur**, en le disant dans la PR.

**Pourquoi.** #1910 est un failure-mode amont (sortie vide silencieuse du modèle) dont la « mitigation active » est une bascule de modèle. La mesure du 2026-08-29 montre le silence **toujours présent** après la bascule du 2026-08-26. Fermer sur la foi d'une mitigation que la mesure contredit, c'est refaire le geste que ce ticket corrige — déclarer réparé ce qui n'a pas été mesuré.

### Risques

| # | Risque | Mitigation |
|---|---|---|
| R-1 | La porte transforme des cycles réussis en échecs (faux rouges) | Quatre protections ci-dessus, chacune testée ; invariant octet-pour-octet sur `produced` |
| R-2 | Le bandeau s'empile sur un callback déjà rouge et noie le vrai diagnostic | Court-circuit sur `PIPELINE FAILURE:` déjà présent + test d'occurrence unique |
| R-3 | Deux passages de la porte (par ex. `_deliver_callback` puis trap) doublent le bandeau | Idempotence : le marqueur `empty_completion` déjà présent bloque un second écrit ; testé |
| R-4 | Le `sed 's/Outcome: .*/…/'` réécrit une ligne dans un bloc de texte cité | Réécriture ancrée en début de ligne et bornée à la première occurrence ; testé sur un `RESULT` contenant un `Outcome:` indenté |
| R-5 | La mesure coûte du temps sur le chemin chaud | Trois `git` locaux + un `grep -c` sur un fichier déjà écrit ; aucun appel réseau |

---

## Implementation Units

### U1 — `_measure_cycle_output` : la mesure, sans effet de bord

Nouvelle fonction dans `skills/bundled/_shared/dispatch-lib.sh`, voisine de `_pilot_left_no_work`. Elle **ne touche pas `RESULT`**. Elle exporte trois variables :

- `CYCLE_OUTPUT_VERDICT` ∈ `produced` | `empty` | `undetermined`
- `CYCLE_OUTPUT_EVIDENCE` — une phrase de ce qui a été mesuré (jamais de ce qui a été supposé)
- `CYCLE_TOOL_CALLS` — entier, ou vide si non mesurable

Séquence :

1. Terrain absent (`WORKTREE_DIR` vide, inexistant, ou `git rev-parse --git-dir` échoue) → `undetermined` en nommant la raison.
2. `PR_URL` non vide → `produced` (P1).
3. `PRE_RUN_HEAD` et `POST_RUN_HEAD` non vides et différents **et** `! git diff --quiet $PRE $POST` → `produced` (P2), en citant le nombre de commits et le range.
4. `git status --porcelain` non vide → `produced` (P3).
5. Compte d'outils : `grep -c '\[tool:request\]'` sur `${PILOT_LOG_DIR:-/var/log/claude-pilot}/${LOG_ID}.stderr` — l'échec de lecture donne `CYCLE_TOOL_CALLS=""` et n'est jamais fatal.
6. Disposition conclusive dans `RESULT` (`^Outcome: (PR_OPENED|PLAN_COMMITTED|PLAN_GROOMED|ESCALATE…)`) **et** `CYCLE_TOOL_CALLS ≥ 1` → `produced` (P4).
7. Sinon → `empty`, avec pour preuve le triplet mesuré : HEAD inchangé, arbre propre, N appels d'outils (ou « compte indisponible »).

Réutilise `_pilot_left_no_work` pour la lecture HEAD/arbre partout où sa sémantique coïncide, en la précédant du test de lisibilité qui la distingue d'`undetermined`.

### U2 — `_gate_non_empty_cycle` : la porte

Appelle U1, journalise sur stderr une ligne structurée grep-able (`cycle_output.produced:` / `cycle_output.empty:` / `cycle_output.undetermined:` — même forme que `rescue_marker.skip_commit:` déjà en place), puis :

- `produced` → **ne touche à rien**.
- `undetermined` → ajoute une ligne `Measurement: cycle output undetermined — <raison>`. L'`Outcome:` reste celui du cycle.
- `empty` **et** `RESULT` ne contient pas `PIPELINE FAILURE:` → préfixe le bandeau `PIPELINE FAILURE: empty_completion — …` incluant la définition appliquée et la preuve, puis réécrit la ligne `Outcome:` (ou l'ajoute si absente) en `Outcome: PIPELINE_INCOMPLETE — empty_completion: <preuve>`.
- `empty` **et** `RESULT` déjà rouge → rien d'autre que la ligne de journal.
- Idempotent : `empty_completion` déjà présent dans `RESULT` ⇒ sortie immédiate.

### U3 — Câblage sur tous les chemins de sortie

- Premier appel exécutable de `_deliver_callback`.
- Dans `_dispatch_lib_exit_trap`, immédiatement avant la troncature à 92 Ko et l'envoi.

### U4 — Garde statique de non-contournement

Dans `skills/bundled/_shared/test-dispatch-lib.sh` : découpage par fonction (awk, de `^…() {` à `^}`), puis pour chaque bloc contenant `--task-complete`, assertion qu'il contient `_gate_non_empty_cycle`. Assertion complémentaire : le nombre de sites de livraison est celui qu'on croit, de sorte qu'un troisième site fasse rougir la suite plutôt que passer inaperçu.

### U5 — Tests dans les deux sens

Dans la même suite (gatée en CI par `make test-dispatch-lib`), un probe à **vrai dépôt git temporaire** — même forme que `_left_no_work_probe` — couvrant :

*Sens « ne rien produire échoue »* : arbre propre + HEAD inchangé + 0 outil → `empty`, bandeau présent, `Outcome` réécrit ; commits vides uniquement → `empty` ; disposition conclusive avec 0 outil → `empty`.

*Sens « produire reste réussi »* : HEAD avancé avec contenu → `produced` et `RESULT` **identique octet pour octet** ; arbre sale → idem ; `PR_URL` présent → idem ; `PLAN_GROOMED` + 3 outils → idem (le cycle légitimement court).

*Ni l'un ni l'autre* : worktree absent → `undetermined`, aucun bandeau d'échec.

*Robustesse* : callback déjà `PIPELINE FAILURE:` → exactement une occurrence ; double passage → un seul bandeau ; `RESULT` contenant un `Outcome:` cité → une seule ligne réécrite.

### U6 — Deux principes gravés (AC4)

`docs/solutions/best-practices/cycle-non-empty-detector-2026-08-30.md` — la définition, ses exclusions, les trois verdicts, et pourquoi le compte d'outils ne peut pas être le critère.

`docs/solutions/best-practices/no-substrate-on-open-failure-mode-2026-08-30.md` — « pas de substrat de production sur un failure-mode OPEN documenté », avec #1910 comme incident fondateur et le corollaire opérationnel : quand un ticket de failure-mode s'ouvre, marquer les substrats qui l'exercent.

Frontmatter YAML conforme aux entrées voisines (`module`, `tags`, `problem_type`, `category`).

### U7 — Note de statut sur mika#1910 (AC5, borné par KTD5)

Commentaire factuel : mitigation active (bascule de modèle du 2026-08-26), mesure du 2026-08-29 qui montre le silence persistant, et le fait que mika#1996 détecte désormais la classe au lieu de la subir. Fermeture laissée à l'opérateur.

### U8 — Vérification contre l'état réel (AC3/AC6)

Au-delà des fixtures : exécuter la mesure contre des `.stderr` de **sessions réelles** présentes sur la machine — au moins une session vide connue et une session productive — et reporter le verdict rendu dans la PR. Une suite construite depuis la définition ne peut pas falsifier la définition ; des journaux du monde, si.

---

## Verification Contract

| Ce qui est vérifié | Comment | Preuve attendue |
|---|---|---|
| Les deux sens | `make test-dispatch-lib` | Toutes les assertions U5 passent, `FAIL=0` |
| Non-contournement | garde statique U4 dans la même suite | Tout site de livraison est gaté |
| Aucune régression de la suite existante | `make test-dispatch-lib` complet | Le total de PASS ne baisse pas |
| Invariant octet-pour-octet sur `produced` | assertion dédiée | `RESULT` avant == après |
| Comportement sur l'état réel | U8, journaux `/var/log/claude-pilot/*.stderr` | Verdict rendu sur ≥1 session vide réelle et ≥1 session productive réelle |
| Artefacts de pipeline | `bash scripts/verify-pipeline.sh` | Sortie « passed », plan + docs solution + source |

Hors périmètre de vérification : le comportement bout-en-bout d'un dispatch réel (nécessite un rebuild + redéploiement du binaire qui embarque les skills) — la PR le dit au lieu de le suggérer.

---

## Definition of Done

- [ ] `_measure_cycle_output` et `_gate_non_empty_cycle` existent dans `dispatch-lib.sh`, la mesure étant sans effet de bord.
- [ ] Les deux chemins de livraison (`_deliver_callback`, trap EXIT) traversent la porte.
- [ ] La garde statique refuse un site de livraison non gaté.
- [ ] Les tests des deux sens passent, y compris l'invariant octet-pour-octet.
- [ ] Les deux documents de principe sont écrits avec #1910 comme incident fondateur.
- [ ] La note de statut est postée sur mika#1910.
- [ ] La définition de « non vide » **et ce qu'elle exclut** figurent dans le corps de la PR.
- [ ] `make test-dispatch-lib` et `scripts/verify-pipeline.sh` passent.
- [ ] Les écarts assumés vis-à-vis d'AC1 et AC5 sont écrits dans la PR, avec leur raison.

## Acceptance criteria

- [ ] **AC1** — `run_claude_pilot` post-flight step : grep transcript pour ≥1 tool_use post-orientation + ≥1 git diff commit. Si zero → mark task `empty_completion` (new status), emit `audit_event(kind=empty_completion, task_id)`. NEVER mark `succeeded` on empty.
- [ ] **AC2** — Unit test : synthetic transcript with 0 tool_use post-Turn-1 → detector fires empty_completion.
- [ ] **AC3** — Integration test : dispatch d'un ticket bidon avec model configured to return empty → detector catches, PR non ouvert, alert dans mika-manager report.
- [ ] **AC4** — Doc solution : `docs/solutions/best-practices/no-substrate-on-open-failure-mode.md` + `docs/solutions/best-practices/cycle-non-empty-detector.md` — deux principes gravés avec incident #1910 comme founding.
- [ ] **AC5** — mika#1910 status update : marked `blocked-by-glm52-in-loop` → close avec workaround note (bascule glm-5.3 = mitigation active).
- [ ] **AC6** — Post-fix : re-run gabarit T1 avec pilot forcé empty → verify detector fires, PR pas ouvert, alert visible.

**Lecture des AC sous les exigences de l'intent** (les écarts sont assumés et écrits dans la PR) :

- **AC1** est rendu par une conjonction plus juste — la production directe (P1–P3) suffit, le compte d'outils qualifie la disposition (P4) — parce que la conjonction stricte fabrique deux classes de faux rouges (KTD2). Le nom de classe `empty_completion` est porté par le texte du callback et le journal stderr plutôt que par un nouveau statut SQLite (KTD3).
- **AC3** est rendu sans forcer un modèle vide en conditions réelles : le probe reconstitue l'état exact que la mesure lit et U8 confronte la mesure à des journaux de sessions réellement vides. Le fait qu'aucune PR ne soit ouverte sur un cycle vide est vérifié structurellement — les deux classes de rescue exigent l'une un arbre sale, l'autre des commits, c'est-à-dire précisément P2/P3.
- **AC5** est rendu par une note de statut sans fermeture (KTD5).
- **AC6** est rendu par U8 : la vérification contre l'état réel du disque, dans les limites d'un changement qui n'est pas redéployé par cette PR.
