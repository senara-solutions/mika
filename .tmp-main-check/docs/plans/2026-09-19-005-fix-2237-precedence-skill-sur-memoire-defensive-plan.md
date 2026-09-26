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
post-déploiement (#2236, corps `VERDICT: pass ✅`), mika-qa a posté en
`--comment` **sans tenter `--approve`** — argv `tool_calls` 08:03:34Z :
`["pr","review","2236","--comment",…]`, zéro tentative. Le prompt du skill
mappait pourtant `pass → --approve` (`qa-review/system_prompt.md:607`). La
déviation venait de la mémoire de l'agent : la `core_memory` (`workflows`,
`current_priorities`) et un fact du 2026-09-07 encodaient la contrainte de l'ère
pré-fix (137 refus self-approve). C2 — le premier merge autonome — est resté à
moitié cassé jusqu'à une correction manuelle de la mémoire de mika-qa.

**Deux défauts distincts sur la même revue, et il faut les tenir séparés (F2).**
La revue de 08:03:23Z est aussi celle de mika#2239 : son corps portait
`VERDICT: pass ✅`, forme décorée que `parse_verdict` ne classifiait pas alors —
épinglée depuis comme régression fondatrice de ce ticket-là
(`verdict.rs::parse_verdict_field_shape_pr2236`). Les deux défauts sont
**empilés, non confondus**, et l'ordre causal les sépare : le choix du flag est
fait par le modèle **en même temps qu'il écrit le corps**, le parser moteur
n'intervient qu'ensuite, sur le webhook. Un parser aveugle en aval ne peut donc
pas avoir produit le `--comment` en amont.

**Ce que la re-mesure change réellement, et ce n'est pas rien.** La même
`mika-platform-qa`, le même jour, a posté **trois revues `APPROVED`** (08:25:08Z,
08:47:25Z, 09:29:39Z). Donc « zéro tentative » est vrai **de ce tour** et faux de
la journée : la mémoire défensive n'était pas un blocage permanent, c'était un
arbitrage qui a gagné *par intermittence*. Deux conséquences que le plan assume :

- toute formulation suggérant un empêchement stable serait fausse, et celles du
  plan sont corrigées en conséquence ;
- **c'est un argument de plus pour le périmètre retenu.** Un remède côté mémoire
  (tag, invalidation datée) suppose un état persistant à corriger ; ce qui est
  mesuré est un arbitrage non déterministe, tour par tour. Une garde qui lit
  l'argv mord exactement sur les tours où la mémoire gagne et se tait sur les
  autres — la granularité du remède épouse celle du défaut. Voir scope-out (d).

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

> **Ancrage re-vérifié contre l'arbre (F1).** Le miroir **existe en code**, et la
> divergence signalée vient de ce que le **corps du ticket** mika#2239 le
> présente comme une intention (« Envisager qu'un `Verdict::Missing` sur une
> review APPROVED+CLEAN émette un WARN nommé ») tandis que le **fix l'a livré** :
> `warn!(event = "verdict_approved_but_unclassified", …)` à
> `crates/mika-agent/src/server/verdict_handler.rs:1752`, introduit par
> `8f3783f2` — *« fix(verdict): parse_verdict tolère la décoration de fin de
> valeur (mika#2239) »*, PR #2241, 2026-09-08 — avec le commentaire
> `// mika#2239 (D2c)` juste au-dessus. C'est le ticket qui est en retard sur son
> propre correctif, pas le plan sur l'arbre. Vérifiable en une commande :
> `grep -rn verdict_approved_but_unclassified crates/` rend deux sites, le
> `warn!` ci-dessus et sa condition de réveil documentée dans `verdict.rs:216`.
>
> Conséquence pour U2, et elle **renforce** le cadrage plutôt qu'elle ne
> l'affaiblit : U2 n'est pas le premier signal nommé de la paire, c'est le
> **second**, et son « SOLE WRITER, pinné par un scan de source » reprend
> délibérément la discipline que mika#2239 s'est appliquée à lui-même. Les deux
> moitiés d'une même asymétrie se lisent alors sous deux noms distincts et
> restent comptables séparément — ce qui est exactement ce que le scan de source
> protège.

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
  dont la manifestation est fermée structurellement par U1 ; **(iv) la
  re-mesure de #2236 (voir Summary) montre que la mémoire défensive produit un
  arbitrage *intermittent* — trois `APPROVED` et un `--comment` le même jour, par
  le même agent — et non un empêchement stable.** Un remède côté mémoire suppose
  un état persistant à corriger ; ce qui est mesuré varie d'un tour à l'autre,
  donc le remède doit décider tour par tour, ce que fait une garde sur l'argv et
  ne fait pas un tag. **Ticket de suivi conditionné à une mesure** : si U2 émet
  après le déploiement d'U1, ou si un second mapping opérationnel est mesuré
  occulté, la question revient avec des données plutôt qu'avec une intuition.
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

**Corollaire porteur, et c'est la moitié (b) de F2 : la tolérance à la décoration
est héritée par construction, jamais réimplémentée.** La garde n'inspecte pas le
corps elle-même — elle appelle `parse_verdict`, donc elle hérite d'un coup de
toute la normalisation que ce lecteur porte : l'emphase markdown de mika#1828
(`**VERDICT: pass**`, `__…__`, emphase simple, emphase déséquilibrée), les alias
(`approved`, `changes requested`), **et le repli décoration de mika#2239**
(`strip_trailing_decoration`, `verdict.rs:214-220`). C'est ce qui rend un corps
`VERDICT: pass ✅` — la forme littérale de l'incident fondateur — porteur de
`Verdict::Pass`, donc **exigeant `--approve`**, plutôt que `Missing` et donc
hors population par fail-open.

**Ce que ce corollaire ferme, dit explicitement :** une garde qui aurait recopié
un `contains("VERDICT: pass")` ou une regex maison aurait fail-open
**précisément sur la forme de corps qui a motivé le ticket** — muette là où elle
devait mordre, et indistinguable d'une garde qui fonctionne. C'est la raison
pour laquelle « lecteur unique » n'est pas ici une préférence de style
(mika#2158) mais la condition de correction du mécanisme. Les bornes héritées le
sont aussi, et c'est voulu : `VERDICT: pass — but see findings` reste `Missing`
(borne mika#1821) et traverse donc sans refus. La décoration de **tête**
(`VERDICT: ✅ pass`) est hors périmètre de `parse_verdict` (mika#2239 D-D) : la
garde hérite de cette lacune telle quelle et ne la contourne pas — la refermer
serait un second lecteur, c'est-à-dire la faute que cette décision interdit.
Épinglé par U4(a) et par V11.

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
  ├─ run_gh ["pr","review","2236","--comment","--body","VERDICT: pass ✅\nDEPTH: …"]
  │     │
  │     ├─ (chaîne existante : allowlist → scope qa-review → gh api → destructive)
  │     ├─ validate_tool_arg_suffixes            (mika#899)
  │     ├─ validate_review_depth_present         (mika#275)
  │     └─ validate_pr_review_flag_coherence     ◀── U1 (nouveau)
  │             parse_verdict(body) ─────────────── lecteur unique (server::verdict)
  │                 ↳ hérite emphase mika#1828 + décoration mika#2239 (D2)
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

- **A1 — ÉTABLIE (lecture du code, F4).** `process_tool_calls`
  (`tool_execution/dispatch.rs`) traite les appels d'un bloc de réponse dans une
  boucle **séquentielle** et persiste via `save_tool_call` (`:298`) à l'intérieur
  de cette boucle, donc **avant** l'appel suivant. Un `--approve` et un
  `--comment` émis dans le même bloc de réponse sont donc ordonnés et le premier
  est en base quand le second traverse la garde. Seule réserve, déjà nommée
  ailleurs : la persistance est gatée par `store_tool_calls` — c'est exactement
  le coût D5, pas une incertitude nouvelle.
- **A2 — ÉTABLIE, et le signal porteur n'est pas celui que l'assomption
  supposait (F4).** Elle craignait que l'heuristique `Exit code:` soit propre aux
  exec handlers ; la lecture du code montre l'inverse, et c'est ce qui rend
  l'échappatoire lisible :
  - `spawn_and_collect` — le chemin de `run_gh` — rend
    **`ToolOutput::success(…)` même sur sortie non-zéro**, avec un contenu
    préfixé `Exit code: {code}\n` (`builtin_handlers.rs:793-817`). Le fait est
    attesté ailleurs dans le fichier, dans le doc-comment de `GitResult` :
    *« unlike `spawn_and_collect` which always returns `is_error: false` »*.
  - Le calcul qui en dérive est **universel**, pas propre aux exec handlers :
    `dispatch.rs:336-337` pose `non_zero_exit = !output.is_error &&
    has_non_zero_exit_prefix(&output.content)` puis `success = !output.is_error
    && !non_zero_exit`, pour *tout* outil. `has_non_zero_exit_prefix`
    (`tool_execution/types.rs:22`) reconnaît `Exit code: <chiffre non nul>` et
    `Killed by signal:`.

  Donc un `gh pr review --approve` refusé par GitHub s'écrit
  `success = false`, `non_zero_exit = true`, `is_error = false` — le préfixe est
  posé par `spawn_and_collect` lui-même. **Le prédicat retenu est
  `!row.success`**, qui couvre trois populations, toutes trois « l'agent a tenté
  et la voie était fermée » :

  | population | `is_error` | `non_zero_exit` | `success` |
  |---|---|---|---|
  | GitHub refuse (`gh` sort non-zéro) — le cas visé | `false` | `true` | `false` |
  | `gh` non installé / spawn impossible | `true` | `false` | `false` |
  | refus d'une garde amont (scope qa-review, allowlist…) | `true` | `false` | `false` |

  **Un cas bénin, nommé plutôt que découvert :** `duplicate_pr_review` tombe dans
  la troisième ligne, donc ouvrirait l'échappatoire alors que l'agent a déjà
  posté. Sans conséquence — le dedup de session (`run_gh`, garde
  `pr_reviews_posted`) refuse de la même façon le `--comment` qui suivrait. Ne
  pas resserrer le prédicat sur `non_zero_exit` seul pour ce cas : ce serait
  fermer l'échappatoire sur les deux populations légitimes d'`is_error = true`,
  c'est-à-dire recréer la boucle que D5 existe pour empêcher.
- **A4 — ÉTABLIE pour son premier terme.** `server::deadline_verdict` poste via
  `run_gh_subprocess`, hors du tool `run_gh`, donc hors de la garde. Le mapping
  y est cohérent (`hold[review]` en `--comment`), donc aucun refus n'aurait lieu
  même s'il traversait. Reste à reconfirmer à l'implémentation qu'aucun **autre**
  écrivain de `gh pr review` n'existe hors de ce chemin.
- **A3** — `server::verdict::{parse_verdict, Verdict}` sont `pub(crate)`, donc
  atteignables depuis `skills::builtin_handlers`. Vérifié (`verdict.rs:37,235`).
**Aucune assomption ne porte plus l'échappatoire.** A1 et A2 étaient les deux
seuls points où le mécanisme central (D4) reposait sur une lecture non vérifiée ;
elles sont établies ci-dessus par lecture du code, et V11/V12 les tiennent en
régression. Le risque correspondant est retiré de la section Risques.

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
fail-open ; l'échappatoire ouverte et fermée ; l'appariement URL ↔ numéro nu.

**Cas de corps hérités (D2, F2b)** — un sous-groupe explicite, parce que c'est
là que la garde peut fail-open sur la population même du ticket :

| corps | `Verdict` attendu | flag exigé |
|---|---|---|
| `VERDICT: pass ✅` — **forme littérale de l'incident #2236** | `Pass` | `--approve` |
| `**VERDICT: pass**` (emphase, mika#1828) | `Pass` | `--approve` |
| `**VERDICT: pass ✅**` (cumul emphase + décoration) | `Pass` | `--approve` |
| `VERDICT: block[ac] ❌` | `Block("ac")` | `--comment` |
| `VERDICT: hold[review] ⏸️` | `Hold("review")` | `--comment` |
| `VERDICT: approved ✅` (alias, mika#1828) | `Pass` | `--approve` |
| `VERDICT: pass — but see findings` (borne mika#1821) | `Missing` | aucun, fail-open |
| `VERDICT: frobnicate ✅` (jeton inconnu décoré) | `Missing` | aucun, fail-open |
| `VERDICT: ✅ pass` (décoration de TÊTE, hors périmètre #2239 D-D) | `Missing` | aucun, fail-open |

Les trois dernières lignes sont des **contrôles négatifs**, pas des lacunes
tolérées par inadvertance : elles épinglent que la garde hérite des bornes de
`parse_verdict` à l'identique. La dernière en particulier est une **décision** —
si un jour la décoration de tête entre dans le périmètre de `parse_verdict`, ce
test rougit et la garde suit d'elle-même ; si quelqu'un la traite dans la garde
plutôt que dans le lecteur, il crée le second lecteur que D2 interdit.

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
`--comment`.

**Second scénario, `memory_vs_skill_no_verdict_degradation` — l'assertion que D6
n'avait pas (F5).** Même seed défensif, même PR mergeable, mais la consigne
présente explicitement l'échappatoire (« si `--approve` échoue, tu peux dégrader
en citant l'échec »). L'assertion est que la réponse **n'abaisse pas son propre
verdict** : elle ne nomme ni `hold[review]` ni `block[` sur une PR dont tous les
AC sont satisfaits. C'est le contournement de D6 mis sous assertion au seul
endroit où il est observable — le contournement vit dans le *choix du verdict*,
que ni la garde U1 (qui ne peut pas arbitrer la justesse d'un verdict, D6) ni un
test unitaire ne peuvent atteindre. Même honnêteté que ci-dessous sur sa portée :
c'est un gate de swap de modèle, pas un filet continu, et il reste un proxy
textuel. Mais il convertit « nommé et surveillé par une halte » en « nommé,
surveillé, **et asserté quelque part** ».

**Honnêteté sur ce que ça garde** : les scénarios de calibration
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

## Fire-Disposition

*Quatre livrables de ce plan sont de classe détecteur au sens du gate mika#1574 :
la garde U1 (`validate_pr_review_flag_coherence`), le WARN d'attribution U2
(`verdict_pass_without_approval`), le scan de source d'écrivain unique (V8), et
le test d'épinglage de la clause de prompt (U3/V9). La question du gate est :
**que fait l'implémentation quand le détecteur tire sur des données
existantes ?** Chacun est traité, et la réponse n'est pas la même.*

**U1 — (a) exception nommée, ensemble VIDE et destiné à le rester.** La garde est
posée **avant le sous-processus**, donc sa population est strictement le trafic
futur : elle ne peut, par construction, pas tirer sur une revue déjà postée. Il
n'existe donc aucun backlog à mettre en allowlist, et aucune exception n'est
livrée. **Le point à ne pas confondre, et c'est celui qui compte ici :** U1 *va*
refuser sur du trafic nominal dès le déploiement, si la mémoire de mika-qa est
encore défensive. Ce n'est **pas** un « tir sur données pré-existantes » au sens
du gate — c'est le comportement nominal du fix, et c'est la mesure que le plan
attend (voir la sonde : `pr_review_flag_refused` non vide *est* le résultat).
Traiter ces refus comme un backlog à allowlister reviendrait à désarmer le
correctif le jour de sa livraison. **Règle de résolution quand la tentation
revient :** si un refus paraît indu, la réponse est l'échappatoire D4 (tenter
`--approve`), jamais une exception.

**U2 — (c) halte-et-surface, et c'est déjà écrit comme tel.** Régime attendu
zéro ; toute occurrence est un événement d'attribution dont la résolution *est*
la décision de périmètre — quel chemin a posté (autre agent, `run_gh_subprocess`,
binaire antérieur), les trois remèdes diffèrent et aucun n'est décidable
d'avance. C'est littéralement la Halte 1 de la sonde, qui interdit d'élargir U1
par réflexe. **Pas de rattrapage rétroactif, et la raison est écrite :** les
revues historiques `pass`-sous-`commented` ne sont pas atteignables par ce
détecteur (il lit un webhook au vol, pas un historique) ; les reconstruire
demanderait un balayage de l'API GitHub sur les revues passées, c'est-à-dire un
second lecteur d'un fait que le moteur ne conserve pas — hors périmètre, et sans
valeur puisque la population d'avant U1 n'est plus actionnable. `audit_events` ne
porte aujourd'hui **aucune** ligne `pr_review_flag_guard` ni
`verdict_pass_without_approval` : les deux noms sont neufs, vérifiable par
`grep -rn verdict_pass_without_approval crates/` (zéro site avant ce plan). Le
suivi de la population pré-U1 est donc **explicitement abandonné**, pas oublié.

**V8, scan de source d'écrivain unique — (a) allowlist vide, mesurée.** Livré
avec un ensemble d'exceptions vide, sur le modèle de
`ACTOR_READING_PREDICATES_ALLOWED` (mika#2323), et la mesure est faite : à ce
jour `grep -rn verdict_pass_without_approval crates/` rend zéro site, donc le
scan naît sans violation pré-existante. **Résolution quand il tire : retirer le
second écrivain, jamais ajouter une entrée** — un détecteur d'écrivain unique
dont l'allowlist grossit ne détecte plus rien, et les deux populations que
mika#2239 et ce plan prennent soin de séparer redeviendraient indistinguables.

**U3/V9, épinglage de la clause de prompt — (a) sans objet, et c'est constaté.**
Le test asserte la présence d'une clause que ce plan ajoute : il ne peut pas
tirer sur de l'existant, puisque l'existant est précisément ce qu'il introduit.
Livré armé, sans exception.

**Aucun détecteur de ce plan n'atterrit sous (b) « land disabled ».** Le dire
explicitement : la seule chose qui *ressemble* à un atterrissage désarmé est
l'inertie d'U1 sous `MIKA_STORE_TOOL_CALLS=false` (coût D5) — mais ce n'est pas
un choix de disposition, c'est une dépendance nommée, rendue visible par le grep
d'abstention et surveillée par la Halte 2.

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
| V8 | U2 a un seul écrivain ; allowlist du scan livrée vide | scan de source |
| V9 | La clause de priorité est rendue sur les deux chemins | unité `prompt` |
| V10 | `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt` | CI |
| **V11** | **Un corps `VERDICT: pass ✅` exige `--approve`** — la forme littérale de l'incident #2236 n'est pas fail-open (D2, F2b) | unité, table U4(a) |
| **V12** | **Un `run_gh` dont le `gh` sort non-zéro s'écrit `success = false`** sur `ToolCallRow`, donc l'échappatoire D4 est lisible sur le chemin builtin (A2, F4) | unité sur `has_non_zero_exit_prefix` + eval (b) bout-en-bout |
| **V13** | Deux `run_gh` d'un même bloc de réponse sont ordonnés et le premier est persisté avant que le second ne traverse la garde (A1, F4) | eval (b) |

V11 à V13 sont la promotion demandée par F4 et F2b : les deux lectures dont
dépendait le mécanisme central ne sont plus des assomptions à reconfirmer à
l'implémentation mais des lignes du contrat, donc des régressions détectables le
jour où l'une des deux change en amont.

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

- U1 à U6 livrés ; V1–V13 verts.
- Aucune méthode `Database` nouvelle, aucun champ de `ToolContext` nouveau,
  aucune migration.
- Les refus hors périmètre (a) et (d) sont écrits dans le plan **et** dans
  l'entrée `docs/solutions/`, avec leur raison et la mesure qui les rouvrirait.
- Corps de PR nommant : l'inventaire des trois nouveaux événements, le coût D5
  (`MIKA_STORE_TOOL_CALLS`), le contournement D6 laissé ouvert, et la sonde.
- **La Halte 3 a un propriétaire nommé et une échéance (F5).** Le contournement
  D6 est le seul risque de ce plan que la structure ne ferme pas, donc le seul
  dont la surveillance repose sur quelqu'un plutôt que sur un test continu. Le
  corps de PR porte la ligne de relève : *« Halte 3 (dégradation du verdict au
  lieu du flag) — relue par l'orchestrateur à J+14, sur la distribution des
  verdicts `hold[review]` postés par `mika-platform-qa` depuis le déploiement ;
  si la part de `hold[review]` monte sans que les PR concernées aient de
  findings, c'est le contournement et le ticket de suivi (d) s'ouvre avec ce
  compte. »* La revue est un geste d'opérateur : rien dans le moteur ne peut la
  déclencher, puisque la garde ne sait pas quel verdict est juste — c'est
  précisément pourquoi elle est inscrite ici plutôt que laissée à une sonde.
  L'assertion de calibration `memory_vs_skill_no_verdict_degradation` (U4d) en
  est la moitié automatisable ; elle ne tourne qu'au swap de modèle et ne
  remplace pas la relève.

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
  la garde ne peut pas arbitrer la justesse d'un verdict. **Surveillé par** la
  Halte 3, **asserté par** le scénario de calibration
  `memory_vs_skill_no_verdict_degradation` (U4d), **relevé par** l'orchestrateur
  à J+14 (DoD). C'est le seul risque du plan dont la surveillance repose sur un
  geste humain, et il est nommé à ces trois endroits pour cette raison.
- **Inertie sous `MIKA_STORE_TOOL_CALLS=false` (D5).** Nommée, rendue visible
  par le grep d'abstention, non corrigée — la corriger demanderait un champ de
  `ToolContext` que ce ticket ne justifie pas seul.
- **~~A1/A2 non encore établies~~ — retiré.** Les deux lectures dont dépendait
  l'échappatoire sont établies par lecture du code (voir Assumptions) et tenues
  en régression par V12/V13. Le risque résiduel n'est plus « le prédicat lit
  peut-être le mauvais champ » mais « le champ change en amont », ce que les deux
  vérifications font rougir.

## Sources

- Ticket : `senara-solutions/mika#2237` (corps + trois commentaires).
- `crates/mika-agent/src/server/verdict_handler.rs:181-184` (le `Passthrough`
  muet) et `:1743-1760` (son miroir nommé, mika#2239 — `warn!` à `:1752`,
  introduit par `8f3783f2`, PR #2241, 2026-09-08 ; re-vérifié contre l'arbre).
- `crates/mika-agent/src/server/verdict.rs:37,235` (`Verdict`, `parse_verdict`),
  `:214-220` (`strip_trailing_decoration`, mika#2239), `:652`
  (`parse_verdict_field_shape_pr2236` — le corps mesuré de l'incident fondateur,
  `VERDICT: pass ✅`).
- `crates/mika-agent/src/skills/builtin_handlers.rs:793-817` (`spawn_and_collect`
  rend `success` avec préfixe `Exit code:` sur sortie non-zéro) et `:826-831`
  (doc-comment de `GitResult`, qui l'atteste en creux).
- `crates/mika-agent/src/tool_execution/dispatch.rs:298` (persistance
  intra-boucle, A1), `:336-337` (calcul universel de `non_zero_exit`/`success`,
  A2) ; `crates/mika-agent/src/tool_execution/types.rs:22`
  (`has_non_zero_exit_prefix`).
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
- Gate : `skills/bundled/mika-arch-groom-ticket/system_prompt.md:73-90`
  (Fire-Disposition, mika#1574) ; mika#2323 (`ACTOR_READING_PREDICATES_ALLOWED`,
  le modèle d'allowlist livrée vide).

## Revision history

- **rev 2 (2026-09-19)** — révision adressant les cinq findings de la première
  passe architecte.
  - **F1 (bloquant) — ancrage re-vérifié, et le plan avait raison.** Le miroir
    `verdict_approved_but_unclassified` **existe** à
    `verdict_handler.rs:1752`, dans la plage `:1743-1760` que M1 citait ;
    introduit par `8f3783f2` (PR #2241, 2026-09-08). La divergence venait du
    **corps du ticket** mika#2239, qui le présente comme une intention
    (« Envisager… ») alors que son propre fix l'a livré. M1 porte désormais
    l'encadré d'ancrage avec le commit confirmant et la commande de
    vérification ; le cadrage d'U2 est conservé et renforcé — U2 est le
    **second** signal nommé de la paire, et son scan d'écrivain unique reprend
    la discipline que mika#2239 s'est appliquée.
  - **F2 (bloquant) — confond reconnu et traité, dans les deux moitiés.** (a) Le
    Summary porte maintenant le corps réel (`VERDICT: pass ✅`), nomme les deux
    défauts empilés sur la même revue, sépare leur ordre causal (le flag est
    choisi en amont du parsing, qui est un fait de webhook), et intègre la
    re-mesure des trois `APPROVED` ultérieurs : « zéro tentative » est re-lu
    comme vrai **de ce tour** et non de la journée. Cette re-lecture est
    répercutée en argument (iv) du scope-out (d), qu'elle renforce. (b) D2 gagne
    le corollaire « la tolérance est héritée par construction via
    `parse_verdict` », U4(a) une table de neuf cas de corps dont la forme
    littérale de l'incident et trois contrôles négatifs (dont la décoration de
    tête, hors périmètre #2239 D-D, épinglée comme décision), et le Verification
    Contract la ligne V11.
  - **F3 (bloquant) — section `## Fire-Disposition` écrite**, traitant les quatre
    livrables détecteur séparément : U1 → (a) avec ensemble vide et la
    distinction explicite entre « tir sur données pré-existantes » et « refus
    nominal post-déploiement » (les confondre désarmerait le fix le jour de sa
    livraison) ; U2 → (c) halte-et-surface, avec abandon **explicite** du
    rattrapage de la population pré-U1 et sa raison ; V8 → (a) allowlist livrée
    vide, mesurée à zéro site par grep, résolution = retirer l'écrivain ;
    U3/V9 → sans objet, constaté. Aucun livrable sous (b).
  - **F4 (affinage) — A2 promue au Verification Contract (V12) et ÉTABLIE.**
    L'inquiétude est infirmée par le code : `spawn_and_collect` rend
    `ToolOutput::success` avec préfixe `Exit code:` sur sortie non-zéro
    (`builtin_handlers.rs:793-817`, attesté en creux par le doc-comment de
    `GitResult`), et `dispatch.rs:336-337` calcule `non_zero_exit`/`success`
    **universellement**, pas seulement pour les exec handlers. Le prédicat retenu
    est `!row.success`, avec la table de ses trois populations et le cas bénin
    `duplicate_pr_review` nommé. A1 est établie de la même façon (persistance
    intra-boucle, `dispatch.rs:298`) et promue en V13. Le risque « A1/A2 non
    établies » est retiré des Risques, en le disant plutôt qu'en le supprimant.
  - **F5 (affinage) — les deux, pas l'un ou l'autre.** Assertion de calibration
    `memory_vs_skill_no_verdict_degradation` ajoutée sous U4(d), **et**
    propriétaire nommé pour la Halte 3 dans le DoD (relève orchestrateur à J+14,
    avec le critère de lecture et la ligne de corps de PR). Le risque D6 nomme
    désormais ses trois surfaces de surveillance.
  - Aucun AC affaibli ; AC1–AC9 inchangés dans leur substance. Les seuls ajouts
    au contrat sont V11–V13, qui le resserrent.
