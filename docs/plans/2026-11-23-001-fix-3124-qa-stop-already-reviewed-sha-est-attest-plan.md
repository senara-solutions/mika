# mika#3124 — `stop_already_reviewed_sha` s'atteste par le résultat de la lecture de tête, pas par son appel

> Branche : `fix/3124/qa-stop-already-reviewed-sha-est-attest`
> Sources : commentaire 43 du parking #2983, mika#3070, mika#2934, mika#2939.

## Le défaut, tel qu'il a été mesuré

Le 2026-11-10, sur `senara-solutions/mika#3070` :

| instant | fait |
|---|---|
| 16:49:49Z | verdict `block[pipeline]` posté sur la tête `d72530065` ; le registre reçoit `verdict_posted` sur cette tête. |
| ~17:01Z | nouvelle tête `24e1f25b2` (commit vide, trailer `Pipeline-Exempt`). |
| 17:01:35Z | tour webhook mika-qa (session 21949859) sur la nouvelle tête. |
| 17:01:47Z | `qa_head_reviews` appelé sur `24e1f25b2` — il répond **zéro revue sur cette tête**. |
| **17:02:06Z** | le registre reçoit **`stop_already_reviewed_sha`** sur `24e1f25b2`. Aucune revue ne portait sur ce commit. |
| 17:02:33Z | le même tour l'écrit en clair dans son propre texte, puis finit sur `Webhook-Outcome: stop_out_of_scope`. |
| 17:16:51Z | `webhook_prefilter_skipped`, classe `green_check_suite_head_already_reviewed`. Aucun tour LLM. |
| 18:00:26Z | la revue est obtenue **à la main** (`mika ask --agent mika-qa`). |

Le mécanisme : `decide_green_pr_branch` (`crates/mika-agent/src/server/webhook_prefilter.rs:1366`)
écarte une check_suite verte dès que le registre `qa_reviewed_head` porte, sur la tête, l'une
des issues de `REVIEWED_HEAD_OUTCOMES`. Une écriture fausse écarte donc la tête pour
`REVIEWED_HEAD_WINDOW_DAYS = 30` jours, sans recours et sans tour.

La garde de preuve du tour existe déjà — `preuve_du_tour_manquante`
(`crates/mika-agent/src/qa_reviewed_head.rs:286`, mika#2934) — et elle n'a pas tiré. Son
site fautif est `lecture_de_tete_dans_le_tour`, ligne 317 :

```rust
deja_execute.iter().any(|resume| {
    resume.name == QA_HEAD_REVIEWS_TOOL
        && resume.success
        && resume.input_summary.to_ascii_lowercase().contains(&tete)
})
```

Elle n'interroge que l'**entrée** de l'appel. Un `qa_head_reviews` réussi qui **nomme** la
tête satisfait la preuve même quand sa **sortie** n'atteste aucune revue. La garde certifie
que le modèle a posé la question ; elle ne certifie pas que la réponse disait oui. Exactement
l'écart entre « j'ai consulté le registre » et « le registre m'a répondu oui » — mika#2939,
mais un étage plus haut.

Le fail-safe directionnel du module reste ce qu'il est : **une ligne manquante coûte un tour
payé ; une ligne fausse coûte une revue.** Le défaut est du côté cher.

## La décision de conception

Piste 1 du commentaire 43 : **le refus vit dans `record_reviewed_head`, pas dans le
préfiltre.** Le préfiltre est le lecteur ; corriger un lecteur d'une ligne fausse est un
rattrapage, pas une garde. Le site de décision reste `tool_execution/dispatch.rs`.

Quatre contraintes bornent la forme de la garde.

**(1) Le prédicat doit être positif.** `ToolCallSummary::output_summary` est tronqué à
`OUTPUT_SUMMARY_MAX = 300` (`crates/mika-agent/src/planning/policy.rs:140`). Un prédicat
négatif — « je ne vois pas de revue, donc je refuse » — serait satisfait par une troncature,
donc anti-fail-safe au mauvais moment : il passerait précisément quand la sortie est longue,
c'est-à-dire quand il y a des revues. La garde doit **voir** la preuve, jamais constater une
vacuité.

**(2) Parser le JSON de la sortie est exclu.** La sortie actuelle du handler est
pretty-printée (pas de `jq -c`) : ~272 octets dès une seule revue, donc tronquée au-delà. Un
parseur rendrait l'objet illisible sur la population **légitime** et refuserait chaque
check_suite verte d'une PR déjà revue — une régression d'un tour payé par événement.

**(3) Trois populations, une seule clé (mika#2205).** « Une revue a jugé cette tête », « zéro
revue a jugé cette tête » et « je ne sais pas lire la réponse » ne sont pas la même chose, et
les confondre est la classe d'erreur que ce ticket corrige. La clé doit les distinguer.

**(4) Le fait n'a qu'un site de vérité (mika#2158).** Le handler le dérive déjà ; le Rust ne
doit pas le re-dériver à partir de la même donnée tronquée.

D'où la forme retenue : **le handler émet une clé à vocabulaire fermé, en première position de
son objet, et le moteur cherche le littéral de la valeur attendue.**

```
head_evidence ∈ { "head_was_ruled_on", "head_never_ruled_on" }
```

- Dérivée du **même** `$kept` que `reviews` — une seule expression source, donc la clé ne peut
  pas dériver de la liste qu'elle résume.
- Les deux valeurs sont **lexicalement disjointes** : aucune n'est sous-chaîne de l'autre, donc
  la recherche littérale ne produit pas de faux positif.
- En **première** position de l'objet jq (jq préserve l'ordre d'écriture du constructeur),
  donc dans les ~45 premiers octets de la sortie : jamais tronquée, quel que soit le nombre de
  revues.
- Lue par recherche du littéral **entre guillemets**, donc indépendante de l'espacement JSON
  et de toute évolution du reste de l'objet.

Lecture côté moteur, par résumé et non par champ : la preuve est un **seul** `ToolCallSummary`
qui à la fois nomme la tête en entrée **et** porte `"head_was_ruled_on"` en sortie. Deux
résumés — l'un nommant la tête, l'autre portant l'attestation — ne se composent pas : ce serait
réintroduire le défaut sur une autre tête.

Classification résultante (`AttestationDeTete`, quatre issues) :

| ce que le tour montre | issue | refus ? |
|---|---|---|
| appel réussi sur la tête, sortie `head_was_ruled_on` | `Jugee` | non — preuve vue |
| appel réussi sur la tête, sortie `head_never_ruled_on` | `JamaisJugee` | oui — la faute de l'incident |
| appel réussi sur la tête, aucun des deux littéraux | `Illisible` | oui — handler antérieur ou sortie tronquée hors contrat |
| aucun appel réussi nommant la tête | `Absente` | oui — le refus actuel de mika#2934 |

`Jugee` l'emporte sur les trois autres (prédicat positif) ; entre les trois refus l'ordre ne
change que le message, et il va du plus précisément observé au plus vague.

Pourquoi quatre variantes de `PreuveManquante` et pas deux : le re-prompt actuel du bras
`StopSansLectureDeTete` dit « No successful `qa_head_reviews` call for `{head_sha}` appears in
this turn's tool history. Call it first (step 3), then conclude. » Dans le cas de l'incident,
cette phrase est **fausse** : le modèle l'avait appelé, à 17:01:47Z. Un re-prompt faux fait
rejouer un appel déjà fait et consomme le tour qu'il était censé sauver.

Le bras large de `preuve_du_tour_manquante` sur les deux `stop_*` est conservé tel quel :
`stop_already_reviewed_sha` et `stop_verdict_recorded` affirment tous deux qu'une revue existe
sur la tête, donc tous deux doivent la montrer.

## Hors périmètre, nommé

- **Le préfiltre ne change pas.** `decide_green_pr_branch` reste le lecteur du registre. Rien
  n'y est ajouté : une garde au point d'écriture rend la garde en lecture inutile, et la
  dédoubler créerait deux juges d'une même question.
- **Pas de compteur `verdict_lines_on_head`.** Conséquence assumée : `stop_verdict_recorded`
  reste acceptable sur une tête portant une revue **sans** ligne `VERDICT:`. Le champ
  `carries_verdict_line` existe déjà dans `reviews[]` mais vit après la troncature.
- **Pas de filtre sur l'auteur.** Conséquence assumée : `stop_already_reviewed_sha` reste
  acceptable sur une tête portant la revue d'un **autre** auteur. Filtrer sur
  `mika-platform-qa` figerait l'identité du compte QA dans le moteur et ferait du moteur un
  second juge d'une question dont le skill est déjà juge.
- **Le second fait du commentaire 43** — mika-qa classant une PR du siège MPC comme
  human-authored — n'est pas ce ticket et n'est pas touché ici.

Les deux premiers résidus sont les deux faces du même arbitrage : la garde refuse ce qu'elle
peut **voir** dans 300 octets, et rien de plus. Les élargir demande de déplacer le fait vers le
début de la sortie, c'est-à-dire un second champ à vocabulaire fermé — un ticket de suivi, pas
ce vol.

## Ordre de commit

L'ordre est choisi pour qu'aucun état intermédiaire ne refuse plus que l'état final.

**Commit 1 — le fait, et sa garde hors-ligne.**
`skills/bundled/qa-review-webhook-success/handlers/qa_head_reviews.sh` : `head_evidence` en
première clé du constructeur jq final, dérivée de `$kept`. En-tête du handler complété : la
clé est un **contrat lu par le moteur**, pas un ornement, et son vocabulaire est fermé.
`tools.json` : le champ décrit dans la description de l'outil, avec son vocabulaire et le fait
qu'une conclusion `stop_*` sans `head_was_ruled_on` est refusée par le moteur.
`system_prompt.md` : étape 4 — la décision se lit sur `head_evidence`, et
`head_never_ruled_on` interdit `stop_already_reviewed_sha`.
`scripts/test-qa-head-reviews-handler.sh` : les cas et la garde (détail plus bas).
Ce commit seul est inoffensif : la clé est émise, personne ne la lit encore.

**Commit 2 — le vocabulaire et le prédicat.**
`crates/mika-agent/src/qa_reviewed_head.rs` : constantes
`HEAD_EVIDENCE_RULED_ON = "head_was_ruled_on"` et
`HEAD_EVIDENCE_NEVER_RULED_ON = "head_never_ruled_on"`, documentées « source de vérité du nom :
le handler » — l'idiome déjà posé par `QA_HEAD_REVIEWS_TOOL` (« Source de vérité du nom :
`tools.json` »). `lecture_de_tete_dans_le_tour` devient `attestation_de_tete_dans_le_tour`
rendant `AttestationDeTete` ; `PreuveManquante` gagne `StopSurTeteJamaisJugee` et
`StopSansAttestationLisible` ; `preuve_du_tour_manquante` route les quatre issues. Tests
inline, dont la mise à jour des fixtures de mika#2934 (détail plus bas).

**Commit 3 — le message et la sonde.**
`crates/mika-agent/src/tool_execution/dispatch.rs` : le `match` sur la variante passe de deux
à quatre bras, chacun avec son `variante_label` et son `ce_qui_manque`. Le préfixe
`[mika-engine] Tool call suppressed: record_reviewed_head needs this turn's own evidence
(mika#2934).` est conservé — c'est la même garde, élargie, pas une nouvelle. La sonde WARN
`reviewed_head_write_without_evidence` est réutilisée avec les deux nouvelles valeurs de
`variante`. Mise à jour du helper de test `lecture_de_tete_reussie`.

Après le commit 3 et avant `make deploy`, le couple « binaire neuf + handler ancien » existe
transitoirement : la sortie ne porte aucun littéral, donc tous les `stop_*` tombent en
`Illisible` et sont refusés. Coût : un tour payé par événement, jamais une revue perdue — le
bon côté du fail-safe. L'inverse (handler neuf, binaire ancien) est inoffensif. Rappel
mika#2340 : un changement sous `skills/bundled/` n'atteint un agent que par la chaîne
**rebuild → seed → read**, donc par `make deploy`.

## Ce que la suite hors-ligne doit tenir

`scripts/test-qa-head-reviews-handler.sh` (456 lignes, faux `gh`, câblée au job
`offline-script-suites` de `.github/workflows/ci.yml:677`) reçoit :

1. **Deux cas de valeur.** Une tête avec revue conforme ⇒ `head_evidence` vaut exactement
   `head_was_ruled_on`. Une tête sans aucune revue, et une tête dont les seules revues sont
   écartées par `dropped_predating_head`, ⇒ `head_never_ruled_on`. Le second cas est celui de
   l'incident et le plus important des deux : il prouve que la clé suit `$kept` et non
   `$on_head`.
2. **La garde de concordance inter-langage (mika#2158).** La suite grep les deux constantes de
   `crates/mika-agent/src/qa_reviewed_head.rs` et les compare aux littéraux que le handler
   vient d'émettre. Un littéral recopié d'un côté sans l'autre fait rougir ici, pas en
   production. (Le chemin exact est lu depuis le dépôt à l'exécution ; la suite échoue si le
   grep ne trouve pas sa cible, jamais si elle trouve zéro ligne.)
3. **L'assertion de borne.** La valeur de `head_evidence` tient dans les `OUTPUT_SUMMARY_MAX`
   premiers octets de la sortie, la borne étant **lue** depuis
   `crates/mika-agent/src/planning/policy.rs` et non recopiée. C'est la garde que la doc de
   `preuve_du_tour_manquante` appelle déjà de ses vœux pour `INPUT_SUMMARY_MAX` : « si une
   évolution de cette borne le mettait en danger, c'est un test qui doit rougir, pas une
   troncature silencieuse. » Le test est construit avec un nombre de revues suffisant pour que
   la sortie **dépasse** la borne, sinon il passerait par vacuité.
4. **Le contrôle anti-vacuité** existant (désarmement du filtre par `sed`, puis vérification
   que le désarmement a bien trouvé sa cible) est étendu aux nouvelles assertions, selon la
   forme déjà en place en fin de fichier.

## Les fixtures de mika#2934 se complètent, elles ne s'exemptent pas

Les helpers actuels ne portent aucune attestation de sortie :

- `qa_reviewed_head.rs` — `fn resume(...)` pose `output_summary: String::new()`.
- `dispatch.rs:968` — `fn lecture_de_tete_reussie(...)` pose `output_summary: "[]".to_string()`.

Avec le nouveau prédicat, les contrôles **positifs** de mika#2934 rougissent : leurs résumés
tombent en `Illisible`. C'est le « fire » du détecteur, et il dit vrai — `"[]"` est
littéralement la sortie du tour fautif. Les fixtures reçoivent donc une sortie conforme au
contrat qu'elles prétendent représenter, pas une exemption :

- `mika2934_ac2_le_force_push_ne_satisfait_pas_la_preuve` (contrôle positif, ~l. 1693-1701) :
  sortie portant `head_was_ruled_on`.
- `mika2934_ac2_un_stop_sans_lecture_de_tete_est_refuse` (contrôle négatif, ~l. 1660-1666) :
  inchangé dans son intention (aucun appel nommant la tête ⇒ `Absente`), vérifié comme tel.
- `mika2934_ac3_un_tour_qui_attend_un_callback_de_build_necrit_pas_verdict_posted` : relit sa
  variante attendue.
- `dispatch.rs::lecture_de_tete_reussie` : `output_summary` porte l'attestation, et un second
  helper `lecture_de_tete_sans_revue` porte `head_never_ruled_on` pour le nouveau cas.

Les trois nouveaux tests inline couvrent les trois refus nouvellement distingués, et un
quatrième vérifie qu'un `head_was_ruled_on` provenant d'un appel sur une **autre** tête ne
satisfait pas la preuve (le test qui empêche la recomposition par champ).

## Definition of Done

- `head_evidence` est émis en première clé par le handler, dérivé du même `$kept` que `reviews`.
- `tools.json` et `system_prompt.md` décrivent la clé, son vocabulaire fermé et la conséquence
  machine d'une conclusion `stop_*` sans `head_was_ruled_on`.
- `preuve_du_tour_manquante` refuse les trois cas non attestés et n'accepte que `Jugee`.
- Chacun des quatre refus porte un message distinct et exact quant à ce que le tour a fait.
- La sonde `reviewed_head_write_without_evidence` porte la variante dans son champ `variante`.
- La suite hors-ligne tient les deux valeurs, la concordance inter-langage et la borne de 300
  octets, et son contrôle anti-vacuité couvre les nouvelles assertions.
- Les fixtures de mika#2934 sont complétées ; aucune n'est exemptée, ignorée ou supprimée.
- `cargo test`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`,
  `bash scripts/verify-pipeline.sh origin/main` passent.

## Acceptance criteria

Le corps du ticket mika#3124 ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de ses deux faits mesurés et du contrat ci-dessus.

1. **AC1 — Le tour de l'incident est refusé.** Un tour qui appelle `qa_head_reviews` avec
   succès sur la tête, dont la sortie porte `head_never_ruled_on`, et qui tente
   `record_reviewed_head` avec `outcome = stop_already_reviewed_sha` sur cette tête, voit
   l'appel supprimé. Le registre ne reçoit rien. Test inline sur
   `preuve_du_tour_manquante`, et test de suppression sur `dispatch.rs` avec la fixture de
   `24e1f25b2`.
2. **AC2 — Le cas légitime passe encore.** Un tour dont la sortie porte `head_was_ruled_on`
   écrit `stop_already_reviewed_sha` sans suppression, y compris quand la sortie est plus
   longue que `OUTPUT_SUMMARY_MAX` et donc tronquée après la clé.
3. **AC3 — Le re-prompt dit vrai.** Le message de suppression de `StopSurTeteJamaisJugee`
   n'enjoint pas d'appeler `qa_head_reviews` : il nomme le fait que l'outil a répondu zéro
   revue sur cette tête, et que la conclusion correcte est donc de réviser. Assertion sur le
   texte du message, pas seulement sur la variante.
4. **AC4 — Une sortie illisible refuse.** Un résumé `qa_head_reviews` réussi nommant la tête
   mais ne portant aucun des deux littéraux (handler antérieur, sortie vide, `"[]"`) refuse
   l'écriture `stop_*`.
5. **AC5 — La preuve ne se recompose pas.** Un `head_was_ruled_on` issu d'un appel nommant une
   **autre** tête ne satisfait pas la preuve pour la tête courante.
6. **AC6 — Les deux littéraux ne divergent pas.** La suite hors-ligne rougit si la constante
   Rust et le littéral du handler cessent de coïncider, et rougit si le grep ne trouve pas sa
   cible.
7. **AC7 — La borne est surveillée.** La suite rougit si `head_evidence` sort des
   `OUTPUT_SUMMARY_MAX` premiers octets, la borne étant lue depuis `planning/policy.rs`.
8. **AC8 — `stop_verdict_recorded` est traité comme `stop_already_reviewed_sha`.** Le bras
   large est conservé : les deux issues exigent l'attestation.
9. **AC9 — `verdict_posted` est inchangé.** Son bras continue de ne dépendre que de
   `pr_review_posted_in_turn` ; aucun test existant sur ce chemin ne change de verdict.

## Fire-Disposition

Le vol livre un détecteur : la garde de suppression dans `dispatch.rs`, dont le chemin de
succès est « aucune écriture non attestée ». Option **(a) — exception nommée en allowlist**,
avec la particularité qui compte ici :

**L'allowlist est vide par construction, et ne peut pas ne pas l'être.** La garde s'évalue sur
`deja_execute`, l'historique d'outils d'un tour **en cours**. Il n'existe aucune population
statique de violations à recenser : pas de fichier à lister, pas de ligne de code à exempter,
pas de site existant à migrer. Le détecteur ne peut tirer que sur un tour futur, et un tour
futur n'est pas une donnée qu'une allowlist pourrait nommer. Il n'y a donc **aucune entrée à
créer**, et l'assertion auto-nettoyante de la forme (a) serait une assertion sur l'ensemble
vide.

Deux conséquences existantes sont néanmoins nommées, parce que ce sont elles qui « tirent » au
sens du ticket :

1. **Les fixtures de mika#2934 rougissent.** Ce sont les seules violations réelles que le
   détecteur produit, et elles sont **corrigées dans le même vol** (section *Les fixtures de
   mika#2934 se complètent*), jamais exemptées. Aucun `#[ignore]`, aucun `#[cfg(skip)]`,
   aucune entrée d'allowlist.
2. **La ligne fausse déjà en base** —
   `pr:senara-solutions/mika#3070@24e1f25b2…`, portée par `audit_events` avec
   `tool_name = "qa_reviewed_head"` — n'est pas touchée. Aucun SQL n'est écrit par ce vol.
   Elle expire d'elle-même vers le **2026-12-10** par `REVIEWED_HEAD_WINDOW_DAYS = 30` (bornée
   au-dessus par `AUDIT_RETENTION_DAYS = 90`), et la PR #3070 a déjà reçu sa revue à la main à
   18:00:26Z le 2026-11-10 : la ligne n'écarte plus rien d'utile. Précédent mika#2927 — les
   lignes hors contrat sont relâchées par le temps, pas par une requête.

Sonde : `reviewed_head_write_without_evidence` (WARN, champs `trace_id`, `step`, `outcome`,
`variante`). Rappel mika#2340 : **l'absence de cette ligne signifie un binaire antérieur au
vol, jamais une population vide.** La première occurrence d'une variante
`stop_sur_tete_jamais_jugee` est la mesure que la garde mord ; son absence après `make deploy`
ne se lit comme « plus de défaut » qu'une fois le sha du déploiement attesté par
`~/.mika/skills/.manifest-writer`.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/qa-review-webhook-success/handlers/qa_head_reviews.sh` | 14 |
| `skills/bundled/qa-review-webhook-success/tools.json` | 2 |
| `skills/bundled/qa-review-webhook-success/system_prompt.md` | 8 |
| `scripts/test-qa-head-reviews-handler.sh` | 42 |
| `crates/mika-agent/src/qa_reviewed_head.rs` (vocabulaire, `AttestationDeTete`, prédicat, variantes) | 80 |
| tests inline `qa_reviewed_head.rs` (4 nouveaux + fixtures complétées) | 55 |
| `crates/mika-agent/src/tool_execution/dispatch.rs` (4 bras, messages, sonde) | 28 |
| tests inline `dispatch.rs` (2 nouveaux + helpers) | 22 |

Total estimé : 251 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 251 est sous la borne de 279 et très en deçà du
seuil par PR de 1000. Trois propriétés du périmètre le maintiennent là :

- **Aucun fichier nouveau.** Les six fichiers touchés existent tous, et les quatre plus gros
  changements sont des modifications locales d'une fonction ou d'un `match` déjà écrits. Pas
  de module, pas de table, pas de migration de schéma, pas de nouvel outil.
- **Le périmètre a été coupé avant l'estimation, pas après.** Les trois extensions naturelles
  — compteur de lignes `VERDICT:`, variante propre à `stop_verdict_recorded`, filtre sur
  l'auteur — sont déclinées en *Hors périmètre, nommé*, chacune avec sa raison de fond et non
  avec un argument de volume.
- **Le coût de test est connu, pas estimé à l'aveugle.** Les fixtures à compléter sont
  identifiées nominativement avec leurs lignes, et le harnais hors-ligne existe déjà avec sa
  forme de contrôle anti-vacuité à imiter.

La fin de pipeline de ce vol est ordinaire : une suite de tests Rust, une suite bash hors
ligne, `make verify-bundled-skills` et `scripts/verify-pipeline.sh`. Aucun `make deploy` n'est
dans le périmètre du vol — le déploiement est l'étape d'après, et sa fenêtre transitoire est
documentée ci-dessus plutôt que gérée en code.
