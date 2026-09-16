---
title: Review-anchor : échec visible en ESCALATE et comparateur tolérant au balisage - Plan
type: fix
date: 2026-09-16
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
issue: mika#2338
---

# Review-anchor : échec visible en ESCALATE et comparateur tolérant au balisage - Plan

**Ticket :** mika issue#2338 — *Grooming ne reconverge pas : arch émet READY à 2/3 ancres verbatim (QuoteNotInBrief) → guard withhold (par conception) → PIPELINE FAILURE opaque*
**Labels :** `p1-important`, `substrate`
**Chemin de réparation :** spawn hors-boucle (le grooming autonome via mika-arch est le composant en panne)
**Fichiers principaux :** `crates/mika-agent/src/agent_loop/review_anchor.rs`, `crates/mika-agent/src/agent_loop/mod.rs`, `skills/bundled/_shared/dispatch-lib.sh`, `skills/bundled/mika-arch-{groom-ticket,groom-milestone,second-review}/system_prompt.md`

---

## Goal Capsule

- **Objective :** un groom dont l'architecte a réellement lu le brief reconverge sous le guard #2037 actif, et un groom qui ne converge pas dit *pourquoi* dans `tasks.result` sans qu'un opérateur ait à ouvrir `server.log`.
- **Means :** normaliser le balisage markdown côté comparateur (KTD1) et remplacer le retrait silencieux de la disposition par un `ESCALATE` terminal qui nomme la cause (KTD2), le prompt de grooming enseignant les trois règles que le comparateur vérifie (KTD4).
- **Autorité :** le corps du ticket mika#2338 (bearing Prime, GO 2026-09-16 10:31) > ce plan > le code existant. Les contraintes Prime sont des décisions closes (voir Key Decisions).
- **Conditions d'arrêt :** si (0) avait montré que le brief vérifié n'est pas celui montré à l'arch, la normalisation aurait été hors sujet — ce n'est pas le cas (voir Mesures M1). Si l'implémentation découvre qu'aucune ligne `ESCALATE` n'est déclarée par un skill producteur de verdict, le repli est le marqueur withheld existant, jamais l'acceptation.
- **Profil d'exécution :** pipeline `/mika` dans le worktree `feat/2338/grooming-ne-reconverge-pas-arch-met` ; PR gated QA ; merge par l'opérateur.

---

## Product Contract

### Summary

Le comparateur `verify_review_anchors` cesse de rejeter une citation exacte au mot près parce que le brief porte `**gras**` et `` `code` `` et que l'architecte les a retirés en citant.
Au second échec du guard, le moteur n'efface plus la disposition : il émet une disposition `ESCALATE` terminale précédée d'une ligne `F1:` signée `[mika-engine]` qui porte `anchors_found`, `anchors_valid` et `miss_reason`.
`dispatch-lib` reconnaît cette ligne par sa forme complète en début de ligne — pour rendre `ESCALATE` avant tout tier textuel et pour la remonter dans `RESULT` avec une raison d'échec qui nomme le moteur, pas l'architecte.
Les trois prompts arch enseignent ce que le comparateur vérifie réellement : citer le message-brief de ce tour, garder la citation sur la ligne d'ancre, copier les mots exactement.

### Problem Frame

Le guard #2037 est fail-closed par conception : re-prompt une fois, puis `withhold_disposition` retire la disposition et `dispatch-lib` conclut « verdict missing » → `PIPELINE FAILURE`.
Depuis le passage de `llm_max_tokens` à 32768 (#2296/#2332), kimi-arch atteint le verdict et attesté, mais des grooms substrat restent bloqués à `anchors_valid=2` sur `anchors_found=3` avec `miss_reason=QuoteNotInBrief`.
Le ticket demandait d'abord de **prouver** que le brief vérifié est celui montré à l'architecte, avant de toucher au marqueur.

### Mesures — exécutées le 2026-09-16 dans le worktree

**M1 — le brief vérifié est exactement le message-utilisateur du tour.** `run_loop` reçoit `review_brief = Some(params.user_message)` (`agent_loop/mod.rs`, appel en mode conversation).
Le message est sauvegardé verbatim (`db.rs::save_message_with_task_context`, `INSERT` sans transformation) et la fenêtre #2295 n'élide jamais le message du tour (`truncate_history_to_token_budget` : « The turn's own user message is never elided », test `mika2295_the_turn_s_own_question_is_never_elided`).
Aucun bornage, aucune normalisation, aucun remplacement de guillemets entre ce que l'arch lit et ce que le comparateur lit.
La question (0) du ticket est donc close : **le texte est le même ; c'est la citation qui diffère.**

**M2 — la citation qui rate, et le fragment le plus proche du brief (#2335, session `975d44d0`, `llm_calls` `8abf1b1b`, message 87416, événement `guard.review_anchor` 2026-09-16T07:32:11Z `anchors_found=3 anchors_valid=2`).**

Ancre A1 émise par l'arch (première ligne, la seule que le comparateur lit) :

```text
A1: "Le mécanisme de kill, lui, est correct : kill_process_gracefully (process_kill.rs:82)
```

Ligne 63 du brief :

```text
**Le mécanisme de kill, lui, est correct** : `kill_process_gracefully` (`process_kill.rs:82`)
```

Les mots sont identiques ; seuls `**` et les backticks diffèrent.
`normalize_whitespace` ne touche qu'aux espaces, donc aucune fenêtre de 40 caractères de A1 n'existe dans le brief : « Le mécanisme de kill, lui, est correct » fait 38 caractères avant le premier écart.
A2 (« Le chaînon manquant existe déjà et est testé. », 45 caractères avant le premier backtick) et A3 (« Une supersession qui annule un parent portant un dispatch », 58 caractères) passent.
Rejoué hors moteur avec le même algorithme en retirant `*` et `` ` `` des deux côtés : **3/3**.

**M3 — le balisage est la norme des briefs, pas l'exception.** Sur les 245 briefs de plus de 2000 caractères reçus par mika-arch depuis le 2026-09-01 (`messages`, `role='user'`) : 237 contiennent `**`, 243 contiennent des backticks, 229 contiennent `«` `»`, 0 contient `’`.
Sur les 199 réponses arch portant `A1:` : 2 contiennent `’`.
Le comparateur rejette donc tout architecte qui cite le texte *rendu* plutôt que la *source* markdown — ce que fait un modèle qui lit un brief comme un humain.

**M4 — le second cas 3/0 (#2296, session `09b565bb`, 2026-09-15T15:58Z, withheld) est une autre classe, déjà fermée.** Les trois ancres citaient « La résolution retenue est l'option 2 : reconstruction explicite… », texte absent des deux briefs #2296 mais présent dans huit briefs **#2293** reçus par le même agent entre 04:53Z et 15:28Z (messages 86670…87136).
L'arch attestait sur le mauvais ticket, lu dans sa fenêtre agent-wide — le défaut AC7 de #2295.
Le guard a correctement refusé.
`mika-arch/identity.toml` porte désormais `[context.history] scope = "session"` (PR #2327 mergée 2026-09-15T14:01Z, binaire du 2026-09-16 08:39 local).
Rien à faire ici ; ce cas prouve que la porte sert.

**M5 — les cas `NoAnchorLine` se corrigent au re-prompt.** Sessions `20223f34`, `da48f5da`, `30664b08` (09-15) : premier tour sans ancre, re-prompt, second tour à 3 ancres valides, pas d'événement withheld.
Le re-prompt fire et fonctionne ; le ticket a raison sur ce point.

**M6 — #2331 (tâche `59a8b389`, session arch `06615128`) n'est pas un cas de guard.** Sept appels d'outils entre 07:07:07Z et 07:10:53Z, aucun `EndTurn`, aucun événement `guard.*`, tâche arch `e555b97b` annulée à 07:33:47Z.
C'est un tour qui ne rend jamais de réponse après ses lectures d'outils — la classe « arch qui hange » déjà connue, hors périmètre de ce ticket.
Le n=3 du ticket est en réalité n=1 (#2335) pour la classe markdown, n=1 (#2296) pour la classe cross-ticket fermée, n=1 (#2331) pour une classe distincte.

### Key Decisions

- **Le guard reste armé à trois ancres vérifiables ; `review_anchor_min_count` ne bouge pas.** (session-settled: user-directed — chosen over abaisser le seuil à 2 ou désactiver le guard : bearing Prime 2026-09-16, « une porte se répare par la cause ».) Governs R1, R2.
- **La normalisation se fait côté comparateur, jamais côté config.** (session-settled: user-directed — chosen over élargir la config du guard : le ticket réserve explicitement ce fix de marqueur au cas où (0) expose un mismatch de normalisation, ce que M2 fait.) Governs R1.
- **Le marqueur (B) passe par le prompt de grooming, jamais par la config.** (session-settled: user-directed — chosen over modifier `skill.toml` : contrainte explicite du ticket ; si le modèle ne peut structurellement pas, c'est une escalade « model-fit » séparée.) Governs R6.
- **Un test négatif est obligatoire : arch attesté 2/3 → ESCALATE-avec-raison, pas withhold silencieux.** (session-settled: user-directed — chosen over un test positif seul : contrainte du ticket.) Governs R3, R4.

### Requirements

**Comparateur (marqueur)**

- R1. `verify_review_anchors` accepte une ancre dont les mots sont dans le brief quand seuls le balisage inline markdown (`*`, `` ` ``) ou la forme de l'apostrophe (`’` vs `'`) diffèrent — la normalisation s'applique symétriquement au brief et à l'ancre.
- R2. Une paraphrase, une citation d'un autre document, ou une citation trop courte après retrait du balisage restent rejetées avec la même raison qu'aujourd'hui ; le balisage ne peut pas acheter de longueur.

**Fail-visible (moteur)**

- R3. Au second échec du guard, la réponse finale porte une disposition terminale `ESCALATE` déclarée par le skill, précédée d'une ligne de finding signée `[mika-engine]` qui nomme `anchors_found`, `anchors_valid` et `miss_reason`.
- R4. Aucune occurrence — ligne propre ou mention inline dans le corps — d'une disposition non terminale déclarée ne survit dans la réponse finale, et `dispatch-lib` ne peut lire que l'`ESCALATE` du moteur, quel que soit le tier qui le lit.
- R5. Quand aucune ligne `ESCALATE` de la famille de la disposition retirée n'est déclarée, le comportement actuel (marqueur `Disposition-Withheld: REVIEW-ANCHOR-MISSING`) est conservé.

**Consommateur shell**

- R6. `dispatch-lib` reconnaît la ligne de finding du moteur par sa forme complète en début de ligne — jamais par un fragment que le modèle pourrait recopier — pour rendre `ESCALATE` avant tout tier textuel et pour recopier la cause dans `RESULT` avec une raison qui nomme le moteur.

**Prompt (marqueur)**

- R7. Les trois prompts arch producteurs de verdict enseignent : citer le message-brief de ce tour (pas un fichier lu par outil, pas un ticket antérieur de la session), garder la citation sur la ligne d'ancre, copier les mots exactement ; et décrivent la nouvelle issue du second échec.

### Scope Boundaries

- **Hors périmètre — la classe #2331** (tour arch sans réponse après lectures d'outils) : preuve en M6, à ficher séparément avec sa session si elle se reproduit.
- **Hors périmètre — la classe cross-ticket #2296** : fermée par #2295 (M4).
- **Hors périmètre — l'ancre multi-lignes** : le comparateur continue de ne lire que la ligne d'ancre elle-même ; le prompt le dit (R7) plutôt que d'élargir le matcher.
- **Non-objectif** : toute modification de `review_anchor_min_count`, `review_anchor_min_quote_chars`, `required_review_anchor_prefixes` (Key Decisions).

### Deferred to Follow-Up Work

- Ficher la classe « tour arch sans EndTurn après outils » avec `06615128` comme preuve, si un second cas est observé (`feedback_n_equals_2_is_the_signal`).

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Une fonction unique `normalize_for_anchor_match` remplace `normalize_whitespace` aux deux points de comparaison.** Elle retire `*` et `` ` ``, replie `’` `‘` sur `'`, puis replie les espaces — dans cet ordre, pour qu'un `** ` en fin de segment ne laisse pas de double espace. `min_quote_chars` se compte sur le texte normalisé (R2). Le repli d'apostrophe est défensif (M3 : 2 réponses sur 199), retenu parce qu'il coûte une ligne et se teste en une. Les `_` ne sont pas retirés : les identifiants du dépôt en sont pleins et aucun cas observé ne les met en cause.
- KTD2. **Le second échec réécrit la réponse au lieu de la vider, et la réécriture couvre le texte entier.** Trois gestes, dans cet ordre :
  1. La disposition non terminale retirée est celle trouvée par `has_declared_disposition` dans les trois dernières lignes ; sa **famille** (`Disposition:` ou `Verdict:`) désigne la ligne `ESCALATE` à émettre — `Disposition: READY` → `Disposition: ESCALATE`, `Verdict: GROOMED` → `Verdict: ESCALATE`. La ligne choisie doit figurer dans `required_suffix_lines` ; sinon repli sur `withhold_disposition` inchangé (R5). Choisir par famille et non par ordre de liste est porteur : les trois skills arch sont `always_on`, donc `collect_required_suffix_lines` rend l'**union des cinq lignes** sur chaque tour de mika-arch, et un « `Disposition: ESCALATE` d'abord » aurait rendu `Verdict: ESCALATE` inatteignable en production.
  2. **Chaque occurrence** de chaque ligne non terminale déclarée — ligne propre *ou* mention inline dans le corps — est remplacée par la ligne `ESCALATE` choisie (remplacement de sous-chaîne sur tout le texte). Le remplace-ligne-à-ligne de `withhold_disposition` ne suffit pas ici : le tier 1a de `_parse_disposition` et le tier 1 de `_parse_verdict` font `grep -oE … | head -1` sur tout le texte, sans ancrage en début de ligne, et le marqueur qui les court-circuitait au tier 0 disparaît sur cette voie. Une phrase du modèle citant `Disposition: READY` — les prompts en portent l'exemple travaillé, et `body_lines` a déjà dû passer à `rposition` pour cette raison — serait lue avant l'`ESCALATE` de fin. Réécrire une mention inline en `ESCALATE` est sans coût : elle n'a jamais été un verdict.
  3. Une ligne de finding du moteur puis la ligne `ESCALATE` sont ajoutées en fin de texte. La ligne de finding a une **forme fixe, exposée comme constante Rust** à côté de `DISPOSITION_WITHHELD_MARKER` : `<prefix> (BLOCKING) [mika-engine] review-anchor: attestation withheld after the corrective re-prompt — anchors_found=<n>, anchors_valid=<m>, miss_reason=<Reason>: <describe>`. Le préfixe est le premier de `required_finding_list_prefixes`, ou `F1:` s'il n'y en a pas (groom-milestone n'en déclare pas). Cette ligne ne contient jamais une `required_suffix_lines` en clair.
- KTD3. **L'événement `guard.review_anchor_withheld` gagne un champ `emitted`** valant `escalate` ou `withheld-marker`, pour que Signal K distingue les deux issues sans relire le texte. Le nom de l'événement ne change pas : l'analyseur existant continue de compter les refus.
- KTD4. **Le prompt cesse de nommer le marqueur et nomme l'ESCALATE.** Le test `review_anchor_prompt_contract.rs::every_verdict_producer_teaches_the_anchor_contract` exige aujourd'hui que le prompt contienne le littéral du marqueur ; il exigera à la place `[mika-engine]` et `ESCALATE` dans la section « Review-Anchor Attestation Contract », plus les trois règles de citation (R7). Le marqueur reste dans le moteur (repli R5) et dans le tier 0 de `dispatch-lib`, dont le drift guard de `test-dispatch-lib.sh` continue de comparer les deux littéraux.
- KTD5. **`dispatch-lib` reconnaît la ligne du moteur par sa forme complète, ancrée en début de ligne, et à deux endroits.** Le motif est `^[[:space:]]*F[0-9]+: \(BLOCKING\) \[mika-engine\] review-anchor:` — le même littéral que la constante Rust de KTD2, tenu en parité par le drift guard de `test-dispatch-lib.sh`. (a) Un **tier 0b** dans `_parse_disposition` *et* `_parse_verdict`, placé avant le tier 1a/1 : une ligne matchant ce motif rend `ESCALATE` et retourne — le refus du moteur est ainsi fail-closed côté shell indépendamment du remplacement de sous-chaîne de KTD2 (ceinture et bretelles, chacun suffisant seul). (b) `_escalate_groom` extrait la première ligne matchant ce motif, la recopie comme `Engine reason:` dans `RESULT` et fixe `GROOM_LOOP_FAILURE_REASON` à « engine ESCALATE (<stage>): review-anchor attestation withheld ». Un simple « contient `[mika-engine]` » ne convient pas : tous les re-prompts du moteur commencent par ce littéral, le modèle le relit dans sa session et le recopie (cas « quoted marker mid-line » de `test-dispatch-lib.sh`), et U5 le fait entrer dans les prompts — un ESCALATE authentique de l'architecte citant le re-prompt serait requalifié « engine ESCALATE ». Sans ligne moteur, le comportement actuel (« architect ESCALATE ») est conservé.

### Fire-Disposition

- **Quand le guard fire une première fois** : inchangé — re-prompt correctif, événement `guard.review_anchor` (WARN).
- **Quand il fire une seconde fois et qu'un `ESCALATE` de la même famille est déclaré** : réponse réécrite (KTD2), événement `guard.review_anchor_withheld` (ERROR, `emitted=escalate`), `dispatch-lib` tier 0b → `ESCALATE` → `_escalate_groom` → `PIPELINE FAILURE` avec `Engine reason:` dans `tasks.result`, findings préservés dans `.iterate/escalate-<stage>.md`.
- **Quand il fire une seconde fois sans `ESCALATE` déclaré** : marqueur withheld (R5), `emitted=withheld-marker`, tier 0 de `dispatch-lib` → UNPARSED comme aujourd'hui.
- **Ce que le guard ne fait toujours pas** : accepter. Aucune branche ne rend `READY`/`GROOMED` sur une attestation non vérifiée.

### Assumptions

- Le comparateur rejoué hors moteur en M2 reproduit fidèlement `find_brief_quote_range` (fenêtre glissante de 40 caractères, `match_indices`) ; le test U1 sur la paire réelle #2335 le confirmera dans le moteur.
- `_parse_disposition` tier 1a et `_parse_verdict` tier 1 lisent la **première** occurrence dans tout le texte, sans ancrage ; KTD2 (2) et KTD5 (a) en dépendent tous deux, et U4 le vérifie sur un corps qui cite `Disposition: READY` inline.
- Le repli R5 est **inatteignable avec les manifestes livrés** : les trois skills arch étant `always_on`, l'union des suffixes contient toujours les deux lignes `ESCALATE`. Il est conservé comme repli défensif pour un skill futur ou une configuration allégée, et testé comme tel (U2, U3).

---

## Implementation Units

### U1. Comparateur tolérant au balisage inline

- **Goal :** R1, R2 — une citation exacte au mot près passe, une paraphrase ne passe pas.
- **Requirements :** R1, R2 ; KTD1.
- **Dependencies :** aucune.
- **Files :** `crates/mika-agent/src/agent_loop/review_anchor.rs` (fonction + tests inline).
- **Approach :**
  1. Introduire `normalize_for_anchor_match` (KTD1) et l'appliquer là où `normalize_whitespace` est appliqué au brief et à chaque ligne d'ancre.
  2. Mettre à jour le doc-commentaire de module : le contrat est « les mots du brief », pas « les octets du brief » ; citer M2 comme cas fondateur.
  3. Laisser `body_lines`, la dédup de lignes, `find_brief_quote_range` et l'ordre des raisons intacts.
- **Patterns to follow :** le matcher reste sans regex (`mika#864`, en-tête du module) ; indexation par `char`, jamais par octet (`scripts/check-byte-slices.sh`).
- **Test scenarios :**
  - Happy path : la paire réelle #2335 — brief contenant la ligne 63 avec `**` et backticks, ancre A1 en clair — passe de `Missing{3,2,QuoteNotInBrief}` à `Satisfied` avec les deux autres ancres inchangées.
  - Symétrie : brief en clair, ancre portant `**gras**` et `` `code` `` → `Satisfied`.
  - Apostrophe : brief `l'option`, ancre `l’option` (et l'inverse) → `Satisfied`.
  - Longueur : une ancre de 44 caractères dont 6 sont des `*`/backticks tombe sous 40 après normalisation → `QuoteTooShort`, pas `Satisfied`.
  - Négatif : paraphrase (`Le mécanisme de kill est correct`) → `QuoteNotInBrief`.
  - Négatif : citation exacte d'un texte absent du brief (le cas M4) → `QuoteNotInBrief`.
  - Régression : les tests existants du module (`STUB_2037`, chevauchement, dédup, brief vide) restent verts sans modification de leurs attentes.
- **Verification :** `cargo test -p mika-agent review_anchor` vert ; la calibration `run_review_anchor_attestation` (qui appelle le même comparateur) ne change pas de forme.

### U2. Second échec → ESCALATE signé par le moteur

- **Goal :** R3, R4, R5 — la disposition non attestée devient un ESCALATE lisible, jamais un vide.
- **Requirements :** R3, R4, R5 ; KTD2, KTD3.
- **Dependencies :** aucune (indépendant de U1 ; testé ensemble en U3).
- **Files :** `crates/mika-agent/src/agent_loop/mod.rs` (bloc withhold du guard, `withhold_disposition`, nouvelle fonction de réécriture, tests inline).
- **Approach :**
  1. Ajouter une constante `REVIEW_ANCHOR_ENGINE_FINDING_MARKER` (`(BLOCKING) [mika-engine] review-anchor:`) à côté de `DISPOSITION_WITHHELD_MARKER`, avec le même commentaire de parité shell.
  2. Ajouter une fonction pure `escalate_unattested_disposition(text, required_suffix_lines, finding_prefixes, anchors_found, anchors_valid, reason) -> Option<String>` appliquant KTD2 (1)–(3) : `None` quand la ligne `ESCALATE` de la famille retirée n'est pas déclarée.
  3. Au point `text = withhold_disposition(...)`, appeler d'abord cette fonction ; sur `None`, garder `withhold_disposition`.
  4. Composer la ligne de finding avec `reason.describe()` et les trois compteurs ; elle ne doit contenir aucune `required_suffix_lines` en clair.
  5. Ajouter `emitted` à l'événement `guard.review_anchor_withheld` (KTD3) ; conserver la prise de `guard_correlation` (#953).
  6. Mettre à jour le doc-commentaire de `withhold_disposition` : il est désormais le repli, et dire pourquoi la réécriture est une sous-chaîne sur tout le texte (tiers non ancrés de `dispatch-lib`).
- **Patterns to follow :** `withhold_disposition` (texte du modèle conservé) ; `has_declared_disposition` pour trouver la disposition retirée et sa famille ; `is_terminal_disposition` pour la liste des lignes terminales.
- **Test scenarios :** (tous construits avec l'**union réelle des cinq lignes** — `Disposition: READY/ITERATE/ESCALATE` + `Verdict: GROOMED/ESCALATE` — sauf mention contraire, puisque c'est ce que mika-arch voit en production)
  - Happy path premier passage : texte `A1… A2… Disposition: READY` → sortie se termine par `F1: (BLOCKING) [mika-engine] review-anchor: … anchors_found=3, anchors_valid=2, miss_reason=QuoteNotInBrief …` puis `Disposition: ESCALATE` ; le corps du modèle est conservé ; aucun `Disposition: READY` ne subsiste.
  - Second passage : texte se terminant par `Verdict: GROOMED`, même union → `Verdict: ESCALATE` (pas `Disposition: ESCALATE`), aucun `Verdict: GROOMED` ne subsiste.
  - Famille non déclarée : texte se terminant par `Verdict: GROOMED`, skill ne déclarant que les trois `Disposition:` → `None`, le marqueur withheld est émis (R5).
  - Repli complet : skill ne déclarant que `Disposition: READY` → `None`.
  - Préfixe : skill sans `required_finding_list_prefixes` → la ligne commence par `F1:`.
  - Ligne propre citée tôt : corps portant `Disposition: READY` sur une ligne propre avant la conclusion → les deux occurrences sont remplacées.
  - Mention inline : corps portant « … se termine par `Disposition: READY` » dans une phrase, puis la conclusion → aucune occurrence de `Disposition: READY` ne subsiste nulle part dans la sortie ; même chose pour `Verdict: GROOMED` inline au second passage.
  - Contenu du finding : la ligne ne contient ni `READY` ni `GROOMED`, et commence par le préfixe suivi du marqueur constant.
- **Verification :** `cargo test -p mika-agent agent_loop` vert ; `cargo clippy` sans avertissement nouveau.

### U3. Test négatif de bout en bout sur le harness

- **Goal :** la contrainte Prime « arch attesté 2/3 → ESCALATE-avec-raison, pas withhold silencieux », prouvée sur `run_agent()`.
- **Requirements :** R1, R3, R4, R5 ; Key Decision 4.
- **Dependencies :** U1, U2.
- **Files :** `crates/mika-agent/tests/eval/grounding_regressions/review_anchor.rs`.
- **Approach :**
  1. Remplacer `test_review_anchor_withholds_disposition_after_failed_retry` par un test où les deux tours portent 3 ancres dont une paraphrasée (2/3), skill déclarant l'union des cinq lignes : asserte `Disposition: ESCALATE`, `[mika-engine] review-anchor:`, `anchors_valid=2`, `QuoteNotInBrief`, absence de `Disposition: READY`, conservation du corps.
  2. Ajouter le miroir second passage : deux tours à 2/3 se terminant par `Verdict: GROOMED`, même union → `Verdict: ESCALATE`, absence de `Verdict: GROOMED`.
  3. Ajouter un test où le second tour porte 3 ancres dont une cite le brief avec balisage retiré → `Disposition: READY` accepté (U1 vu du harness — le cas #2335 exact, brief de test portant `**` et backticks).
  4. Ajouter un test de repli : skill ne déclarant que les trois `Disposition:` et texte se terminant par `Verdict: GROOMED` → `WITHHELD_MARKER` présent (R5) ; conserver la constante littérale et son commentaire de dérive.
  5. Mettre à jour le doc-commentaire de tête du fichier.
- **Patterns to follow :** `EvalHarness::builder().responses(...).skills(...)`, `grounding_assertions::assert_response_contains/forbids`, `make_anchor_skill`.
- **Test scenarios :** les trois ci-dessus, plus la non-régression de `test_review_anchor_no_op_on_anchored_ready` et `test_review_anchor_caught_on_unanchored_groomed`.
- **Verification :** `cargo test -p mika-agent --test eval review_anchor` vert.

### U4. `dispatch-lib` : la cause du moteur remonte dans `RESULT`

- **Goal :** R6 — l'opérateur lit la cause dans `tasks.result`, pas dans `server.log`.
- **Requirements :** R6 ; KTD5.
- **Dependencies :** U2 (format de la ligne).
- **Files :** `skills/bundled/_shared/dispatch-lib.sh` (`_escalate_groom`), `skills/bundled/_shared/test-dispatch-lib.sh`.
- **Approach :**
  1. Définir une fois le motif de la ligne moteur (`^[[:space:]]*F[0-9]+: \(BLOCKING\) \[mika-engine\] review-anchor:`) et l'utiliser aux trois sites ci-dessous ; étendre le drift guard de `test-dispatch-lib.sh` pour comparer le littéral shell à `REVIEW_ANCHOR_ENGINE_FINDING_MARKER` (même mécanique que le marqueur withheld).
  2. Ajouter un **tier 0b** à `_parse_disposition` et `_parse_verdict`, juste après le tier 0 et avant tout `grep` textuel : si une ligne matche le motif, journaliser et rendre `ESCALATE`.
  3. Dans `_escalate_groom`, extraire la première ligne matchant le motif ; si présente, ajouter `Engine reason: <ligne>` à `RESULT` et fixer `GROOM_LOOP_FAILURE_REASON` à « engine ESCALATE (<stage>): review-anchor attestation withheld ». Les appelants fixent la raison *avant* l'appel ; `_escalate_groom` ne la surcharge que dans ce cas.
  4. Mettre à jour le commentaire de tier 0 : les prompts n'enseignent plus le marqueur ; le tier reste comme repli R5, le tier 0b est la voie nominale.
- **Patterns to follow :** tier 0 existant (`case` sur début de ligne, position load-bearing commentée) ; `_escalate_groom` existant (findings préservés, `RESULT` append) ; drift guard `RUST_MARKER`/`WITHHELD_MARKER` ; assertions `assert_eq` de `test-dispatch-lib.sh`.
- **Test scenarios :**
  - Tier 0b : un texte réécrit par U2 (ligne moteur + `Disposition: ESCALATE`) rend `ESCALATE` par `_parse_disposition` ; le miroir `Verdict:` rend `ESCALATE` par `_parse_verdict`.
  - Tier 0b prime sur le tier 1a : un texte portant `Disposition: READY` inline dans une phrase *avant* la ligne moteur rend `ESCALATE`, pas `READY`.
  - Écho du modèle : un ESCALATE authentique de l'architecte dont une phrase mentionne `[mika-engine]` sans ligne moteur → tier 0b muet, `RESULT` sans `Engine reason:`, raison « architect ESCALATE » inchangée.
  - `_escalate_groom` avec ligne moteur → `RESULT` contient `Engine reason:` et la ligne ; `GROOM_LOOP_FAILURE_REASON` nomme le moteur.
  - Tier 0 : le marqueur withheld en début de ligne rend toujours vide (non-régression) ; les deux drift guards Rust↔shell sont verts.
- **Verification :** `bash skills/bundled/_shared/test-dispatch-lib.sh` vert.

### U5. Prompts arch : enseigner ce que le comparateur vérifie

- **Goal :** R7 — kimi-arch produit trois ancres vérifiables sans re-prompt, et sait ce qui arrive sinon.
- **Requirements :** R7 ; KTD4 ; Key Decision 3.
- **Dependencies :** U2 (comportement à décrire).
- **Files :** `skills/bundled/mika-arch-groom-ticket/system_prompt.md`, `skills/bundled/mika-arch-groom-milestone/system_prompt.md`, `skills/bundled/mika-arch-second-review/system_prompt.md`, `crates/mika-agent/tests/review_anchor_prompt_contract.rs`.
- **Approach :**
  1. Dans la section « Review-Anchor Attestation Contract » de chaque prompt, remplacer le paragraphe « stripped … replaced with `Disposition-Withheld: …` » par la description du second échec : la disposition est remplacée par `Disposition: ESCALATE` (ou `Verdict: ESCALATE`) et une ligne `F1: … [mika-engine]` nomme la cause ; le groom échoue avec cette raison.
  2. Ajouter trois règles sous « What does NOT satisfy the contract » : (a) citer un texte lu par outil ou un ticket antérieur de la session — seul le message-brief de ce tour est comparé ; (b) une citation qui continue sur la ligne suivante — seule la ligne d'ancre est lue, garder ≥ 40 caractères sur elle ; (c) une citation dont les mots diffèrent — le balisage `**`/backticks et la forme de l'apostrophe sont tolérés, les mots et la ponctuation ne le sont pas.
  3. Dans `mika-arch-groom-milestone/system_prompt.md`, **retirer** la phrase « the anchors quote the sub-issue plans you actually read — three distinct regions across the milestone's plan set » : elle contredit (a) — le brief milestone porte les décisions-clés par sous-ticket, pas le texte des plans, et un plan lu par `gh_read` n'est jamais dans le texte comparé. La remplacer par : les ancres citent le brief milestone lui-même, trois régions distinctes (sections par sous-ticket, dépendances, séquencement), jamais les fichiers de plan.
  4. Mettre à jour `every_verdict_producer_teaches_the_anchor_contract` : exiger `[mika-engine]`, `ESCALATE` et la phrase-clé « message-brief de ce tour » (ou son équivalent anglais retenu) ; retirer l'assertion sur le marqueur ; ajouter l'assertion d'absence de la phrase milestone retirée.
  5. Garder l'exemple READY à trois ancres (test `the_worked_ready_example_carries_three_anchors`).
- **Patterns to follow :** ton et structure existants de la section ; `feedback_prompt_enforcement_fragile` — le prompt explique, le moteur impose.
- **Test scenarios :**
  - `review_anchor_prompt_contract` vert sur les trois skills avec les nouvelles assertions.
  - `make verify-bundled-skills` (ou son équivalent CI) vert.
- **Verification :** tests ci-dessus ; relecture des trois diffs pour l'absence de contradiction avec KTD1 (ne pas dire « byte-exact »).

---

## Verification Contract

| Preuve | Commande | Unités |
|---|---|---|
| Comparateur | `cargo test -p mika-agent review_anchor` | U1 |
| Moteur | `cargo test -p mika-agent agent_loop` et `cargo clippy -p mika-agent` | U2 |
| Harness | `cargo test -p mika-agent --test eval review_anchor` | U3 |
| Shell | `bash skills/bundled/_shared/test-dispatch-lib.sh` | U4 |
| Contrat de prompt | `cargo test -p mika-agent --test review_anchor_prompt_contract` | U5 |
| Global | `cargo test` et `cargo fmt --check` | toutes |

Pas de calibration exigée : aucun modèle ne change (`docs/solutions/best-practices/an-exempted-disposition-is-the-attack-surface-2026-08-30.md` § What this deliberately does not do).

---

## Definition of Done

- [ ] U1–U5 livrées dans la branche `feat/2338/grooming-ne-reconverge-pas-arch-met`, tests de la Verification Contract verts.
- [ ] La paire réelle #2335 (M2) est un test du comparateur et passe.
- [ ] Le test négatif de bout en bout (U3) prouve `ESCALATE` + cause, jamais un `READY`.
- [ ] Aucun changement dans `skill.toml` des trois skills arch.
- [ ] Le corps de mika#2338 reçoit un commentaire portant M1–M6 (la citation exacte qui rate et le fragment le plus proche, comme le ticket le demande).
- [ ] Aucun code d'essai abandonné dans le diff.
- [ ] PR ouverte avec `Closes #2338`, gated QA ; merge par l'opérateur.

## Acceptance criteria

- [ ] AC1 — `verify_review_anchors` sur le brief et la réponse réels de #2335 (message 87416 / appel `8abf1b1b`) rend `Satisfied` ; sur une paraphrase de A1 il rend `Missing{…, QuoteNotInBrief}`.
- [ ] AC2 — Sur le harness eval, deux tours à 2/3 ancres valides produisent une réponse finale dont la dernière ligne non vide est `Disposition: ESCALATE`, contenant une ligne `[mika-engine]` avec `anchors_valid=2` et `QuoteNotInBrief`, et ne contenant aucun `Disposition: READY`.
- [ ] AC3 — Un skill ne déclarant pas de ligne `ESCALATE` produit toujours `Disposition-Withheld: REVIEW-ANCHOR-MISSING`.
- [ ] AC4 — `test-dispatch-lib.sh` prouve que le tier 0b rend `ESCALATE` sur un texte réécrit même quand le corps cite `Disposition: READY` inline, que `_escalate_groom` recopie la ligne moteur dans `RESULT` et nomme le moteur dans la raison, qu'un écho de `[mika-engine]` sans ligne moteur ne déclenche rien ; le tier 0 et les deux drift guards restent verts.
- [ ] AC7 — Sur l'union des cinq lignes de suffixe (configuration réelle de mika-arch), un second passage retiré rend `Verdict: ESCALATE` et un premier passage retiré rend `Disposition: ESCALATE`.
- [ ] AC5 — Les trois prompts arch décrivent l'issue `ESCALATE` et les trois règles de citation ; `review_anchor_prompt_contract` l'asserte.
- [ ] AC6 — `review_anchor_min_count = 3` et `review_anchor_min_quote_chars = 40` sont inchangés dans les trois `skill.toml`.

---

## Risques et voisinage

- **Les tiers textuels de `dispatch-lib` ne sont pas ancrés.** Sur la voie ESCALATE le tier 0 (marqueur) ne joue plus ; une mention inline de `READY`/`GROOMED` dans le corps serait lue en premier. Couvert deux fois, chaque couche suffisant seule : KTD2 (2) réécrit chaque occurrence dans le texte, KTD5 (a) rend `ESCALATE` au tier 0b avant tout `grep`.
- **KTD1 normalise sur un cas mesuré (n=1) et un comptage borné à trois caractères.** Les briefs portent aussi des liens `[texte](url)`, des échappements `\_` et des puces ; un modèle citant le rendu les retire de la même façon. Si la prochaine classe de `QuoteNotInBrief` est l'une de celles-là, le second échec devient un ESCALATE lisible (ce plan) mais le groom ne reconverge pas — la forme de KTD1 est prête à recevoir la normalisation suivante, avec sa citation exacte et son test, quand elle sera mesurée.
- **Calibration mika-arch** : `run_review_anchor_attestation` appelle le comparateur ; U1 ne peut que faire passer plus d'attestations légitimes. Aucun baseline à mettre à jour.
- **Critère de réouverture du drain (Prime)** : #2338 mergé **et** un groom qui reconverge sous le guard actif. Ce plan livre le premier ; le second se mesure sur le prochain groom substrat après déploiement (`feedback_never_conclude_inside_the_mechanism_period`).
- **Voisinage** : `docs/plans/2026-08-30-002-fix-2037-disposition-ready-sans-attestation-plan.md` (R6 y décrivait le withhold que ce plan remplace) ; `docs/solutions/best-practices/an-exempted-disposition-is-the-attack-surface-2026-08-30.md` ; mika#2295 (fenêtre `scope="session"`, M4) ; mika#2296/#2332 (budget de sortie, qui a rendu ce défaut visible).
