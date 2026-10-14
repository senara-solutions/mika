# mika#2687 — un `block[pipeline]` ne se poste plus sans citer une garde en échec

> **Ticket :** senara-solutions/mika#2687
> **Vol :** 1 sur 2 (voir § *Dimensionnement par vol*)

## Pourquoi

Sur mika#2685, mika-qa a rendu deux fois `VERDICT: block[pipeline]` sur la même
tête `0a53b5f0` alors que `scripts/verify-pipeline.sh` **passait** sur cette tête
(check *Pipeline Artifacts* : success, deux fois). Aucune des deux revues ne
portait de section `PIPELINE:`. Les deux raisons énoncées — « Missing plan
document in `docs/plans/` », puis « `pipeline-exempt` present but PR contains
non-docs files » — sont des règles que **la garde ne porte pas** : `verify-pipeline.sh`
compte `docs/solutions/` comme doc, et son verdict sur cette tête était 0.

La PR a été débloquée en ajoutant un plan dans `docs/plans/`. On a satisfait la
lecture erronée ; la cause est restée.

L'auteur de mika#2685 est **`samidarko` (humain)**, pas un automate : la classe
n'est donc couverte par aucune garde existante — voir ci-dessous.

## Ce qui existe déjà, et pourquoi ça n'a pas tenu

Trois dispositifs visent cette faute. Aucun ne l'atteint.

| dispositif | ce qu'il tient | pourquoi il manque ce cas |
|---|---|---|
| `qa-review/system_prompt.md` Step 2 / 2E (mika#2172) | interdit de réimplémenter une règle de garde ; exige une section `PIPELINE:` citant chaque garde verbatim | **prompt seul.** Le prompt déployé (md5 `447f1af4…`) porte déjà l'interdiction ; le modèle a paraphrasé une règle de plan-doc que ce même prompt avait retirée |
| `qa_review_executes_repo_guards.rs::prompt_does_not_reimplement_the_plan_doc_presence_check` | que la chaîne `Missing plan document in docs/plans/` n'est **pas dans le prompt** | elle n'y est pas. Elle est sortie du **modèle**. Un scan de prompt ne voit pas ce qu'un modèle invente |
| `evidence::guards` B1 (mika#2519) | refuse `block[pipeline]` quand l'auteur de la PR est un automate | l'auteur de mika#2685 est humain. B1 s'abstient, correctement |

Le ticket le dit : « le correctif par le prompt seul ne tient pas ». Ce vol ne
retouche donc **pas** le prompt — il ajoute le contrôle structurel que le ticket
désigne comme la piste.

## Décision

### D1 — la porte : la chaîne pré-subprocess de `run_gh`

`qa-review` poste sa revue par `run_gh pr review --body …`
(`skill.toml: required_tools = [… "run_gh" …]`). La chaîne pré-subprocess de
`run_gh` (`skills/builtin_handlers.rs::run_gh`) porte déjà **quatre** maillons de
cette famille, tous construits sur le même patron : prédicat pur dans
`evidence/guards.rs`, application tenant le `ToolContext`, refus `Err(ToolOutput::error)`
avec un `remedy` qui nomme la sortie correcte.

| maillon | ticket | réseau |
|---|---|---|
| suffixe de corps requis | mika#899 | non |
| `DEPTH:` présent | mika#275 | non |
| cohérence verdict↔flag | mika#2237 | non |
| **cohérence verdict↔citation de garde** | **mika#2687** | **non** |
| cohérence CI↔verdict | mika#2455 | oui |
| cohérence verdict↔dependabot | mika#2519 / mika#2565 | oui |

Refuser **ici** satisfait le CA1 au sens littéral : la revue n'existe pas encore
sur GitHub, donc rien n'est publié comme bloquant. Une rétrogradation côté
`verdict_handler` (l'autre branche du « refuse ou rétrograde » du ticket) arriverait
après la publication : la PR porterait déjà le `changes_requested`. Cette branche
n'est donc pas retenue.

### D2 — la place dans la chaîne

**Immédiatement après mika#2237, avant mika#2455.** La chaîne est ordonnée du
plus local au plus engageant, et ce maillon est le dernier qui ne lit **ni réseau
ni base** : il ne lit que le corps que l'appelant vient d'écrire. Dépenser le
`gh pr checks` de mika#2455 sur une revue que ce maillon va refuser serait payer
pour une décision déjà prise — le raisonnement que mika#2455 porte lui-même pour
sa propre position.

Comme ses trois voisins de corps, il n'est **pas** gaté sur
`required_tool_arg_suffixes` : son sujet est le corps, pas le skill actif.

### D3 — le prédicat : population, décision, formes acceptées

**Population.** `parse_verdict(&body)` vaut `Verdict::Block(k)` avec
`k.eq_ignore_ascii_case("pipeline")`. La classe est lue **exactement comme
`verdict_handler` la lit** (casse repliée, aucun `trim`), pour la raison que
mika#2519 écrit à son propre site : la lire plus permissivement ferait refuser une
revue dont la machine d'état aval ne reconnaît pas la classe. Tout autre verdict
— `pass`, `hold[review]`, `block[ac]`, `block[ci]`, `block[security]`,
`block[dependency]` — et tout corps sans ligne `VERDICT:` passent intacts.

**Décision**, en deux termes seulement :

| état du corps | issue |
|---|---|
| aucune ligne `PIPELINE:` | `RefusedSectionAbsent` |
| une ligne `PIPELINE:`, mais aucune citation d'échec de garde | `RefusedNoFailingGuardCited` |
| une ligne `PIPELINE:` et au moins une citation d'échec | `Allowed` |

**Ce qui compte comme citation d'échec** — disjonction de trois motifs, chacun
produit soit par le format que Step 2E prescrit, soit par `verify-pipeline.sh`
lui-même sur son chemin d'échec (`scripts/verify-pipeline.sh:354,374,390-391`) :

1. `exit` suivi d'un séparateur optionnel (`=`, `:`, espaces) puis d'un entier
   **non nul** — couvre les deux formes en circulation, `→ exit 1` (Step 2E) et
   `exit=1` (la boucle de Step 2B) ;
2. `REJECT:` — préfixé `[pipeline-exempt: none] ` par le script ;
3. `Verification FAILED` — la ligne de synthèse du script.

Casse repliée, et préfixe d'emphase markdown toléré sur la ligne `PIPELINE:`
(`**PIPELINE: block (mika)**`), comme `VERDICT_RE` le fait depuis mika#1828.

`PIPELINE: not-applicable (…)` (Step 2D) tombe sous le second terme : section
présente, aucune citation d'échec → refus. C'est la bonne réponse — un
`block[pipeline]` dont la propre section dit qu'aucune garde n'était applicable
est une contradiction dans le corps.

### D4 — le sens d'erreur : fail-open, et pourquoi

L'arithmétique de mika#2455 et mika#2519, mot pour mot :

- **Faux positif du refus** (on refuse une revue légitime) : mika-qa ne peut plus
  poster un `block[pipeline]` réellement fondé, et une PR réellement cassée franchit
  la porte de merge. Coût élevé.
- **Faux négatif** (on laisse passer un `block[pipeline]` infondé) : l'état
  d'aujourd'hui subsiste, l'opérateur débloque à la main. C'est le coût déjà payé.

La disjonction à trois motifs est donc **délibérément large**, et sa borne est
nommée : un corps dont la seule mention d'échec est un « exit 1 » apparaissant dans
la prose d'une garde par ailleurs verte **passera**. Ce vol ne ferme que la classe
mesurée — pas de section du tout, ou une section qui ne cite que des exit 0 —
et c'est la classe des deux revues de mika#2685.

### D5 — interaction connue avec mika#2519 B1

Sur une PR dependabot portant un `block[pipeline]` sans section `PIPELINE:`, les
deux gardes refusent. Notre maillon étant en amont, c'est lui qui répond, et c'est
voulu : il nomme la citation manquante, qui est la cause la plus proche, et son
refus ne dépend pas d'une lecture réseau de l'auteur. B1 reste la seule garde pour
le cas où la section **est** présente et fondée mais l'auteur est un automate — un
cas que D3 laisse passer par construction.

## Dimensionnement par vol (essai mika#2687)

> **Section marquée essai.** Elle applique la règle de dimensionnement par vol
> posée en commentaire opérateur sur mika#2687 (2026-10-14). Essai borné à ce
> ticket ; son résultat — mort ou non avant le plafond, PR ouverte ou non, tours
> réels — nourrit la règle.

**La règle, et sa dérivation.** Budget d'un vol : 200 tours. ~40 d'orientation,
~40 à 60 de fin de pipeline (simplification, revue, TODOs, audit doc, compound,
PR) ⇒ ~100 tours de travail. À 7–8 lignes réelles par tour : 700–800 lignes
réelles. Au facteur estimé→réel de 1,3–1,7 mesuré sur mika#2683 et mika#2654 :
**~450 lignes estimées hors `docs/`**, borne prudente. Le seuil de mika#2636
(1 000 lignes par PR) reste en vigueur ; celui-ci s'y **ajoute** comme borne par
vol.

**Calibration sur les voisins immédiats**, parce qu'elle est ce qui a décidé le
découpage. Mesuré au `git show --stat`, hors `docs/` et `CLAUDE.md` :

| garde de la même famille | lignes réelles |
|---|---|
| mika#2519 (`4ab3a518`) — un maillon de chaîne, prédicat + Layer A + Layer B | **2 250** |
| mika#2667 (`f129afd8`) — deux portes, trois lecteurs | **880** |

Le style de ce dépôt (doc-comments portant le raisonnement, contrôles positifs et
négatifs, parité shell↔Rust) coûte structurellement plus que l'intuition le dit.
L'estimation de ce vol est à **443 lignes**, donc sous la borne — mais elle n'y
tient que parce que deux livrables en sont sortis, nommés au § suivant. Ce
découpage est le contenu de l'essai, pas un confort.

**Pourquoi deux vols et pas un.** Le vol 1 met le refus en force : c'est ce que le
ticket demande, et il est mergeable seul. Le vol 2 ajoute une **classe de refus
différente** (la preuve d'exécution, lue en base) et un **scan source** — chacun
avec son propre risque de faux positif, sa propre Fire-Disposition, et sa propre
dette de doctrine (`source_guard`). Les mêler ferait un vol dont l'estimation seule
dépasse la borne, et dont la moitié risquée bloquerait la moitié sûre.

**Ordre de commit du vol 1, et son plancher.**

1. `evidence/guards.rs` — constantes, enum, prédicat pur, lecteurs, gate.
2. `evidence/guards.rs` — tests Layer A (`mika2687_*`). **Plancher : un commit
   sans ces tests n'est pas un plancher.** Le prédicat et son contrôle négatif
   partent ensemble.
3. `scripts/canonical-tokens.tsv` — les trois jetons déclarés avec leur site.
4. `builtin_handlers.rs` — `validate_pipeline_block_grounding` + le maillon.
5. `builtin_handlers.rs` — tests de câblage.
6. `server/mod.rs` — `log_pipeline_grounding_gate_state()` au démarrage.

Plancher mergeable : commits 1–5. Le commit 6 est une ligne d'observabilité ; son
absence rendrait un gate désarmé muet, ce qui est un défaut, pas une régression de
correction.

**Vol 2 — ticket de suivi.**

`Tracked in: senara-solutions/mika#2687`

Le contrat de cette session est *content-only* (aucune écriture GitHub), donc le
vol 2 est tracké sur **ce ticket**, qui reste ouvert après le merge du vol 1.
L'opérateur peut le scinder en un ticket propre ; la ligne `Tracked in:` ci-dessus
est la forme que `scripts/check-pr-body-consistency.sh` lit, et elle devra figurer
dans le corps de la PR du vol 1.

Contenu du vol 2, estimé ~340 lignes :

- **La preuve d'exécution.** Un `block[pipeline]` dont le corps cite un échec
  plausible mais dont **aucun `run_shell` de cette session n'a exécuté de garde**.
  Lecture en base sur le patron de B3 (mika#2565,
  `find_recent_build_invocation`). Classe de refus distincte, et son faux positif
  est réel : une revue reprise sur un callback de build change de session, donc
  l'abstention doit être dite (`pipeline_grounding_abstained`) avant de pouvoir
  être armée.
- **Le scan à lecteur unique.** `classify_pipeline_block_grounding` doit avoir un
  seul lecteur de production (doctrine mika#2201/#2205), allowlist livrée vide.
  Ce scan **doit** passer par `mika_common::source_guard::ProductionScanner` —
  `cargo test -p mika-common mika2398` refuse toute recomposition à la main de la
  frontière production/test, sans exception possible.
- **L'alignement du prompt** : une ligne de Step 2C/2E nommant le refus du moteur,
  plus son test de conformité dans `qa_review_executes_repo_guards.rs`. Hors du
  vol 1 délibérément : le `remedy` du refus porte déjà l'enseignement au moment où
  il sert, et le ticket établit que le prompt seul ne tient pas.

## Périmètre du vol 1

### `crates/mika-agent/src/evidence/guards.rs`

```rust
/// Le préfixe de la ligne par laquelle une revue rapporte les gardes exécutées.
/// Jeton de fil (mika#2201) → déclaré dans scripts/canonical-tokens.tsv.
pub const PIPELINE_SECTION_LINE_PREFIX: &str = "PIPELINE:";

/// Les deux chaînes que `scripts/verify-pipeline.sh` émet sur son chemin
/// d'échec. Jetons de fil, déclarés avec leur site.
pub const PIPELINE_GUARD_REJECT_TOKEN: &str = "REJECT:";
pub const PIPELINE_GUARD_FAILED_TOKEN: &str = "Verification FAILED";

pub const PIPELINE_GROUNDING_AUDIT_TOOL: &str = "pipeline_grounding_guard";
pub const PIPELINE_GROUNDING_GATE_ENV: &str = "MIKA_PIPELINE_GROUNDING_GATE";

pub enum PipelineGroundingOutcome {
    RefusedSectionAbsent,
    RefusedNoFailingGuardCited,
    Allowed,
}

/// Le corps porte-t-il une ligne `PIPELINE:` ? (casse repliée, emphase tolérée)
pub fn pipeline_section_present(body: &str) -> bool;

/// Le corps cite-t-il une garde sortie en échec ? (disjonction D3)
pub fn body_cites_failing_guard(body: &str) -> bool;

/// La décision, pure, sur les seuls faits lus. `verdict_is_pipeline_block` est
/// passé plutôt que le `Verdict` : ce module ne dépend pas de `server::verdict`.
pub fn classify_pipeline_block_grounding(
    verdict_is_pipeline_block: bool,
    body: &str,
) -> PipelineGroundingOutcome;

pub fn pipeline_grounding_gate_is_enabled(raw: Option<&str>) -> bool;
pub fn pipeline_grounding_gate_enabled() -> bool;   // OnceLock, lu une fois
pub fn log_pipeline_grounding_gate_state();          // muet si armé
```

Polarité du gate reprise **à l'identique** de ses deux voisins : armé par défaut,
désarmé par le seul `0`/`false`/`off`/`no`, et toute valeur non reconnue laisse
**armé** avec un WARN qui nomme la valeur entre guillemets (mika#2220 — sans les
guillemets, un espace parasite est invisible).

### `crates/mika-agent/src/skills/builtin_handlers.rs`

`validate_pipeline_block_grounding(args, ctx) -> Result<(), ToolOutput>` :

1. gate désarmé → `Ok(())` ;
2. `extract_pr_review_body(args)` absent → `Ok(())` ;
3. `parse_verdict` hors population → `Ok(())` ;
4. `classify_pipeline_block_grounding` ⇒ audit sur **les trois issues**, y compris
   `Allowed` : « zéro refus » ne doit pas être indistinguable d'une garde inerte
   (classe mika#2205, contrôle positif) ;
5. sur refus : WARN `pipeline_block_refused` (champs `agent_id`, `session_id`,
   `target`, `verdict`, `reason`) + `Err(ToolOutput::error(payload))`.

Le `payload` est un JSON portant `error`, `doctrine: "mika#2687"`, `target` et un
`remedy` nommant les deux sorties correctes : (a) exécuter Step 2B pour de vrai et
poster sa section `PIPELINE:` verbatim ; (b) si aucune garde n'a rejeté, le verdict
est `pass`, `hold[review]` ou `block[ac]` selon ce que les autres étapes ont trouvé
— jamais `block[pipeline]`.

**Contrainte sur le corps du `remedy`, reprise de ses deux voisins :** aucune ligne
ne doit **commencer** par `VERDICT:`. `VERDICT_RE` est ancré sur ce préfixe de
ligne et une ligne égarée serait relue comme un verdict. Les jetons sont nommés en
clair, dans une phrase.

Insertion dans `run_gh`, entre le maillon mika#2237 et le maillon mika#2455 (D2).

### `scripts/canonical-tokens.tsv`

Trois lignes, chacune avec son site de match et sa tolérance (`exact:line-anchored`
pour `PIPELINE:`, `exact:literal` pour les deux jetons du script), plus la ligne
de commentaire que le format demande. Sans cette déclaration,
`canonical-tokens-lint` ne voit pas les jetons et la classe mika#2201 se rouvre.

### Tests

**Layer A — `evidence::guards::tests::mika2687` (prédicat pur).**

| cas | attendu |
|---|---|
| `block[pipeline]`, corps sans `PIPELINE:` — **le corps mesuré sur mika#2685, revue 1** | `RefusedSectionAbsent` |
| `block[pipeline]`, corps sans `PIPELINE:` — **revue 2** (raison `pipeline-exempt`) | `RefusedSectionAbsent` |
| `block[pipeline]` + `PIPELINE: pass (mika)` + `→ exit 0` | `RefusedNoFailingGuardCited` |
| `block[pipeline]` + `PIPELINE: not-applicable (…)` | `RefusedNoFailingGuardCited` |
| `block[pipeline]` + `PIPELINE: block (mika)` + `→ exit 1` — **contrôle positif**, le corps exemple de Step 2E | `Allowed` |
| idem avec `exit=1` (forme de la boucle Step 2B) | `Allowed` |
| idem avec `[pipeline-exempt: none] REJECT: code-only PR` seul | `Allowed` |
| idem avec `Verification FAILED: 1 missing artifact(s).` seul | `Allowed` |
| `**PIPELINE: block (mika)**` (emphase) + `exit 1` | `Allowed` |
| `pass`, `hold[review]`, `block[ac]`, `block[ci]`, `block[security]`, sans `PIPELINE:` — **hors population** | `Allowed` |
| corps sans ligne `VERDICT:` | `Allowed` |
| `exit 10`, `exit 127` | citation d'échec |
| `exit 0`, `exit=0` seuls | pas de citation |
| gate : `None`, `""`, `"0"`, `"off"`, `"NO"`, `"frobnicate"` | armé sauf les quatre formes de désarmement ; `"frobnicate"` armé + WARN |

**Layer B — `builtin_handlers::tests` (câblage).** Le prédicat ne lit ni réseau ni
base : le helper `ToolContext` existant (`ctx_with_gateway_no_token`, DB
in-memory) suffit, sans harnais `run_agent`. Quatre cas : le refus atteint
l'appelant sous forme d'`Err` ; le corps fondé passe ; un `pass` passe ; gate
désarmé ⇒ même corps, passe. Sans ce Layer, un prédicat qui décide bien resterait
compatible avec une garde que personne n'appelle.

## Fichiers

| fichier | nature |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` | constantes, enum, prédicat, lecteurs, gate, Layer A |
| `crates/mika-agent/src/skills/builtin_handlers.rs` | application + maillon de chaîne + Layer B |
| `crates/mika-agent/src/server/mod.rs` | `log_pipeline_grounding_gate_state()` au démarrage |
| `scripts/canonical-tokens.tsv` | trois jetons déclarés |
| `docs/operator/environment-variables.md` | `MIKA_PIPELINE_GROUNDING_GATE` (hors compte) |
| `crates/mika-agent/CLAUDE.md` | la garde et son STOP (hors compte) |
| `docs/solutions/best-practices/…-2026-10-14.md` | compound (hors compte) |

## Observabilité

| surface | quand | champs |
|---|---|---|
| WARN `pipeline_block_refused` | à chaque refus | `agent_id`, `session_id`, `target`, `verdict`, `reason` ∈ {`section_absent`, `no_failing_guard_cited`} |
| `audit_events` `tool_name = 'pipeline_grounding_guard'` | **chaque** décision, `allowed` incluse | `target_key` = `pr_review_target(argv)`, `after_value` = l'issue |
| INFO `pipeline_grounding_gate_disabled` | au démarrage, **seulement si désarmé** | `env` |

Sonde opérateur :

```sql
SELECT target_key, after_value, count(*)
FROM audit_events
WHERE tool_name = 'pipeline_grounding_guard'
GROUP BY 1, 2;
```

Une table **vide** veut dire que le maillon n'est pas atteint — pas qu'il ne refuse
rien. C'est la distinction que la ligne `allowed` existe pour porter.

**STOP opérateur :** `MIKA_PIPELINE_GROUNDING_GATE=0` puis redémarrage (la valeur
est lue une fois par process, comme ses deux voisins). Un gate désarmé laisse
repasser le défaut mesuré, et le dit au démarrage.

## Fire-Disposition

Ce plan livre un détecteur : la garde de refus et ses tests. Option retenue :
**(a) exception nommée en allowlist — allowlist vide par construction.**

- **Détail d'implémentation.** Le détecteur est un gate d'exécution, pas un scan de
  corpus : il n'existe aucune population de violations dans l'arbre à exempter.
  Aucune allowlist n'est livrée, et aucune ne doit être ajoutée — un corps
  exempté de la citation serait exactement le défaut mesuré, rendu légitime.
- **L'échappatoire nommée, grep-visible**, est l'interrupteur
  `MIKA_PIPELINE_GROUNDING_GATE=0` : il nomme la donnée précise (le gate entier),
  référence son ticket (mika#2687, porté dans le WARN et dans
  `docs/operator/environment-variables.md`), et n'est jamais silencieux — un gate
  désarmé l'annonce au démarrage par `pipeline_grounding_gate_disabled`.
- **Assertion auto-nettoyante.** `pipeline_grounding_gate_is_enabled` est testée
  sur ses six entrées, dont `"frobnicate"` : un désarmement par coquille sur un
  gate de sûreté serait la panne silencieuse que tout ceci ferme, et le test rougit
  si la polarité est relâchée. Le contrôle de bonne foi du détecteur est la ligne
  `Allowed` de la table d'audit : une garde devenue inerte cesse d'écrire, et la
  sonde ci-dessus le montre.
- Ce vol **n'ajoute aucun scan source**, donc aucune allowlist de fichiers. Le scan
  à lecteur unique du vol 2 en aura une, livrée vide, et devra passer par
  `mika_common::source_guard` (CA3).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` — constantes, enum, prédicat, lecteurs, gate | 170 |
| `crates/mika-agent/src/evidence/guards.rs` — tests Layer A (`mika2687_*`) | 120 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` — application + maillon | 80 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` — tests de câblage Layer B | 60 |
| `crates/mika-agent/src/server/mod.rs` — log du gate au démarrage | 3 |
| `scripts/canonical-tokens.tsv` — trois jetons + commentaire | 10 |

Total estimé : 443 lignes

**Dérivation** (§ *Dimensionnement par vol*) : 200 tours de budget − ~40
d'orientation − ~40 à 60 de fin de pipeline ⇒ ~100 tours de travail ; à 7–8 lignes
réelles par tour ⇒ 700–800 lignes réelles ; au facteur estimé→réel de 1,3–1,7 ⇒
**~450 lignes estimées**, borne prudente. 443 tient sous la borne, et n'y tient que
par la sortie de la preuve d'exécution, du scan source et du prompt vers le vol 2.
Sous le seuil de mika#2636 (1 000 lignes par PR) avec une large marge.

## Definition of Done

- [ ] `classify_pipeline_block_grounding` et ses deux lecteurs livrés dans
      `evidence/guards.rs`, avec leur doc-comment portant le sens d'erreur (D4).
- [ ] Le gate `MIKA_PIPELINE_GROUNDING_GATE` suit la polarité de ses deux voisins
      et s'annonce au démarrage quand il est désarmé.
- [ ] `validate_pipeline_block_grounding` inséré dans la chaîne `run_gh` entre
      mika#2237 et mika#2455.
- [ ] Audit écrit sur les trois issues, `Allowed` comprise.
- [ ] Layer A : les deux corps mesurés sur mika#2685 sont refusés ; le corps
      exemple de Step 2E passe.
- [ ] Layer B : le refus atteint l'appelant ; le corps fondé passe ; gate désarmé
      ⇒ passe.
- [ ] Les trois jetons déclarés dans `scripts/canonical-tokens.tsv`,
      `canonical-tokens-lint` vert.
- [ ] `cargo test -p mika-agent`, `cargo test -p mika-common`, `cargo clippy`,
      `cargo fmt --check` verts.
- [ ] `MIKA_PIPELINE_GROUNDING_GATE` documenté dans
      `docs/operator/environment-variables.md` ; la garde et son STOP dans
      `crates/mika-agent/CLAUDE.md`.
- [ ] Compound doc sous `docs/solutions/best-practices/`.
- [ ] Le corps de PR porte `Tracked in: senara-solutions/mika#2687` pour le vol 2.

## Acceptance criteria

Transcrits du corps de mika#2687.

- **AC1** — Un verdict `block[pipeline]` sans section `PIPELINE:` citant une garde
  à exit non-zéro n'est pas publié comme bloquant.
  *Tenu par* : le refus en amont du subprocess `gh pr review` (D1), donc avant
  toute publication.
- **AC2** — Test : un verdict simulé `block[pipeline]` sur une PR dont la garde
  passe est refusé ou rétrogradé ; un verdict `block[pipeline]` citant une garde en
  échec passe inchangé (contrôle positif et négatif).
  *Tenu par* : Layer A (les deux corps mesurés sur mika#2685 et un corps
  `PIPELINE: pass` + `exit 0` ⇒ refus ; le corps exemple de Step 2E ⇒ `Allowed`) et
  Layer B pour le câblage.
- **AC3** — `cargo test -p mika-common mika2398` est vert.
  *Tenu par* : non-régression. Le vol 1 n'ajoute **aucun** scan source, donc rien
  qui puisse recomposer la frontière production/test à la main. Le scan à lecteur
  unique du vol 2 devra passer par `mika_common::source_guard::ProductionScanner` —
  ce test n'admet aucune exception.

## Hors périmètre

- **La rétrogradation côté `verdict_handler`** (la seconde branche du « refuse ou
  rétrograde » du ticket) : elle agirait après publication, donc ne satisfait pas
  AC1. Non retenue, pas différée.
- **La preuve d'exécution en base**, le **scan à lecteur unique** et
  **l'alignement du prompt** : vol 2, `Tracked in: senara-solutions/mika#2687`.
- **Les autres classes de `block[…]`** : `block[ac]`, `block[ci]`,
  `block[security]` ont chacune leur propre question d'assise et ne sont pas
  touchées. Aucun motif ne suggère de généraliser avant d'avoir mesuré.
- **`scripts/verify-pipeline.sh` lui-même** : il a eu raison deux fois sur
  mika#2685. Rien à y corriger.
