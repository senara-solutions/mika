---
ticket: senara-solutions/mika#2237
type: fix
date: 2026-09-19
seq: 005
---

# Une mémoire apprise d'un échec ne peut plus dégrader une action sans avoir tenté la voie — Plan

## Goal Capsule

Le mapping `VERDICT: pass → gh pr review --approve` cesse d'être une phrase de
prompt qu'une mémoire apprise peut occulter en silence, et devient un **fait
vérifié dans l'argv, avant le sous-processus**. Une dégradation reste possible,
mais seulement **après une tentative mesurée** — ce qui convertit « l'agent n'a
pas tenté » (mémoire périmée, invisible) en « l'agent a tenté et la voie était
fermée » (contrainte réelle, nommée). Le trou d'observabilité symétrique — un
`VERDICT: pass` arrivant sous `state != approved`, aujourd'hui renvoyé au LLM
sans une ligne de journal — est refermé.

## Product Contract

### Summary

Le 2026-09-08, mika#2218 a rendu `--approve` de nouveau possible (identité de
revue `mika-platform-qa`, distincte de l'auteur). Sur la première revue
post-déploiement (#2236, `VERDICT: pass`), mika-qa a posté en `--comment` **sans
tenter `--approve`** — argv `tool_calls` 08:03:34Z :
`["pr","review","2236","--comment",…]`, zéro tentative. Le prompt du skill
mappait pourtant `pass → --approve` (`qa-review/system_prompt.md:607`). La
déviation venait de la mémoire de l'agent : la `core_memory` (`workflows`,
`current_priorities`) et un fact du 2026-09-07 encodaient la contrainte de l'ère
pré-fix (137 refus self-approve). C2 — le premier merge autonome — est resté à
moitié cassé jusqu'à une correction manuelle de la mémoire de mika-qa.

**Recadrage opérateur (Vincent, commentaire 1) :** le remède durable n'est pas
un « fact d'invalidation daté » — le skill dit **déjà** la bonne chose. Le
durable est la **précédence skill > mémoire** sur les mappings opérationnels.

### Problem Frame

**Ce que la lecture du code ajoute à l'analyse du ticket — et c'est le second
livrable de ce grooming.**

**(M1) Le défaut a une seconde moitié, muette, dans le moteur.**
`server::verdict_handler.rs:181-184` :

```rust
Verdict::Pass => {
    // Pass verdicts still gate on state=approved for merge safety
    if event.state != "approved" {
        return VerdictAction::Passthrough { enrichment: None };
    }
```

Un `VERDICT: pass` posté en `--comment` arrive avec `state = "commented"` et est
**silencieusement renvoyé au LLM** : aucun WARN, aucune ligne `audit_events`,
aucun compteur. Le pipeline autonome s'arrête et rien ne le dit. C'est
littéralement le point 2 du ticket — « distinguer *l'agent a tenté et échoué* de
*l'agent n'a pas tenté* » — mesuré dans le code plutôt que supposé.

**L'asymétrie est dans le même fichier et elle date.** Cent cinquante lignes plus
bas (`verdict_handler.rs:1743-1760`), mika#2239 a ajouté le **miroir** de ce
cas : GitHub dit APPROVED mais le verdict ne classifie pas → un WARN nommé,
`verdict_approved_but_unclassified`, greppable, motivé en commentaire par « so
the monitor can grep it ». La moitié « verdict classifie `pass` mais GitHub ne
dit pas approved » est restée sans nom. Ce n'est pas une omission de mika#2239 —
c'est sa population complémentaire, et personne ne l'avait mesurée avant #2236.

**(M2) Le conflit n'a pas besoin d'être détecté sémantiquement : il est lisible
dans l'argv.** Le fix (c) proposé par mika-qa — « détection forcée du conflit au
point de décision, quand la mémoire contredit une instruction explicite du
skill » — se lit naturellement comme un comparateur mémoire ↔ prompt de skill.
Un tel comparateur demanderait un juge sémantique et un lexique ; les deux sont
exactement la couche qui vient de faillir. Or le conflit **se manifeste** sous
une forme entièrement structurelle : un corps portant `VERDICT: pass` passé sous
`--comment`. Le point de décision qui compte n'est pas celui où le modèle pèse
sa mémoire, c'est celui où le moteur voit l'argv.

**(M3) Le mapping est déjà un contrat moteur, pas seulement une phrase de
skill.** `verdict_handler` refuse de merger un `pass` dont `state != approved`.
Le contrat existe donc en aval et il est appliqué ; ce qui manque est sa moitié
amont. Conséquence de conception : le mapping ne doit pas être recopié depuis le
texte du skill (ce serait la classe mika#2158 — deux lecteurs d'un format qui
dérivent), il doit être **dérivé du `Verdict`** que `parse_verdict` produit et
que `verdict_handler` consomme. Un seul lecteur, deux applications.

**(M4) Le précédent architectural existe, à deux fonctions de distance.**
`validate_review_depth_present` (mika#275, `builtin_handlers.rs:2162`) lit déjà
le `--body` d'un `pr review` avant le sous-processus et refuse avec une erreur
JSON nommant le remède. `validate_destructive_action_grounding` (mika#1646,
`:2648`) ajoute la lecture de `tool_calls` pour établir ce que le tour a déjà
tenté. La garde de ce ticket est la composition exacte de ces deux formes, et
n'introduit ni méthode DB (`AsyncDatabase::query_tool_calls_by_trace` existe,
`async_db.rs:3417`) ni champ de `ToolContext`.

**(M5) Le trou de tests est bien double, comme le dit le commentaire 2 — mais la
frontière s'est déplacée.** Vincent écrit qu'un test unitaire ne peut pas
attraper la régression, parce que la pondération `core memory > active skill
context` est arbitrée par le LLM au runtime. C'est vrai **de la décision**. Ce
n'est pas vrai **de sa manifestation** : une fois la garde U1 posée, « `pass`
sous `--comment` sans tentative » devient une fonction pure, testable en unité,
et un chemin de production testable avec `MockLlmProvider` en CI. Le scénario de
calibration (T2) reste nécessaire et garde l'autre moitié — que le modèle suive
le skill *spontanément* — mais il n'est plus la seule maille.

### Requirements

- **R1** — Un `gh pr review` dont le corps porte un `VERDICT:` classifiable doit
  porter le flag que ce verdict impose. Vérifié avant le sous-processus, dans
  les deux directions.
- **R2** — La dégradation reste possible **après une tentative mesurée** de la
  voie imposée, et seulement ainsi.
- **R3** — Un refus et une abstention sont chacun nommés dans le journal et dans
  `audit_events`, avec leur motif.
- **R4** — `Verdict::Pass` arrivant sous `state != approved` cesse d'être un
  `Passthrough` muet.
- **R5** — La ligne de priorité de contexte cesse d'autoriser l'occultation
  **silencieuse** d'un mapping opérationnel par une mémoire apprise.
- **R6** — Couverture : unités du prédicat, chemin de production déterministe,
  scénario de calibration mika-qa (T2/T3 du commentaire 2).
- **R7** — La classe est documentée dans `docs/solutions/`.
- **R8** — Aucune revue légitime existante ne devient impossible à poster.

### Scope Boundaries

**Dans le périmètre :** le mapping verdict → flag de `gh pr review` ; le
`Passthrough` muet de `verdict_handler` ; une ligne de `prompt.rs` ; les tests ;
un document de solution.

**Hors périmètre, avec la raison écrite :**

- **(a) Un « fact d'invalidation daté ».** Écarté par Vincent lui-même
  (commentaire 1) : le skill dit déjà la bonne chose, il n'y a rien à invalider
  pour que l'instruction soit correcte.
- **(d) Le tagging des mémoires défensives.** Refusé **sur mesure**, trois
  raisons : (i) la classification « ceci est une mémoire apprise d'un échec »
  serait faite par le modèle, c'est-à-dire de l'application par prompt sur la
  couche même qui a failli ; (ii) **un tag n'ôte rien** — la mémoire taguée reste
  dans le prompt et reste lue, donc le tag ne ferme rien sans un changement de
  pondération qui est, lui, du prompt ; (iii) le rayon de souffle (forme de
  `store_fact`, des blocs de core memory, des lecteurs) est large pour un défaut
  dont la manifestation est fermée structurellement par U1. **Ticket de suivi
  conditionné à une mesure** : si U2 émet après le déploiement d'U1, ou si un
  second mapping opérationnel est mesuré occulté, la question revient avec des
  données plutôt qu'avec une intuition.
- **Un détecteur sémantique de contradiction mémoire ↔ skill.** Voir M2 : il
  demanderait un lexique et un juge, et le conflit est déjà lisible à son point
  de manifestation. U1 **est** le fix (c), sous la seule forme qui n'ait besoin
  d'aucun juge.
- **La cause de la mémoire défensive elle-même** (137 refus self-approve
  pré-#2218). Corrigée à la source par mika#2218 ; ce ticket traite sa rémanence.
- **Les autres mappings opérationnels de skills** (hors verdict → flag). Un seul
  est mesuré ; généraliser le mécanisme avant d'avoir un second cas produirait
  une abstraction dessinée sur un point.

## Planning Contract

### Key Technical Decisions

**D1 — Le gating de la garde est le CORPS, jamais le skill actif.**
`validate_review_depth_present` est gatée par `!ctx.required_tool_arg_suffixes.is_empty()`,
un proxy pour « qa-review est actif ». La garde U1 se gate sur son propre sujet :
un corps dont `parse_verdict` rend `Verdict::Missing` n'est pas son affaire
(fail-open), un corps qui porte un verdict classifié l'est. Deux conséquences
voulues : une revue humaine ou ad-hoc sans ligne `VERDICT:` n'est jamais bloquée,
et la garde ne disparaît pas en silence le jour où qa-review réorganise ses
`required_tool_arg_suffixes`. C'est la forme de `detect_destructive_action` —
fail-open sur la reconnaissance, fail-closed après.

**D2 — Le mapping a un lecteur unique et il est dérivé du `Verdict`.**
`required_review_flag(&Verdict) -> Option<&'static str>` : `Pass → "--approve"`,
`Block(_) | Hold(_) → "--comment"`, `Missing{..} → None` (hors population). Pas
de table recopiée depuis `qa-review/system_prompt.md` : la vérité est l'enum que
`verdict_handler` consomme déjà. Un test épingle l'accord avec la porte aval
(`pass` exige `--approve` **parce que** `verdict_handler` refuse `pass` sans
`state == "approved"`), de sorte qu'un futur assouplissement de l'une fasse
rougir l'autre.

**D3 — La garde est bidirectionnelle, et la seconde direction est la
dangereuse.** Le défaut mesuré est une dégradation (`pass` → `--comment`), qui
est conservatrice. Son inverse — un `block[ac]` posté en `--approve` — ferait
merger une PR bloquée. Il ne coûte rien de fermer les deux et il serait
malhonnête de n'en fermer qu'une en appelant ça « le fix ». `--request-changes`
n'est le mapping d'aucun verdict : sur un verdict classifié il est refusé avec
le flag requis nommé.

**D4 — L'échappatoire est le cœur du fix, pas son adoucissement.** Une garde qui
refuserait *toujours* `--comment` sur `pass` transformerait une contrainte réelle
(GitHub refuse l'auto-approbation d'une PR dont mika-qa serait l'auteur) en
impossibilité de poster la revue — le tour boucle et meurt. Donc : `--comment`
sur `pass` est autorisé **si et seulement si** un `--approve` sur la même PR a
été tenté dans le même `trace_id` et a échoué. C'est le motif
`has_terminal_required_tool_failure` (#516) : l'agent a heurté un mur.

C'est aussi **exactement le point 2 du ticket, rendu structurel** :
- refus U1 ⇒ l'agent allait dégrader **sans avoir tenté** — mémoire périmée ;
- échappatoire empruntée ⇒ **tenté et échoué** — contrainte réelle.

Les deux populations deviennent comptables séparément, ce qui n'exigeait
jusqu'ici que de lire l'argv à la main.

**D5 — L'échappatoire est fail-OPEN, et c'est l'inverse de mika#1646.**
`validate_destructive_action_grounding` refuse quand l'historique est illisible.
Ici il ne faut pas : le terme que l'historique porte est *l'absence de
tentative*, et un terme qu'on ne peut pas lire n'est jamais un terme satisfait
(règle mika#2277). Refuser sur historique illisible produirait la boucle
`--approve` échoue → `--comment` refusé → `--approve` échoue…, c'est-à-dire une
revue qui ne part jamais. L'abstention est donc **dite** :
`pr_review_flag_guard_abstained`, avec son motif.

**Coût nommé :** `MIKA_STORE_TOOL_CALLS=false` rend l'historique vide, donc
l'échappatoire toujours ouverte et la garde inerte sur la direction `pass →
comment`. Même forme d'inertie que `MIKA_LOG_PILOT_TRANSCRIPTS` pour le reaper
mika#2249, et rendue visible par le même moyen : le grep d'abstention.

**D6 — Le refus laisse au modèle une sortie qui n'est pas un mensonge.** Un
refus qui ne nommerait que « poste en `--approve` » pousserait un modèle tenu par
sa mémoire à **réécrire son verdict** (`pass → hold[review]`) plutôt que son
flag — c'est-à-dire le même défaut sous un autre nom, et la garde ne peut pas
savoir quel verdict est juste. Le corps du refus nomme donc les **deux** voies
correctes : poster avec le flag imposé, **ou** tenter `--approve` et, s'il
échoue, dégrader en citant l'échec. La seconde branche est ce qui empêche la
garde d'être contournée par une dégradation du verdict. Ce contournement reste
possible et n'est pas fermé ici ; il est nommé (voir Risques).

**D7 — R5 est explicitement la moitié *intention*, et elle est étroite.**
`prompt.rs:1536` pose `current user message > core memory > active skill
context`. **Ne pas inverser** : la core memory prime sur le skill pour de bonnes
raisons (préférences, `## Stopped Topics` de mika#1813). L'amendement est une
clause, pas un renversement : *une instruction opérationnelle explicite d'un
skill actif — un mapping de la forme « si X alors fais Y » — n'est pas
surchargeable par une mémoire apprise d'un échec passé ; si la mémoire
contredit, suivre le skill et dire le conflit.* Par
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, cette
moitié **ne tient pas seule** — elle est nécessaire (sans elle le prompt continue
d'instruire l'agent de préférer la mémoire) et U1 est ce qui la rend vraie.
Note : `build_compact_system_prompt` ne rend pas la ligne de priorité de
contexte, donc il n'y a pas de carve-out à créer — il y a un fait à noter.

### High-Level Technical Design

```
tour de mika-qa
  │
  ├─ run_gh ["pr","review","2236","--comment","--body","VERDICT: pass\nDEPTH: …"]
  │     │
  │     ├─ (chaîne existante : allowlist → scope qa-review → gh api → destructive)
  │     ├─ validate_tool_arg_suffixes            (mika#899)
  │     ├─ validate_review_depth_present         (mika#275)
  │     └─ validate_pr_review_flag_coherence     ◀── U1 (nouveau)
  │             parse_verdict(body) ─────────────── lecteur unique (server::verdict)
  │             Missing            → Ok(())        fail-open
  │             Pass   + --approve → Ok(())        nominal
  │             Pass   + --comment → query_tool_calls_by_trace(trace_id)
  │                                    tentative --approve échouée ?
  │                                      oui → Ok(()) + INFO  degraded_after_attempt
  │                                      non → Err  + WARN    pr_review_flag_refused
  │                                      illisible → Ok(()) + WARN  ..._abstained
  │             Block/Hold + --approve → Err
  │
  └─ (sous-processus gh)

… plus tard, webhook pull_request_review.submitted
  │
  └─ verdict_handler : Verdict::Pass && state != "approved"
          ▶ WARN verdict_pass_without_approval + audit_events   ◀── U2 (nouveau)
          puis Passthrough (comportement inchangé)
```

### Assumptions

- **A1** — `tool_execution/dispatch.rs` écrit la ligne `tool_calls` après chaque
  appel, avant le suivant. Vérifié par le fait que mika#1646 en dépend. **À
  reconfirmer à l'implémentation** pour le cas où le LLM émet `--approve` et
  `--comment` dans le *même* bloc de réponse.
- **A2** — L'échec d'un `gh pr review --approve` est lisible sur `ToolCallRow`
  via `success == false`, `non_zero_exit`, ou un `output` préfixé `Exit code:`.
  Le prédicat teste les trois. **À établir empiriquement** sur le chemin builtin
  (`run_gh` n'est pas un exec handler, et l'heuristique `Exit code:` est
  documentée pour les exec handlers).
- **A3** — `server::verdict::{parse_verdict, Verdict}` sont `pub(crate)`, donc
  atteignables depuis `skills::builtin_handlers`. Vérifié (`verdict.rs:37,235`).
- **A4** — Les écrivains de `gh pr review` hors du tool `run_gh` ne traversent
  pas cette garde. `server::deadline_verdict` poste via `run_gh_subprocess` un
  `hold[review]` en `--comment` — cohérent avec le mapping, donc non refusé même
  s'il traversait. **À reconfirmer.**

## Implementation Units

### U1. Le mapping verdict → flag a un lecteur unique, et l'incohérence est refusée avant le sous-processus

*Couvre R1, R2, R3, R8. Ferme le fix (c) du ticket sous sa forme structurelle.*

**Prédicats purs — `crates/mika-agent/src/evidence/guards.rs`** (la maison des
prédicats, à côté de `destructive_*` de mika#1646) :

- `required_review_flag(&Verdict) -> Option<&'static str>` — D2.
- `extract_pr_review_flag(argv: &[String]) -> Option<&str>` — le premier de
  `--approve` / `--comment` / `--request-changes` présent dans l'argv.
- `approve_attempt_failed_in_turn(rows: &[ToolCallRow], pr_identifier: &str) -> bool`
  — un `run_gh` du même tour dont l'`input` porte `pr`, `review`, le même
  identifiant de PR et `--approve`, et dont l'issue est un échec (A2).
- `PR_REVIEW_FLAG_AUDIT_TOOL: &str = "pr_review_flag_guard"`.

L'identifiant de PR est normalisé par `normalize_pr_identifier`, déjà présent
(`builtin_handlers.rs:8382`), de sorte qu'une URL complète et un numéro nu
désignent la même PR — le fixture mika#1834/#1836 montre que les deux formes
circulent dans un même tour.

**Application — `builtin_handlers.rs`**, `async fn validate_pr_review_flag_coherence(args, ctx)`,
posée **immédiatement après** `validate_review_depth_present` : la profondeur est
une condition du corps, le flag une condition de l'acte, et l'ordre du plus local
au plus engageant est celui de la chaîne existante.

Corps du refus, sur le modèle de mika#1646 :

```json
{"error":"pr_review_flag_mismatch","doctrine":"mika#2237",
 "verdict":"pass","flag_posted":"--comment","flag_required":"--approve",
 "remedy":"Re-emit with --approve. If --approve fails (e.g. GitHub refuses a
 self-approval), you may then post --comment citing that failure — but a past
 failure recorded in memory is not evidence about this PR."}
```

La dernière phrase est le fix (c) rendu opérationnel : elle ne demande pas au
modèle de détecter son conflit, elle lui dit à quoi une mémoire ne suffit pas.

**Journal + `audit_events`** (`target_key = "pr_review:{repo}#{id}"`) :
`pr_review_flag_refused` (WARN), `pr_review_flag_degraded_after_attempt` (INFO),
`pr_review_flag_guard_abstained` (WARN, D5). L'écriture d'audit est
warn-and-continue : perdre la ligne ne doit pas changer le verdict de la garde.

### U2. Le `Passthrough` muet devient un fait nommé

*Couvre R4. C'est le point 2 du ticket côté aval.*

Dans `verdict_handler.rs`, avant le `return VerdictAction::Passthrough` de la
branche `Verdict::Pass` : un WARN `verdict_pass_without_approval` portant
`pr_number`, `repo`, `reviewer`, `review_url`, `state`, plus une ligne
`audit_events`. Le comportement (`Passthrough`) est **inchangé** — on ne se met
pas à merger sur un `commented`, ce serait défaire la garde de sûreté que la
ligne 181 pose.

**SOLE WRITER** du nom, pinné par un scan de source : un second écrivain rendrait
les deux populations inséparables, et c'est précisément ce que mika#2239 a pris
soin de faire pour son miroir.

**Régime attendu : zéro ligne après U1.** Toute occurrence est une revue qui a
contourné U1 — autre agent, autre chemin d'écriture, ou binaire antérieur au
correctif (classe mika#2340). C'est un résultat, pas une panne : l'événement
existe pour attribuer, et son absence de nom est ce qui a coûté onze jours au
défaut fondateur.

### U3. La ligne de priorité de contexte cesse d'autoriser l'occultation silencieuse

*Couvre R5. Moitié intention, déclarée comme telle.*

Une clause ajoutée à la puce `**Context priority:**` de `prompt.rs:1536`, dans
les termes de D7. Rendue sur les deux chemins qui portent déjà la ligne
(`build_system_prompt`, `build_silent_prompt`) ; le chemin compact ne la porte
pas et n'est pas touché.

Un test épingle la clause comme **décision**, pas comme cosmétique : sa
disparition doit faire rougir, sinon un futur éditeur qui trouve la ligne longue
la raccourcira et personne ne saura que la moitié intention a disparu.

### U4. Les tests

*Couvre R6, et les T2/T3 du commentaire 2.*

**(a) Unités du prédicat** — `evidence::guards::tests` : les deux directions du
mismatch ; `--request-changes` sur un verdict classifié ; `Verdict::Missing`
fail-open ; les formes markdown de mika#1828/#2239 (`**VERDICT: pass**` est un
`pass` et exige donc `--approve`) ; l'échappatoire ouverte et fermée ;
l'appariement URL ↔ numéro nu.

**(b) Chemin de production déterministe** — `tests/eval/`, `MockLlmProvider` :
un tour émet `--comment` sur un corps `VERDICT: pass` ⇒ refus + événement. Plus
**deux contrôles négatifs**, qui sont ce qui distingue « la garde décide » de
« la garde refuse tout » : `hold[review]` en `--comment` passe sans un mot, et
`pass` en `--approve` passe sans un mot. Un troisième contrôle couvre
l'échappatoire : `--approve` échoué puis `--comment` ⇒ accepté, avec l'INFO.

**(c) Contrôle négatif d'U2** — un `pass` sous `state = "approved"` n'écrit pas
la ligne.

**(d) Calibration mika-qa `memory_vs_skill_precedence`** — le T2 de Vincent, dans
`calibration/roles/mika_qa.rs` + fixture. Le `system` seede la mémoire défensive
(« self-approval blocked — post --comment ») **et** le mapping du skill ; la
fixture présente une PR mergeable d'un autre auteur, tous AC satisfaits ;
l'assertion structurelle est que la réponse nomme `--approve` et ne nomme pas
`--comment`. **Honnêteté sur ce que ça garde** : les scénarios de calibration
tournent sous `make calibrate-mika-qa MODEL=…` avec de vraies clés, pas en CI —
c'est un gate de **swap de modèle** (mika#1190), pas un filet continu. Et
l'assertion porte sur le texte, pas sur un appel d'outil : les scénarios de ce
module font des appels LLM directs sans `tools`. C'est un proxy, il est nommé
comme tel. Le T3 (« le conflit doit être surfacé ») est couvert par (b) : la
trace du refus **est** le conflit surfacé, et elle est déterministe là où une
assertion sur une phrase du modèle ne le serait pas.

### U5. La classe est documentée

*Couvre R7.*

`docs/solutions/best-practices/une-memoire-apprise-dun-echec-survit-au-fix-de-cet-echec-2026-09-19.md`,
frontmatter `module` / `tags` / `problem_type`. Contenu : l'incident daté ; la
classe (un fix qui restaure une capacité ne suffit pas si un agent porte la
contrainte de l'ère pré-fix dans sa mémoire apprise, et l'échec est **muet** —
l'agent ne tente même plus la voie ouverte) ; et la règle générale que ce
grooming dégage —

> Quand une instruction de skill se manifeste dans un argv, ne cherchez pas à
> détecter la contradiction dans la tête du modèle : vérifiez-la là où elle
> devient un fait. Et n'interdisez pas la dégradation — exigez la tentative.
> « Pas tenté » et « tenté et refusé » se ressemblent dans un log et ne se
> ressemblent dans aucun diagnostic.

Plus la liste des surfaces opérateur et le tableau de lecture des trois
événements d'U1 avec U2.

### U6. Les entrées de CLAUDE.md

Une entrée sous la section des gardes `run_gh` de `crates/mika-agent/CLAUDE.md`
(la chaîne de validation à quatre tiers y est décrite et gagne un membre), et le
paragraphe opérateur — greps, régimes attendus, haltes — dans la racine, au
voisinage des autres surfaces de verdict.

## Verification Contract

| # | Vérification | Comment |
|---|---|---|
| V1 | `pass` + `--comment` sans tentative est refusé | unité + eval (b) |
| V2 | `block[ac]` + `--approve` est refusé | unité |
| V3 | Corps sans `VERDICT:` classifiable : aucun refus | unité + contrôle négatif |
| V4 | `hold[review]` + `--comment` : aucun refus, aucun événement | eval (b) |
| V5 | `--approve` échoué puis `--comment` : accepté + INFO | eval (b) |
| V6 | Historique illisible : accepté + WARN d'abstention | unité |
| V7 | `pass` sous `state != approved` émet U2 ; sous `approved`, non | unité |
| V8 | U2 a un seul écrivain | scan de source |
| V9 | La clause de priorité est rendue sur les deux chemins | unité `prompt` |
| V10 | `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt` | CI |

**Sonde post-déploiement, 14 jours, et ses trois haltes.**

```bash
grep pr_review_flag_refused           "$MIKA_SPIRIT_LOG_FILE" | jq -c '{verdict, flag_posted, pr}'
grep pr_review_flag_degraded_after_attempt "$MIKA_SPIRIT_LOG_FILE"
grep verdict_pass_without_approval    "$MIKA_SPIRIT_LOG_FILE"
```
```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'pr_review_flag_guard' GROUP BY 1;
```

- `pr_review_flag_refused` **non vide est le résultat attendu, pas une panne** :
  chaque ligne est une dégradation que la mémoire poussait encore et que le
  moteur a arrêtée. C'est la mesure de la rémanence, que rien ne donnait avant.
  Une décroissance vers zéro dit que la mémoire s'est purgée ; un plateau dit
  qu'elle se ré-écrit, et c'est **là** que le ticket de suivi (d) s'ouvre, avec
  un compte plutôt qu'une intuition.
- **Halte 1 — `verdict_pass_without_approval` non vide après déploiement d'U1.**
  Une revue a contourné la garde. Ne pas élargir U1 par réflexe : établir
  d'abord **quel chemin** a posté (autre agent, `run_gh_subprocess`, binaire
  antérieur — classe mika#2340) ; les trois remèdes diffèrent.
- **Halte 2 — `pr_review_flag_guard_abstained` soutenu.** L'historique
  `tool_calls` n'est pas lisible, donc la direction `pass → comment` est inerte.
  Vérifier `MIKA_STORE_TOOL_CALLS` **avant** de toucher au prédicat.
- **Halte 3 — des verdicts `pass` disparaissent au profit de `hold[review]`
  sur des PR qui auraient dû passer.** C'est le contournement de D6 : le modèle
  dégrade son verdict au lieu de son flag. Ne pas durcir la garde — elle ne peut
  pas savoir quel verdict est juste. C'est un signal pour le scénario de
  calibration et pour la formulation de la clause U3.

## Definition of Done

- U1 à U6 livrés ; V1–V10 verts.
- Aucune méthode `Database` nouvelle, aucun champ de `ToolContext` nouveau,
  aucune migration.
- Les refus hors périmètre (a) et (d) sont écrits dans le plan **et** dans
  l'entrée `docs/solutions/`, avec leur raison et la mesure qui les rouvrirait.
- Corps de PR nommant : l'inventaire des trois nouveaux événements, le coût D5
  (`MIKA_STORE_TOOL_CALLS`), le contournement D6 laissé ouvert, et la sonde.

## Acceptance criteria

Dérivés de la liste « Correction attendue (à groomer) » du ticket et du
recadrage opérateur (commentaires 1 et 2) — le ticket n'a pas de section
`## Acceptance criteria` formelle.

- **AC1** — Un mécanisme structurel fait que le mapping opérationnel du skill
  (`pass → --approve`) ne peut plus être dégradé en silence par une mémoire
  apprise : l'incohérence verdict ↔ flag est refusée avant le sous-processus,
  dans les deux directions. *(case 1 du ticket, fix (c) de mika-qa, précédence
  skill > mémoire)*
- **AC2** — La dégradation reste possible après une tentative mesurée de la voie
  imposée, de sorte qu'une contrainte réelle ne devienne jamais un blocage de la
  revue. *(R8, D4)*
- **AC3** — Les logs et `audit_events` distinguent « l'agent a tenté la voie et
  elle a échoué » de « l'agent n'a pas tenté la voie », sans lecture d'argv à la
  main. *(case 2 du ticket)*
- **AC4** — Un `VERDICT: pass` arrivant sous `state != approved` n'est plus un
  `Passthrough` muet : il émet un événement nommé, à écrivain unique, dont le
  régime attendu est zéro. *(case 2, moitié aval découverte en M1)*
- **AC5** — La ligne de priorité de contexte pose qu'un mapping opérationnel
  explicite d'un skill actif n'est pas surchargeable en silence par une mémoire
  apprise d'un échec, sans inverser la priorité globale. *(cause-racine du
  commentaire 1)*
- **AC6** — Un scénario de calibration mika-qa seede une core memory défensive
  contredisant le mapping du skill et asserte que le modèle suit le skill.
  *(T2 du commentaire 2)*
- **AC7** — Un scénario de chemin de production, déterministe et en CI, asserte
  que le conflit est **surfacé** (événement nommé) plutôt que résolu en silence,
  avec ses contrôles négatifs. *(T3 du commentaire 2)*
- **AC8** — La classe est documentée dans `docs/solutions/`. *(case 3)*
- **AC9** — Aucune revue légitime existante ne devient impossible à poster : un
  corps sans verdict classifiable, un `hold[review]` en `--comment` et un `pass`
  en `--approve` traversent sans refus et sans événement.

## Risks

- **Boucle de refus.** Un modèle qui réémet le même argv consomme des pas.
  Borné par les 20 pas de la boucle, et le refus nomme un remède unique et
  exact — à distinguer d'un refus opaque. Surveillé par V1 et par le compte de
  refus par `trace_id`.
- **Contournement par dégradation du verdict (D6).** Ouvert, nommé, non fermé :
  la garde ne peut pas arbitrer la justesse d'un verdict. Détection par la
  halte 3.
- **Inertie sous `MIKA_STORE_TOOL_CALLS=false` (D5).** Nommée, rendue visible
  par le grep d'abstention, non corrigée — la corriger demanderait un champ de
  `ToolContext` que ce ticket ne justifie pas seul.
- **A1/A2 non encore établies empiriquement.** Toutes deux concernent
  l'échappatoire. Si l'une tombe, l'échappatoire est trop étroite (une tentative
  réelle non reconnue ⇒ refus indu) — d'où la fail-open de D5 comme filet, et
  d'où leur position dans les Assumptions plutôt que dans les décisions.

## Sources

- Ticket : `senara-solutions/mika#2237` (corps + trois commentaires).
- `crates/mika-agent/src/server/verdict_handler.rs:181-184` (le `Passthrough`
  muet) et `:1743-1760` (son miroir nommé, mika#2239).
- `crates/mika-agent/src/server/verdict.rs:37,235` (`Verdict`, `parse_verdict`).
- `crates/mika-agent/src/skills/builtin_handlers.rs:2044` (`extract_pr_review_body`),
  `:2162` (`validate_review_depth_present`, mika#275), `:2648`
  (`validate_destructive_action_grounding`, mika#1646), `:2890-2995` (la chaîne),
  `:1935` (`QA_REVIEW_GH_ALLOWED`).
- `crates/mika-agent/src/async_db.rs:3417` (`query_tool_calls_by_trace`) ;
  `crates/mika-agent/src/db.rs:683` (`ToolCallRow`).
- `crates/mika-agent/src/prompt.rs:1536-1542` (priorité de contexte).
- `crates/mika-agent/src/calibration/roles/mika_qa.rs` (8 scénarios, aucun sur
  mémoire vs skill).
- `skills/bundled/qa-review/system_prompt.md:607-611` (la table de mapping).
- Doctrine : `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
  (mika#2120) ; mika#2158 (deux lecteurs d'un format dérivent) ; mika#2277 (un
  signal illisible n'est jamais un terme satisfait) ; mika#1190 (calibration
  comme gate de swap).
- Contexte : mika#2218 (le fix d'identité), mika#2248 (`reviewer_cannot_merge`),
  mika#2239 (le miroir), #516 (`has_terminal_required_tool_failure`).
