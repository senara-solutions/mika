---
title: Attestation d'ancrage de revue sur disposition non terminale - Plan
type: fix
date: 2026-08-30
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Attestation d'ancrage de revue sur disposition non terminale - Plan

## Goal Capsule

- **Objectif** — Un plan qui entre en implémentation avec la mention « validé par l'architecte » a réellement été revu. L'opérateur qui lit la lignée de commit d'une branche de grooming peut se fier au premier commit : il atteste une revue, pas un accusé de réception. Vérifiable sans connaître les internes du moteur : on relit la réponse d'architecte attachée à une disposition et on y trouve la revue.
- **Moyens** — Étendre la famille de gardes de post-condition `[output]` par son complément manquant : les dispositions non terminales exigent une attestation d'ancrage vérifiable mécaniquement contre le brief (KTD1), avec un refus fail-closed appliqué aux deux couches de la chaîne (KTD4).
- **Hiérarchie d'autorité** — Le corps de mika#2037 et les quatre exigences de conception de l'opérateur priment sur ce plan. Ce plan prime sur les préférences de l'implémenteur. La matrice de mesure (U7) prime sur les seuils écrits ici : un seuil que la matrice invalide se corrige, et la correction se documente.
- **Conditions d'arrêt** — S'arrêter et router vers l'opérateur si la matrice montre qu'aucun seuil ne sépare une vraie revue d'une réponse-croupion — cela invaliderait KTD1 et rouvrirait le choix de l'artefact d'attestation.
- **Profil d'exécution** — Le matcher se mesure avant d'entrer dans le fichier (U3 avant U4), conformément à `docs/solutions/best-practices/groomed-plan-is-a-shape-contract-not-a-fact-contract-2026-08-27.md`.
- **Propriété de la queue** — L'implémenteur ouvre la PR et ajoute `mika-platform-qa` comme relecteur. La fusion appartient à mika-dev.

---

## Product Contract

### Summary

Ajouter une garde de post-condition qui exige, sur toute disposition **non terminale** (`Disposition: READY`, `Verdict: GROOMED`), une attestation d'ancrage : des lignes préfixées dont le contenu cite verbatim le brief revu, à des positions distinctes. Sans attestation valide, le moteur retire la disposition, la remplace par un marqueur d'invalidation littéral, et `dispatch-lib` refuse de dériver un verdict de la réponse — y compris par son appariement flou. Une absence d'attestation devient une absence de verdict, jamais un accord.

### Problem Frame

Le 2026-08-29, pendant le grooming manuel de mika#2013, mika-arch a rendu `Disposition: READY` sur une revue de plan qu'il n'a pas faite. Le brief faisait 10 492 octets et posait quatre questions numérotées. La réponse faisait 302 octets, en 114 secondes (`session_id` `d6411163-6bc0-47d9-b642-954adf8d3f64`) :

> Préférence stockée — le pattern de re-résolution par cycle et le seuil N=3 pour mika#2013.
>
> Disposition: READY

Aucune des quatre questions n'est abordée. Aucun code n'est cité. Une décision est hallucinée — le brief disait littéralement « Je n'ai pas fixé N », la réponse affirme un seuil N=3. Et le mot-clé qui fait avancer la chaîne est émis quand même.

Ce mot-clé n'est pas décoratif. `/mika-groom-ticket` Phase 3 étape 10 parse `Disposition: READY` et commite le plan comme validé par l'architecte. Toute la discipline de staging-avant-commit existe pour que le premier commit d'une branche de grooming porte la signature d'une revue. Un READY creux forge cette lignée : le plan part en implémentation avec l'apparence d'avoir passé la porte de qualité.

La cause n'est pas le modèle. C'est une asymétrie dans la famille de gardes existante. `required_finding_list_prefixes` (mika#901) exige une F-list, mais `is_terminal_disposition()` (`crates/mika-agent/src/agent_loop/mod.rs:5352`) ne considère terminal que `ITERATE`, `ESCALATE` et `Verdict: ESCALATE`. `READY` et `Verdict: GROOMED` sont exemptés par conception — le system_prompt le dit explicitement : « On READY, the F-list is NOT required — the message may stay short since no iteration is needed » (`skills/bundled/mika-arch-groom-ticket/system_prompt.md:124`). **La seule disposition qui fait avancer la chaîne est la seule qui n'exige aucune preuve.** N'importe quel modèle assis sur ce siège hérite du trou.

Le défaut est déjà à n=2. mika#1957 a reçu un `READY` de premier passage sur un plan portant deux prémisses fausses, dont une qui aurait régressé une conception livrée (`docs/solutions/best-practices/groomed-plan-is-a-shape-contract-not-a-fact-contract-2026-08-27.md`).

### Requirements

**Contrat d'attestation**

- R1. Sur une disposition non terminale (`Disposition: READY`, `Verdict: GROOMED`), un skill peut exiger une attestation d'ancrage : au moins `review_anchor_min_count` lignes du corps du message commençant par un préfixe déclaré dans `required_review_anchor_prefixes`.
- R2. Chaque ligne d'ancrage est valide seulement si elle contient une sous-chaîne d'au moins `review_anchor_min_quote_chars` caractères présente **verbatim** dans le brief de la requête.
- R3. Deux lignes d'ancrage ne peuvent pas se satisfaire de la même région du brief : les intervalles de correspondance retenus dans le brief ne se chevauchent pas.
- R4. Un skill sans `required_review_anchor_prefixes` déclaré n'est pas affecté. La garde est opt-in, comme le reste de la famille `[output]`.

**Refus fail-closed**

- R5. Quand l'attestation manque, le moteur émet un unique re-prompt correctif nommant le contrat violé et ce qui satisfait la garde.
- R6. Quand le re-prompt échoue, le moteur **ne laisse pas sortir la disposition** : il retire la ligne de disposition du texte final, insère à sa place le marqueur littéral `Disposition-Withheld: REVIEW-ANCHOR-MISSING`, et conserve le corps de la réponse intact. Aucune information n'est perdue ; seule l'attestation non méritée est retirée.
- R7. `dispatch-lib` court-circuite sur ce marqueur avant tout appariement : ni le tier 1a littéral, ni le tier 1b de report de verdict, ni le tier 2 flou ne peuvent dériver une disposition d'une réponse marquée. La fonction n'émet rien.
- R8. Une réponse sans disposition dérivable reste traitée comme `UNPARSED` par le consommateur existant — retry borné puis échec de pipeline. Aucun chemin ne dégrade une absence en `READY`.

**Portée des skills**

- R9. Les trois skills producteurs de verdict déclarent le contrat : `mika-arch-groom-ticket`, `mika-arch-second-review`, `mika-arch-groom-milestone`.
- R10. Les system_prompts décrivent le contrat d'attestation et n'affirment plus qu'une réponse READY peut rester brève et sans preuve.

**Validation du manifeste**

- R11. Une déclaration incohérente échoue bruyamment à la validation du manifeste : préfixe vide, liste explicitement vide, `review_anchor_min_count` à zéro, `review_anchor_min_quote_chars` sous un plancher de sûreté.

### Success Criteria

- Le croupion littéral de mika#2037, rejoué contre le matcher avec son brief d'origine, ne produit aucune disposition.
- Les réponses de revue réelles du corpus de mesure produisent toutes leur disposition, sans exception à justifier.
- La garde est indifférente au modèle : aucun identifiant de modèle, de fournisseur ou de siège n'apparaît dans son code ni dans sa configuration.

### Scope Boundaries

**Hors périmètre — décidé, pas oublié**

- **La couverture des questions numérotées du brief.** Elle a été évaluée comme second volet de l'attestation et écartée : détecter « les points numérotés » d'un brief en texte libre est une heuristique qui rate les briefs non numérotés (garde inerte) et se déclenche sur la numérotation interne du plan cité (faux positif). L'ancrage verbatim est binaire et ne dépend d'aucune convention de rédaction du brief. Rouvrir seulement si la matrice montre qu'un croupion plausible passe l'ancrage.
- **Le changement de modèle sur le siège mika-arch.** Exigence explicite de l'opérateur. `openrouter/moonshotai/kimi-k2.5` a produit cette réponse ce jour-là ; le défaut est qu'aucune garde ne l'a rattrapée. Un correctif qui changerait de modèle laisserait la porte ouverte au suivant. La deuxième piste du corps de l'issue — instruire modèle contre routage de compétence — reste une investigation valable, mais elle ne conditionne pas cette garde et ne la modifie pas.
- **L'élargissement de `is_terminal_disposition()`.** Rendre `READY` terminal ferait exiger une F-list sur un plan sans objection, ce qui pousserait le modèle à fabriquer des findings. La garde d'ancrage est un mécanisme distinct pour la moitié non terminale, pas une extension de mika#901.

**Reporté à un travail de suivi**

- Le durcissement de `_parse_disposition_fuzzy` (tier 2) au-delà du court-circuit de R7. Le tier 2 reste une surface d'appariement permissive ; ce plan la neutralise sur le chemin marqué, sans la refondre.

### Sources

- Corps de mika#2037 — mesure, session_id, réponse intégrale.
- `crates/mika-agent/src/agent_loop/mod.rs:2340-2410` — garde mika#901, forme de référence.
- `crates/mika-agent/src/agent_loop/mod.rs:5352` — `is_terminal_disposition()`, l'exemption qui est le trou.
- `crates/mika-agent/src/agent_loop/mod.rs:3216` — `collect_required_tools(&matched, params.user_message)`, précédent d'une garde qui consomme le message utilisateur.
- `skills/bundled/_shared/dispatch-lib.sh:2862` — `_parse_disposition`, les trois tiers.
- `skills/bundled/_shared/dispatch-lib.sh:3363-3430` — le retry `UNPARSED` borné et l'échec de pipeline qui rendent R8 déjà vrai.
- `docs/solutions/architecture-patterns/required-finding-list-guard-conditional-disclosure-evasion-2026-05-13.md` — le patron de garde moteur que ce plan étend.
- `docs/solutions/best-practices/groomed-plan-is-a-shape-contract-not-a-fact-contract-2026-08-27.md` — mika#1957, deuxième instance de la classe, et la discipline « mesurer le matcher avant de l'écrire ».

---

## Planning Contract

### Key Technical Decisions

- KTD1. **L'artefact d'attestation est une citation verbatim ancrée dans le brief, pas une réponse aux questions ni un verdict par critère.** Trois candidats étaient sur la table (exigence 1 de l'opérateur). La réponse aux questions dépend d'une numérotation que le brief n'a pas toujours. Le verdict par critère dépend d'une liste de critères que le brief ne porte pas toujours non plus, et il est trivialement satisfaisable par une grille de « OK » sans contenu. La citation verbatim est la seule des trois que le moteur peut valider contre une source qu'il possède déjà — le brief est dans `request.messages` — et que rien d'autre qu'une lecture du brief ne peut produire. Gouverne R1, R2, R3.
- KTD2. **Le mordant vient de la dispersion, pas du volume.** Une seule citation de 40 caractères serait franchissable en recopiant la première ligne du brief. Trois citations distinctes, à des positions non chevauchantes d'un brief de 10 Ko, ne sont pas un sous-produit d'un accusé de réception : il faut avoir parcouru le document. Valeurs initiales : `review_anchor_min_count = 3`, `review_anchor_min_quote_chars = 40`. Ces valeurs sont des hypothèses que U7 mesure ; la matrice, pas ce paragraphe, en est l'autorité. **Mesuré : confirmées.** Le balayage de U7 trouve neuf paires séparantes — `(2,24) (2,32) (2,40) (2,56) (3,16) (3,24) (3,32) (3,40) (3,56)` — et les valeurs livrées sont au centre de cette région, pas sur un fil. `min_count = 1` ne sépare jamais, ce qui mesure la raison du choix de 3. Gouverne R2, R3.
- KTD3. **La garde vit au moteur, avec un verrou complémentaire au shell.** Le moteur est la seule couche qui peut *corriger* — il re-prompte et donne au modèle une chance de produire la revue qu'il sait faire (le corps de mika#2037 rapporte qu'un recadrage explicite a produit une revue substantielle et un `ITERATE` juste). Le shell ne peut que rejeter. Mais le moteur seul ne suffit pas : voir KTD4. Gouverne R5, R6, R7.
- KTD4. **Retirer la disposition ne suffit pas — il faut un marqueur que le shell reconnaît.** `_parse_disposition` a trois tiers, et le tier 2 apparie des paraphrases (`proceed`, `dispatch`, `good to go`, `plan is clean`) n'importe où dans le texte. Une réponse dont on retire seulement la ligne `Disposition: READY` reste susceptible de produire `READY` par appariement flou sur son propre corps. Le moteur insère donc un marqueur littéral, et `_parse_disposition` / `_parse_verdict` court-circuitent dessus **avant le tier 1a**. Sans ce court-circuit, le refus du moteur est contournable par le consommateur. Gouverne R6, R7.
- KTD5. **Le fail-closed est une réécriture du texte, pas une nouvelle variante de `LoopResult`.** `agent_loop/mod.rs` ne contient aucun `bail!` ni `return Err` dans la chaîne de gardes : le refus dur n'existe pas comme mécanisme, et l'introduire demanderait une variante de `LoopResult` et sa gestion aux trois sites d'appel — surface large pour un bénéfice nul, puisque R8 est déjà vrai chez le consommateur. La réécriture obtient le même fail-closed en touchant une seule couche. Elle ne masque rien : le corps de la réponse est conservé intégralement et le marqueur nomme la raison. Gouverne R6.
- KTD6. **La garde est opt-in par manifeste, comme toute la famille `[output]`.** Aucun comportement par défaut n'est modifié pour un skill qui ne déclare rien. Cela rend le déploiement progressif et laisse la trace de conception dans le `skill.toml`, là où les trois gardes existantes la laissent déjà. Gouverne R4, R9.

### High-Level Technical Design

Chaîne actuelle des post-conditions EndTurn, et l'insertion proposée :

```
EndTurn du modèle
  │
  ├─ … gardes amont (outils requis, fabrication, persistance) …
  │
  ├─ #864  required_suffix_lines
  │        la réponse finit-elle par une disposition déclarée ?
  │        non → 1 re-prompt → puis ACCEPTE (fail-open)
  │
  ├─ #901  required_finding_list_prefixes
  │        disposition TERMINALE (ITERATE / ESCALATE) ?
  │        oui → F-list requise → 1 re-prompt → puis ACCEPTE (fail-open)
  │        non (READY / GROOMED) → EXEMPTÉ  ◄── le trou de #2037
  │
  └─ #2037 required_review_anchor_prefixes           ◄── NOUVEAU
           disposition NON TERMINALE (READY / GROOMED) ?
           oui → attestation d'ancrage requise
                 │
                 ├─ ≥ min_count lignes préfixées
                 ├─ chacune contient ≥ min_quote_chars caractères verbatim du brief
                 └─ régions du brief non chevauchantes
                 │
                 satisfait → ACCEPTE
                 non → 1 re-prompt correctif
                       │
                       toujours non → RETIRE la ligne de disposition,
                                      INSÈRE `Disposition-Withheld: REVIEW-ANCHOR-MISSING`,
                                      conserve le corps  (fail-CLOSED)
```

Les deux gardes couvrent alors l'espace entier des dispositions : mika#901 la moitié terminale, mika#2037 la moitié non terminale. Aucune disposition ne sort sans preuve de sa propre classe.

Chaîne de bout en bout après le refus, montrant pourquoi le verrou shell est nécessaire :

```
moteur ──► texte marqué ──► a2a / mika ask ──► dispatch-lib `_parse_disposition`
                                                 │
                                                 ├─ tier 0 : marqueur présent ? ──► n'émet RIEN   ◄── NOUVEAU
                                                 ├─ tier 1a : `Disposition: X` littéral    (retirée par le moteur)
                                                 ├─ tier 1b : `Verdict: GROOMED` → READY   (retirée par le moteur)
                                                 └─ tier 2  : paraphrase floue sur le corps ◄── le contournement
                                                                                                que le tier 0 ferme
                                                 │
                                              rien émis
                                                 │
                                    `_iterate_groom_loop` → UNPARSED
                                    → 1 retry borné (mika#1823) → échec de pipeline
```

Structure du verdict du matcher (fonction pure, testable sans moteur) :

```
verify_review_anchors(text, brief, prefixes, min_count, min_quote_chars)
  → Satisfied
  | Missing { anchors_found, anchors_valid, reason }

  reason ∈ { PasDeLigneAncrage, CitationTropCourte, CitationAbsenteDuBrief, RegionsChevauchantes }
```

Le verdict porte la raison pour que le re-prompt correctif dise au modèle laquelle des quatre conditions il a manquée, plutôt que de répéter le contrat en bloc.

### Assumptions

Ces paris sont inférés du terrain, pas confirmés par l'opérateur. Ils sont ici pour être invalidés tôt.

- A1. Le brief pertinent est le message utilisateur de la requête. Si un skill devait ancrer contre un autre contenu (une pièce jointe, un fichier récupéré par outil), la garde le manquerait. Aucun des trois skills visés n'est dans ce cas — leurs briefs arrivent en message utilisateur.
- A2. Le corpus de vraies revues disponible pour la matrice (fixtures de calibration existantes, réponses d'architecte retrouvables) est représentatif des revues READY que la garde doit laisser passer. Si la matrice n'a que des exemples ITERATE, l'anti-vacuité sens 1 n'est pas mesurée — U7 doit alors construire les cas READY manquants à partir de revues réelles, pas d'exemples inventés pour passer la garde.
- A3. Les revues READY légitimes actuellement produites échoueront la garde au premier passage, parce que le contrat qu'elles suivent (`system_prompt.md:124`, exemple ligne 142) autorise explicitement la brièveté. Le re-prompt correctif est le mécanisme prévu pour cette transition, et U5 supprime la prescription qui la cause. Un pic transitoire de re-prompts sur le siège mika-arch est attendu et n'est pas une régression.

### Sequencing

U1 → U2 (le manifeste avant sa validation). U3 est indépendant et vient tôt : le matcher se mesure avant d'être câblé (leçon `groomed-plan-is-a-shape-contract`). U4 dépend de U1 et U3. U5 dépend de U1 (les champs doivent exister pour être déclarés). U6 et U8 sont indépendants. U7 dépend de U3 et clôt le plan — c'est lui qui autorise ou corrige les seuils de KTD2.

### Corrections post-plan

Deux corrections trouvées à l'implémentation. Toutes deux changent le mécanisme sans changer le livrable, donc portées ici plutôt que routées vers l'architecte.

- C1. **Le préfixe d'ancrage était compté dans la fenêtre de citation.** La première rédaction du matcher cherchait la citation dans la ligne entière, `A1: ` compris — un préfixe plus long achetait donc une citation plus courte. Le préfixe déclaré est retiré avant recherche. Trouvé par le test de borne de U3, pas par relecture : la valeur écrite dans le plan (40) mesurait en réalité 36 caractères de contenu.
- C2. **Les blancs sont normalisés des deux côtés avant comparaison.** Non prévu au plan. Un relecteur qui cite un plan re-formate ce qu'il lève : le retour à la ligne qui coupait la phrase dans un brief de 10 Ko ne survit pas dans une citation d'une ligne. Sans normalisation, une citation authentique échouait sur le retour à la ligne du brief — un faux rejet portant précisément sur la réponse que la garde doit laisser passer. Couvert par `line_wrapping_in_the_brief_does_not_break_a_genuine_quote`.

### Écart au périmètre, assumé

- U6 livre **un** scénario de calibration au lieu des deux prévus. Le second (« croupion refusé ») n'est pas exprimable comme comportement de modèle : on ne demande pas à un modèle de produire un croupion. Ce sens est couvert structurellement par la matrice de U7 et les sept scénarios eval de U4.

---

## Implementation Units

### U1. Champs de manifeste pour l'attestation d'ancrage

- **Goal** — Le contrat d'ancrage est déclarable dans `[output]` d'un `skill.toml`.
- **Requirements** — R1, R2, R3, R4.
- **Files** — `crates/mika-agent/src/skills/manifest.rs`.
- **Approach** — Ajouter trois champs à `Output`, tous `#[serde(default)]` pour que l'absence reste le comportement actuel : `required_review_anchor_prefixes: Vec<String>`, `review_anchor_min_count: usize`, `review_anchor_min_quote_chars: usize`. Documenter chacun dans le doc-comment de `Output` en citant mika#2037, comme les champs voisins citent mika#864 et mika#901. Étendre `Output::is_empty()` pour tenir compte du nouveau champ de préfixes. Suivre le patron `RequiredToolArgSuffix` pour les valeurs par défaut si des constantes nommées sont préférables à des littéraux.
- **Test Scenarios**
  - Un `[output]` déclarant les trois champs parse et expose les valeurs exactes.
  - Un `[output]` sans les champs laisse `required_review_anchor_prefixes` vide et `Output::is_empty()` inchangé dans son verdict pour les manifestes existants.
  - Un `skill.toml` complet du terrain (celui de `mika-arch-groom-ticket` augmenté) parse en une passe avec ses quatre contrats `[output]` et `[constraints]` coexistants.
  - `Output::is_empty()` retourne faux quand seul le contrat d'ancrage est déclaré.
- **Verification** — `cargo test -p mika-agent skills::manifest`.

### U2. Validation bruyante du contrat d'ancrage

- **Goal** — Une déclaration incohérente échoue au chargement du skill, pas au premier verdict.
- **Requirements** — R11.
- **Files** — `crates/mika-agent/src/skills/index.rs`.
- **Approach** — Ajouter un bloc de validation à côté de la validation `required_finding_list_prefixes` existante (~ligne 1024), en suivant sa forme exacte, y compris la détection de la liste explicitement vide via la table TOML brute. Rejeter : un préfixe vide ou uniquement blanc ; une liste explicitement vide ; `review_anchor_min_count == 0` ; `review_anchor_min_quote_chars` sous un plancher de sûreté nommé. Le plancher existe parce qu'un seuil de quelques caractères rendrait R2 satisfaisable par n'importe quel mot commun du brief — il neutralise la garde sans la désactiver visiblement.
- **Test Scenarios**
  - Préfixe vide → erreur de validation nommant le champ et l'index.
  - Liste explicitement vide (`required_review_anchor_prefixes = []`) → erreur, comme pour le champ jumeau.
  - `review_anchor_min_count = 0` → erreur.
  - `review_anchor_min_quote_chars` sous le plancher → erreur nommant le plancher.
  - Déclaration cohérente → validation passe.
- **Verification** — `cargo test -p mika-agent skills::index`.

### U3. Le matcher d'ancrage, mesuré avant d'être câblé

- **Goal** — Une fonction pure décide si un texte porte une attestation d'ancrage contre un brief, et sa décision est mesurée sur une table de cas avant qu'elle n'entre dans la chaîne de gardes.
- **Requirements** — R1, R2, R3.
- **Files** — `crates/mika-agent/src/agent_loop/mod.rs` (ou un module frère si la taille du fichier le justifie — l'implémenteur tranche en suivant l'organisation existante des helpers `is_terminal_disposition` / `collect_*`).
- **Approach** — Écrire `verify_review_anchors(text, brief, prefixes, min_count, min_quote_chars) -> AnchorVerdict`. Découper le corps du message jusqu'au repère de ligne de disposition, exactement comme la garde mika#901 découpe pour la F-list — même fenêtre de scan, pour que les deux gardes ne divergent pas sur ce qu'est « le corps ». Pour chaque ligne d'ancrage, chercher la plus longue sous-chaîne présente verbatim dans le brief ; retenir sa position ; valider la longueur contre `min_quote_chars` ; rejeter une correspondance dont l'intervalle chevauche celui d'un ancrage déjà retenu. Aucune regex — la famille de gardes a un précédent anti-regex explicite (mika#864 : « regex is a footgun — silent failure to fire when pattern is malformed »), et un matcher de garde qui échoue silencieusement est exactement le défaut qu'on ferme. Normaliser les blancs avant comparaison si la matrice montre que le retour à la ligne casse des citations légitimes ; ne pas normaliser plus que ce que la matrice exige.
- **Execution note** — Le test précède le câblage. La table de cas de U7 est écrite contre cette fonction et doit tourner avant U4.
- **Test Scenarios**
  - Le croupion littéral de mika#2037 contre son brief → `Missing`, raison `PasDeLigneAncrage`.
  - Trois ancrages citant trois régions distinctes du brief → `Satisfied`.
  - Trois ancrages citant tous la même phrase du brief → `Missing`, raison `RegionsChevauchantes`.
  - Trois ancrages dont un cite une phrase absente du brief → `Missing`, raison `CitationAbsenteDuBrief`.
  - Trois ancrages dont un cite 39 caractères pour un seuil de 40 → `Missing`, raison `CitationTropCourte` (test de borne, inclusif et exclusif).
  - Deux ancrages valides pour `min_count = 3` → `Missing`.
  - Ancrage placé après la ligne de disposition → non compté (le corps s'arrête au repère).
  - Brief vide → `Missing`, jamais `Satisfied` par vacuité.
- **Verification** — `cargo test -p mika-agent verify_review_anchors`.

### U4. Câblage de la garde et refus fail-closed

- **Goal** — La garde s'exécute sur disposition non terminale et, en cas d'échec après re-prompt, la disposition ne sort pas.
- **Requirements** — R1, R5, R6.
- **Files** — `crates/mika-agent/src/agent_loop/mod.rs`.
- **Approach** — Ajouter `collect_review_anchor_contract(&matched)` sur le modèle de `collect_required_finding_list_prefixes` (~5317) et le câbler aux trois sites d'appel de `run_loop` (~3217, ~4370, ~4914) comme les collecteurs voisins. Obtenir le brief : `params.user_message` est déjà en portée au site d'appel et déjà passé à `collect_required_tools` (~3216) — suivre ce précédent plutôt que de fouiller `request.messages` dans la boucle. Placer la garde immédiatement après la garde mika#901, avec le même préambule de conditions (`!skip_remaining_guards`, `EndTurn`, drapeau de retry, contrat non vide) et une condition de disposition **non terminale** — le complément de `is_terminal_disposition()`, exprimé en réutilisant ce helper plutôt qu'en dupliquant sa fenêtre de scan. Le premier échec pousse le couple (réponse assistante, re-prompt correctif) et `continue`, comme ses voisines ; le re-prompt nomme la raison portée par `AnchorVerdict`. Le second échec ne `continue` pas : il réécrit le texte final — ligne de disposition retirée, `Disposition-Withheld: REVIEW-ANCHOR-MISSING` insérée à sa place, corps conservé — puis laisse le tour se terminer normalement. Émettre un `warn!` et un événement de télémétrie de garde en suivant `GuardCorrelation` (mika#953), pour que le refus soit visible en audit et pas seulement dans le texte.
- **Test Scenarios**
  - Disposition READY sans ancrage → un re-prompt correctif est émis, contenant la raison du verdict.
  - Disposition READY sans ancrage après re-prompt → le texte final ne contient plus `Disposition: READY` et contient le marqueur ; le corps d'origine est intégralement présent.
  - Disposition READY avec ancrage valide → aucun re-prompt, texte inchangé.
  - Disposition ITERATE sans ancrage → la garde ne fire pas (moitié terminale, propriété de mika#901).
  - `Verdict: GROOMED` sans ancrage → la garde fire (deuxième disposition non terminale).
  - Skill sans contrat d'ancrage déclaré → la garde ne fire jamais, quel que soit le texte.
  - Le drapeau de retry de cette garde est indépendant de ceux de mika#864 et mika#901 (un re-prompt de F-list ne consomme pas le re-prompt d'ancrage).
- **Verification** — `cargo test -p mika-agent agent_loop`.

### U5. Déclaration et contrat de prompt sur les trois skills

- **Goal** — Les trois producteurs de verdict exigent l'attestation, et leur prompt décrit ce qu'elle est.
- **Requirements** — R9, R10.
- **Files** — `skills/bundled/mika-arch-groom-ticket/skill.toml`, `skills/bundled/mika-arch-groom-ticket/system_prompt.md`, `skills/bundled/mika-arch-second-review/skill.toml`, `skills/bundled/mika-arch-second-review/system_prompt.md`, `skills/bundled/mika-arch-groom-milestone/skill.toml`, `skills/bundled/mika-arch-groom-milestone/system_prompt.md`.
- **Approach** — Déclarer les trois champs dans chaque `[output]`. Noter que `mika-arch-groom-milestone` n'a aujourd'hui **aucun** `required_finding_list_prefixes` — il est plus exposé que ses frères, pas moins ; lui donner le contrat d'ancrage ne comble pas ce manque et ne prétend pas le combler. Dans `mika-arch-groom-ticket/system_prompt.md`, supprimer la ligne 124 (« On READY, the F-list is NOT required — the message may stay short since no iteration is needed ») et remplacer l'exemple READY de la ligne 142, qui est littéralement la forme du croupion de mika#2037 et enseigne donc le défaut. Le nouvel exemple montre trois ancrages citant le plan revu, suivis de la disposition. Ajouter une section « Contrat d'attestation d'ancrage » symétrique de la section « F-list Emission Contract » existante, décrivant les trois conditions et disant explicitement que l'ancrage n'est pas satisfait en recopiant trois fois la même phrase.
- **Test Scenarios**
  - Les trois `skill.toml` chargent sans erreur de validation (couvert par U2, exercé ici sur les manifestes réels).
  - Le system_prompt de groom-ticket ne contient plus la prescription de brièveté sur READY — vérifiable par recherche littérale.
  - L'exemple READY du prompt satisfait le matcher de U3 contre un brief d'exemple : le prompt n'enseigne pas une forme que la garde refuse.
- **Verification** — `cargo test -p mika-agent skills`, plus le chargement des skills bundled.

### U6. Fixtures de calibration pour les deux sens

- **Goal** — Le harnais de calibration mika-arch couvre la classe de défaut.
- **Requirements** — R1, R5.
- **Files** — `crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/review_anchor_stub_rejected.md`, `crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/review_anchor_real_review_passes.md`, `crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/manifest.yaml`, `crates/mika-agent/src/calibration/roles/mika_arch.rs`.
- **Approach** — Deux scénarios, déclarés dans `manifest.yaml` sur la forme des huit existants, et câblés dans `mika_arch.rs` comme `required_finding_list` l'est (id, dispatch, exécution). Le premier fixture est un plan propre accompagné d'un brief substantiel, où la réponse attendue est une revise READY ancrée — classe d'échec absente : `unanchored_ready`. Le second est le cas de non-régression du sens 1 : un plan sans objection réelle doit continuer à produire READY, pas une escalade défensive — classe d'échec absente : `false_escalation`. Réutiliser les classes d'échec existantes du manifeste quand elles décrivent déjà la défaillance ; n'en inventer une nouvelle que pour `unanchored_ready`.
- **Test Scenarios**
  - Le scénario de croupion échoue quand la garde est absente et passe quand elle est présente.
  - Le scénario de revue réelle passe dans les deux cas — c'est ce qui prouve que la garde ne bloque pas le grooming.
- **Verification** — Le harnais de calibration mika-arch, selon la commande que `mika_arch.rs` expose déjà pour les scénarios existants.

### U7. Matrice de mesure des deux sens

- **Goal** — Les seuils de KTD2 sont mesurés, pas raisonnés. La matrice reste au dépôt comme garde permanente.
- **Requirements** — R1, R2, R3 ; critères de succès.
- **Files** — un test table-driven à côté de U3 (`crates/mika-agent/src/agent_loop/`), plus les corpus nécessaires sous `crates/mika-agent/tests/`.
- **Approach** — Construire deux colonnes de cas. **Doit refuser** : le croupion littéral de mika#2037 avec son brief ; le croupion augmenté d'une seule citation longue (l'attaque évidente contre `min_count = 1`) ; trois ancrages citant tous la même région ; trois ancrages paraphrasant le brief sans le citer. **Doit accepter** : les revues READY réelles retrouvables — la revue substantielle mentionnée dans le corps de mika#2037 (relancée le matin même avec cadrage explicite, `Disposition: ITERATE`, ce qui en fait un cas de corps riche même si sa disposition est terminale), les fixtures de calibration existantes portant des dispositions non terminales, et l'exemple READY réécrit de U5. Faire varier `min_count` et `min_quote_chars` sur la table et enregistrer le résultat : si aucune paire de seuils ne sépare les deux colonnes, KTD1 est invalidé — s'arrêter et router vers l'opérateur au lieu de choisir un seuil qui n'existe pas. Si les seuils retenus diffèrent de ceux de KTD2, corriger KTD2 dans ce plan et dire dans la PR ce que la mesure a montré.
- **Execution note** — Cette unité est la preuve, pas une formalité. Une matrice qui ne contient que des cas construits pour passer ne mesure rien (`docs/solutions/best-practices/a-stub-built-from-the-doc-cannot-falsify-the-doc` — un stub écrit depuis la doc teste la prémisse, pas le monde). Les cas « doit accepter » viennent de revues réelles ; ceux qui n'en viennent pas sont marqués comme tels.
- **Test Scenarios**
  - Chaque cas « doit refuser » produit `Missing` aux seuils retenus.
  - Chaque cas « doit accepter » produit `Satisfied` aux seuils retenus.
  - L'inversion tient : aux seuils retenus, retirer la contrainte de non-chevauchement fait passer le cas « trois ancrages sur la même région » — ce qui prouve que R3 porte du poids et n'est pas décoratif.
- **Verification** — `cargo test -p mika-agent review_anchor_matrix`.

### U8. Verrou fail-closed au consommateur shell

- **Goal** — Aucun tier de `dispatch-lib` ne peut dériver une disposition d'une réponse marquée.
- **Requirements** — R7, R8.
- **Files** — `skills/bundled/_shared/dispatch-lib.sh`, `skills/bundled/_shared/tests/test_parse_disposition.sh`, `skills/bundled/_shared/test-dispatch-lib.sh`.
- **Approach** — Ajouter un tier 0 en tête de `_parse_disposition` et de `_parse_verdict` : si le texte contient le marqueur littéral, journaliser sur stderr et retourner sans rien émettre, avant toute autre correspondance. Le placement en tête est le point : après le tier 1a le marqueur serait encore respecté, mais après le tier 2 il ne le serait plus, et l'ordre est la seule chose qui l'empêche. Vérifier — sans le modifier — que `_iterate_groom_loop` traite l'absence d'émission comme `UNPARSED` et que le chemin `UNPARSED` retourne 1 après son retry borné (mika#1823, ~3363-3430) ; ajouter un test qui fige ce comportement, puisque R8 en dépend et qu'aucun test ne le garde aujourd'hui.
- **Test Scenarios**
  - Texte marqué contenant `Disposition: READY` → aucune émission (le tier 1a ne l'emporte pas).
  - Texte marqué contenant `Verdict: GROOMED` → aucune émission (le tier 1b de report ne l'emporte pas).
  - Texte marqué contenant « proceed » et « good to go » dans le corps → aucune émission (le tier 2 flou ne l'emporte pas) — c'est le cas qui justifie l'unité.
  - Texte non marqué → les trois tiers se comportent exactement comme avant (non-régression sur la table de cas existante de `test_parse_disposition.sh`).
  - `_parse_verdict` marqué → aucune émission, symétrique.
  - Une disposition absente ne devient jamais `READY` en aval : test sur le chemin `UNPARSED`.
- **Verification** — `bash skills/bundled/_shared/tests/test_parse_disposition.sh` et `bash skills/bundled/_shared/test-dispatch-lib.sh`.

---

## Verification Contract

- `cargo build --workspace` puis `cargo clippy --workspace --all-targets -- -D warnings` — le dépôt traite les avertissements clippy comme bloquants.
- `cargo test -p mika-agent` — couvre U1, U2, U3, U4, U7.
- `cargo fmt --all --check`.
- `bash skills/bundled/_shared/tests/test_parse_disposition.sh` et `bash skills/bundled/_shared/test-dispatch-lib.sh` — couvre U8, y compris la non-régression des trois tiers existants.
- Évaluation comportementale du skill : les scénarios de calibration mika-arch de U6 tournent et leurs classes d'échec déclarées sont absentes.
- Sonde fonctionnelle avant de déclarer le correctif déployé : le binaire reconstruit refuse effectivement un READY non ancré sur un brief réel. `strings` sur le binaire ne certifie rien (`feedback_strings_is_not_a_deployment_probe`) ; la sonde est un appel qui traverse la garde.
- La matrice de U7 est exécutable et verte ; ses seuils retenus correspondent à ce que déclarent les trois `skill.toml`.

---

## Definition of Done

**Global**

- Les huit unités sont implémentées, avec leurs tests.
- La matrice de U7 est verte et ses seuils sont ceux déclarés dans les manifestes ; si la mesure a corrigé KTD2, la correction est écrite dans ce plan et expliquée dans la PR.
- Le corps de la PR argumente le choix de l'artefact d'attestation (KTD1) et dit pourquoi les deux autres candidats ont été écartés — exigence explicite de l'opérateur.
- Aucun identifiant de modèle, de fournisseur ou de siège n'apparaît dans le code de la garde, sa configuration ou ses tests.
- Le code des approches abandonnées est retiré du diff.
- La PR est ouverte avec `mika-platform-qa` en relecteur.

**Par unité**

- U1 — Les champs parsent ; l'absence de déclaration ne change rien pour les manifestes existants.
- U2 — Les cinq cas de validation se comportent comme spécifié.
- U3 — Les huit scénarios du matcher passent, bornes incluses.
- U4 — La garde fire sur la moitié non terminale, ne fire pas sur la moitié terminale, et son refus retire effectivement la disposition tout en conservant le corps.
- U5 — La prescription de brièveté sur READY a disparu du prompt ; l'exemple READY réécrit satisfait le matcher.
- U6 — Les deux fixtures sont déclarées, câblées, et le scénario de revue réelle passe avec et sans la garde.
- U7 — Les deux colonnes de la matrice séparent aux seuils retenus, et l'inversion prouve que la contrainte de non-chevauchement porte du poids.
- U8 — Les trois tiers sont court-circuités par le marqueur, et la non-régression de la table existante tient.

---

## Acceptance criteria

- [ ] Une réponse d'architecte portant `Disposition: READY` sans attestation d'ancrage ne produit aucune disposition en sortie de chaîne — ni au moteur, ni chez le consommateur `dispatch-lib`, ni par appariement flou.
- [ ] Le croupion littéral de mika#2037, rejoué avec son brief d'origine, est refusé par le matcher et par la chaîne complète.
- [ ] Une revue réelle produisant `READY` avec des citations du plan est acceptée sans intervention — mesuré sur des revues réelles, pas sur des exemples construits pour passer.
- [ ] Le refus est fail-closed : après le re-prompt correctif, un doute n'est jamais arrondi en accord ; l'absence d'attestation devient une absence de verdict.
- [ ] `Verdict: GROOMED` (second-review) est couvert au même titre que `Disposition: READY`.
- [ ] Les trois skills producteurs de verdict déclarent le contrat, et le system_prompt de `mika-arch-groom-ticket` n'affirme plus qu'une réponse READY peut rester brève et sans preuve.
- [ ] Le correctif ne cible aucun modèle : la garde et ses tests sont indifférents au fournisseur assis sur mika-arch.
- [ ] Les seuils sont mesurés sur une matrice qui reste au dépôt comme test, et non fixés par raisonnement en prose.
