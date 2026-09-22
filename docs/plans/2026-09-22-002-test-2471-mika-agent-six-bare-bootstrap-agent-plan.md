# mika#2471 — six `bootstrap_agent()` de test dans `well_known_agents.rs` courent contre `MIKA_AGENT_TIER` ; les convertir et élargir la garde de mika#2073 au workspace

**Ticket :** senara-solutions/mika#2471
**Branche :** `test/2471/mika-agent-six-bare-bootstrap-agent`
**Type :** correctif de substrat de test (comportement de production inchangé)
**Base :** branche rebasée sur `main` @ `9aa39998` (la mesure initiale portait sur
`ff02386f`) — la PR compagnon #2441 (mika#2073) est mergée le 2026-09-22 10:20Z ;
`bootstrap_agent_with_tier` (`home.rs:404`) et la garde (`home.rs:2513`) sont sur
`main`. Le callout `Branch: test/2073/…` du corps initial pointait sur une branche
supprimée au merge ; il est remplacé par celui-ci. Toutes les ancres de ce plan
ont été re-vérifiées sur `9aa39998` — voir la note de révision en fin de document
pour les deux nombres descriptifs qui ont bougé.

---

## 1. Le défaut, et ce que la lecture du code confirme ou rectifie

### 1.1 Les six sites sont exactement où le ticket les place

Re-vérifié sur `ff02386f` — les lignes n'ont pas dérivé depuis `fb8c108f` :

| ligne | item | `#[test]` ? | ce qu'il assère après le bootstrap |
|---|---|---|---|
| `:2252` | `test_provision_skips_existing_agent` | nu | écrase `soul.md`, assère la conservation |
| `:2273` | `test_provision_partial_state` | nu | existence de `mika-qa` |
| `:3210` | `pre_seed_identity` (helper, 12 appelants) | **pas un item de test** | écrase `identity.toml` |
| `:3660` | `test_reconcile_config_toml_for_mika_dev` | nu | écrase `identity.toml` + `config.toml` |
| `:3681` | `test_reconcile_config_toml_idempotent` | nu | idem |
| `:3705` | `test_reconcile_config_skips_agents_without_spec_config` | nu | idem |

Aucun n'assère le contenu d'un gabarit : ce sont des **mines armées** (§1.4 du
plan de mika#2073), pas un second défaut vivant. C'est ce qui fixe la priorité à
`p2`, et c'est ce qui rend la conversion sans risque — aucune assertion ne change
de sens quand le tier devient explicite.

Le septième appel, `:927`, est dans `provision_agent` (production). Il **reste**
un lecteur d'environnement : c'est par lui que `MIKA_AGENT_TIER` atteint un agent
bien connu au premier démarrage (mika#1778). Le scan ne peut pas le voir de toute
façon — il n'est pas sous un attribut de test — mais la conversion ne doit pas le
toucher non plus.

### 1.2 Les poseurs, mesurés sur tout le workspace

Trois `set_var("MIKA_AGENT_TIER", …)` hors `home.rs`, tous sériels :

- `crates/mika-agent/src/server/mod.rs:1998` — `agent_state_tier_survives_env_drift`, `#[serial_test::serial]` (attribut `:1993`)
- `crates/mika-agent/src/server/tier_guard.rs:464` — `error_message_does_not_contradict_a_tier_the_env_disagrees_with`, `#[serial_test::serial]`
- `crates/mika-cli/src/commands/agents.rs:2143` — `mika2230_the_env_is_the_fallback_when_the_flag_is_absent`, `#[serial_test::serial]`

Les deux premiers partagent le binaire `mika-agent --lib` avec les cinq tests nus
du §1.1 : la course est la même que celle de mika#2073, à la ligne près. Le
troisième est dans `mika-cli` — un autre binaire, donc il ne court contre rien
aujourd'hui ; il est cité parce que le scan élargi le verra, et qu'il est sériel.

Les autres lecteurs `AgentTier::from_env()` en position de test —
`server/mod.rs:2001` et `:2009` — sont dans le test sériel ci-dessus. Tous les
autres appels du workspace (`mika-cli/commands/{setup,agents,ask,chat}.rs`,
`mika-agent/src/bin/mika-spirit.rs`, `tools/create_agent.rs`, `server/{mod,state}.rs`)
sont de production, hors de portée du scan par construction.

### 1.3 Le périmètre élargi, mesuré

`crates/` porte **611** fichiers `.rs` sur `9aa39998` (mika-agent 462, mika-cli 52,
mika-common 40, mika-gateway 40, mika-a2a 10, mika-os 7). Le walker du garde,
lancé sur les **610** que portait la tête de #2441 (`9d3214e9`), rend
`tests_seen=7836 offenders=5 desyncs=0` — mesure de l'opérateur, commentaire du
2026-09-22 02:22Z. Les cinq offenders sont exactement les cinq items de test du
§1.1 ; `:3210` est invisible parce que `pre_seed_identity` n'a pas d'attribut de
test.

Le `+1` fichier entre les deux commits est du bruit de `main`, pas un signal : il
ne change ni la population d'offenders (l'étape 7 la re-mesure) ni les planchers
du §2.3, qui sont dimensionnés à ~2/3 de la mesure précisément pour ne pas avoir
à suivre ce nombre commit par commit.

**Pourquoi inclure `tests/` et `src/bin/`.** Un test d'intégration compile dans
son propre binaire, donc un `#[test]` nu qui y lit le tier ne court contre aucun
poseur de `src/`. Le scan élargi le signalerait quand même — c'est plus strict que
la course réelle. C'est voulu : (a) le ticket dit « workspace », pas « les binaires
où un poseur existe » ; (b) le geste correctif (`_with_tier`) coûte zéro et rend
le test honnête sur le tier qu'il suppose ; (c) une exclusion par répertoire est
une allowlist déguisée, et l'allowlist est vide. Aujourd'hui, zéro site n'y
tombe (§1.2).

### 1.4 Ce que le walker doit changer, et ce qu'il ne doit pas

Le walker `scan_bare_tests_reading_the_tier(source)` (`home.rs:2389`) est
générique sur le texte, mais deux détails le bornent à un fichier :

1. **L'étiquette de l'offender est codée en dur** : `format!("  home.rs:{} in …")`
   (`home.rs:2473`). Élargi tel quel, il attribuerait un offender de
   `well_known_agents.rs` à `home.rs`. L'étiquette devient un paramètre.
2. **Le garde-fou « a-t-il lu ? »** est `scan.tests_seen >= 40` sur un seul
   fichier (`home.rs:2524`). Sur 611 fichiers ce nombre ne dit plus rien : un
   walker dont la racine a dérivé vers `crates/mika-agent` seul verrait encore
   ~6 000 items. Le garde-fou doit vérifier **la présence des deux fichiers
   ancres**, chacun avec son propre plancher, en plus d'un plancher global.

Ce qui ne change pas : `attribute_at`, `inner_is_test`, `inner_is_serial`,
`reads_the_tier_from_the_environment`, `brace_scan`, et la discipline des
desyncs (une faute, jamais une note). Le contrôle de bonne foi
`mika2073_the_guard_fires_on_a_relapse` continue d'appeler le walker sur sa source
fabriquée ; ses assertions sont des `.contains(<nom du test>)`, insensibles à
l'étiquette de chemin.

### 1.5 Deux branches du prédicat rencontrent enfin du source réel

`home.rs` n'a ni `#[tokio::test]` ni `#[serial_test::serial]` en position
d'attribut, donc ces deux branches n'étaient attestées que sur lignes fabriquées.
Le workspace porte **2 093** items `#[tokio::test…]` et **32** attributs
`#[serial_test::serial]` (dont les deux poseurs du §1.2). Après U1 le scan les
traverse tous — `desyncs=0` sur 7 836 items est la preuve que `attribute_at`
et `brace_scan` tiennent sur du rustfmt réel, y compris les attributs
multi-lignes de `tokio::test(flavor = …)`. Le `#[serial(clé)]` à clé reste sans
site réel (les quatre occurrences du workspace sont dans le contrôle fabriqué) ;
son attestation reste celle du contrôle, et le doc-comment du contrôle doit le
dire à jour.

### 1.6 `rust_sources_under` a zéro appelant

`source_guard::rust_sources_under(root)` (`source_guard.rs:932`) existe
précisément pour « la boucle que quinze gardes écrivent à l'identique » — et
n'a aujourd'hui aucun appelant hors de son module. Ce garde devient le premier ;
on n'écrit pas une seizième boucle.

---

## 2. Conception

### 2.1 L'ordre est le cœur du ticket : rouge d'abord

Le ticket l'exige et la raison est la même qu'en mika#2073 : un détecteur qui
démarre vert n'a jamais démontré qu'il voit. La séquence est donc, **dans un seul
commit final** mais avec une preuve intermédiaire :

1. U1 — élargir la garde. `cargo test -p mika-common --lib mika2073 -- --nocapture`
   doit être **rouge** avec exactement cinq offenders, tous dans
   `mika-agent/src/well_known_agents.rs`, et `desyncs=0`. Sortie collée
   verbatim dans le corps de PR (AC3).
2. U2 — convertir les six sites. Le même test passe au vert.
3. Commit unique des deux gestes (+ U3, U4). Le motif n'est pas le bisect —
   une PR qui atterrit en un commit ne fait jamais voir l'état rouge à `main` —
   mais la lettre du ticket : « deux gestes indissociables, dans cet ordre, au
   même commit ». Un commit où la garde est rouge est un commit inlivrable, et la
   preuve du rouge vit dans le corps de PR (AC3), pas dans l'historique.

### 2.2 Surface du walker

```rust
struct TierScan {
    offenders: Vec<String>,   // "  <label>:<n> in `<signature>` — <ligne>"
    tests_seen: usize,
    desyncs: Vec<String>,     // "  <label>:<n> in `<signature>` — …"
}

/// `label` est le chemin relatif à `crates/` (ex. `mika-common/src/home.rs`) ;
/// le contrôle de bonne foi passe `"fabricated.rs"`.
fn scan_bare_tests_reading_the_tier(label: &str, source: &str) -> TierScan
```

Le garde agrège :

```rust
let crates_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
let files = crate::source_guard::rust_sources_under(&crates_root);
let mut per_file: BTreeMap<String, usize> = BTreeMap::new();
let mut total = TierScan::default();
for path in &files {
    let label = path.strip_prefix(&crates_root).unwrap().display().to_string();
    let scan = scan_bare_tests_reading_the_tier(&label, &std::fs::read_to_string(path)?);
    per_file.insert(label, scan.tests_seen);
    total.absorb(scan);
}
```

`strip_prefix` sur un `..` non canonisé : canoniser `crates_root` une fois
(`std::fs::canonicalize`) avant le walk, sinon le préfixe ne correspond pas et
l'étiquette dégénère en chemin absolu — lisible, mais instable entre machines
et dans les assertions d'ancre ci-dessous.

### 2.3 Le garde-fou « a-t-il lu ? » adapté au workspace

Trois assertions, dans cet ordre, **avant** les desyncs et les offenders :

| assertion | plancher | mesure du 2026-09-22 | ce que son rouge dit |
|---|---|---|---|
| `per_file["mika-common/src/home.rs"] >= 40` | conservé de mika#2073 | 67 `#[test]` | la racine a dérivé, ou `home.rs` a été renommé |
| `per_file["mika-agent/src/well_known_agents.rs"] >= 40` | même plancher | 76 `#[test]` | le walk n'a pas atteint le crate où vivent les six sites |
| `files.len() >= 400 && total.tests_seen >= 4000` | ~2/3 de la mesure | 611 / ~7 836 | la racine pointe sur un sous-arbre |

Les deux ancres sont **load-bearing** : elles nomment les deux fichiers dont
l'histoire de cette garde est faite, et un walk qui ne les voit pas n'atteste
rien. Le plancher global est une seconde ligne — un futur découpage de
`well_known_agents.rs` ferait tirer l'ancre, et le geste serait alors de mettre
l'ancre à jour, pas de la retirer (message d'assertion à écrire dans ce sens).

Une ancre absente de `per_file` est un `None` → assertion rouge avec la liste
des cinq premiers chemins vus, pour que le message dise **d'où** le walk est
parti.

### 2.4 Message des offenders : le chemin, puis le geste

Le message d'échec de la garde (`home.rs:2548-2567`) ne change pas de fond ; il
gagne le chemin en tête de chaque ligne (déjà porté par `label`) et son
paragraphe `FIX:` nomme les trois variantes, pas seulement `bootstrap_with_tier`
— un offender de `well_known_agents.rs` doit lire
`bootstrap_agent_with_tier(home, name, AgentTier::Default)` dans le message qui
le désigne.

### 2.5 Ce que la conversion écrit dans `well_known_agents.rs`

`AgentTier` n'est pas importé dans ce fichier (zéro occurrence). Les six sites
deviennent, chemin complet, sans ajouter d'import au module de test :

```rust
mika_common::home::bootstrap_agent_with_tier(home, "mika-dev", mika_common::home::AgentTier::Default).unwrap();
```

`Default` est le tier que chacun de ces tests suppose : ils écrasent ou ignorent
les gabarits, et `MIKA_DEV_IDENTITY` / `MIKA_TEST_IDENTITY` sont des identités
opérateur. Aucun `#[serial]` n'est ajouté ; aucun n'est retiré — les poseurs du
§1.2 gardent le leur, ils posent réellement la variable.

Pour `pre_seed_identity` (`:3210`), un doc-comment d'une ligne dit pourquoi le
helper pose le tier : il est appelé depuis douze tests nus, et le poser ici les
couvre tous — c'est la limite « helper-wrapped » du scan, fermée à la main.

---

## Fire-Disposition

Inchangée de mika#2073 pour les trois détecteurs ; ce plan n'en ajoute aucun, il
élargit le premier.

| Détecteur | Ce que son rouge signifie | Disposition |
|---|---|---|
| `mika2073_no_bare_test_reads_the_tier_from_the_environment` — **périmètre workspace** | un test non sériel, **n'importe où sous `crates/`**, lit `MIKA_AGENT_TIER` via `bootstrap*` / `from_env` | **(c) halt-and-surface, allowlist vide** — passer le tier, jamais `#[serial]`, jamais une exclusion de répertoire |
| — ses trois assertions d'ancre (§2.3) | le walk n'a pas lu ce qu'il croit lire | **(c) halt-and-surface** — corriger la racine ou l'ancre renommée, jamais baisser le plancher |
| `mika2073_the_guard_fires_on_a_relapse` | le prédicat ou le walk est cassé | **(c) halt-and-surface** |

**Allowlist vide, et livrée vide.** Aucune constante d'exceptions, aucun filtre
de répertoire (§1.3). Une exclusion `tests/` serait une allowlist avec un autre
nom.

**Le sixième site n'est attesté par AUCUN détecteur, et c'est écrit ici plutôt
que découvert en revue (F2, première passe architecte).** `pre_seed_identity`
(`:3210`) n'a pas d'attribut de test, donc le scan ne le voit ni avant ni après
la conversion : la preuve rouge attend **cinq** offenders, et le vert d'après
U2 en attend **zéro** — ni l'un ni l'autre ne dit quoi que ce soit de `:3210`.
Ce qui l'atteste est donc, et seulement : (a) le vert de
`cargo test -p mika-agent --lib`, qui exerce les douze appelants du helper ;
(b) la **revue de diff**, qui compte six conversions et un `:927` intact ;
(c) le doc-comment posé sur le helper en U2 étape 9, qui **nomme** l'angle mort
au site plutôt que de le laisser au plan. Un angle mort connu se nomme, il ne se
tait pas (classe mika#2205).

**Ce qu'elle ne couvre toujours pas.** Elle lit du texte, pas un AST : un alias
(`use home::bootstrap_agent as b;`), un appel dont le nom et la `(` ne sont pas
sur la même ligne (que rustfmt n'émet pas, mais que rien n'interdit), ou un
**helper** sans attribut de test (`pre_seed_identity`, §1.1) lui échappent. La limite helper-wrapped reste
documentée dans `crates/mika-common/CLAUDE.md` ; ce ticket ferme son seul cas
réel à la main, il ne prétend pas la lever.

---

## Plan d'implémentation

### U1 — élargir la garde (`crates/mika-common/src/home.rs`)

1. `scan_bare_tests_reading_the_tier(label: &str, source: &str)` ; `label`
   remplace `home.rs` dans les lignes d'offender **et** de desync (§2.2).
2. Le garde walke `canonicalize(CARGO_MANIFEST_DIR/..)` via
   `source_guard::rust_sources_under`, agrège, et pose les trois assertions
   d'ancre (§2.3) avant les deux existantes.
3. Message `FIX:` nomme les trois variantes (§2.4).
4. Doc-comment du garde : le paragraphe **« Scope: this file only »** est
   réécrit — portée `crates/`, `tests/` inclus et pourquoi (§1.3), et ce qui
   échappe encore (Fire-Disposition, dernier paragraphe).
5. Doc-comment de `mika2073_the_guard_fires_on_a_relapse` : « three of the
   shapes below do not occur in the file actually scanned » devient vrai pour
   **une** seule (`#[serial(clé)]`) ; les deux autres sont désormais traversées
   sur du source réel (§1.5). Le contrôle n'est pas allégé pour autant.
6. Contrôle de bonne foi : l'appel `scan_bare_tests_reading_the_tier(&fabricated)`
   passe `"fabricated.rs"` ; aucune assertion ne change.
7. **Preuve rouge** : `cargo test -p mika-common --lib mika2073 -- --nocapture 2>&1 | tee /tmp/mika2471-red.txt`
   → attendu : `mika2073_the_guard_fires_on_a_relapse` vert,
   `mika2073_no_bare_test_reads_the_tier_from_the_environment` **rouge**,
   cinq offenders sous `mika-agent/src/well_known_agents.rs` (`:2252`, `:2273`,
   `:3660`, `:3681`, `:3705`), `desyncs` vide, ancres satisfaites. Si le compte
   n'est pas cinq, ou si un desync apparaît, **s'arrêter et lire** : c'est soit
   une dérive de `main` depuis la mesure du §1.3, soit une faute du walk — les
   deux se distinguent à la ligne citée.

### U2 — convertir les six sites (`crates/mika-agent/src/well_known_agents.rs`)

8. Les six appels du §1.1 → `bootstrap_agent_with_tier(…, AgentTier::Default)`
   (§2.5). `:927` intact — le diff de ce fichier ne touche que le module `tests`.
9. Doc-comment sur `pre_seed_identity` : pourquoi le helper pose le tier (douze
   appelants nus couverts d'un coup) **et** qu'il est hors de portée du scan par
   construction — c'est l'attestation (c) de la Fire-Disposition.
10. Le même test qu'en 7 passe au vert ; `cargo test -p mika-agent --lib` vert.

### U3 — documentation (`crates/mika-common/CLAUDE.md`)

11. Dans le paragraphe **Home directory** : retirer « **Its scope is
    `crates/mika-common/src/home.rs` only** … » et la mention du ticket de
    suivi mika#2471 ; dire que la portée est `crates/` entier, que les deux
    ancres sont le garde-fou, et garder la limite « helper-wrapped / alias /
    multi-ligne » telle quelle.

### U4 — preuve et commit

12. Corps de PR : sortie de l'étape 7 **verbatim** (AC3), et la phrase « `:3210`
    est le corps d'un helper, hors de portée du scan par construction, converti
    au même geste ».
13. `cargo test -p mika-common --lib` et `cargo test -p mika-agent --lib` verts ;
    `cargo clippy --all-targets` et `cargo fmt --check` propres.
14. Un seul commit portant U1+U2+U3 (§2.1).

---

## Definition of Done

- [ ] Les six sites du §1.1 passent `AgentTier::Default` ; `:927` est inchangé ;
      aucun `#[serial]` ajouté ni retiré.
- [ ] La garde walke `crates/` via `rust_sources_under`, étiquette chaque
      offender par son chemin relatif, et pose les trois assertions d'ancre du
      §2.3 ; allowlist vide, aucun filtre de répertoire.
- [ ] La garde a été **vue rouge** sur les cinq items de test avant U2, sortie
      collée verbatim dans le corps de PR, `desyncs=0`.
- [ ] Le sixième site (`:3210`) est attesté par les trois voies de la
      Fire-Disposition — vert `-p mika-agent --lib`, compte de six conversions au
      diff, doc-comment nommant l'angle mort — et par aucun détecteur.
- [ ] Le contrôle de bonne foi est vert et inchangé dans ses assertions.
- [ ] Les doc-comments du garde et du contrôle disent la portée réelle (§1.5,
      Fire-Disposition).
- [ ] `crates/mika-common/CLAUDE.md` ne mentionne plus la portée bornée ni
      mika#2471 comme suivi.
- [ ] `cargo test -p mika-common --lib` et `cargo test -p mika-agent --lib`
      verts ; clippy + fmt propres.
- [ ] Un commit ; comportement de production inchangé, et le corps de PR le dit.

---

## Acceptance criteria

Transcrits verbatim du corps de mika#2471 (état du 2026-09-22 après correction
AC3 par le groomeur), chacun lié à l'unité qui le livre — la section existe et
est non vide, ce que la première passe architecte ne pouvait pas vérifier
(`gh_read` ne lit pas les fichiers du dépôt) et a soulevé en F1.

- **AC1** — Les six appels de test de `well_known_agents.rs` passent le tier
  explicitement. `:927` (production) est inchangé. → U2, étapes 8-9.
- **AC2** — Le scan de `mika2073_no_bare_test_reads_the_tier_from_the_environment`
  couvre le workspace, allowlist **vide**, disposition halt-and-surface inchangée.
  Son garde-fou « le scan a-t-il vraiment lu quelque chose ? » est adapté au
  nouveau périmètre. → U1, étapes 1-3 ; §2.3.
- **AC3** — Le scan élargi a été **vu rouge** sur les **cinq** items de test
  (`:2252`, `:2273`, `:3660`, `:3681`, `:3705`) avant leur conversion, et la PR le
  dit (sortie collée verbatim). Le sixième site, `:3210`, est le corps du helper
  `pre_seed_identity` : sans attribut `#[test]`, il est hors de portée du scan par
  construction — il est converti au même geste, pas « vu rouge ». → U1 étape 7,
  U4 étape 12.
- **AC4** — `crates/mika-common/CLAUDE.md` retire la mention du périmètre borné à
  `home.rs` et du présent ticket de suivi. → U3.
- **AC5** — `cargo test -p mika-agent --lib` et `cargo test -p mika-common --lib`
  verts. → U4 étape 13.

---

## Hors périmètre

- Le site de production `:927`, et les appels de production de `mika-cli`,
  `mika-spirit`, `create_agent.rs` (§1.2) — lecteurs d'environnement légitimes.
- `MIKA_HOME` et `MIKA_DEPLOYMENT` : même classe en principe, aucun défaut
  mesuré ; généraliser depuis un point n'est pas un geste.
- Lever la limite helper-wrapped / alias / multi-ligne du scan (passer à un AST).
  Ce ticket ferme le seul cas réel connu à la main.
- Faire tourner `rust_sources_under` chez les quinze autres gardes qui écrivent
  la boucle — hygiène séparée, sans lien avec la course.
- **Tout carve-out de répertoire, `benches/` compris.** La mesure de 611 fichiers
  les inclut, « aucun filtre de répertoire » les couvre, et en exclure un serait
  la même allowlist déguisée que pour `tests/` (§1.3).

## Risques

- **Un desync sur un des 611 fichiers.** Mesuré à zéro sur `9d3214e9` ; `main` a
  depuis avancé jusqu'à `9aa39998` sans toucher un fichier à raw string
  inhabituel, mais l'étape 7 le re-mesure. Si un desync
  apparaît, c'est une faute du walk (probablement une forme de littéral que
  `source_guard::scan_line` ne porte pas) : la réparer dans `source_guard`, pas
  exclure le fichier.
- **Coût du garde.** 611 lectures + ~7 836 walks de corps par exécution de
  `cargo test -p mika-common --lib`. Les gardes voisins (`budget_provenance`,
  `agent_loop::mika2247_…`) font déjà des walks de crate ; mesurer une fois le
  temps du test à l'étape 10 et le noter dans le corps de PR — si > 2 s, le dire,
  ne pas optimiser dans cette PR.
- **`canonicalize` sur un checkout via symlink** (worktrees sous `.claude/`) :
  les chemins de `rust_sources_under` sont construits depuis la racine canonisée,
  donc le `strip_prefix` tient. Ne pas canoniser chaque fichier séparément.

## Revision history

- 2026-09-22 — plan initial (groom sur `main` @ `ff02386f`, après merge de #2441).
- 2026-09-22 — première passe mika-arch (`c84b4c7e`, ITERATE) : les cinq
  incertitudes endossées telles quelles (garde-fou à deux ancres, inclusion de
  `tests/` et `src/bin/`, signature `(label, source)`, mesurer-sans-optimiser,
  racine canonisée). F1 (section AC non attestée) : la section existait déjà —
  l'attestation est rendue explicite plutôt que la section ajoutée. F2
  (attestation du sixième site) : appliqué en Fire-Disposition, U2 étape 9 et
  DoD. Deux lectures de cohérence de l'architecte appliquées : pas de carve-out
  `benches/` (hors périmètre), et la justification du commit unique corrigée
  (la lettre du ticket, pas le bisect).
- 2026-09-22 — re-vérification des ancres après rebase de la branche sur `main`
  @ `9aa39998`. **Rien de décisionnel n'a bougé** : les six sites sont aux mêmes
  lignes (`:2252`, `:2273`, `:3210`, `:3660`, `:3681`, `:3705`), `:927` reste le
  seul appel de production, `bootstrap_agent_with_tier` est à `home.rs:404`, la
  garde à `:2513`, le walker à `:2389` avec son étiquette `home.rs` codée en dur
  et son plancher `tests_seen >= 40` sur un fichier, `rust_sources_under`
  (`source_guard.rs:932`) n'a toujours aucun appelant, les deux fichiers-ancres
  portent 67 et 76 `#[test]`, et la cible d'AC4 est bien en place dans
  `crates/mika-common/CLAUDE.md` (« **Its scope is `crates/mika-common/src/home.rs`
  only** » + la mention de mika#2471 comme suivi, toutes deux sur la même ligne).
  Trois nombres **descriptifs** rafraîchis : le compte de fichiers (610 → 611),
  le poseur de `server/mod.rs` (`:1993` → `:1998`, son attribut sériel restant à
  `:1993`) et ses deux lecteurs `from_env` voisins (`:1996`/`:2004` →
  `:2001`/`:2009`). Aucun n'est load-bearing — les planchers du §2.3 sont à 400 et
  4 000 —, mais un plan qui cite une ligne doit citer la bonne.
