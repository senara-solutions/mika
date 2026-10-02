# mika#2617 — la porte de merge lit TOUS les checks, et la relance ne joue qu'une fois

**Ticket :** senara-solutions/mika#2617 (ratifié par Vincent le 2026-10-02 08:20
locales, sur bearing Prime)
**Lié :** mika#2616 (le main rouge que ce défaut a produit), mika#485/#490 (la
porte d'origine), mika#2455 (la garde CI↔verdict de qa-review), mika#2238 (le
motif rendez-vous), mika#571 (`ci_success_handler`), mika#2248 (qui merge)
**Fichier decision-core — merge par Vincent (CODEOWNERS).**

---

## 1. Ce que la lecture du code déplace dans le ticket

C'est le premier livrable. Neuf rectifications, chacune change ce qu'il faut
écrire.

### R1 — `gh run rerun --job <id> --failed` n'est pas une commande valide

`gh run rerun` accepte `--failed` (relance les jobs échoués du run) **ou**
`--job <id>` (relance un job et ses dépendances) ; les deux sont mutuellement
exclusifs. La forme d'AC2 est donc inexécutable telle quelle.

La forme retenue est `gh run rerun <run_id> --failed`, et le changement de clé
est un **gain** plutôt qu'un pis-aller : N lints rouges du même run `ci.yml`
sont couverts par **une seule** relance, là où `--job` en demanderait N. La
granularité de la relance devient le **run**, et le budget « une fois » se compte
par `(dépôt, PR, tête, run)`.

### R2 — le `run_id` est déjà dans la charge utile, il n'y a aucun appel à ajouter

`run_gh_checks_raw` demande déjà `--json name,state,bucket,link`, et le `link`
d'un check GitHub Actions est
`https://github.com/<owner>/<repo>/actions/runs/<run_id>/job/<job_id>`.
L'extraction est une fonction pure. Un `link` absent, ou d'une forme qui n'est
pas un run Actions (check externe), ne produit **pas** de relance et le dit.

### R3 — la relance a besoin d'un scope que rien dans ce dépôt ne garantit

`rerun-failed-jobs` demande `actions: write`. Les scopes documentés du jeton de
la plateforme sont « Pull requests R/W, Issues R/W, Contents R » (racine
`CLAUDE.md`, `MIKA_GITHUB_TOKEN`) — `actions` n'y figure pas. **AC2 peut donc
être inerte au déploiement, en 403.** Elle est livrée fail-safe (la relance
échoue ⇒ la porte reste fermée, motif nommé) et **n'est pas revendiquée tenue
avant que la sonde S3 ait observé une relance réussie**. Un plan qui l'annoncerait
close poserait la garantie que mika#2304 nomme : un champ qui affirme, avec
autorité, l'override qui n'a pas eu lieu.

### R4 — `--required` a UN seul lecteur, et les quatre consommateurs en descendent

Mesuré par lecture : `run_gh_checks_raw` est le site unique du flag, et sa sortie
alimente

| consommateur | via |
|---|---|
| `tools::pr_merge_with_gate::execute` (la porte) | `run_gh_checks` → `classify_checks` |
| `server::verdict_handler` (merge sur verdict) | idem |
| `server::ci_success_handler` (évaluateur de la boucle) | idem |
| la garde CI↔verdict de qa-review (mika#2455) | `run_gh_checks_raw` direct → `classify_ci_coherence` → `classify_checks` |

**Le point 4 de « Attendu » — aligner la garde qa-review sur la même source — est
donc gratuit par construction.** mika#2455 a déjà payé ce lecteur unique, en
écrivant pourquoi : *« une seconde définition de « requis » est la classe que
`grooming_marker` (mika#2158) a dû fermer après des mois de divergence
silencieuse. »* Un seul `--required` à retirer aligne les quatre, et c'est ce qui
rend le cœur du correctif minuscule.

### R5 — `--auto` est la dernière définition divergente du vert, et AC1 l'implique

Sur `CheckClassification::HasPending`, **deux des trois chemins de merge arment
`gh pr merge --auto`** (`pr_merge_with_gate` étape 3, `verdict_handler` l. 646).
GitHub merge alors dès que les checks **requis** passent — c'est-à-dire sous la
définition du vert que ce ticket existe pour cesser d'employer. Lire tous les
checks puis armer `--auto` laisserait le correctif **inerte sur ce chemin**, et
c'est littéralement la phrase de Prime : *« Deux définitions du même vert
produisent toujours un main rouge. »*

Le chemin de re-entrée **existe déjà et porte déjà la bonne sémantique** :
`ci_success_handler` exige `AllPassed` **strict** et rend `Passthrough` sinon
(l. 421) — il n'arme jamais `--auto`. C'est sa raison d'être écrite (mika#571 :
*« re-evaluates merge eligibility for PRs that have a pending `VERDICT` pass but
were blocked on CI at approval time »*). **`--auto` est donc redondant avec lui,
et le seul des deux à employer une définition étrangère du vert.**

**Coût nommé.** Une PR dont la CI est en vol n'est plus mergée par GitHub derrière
nous : elle attend un `check_suite.completed(success)`. Si ce webhook est perdu —
mika#2334 a mesuré les quatre endroits où il peut l'être — la PR reste **ouverte**.
L'asymétrie tranche : une PR ouverte est visible et rattrapable à la main, un main
rouge bloque la boucle entière et vaut un p0 (mika#2616). Sonde S4 et sa halte
ci-dessous.

### R6 — un bucket inconnu reste non bloquant, et c'est à dire plutôt qu'à corriger

`classify_checks` ne refuse que `fail`/`cancel` ; `skipping` tombe dans
`AllPassed`, ce qu'AC1 demande explicitement (« `skipped` et `neutral` comptent
comme passés »). Mais **tout bucket que `gh` ajouterait naîtrait non bloquant** —
un fail-open sur une porte de merge. La politique n'est pas changée (refuser sur
l'inconnu casserait le merge sur une autre version de `gh`, c'est-à-dire
échangerait un main rouge contre une boucle arrêtée) ; une **ligne** est ajoutée
quand un bucket hors de l'ensemble connu apparaît.

### R7 — `required_check_failed` est un format de fil et ne se renomme pas

La valeur `#[serde(rename = "required_check_failed")]` de `BlockReason` est portée
par la taxonomie des trois prompts `self-dev*` (précédent : mika#2248 a ajouté
`reviewer_cannot_merge` « carried in the three `self-dev*` prompts' taxonomy »).
Le mot « requis » y devient un **vestige**, daté sur la variante. Ce qui change est
le `detail`, qui disait un compte (`"{} required check(s) failed"`) et dira les
**noms** — c'est AC3.

### R8 — AC5 doit vivre au chemin de MERGE, pas dans le tool seul

`senara-solutions/claude-pilot-py` est dans `INTERNAL_REPOS` du gateway
(`crates/mika-gateway/src/github.rs:265`), donc ses webhooks routent vers mika-dev,
donc `ci_success_handler` → `merge_ready_handler` **peut merger une PR
claude-pilot-py sans jamais passer par le tool**. Trois sites appellent
`run_gh_merge` : le tool, `verdict_handler` (l. 649), `merge_ready_handler`
(l. 244). Poser AC5 dans `execute` seul la laisserait contournable par les deux
autres.

### R9 — AC4 « vu rouge » ne peut pas être un test comportemental sur `classify_checks`

Cette fonction reçoit une liste de checks et **ne voit pas le flag** : un fixture
« un check rouge dans la liste » est déjà refusé aujourd'hui. Le seul test qui
rougit sur le code actuel est **structurel** — l'argv construit ne porte plus
`--required`. Le test comportemental est un **contrôle positif**, pas la preuve du
correctif. Écrire l'inverse revendiquerait une couverture qu'on n'a pas.

---

## 2. Requirements

### U1 — la porte lit tous les checks (AC1, première moitié)

- Extraire `gh_checks_args(pr_number: u64, repo: &str) -> Vec<String>`, fonction
  pure, site unique de l'argv de `gh pr checks`.
- **Retirer `--required`.**
- Réécrire le doc-comment de `run_gh_checks_raw`, qui **affirme aujourd'hui
  l'inverse** (« The `--required` flag is what makes this the single reader of
  "which check blocks" (D6) ») : y poser la mesure du 2026-10-01, la raison du
  changement et sa date. Sans ça le prochain lecteur restaure le flag en croyant
  réparer une régression.
- Corriger le doc-comment de `parse_gh_checks` (« Empty output or `[]` means **no
  required check** » → « aucun check du tout »).
- `classify_checks` et `classify_ci_coherence` ne bougent pas d'une ligne : elles
  héritent (R4).
- Ligne `merge_gate_unknown_check_bucket` (WARN, champs `repo`, `pr`, `name`,
  `bucket`) sur tout bucket hors `{pass, fail, pending, skipping, cancel}` (R6).
  **Pas** de ligne par évaluation nominale (doctrine mika#2131).

### U2 — la porte cesse de déléguer le vert à la protection de branche (AC1, seconde moitié)

- **Retirer le paramètre `auto` de `run_gh_merge`.** L'incapacité est structurelle,
  pas une consigne : `--auto` devient inexprimable et le compilateur force les
  trois sites (doctrine mika#1991, *construis l'incapacité, ne promets pas la
  retenue*). Un scan lexical sur un argument positionnel serait fragile là où le
  type est sûr.
- `pr_merge_with_gate` : `HasPending` devient
  `Blocked { reason: BlockReason::ChecksPending { pending_checks } }`, nom de fil
  `checks_pending`. Le `detail` nomme les checks en attente.
- `verdict_handler` : la branche `HasPending | AllPassed` se scinde ; `HasPending`
  rend `Passthrough` avec un enrichissement nommant les checks en attente, comme
  sa branche `HasFailures` voisine le fait déjà.
- `MergeGateResult::AutoMergeEnabled` est **conservé** (format de fil ; un
  `tasks.result` ancien peut le porter) avec un doc-comment daté disant qu'aucun
  site ne le produit plus. Aucun renommage.
- **Effet de bord à couvrir, trouvé en lisant :** `write_auto_merge_pr_url_to_supervisor`
  (mika#1211) n'était appelé que sur la branche auto-merge, et son rôle est de
  neutraliser le prédicat `pr_url IS NULL` du faucheur orphelin (#871). Sans lui,
  un parent `in_progress` dont le callback est `delivered` est fauché `failed`
  après 600 s de grâce. L'appel est donc **déplacé** sur le nouveau chemin
  `checks_pending` (renommé `write_pending_pr_url_to_supervisor`) : un déplacement
  d'appel, pas un mécanisme neuf.
- La taxonomie `BlockReason` des trois prompts `self-dev*` apprend
  `checks_paused`… **non** : `checks_pending`, à l'identique du nom de fil, et
  rien d'autre.

### U3 — relance une fois, puis blocage (AC2)

- `extract_actions_run_id(link: &str) -> Option<u64>` — pure, R2. Rend `None` sur
  lien absent, vide, non-Actions, ou `run_id` non numérique.
- `rerun_failed_jobs(repo, run_id, token)` → `gh run rerun <run_id> --failed`
  (R1), borné par `tokio::time::timeout`, 30 s.
- **Ledger durable**, parce qu'un ledger en mémoire ne tient pas « jamais de
  seconde relance » à travers un redémarrage : `audit_events`,
  `tool_name = "merge_gate_check_rerun"`,
  `target_key = "rerun:{repo}#{pr}@{head_sha}:{run_id}"`, lu par
  `count_recent_audit_events_for_target` sur une fenêtre de 7 jours (bornée par la
  rétention de 90 jours). Motif mika#1869 / mika#2347, clé portant la tête, donc
  **un nouveau sha rouvre le budget de lui-même** — nouveau code, nouvelle chance.
- `head_sha` : ajouter `headRefOid` au `--json` de `run_gh_pr_view` et le champ
  `head_ref_oid` à `PrPreflight` (`#[serde(default)]`, vide = illisible).
  `ci_success_handler` a déjà `pr.head_sha`.
- **Fail-CLOSED sur le ledger** — l'inverse du reste de ce travail, et l'arbitrage
  est local : un faux « déjà relancé » coûte une relance perdue sur une PR qui
  attend de toute façon un humain ; un faux « jamais relancé » relance en boucle,
  ce qu'AC2 interdit nommément.
- `RerunOutcome` : `Triggered { run_id }` | `AlreadySpent` | `NoActionsRun` |
  `LedgerUnreadable` | `Refused(String)` | `Disarmed`. `match` exhaustif **sans
  bras `_ =>`** chez l'appelant, pour qu'une issue ajoutée demain ne puisse pas
  tomber dans un défaut silencieux.
- Appelée depuis les **trois** branches `HasFailures` (le tool, `verdict_handler`,
  `ci_success_handler`), par une fonction partagée. Le ledger par
  `(pr, sha, run)` rend l'emplacement peu sensible, mais les trois y passent parce
  que le défaut mesuré (#2614, `mergedBy: mika-platform-dev`) ne dit pas lequel a
  mergé, et armer un seul site serait armer peut-être le mauvais.
- Deux échecs ⇒ `merge_gate_rerun_exhausted` (WARN + ligne d'audit) et la porte
  reste fermée. C'est « l'alarme » d'AC2 : WARN plus audit, pas de notification
  Telegram — `verdict_handler` notifie déjà l'opérateur sur un `block`, et un
  second canal pour le même fait est le churn que mika#2131 borne.
- **Pas de seconde relance automatique, jamais** : aucune variable d'environnement
  ne relève le budget. Le désarmement de la relance (et d'elle seule) est
  `MIKA_MERGE_GATE_RERUN` (défaut armé ; `0`/`false`/`off`/`no` désarment ; valeur
  non reconnue **reste armée** avec un WARN citant la valeur entre guillemets —
  polarité de `MIKA_QA_CI_COHERENCE_GATE`). Il gate la **relance**, jamais la
  lecture ni le refus : la détection est inconditionnelle (motif mika#2249/#2272).

### U4 — zéro liste d'exemptions (AC3)

- Aucune constante, aucune clé de configuration, aucun label ne permet de passer
  outre un check rouge. En particulier : la chaîne de décision ne lit **aucun**
  label de PR.
- Trois détecteurs, disposition au § 3 :
  - `mika2617_no_check_exemption_list_exists` — scan de source sous
    `tools/pr_merge_with_gate.rs` et les trois handlers, refusant une constante de
    forme `&[&str]` dont le nom porte `EXEMPT`, `ADVISORY`, `IGNORE` ou `SKIP` à
    proximité de `CHECK`. Porte son **contrôle de bonne foi** (une fixture rouge),
    sans quoi un scan visant un nom mort se lit exactement comme un arbre propre
    (classe mika#2205).
  - `mika2617_no_label_is_read_in_the_merge_decision` — scan refusant une lecture
    de `labels` dans le corps des fonctions de décision du gate, avec son contrôle
    de bonne foi. Le label est écrivable par un humain : le brancher ici
    transformerait un geste d'interface en autorisation de merge, ce que mika#2248
    a déjà dû refuser pour le signal `merge-ready`.
  - `mika2617_classify_checks_is_blind_to_the_check_name` — test de propriété :
    `classify_checks` refuse un check rouge **quel que soit son nom**, balayé sur
    les 22 noms du pont (b) plus `Docker Build`, `validate`,
    `Pilot Egress Status Tap` et un nom arbitraire.

### U5 — la trace de gate MPC, et elle vit au chemin de merge (AC5, R8)

- `MPC_GATE_MARKER_PREFIX = "<!-- mpc-gate: ok sha="` et
  `MPC_GATE_REQUIRED_REPOS = &["senara-solutions/claude-pilot-py"]`, dans
  `mika_common::forge_identity` — où `merge_disposition` vit déjà, parce que c'est
  la même famille de décision et qu'une seconde liste de politique de merge est la
  divergence programmée que ce dépôt a payée deux fois (sièges mika#2092,
  `DISPATCHABLE_REPOS` ↔ `labels.yml`).
- `extract_mpc_gate_shas(comments: &[String]) -> Vec<String>` — pure.
- `mpc_gate_verdict(repo, head_sha, comments) -> MpcGateVerdict` :
  `NotRequired` | `Attested` | `Missing` | `StaleSha { attested, head }`.
  Comparaison d'**égalité** sur le sha complet : tout push ultérieur invalide le
  marqueur de lui-même, ce qui est exactement la propriété qu'AC5 achète.
- **Fail-CLOSED** : commentaires illisibles, `gh` en échec, `head_sha` vide ⇒
  `Missing`. C'est une précondition **supplémentaire** sur une population petite ;
  son illisibilité doit refuser, sinon la précondition n'en est pas une.
- **Témoin de type, et c'est ce qui rend AC5 non contournable.**
  `run_gh_merge` prend un `MergeClearance` dont le champ est privé et dont le seul
  constructeur est `MergeClearance::from_mpc_verdict(&MpcGateVerdict) -> Option<Self>`,
  qui ne rend `Some` que sur `NotRequired` et `Attested`. Les trois sites sont
  forcés par le compilateur ; aucun scan de source fragile n'est nécessaire.
  Deuxième incapacité structurelle imposée dans le même changement de signature
  que U2 (retrait de `auto`).

### U6 — tests, surfaces et documentation (AC4)

Détaillés aux § 4 et § 6.

---

## 3. Fire-Disposition

Ce plan livre des détecteurs : trois scans de source / tests structurels (U4),
un scan d'argv (U1), plus les tests de contrat. Disposition retenue :

**(a) exception nommée en allowlist — livrée VIDE, et épinglée vide.**

- Chaque scan porte une constante d'allowlist `&[&str]` livrée **vide**, et un
  test frère assère qu'elle **reste** vide
  (`mika2617_the_exemption_allowlists_are_empty`). Une allowlist née vide est un
  tiroir où déposer la prochaine infraction (mika#2323) ; l'assertion est ce qui
  empêche le tiroir de s'ouvrir en silence.
- **Population existante : zéro** pour U4 (aucune liste d'exemption, aucune lecture
  de label dans la décision — vérifié par lecture). Donc aucune entrée à nommer,
  et c'est le régime attendu permanent.
- **Une seule violation existante, et elle est corrigée dans la même PR** : le scan
  d'argv de U1 rougit sur le site actuel qui porte `--required`. Après U1, zéro.
- **Assertion auto-nettoyante** sur chaque scan : il **échoue** si le nom qu'il
  cherche n'est écrit nulle part dans la population scannée — un scan visant un nom
  mort vérifie zéro chose et se lit exactement comme un arbre propre (classe
  mika#2103 / mika#2205).
- **Quand un scan tire, on route le site par la garde ; on n'ajoute pas de ligne**
  (doctrine mika#2201). Un site de merge qu'on ne veut pas garder est un site à
  supprimer.

Aucun détecteur n'est livré désarmé : tous visent une population vide et aucun ne
peut rougir sur l'arbre tel qu'il est après U1.

---

## 4. Verification contract

### Tests d'unité (fonctions pures)

| test | ce qu'il atteste |
|---|---|
| `mika2617_gh_checks_args_carries_no_required_flag` | **vu rouge sur le code actuel** — R9, le seul test qui rougit |
| `mika2617_gh_checks_args_still_requests_the_link_field` | la relance (U3) dépend de `link` ; le perdre la rendrait inerte sans rien casser |
| `mika2617_classify_checks_is_blind_to_the_check_name` | AC3, test de propriété sur 25 noms |
| `mika2617_extract_actions_run_id_*` | lien nominal ; absent ; non-Actions ; `run_id` non numérique ; lien d'un autre dépôt |
| `mika2617_extract_mpc_gate_shas_*` | marqueur nominal ; plusieurs commentaires ; sha tronqué ; marqueur dans un bloc de code (**contrôle négatif**) |
| `mika2617_mpc_gate_verdict_*` | `NotRequired` hors population ; `Attested` sur égalité ; `StaleSha` sur divergence ; `Missing` sur absence et sur illisible |
| `mika2617_merge_clearance_is_not_constructible_from_a_refusal` | le témoin de type ne se fabrique pas depuis `Missing`/`StaleSha` |
| `mika2617_rerun_outcome_has_no_wildcard_arm` | scan de source : le `match` de l'appelant est exhaustif |

### Tests comportementaux (contrôles positifs, R9)

`crates/mika-agent/tests/eval/test_merge_gate_all_checks_2617.rs`, patron
mika#2455 : **le seam est le stdout brut du subprocess**, jamais les checks déjà
parsés — un test qui injecterait des `GhCheck` laisserait `parse_gh_checks` et
`classify_checks` hors du chemin de production et attesterait son propre parseur.

| cas | attendu |
|---|---|
| T1 — un check rouge, nom hors des 6 requis d'avant le pont | porte **fermée**, `required_check_failed`, le `detail` **nomme** le check (AC3) |
| T2 — contrôle positif : tous verts | porte **ouverte**, merge immédiat |
| T3 — contrôle positif : un `skipping` et un `pass` | porte **ouverte** (AC1, « skipped compte comme passé ») |
| T4 — un `pending` | **refus** `checks_pending`, aucun `--auto` armé (U2), `pr_url` écrit sur le superviseur |
| T5 — rouge, ledger vide | relance déclenchée une fois, porte fermée sur `rerun_triggered` |
| T6 — rouge, ledger portant déjà une ligne pour ce `(pr, sha, run)` | **aucune** relance, porte fermée sur `rerun_exhausted` + WARN |
| T7 — rouge, `link` non-Actions | aucune relance, motif `no_actions_run`, porte fermée |
| T8 — rouge, relance refusée en 403 | aucune boucle, motif `rerun_refused`, porte fermée (R3) |
| T9 — nouveau sha après T6 | le budget est **rouvert**, une relance |
| T10 — `claude-pilot-py`, aucun marqueur | merge **refusé**, `mpc_gate_missing` |
| T11 — `claude-pilot-py`, marqueur sur un sha ancien | merge **refusé**, `mpc_gate_stale_sha` |
| T12 — `claude-pilot-py`, marqueur sur la tête | merge autorisé par ce terme |
| T13 — contrôle négatif : `senara-solutions/mika`, aucun marqueur | merge autorisé — le gate MPC ne mord **que** sur sa population |

**Chaque terme est vu rouge par mutation, un à la fois.** Une conjonction de
termes fail-safe ne se prouve pas en les neutralisant tous ensemble (leçon
mika#2277) : T2, T3 et T13 sont les contrôles sans lesquels « la porte décide »
est indistinguable de « la porte bloque tout », et ce second état casserait la
boucle en entier avec toutes les autres assertions au vert.

### Vérifications non automatisables, déclarées comme telles

- **V1 (R3)** — `actions: write` sur le jeton : non vérifiable depuis le bac à
  sable de dispatch (`gh` y est refusé par la politique). Geste opérateur, sonde S3.
- **V2** — la distribution réelle des relances (quelle part repasse au vert) :
  demande `~/.mika/data/mika.db`, non montée dans le bac à sable. Sonde S5.
- **V3** — le pont (b) côté ruleset est un geste Vincent, hors code.

---

## 5. Acceptance criteria

- [ ] **AC1.** `pr_merge_with_gate` lit **tous** les checks de la tête, pas
      seulement `--required` ; la porte n'est ouverte que si aucun check n'est
      rouge, en attente ou annulé ; `skipped` et `neutral` comptent comme passés.
      **Tenue sur le merge immédiat ET sur l'attente** : U2 retire l'armement de
      `--auto`, qui était la dernière délégation du vert à la protection de
      branche (R5).
- [ ] **AC2.** Un check rouge déclenche **une** relance automatique
      (`gh run rerun <run_id> --failed`, R1). S'il repasse au vert, la porte se
      ré-évalue par le rendez-vous `check_suite.completed` existant (motif
      mika#2238), jamais par une boucle d'attente. Deux échecs ⇒ porte fermée,
      motif nommé, alarme. **Jamais de seconde relance automatique** (ledger
      durable par `(dépôt, PR, tête, run)`). **Non revendiquée tenue avant la
      sonde S3** (R3).
- [ ] **AC3.** Zéro liste d'exemptions : aucune constante, aucune configuration,
      aucun label ne permet de passer outre un check rouge. Trois détecteurs
      l'épinglent (U4), allowlists livrées vides et épinglées vides.
- [ ] **AC4.** Un check **non requis** rouge bloque le merge autonome, **vu rouge
      sur le code actuel** — le test qui rougit est structurel (l'argv), les tests
      de comportement sont des contrôles positifs (R9). Contrôle positif tous
      verts ; relance échec-puis-vert ouvre, deux échecs bloque.
- [ ] **AC5.** Pour `senara-solutions/claude-pilot-py`, la porte exige en plus un
      commentaire portant `<!-- mpc-gate: ok sha=<SHA complet> -->` dont le `sha`
      **égale** la tête. Posée au chemin de merge et tenue par un **témoin de
      type**, donc non contournable par les deux handlers (R8).
- [ ] **AC6 (ajoutée, R4).** La garde CI↔verdict de qa-review lit la même source.
      Gratuit par construction — attesté par un test, pas par du code neuf.

---

## 6. Surfaces opérateur

```bash
# 1. Une relance a-t-elle été déclenchée, et sur quel run ?
grep merge_gate_check_rerun "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{repo, pr, head_sha, run_id, outcome}'

# 2. Le budget a-t-il été épuisé ? (régime attendu : FAIBLE, non vide)
grep merge_gate_rerun_exhausted "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{repo, pr, head_sha, run_id, failing}'

# 3. CONTRÔLE POSITIF — la porte s'évalue-t-elle seulement ?
grep -c merge_gate_blocked "$MIKA_SPIRIT_LOG_FILE"

# 4. Le gate MPC a-t-il refusé ? (régime attendu : VIDE hors claude-pilot-py)
grep mpc_gate_refused "$MIKA_SPIRIT_LOG_FILE" | jq -c '{repo, pr, reason, attested, head}'

# 5. Un bucket inconnu est-il apparu ? (régime attendu : VIDE)
grep merge_gate_unknown_check_bucket "$MIKA_SPIRIT_LOG_FILE" | jq -c '{name, bucket}'
```

```sql
-- La population des relances, par issue
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'merge_gate_check_rerun' GROUP BY 1 ORDER BY 2 DESC;

-- Le résidu d'AC1 : combien de merges ont délégué le vert à GitHub avant U2 ?
-- (zéro après déploiement — la ligne cesse d'être écrite parce que le site
--  cesse d'exister ; cette requête est l'avant/après)
SELECT count(*) FROM audit_events
 WHERE tool_name = 'verdict_handled' AND after_value = 'auto_merge_enabled';
```

| surface | niveau | régime attendu | lecture |
|---|---|---|---|
| `merge_gate_check_rerun`, `outcome = triggered` | INFO | **non vide** après S3 | chaque ligne est une relance ; **vide** veut dire inerte, voir halte 2 |
| `merge_gate_check_rerun`, `outcome = refused` | WARN | **vide** | 403 probable : le scope `actions: write` manque (R3) |
| `merge_gate_rerun_exhausted` | WARN | non vide, faible | deux échecs : ce n'est pas flaky, c'est cassé — le comportement voulu |
| `merge_gate_unknown_check_bucket` | WARN | **vide** | `gh` a ajouté un bucket : il naît non bloquant (R6) |
| `mpc_gate_refused` | WARN | **vide** hors `claude-pilot-py` | une occurrence sur un autre dépôt est une fuite de population |
| `checks_pending` dans `tasks.result` | — | non vide | le nouveau refus de U2 ; son absence avec des PR qui attendent = halte 4 |

`merge_gate_check_rerun` est **SOLE WRITER** de son nom dans le journal et dans
`audit_events` (scan de source, allowlist livrée vide) : c'est ce qui rend le
`GROUP BY` exact plutôt qu'un nombre sur lequel deux sites peuvent diverger.
Aucun test comportemental ne peut voir cette classe — un second écrivain ne rend
aucune décision fausse le jour où il est écrit.

**Coût daté, nommé plutôt que découvert :** un `count(*)` sur
`after_value = 'auto_merge_enabled'` qui enjambe le déploiement compare deux
régimes — avant, la population existe ; après, elle est close par construction.
Les lignes antérieures ne sont **pas** réécrites (motif mika#2361).

---

## 7. Sondes post-déploiement, et leurs six haltes

> **Préalable.** Ces mesures décrivent le **binaire servi**. Établir après
> `make deploy` que le `mika-spirit` qui tourne porte le correctif avant toute
> conclusion (classe mika#2340). Ce sont des **gestes d'opérateur sur l'hôte** :
> la base n'est pas montée dans le bac à sable de dispatch et `gh` y est refusé.

**S1 — le défaut fondateur est rejoué (première PR dont un check non requis est
rouge).** La porte refuse, le `detail` nomme le check, aucun merge.
*Halte 1 — la PR est mergée quand même :* **ne pas élargir le prédicat par
réflexe.** Lire le contrôle positif (commande 3) : zéro ligne signifie que la
porte ne s'évalue pas du tout, et la question n'est alors pas le prédicat.
Établir ensuite **quel** des trois chemins a mergé (`mergedBy` plus
`verdict_handled` / `ci_success_merge_ready` / `pr_merge_with_gate`) — les trois
remèdes diffèrent.

**S2 — contrôle négatif : une PR entièrement verte merge toujours (48 h).**
*Halte 2 — plus rien ne merge :* la porte est passée de laxiste à bloquante, ce
qui casse la boucle en entier. **Désarmer d'abord** (revert de U1), diagnostiquer
ensuite : lire quel bucket la porte compte comme rouge — un bucket inconnu
(commande 5) ou un check `Expected — waiting` qu'un ruleset vient de rendre requis
sans que le job tourne (le risque que Vincent nomme dans le pont (b)).

**S3 — la relance tourne, c'est-à-dire que le scope existe (R3, première PR
rouge).** `merge_gate_check_rerun` avec `outcome = triggered`, et le run
réapparaît sur GitHub.
*Halte 3 — `outcome = refused` avec un 403 :* **AC2 est inerte et il faut le
dire** plutôt que la revendiquer. Le remède est un scope `actions: write` sur
l'App ou le PAT — geste opérateur, pas un correctif de code. Entre-temps la porte
reste fermée, ce qui est le bon état.

**S4 — contrôle négatif de U2 (7 jours).** Aucune PR verte et approuvée ne reste
ouverte plus de 30 min.
*Halte 4 — une PR reste ouverte :* le rendez-vous `check_suite.completed` n'a pas
eu lieu (webhook perdu, les quatre endroits de mika#2334). **Ne pas remettre
`--auto`** — ce serait rouvrir AC1. Lire d'abord si un `check_suite` est arrivé
(`ci_success_handler_processed`), puis si `ci_success_handler` a refusé et sur quel
terme. Le remède est en amont, sur le canal, et il a son propre ticket.

**S5 — la mesure qui conditionne la révision de la relance (30 jours).** Quelle
part des `triggered` est suivie d'un run vert ?
*Halte 5 — la part est proche de zéro :* la relance ne rattrape rien et coûte un
run CI complet par PR rouge. C'est un **résultat**, pas une panne : il réfute
l'hypothèse flaky pour cette flotte et c'est la précondition d'un ticket qui
retire la relance ou la restreint à une liste de checks. L'ouvrir **avec ce
compte**, jamais avec une intuition. Et **ne pas désarmer entre-temps** : la
relance est ratifiée par Vincent sur bearing Prime.

**S6 — le gate MPC mord sur sa population et nulle part ailleurs (30 jours).**
`mpc_gate_refused` ne porte que `claude-pilot-py`.
*Halte 6 — une occurrence sur un autre dépôt :* la liste de population est lue de
travers et des merges légitimes sont refusés. Désarmer par revert de U5 **avant**
diagnostic.

**Halte transverse — les sondes muettes.** Zéro refus **et** zéro évaluation ne
prouve rien : il faut qu'une PR ait traversé la porte depuis le déploiement.
Vérifier le contrôle positif avant toute conclusion. *Une garde que personne n'a
exercée se lit exactement comme une garde qui marche* (mika#2205).

---

## 8. Ce que ce travail n'achète PAS

- **Il ne rattrape pas #2614 ni mika#2616.** La PR est mergée, main a été rouge, et
  **rien ici ne rétro-estampille** — fabriquer une ligne d'audit datée d'un refus
  qu'on n'a pas observé est l'inverse de ce que ce travail défend. La sonde est la
  **prochaine** occurrence.
- **Il ne pose pas le pont (b).** Rendre 22 checks requis est le ruleset de `main`,
  donc un geste Vincent. La règle (a) est sûre **sans** le pont (elle est plus
  stricte que la protection de branche) ; le pont reste utile parce qu'il réaligne
  les deux définitions du vert pour tout consommateur **hors** de ce dépôt — y
  compris un merge humain par l'interface GitHub, que ce code ne traverse pas.
- **Il ne couvre pas le merge par l'interface GitHub ni par `gh pr merge` tapé à la
  main.** La porte garde le moteur. Un humain avec le bypass admin merge toujours
  sur rouge, et c'est la moitié que le pont (b) ferme.
- **Il ne garantit pas qu'une relance répare quoi que ce soit** (S5), et il ne
  garantit pas que la relance soit même possible (R3, S3).
- **Il ne rend pas la porte surveillée.** Les seuls instruments sont les greps et
  les requêtes du § 6, et **leur silence ne prouve rien tant que personne ne les
  exécute**.
- **Il ne ferme pas le fail-open sur un bucket inconnu** (R6) : il le rend visible.

---

## 9. Hors périmètre, délibérément

- **Le point 2 du commentaire opérateur du 2026-10-01 (n=2, #2618 mergée sans
  `docs/solutions` ni trailer `Compound: none`).** Même classe — la porte moteur
  est plus laxiste que la recette humaine — mais sur la **complétude du pipeline**
  et non sur la CI. Son signal existe déjà et est exécutable
  (`scripts/verify-pipeline.sh`), donc le remède est de brancher ce script sur la
  porte, ce qui est un terme de plus avec sa propre politique fail-safe, sa propre
  population et son propre coût en appels. **Ticket de suivi**, nommé dans le corps
  de la PR ; le mélanger ici ferait un plan dont aucun des deux moitiés ne serait
  revue correctement.
- **La fermeture du fail-open sur un bucket inconnu** (R6) — précondition : que la
  commande 5 rende au moins une ligne.
- **Le retrait de la relance** (S5) — précondition : la mesure de S5.
- **`MergeGateResult::AutoMergeEnabled`** : conservé comme format de fil, aucun
  renommage, aucun retrait.
- **`classify_checks`, `classify_ci_coherence` et la garde mika#2455** : aucune
  ligne modifiée. Elles héritent (R4), et c'est la propriété qui rend AC6 gratuite.
- **Le bypass admin de l'identité du pilote et le durcissement ruleset no-bypass
  `main`** : seconde moitié de l'incident mika#2520, ticket distinct.
- **`gh api` plutôt que `gh run rerun`** pour la relance : l'endpoint
  `rerun-failed-jobs` serait plus explicite, mais `gh run rerun <run_id> --failed`
  reste dans le vocabulaire du ticket et n'ajoute aucune forme nouvelle à relire.

---

## 10. Definition of Done

- [ ] U1 à U6 implémentés ; `cargo fmt`, `cargo clippy`, `cargo test` verts.
- [ ] Le test d'argv (`mika2617_gh_checks_args_carries_no_required_flag`) a été
      **vu rouge** sur le code actuel avant correctif, et le fait est rapporté
      dans le corps de la PR (R9/AC4).
- [ ] Les treize cas de `test_merge_gate_all_checks_2617.rs` passent, et chaque
      terme a été **vu rouge par mutation individuelle**, rapporté dans le corps.
- [ ] Les trois allowlists d'exemption sont livrées vides et le test frère qui les
      épingle vides passe (§ 3).
- [ ] Les trois prompts `self-dev*` apprennent `checks_pending` dans leur taxonomie
      `BlockReason` (U2).
- [ ] Le doc-comment de `run_gh_checks_raw` ne dit plus l'inverse de ce que fait le
      code, et porte la mesure du 2026-10-01 avec sa date (U1).
- [ ] `crates/mika-agent/CLAUDE.md` § *PR Merge Gate* et la racine `CLAUDE.md`
      portent la nouvelle sémantique, les cinq surfaces opérateur, les six sondes
      et leurs haltes, et le § *Ce que ça n'achète pas*.
- [ ] `docs/solutions/` porte l'entrée de la leçon : *deux définitions du même vert
      produisent un main rouge, et `--auto` en est une* — ou le trailer
      `Compound: none` est justifié dans le corps de la PR.
- [ ] Le corps de la PR nomme **AC2 comme non revendiquée tenue** jusqu'à la sonde
      S3, et nomme le ticket de suivi du § 9 avec `Tracked in:`.
- [ ] Le corps de la PR nomme la **migration manuelle** : aucune, et aucune
      variable d'environnement existante ne change de sens. La seule variable neuve
      est `MIKA_MERGE_GATE_RERUN`, armée par défaut.
