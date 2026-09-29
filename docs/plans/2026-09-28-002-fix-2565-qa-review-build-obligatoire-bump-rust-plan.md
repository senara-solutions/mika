# mika#2565 — Un bump Rust se vérifie en compilant, jamais en affirmant

**Ticket :** senara-solutions/mika#2565
**Type :** fix (loop-substrate, p1)
**Branche :** `fix/2565/qa-review-pipeline-exempt-dispense-du`

---

## 1. Le défaut, mesuré (2026-09-28)

Deux PR dependabot approuvées par mika-qa avec un check requis rouge derrière.

**mika#2560** (`sha2 0.10.9 → 0.11.0`) — trois `pass` (07:33Z, 07:58Z, 08:27Z), dont
`REASON: … API-compatible (Sha256/Digest trait stable) … BUILD VERIFICATION: skipped
(pipeline-exempt label)`. `Docker Build` agent et gateway rouges. L'affirmation était
fausse : `finalize()` a perdu son `LowerHex` (les call sites passent à `hex::encode`) et
`hmac` a dû suivre en 0.13 (`KeyInit` ré-exporté séparément). La PR n'a été mergée
qu'après qu'un humain eut poussé ces adaptations.

**mika#2561** (`utoipa 5.5.0 → 6.0.0`) — le verdict **s'inverse** au fil des tours :
`hold[review]` à 07:25Z et 07:35Z (« build verification recommended … heavy utoipa
usage »), puis `pass` à 08:02Z, 08:25Z, 10:52Z, 11:40Z. `Check` rouge (6 min 21 s).

La porte de merge a tenu. Le défaut est l'**approbation** : elle porte une affirmation
de compatibilité non vérifiée, et elle s'affiche `APPROVED` pour l'humain qui merge.

---

## 2. Ce que la lecture du code établit — trois rectifications, et c'est le premier livrable

Les trois remèdes proposés par le ticket portent sur un substrat qui n'est pas celui qui
a produit le défaut. Appliqués à la lettre, deux sur trois seraient **inertes**.

### R1 — `pipeline-exempt` n'exempte le build nulle part. Il n'y a rien à retirer.

Recherche exhaustive de `pipeline-exempt` dans `skills/bundled/qa-review/system_prompt.md` :
quatre occurrences, **lignes 197, 237, 247, 731**, toutes dans Step 2 (l'exécution des
guards de dépôt) ou dans un exemple de sortie de guard. **Aucune n'est à portée du build.**

Ce qui prescrit le skip est **Step 1.6 item 2**, littéralement :

> Skip Step 2 pipeline checks and Step 2.5 plan-AC verification. Emit
> `PLAN-AC VERIFICATION: skipped (…)` and **`BUILD VERIFICATION: skipped (Dependabot
> dependency PR)`**.

et sa clôture, item 8 : « **Do NOT run Steps 2/2.5/3e for a Dependabot PR.** »

Le `(pipeline-exempt label)` des verdicts mesurés est une **paraphrase du modèle**. La
trace le montre : la toute première revue de #2561 (07:25Z) écrit la formulation du
prompt — `BUILD VERIFICATION: skipped (no Behavioral ACs — Dependabot PR)` — et c'est
seulement aux tours suivants que le label devient la raison invoquée. Le modèle a
attribué au label une dispense que le chemin Dependabot lui accordait déjà.

**Conséquence de plan : le remède 1 du ticket vise un mécanisme inexistant.** La cible
est Step 1.6, pas la lecture du label.

### R2 — La garde `API-SURFACE:` a tourné, a été satisfaite, et n'a rien empêché

mika#2519 a livré `evidence::guards::classify_dependabot_verdict`, dont la branche **B2**
refuse un `pass` sur un saut de majeure dont le corps n'affirme aucune vérification des
sites d'appel (`body_asserts_api_surface`). `utoipa 5 → 6` **est** un saut de majeure :
la garde s'appliquait.

Elle a laissé passer, et à juste titre au regard de son prédicat. Chaque `pass` de #2561
porte une ligne `API-SURFACE:` ancrée en début de ligne et non vide — la dernière
(11:40Z) fait onze lignes : « 29 `#[derive(ToSchema)]` sites (mika-agent: 27,
mika-gateway: 2) … 14 `#[utoipa::path(...)]` call sites … 0 `#[into_params]` usages … ».

**Ce travail n'est pas fabriqué. Il est réel, sourcé par grep, et faux sur la
conclusion** — la casse d'utoipa 6 n'est pas dans la syntaxe des macros. Le modèle a fait
exactement ce que le prompt demande, avec soin, et ça n'a pas suffi.

**Conséquence de plan : le remède 3 du ticket (« aucune phrase *API-compatible* sans
preuve citée ») est déjà en vigueur et déjà satisfait.** Durcir l'exigence de citation ne
ferme rien. *Une lecture d'API, même honnête et détaillée, n'est pas une compilation.*

### R3 — #2560 est hors de la garde par construction, et rien de ce qui existe ne l'attrapait

`version_major("0.10.9")` et `version_major("0.11.0")` rendent tous deux `Some(0)` →
`VersionBump::NoMajorJump` → B2 ne s'applique pas. Vérifiable dans la trace : les trois
`pass` de #2560 ne portent **aucune** ligne `API-SURFACE:`.

La règle `0.x` de semver est délibérément non appliquée (doc-comment de
`classify_version_bump` : l'élargir ferait entrer `0.22 → 0.23` et refuserait le rail que
mika#2519 existe pour ouvrir). Cette décision est **maintenue ici**, et pour une raison
neuve : élargir B2 aux `0.x` n'aurait pas attrapé #2560 non plus. Le défaut y est une
incompatibilité **transverse** — `sha2 0.11` sur `digest 0.11` contre `hmac 0.12` sur
`digest 0.10` — qu'aucune lecture des call sites de `sha2` ne montre. Seul le résolveur
de cargo la voit.

**C'est l'argument décisif du ticket, et il est plus fort que ce que le ticket en dit :
le seul signal qui attrape les deux cas est la compilation.** Le remède 1/2 est juste ;
sa justification ne l'était pas.

---

## 3. Le remède — le build, et pas une meilleure assertion

**Une PR dont le diff touche `Cargo.toml` ou `Cargo.lock` compile avant tout verdict.
À défaut de build réussi, le verdict ne peut pas être `pass`.**

Deux moitiés, et comme toujours c'est la structurelle qui tient
(`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`) :

| moitié | site | ce qu'elle fait |
|---|---|---|
| **intention** | `qa-review/system_prompt.md` Step 1.6 + `qa-review-build-callback` | prescrit le build et sa reprise |
| **structure** | `evidence::guards` branche **B3** + son appelant | refuse le `pass` quand **le moteur n'a pas vu de build** |

La moitié structurelle ne lit pas une phrase du corps. C'est la leçon de R2 : B2 exige une
**assertion** et le modèle en produit une, sincère et insuffisante. B3 exige un **fait
moteur** — un appel `build_mika` réellement enregistré dans `tool_calls` pour cette
session. Le précédent maison est `find_recent_destructive_actions` (mika#1646), qui lit
la même table pour la même raison : *un fait qu'on lit en base ne se rédige pas.*

---

## 4. Quatre obstacles structurels, et c'est le vrai contenu du travail

### O1 — Le chemin de build existant est inatteignable depuis une PR dependabot

Step 3e.2 dérive `~/workspace/mika-platform/.claude/worktrees/<branche-assainie>/mika/`
— un worktree de **dispatch**. Une PR dependabot n'est pas dispatchée par la boucle et
n'en a jamais. Le prompt conclut alors « BUILD VERIFICATION: skipped (no worktree found
at expected path) ». Brancher Step 3e sans plus serait donc **inerte** : un skip de plus
sous un autre motif.

**Le worktree doit être créé.** Le mécanisme existe déjà à un pas de là — Step 2B crée un
worktree jetable pour les guards (`git worktree add`, `run_shell`) — mais il est détruit
par un `trap EXIT`, alors que `build_mika` est long-running et a besoin que le répertoire
**survive au tour**.

### O2 — Le worktree doit être MANAGÉ, sinon il devient un orphelin de 40 Go

Le chemin canonique de Step 3e.2 est sous `.claude/worktrees/` : c'est exactement le
terme **T1** du faucheur mika#2420, et la population de la purge de `target/` mika#2497.
Un worktree créé là est retiré quand la PR est fermée/mergée + grâce, et son `target/`
purgé après inactivité. Un worktree créé ailleurs ne l'est **jamais**.

**Créé attaché à la branche** (`git worktree add <path> <branche>`), pas `--detach` : le
faucheur résout alors par la clé branche (T3), le chemin le plus direct. mika#2518 couvre
aussi le détaché par appariement SHA, mais s'appuyer là-dessus serait dépendre d'un
rattrapage quand la voie nominale est disponible.

**Coût nommé, et il est réel :** un `cargo build --release --features telemetry` sur un
`target/` vide pèse 15 à 50 Go (mesure du CLAUDE.md racine) et plusieurs minutes. À
4–8 PR dependabot par semaine, c'est borné par les deux faucheurs — mais c'est un coût
que ce travail crée, et il faut le dire plutôt que le découvrir. Le budget de temps, lui,
n'est pas un obstacle : `build_mika` est long-running (`estimated * 3`, plancher 600 s),
pas soumis aux 300 s du `skill.toml` ni aux 30 s de `run_shell`.

`run_shell` admet le geste : la garde mika#2449 refuse `checkout` / `stash` / `reset` /
merge non-`--ff-only` sur le checkout partagé et **admet explicitement `worktree *`**.

### O3 — Le callback de build est plan-centré, et produirait le verdict que mika#2519 interdit

`qa-review-build-callback` ouvre sur « **Mandatory plan re-read** … If the plan file is
unreadable: `VERDICT: block[pipeline]` … **NOT** downgraded to `hold[review]` ».

Sur une PR dependabot il n'y a **pas de plan**. Le callback rendrait donc
`block[pipeline]` — que la branche **B1** de mika#2519 refuse structurellement sur un
auteur automatisé (« Step 1.6 rend ce verdict structurellement inatteignable sur cette
classe »). Un couplage vicieux : le callback produirait le verdict que la garde interdit,
et la revue mourrait sur un refus d'outil.

**Le callback a besoin d'une branche dependabot explicite**, placée **avant** la
relecture du plan.

### O4 — La garde ne doit pas reproduire la faiblesse qu'elle corrige

Une B3 qui chercherait une ligne `BUILD-VERIFIED:` dans le corps serait B2 sous un autre
nom : une assertion que le modèle apprend à produire. D'où la lecture en base.

**Limite nommée :** B3 atteste que le build **a eu lieu**, pas que son résultat était
vert. Un modèle qui lancerait le build, le verrait rouge et posterait `pass` quand même
franchit B3. Cette population relève d'une autre famille (`assert_grounded`), elle est
**mesurable a posteriori** (le `tasks.result` du callback porte `Build FAILED`), et la
fermer demanderait de joindre la tâche de callback au verdict — un travail de plus, avec
sa propre population à mesurer. Ce qui est fermé ici est le **saut** du build, qui est le
défaut mesuré des deux PR témoins.

---

## 5. Design

### 5.1 Le prédicat de population — `is_cargo_dependency_pr`

Pure, dans `evidence::guards`, à côté de `classify_version_bump` :

```rust
/// Le diff de la PR touche-t-il la résolution de dépendances Rust ?
///
/// Le discriminant est le FICHIER, jamais l'écosystème deviné depuis le titre :
/// un bump npm ou `actions/*` ne se ferme pas par `build_mika`, et lui demander
/// une compilation Rust serait un refus sans remède.
pub fn is_cargo_dependency_pr(files: &[String]) -> bool
```

Vrai dès qu'un chemin a pour nom de base `Cargo.toml` ou `Cargo.lock`, à n'importe quelle
profondeur (workspace racine et crates membres). Comparaison sur le **segment final**, pas
`contains` : un fichier `docs/Cargo.toml-migration.md` ne doit pas apparier.

### 5.2 Comment la garde apprend les fichiers

Le lecteur du gate est déjà `gh pr view --json author,title`. Un champ de plus sur le
**même** appel — `files` — coûte zéro round-trip. Le parse suit celui d'`author.login` :
`.files[].path`.

**Fail-open sur `files` illisible ou absent**, avec un motif d'abstention dédié
`DependabotAbstention::NO_FILES`. Cohérent avec la doctrine du gate mika#2519 (« le
verdict est laissé passer plutôt que refusé sur un terme inobservable »). Le coût est réel
— c'est le défaut qui repasse — et c'est pour ça que l'abstention porte un **nom à elle**
plutôt que de se replier sur `unparseable` : elle doit être comptable séparément (sonde
S4).

### 5.3 Le fait moteur — `find_recent_build_invocation`

Sœur de `find_recent_destructive_actions` (mika#1646), dans `db.rs` + son wrapper
`async_db.rs` :

```rust
pub fn find_recent_build_invocation(
    &self,
    agent_id: &str,
    session_id: &str,
    window_secs: i64,
) -> Result<Vec<ToolCallRow>>
```

`SELECT … FROM tool_calls WHERE agent_id = ?1 AND session_id = ?2 AND tool_name = ?3 AND
created_at >= ?4`. Le nom d'outil est la constante du registre (`build_mika`), jamais un
littéral recopié.

**Scopé à la session**, pas seulement à l'agent : un build lancé pour une *autre* PR il y
a dix minutes ne vaut rien ici. Fenêtre par variable d'environnement
`MIKA_QA_BUILD_EVIDENCE_WINDOW_SECS`, défaut **7200** (2 h) — trois paliers maison
(absent/vide → défaut ; illisible, `0` ou négatif → défaut + WARN). Le défaut est large
parce que le coût des deux erreurs n'est pas le même : une fenêtre trop courte refuse un
build réel et casse une revue légitime ; une fenêtre trop longue laisse passer un build
de la même session, ce qui reste un fait moteur.

**Inertie nommée :** si `MIKA_STORE_TOOL_CALLS` est désarmé, la table est vide et B3
refuserait tout `pass`. C'est le mode de panne inverse du défaut, et il couche la revue.
B3 **abstient** dans ce cas, sous le motif `build_evidence_unavailable`, avec un WARN qui
le nomme — même arbitrage que `stuck_pending_activity_not_recorded` (mika#2184), et même
conduite pour l'opérateur : vérifier le réglage, pas le prédicat.

### 5.4 La branche B3

Nouveau variant, en queue de `DependabotVerdictOutcome` — **ajout, jamais renommage** :
les deux populations existantes gardent leur nom et les `GROUP BY` publiés restent exacts.

```rust
/// **B3** — un `pass` sur une PR dependabot dont le diff touche la résolution
/// de dépendances Rust, sans qu'aucun build n'ait tourné dans cette session.
RefusedUnbuiltCargoBump { author: String, files_seen: usize },
```

Ordre des branches : B1, puis **B3**, puis B2. B3 avant B2 parce qu'un bump majeur Rust
satisfait les deux prédicats et que le refus le plus informatif est celui qui nomme le
build — R2 établit que l'`API-SURFACE:` de #2561 était présente et insuffisante, donc
refuser d'abord sur l'assertion enverrait le modèle réécrire la ligne qui n'était déjà pas
le problème.

`classify_dependabot_verdict` reçoit deux arguments de plus (`files: &[String]`,
`build_observed: bool`). La fonction reste **pure** : l'appelant fait la lecture en base
et lui passe le booléen, exactement comme il lui passe déjà `verdict_is_pass`.

Le corps du refus nomme les **deux** sorties correctes, sur le modèle de B2 : (a) lancer
`build_mika` sur le worktree et reprendre au callback ; (b) si la compilation casse,
`block[dependency]` posté en `--comment`. Et il dit pourquoi une lecture d'API ne suffit
pas ici, en citant #2560 : *une incompatibilité transverse entre `sha2 0.11` et
`hmac 0.12` ne se voit sur aucun call site.*

### 5.5 Le prompt — Step 1.6

Quatre éditions, toutes dans le même bloc :

1. **item 2** cesse de prescrire un skip inconditionnel. Nouvelle formulation
   conditionnelle : le diff touche `Cargo.toml`/`Cargo.lock` ⇒ le build est **requis**
   (renvoi au nouveau 5c) ; sinon `BUILD VERIFICATION: skipped (dependency PR, no Rust
   dependency resolution in the diff)` — une raison **vraie**, et qui n'invoque plus le
   label.
2. **nouveau 5c — « Compile it »** : dérivation du chemin managé (formule de 3e.2,
   composée d'une racine **littérale**, jamais d'un `$VAR` — mika#2536), création du
   worktree **attaché à la branche** s'il n'existe pas, `build_mika(cwd=<worktree>)`,
   **fin de tour**. Échec de création du worktree ⇒ `hold[review]` nommant l'erreur,
   jamais `pass` (doctrine 2C : *un guard qu'on n'a pas pu lancer n'est pas un guard qui
   est passé*).
3. **item 8** gagne un terme : sur une PR dependabot Rust, `pass` exige
   `BUILD VERIFICATION: Build: pass`. Build rouge ⇒ `block[dependency]` en nommant
   l'erreur de compilation.
4. **la clôture** devient « Do NOT run Steps 2/2.5 for a Dependabot PR » — Step 3e cesse
   d'y être interdit.

La butée est **topique, jamais énumérative** : le prompt ne liste pas les phrases
interdites (« API-compatible », « API-surface verified »), il exige la compilation. Lister
les formulations fournirait au modèle le gabarit qu'on veut lui retirer (doctrine
mika#2292), et R2 montre que la phrase n'était pas le problème.

### 5.6 Le prompt — `qa-review-build-callback`

Une branche, **avant** l'item 1 (« Mandatory plan re-read ») :

> **Dependabot PR — no plan to re-read.** Si le tour d'ouverture a suivi Step 1.6 (auteur
> automatisé, aucun `> - **Plan:**` callout), ne cherche aucun plan : reprends directement
> au verdict de Step 1.6 item 8 avec le résultat du build, et émets
> `PLAN-AC VERIFICATION: skipped (Dependabot dependency PR — no plan contract, mika#1729)`.
> **N'émets jamais `block[pipeline]` sur cette classe** — il est structurellement
> inatteignable (mika#2519 B1) et l'outil de publication le refuse.

Discriminant : **l'absence de plan-callout**, qui est déjà le discriminant naturel du
callback (l'item 1 dérive `<plan-path>` de ce callout). Pas le préfixe de branche
`dependabot/`, qui serait une heuristique de nommage là où un fait est disponible.

---

## 6. Unités d'implémentation

| # | unité | fichiers |
|---|---|---|
| **U1** | `is_cargo_dependency_pr` + variant B3 + `classify_dependabot_verdict` étendu | `crates/mika-agent/src/evidence/guards.rs` |
| **U2** | `find_recent_build_invocation` + wrapper async + fenêtre paramétrée | `crates/mika-agent/src/db.rs`, `async_db.rs`, `evidence/guards.rs` |
| **U3** | Câblage : `--json …,files`, parse, lecture en base, refus B3, abstentions | `crates/mika-agent/src/skills/builtin_handlers.rs` |
| **U4** | Step 1.6 — quatre éditions (§ 5.5) | `skills/bundled/qa-review/system_prompt.md` |
| **U5** | Branche dependabot du callback (§ 5.6) | `skills/bundled/qa-review-build-callback/system_prompt.md` |
| **U6** | Scénario de calibration `dependabot_cargo_bump_requires_build` (AC4) | `crates/mika-agent/src/calibration/roles/mika_qa.rs` |
| **U7** | Tests Layer A (prédicats) et Layer B (câblage), scan de source | `evidence/guards.rs` tests, `tests/eval/test_dependabot_build_evidence_2565.rs`, `tests/qa_review_executes_repo_guards.rs` |
| **U8** | Jetons de fil + doc | `scripts/canonical-tokens.tsv`, `CLAUDE.md` |

`U1`+`U2` sont indépendantes ; `U3` en dépend ; `U4`/`U5`/`U6` sont indépendantes de tout.

---

## Fire-Disposition

Ce plan livre trois détecteurs : la branche **B3** (garde de verdict), le **scan de
source** d'écrivain unique du nom d'audit, et le scan de **co-localisation** de la lecture
de fenêtre. Disposition retenue : **(a) exception nommée en allowlist — livrée VIDE**,
plus une assertion auto-nettoyante.

**Aucune population existante à exempter, et c'est établi plutôt que supposé :**

- **B3 est une garde de verdict** : elle ne s'applique qu'aux verdicts **futurs**. Il n'y
  a pas de corpus historique à passer en revue. Les verdicts de #2560/#2561 sont postés et
  rien ici ne les réécrit (§ 10).
- **Le scan d'écrivain unique** porte sur un nom d'audit **créé par ce travail** : sa
  population est vide par construction, et son allowlist (`B3_AUDIT_NAME_ALLOWED`) est
  livrée vide, pinnée vide par un test frère — doctrine mika#2201 : *quand il tire, on
  retire l'écriture, on n'ajoute pas de ligne.*
- **Anti-vacuité obligatoire** : chaque scan échoue si son nom n'est écrit **nulle part**
  dans le fichier visé. Un scan qui vise un nom mort ne vérifie rien et se lit exactement
  comme un arbre propre (classe mika#2205, et le motif est déjà celui de
  `mika2496_the_cost_overrun_name_has_a_single_writer`).

**Ce qui va rougir au déploiement, et qui n'est pas une violation à exempter :** toute PR
dependabot Rust re-revue après le déploiement devra compiler avant d'obtenir un `pass`.
C'est l'effet voulu. Les PR dependabot en vol au moment du déploiement verront leur
prochaine revue déclencher un build au lieu d'un `pass` immédiat — c'est le correctif qui
mord, pas une régression, et la sonde S1 le mesure.

---

## 7. Surfaces opérateur

```bash
# 1. La garde a-t-elle refusé un pass sans build ? (régime attendu : NON VIDE puis décroissant)
grep dependabot_verdict_refused "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.reason == "unbuilt_cargo_bump") | {target, author, files_seen}'

# 2. CONTRÔLE POSITIF — la garde tourne-t-elle seulement ?
grep -c dependabot_verdict_abstained "$MIKA_SPIRIT_LOG_FILE"

# 3. Les builds réellement lancés sur une PR dependabot
grep build_mika "$MIKA_SPIRIT_LOG_FILE" | jq -c 'select(.cwd | test("dependabot"))'
```

```sql
-- Les trois issues du même gate, soustractibles.
-- `tool_name` est la constante EXISTANTE `DEPENDABOT_VERDICT_AUDIT_TOOL`
-- (`evidence/guards.rs`), valeur `dependabot_verdict_guard` : ce travail
-- AJOUTE une valeur d'`after_value`, il ne crée pas un second nom d'outil.
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'dependabot_verdict_guard' GROUP BY 1 ORDER BY 2 DESC;
```

**Coût nommé, et daté :** un `GROUP BY after_value` qui enjambe le déploiement voit
apparaître une valeur qui n'existait pas avant. Les lignes antérieures ne sont **pas**
réécrites — les réécrire rendrait faux ce qu'elles ont dit quand elles ont été écrites
(motif mika#2361). Les deux populations préexistantes restent comparables à elles-mêmes.

| valeur `after_value` | régime attendu | lecture |
|---|---|---|
| `refused_unbuilt_cargo_bump` | **non vide, décroissant** | chaque ligne est une approbation non compilée arrêtée. Décroît quand le prompt prend. |
| `refused_unverified_major_bump` | inchangé | population B2, comptable séparément |
| `abstained` / `no_files` | **proche de zéro** | `gh` n'a pas rendu les fichiers ; le défaut peut repasser |
| `abstained` / `build_evidence_unavailable` | **vide** | `MIKA_STORE_TOOL_CALLS` désarmé : la garde est inerte |

Le nom `refused_unbuilt_cargo_bump` est **distinct** de `refused_unverified_major_bump`
bien que les deux mènent au même verdict : le premier dit « rien n'a compilé », le second
« rien n'a été lu ». Les remèdes diffèrent, et les confondre rendrait les deux populations
incomptables (motif `phantom_aged_out` / `phantom_sweep_spared`, mika#2156).

---

## 8. Sondes post-déploiement, et leurs cinq haltes

> **Préalable.** `skills/bundled/` est une projection du **binaire**, pas du checkout
> (mika#2340). `cat ~/.mika/skills/.manifest-writer` doit porter le sha qu'on vient de
> bâtir — sans quoi chaque sonde ci-dessous décrit le binaire d'hier.

**S1 — le rejeu du défaut fondateur (première PR dependabot Rust après déploiement).**
Le tour d'ouverture doit appeler `build_mika` et finir sans verdict ; le callback doit
poster un verdict portant `BUILD VERIFICATION: Build: pass|fail`. Aucun `pass` sans build.

*Halte 1 — la revue rend `pass` immédiatement, et la garde est muette.* **Ne pas élargir
le prédicat par réflexe** : lire d'abord `abstained`/`no_files` (sonde 2), puis établir
que le binaire servi porte le correctif (classe mika#2340).

**S2 — contrôle négatif docs-only (AC3).** Une PR dependabot `pipeline-exempt` dont le
diff ne touche ni `Cargo.toml` ni `Cargo.lock` (bump `actions/*`, npm) ne doit déclencher
**aucun** build et rester éligible au `pass`.

*Halte 2 — un build part sur une PR sans Cargo.* `is_cargo_dependency_pr` apparie trop
large (le piège est `contains` au lieu du segment final). Réparer le prédicat, pas le
prompt.

**S3 — le worktree est fauché (7 jours après la première PR mergée).** Après merge +
grâce, `SELECT target_key FROM audit_events WHERE tool_name = 'worktree_reaped'` doit
porter le worktree dependabot.

*Halte 3 — il n'est jamais fauché.* Lire le motif de refus :
`SELECT after_value, count(*) FROM audit_events WHERE tool_name = 'worktree_reap_skipped'
GROUP BY 1`. Un `detached_head` signifie que le worktree a été créé `--detach` contre la
décision du § 4/O2 ; un `outside_managed_root` que le chemin n'est pas sous
`.claude/worktrees/`. Les deux sont des orphelins de dizaines de gigaoctets et se réparent
à la création, pas au faucheur.

**S4 — l'abstention reste rare (30 jours).** `no_files` proche de zéro.

*Halte 4 — `no_files` porte du trafic nominal.* `gh pr view --json files` ne rend pas ce
qu'on croit (pagination, champ absent selon la version de `gh`). **Ne pas basculer en
fail-closed** — ce serait refuser des revues sur un terme qu'on ne sait pas lire ; réparer
la lecture.

**S5 — contrôle négatif de bruit (30 jours).** Aucun `refused_unbuilt_cargo_bump` sur une
PR **non** dependabot : le premier terme de `classify_dependabot_verdict` est l'auteur
automatisé, et une occurrence hors de cette classe signifie qu'il a été relâché.

*Halte 5 — une occurrence.* Désarmer (`MIKA_DEPENDABOT_VERDICT_GATE=0`, le levier existant
de mika#2519), puis diagnostiquer. Une garde qui refuse les revues humaines coûte plus que
le défaut qu'elle ferme.

**Halte transverse — les deux sondes muettes.** Zéro refus **et** zéro abstention ne
prouve rien : il faut qu'une PR dependabot Rust ait été revue depuis le déploiement.
Vérifier le contrôle positif (sonde 2) avant toute conclusion. *Une garde que personne n'a
exercée se lit exactement comme une garde qui marche* (mika#2205).

---

## 9. Verification Contract

**Layer A — prédicats purs** (`evidence::guards::tests`, aucun réseau, aucune base) :

- `is_cargo_dependency_pr` : vrai sur `Cargo.lock`, `Cargo.toml`, `crates/x/Cargo.toml` ;
  **faux** sur `docs/Cargo.toml-migration.md`, `package.json`,
  `.github/workflows/ci.yml`, liste vide.
- `classify_dependabot_verdict` : B3 sur (`app/dependabot`, `pass`, fichiers Cargo,
  `build_observed = false`) ; **Allowed** quand `build_observed = true` ; **Allowed** sur
  un auteur humain ; **Allowed** sur un `hold[review]`. Précédence B3 avant B2 sur un
  bump majeur Rust non compilé.
- Les motifs d'abstention sont un format de fil (test frère de
  `mika2519_les_causes_dabstention_sont_un_format_de_fil`).
- **Contrôle négatif de non-régression** : les deux cas témoins de mika#2519 (#2453
  `base64 0.22.1 → 0.23.0`) gardent leur verdict.

**Layer B — câblage** (`tests/eval/test_dependabot_build_evidence_2565.rs`, sur le modèle
de `test_dependabot_verdict_coherence_2519.rs`, `MockLlmProvider`, aucun réseau) :

- **Rejeu de #2561** : payload `gh pr view` réel (auteur objet, titre
  `Bump utoipa from 5.5.0 to 6.0.0`, fichiers `Cargo.toml`+`Cargo.lock`), corps `pass`
  **portant** une ligne `API-SURFACE:` détaillée — donc satisfaisant B2 — et aucun build
  en base ⇒ **refusé par B3**. *C'est le test qui distingue ce travail de mika#2519 : sans
  B3, ce corps passe.*
- **Rejeu de #2560** : `sha2 0.10.9 → 0.11.0`, aucune ligne `API-SURFACE:`, hors B2 par
  `NoMajorJump` ⇒ **refusé par B3**.
- **Contrôle positif** : le même corps avec un `tool_calls` `build_mika` inséré dans la
  session ⇒ **laissé passer**. Sans ce test, « B3 refuse » et « B3 refuse tout » sont
  indistinguables.
- **Abstentions** : `files` absent ⇒ laissé passer + ligne `no_files` ;
  `MIKA_STORE_TOOL_CALLS` désarmé ⇒ laissé passer + `build_evidence_unavailable`.

**Layer C — prompts** (`tests/qa_review_executes_repo_guards.rs`, scans de source) :

- Step 1.6 ne porte plus `BUILD VERIFICATION: skipped (Dependabot dependency PR)` en
  prescription inconditionnelle, et ne porte plus « Do NOT run Steps 2/2.5/**3e** ».
- Step 1.6 nomme `build_mika` et la formule de dérivation du worktree.
- Le callback porte sa branche dependabot **avant** la relecture du plan.

**Layer D — calibration** (U6) : `dependabot_cargo_bump_requires_build`, rejouant #2561
(bump majeur, label posé, aucun build) et attendant **autre chose que `pass`**. Le test
`scenario_count_is_ten` devient `scenario_count_is_eleven`.

**Non testable ici, et écrit plutôt que découvert :** que `cargo build` attrape
effectivement la casse d'utoipa 6 ou de sha2 0.11 s'exécute dans un autre processus, sur
un vrai registre de crates. Le contrat **côté mika** est *le build est lancé et son
résultat gate le verdict*, et Layer B l'atteste déterministiquement. La moitié
comportementale est la sonde S1.

**Commandes :** `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check`, `make verify-bundled-skills`, `bash scripts/check-canonical-tokens.sh`.

---

## 10. Ce que ce travail n'achète PAS

- **Il ne rattrape pas #2560 et #2561.** Les verdicts sont postés, #2560 est mergée. Rien
  ici ne réécrit un verdict passé : fabriquer une ligne décrivant une vérification qui n'a
  pas eu lieu est exactement l'inverse de ce que ce travail défend. La sonde est la
  **prochaine** PR dependabot.
- **Il ne garantit pas qu'un `pass` soit vrai.** Il garantit qu'un `pass` sur un bump Rust
  a été précédé d'une compilation. Un modèle qui compile, voit rouge et poste `pass` quand
  même franchit B3 (§ 4/O4) — autre famille, population mesurable, suivi nommé.
- **Il ne couvre pas les bumps non-Rust.** Un bump npm ou `actions/*` qui casse n'est pas
  fermé : `build_mika` ne compile pas ces écosystèmes. C'est la conséquence assumée d'un
  prédicat sur le fichier plutôt que sur l'intention.
- **Il n'élargit pas B2 aux `0.x`**, et R3 montre que l'élargir n'aurait rien attrapé.
- **Aucune ligne de journal nouvelle côté producteur** : le seul instrument est le gate
  existant, enrichi d'un motif. **Son silence ne prouve rien tant que le contrôle positif
  n'est pas établi.**

---

## 11. Definition of Done

- [ ] `is_cargo_dependency_pr` et le variant B3 livrés, `classify_dependabot_verdict`
      étendu et toujours pur.
- [ ] `find_recent_build_invocation` livré avec son wrapper async et sa fenêtre à trois
      paliers.
- [ ] Le câblage lit `files` sur le **même** appel `gh pr view`, lit la base, refuse B3, et
      abstient sous deux motifs nommés.
- [ ] Step 1.6 : les quatre éditions du § 5.5, dont la création du worktree **attaché** au
      chemin **managé**.
- [ ] Le callback porte sa branche dependabot avant la relecture du plan.
- [ ] Scénario de calibration livré ; le compte de scénarios est mis à jour.
- [ ] Layers A/B/C/D verts, **dont le contrôle positif** (un build en base laisse passer).
- [ ] Fire-Disposition : allowlists livrées vides, pinnées vides, anti-vacuité en place.
- [ ] `scripts/canonical-tokens.tsv` déclare les jetons nouveaux ;
      `check-canonical-tokens.sh` vert.
- [ ] `CLAUDE.md` racine : entrée décrivant la garde, ses surfaces, ses cinq haltes.
- [ ] `cargo test -p mika-agent`, `clippy -D warnings`, `fmt --check`,
      `make verify-bundled-skills` verts.

---

## Acceptance criteria

*(Transcrits du corps de mika#2565, avec la rectification du § 2 sur le mécanisme visé.)*

- [ ] **AC1** — Sur une PR dependabot `pipeline-exempt` qui touche `Cargo.lock`, qa-review
      exécute le build avant de rendre son verdict.
- [ ] **AC2** — Si le build échoue ou n'est pas lancé, le verdict n'est pas `pass`. La
      moitié structurelle est la branche B3, qui lit un **fait moteur** (`tool_calls`) et
      non une affirmation du corps.
- [ ] **AC3** — Contrôle négatif : une PR docs-only `pipeline-exempt` n'exige pas de
      build, et aucun build n'est lancé.
- [ ] **AC4** — Un test de calibration mika-qa rejoue le cas #2561 (bump majeur, label
      posé) et attend autre chose que `pass` sans build.
- [ ] **AC5** *(dérivé de R1)* — Aucune prescription du prompt ne présente
      `pipeline-exempt` comme une dispense de build, et Step 1.6 n'interdit plus Step 3e.
- [ ] **AC6** *(dérivé de R2/O4)* — Le refus B3 ne peut pas être satisfait en ajoutant une
      ligne au corps du verdict : le test de contrôle positif exige un `tool_calls`
      `build_mika` réel.

---

## 12. Hors périmètre, délibérément

- **La porte de merge** (`pr_merge_with_gate`) — elle a tenu, le ticket l'exclut.
- **Rendre `Docker Build` requis sur `main`** — question de ruleset, décision de Vincent.
- **Élargir B2 aux sauts de mineure `0.x`** — refusé sur mesure (R3) ; à rouvrir avec un
  **compte**, jamais au jugé.
- **Le mensonge sur le résultat du build** (§ 4/O4) — **suivi**, précondition : que la
  sonde S1 montre un `pass` posté sur un `tasks.result` portant `Build FAILED`.
- **Les bumps npm / `actions/*` / pip** — `build_mika` ne les compile pas ; fermer cette
  classe demande un vérificateur par écosystème, et aucune mesure ne le demande.
- **Le rescue vide de mika#2546**, que le ticket rattache à la même classe — le rapproche­
  ment est plausible mais non établi par ce travail : population différente (rescue draft,
  pas dependabot), remède différent. **Suivi**, précondition : une mesure qui montre un
  `pass` sur un rescue vide après ce déploiement.
- **Un `CARGO_TARGET_DIR` partagé** pour amortir les builds dependabot — refusé par
  mika#2497 avec ses trois motifs (verrou exclusif, accumulation inter-branches, coût non
  mesuré).
