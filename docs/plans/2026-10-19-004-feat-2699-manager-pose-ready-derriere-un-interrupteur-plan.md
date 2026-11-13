# mika#2699 — vol 4 : le geste d'écriture derrière `MIKA_MANAGER_POOL_WRITE`

> **Ticket :** senara-solutions/mika#2699 — *mika-manager Phase 2 (a) : tenir le bassin ready entre 2 et 4, en ombre puis derrière un interrupteur*
> **Vol :** 4 sur 5 (le découpage passe de quatre à cinq vols par mesure — § *Taille estimée*)
> **Plan antérieur :** `docs/plans/2026-10-15-001-feat-2699-manager-tient-le-bassin-ready-plan.md` (sur `main`) — son vol « 3 » est ce vol 4, la numérotation a glissé quand le vol 2 a détaché son second motif de refus.
> **Vols livrés :** vol 1 (#2710), vol 2 (#2714), vol 3 (#2800).
> **Porte :** la boucle construit (a) en ombre — décision de Vincent du 18/10/2026. **L'interrupteur reste OFF par défaut et ne s'ouvre que sur le mot de Vincent.** Ce vol livre l'interrupteur, pas son ouverture.

---

## Pourquoi ce plan existe

Les trois premiers vols ont rendu le bassin `ready` **observable** : le manager
calcule à chaque cycle sa profondeur, sa bande, l'ordre d'admission et le motif
nommé de chaque refus — et n'a, structurellement, aucun moyen d'écrire. Le vol 4
est celui où le manager cesse de commenter et se met à poser le label.

C'est le seul vol du ticket qui ouvre un chemin d'écriture dans un module placé
sous charte **LECTURE SEULE**. Tout le plan tient dans la manière de l'ouvrir :
un interrupteur fermé par défaut, un geste unique et plat, une exception nommée
au détecteur qui arrive dans le même commit que le geste, et aucune
reformulation du gabarit.

Le précédent structurel est **mika#2698 vol 2** (`verdict_poster.rs`,
`b3167e0d`) : même forme — un interrupteur, un geste, une exception nommée, un
câblage de cycle. Ce vol en est la transposition, et toutes les décisions
ci-dessous consistent à suivre ce gabarit plutôt qu'à en inventer un second.

---

## Ce que ce vol construit

### 1. `crates/mika-agent/src/milestone_manager/pool_writer.rs` — nouveau fichier

**Pourquoi un fichier neuf et pas `pool.rs`.** L'exception de
`no_dispatch_test.rs` est **par couple (fichier, jeton)**. Exempter `pool.rs`
pour `gh issue edit` exempterait 1 412 lignes d'évaluation pure pour un geste
qui en fait cinquante : la surface exemptée serait trente fois le geste. Un
fichier sibling dédié borne l'exception à ce qu'elle couvre. `pool.rs` reste
pur — `assess_pool` ne fait toujours aucune E/S.

**Cinq propriétés, chacune portée par du code et non par de la prose :**

```rust
/// L'interrupteur, et son lecteur unique (doctrine mika#2201).
pub const ENV_POOL_WRITE: &str = "MIKA_MANAGER_POOL_WRITE";

/// Le geste, **plat**, pour que le grep de `no_dispatch_test.rs` voie
/// exactement ce qui s'exécute.
const GESTE: &str = "gh issue edit";

pub fn pool_write_is_armed() -> bool;              // seul lecteur de la variable
fn armed_from_value(raw: Option<&str>) -> bool;    // None / vide / coquille = false

/// `armed` est un **paramètre**, pas une lecture d'environnement : la branche
/// OFF est donc testable sans `unsafe { env::set_var }`.
pub async fn admit_pool_tickets(
    armed: bool,
    runner: &dyn GhRunner,
    repo: &str,
    milestone_title: &str,
    admissions: &[Admission],
) -> Vec<u64>;                                     // les numéros réellement posés

/// L'argv **dérive** de `GESTE` : le jeton que le scan lit est le programme
/// qui tourne. `debug_assert_eq!(programme, Some("gh"))`.
fn geste_args() -> Vec<&'static str>;
```

- **Fermé par défaut.** `armed_from_value` : `None`, chaîne vide, valeur
  inconnue ⇒ `false` ; `1` / `true`, insensible à la casse et trimmé ⇒ `true`.
  Table de vérité reprise à l'identique de `verdict_poster::armed_from_value`.
  *Décision : on duplique les dix lignes de la table plutôt que de refactorer un
  site partagé.* Un extracteur toucherait `verdict_poster.rs` — un chemin
  d'écriture déjà en production — pour économiser dix lignes ; le vol paie la
  duplication et la nomme.
- **Le geste groupé, en un seul appel.** `gh issue edit <n> --repo <repo>
  --add-label ready --milestone "<titre>"`. Les deux gestes du point 3 du ticket
  (« poser `ready` et rattacher au milestone cible, **en un seul geste
  groupé** ») sont deux drapeaux d'une même invocation, jamais deux appels.
- **`--milestone` résout par *titre*.** `gh issue edit -m` prend le nom du
  milestone, pas son numéro. Le geste consomme donc `MilestoneState.title` — lu
  au même cycle — et non `MilestoneRef.number`, qui ne porte pas de titre.
  `gh api -X PATCH` n'est pas une alternative : `"gh api -X PATCH"` et
  `"\"PATCH\""` sont tous deux des `FORBIDDEN_TOKENS`.
- **Jamais de retrait.** Propriété **positive**, pas une promesse : `GESTE` est
  plat, l'argv en dérive, et un test assère que l'argv composé ne contient aucun
  jeton préfixé `--remove` (ni `--remove-label`, ni `--remove-milestone`). Le
  retrait de `ready` pendant un vol n'est pas interdit par convention — il n'est
  pas exprimable.
- **Un titre vide n'écrit rien.** `milestone_title.trim().is_empty()` ⇒ le
  ticket n'est pas admis et l'événement le dit. Motif mika#2331 : un titre absent
  n'est pas un titre vide, et surtout pas un milestone à détacher.
- **Un échec `gh` ne compte pas comme posé.** La valeur de retour ne porte que
  les numéros dont l'invocation a réussi, ticket par ticket : un refus de la
  forge sur le troisième candidat ne retire pas les deux premiers.

**Tests en ligne (4), transposés de `verdict_poster.rs` :**

| test | ce qu'il pin |
|---|---|
| `mika2699_la_table_de_verite_de_linterrupteur` | `None`/vide/coquille ⇒ OFF ; `1`/`true`/` TRUE ` ⇒ ON. Et `pool_write_is_armed()` est faux sans la variable. |
| `mika2699_interrupteur_off_aucun_appel_decriture` | **AC1.** `armed = false` + trois admissions ⇒ le `RunnerEnregistreur` n'a enregistré **aucun** appel, et la valeur de retour est vide. |
| `mika2699_interrupteur_on_un_seul_appel_groupe_et_cest_le_bon_geste` | **Contrôle négatif d'AC1 + première moitié d'AC3.** `armed = true`, une admission ⇒ **exactement un** appel, argv assérée **position par position** (`issue`, `edit`, `<n>`, `--repo`, `<repo>`, `--add-label`, `ready`, `--milestone`, `<titre>`), et aucun jeton `--remove*`. Sans ce test, AC1 est satisfait par un module qui n'écrit jamais — motif mika#2205 : une garde cassée et un module propre rendent les mêmes octets. |
| `mika2699_un_echec_gh_et_un_titre_vide_ne_comptent_pas_comme_poses` | Le `RunnerEnPanne` ⇒ retour vide ; titre vide ⇒ aucun appel. |

Harness local : un `RunnerEnregistreur` (`Mutex<Vec<Vec<String>>>`) et un
`RunnerEnPanne`, ≈ 30 lignes. *Décision : on les re-déclare plutôt que
d'élargir la visibilité de ceux du `mod tests` de `verdict_poster.rs`* — rendre
un helper de test importable entre siblings coûte plus de surface que les trente
lignes qu'il économise.

### 2. `no_dispatch_test.rs` — l'exception nommée, dans le même commit

Le gabarit grep reste **inchangé octet pour octet** : aucun jeton retiré d'
`EXEMPT_FILES`, aucun assouplissement de `scan_source`. Seule la table
d'exceptions gagne une entrée :

```rust
const EXEMPTED_OCCURRENCES: &[(&str, &str, &str)] = &[
    (
        "verdict_poster.rs",
        "gh issue comment",
        "mika#2698 vol 2 — le seul geste d'écriture autorisé, derrière MIKA_MANAGER_EVALUATE_WRITE",
    ),
    (
        "pool_writer.rs",
        "gh issue edit",
        "mika#2699 vol 4 — pose `ready` + rattache au milestone cible en un geste groupé, derrière MIKA_MANAGER_POOL_WRITE",
    ),
];
```

`mika2698_lallowlist_ne_porte_que_le_geste_du_vol_2` assère aujourd'hui
`EXEMPTED_OCCURRENCES.len() == 1` : **il rougit** et doit être réécrit, pas
supprimé. Il devient
`mika2699_lallowlist_porte_les_deux_gestes_et_pas_un_de_plus` et pin :

1. `len() == 2`, et le triplet `(fichier, jeton, raison)` exact de chaque entrée
   — toute troisième exception future est un rouge, donc une décision ;
2. chaque `raison` cite un ticket (`mika#`) — une exception sans ticket de suivi
   est un rouge ;
3. **l'assertion auto-nettoyante** : pour chaque entrée, le fichier exempté
   existe sous `milestone_manager/` **et** sa source de production contient
   réellement le jeton. Une exception devenue inutile — geste retiré, fichier
   renommé — rougit au lieu de dormir. La source est lue **commentaires
   dépouillés** (`strip_line_comments`), motif mika#2158/#2050 Signal S : de la
   prose qui mentionne le jeton n'est pas un geste.

`mika2698_une_exception_ne_couvre_quun_seul_jeton_du_fichier` et
`mika2698_le_scan_rougit_sur_une_source_fautive` restent **verts et inchangés**.
`mika2700_le_vol_1_nouvre_aucun_chemin_decriture` (dans `mod.rs`) reste vert
aussi : il porte sur `relief.rs` / `reporter.rs` / `mika#2700` et sur
`MIKA_MANAGER_GROOM_WRITE`, aucun des cinq.

**Limite connue, héritée et non corrigée ici :** le scan est un grep plat, donc
la forme éclatée `.args(["issue", "edit", …])` lui échappe. C'est pourquoi
`GESTE` est un littéral plat et l'argv en dérive — le fichier qui écrit est
celui dont le scan voit le geste.

### 3. `cadence.rs` — le câblage du cycle

Sans câblage, `MIKA_MANAGER_POOL_WRITE=1` ne changerait rien : le docstring de
`mod.rs` nomme déjà cet état deux fois — **« un interrupteur dont la branche ON
ne fait rien est un mensonge de surface »**. Le câblage est donc dans **ce** vol,
pas dans le suivant.

- `async fn admit_pool_tickets(cfg, ctx, state, assessment)` : sort tôt si
  `ctx.gh_runner` est `None`, si `assessment.pool` est `None`, ou si
  `verdict.admit` est vide ; sinon appelle
  `pool_writer::admit_pool_tickets(pool_write_is_armed(), runner, repo,
  &state.title, &verdict.admit)`.
- Site d'appel dans le corps du cycle, **après** `assessor.assess(&state)` (le
  verdict du bassin est un produit de l'évaluation) et dans le même voisinage que
  `post_flight_verdicts`.
- Un événement `tracing` `manager_pool_admitted` (numéros posés) et son
  contraire `manager_pool_not_admitted` (raison : désarmé / pas de runner / titre
  vide), pour que la sonde existe dès le premier cycle — motif mika#2340 :
  l'absence de ligne de sonde est le signe d'un binaire périmé, pas d'un cycle
  calme.
- **Aucun champ de checkpoint.** `posted_flight_task_ids` existe parce qu'un
  *commentaire* ne laisse aucune trace dans l'ensemble que le manager relit. Un
  *label*, si : une fois `ready` posé, le filtre de candidats d'`assess_pool`
  écarte le ticket et la profondeur remontée sort la bande de la famine, donc
  `admit` se vide de lui-même. L'idempotence est **structurelle**, portée par la
  lecture du cycle suivant, et `--add-label` est de surcroît un no-op à la forge
  sur un ticket déjà labellisé. *C'est la décision la plus importante du vol :
  elle supprime un champ persisté, une rétention et une fusion.*
- Un test de cycle : interrupteur OFF, un verdict portant une admission ⇒ aucun
  appel `issue edit` dans le runner enregistré du cycle. Il réutilise la fixture
  `with_gh_runner(Some(&gh))` déjà présente dans les tests de `cadence.rs`.

### 4. Docstrings et déclaration

- **`mod.rs`** — la section Phase 2 (a) (lignes 51–85) passe du futur au présent :
  le geste existe, il est derrière `MIKA_MANAGER_POOL_WRITE`, il est exempté
  nommément, et **la charge de l'ouverture est opératrice**. La phrase du
  mensonge de surface reste : elle est maintenant honorée, pas promise.
  Plus `pub mod pool_writer;` et le `pub use` de `ENV_POOL_WRITE`,
  `admit_pool_tickets`, `pool_write_is_armed`.
- **`pool.rs`** — le § *Ce que ce module ne dit pas encore* annonce ce vol ; il
  devient un renvoi : le geste vit dans `pool_writer.rs`, `pool.rs` reste pur.
- **`.env.example`** — `# MIKA_MANAGER_POOL_WRITE=1` sur le modèle de
  `MIKA_MANAGER_EVALUATE_WRITE` (ligne 386) : le geste exact, le fait qu'il est
  OFF par défaut, le fait que **l'ouvrir met le manager à alimenter la boucle
  autonome**, et les deux sondes à grepper. **Dette structurelle, pas
  cosmétique :** `sink_dir.rs:599` (garde T7,
  `mika2267_every_manager_env_const_is_declared_in_env_example`) rougit sur tout
  `pub const ENV_…` du module absent de `.env.example` — donc ce fichier bouge
  dans le **même commit** que la const.

---

## Ordre de commit

1. **Le geste et son exception, indivisibles.** `pool_writer.rs` +
   l'amendement de `no_dispatch_test.rs` + `mod.rs` (`pub mod`, `pub use`,
   docstring) + `.env.example`. Trois gardes lient ce commit : le scan rougit si
   l'exception manque, la garde T7 rougit si `.env.example` manque, et le
   docstring de `mod.rs` a promis l'atomicité. Le découper, c'est livrer un
   commit rouge.
2. **Le câblage.** `cadence.rs` + son test de cycle.
3. **Le renvoi.** Docstring de `pool.rs`.

---

## Fire-Disposition

**Option (a) — exception nommée en allowlist**, armée, sans report.

Détecteurs livrés ou amendés par ce vol :

| détecteur | état |
|---|---|
| le scan structurel de `no_dispatch_test.rs` | **amendé** : une seconde entrée en allowlist, gabarit inchangé |
| `mika2699_lallowlist_porte_les_deux_gestes_et_pas_un_de_plus` | **neuf** : borne la table à deux entrées et les pin au triplet exact |
| l'assertion de staleness sur chaque exception | **neuve** : fichier exempté existant + jeton réellement présent dans sa production |
| l'assertion « aucun jeton `--remove*` dans l'argv composé » | **neuve**, dans `pool_writer.rs` |

Violations existantes au moment de l'atterrissage : **deux**, toutes deux
exemptées nommément.

- `("verdict_poster.rs", "gh issue comment", …)` — préexistante, mika#2698 vol 2.
- `("pool_writer.rs", "gh issue edit", …)` — celle de ce vol, mika#2699 vol 4.

Chaque entrée nomme **la donnée précise** (le fichier et le jeton, pas le
module), **référence un ticket de suivi** dans sa troisième colonne, et est
couverte par **l'assertion auto-nettoyante** ci-dessus : le jour où le geste
disparaît ou change de fichier, l'exception rougit au lieu de devenir un trou
silencieux dans la charte. C'est exactement ce qui distingue cette allowlist de
celle que mika#2309 condamne — le geste qui rend l'exception inutile est le geste
qui la fait expirer.

Ni (b) ni (c) : livrer désarmé n'a pas de sens pour un détecteur dont tout le
dépôt dépend déjà, et il n'y a rien à remonter — l'ouverture de l'interrupteur
est déjà, par construction, une décision opératrice.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/pool_writer.rs` (production + docstrings) | 148 |
| `crates/mika-agent/src/milestone_manager/pool_writer.rs` (4 tests en ligne + harness) | 100 |
| `crates/mika-agent/src/milestone_manager/no_dispatch_test.rs` (exception, bornage, staleness) | 72 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` (câblage, sondes, test de cycle) | 48 |
| `crates/mika-agent/src/milestone_manager/mod.rs` (docstring, `pub mod`, `pub use`) | 24 |
| `crates/mika-agent/src/milestone_manager/pool.rs` (docstring § *ce que ce module ne dit pas encore*) | 8 |
| `.env.example` (déclaration commentée + sondes) | 18 |

Total estimé : 418 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et pourquoi il se découpe quand même.** Le périmètre
complet du vol 4 tel que le ticket le décrit — interrupteur, geste, exception,
câblage, AC1 **et AC3 en entier**, plus la ligne de rapport nommant ce qui a été
posé — a été chiffré livrable par livrable à **≈ 520 lignes**, au-delà de la
borne. Les ancres mesurées confirment le risque plutôt que l'infirment : le vol
structurellement identique (mika#2698 vol 2, `b3167e0d`) a livré **903
insertions hors `docs/`** pour une borne de 431, et le vol 3 de ce ticket
(`018615c8`) **1 604**. Le vol 2 a nommé la cause : la densité de docstrings
(37 % du diff) que l'estimation n'avait pas comptée — elle **est** comptée
ci-dessus, docstrings inclus dans chaque ligne du tableau.

Le périmètre ci-dessus repasse donc sous la borne, à 418 pour 429. La marge est
mince et c'est assumé : **garde-fou de volume** — si le diff passe 500 lignes
avant que l'amendement de `no_dispatch_test.rs` soit écrit, le test de cycle de
`cadence.rs` part au vol 5 et le vol courant atterrit sans lui (AC1 reste tenu
au niveau unitaire par les deux tests de `pool_writer.rs`).

**Ce qui part au vol 5** (≈ 130 lignes estimées) :

- **La seconde moitié d'AC3 — l'idempotence pinnée.** Ce vol livre « une seule
  mutation par décision » (une invocation groupée par ticket admis, argv assérée)
  et **énonce** l'idempotence comme propriété du lecteur ; le vol 5 la **prouve**
  par un test de verdict réinjecté : `assess_pool` rejoué sur un bassin où
  `ready` vient d'être posé rend un `admit` vide. Risque accepté d'ici là :
  l'effet d'un `--add-label` répété est un no-op à la forge — contrairement à un
  commentaire, un label n'a pas d'effet visible en double. Le pire cas d'un pin
  manquant est un appel API redondant, jamais un effet dupliqué.
- **La ligne de rapport** nommant les tickets réellement posés, dans la section
  « Bassin ready », et le renommage de son titre (qui porte encore « mode ombre
  — aucune écriture », faux dès que l'interrupteur s'ouvre).

Tracked in: senara-solutions/mika#2699 (vol 5 — AC3 pinné et la ligne de rapport
de ce qui a été posé)

---

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2699 :

- **AC1** : avec l'interrupteur OFF, aucune écriture (garantie structurelle avec
  contrôle négatif).
- **AC2** : tests du tri, des bornes 2 à 4, et du refus des mal formés.
- **AC3** : avec l'interrupteur ON, une seule mutation par décision, idempotente.
- **Taille** : à estimer tests compris, et à découper en vols au-delà de la borne.

Couverture par ce vol :

| AC | ce vol | ailleurs |
|---|---|---|
| AC1 | **complet.** Garantie structurelle = le scan de `no_dispatch_test.rs` borné à deux exceptions nommées + `armed: bool` en paramètre + sortie précoce `if !armed { return Vec::new() }`. Contrôle négatif = `…interrupteur_on_un_seul_appel_groupe_et_cest_le_bon_geste`, qui rougit si le module n'écrit jamais rien (motif mika#2205). Plus un test de cycle OFF. | — |
| AC2 | — | **déjà livré** : vols 1 (#2710), 2 (#2714) et 3 (#2800) — tri à deux critères, bornes 2–4 et hystérésis, les deux motifs de refus nommés. |
| AC3 | **première moitié** : exactement une invocation `gh` par ticket admis, les deux gestes groupés en drapeaux de cet appel, argv assérée position par position. | **seconde moitié au vol 5** : l'idempotence inter-cycles pinnée par un test de verdict réinjecté (`Tracked in:` ci-dessus). |
| Taille | **complet** : § *Taille estimée*, tests compris, découpage en vols déclaré. | — |

Le **seuil d'ouverture** du ticket (geste opérateur) n'est pas un AC de ce vol :
il demande que l'interrupteur de l'évaluation des vols soit ouvert et qu'aucun
`ready` mal formé n'ait été admis en ombre. Ce vol le rend *franchissable* ; il
ne le franchit pas.

---

## Definition of Done

- [ ] `pool_writer.rs` existe, déclare `ENV_POOL_WRITE`, `GESTE` plat, `pool_write_is_armed`, `armed_from_value`, `admit_pool_tickets(armed, …)` et `geste_args` dérivé.
- [ ] Le geste composé est **une** invocation : `issue edit <n> --repo <repo> --add-label ready --milestone "<titre>"`, et l'argv composé ne porte aucun jeton `--remove*` (asséré).
- [ ] `EXEMPTED_OCCURRENCES` porte exactement deux entrées ; `mika2699_lallowlist_porte_les_deux_gestes_et_pas_un_de_plus` pin les deux triplets, exige un `mika#` dans chaque raison, et assère pour chacune que le fichier existe et que sa production (commentaires dépouillés) contient le jeton.
- [ ] Le gabarit du scan est inchangé : `git diff origin/main -- crates/mika-agent/src/milestone_manager/no_dispatch_test.rs` ne touche ni `FORBIDDEN_TOKENS`, ni `EXEMPT_FILES`, ni `scan_source`, ni `walk_and_scan`.
- [ ] `mika2698_le_scan_rougit_sur_une_source_fautive`, `mika2698_une_exception_ne_couvre_quun_seul_jeton_du_fichier` et `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` sont verts sans modification.
- [ ] Le cycle de `cadence.rs` appelle le geste via `pool_write_is_armed()`, émet `manager_pool_admitted` / `manager_pool_not_admitted`, et **n'ajoute aucun champ** à `MilestoneCheckpoint`.
- [ ] `MIKA_MANAGER_POOL_WRITE` est déclaré commenté dans `.env.example` (garde T7 `sink_dir.rs:599` verte) et documenté dans `docs/operator/environment-variables.md`.
- [ ] Docstrings de `mod.rs` et `pool.rs` amendés dans l'ordre de commit ci-dessus — le geste et son exception dans le même commit.
- [ ] `cargo test -p mika-agent milestone_manager`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main` : verts.
- [ ] Le corps de PR nomme la taille réelle mesurée face aux 418 estimées (motif du vol 2 : la mesure se dit, elle ne se tait pas).

---

## Ce que ce vol ne fait pas

- **Il n'ouvre pas l'interrupteur.** Valeur par défaut OFF, et aucun chemin ne
  l'arme sans une variable d'environnement posée par un opérateur. L'ouverture
  met le manager à **alimenter la boucle autonome** (`ready` est le signal de
  consentement de dispatch, mika#841) : c'est pourquoi elle ne se fait que sur le
  mot de Vincent.
- **Il ne retire jamais `ready`,** ni ne détache un milestone — ces gestes ne
  sont pas exprimables, pas seulement interdits.
- **Il n'élargit pas le gabarit du scan.** L'exception est nommée ; le gabarit
  est intact.
- **Il n'ajoute aucun champ persisté.** L'idempotence est portée par la lecture
  du cycle suivant, pas par un checkpoint.
- **Il ne touche pas au rapport.** La section « Bassin ready (mode ombre —
  aucune écriture) » garde son titre à ce vol ; le vol 5 le corrige en même temps
  qu'il nomme ce qui a été posé.
