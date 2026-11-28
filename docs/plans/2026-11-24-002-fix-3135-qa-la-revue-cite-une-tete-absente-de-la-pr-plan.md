# fix(3135) — un corps de revue ne peut plus nommer une tête que le `qa_pr_view` de la session n'a pas rendue

**Ticket :** senara-solutions/mika#3135
**Vol :** 1 sur 2 — la tête citée. Le rejeu intégral des étapes du job rouge est le vol 2.
**Tracked in:** senara-solutions/mika#3135 (vol 2 — rejeu intégral des étapes du job rouge ; ce vol ne clôt pas le ticket)

**Révision du 2026-11-27.** Le plan de première passe a reçu un `ESCALATE` de l'architecte le
2026-11-24, sur le seul finding F1, et l'opérateur y a répondu dans le corps du ticket. Cette
révision porte exactement trois choses, et ne touche à rien d'autre :

1. **F1(a) est tranchée par l'opérateur** et le § *Périmètre* l'enregistre : le véhicule du vol 2
   est #3135 elle-même. La réserve qu'il portait là est levée — par décision, pas par amendement.
2. **F1(b) reste réservée, et l'opérateur l'a explicitement reportée** au départ du vol 2. Le
   § *Périmètre* la nomme comme telle. Le vol courant n'en dépend pas, et le dit.
3. **Les repères de code sont rafraîchis.** `guards.rs` a pris ≈ 720 lignes et
   `builtin_handlers.rs` ≈ 80 depuis le 2026-11-24 ; `qa-review/system_prompt.md` a été réécrit.
   Tous les numéros de ligne du plan ont été relus dans l'arbre, et deux citations fausses sont
   corrigées (§ *Repères relus*). Deux voisins nouveaux sont nommés — le registre
   `qa_reviewed_head` et le survey des jetons canoniques.

Aucune ligne du Design, des Requirements, du Verification Contract, de la Fire-Disposition ou de
l'Ordre de commit ne change de substance.

---

## Le défaut, tel qu'il se mesure

La revue mesurée est lisible en entier :
`gh api repos/senara-solutions/mika/pulls/3133/reviews --jq '.[0].body'`, soumise le
2026-11-22T15:37:04Z par `mika-platform-qa`, verdict `hold[review]`.

Son `FINDINGS` ouvre ainsi :

> CI check 'Egress Manifest Lint' is failing on PR head **b7b34f12**, but the lint passes
> locally on the same branch.

`b7b34f12` n'est aucun des sept commits de la PR #3133. La tête au moment de la revue était
`4cfd3e3b` (tête finale après les deux commits suivants : `33a4f0df`). Le corps ne porte aucune
autre mention de tête, donc le seul SHA que cette revue a publié est inventé.

### D'où vient la connaissance du job rouge, et pourquoi elle n'a pas suffi

La *Data Integrity Rules* de `skills/bundled/qa-review/system_prompt.md:37` retire au modèle la
capacité de lire la CI, et `skills/bundled/qa-review/handlers/qa_pr_view.sh:6-7` la retire **à la
source** : `SAFE_FIELDS` (`:34`) n'inclut ni `statusCheckRollup`, ni `commits`. Le modèle n'a donc
appris « *Egress Manifest Lint* est rouge » que par un seul canal : **le corps du refus de
mika#2455** (`validate_qa_ci_coherence`,
`crates/mika-agent/src/skills/builtin_handlers.rs:4676-4916`), qui nomme les checks rouges pour
que le refus soit auto-suffisant, et dont le remède nomme littéralement le mot repris par la
revue :

> use `block[ci]` if the failure looks repairable by a code fix …, or `hold[review]` if it does
> not (broken infrastructure, **a flake**, an unrelated failure)

Ce canal donne le **nom du job**. Il ne donne ni la tête, ni la liste des étapes du job. La revue
a comblé les deux trous par inférence : une tête fabriquée, et un rejeu d'une seule des deux
étapes `run:` que `.github/workflows/ci.yml:956-971` déclare pour ce job
(`verify-egress-manifest.sh --report` à `:969`, puis `test-verify-egress-manifest.sh` à `:971` —
c'est la seconde qui rougissait, assertion N15, corrigée depuis par mika#3134).

### Les deux trous sont deux défauts, et un seul tient dans un vol

| trou | ce qu'il produit | fermé par |
|---|---|---|
| la tête citée n'est ancrée sur rien | un corps de revue affirme avoir revu un code qu'il ne sait pas nommer | **ce vol** |
| le rejeu partiel passe pour un rejeu | « ça passe en local » conclut au flake sur un rouge reproductible | vol 2 |

Le premier est entièrement local au corps : la tête réelle est **déjà** dans la session, rendue
par `qa_pr_view` (`headRefOid` est dans `SAFE_FIELDS`) et déjà lue par le prompt à trois endroits
(`system_prompt.md:530`, `:534`, `:585`). Rien ne vérifie que le corps cite celle-là. C'est ce que
ce vol ferme.

### Pourquoi ce n'est pas une phrase de prompt

mika#2455 l'a écrit pour sa propre classe (point 4 de sa doctrine, `guards.rs:3476-3484`), et
mika#2961 l'a **mesuré** sur une exigence de prompt voisine : relevé du 2026-10-29, 60 dernières
PR par dépôt, **0 ligne `PIPELINE:` ancrée sur 261 verdicts `pass`** alors que Step 2E la déclare
obligatoire dans chaque verdict ; régime réel 426 sur 426. Une exigence que seul le prompt porte
n'est pas tenue dans ce dépôt (doctrine
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`). Le remède est donc un
garde moteur, et le vol ne touche **aucune** ligne de `system_prompt.md`.

---

## Requirements

- **R1** — Un appel `run_gh("pr review …")` dont le corps nomme une tête qui n'est pas un préfixe
  du `headRefOid` rendu par le `qa_pr_view` de cette session est **refusé avant le subprocess**,
  comme ses quatre voisins de la famille pre-subprocess.
- **R2** — Le corps du refus est auto-suffisant : il nomme le ou les SHA fautifs **et** la tête
  réelle, pour que la réécriture ne demande aucun appel supplémentaire.
- **R3** — Le prédicat est **ancré sur une position de tête**, jamais sur la seule présence d'une
  chaîne hexadécimale. Un SHA cité pour autre chose (un commit reverté, un commit d'une autre PR
  dans une comparaison d'équivalence) ne déclenche rien.
- **R4** — Zéro appel réseau. La tête réelle est lue dans le **ledger d'outils de la session**,
  jamais à la forge.
- **R5** — La population payante est bornée par un prédicat **gratuit** : la lecture en base n'est
  payée que par les corps qui citent effectivement une tête.
- **R6** — Fail-OPEN sur tout signal illisible, avec une cause d'abstention **nommée** par
  variable de fil, selon l'arithmétique de mika#2455 : un faux négatif laisse subsister le régime
  actuel ; un faux refus rend une revue légitime impostable et tue le tour sans revue.
- **R7** — Interrupteur d'opérateur `MIKA_CITED_HEAD_GATE`, polarité et vocabulaire **identiques**
  à ses trois voisins : armé par défaut, désarmé par le seul `0`/`false`/`off`/`no` explicite,
  toute valeur non reconnue laisse **armé** avec un WARN citant la valeur entre guillemets.
- **R8** — Chaque décision est écrite dans `audit_events` — le refus **et** la décision nominale.
  « Zéro refus » doit distinguer une flotte saine d'un garde inerte (classe mika#2205).
- **R9** — Le garde ne réimplémente ni l'extraction du corps (`extract_pr_review_body`,
  `builtin_handlers.rs:2153`), ni la cible (`pr_review_target`, `guards.rs:3391`), ni la fenêtre
  de lecture du ledger (`PIPELINE_GROUNDING_WINDOW_SECS`, `guards.rs:4908`) : il consomme les
  sites existants.

---

## Design

### Placement dans la chaîne

Le site est `handle_run_gh` dans `crates/mika-agent/src/skills/builtin_handlers.rs`, qui exécute
la chaîne pre-subprocess « du plus local au plus engageant » :

```
validate_tool_arg_suffixes            :5654  (argv, gratuit)
validate_review_depth_present         :5671  (corps, gratuit)
validate_pr_review_flag_coherence     :5683  (corps + flag, mika#2237)
validate_pipeline_block_grounding     :5704  (corps + ledger, mika#2687/#2961)
→ validate_cited_head_grounding       :5707  (corps + ledger, CE VOL)
validate_qa_ci_coherence              :5720  (réseau, mika#2455)
validate_dependabot_verdict_coherence :5733  (réseau, mika#2519)
```

L'insertion est **après** mika#2687/#2961 et **avant** mika#2455 : c'est le dernier maillon qui ne
touche pas le réseau, et le critère d'ordre du fichier est exactement celui-là — le commentaire de
mika#2455 à `builtin_handlers.rs:5708-5719` l'énonce sur place (« *it is the only link that makes
a network call, and the chain runs from the most local to the most committing* »). Comme ses
quatre voisins, il n'est **pas** conditionné à `required_tool_arg_suffixes` : son sujet est le
corps lui-même, et les quatre commentaires d'ordre le disent chacun en toutes lettres.

### Le prédicat, en deux temps

**Temps 1 — la position de tête (gratuit).** Une regex ancrée sur un token de tête, puis au plus
24 caractères de la même ligne, puis la chaîne hexadécimale :

```
(?i)\b(?:pr\s+head|head|head_sha|headrefoid)\b.{0,24}?\b([0-9a-f]{7,40})\b
```

Trois propriétés chacune payée par un test :

- **La fenêtre de 24 caractères est bornée et non-gourmande.** Sans elle, un `head` en début de
  paragraphe capturerait un SHA trois phrases plus loin, cité pour autre chose. Le cas mesuré
  (`on PR head b7b34f12,`) tient dans un seul espace.
- **La capture exige au moins un chiffre.** Un mot anglais de sept lettres toutes dans `a-f`
  (`effaced`, `defaced`) est une chaîne hexadécimale valide ; sans ce filtre, « *the head of the
  effaced branch* » serait un refus. Le filtre est un prédicat sur le groupe capturé, pas une
  complication de la regex.
- **Le prédicat est ancré, jamais un `contains`.** C'est la doctrine que mika#2961 a dû écrire
  après avoir compté la prose d'une revue comme sa propre preuve — elle vit à
  `guards.rs:4716-4726`, au-dessus de `line_prefix_present` (`:4744`) : « *un `contains`
  compterait la prose d'une revue comme sa preuve* ». Un `\b[0-9a-f]{7,40}\b` nu refuserait un
  corps qui cite légitimement un commit reverté.

Aucune citation en position de tête ⇒ `Allowed`, et **rien n'est lu en base** (R5).

**Temps 2 — la tête réelle (une lecture en base).** `find_recent_tool_invocation`
(`db.rs:2686`, `async_db.rs:3811`) est appelée avec `tool_name = "qa_pr_view"` et la fenêtre
existante, sur le patron exact de l'appel de mika#2687 à `builtin_handlers.rs:4599-4607` ; puis
`headRefOid` est extrait de la **sortie** de l'appel le plus récent — par regex sur
`"headRefOid":"<hex>"`, sans désérialiser l'enveloppe : la sortie de `gh pr view` porte `files`
avant `headRefOid` en ordre alphabétique de clés, donc une sortie tronquée est un cas réel, et
c'est une abstention, pas une erreur de parsing.

La comparaison est un préfixe, casse repliée : `head_oid.starts_with(cited)`. Elle accepte donc
les trois formes que les revues emploient réellement — 7, 8 ou 40 caractères — et refuse
`b7b34f12` contre `4cfd3e3b…`.

### Les trois abstentions, et pourquoi elles sont trois

Format de fil (doctrine mika#2131 : deux orthographes d'une même cause scindent une population
sans le dire). Elles partent dans `audit_events.after_value` et dans le champ `reason` d'un WARN
`cited_head_abstained`.

| cause | ce qu'elle dit |
|---|---|
| `no_pr_view_recorded` | aucune ligne `qa_pr_view` dans la session — ledger désarmé (`MIKA_STORE_TOOL_CALLS`), ou revue reprise sur un callback de build, qui tourne sous un `session_id` neuf et ne voit pas l'appel d'origine |
| `tool_ledger_unreadable` | la base n'a pas répondu — un échec de lecture n'est pas un `false` |
| `head_oid_absent` | `qa_pr_view` a répondu, mais sa sortie enregistrée ne porte pas `headRefOid` (troncature, ou handler en erreur) |

Les deux premières sont la frontière que `PipelineGroundingAbstention` trace déjà pour son propre
outil (`guards.rs:4931-4939`, et sa doc à `:4920-4930` : « *dans les deux cas le ledger est muet,
pas accusateur* ») ; la troisième lui est étrangère et ne s'y fusionne pas : « l'appel n'a pas eu
lieu » et « l'appel a eu lieu et sa sortie est muette » sont deux états de la flotte qu'un
`GROUP BY` doit pouvoir séparer.

### Le corps du refus

Même forme que mika#2455, avec sa contrainte : **aucune ligne du corps ne commence par
`VERDICT:`** — `VERDICT_RE` est ancrée sur ce préfixe de ligne et une ligne parasite serait lue
comme un verdict.

```json
{
  "error": "cited_head_not_the_reviewed_head",
  "doctrine": "mika#3135",
  "target": "pr_review:senara-solutions/mika#3133",
  "cited_unknown": ["b7b34f12"],
  "reviewed_head": "4cfd3e3b…",
  "remedy": "This body names 1 head SHA that is not a prefix of the head this session
             actually read: b7b34f12. The head rendered by `qa_pr_view` in this session is
             4cfd3e3b…. Re-emit this call with every head citation replaced by that SHA, or
             remove the citation. Do not invent a SHA."
}
```

Le remède nomme la tête réelle, donc la réécriture ne coûte aucun appel (R2).

### Ce que le garde ne fait pas

- **Il ne juge pas l'absence de citation.** Un corps qui ne nomme aucune tête est hors population.
  Le défaut mesuré est une tête *fausse*, pas une tête *absente* ; exiger la citation serait une
  seconde classe, sur une population de plusieurs centaines de verdicts par mois, et le terme
  n'est pas celui du ticket.
- **Il ne juge pas les SHA hors position de tête** (R3), donc il ne recoupe pas
  `guard.equivalence_claim` (mika#1645), qui porte sur l'artefact comparé et non sur la tête.
- **Il ne lit pas la liste des commits de la PR.** `qa_pr_view` retire délibérément `commits`
  (`handlers/qa_pr_view.sh:6-7`) ; la seule tête ancrable dans la session est `headRefOid`, et la
  ramener de la forge serait un appel réseau que R4 interdit.

### Les deux voisins qu'il ne réimplémente pas

Nommés ici parce que tous deux portent le mot « tête » et qu'un lecteur pourrait les prendre pour
le même mécanisme. Ils ne le sont pas, et ni l'un ni l'autre ne ferme la classe de #3135.

- **`crates/mika-agent/src/qa_reviewed_head.rs` + `src/tools/record_reviewed_head.rs`**
  (mika#2917, #2927, #2934, #3124) — le registre des têtes sur lesquelles mika-qa a **statué**.
  Trois raisons structurelles pour lesquelles ce vol ne s'y branche pas, et une pour laquelle il
  le renforce :
  1. **Il est postérieur au `pr review`.** L'écriture vit « *dans le tour qui a posté la revue sur
     la forge, après un `pr review` réussi* » (doc de module, point 1) — correction mesurée de
     mika#2927. Au moment où le garde de ce vol décide, la ligne du registre n'existe pas encore :
     elle ne peut pas être sa source.
  2. **Son `head_sha` est attesté par le modèle, et non vérifié** : `record_reviewed_head.rs:25`
     le déclare sur place — l'outil ne vérifie « *ni que la PR existe, ni que le `head_sha` est
     bien sa tête* ». Un garde qui s'ancrerait là s'ancrerait sur la même affirmation qu'il juge.
  3. **Il ne décide rien sur un corps de revue.** Son unique lecteur est
     `PrefilterState::reviewed_head` du pré-filtre webhook, borné à un par une garde de ce module.
  4. **Ce vol le renforce sans le toucher.** Le `head_sha` que le modèle passe au registre vient
     du même `headRefOid` du Step 1 (`system_prompt.md:585`) que celui que ce garde ancre. Une
     tête fabriquée refusée au `pr review` est une tête qui n'atteint jamais le registre, puisque
     l'appel du registre est conditionné au succès du `pr review`.
- **`scripts/canonical-tokens-survey.sh --check` + `canonical_tokens.rs`** (mika#2201) — le survey
  part des **formes de lecture** et reconnaît `Regex::new(` (`:323`) ; il faut donc savoir s'il
  exigerait une ligne de `scripts/canonical-tokens.tsv` pour la regex de ce vol. **Non**, et c'est
  structurel : des littéraux d'une ligne reconnue il n'émet que quatre formes de jeton
  (`:352-380`) — une clé de callout `> - **Nom:**`, un préfixe `[GitHub…]`/`[callback…]`, un
  marqueur kebab contenant `verified|pending|marker`, et un run ALL-CAPS de cinq caractères ou
  plus. La regex de ce vol est entièrement en minuscules et ne porte aucune des quatre. **Ce vol
  n'ajoute donc aucune ligne au TSV** — et s'il en ajoutait une, ce serait une ligne de
  déclaration, jamais une entrée d'exemption (« on déclare, on n'allowliste pas », doc de module
  de `canonical_tokens.rs`).

---

## Périmètre — ce vol et ce qu'il laisse

**Ce vol livre** le garde de la tête citée, armé, câblé, audité.

**Ce vol ne livre pas** le second terme du ticket — « rejouer toutes les étapes du job rouge ».

### La décision d'opérateur du 2026-11-27, et ce qu'elle ferme

L'architecte a rendu `ESCALATE` le 2026-11-24 sur F1, qui portait les deux réserves que la
première passe de ce plan laissait ouvertes. L'opérateur a répondu dans le corps du ticket, et sa
réponse les traite différemment. Le plan enregistre les deux, sans en pratiquer aucune (mika#3122).

**F1(a) — le véhicule du vol 2 : TRANCHÉE.** Le vol 2 est porté par **#3135 elle-même**. Aucun
ticket dédié n'est ouvert. Conséquences, toutes déjà dans ce plan et désormais sans réserve :

- le ticket reste **ouvert** après ce vol ;
- la PR de ce vol porte `Refs #3135`, **jamais** `Closes` ;
- la ligne `Tracked in: senara-solutions/mika#3135` de l'en-tête nomme le vol 2 sur ce même
  ticket.

**F1(b) — la voie du rejeu intégral (α, β ou γ) : REPORTÉE par l'opérateur**, à trancher avant le
départ du vol 2. Deux choses que cette réponse établit, et que l'architecte juge sur le vol
courant :

- **le vol 1 n'en dépend pas** — il ne lit ni la définition du job, ni ses étapes, ni le signal
  que les trois voies se disputent ; son unique entrée hors corps est le `headRefOid` déjà rendu
  dans la session. Le vol courant est donc jugeable seul, et c'est ce que l'opérateur demande ;
- **si la voie retenue est γ, la décision reste à l'opérateur** — γ relâche une règle d'intégrité
  des données de la QA, et ce n'est pas un arbitrage de grooming.

Les trois voies, chiffrées et **aucune pratiquée** :

- **(α) le moteur énumère les étapes dans le corps du refus de mika#2455.** Le gate lit déjà
  `gh pr checks` ; il faudrait y ajouter la résolution du workflow et de ses `run:`. Le modèle ne
  voit toujours pas la CI — il reçoit une liste nommée. Coût : ≈ 120 lignes dans
  `validate_qa_ci_coherence`, plus un second appel forge dans un gate qui en fait déjà un.
- **(β) le corps de la revue dénombre lui-même**, sous une forme ancrée
  (`CI-REPLAY: <job> — <k>/<n> steps`, une ligne par étape avec sa commande et son `exit`), et le
  garde refuse `k != n` et toute commande citée qu'aucun `run_shell` de la session n'a exécutée.
  Coût : ≈ 150 lignes, zéro appel réseau, mais `n` reste déclaré par le modèle.
- **(γ) relâcher les Data Integrity Rules** (`system_prompt.md:37`) pour autoriser la QA à lire le
  job rouge et sa définition. Coût en lignes : faible. Coût en doctrine : élevé — la règle est une
  capacité *retirée à la source* et pas seulement interdite, ce que `handlers/qa_pr_view.sh:6-12`
  énonce sur place (« *CI fields … are excluded so the reviewing LLM never sees CI data* » ; la
  référence `docs/solutions/prompt-engineering/2026-04-08-remove-capability-not-prohibit-it.md`
  qu'il porte en pied est **pendante** — aucun fichier à ce chemin, re-vérifié le 2026-11-27). Son
  relâchement rouvre la classe que mika#2423 a fermée. **C'est la voie dont l'opérateur se réserve
  la décision.**

**La seconde raison du report tient toujours, et elle est de volume.** Le terme 2 demande, au
minimum : un variant de refus sur un `hold[review]` qui attribue un check requis rouge à une cause
d'environnement, une section de corps dénombrant les étapes rejouées, et l'ancrage de chaque
commande citée sur un `run_shell` réellement enregistré dans la session (mécanisme de
`PipelineGroundingOutcome::RefusedNoGuardExecuted`, `guards.rs:4600`). Estimé ≈ 290 lignes à lui
seul, soit **plus que la borne par vol entière** (279). Les deux vols ne pouvaient donc pas tenir
dans une PR, indépendamment de F1(b).

**Hors périmètre, nommé :** aucune ligne de `skills/bundled/qa-review/system_prompt.md` n'est
touchée (§ *Pourquoi ce n'est pas une phrase de prompt*) ; aucune migration de schéma
(`audit_events.tool_name` est du TEXT libre, convention de `QA_CI_COHERENCE_AUDIT_TOOL`) ; aucune
ligne de `scripts/canonical-tokens.tsv` (§ *Les deux voisins qu'il ne réimplémente pas*) ; aucun
`log_cited_head_gate_state` au démarrage — le WARN sur valeur non reconnue et les deux lignes
d'audit suffisent à dire si le gate tourne, et un sixième log de démarrage est une ligne de plus
dans une fonction d'init que ce vol n'a pas besoin d'ouvrir.

---

## Repères relus

L'arbre a bougé entre la première passe (2026-11-24) et cette révision : `guards.rs` a pris
≈ 720 lignes, `builtin_handlers.rs` ≈ 80, et `qa-review/system_prompt.md` a été réécrit (mika#3160
et voisins). Tous les repères du plan ont été relus. Deux étaient **faux** et sont corrigés ; les
autres ont glissé.

| ce que la première passe citait | ce que l'arbre porte le 2026-11-27 |
|---|---|
| `system_prompt.md:46` (Data Integrity Rule) | `:37` — `### Data Integrity Rules` |
| `system_prompt.md:601`, `:656` (headRefOid) | `:530`, `:534`, `:585` — trois sites, pas deux |
| `guards.rs:3479-3484` (doctrine mika#2455, pt 4) | `:3476-3484` |
| **`guards.rs:4355-4365`** (donné pour la doctrine « ancré, jamais `contains` ») | **FAUX** — ces lignes sont le gate de mika#2519. La doctrine vit à `:4716-4726`, au-dessus de `line_prefix_present` (`:4744`) |
| `guards.rs:4685-4690` (`PipelineGroundingAbstention`) | `:4931-4939`, sa doc à `:4920-4930` |
| `guards.rs:3492-3620`, `:4325-4745` (blocs voisins transcrits) | bloc mika#2455 `:3434-3690` ; bloc mika#2687/#2961 `:4536-4950` et `:5019-5228` |
| `builtin_handlers.rs:4841-4916` (`validate_qa_ci_coherence`) | `:4676-4916` |
| `builtin_handlers.rs:5590-5675` (chaîne pre-subprocess) | `:5654-5740` |
| `builtin_handlers.rs:5625-5630` (critère d'ordre) | `:5708-5719` |
| `builtin_handlers.rs:5643` (point d'insertion) | **`:5707`**, entre la fin du `if let Err` de mika#2687 et le commentaire de mika#2455 |
| `builtin_handlers.rs:4245-4330` (patron de lecture du ledger) | `:4245-4273`, et l'appel lui-même `:4599-4607` |
| **`ci.yml:953-956`** (les deux étapes du job rouge) | **FAUX** — c'est le job `egress-no-log-lint`. `egress-manifest-lint` est à `:956-971`, ses deux `run:` à `:969` et `:971` |
| `handlers/qa_pr_view.sh:6-8` | `:6-7` (le commentaire) et `:34` (`SAFE_FIELDS`) |
| `db.rs:2686`, `async_db.rs:3811` | inchangés |

Deux repères nouveaux, qui n'existaient pas dans la première passe du plan :
`pr_review_target` → `guards.rs:3391` ; `extract_pr_review_body` →
`builtin_handlers.rs:2153` ; la fenêtre réutilisée → `PIPELINE_GROUNDING_WINDOW_SECS = 7200`
(`guards.rs:4908`) et son outil `PIPELINE_GROUNDING_SHELL_TOOL` (`:4899`).

Le point d'insertion dans `guards.rs` est **juste avant son module de tests**, qui commence à
`:5229` : le fichier de production s'arrête donc à `:5228`, à la suite du bloc mika#2961.

---

## Verification Contract

1. `cargo test -p mika-agent cited_head` — les huit tests du § Taille estimée, verts.
2. `cargo test -p mika-agent` — suite complète, aucune régression (les voisins de la chaîne
   pre-subprocess ont leurs propres tests sur `handle_run_gh`).
3. `cargo clippy --all-targets -- -D warnings` — propre.
4. `cargo fmt --check` — propre.
5. `bash scripts/verify-pipeline.sh origin/main` — exit 0.
6. **Le corps mesuré du 2026-11-22T15:37Z est la fixture rouge.** Le test
   `mika3135_le_corps_mesure_du_15h37_cite_une_tete_inconnue` porte l'extrait réel de
   `pulls/3133/reviews[0].body` et la tête réelle `4cfd3e3b`, et asserte
   `Refused { unknown: ["b7b34f12"], .. }`. Un plan qui n'épingle pas le corps qui l'a motivé
   n'a pas de contrôle positif.
7. **Le contrôle négatif est sur le même corps.** Le même extrait, avec `b7b34f12` remplacé par
   `4cfd3e3b`, rend `Allowed`. C'est l'assertion qui rougit le jour où le prédicat cesse de
   reconnaître une citation légitime.
8. **L'anti-vacuité du prédicat.** `mika3135_un_sha_hors_position_de_tete_est_ignore` et
   `mika3135_un_mot_sans_chiffre_nest_pas_un_sha` établissent que le garde n'est pas un
   « refuse tout » déguisé : deux corps qui portent une chaîne hexadécimale passent.
9. **Les trois causes d'abstention sont épinglées par valeur** dans
   `mika3135_les_causes_dabstention_sont_un_format_de_fil` — ce sont des chaînes de fil, pas des
   libellés.
10. **La polarité de l'interrupteur est épinglée** sur les quatre familles de valeurs, dont
    « valeur non reconnue laisse armé ».
11. **Aucune ligne de `scripts/canonical-tokens.tsv` n'est ajoutée**, et
    `cargo test -p mika-agent canonical_tokens` reste vert — la vérification que le survey n'exige
    rien de la nouvelle regex (§ *Les deux voisins*) est une assertion de l'arbre, pas un
    raisonnement.

---

## Definition of Done

- `classify_cited_head_citation` et sa population vivent dans
  `crates/mika-agent/src/evidence/guards.rs`, à la suite du bloc mika#2961 et avant le module de
  tests, avec la doctrine de leurs choix écrite sur place.
- `validate_cited_head_grounding` est câblée dans `handle_run_gh` à
  `builtin_handlers.rs:5707`, entre mika#2687/#2961 et mika#2455, avec le commentaire d'ordre qui
  dit pourquoi là — sur la forme des quatre commentaires voisins.
- Les huit tests sont dans le module de tests de `guards.rs`, aux côtés de ceux de mika#2455 et
  mika#2961.
- `MIKA_CITED_HEAD_GATE` est documenté dans `docs/operator/environment-variables.md` avec son
  défaut, son WARN et sa cause d'abstention, à côté de ses trois voisins (`:5028`, `:5223`,
  `:5602`).
- Les cinq points du Verification Contract 1-5 sont verts, chacun avec sa sortie citée dans le
  corps de la PR.
- La PR porte `Refs #3135` et la ligne `Tracked in:` du § Périmètre.

---

## Acceptance criteria

Le corps de senara-solutions/mika#3135 ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés des Requirements et du Verification Contract, et bornés au
**vol 1**.

- **AC1** — Un appel `run_gh pr review` dont le corps nomme, en position de tête, un SHA qui n'est
  pas un préfixe du `headRefOid` rendu par le `qa_pr_view` de la session est refusé avant le
  subprocess. Épinglé par le corps mesuré du 2026-11-22T15:37Z (VC6).
- **AC2** — Le corps du refus nomme le ou les SHA fautifs **et** la tête réelle. Assertion sur les
  deux champs `cited_unknown` et `reviewed_head` du JSON de refus.
- **AC3** — Un corps citant une tête légitime — préfixe de 7, de 8 ou les 40 caractères, dans
  n'importe quelle casse — n'est pas refusé (VC7).
- **AC4** — Un SHA cité **hors** position de tête ne déclenche rien, et un mot hexadécimal sans
  chiffre n'est pas lu comme un SHA (VC8).
- **AC5** — Le garde ne fait aucun appel réseau : la tête réelle vient de
  `find_recent_tool_invocation(.., "qa_pr_view", ..)`. Vérifiable par absence de tout appel forge
  dans la fonction, et par le fait que les huit tests tournent sans fixture réseau.
- **AC6** — Un corps qui ne cite aucune tête ne paie aucune lecture en base. La lecture est après
  le prédicat textuel dans l'ordre d'exécution de la fonction.
- **AC7** — Chacune des trois causes d'abstention rend `Ok(())` et écrit sa ligne d'audit sous sa
  chaîne de fil exacte (VC9).
- **AC8** — `MIKA_CITED_HEAD_GATE` désarme le garde sur `0`/`false`/`off`/`no` uniquement ; toute
  autre valeur laisse armé avec un WARN citant la valeur entre guillemets (VC10).
- **AC9** — La décision nominale est auditée autant que le refus : un `allowed` et un `refused`
  distincts sous `CITED_HEAD_AUDIT_TOOL` (R8, classe mika#2205).
- **AC10** — Aucune ligne de `skills/bundled/qa-review/system_prompt.md` n'est modifiée, aucune
  ligne de `scripts/canonical-tokens.tsv` n'est ajoutée (VC11), et le ticket #3135 n'est pas clos
  par ce vol.

---

## Fire-Disposition

Ce vol livre un détecteur : `validate_cited_head_grounding` est un garde pre-subprocess dont le
chemin de succès est « aucune tête fabriquée trouvée ». Les huit tests en sont les détecteurs de
second rang.

**Option retenue : (a) exception nommée en allowlist — avec une population d'exceptions VIDE, et
la raison pour laquelle elle est vide est structurelle, pas optimiste.**

Détail d'implémentation :

1. **La population jugée est un flux, pas un corpus.** Le garde juge des corps de revue produits
   *après* son déploiement ; il ne balaie aucun artefact du dépôt. Il n'existe donc aucune
   violation préexistante à excepter — mesure : `git ls-files` ne porte aucun corps de revue, et
   les seuls corps que le dépôt contiendra après ce vol sont les **fixtures de test**, dont l'une
   est délibérément rouge. Une entrée d'allowlist n'aurait littéralement aucune donnée à nommer.
2. **Le détecteur atterrit ARMÉ.** Aucun `#[ignore]`, aucun `#[cfg]`, aucun cas commenté, aucune
   sortie conditionnelle. Il mord dès le merge.
3. **L'anti-vacuité est portée par deux fixtures mesurées sur le même corps réel** : l'extrait du
   2026-11-22T15:37Z avec `b7b34f12` est rouge (VC6), le même extrait avec `4cfd3e3b` est vert
   (VC7). Un prédicat qui se mettrait à refuser tout, ou à ne plus rien refuser, rougit sur l'une
   des deux.
4. **L'assertion auto-nettoyante est le contrôle négatif (VC7), et c'est elle qui rougirait si
   l'exception devenait stale** : le jour où le prédicat cesse de reconnaître une citation
   légitime, le vert devient rouge et la régression est nommée avant d'atteindre la flotte.
5. **La soupape d'opérateur n'est pas une allowlist, et ce vol ne la confond pas avec une.** Les
   trois abstentions nommées et `MIKA_CITED_HEAD_GATE` sont le contrat de la famille
   pre-subprocess (mika#2455, mika#2687, mika#2519) : un STOP qui désarme, et un fail-open qui
   laisse passer sur signal illisible. Ils ne dispensent aucune donnée particulière — ils règlent
   le garde entier.
6. **Les deux lignes d'audit (AC9) sont ce qui empêche le garde d'être inerte sans qu'on le
   sache.** « Zéro refus » sans ligne nominale ne distingue pas une flotte saine d'un garde mort
   (classe mika#2205) ; ici `allowed` et `refused` sont deux populations qu'un `GROUP BY`
   sépare.
7. **Le registre de jetons canoniques n'est pas une allowlist non plus, et ce vol n'y touche
   pas.** Le § *Les deux voisins qu'il ne réimplémente pas* le mesure : la regex de ce vol
   n'émet aucun des quatre jetons du survey, donc aucune déclaration n'est requise — et VC11
   l'asserte dans l'arbre plutôt que de s'en tenir à ce raisonnement.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` — `CITED_HEAD_RE` + filtre « au moins un chiffre » | 16 |
| `crates/mika-agent/src/evidence/guards.rs` — `CitedHeadOutcome` (2 variants) | 16 |
| `crates/mika-agent/src/evidence/guards.rs` — `CitedHeadAbstention` (3 causes de fil) | 18 |
| `crates/mika-agent/src/evidence/guards.rs` — `head_oid_from_pr_view` | 14 |
| `crates/mika-agent/src/evidence/guards.rs` — `classify_cited_head_citation` | 22 |
| `crates/mika-agent/src/evidence/guards.rs` — interrupteur + WARN valeur non reconnue | 24 |
| `crates/mika-agent/src/evidence/guards.rs` — consts audit / outil / fenêtre | 8 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` — `validate_cited_head_grounding` | 47 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` — câblage dans `handle_run_gh` + commentaire d'ordre | 8 |
| tests (module de tests de `guards.rs`, 8 tests) | 90 |

Total estimé : 263 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Le re-mesurage du 2026-11-27 ne change pas le total**, et la raison est que la révision n'ajoute
aucun livrable : la décision d'opérateur ne touche que la prose du § *Périmètre*, et les repères
relus ne font que corriger des numéros de ligne. Le § *Les deux voisins* ajoute une assertion
(VC11) qui n'écrit aucun code — elle lance un test existant. Aucune ligne de
`scripts/canonical-tokens.tsv`, mesure à l'appui.

**Pourquoi un seul vol tient, et il tient de justesse** — 263 contre 279, soit 16 lignes de marge.
Trois propriétés la rendent crédible, et chacune a été vérifiée dans l'arbre le 2026-11-27, pas
seulement raisonnée :

- **Rien n'est inventé : tout est copié d'un voisin lu.** La forme de l'enum de sortie
  (`PipelineGroundingOutcome`, `guards.rs:4600`), la struct d'abstention à consts de fil
  (`:4931-4939`), la polarité de l'interrupteur (`:4980-5011`), le corps de refus en JSON avec sa
  contrainte `VERDICT:`, l'ordre d'insertion dans la chaîne (`builtin_handlers.rs:5708-5719`), la
  lecture du ledger (`:4599-4607`) — les six existent déjà et ce vol les transcrit sur un septième
  sujet. C'est pourquoi la part de prose de doctrine, lourde dans ce fichier, est ici de la
  transcription et non de la rédaction.
- **Deux fichiers, deux points d'insertion connus au numéro de ligne.** `guards.rs:5228` (juste
  avant son `mod tests` de `:5229`), et `builtin_handlers.rs:5707` entre deux `if let Err`
  existants. Aucun fichier nouveau, aucune migration, aucun module à déclarer.
- **La fin de pipeline est légère.** Pas de `make verify-bundled-skills` (aucun bundle touché),
  pas de `make deploy`, pas de `docs-sync` (la seule ligne de `docs/` est une entrée de
  `environment-variables.md`, hors compte). Quatre commandes : `cargo test`, `clippy`, `fmt`,
  `verify-pipeline.sh`.

Le second terme du ticket, estimé ≈ 290 lignes à lui seul — soit au-dessus de la borne par vol à
lui tout seul —, part au vol 2 sur **#3135 elle-même** (F1(a), décision d'opérateur du
2026-11-27), avec la ligne `Tracked in:` de l'en-tête et la voie α/β/γ que l'opérateur tranchera
avant son départ.

---

## Ordre de commit

1. `feat(3135): la classification d'une tête citée, et ses trois abstentions nommées`
   — `guards.rs` seul : regex, `CitedHeadOutcome`, `CitedHeadAbstention`,
   `head_oid_from_pr_view`, `classify_cited_head_citation`, interrupteur, consts. Compile et passe
   `clippy` sans être câblé. **Plancher :** les huit tests de classification sont verts à ce
   commit, le contrôle positif et le contrôle négatif compris, et
   `cargo test -p mika-agent canonical_tokens` reste vert (VC11).
2. `feat(3135): le garde refuse avant le subprocess, entre mika#2961 et mika#2455`
   — `builtin_handlers.rs` : `validate_cited_head_grounding`, ses deux lignes d'audit, son câblage
   à `:5707` et le commentaire qui justifie sa place dans l'ordre de la chaîne. **Plancher :**
   `cargo test -p mika-agent` entier vert — c'est le commit qui pourrait casser un voisin.
3. `docs(3135): MIKA_CITED_HEAD_GATE, son défaut et sa cause d'abstention`
   — `docs/operator/environment-variables.md`. Hors compte de taille.
