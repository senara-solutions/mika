# mika#2511 — Tenir le verrou pendant la suppression, et découpler le budget de purge

**Ticket :** `senara-solutions/mika#2511` (enfant de mika#2491)
**Cible :** `crates/mika-agent/src/worktree_reaper.rs` (bras de purge `target/`, mika#2497 / PR #2510)
**Type :** fix substrat, deux bloquants + une veille

---

## 0. Ce que la lecture du code déplace dans le ticket

Trois rectifications, toutes vérifiées sur l'arbre à `bde39c24`. Aucune ne change
le remède ; deux changent la façon de le présenter, la troisième ajoute un
bénéfice que le ticket n'a pas vu.

### R1 — La fenêtre TOCTOU n'est pas uniformément « large ». Elle est bornée pour le premier candidat et longue pour le second.

Le ticket écrit « un `measure_tree_size` (:2603) sur un arbre de dizaines de Go —
fenêtre réelle et large ». `measure_tree_size` est **budgété** :
`SIZE_MAX_ENTRIES = 400_000`, `SIZE_TIME_BUDGET = 2_000 ms`
(`worktree_reaper.rs:923-925`), et une marche tronquée rend un minorant étiqueté
plutôt que de continuer. Donc pour le **premier** candidat la fenêtre est de
l'ordre de 2 s + quelques `.await` d'écriture SQLite.

Ce qui rend la fenêtre longue est ailleurs, et c'est exactement ce que le ticket
signale une phrase plus loin : avec `PURGE_MAX_PER_TICK_DEFAULT = 2` (:2014), le
**second** candidat a devant lui le `remove_dir_all` complet du premier — des
dizaines de secondes sur 40 Go et des centaines de milliers d'inodes. La fenêtre
du second candidat est donc de l'ordre d'une suppression entière.

**Conséquence pour le plan :** le remède est inchangé, mais la mesure attendue de
sa surface opérateur l'est. Un `build_lock_raced` sera rare, et beaucoup plus
probable sur le second candidat d'un tick que sur le premier. C'est ce qui
interdit de lire un compte nul comme « le défaut n'existait pas » (halte H3 § 8).

### R2 — Tenir le `flock` protège jusqu'à l'unlink du fichier de verrou par la suppression elle-même, et pas au-delà.

Le DoD dit « tenir le `flock` sur chaque `.cargo-lock` pendant le
`remove_dir_all` (le relâcher après) ». Livrable, et c'est ce que fait ce plan.
Mais la propriété obtenue doit être écrite exactement, parce qu'un futur lecteur
qui la croit totale prendra une mauvaise décision :

`flock(2)` porte sur une **open file description**, donc sur l'inode. Le
`remove_dir_all` supprime `<target>/<profil>/.cargo-lock` en cours de route. Une
fois cet unlink passé, un `cargo` qui démarre **crée un nouvel inode** au même
chemin et prend un `flock` dessus sans jamais rencontrer le nôtre. Le verrou tenu
couvre donc :

| fenêtre | avant mika#2511 | après |
|---|---|---|
| sonde → début de la suppression | **non protégée** (2 s à plusieurs dizaines de s) | protégée par l'acquisition tenue |
| début de la suppression → unlink du `.cargo-lock` | non protégée | protégée |
| unlink du `.cargo-lock` → fin de la suppression | non protégée | **toujours non protégée** |

Le résidu est réel et il est nommé dans le doc-comment de l'acquisition. Il est
acceptable pour la raison que mika#2497 a déjà écrite comme fondement de tout le
bras : *un faux positif coûte du temps de rebuild, jamais une perte* — et un
`cargo` qui démarre dans la seconde moitié d'un `remove_dir_all` est un build de
quelques secondes, pas un build de vingt minutes.

**Une option a été examinée et écartée : `rename(target, target.mika-purge-<n>)`
avant de supprimer.** Elle ramènerait la fenêtre à quelques microsecondes (le
`rename` est atomique, un `cargo` qui démarre ensuite recrée un `target/` neuf et
travaille dedans). Elle est refusée pour un mode de panne qu'elle crée et que ce
ticket ne peut pas absorber : `target.mika-purge-*` n'est couvert par aucun
`.gitignore`, donc `git status --porcelain` le liste `??`, donc **T7 du faucheur
mika#2420 lit le worktree `dirty` et refuse de le retirer**. Un processus mort
entre le `rename` et la fin de la suppression laisse alors un orphelin de 40 Go
dans un worktree devenu non-retirable — c'est-à-dire précisément le problème que
tout ce bras existe pour résoudre, aggravé. Le refermer demanderait un balayage
des orphelins plus haut que la population `pr_open`, soit un mécanisme nouveau
avec ses propres modes de panne dans un ticket de réparation. **Suivi nommé, non
ouvert** (§ 9), précondition : que la sonde S2 montre une population
`build_lock_raced` non négligeable.

### R3 — Le bloquant (b) a un troisième effet que le ticket ne nomme pas : il coupe aussi la sonde de saleté de mika#2449.

Le `break` de `worktree_reaper.rs:1307` est en **tête** du corps de boucle, donc
il saute tout ce qui suit pour les dépôts restants — y compris
`probe_main_checkout` (:1331, mika#2449 U3), la sonde qui détecte un checkout
principal sali par `run_shell`. Ce n'est pas une inertie théorique : la sonde est
la seule chose qui *date* la prochaine occurrence de cette classe, et sans date
la requête d'attribution sur `tool_calls` n'a pas de bornes.

Corriger (b) restaure cette sonde sur les dépôts suivants. C'est un bénéfice
collatéral à nommer, pas un effet de bord à cacher.

### R4 — Ce que le ticket a vu juste, et qui est confirmé au code

- **(a)** `probe_one_cargo_lock` : `flock(LOCK_EX|LOCK_NB)` :2403, `flock(LOCK_UN)`
  :2408, `return LockProbe::Free` :2409. Sondes en amont pour tous les candidats
  :2566-2573, `apply_lock_probes` :2574, boucle de disposition :2580,
  `target_path_is_disposable` re-vérifié :2586, `remove_dir_all` :2608 **sans
  re-sonde**. Confirmé.
- **(b)** `if budget == 0 { break; }` :1307 sur le budget du faucheur (:1298),
  `purge_stale_target_dirs` :1493 en fin de corps avec `purge_budget` distinct
  (:1299). Confirmé.
- **(c)** `Disposition::Observe => true` :2607 puis `stats.purged += 1` :2622
  inconditionnel, lu par le log d'agrégat :1511. Confirmé, et le défaut est bien
  borné au compteur en mémoire : la surface d'audit durable est correctement
  séparée par `purge_outcome_for` (:1985) et épinglée par
  `mika2497_v5_observe_ne_supprime_rien` (:4611).

---

## 1. Requirements

**Bloquants (le ticket les conditionne l'un à l'autre pour l'armement).**

- **B1** — La disposition ne supprime un `target/` qu'après avoir **acquis et
  retenu** le `flock` de chaque `.cargo-lock` de cet arbre, et ne le relâche
  qu'après la suppression.
- **B2** — L'échec de cette acquisition **conserve** l'arbre, et le dit sous un
  motif comptable qui le distingue du refus produit par le filtre amont.
- **B3** — Le budget du faucheur n'empêche plus la purge de tourner sur les
  dépôts suivants, ni la sonde mika#2449 (R3).
- **B4** — Le budget de purge et le kill-switch de purge continuent de faire
  cesser la boucle quand ils sont, eux, épuisés ou désarmés — la correction de B3
  ne doit pas transformer le `break` en boucle qui tourne pour rien.

**Veille (c), incluse parce que son coût marginal est nul et son piège non trivial.**

- **B5** — `stats.purged` ne compte que les suppressions **effectives** ; la
  population `observe` est comptée sous un nom distinct.
- **B6** — Le log d'agrégat `target_purge_tick` **continue d'être émis en
  `observe`**. C'est le piège central de (c) : la condition d'émission actuelle
  (:1508) est `purged > 0 || failed > 0`, donc corriger B5 seul rendrait le tick
  muet dans le mode même que la sonde S0 de mika#2497 prescrit d'utiliser en
  premier — une régression d'observabilité introduite par un correctif
  d'observabilité.

**Non-régressions explicitement exigées.**

- **N1** — Aucune valeur de réglage ne bouge : `PURGE_IDLE_DEFAULT_SECS` (14 400),
  `PURGE_MAX_PER_TICK_DEFAULT` (2), `TARGET_MTIME_SCAN_DEPTH` (2),
  `SIZE_MAX_ENTRIES`, `SIZE_TIME_BUDGET`, ni aucun défaut du faucheur.
- **N2** — Le sens des cinq termes P1–P5 est inchangé. La veille (d) demande de
  surveiller si le remède (a) modifie la sonde : **il ne modifie ni
  `newest_mtime_bounded`, ni `TARGET_MTIME_SCAN_DEPTH`, ni la fenêtre d'inactivité,
  ni `inspect_target_dir`.** `cargo_build_lock_is_free` garde sa sémantique de
  filtre et ses trois états.
- **N3** — Aucune variable d'environnement créée, aucun kill-switch ajouté.
- **N4** — Les deux populations d'audit du bras (`target_purged`,
  `target_purge_would_dispose`) et les clés (`purged_audit_key`,
  `purge_refusal_audit_key`) sont inchangées.
- **N5** — Hors Linux, la purge continue de ne jamais firer (l'acquisition rend
  `Unevaluable`, comme la sonde amont aujourd'hui).

---

## 2. Conception — bloquant (a) : l'acquisition tenue

### 2.1 Un seul énumérateur de profils, consommé par les deux étages

`cargo_build_lock_is_free` (:2442) énumère aujourd'hui les enfants directs de
`target/` et teste `<enfant>/.cargo-lock`. L'acquisition doit énumérer **la même
chose**, faute de quoi le filtre amont pourrait voir un profil que l'acquisition
ne verrouille pas — un trou silencieux, et exactement le défaut que le ticket
ferme, reproduit un cran plus bas.

Extraction :

```rust
/// Les `.cargo-lock` d'un `target/`, **découverts et jamais devinés**.
///
/// Rend `Err(())` quand l'énumération elle-même a échoué (le répertoire n'est pas
/// lisible) : la population est alors inconnue, jamais vide. Le `bool` dit qu'au
/// moins une entrée n'a pas pu être inspectée — le terme est alors inévaluable
/// même si les entrées lues sont libres.
#[cfg(target_os = "linux")]
fn cargo_lock_paths(target: &Path) -> Result<(Vec<PathBuf>, bool), ()>
```

`cargo_build_lock_is_free` et `acquire_cargo_build_locks` l'appellent toutes
deux. Aucun changement de sémantique pour la première : `Err(())` →
`Unevaluable`, `bool` vrai → `Unevaluable` si aucun verrou tenu n'a été trouvé,
`Held` l'emportant toujours sur un inévaluable (comportement actuel, :2467-2472).

### 2.2 Le garde RAII

```rust
/// Les verrous de build **tenus**, relâchés au `Drop`.
#[must_use = "relâcher le garde avant la suppression rouvre la fenêtre mika#2511"]
pub struct CargoBuildLockGuard { /* Vec<File> sous Linux, () ailleurs */ }

/// Ce qu'une tentative d'acquisition a pu dire.
pub enum LockAcquisition {
    /// Tous les verrous sont à nous, et le restent tant que le garde vit.
    Acquired(CargoBuildLockGuard),
    /// Au moins un verrou est tenu par un `cargo`.
    Held,
    /// On n'a pas pu regarder. **Conserve.**
    Unevaluable,
}

pub fn acquire_cargo_build_locks(target: &Path) -> LockAcquisition
```

Trois propriétés, chacune avec sa raison :

1. **Le `File` est retenu, pas relâché.** `flock` est relâché par la fermeture du
   descripteur ; garder le `File` vivant est donc la totalité du mécanisme. Un
   `impl Drop` qui fait le `LOCK_UN` explicite est ajouté malgré cela, pour la
   raison que le code dit déjà à son site actuel (:2405-2407) : « la fermeture le
   relâcherait de toute façon ; le dire explicitement rend l'intention lisible ».
2. **Un seul verrou tenu annule toute l'acquisition**, et les descripteurs déjà
   acquis sont relâchés par le `drop` du `Vec` partiel. Pas de verrou orphelin.
3. **`Unevaluable` conserve**, comme partout ailleurs dans ce bras : *un signal
   qu'on ne peut pas lire n'est jamais un terme satisfait*. Hors Linux le garde
   est un type vide et l'acquisition rend `Unevaluable` — la purge n'y fire
   jamais, ce qui est déjà le cas aujourd'hui (N5).

### 2.3 L'ordre de la disposition

Remplacement du corps de `for candidate in selection.candidates` (:2580-2664) :

```
if budget == 0 { break }

if !target_path_is_disposable(target) { refus outside_managed_root ; continue }

let guard = match acquire_cargo_build_locks(target) {
    LockAcquisition::Held        => { refus build_lock_raced        ; continue }
    LockAcquisition::Unevaluable => { refus build_lock_unreadable   ; continue }
    LockAcquisition::Acquired(g) => g,
};

budget -= 1;                        // ← après l'acquisition (§ 2.4)
let size = measure_tree_size(target);   // ← sous verrou (§ 2.5)

let removed = match cfg.disposition {
    Disposition::Observe => true,
    Disposition::Armed   => std::fs::remove_dir_all(target).is_ok(),
};
drop(guard);                        // ← explicite, après la suppression

… (le reste inchangé : `failed`, compteurs, log, `record_purged`)
```

**L'acquisition est la dernière chose avant la suppression, et il n'y a entre
elles ni `.await`, ni appel réseau, ni opération non bornée.** C'est la propriété
que le scan structurel du § 6 existe pour tenir.

**L'acquisition a lieu dans les deux dispositions, `observe` comprise.** Sans
cela `observe` rendrait une population *plus large* que ce que `armed` retirerait,
et la sonde S0 de mika#2497 — « commencer en `observe` et lire la population qui
serait retirée » — mentirait sur son propre objet.

### 2.4 Le débit du budget se déplace après l'acquisition

Aujourd'hui `budget -= 1` (:2603) précède la mesure et la suppression, mais suit
le refus `outside_managed_root` (:2586-2601, qui `continue` sans débiter). Le cap
est documenté comme « plafond d'écritures par tick » (:2031-2033, leçon
mika#2347 : *un cap sur les écritures, jamais sur les sauts*). Un refus tardif
n'écrit rien : il ne doit pas débiter. Le déplacement rend donc les deux refus
tardifs cohérents entre eux et cohérents avec la doctrine.

Conséquence assumée : un tick où deux candidats sont refusés à l'acquisition peut
en tenter un troisième. C'est voulu — l'acquisition est bon marché (quelques
`open` + `flock` non bloquants) et le cap borne les **tempêtes d'E/S**, qui
viennent des suppressions.

Contrôle négatif : `mika2497_v1_…` asserte `budget == 1` après une purge réussie
(:4197) — inchangé.

### 2.5 La mesure passe sous verrou

`measure_tree_size` après l'acquisition plutôt qu'avant : la mesure porte alors
sur un arbre qu'aucun `cargo` ne peut plus étendre, et ses 2 s de budget sortent
de la fenêtre non protégée au lieu d'y entrer. Strictement meilleur sur les deux
axes, coût nul.

### 2.6 Le motif de refus, et pourquoi il est distinct

`PURGE_REASON_BUILD_LOCK_RACED = "build_lock_raced"`, ajouté **en queue** de
`ALL_PURGE_REFUSAL_REASONS`.

Un motif distinct plutôt que la réutilisation de `build_lock_held` : ce refus est
la **mesure de la fenêtre que ce ticket ferme**. Chaque ligne dit « le verrou
était libre à la sonde et tenu à l'acquisition », c'est-à-dire une suppression que
l'état d'avant mika#2511 aurait laissé passer sur un arbre en cours de build.
Fusionner les deux populations rendrait cette mesure incomptable, et la clé de
dédup `purge_refusal_audit_key(chemin, motif)` porte déjà le motif, donc les deux
restent soustractibles sans autre changement.

`build_lock_unreadable` est en revanche **réutilisé** pour l'acquisition
inévaluable : le remède opérateur est identique aux deux étages (« pourquoi ce
`target/` n'est-il pas lisible ? »), et un motif de plus alourdirait un format de
fil sans rien trancher. Décision, pas oubli.

**Rupture de format de fil, datée :** `ALL_PURGE_REFUSAL_REASONS` est figé par
`mika2497_les_motifs_de_purge_sont_un_format_de_fil` (:4853), dont le message
dit « renommer un motif est une rupture de format de fil : la dater dans
CLAUDE.md, jamais mettre ce test à jour en silence ». Il s'agit ici d'un **ajout
en queue**, pas d'un renommage : aucune population existante ne change de nom ni
de sens, et les `GROUP BY` publiés restent exacts. La mise à jour du test est
accompagnée de son entrée CLAUDE.md (§ 7).

---

## 3. Conception — bloquant (b) : découpler la boucle des dépôts

### 3.1 Prédicat pur

```rust
/// Quand la boucle des dépôts peut cesser.
///
/// Les deux bras ont des budgets distincts (mika#2497) ; casser sur celui du
/// faucheur seul prive la purge — **et la sonde de saleté mika#2449** — de tous
/// les dépôts suivants. Le terme de la purge intègre son kill-switch, sans quoi
/// un bras désarmé garderait la boucle vivante pour rien.
pub fn should_stop_repo_loop(
    reaper_budget: usize,
    purge_budget: usize,
    purge_enabled: bool,
) -> bool {
    reaper_budget == 0 && (purge_budget == 0 || !purge_enabled)
}
```

Un prédicat pur nommé plutôt qu'une conjonction en ligne : il est testable à ses
quatre coins sans monter de dépôt factice, et il est l'endroit où le raisonnement
est écrit.

### 3.2 Le corps de boucle

- Ligne 1307 : `if budget == 0 { break }` → `if should_stop_repo_loop(budget, purge_budget, purge_cfg.enabled) { break }`.
- Le bloc T7 + disposition du faucheur (:1394-1485) est enveloppé dans
  `if budget > 0 { … }`. Sans cela, un budget faucheur épuisé ferait payer deux
  `git` par candidat (`collect_work_state`, :1397) pour une boucle de disposition
  qui casserait aussitôt.
- Tout ce qui **précède** ce bloc reste inconditionnel : `probe_main_checkout`
  (R3), le registre, le remote, `list_prs`, `screen_worktrees` et l'écriture de
  ses refus. Ce n'est pas du travail gaspillé — `screened.refusals` et
  `prs_by_branch` sont exactement les deux entrées dont la purge a besoin
  (:1490-1498).
- Le `if budget == 0 { break }` interne à la boucle de disposition (:1407) est
  conservé : il sert quand le budget s'épuise **en cours** de dépôt.

**Ce qui ne change pas :** l'ordre purge-après-faucheur (:1487-1492) est
inchangé, et sa raison le reste — ce que le faucheur vient de retirer n'existe
plus.

---

## 4. Conception — veille (c) : `would_purge`

```rust
struct TargetPurgeStats {
    purged: usize,       // suppressions EFFECTIVES (armed)
    would_purge: usize,  // éligibles non retirés (observe)
    failed: usize,
    refused: usize,
    bytes: u64,
}
```

- Le bras `Observe` incrémente `would_purge`, le bras `Armed` incrémente
  `purged`. La dérivation suit la disposition au même endroit que
  `purge_outcome_for`, qui est déjà la source unique du triplet
  (event, tool_name, message).
- **B6 — la condition d'émission du log d'agrégat devient**
  `purged > 0 || would_purge > 0 || failed > 0`, et le log gagne un champ
  `would_purge`. C'est la moitié non triviale de (c) : sans elle, corriger le
  compteur rend `target_purge_tick` muet en `observe`.
- `bytes` continue d'agréger dans les deux dispositions : c'est la taille que la
  purge *rendrait*, et c'est le chiffre que la sonde S0 lit pour dimensionner.

**Test à mettre à jour, avec sa raison écrite :**
`mika2497_v5_observe_ne_supprime_rien` (:4639) asserte `stats.purged == 1` en
`observe` — l'assertion **fige le défaut**. Elle devient
`stats.would_purge == 1 && stats.purged == 0`. C'est une correction, pas un
assouplissement : la moitié durable de ce test (`target_purge_would_dispose`
écrit, `target_purged` absent) est inchangée et reste l'assertion porteuse.

**Le bras faucheur porte le même motif** (`disposed += 1` en `observe`, :1459) et
**n'est pas touché** : il appartient à mika#2420/mika#2469, et changer
`worktree_reap_tick.disposed` déplacerait une surface opérateur d'un autre ticket
dans un ticket de réparation de celui-ci. Le ticket le classe en observation, pas
en demande. **Suivi nommé** (§ 9).

---

## 5. Contrat de vérification

### 5.1 Ce qui est attestable, et par quoi

| # | Propriété | Forme |
|---|---|---|
| V1 | `acquire_cargo_build_locks` rend `Held` quand un `.cargo-lock` est tenu | unité, verrou pris dans le test |
| V2 | Elle rend `Free`/`Acquired` quand aucun `.cargo-lock` n'existe | unité |
| V3 | Elle rend `Unevaluable` quand `target/` n'est pas lisible | unité |
| V4 | **Le garde tient réellement** : tant qu'il vit, une seconde acquisition rend `Held` ; après `drop`, elle réussit | unité, déterministe |
| V5 | Un verrou dans un profil `release` est vu (pas seulement `debug`) | unité |
| V6 | L'acquisition annulée par un verrou tenu ne laisse aucun descripteur verrouillé | unité (une acquisition ultérieure réussit) |
| V7 | Bout en bout : un `target/` dont le verrou est tenu n'est **pas** supprimé, et un refus est écrit | `#[tokio::test]` sur `purge_stale_target_dirs`, patron :4164 |
| V8 | `should_stop_repo_loop` à ses quatre coins, kill-switch inclus | unité, table |
| V9 | `observe` incrémente `would_purge` et **pas** `purged` | `#[tokio::test]`, V5 corrigé |
| V10 | `armed` incrémente `purged` et **pas** `would_purge` | `#[tokio::test]`, contrôle négatif de V9 |
| V11 | `ALL_PURGE_REFUSAL_REASONS` contient le nouveau motif, en queue, sans doublon | unité existante mise à jour |

**V4 est le test porteur du ticket.** Il est déterministe et sans course :
`flock` porte sur l'*open file description*, donc deux `open()` du même chemin
dans le **même** processus obtiennent deux OFD distinctes et la seconde
acquisition `LOCK_EX|LOCK_NB` échoue bien avec `EWOULDBLOCK`. Aucun thread,
aucun `fork`, aucune temporisation.

### 5.2 Ce qui n'est PAS testable ici, écrit plutôt que découvert

**« La re-sonde mord » n'est pas attestable de bout en bout.** Pour que le refus
`build_lock_raced` se produise dans un test, il faudrait qu'un `cargo` prenne le
verrou **entre** le filtre amont (:2566-2573, appelé depuis
`purge_stale_target_dirs` elle-même, donc incontournable par un appelant) et
l'acquisition. Un verrou pris avant l'appel est intercepté par le filtre amont et
produit `build_lock_held` ; la re-sonde n'est jamais atteinte. Fabriquer la
course demanderait un point d'injection dans la fonction de production, c'est-à-dire
d'ajouter au substrat un crochet dont le seul consommateur serait un test.

Ce qui est attesté à la place : l'**unité** (V1–V6, dont la propriété RAII), le
**bout en bout du conservatisme** (V7), et le **scan structurel** du § 6, qui est
la seule garde capable de voir la régression « quelqu'un retire l'acquisition ».
C'est précisément pourquoi le scan n'est pas de la cérémonie ici.

### 5.3 Pré-vol (à exécuter avant de livrer le scan)

Le scan du § 6 est livré avec une **allowlist vide**. Ce n'est légitime que s'il
n'existe aucune violation préexistante. Vérification, à faire passer avant de
figer l'allowlist :

```bash
grep -n "remove_dir_all" crates/mika-agent/src/worktree_reaper.rs
```

Attendu au moment de la rédaction : un seul site exécutable (:2608), plus trois
occurrences en prose (:2012, :2617, :4162). Si un second site exécutable
apparaît, **halte-et-remontée** plutôt qu'une entrée d'allowlist : savoir si ce
site doit acquérir le verrou est une question que le scan ne peut pas trancher.

---

## 6. Fire-Disposition

Ce plan livre **un détecteur** : le scan structurel
`mika2511_toute_suppression_est_precedee_de_lacquisition`.

**Ce qu'il fait.** Un scan de source sur `worktree_reaper.rs` (et sur lui seul —
`src/` compte 39 `remove_dir_all`, dont aucun n'appartient à ce bras) : tout
`std::fs::remove_dir_all` en position exécutable doit être précédé, **dans la
même fonction**, d'un appel à `acquire_cargo_build_locks`. Les lignes de
commentaire et de doc sont retirées avant l'analyse, sur le motif mesuré de
mika#2050 : la prose de ce fichier cite les jetons qu'elle décrit.

**Pourquoi un scan et pas un test comportemental.** Retirer l'acquisition ne rend
**aucune décision fausse** le jour où on l'écrit : la purge continue de purger,
V1–V11 restent verts, et seule la fenêtre se rouvre — en silence. C'est la classe
exacte que `mika2342_every_llm_call_is_wrapped_in_a_timeout` a dû fermer par un
scan, avec la même phrase (« retirer le filet ne casse aucune assertion »).

**Disposition retenue : (a) — exception nommée en allowlist, allowlist livrée
vide.**

```rust
/// Allowlist du scan — **livrée vide, et elle le reste**.
const REMOVE_DIR_ALL_SITES_ALLOWED: &[&str] = &[];
```

- **Aucune violation existante** : le pré-vol § 5.3 l'établit avant la livraison,
  et l'unique site exécutable est celui que ce ticket rend conforme. Il n'y a donc
  aucune exception à nommer — l'option (a) se réduit ici à son cas dégénéré, qui
  est la forme que la maison livre déjà (`mika2496` U3, `mika2497`
  `TARGET_PURGE_WRITERS_ALLOWED`, mika#2323).
- **Assertion sœur auto-nettoyante** : `mika2511_lallowlist_du_scan_est_vide`
  rougit si l'allowlist cesse d'être vide, sur le modèle immédiatement voisin
  `mika2497_lallowlist_de_la_garde_est_vide` (:4888). Doctrine mika#2201 : quand
  le scan tire, **on rend le site conforme, on ne l'allowliste pas** — une
  allowlist née vide est une place où déposer la prochaine infraction.
- **Contrôle de non-vacuité** : le scan échoue si `remove_dir_all` n'est écrit
  **nulle part** en position exécutable dans le fichier. Sans ce terme, un scan
  visant un jeton mort se lirait exactement comme un scan propre (mika#2496 U4,
  classe mika#2205).
- **Contrôle négatif vu rouge** : le scan doit être observé rouge sur une fixture
  portant un `remove_dir_all` non précédé de l'acquisition, et vert sur une
  fixture le portant avec. Sans les deux, « le scan lit la séquence » est
  indistinguable de « le scan rougit sur tout `remove_dir_all` » et de « le scan
  ne lit rien ».

Ni (b) ni (c) ne s'appliquent : rien n'a à être livré désarmé (le détecteur est
vert sur l'arbre après correctif) et il n'y a aucune violation à faire arbitrer.

---

## 7. Documentation

Une seule entrée, dans la section existante **§ *Optional (purge du `target/`
d'un worktree vif — mika#2497)*** du `CLAUDE.md` racine, plutôt qu'une section
nouvelle : le lecteur qui cherche ce comportement cherche là.

Y sont écrits, et nulle part ailleurs :

1. Le nouveau motif `build_lock_raced`, sa glose, **son régime attendu (non vide
   et faible)** et la halte H3.
2. La borne exacte de la protection (R2, le tableau des trois fenêtres) — c'est la
   phrase qu'un futur lecteur doit trouver avant de croire la protection totale.
3. Le découplage des deux budgets et son effet sur la sonde mika#2449 (R3).
4. Le champ `would_purge` sur `target_purge_tick`, et le fait que **le compte
   `purged` change de sens en `observe` au déploiement** : une comparaison qui
   enjambe le déploiement compare deux vocabulaires. La requête juste est
   `SELECT after_value, count(*) … GROUP BY 1` sur `audit_events`, dont les deux
   `tool_name` étaient déjà séparés et ne bougent pas.
5. La rupture de format de fil (ajout en queue), datée.

---

## 8. Surfaces opérateur et sondes

### SQL

```sql
-- Les suppressions que la fenêtre aurait laissé passer (la mesure du ticket)
SELECT count(*) FROM audit_events
 WHERE tool_name = 'target_purge_skipped' AND after_value = 'build_lock_raced';

-- La distribution complète des refus : la forme réelle de la population
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'target_purge_skipped' GROUP BY 1 ORDER BY 2 DESC;
```

### Journal (`$MIKA_SPIRIT_LOG_FILE`)

```bash
# Le tick agit-il encore en observe ? (B6 — contrôle positif du correctif (c))
grep target_purge_tick "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{purged, would_purge, failed, refused, disposition}'

# La purge tourne-t-elle sur les dépôts au-delà du premier ? (B3)
grep -E 'target_purge_tick|worktree_reap_tick' "$MIKA_SPIRIT_LOG_FILE" | tail
```

| événement / valeur | régime attendu | lecture |
|---|---|---|
| `after_value = 'build_lock_raced'` | **non vide, faible** | chaque ligne est un `target/` qu'on s'apprêtait à supprimer sous un build qui venait de démarrer |
| `target_purge_tick` avec `disposition=observe` | **non vide** dès qu'un candidat est éligible | si muet, B6 n'a pas pris |
| `would_purge` en `armed` | **zéro** | un non-zéro veut dire que la dérivation suit autre chose que la disposition |
| `after_value = 'build_lock_unreadable'` | **rare** | inchangé ; fusionne désormais les deux étages, par décision (§ 2.6) |

### Sondes post-déploiement, et leurs haltes

**S1 — le correctif (b) mord (un tick, multi-dépôts).** Poser
`MIKA_WORKTREE_REAP_REPO_DIRS` sur deux checkouts et
`MIKA_WORKTREE_REAP_MAX_PER_TICK=1` avec deux worktrees terminaux dans le
premier. Attendu : `target_purge_tick` rend compte d'une activité sur le **second**
dépôt, et `main_checkout_dirty` reste consultable pour lui.
**Halte H1 —** rien sur le second dépôt alors que le premier a consommé son
budget : le prédicat n'est pas lu là où on croit. Vérifier d'abord que le binaire
servi porte le correctif (classe mika#2340) **avant** de toucher à
`should_stop_repo_loop`.

**S2 — la fenêtre est mesurée (30 jours).** Le compte `build_lock_raced`.
**Halte H2 —** un compte qui porte du **trafic nominal** (plusieurs par jour, sur
des worktrees différents) ne veut pas dire que le prédicat est trop large : il
veut dire que des builds démarrent couramment dans la fenêtre résiduelle de R2, et
c'est **là** que le suivi `rename`-avant-suppression s'ouvre, avec ce compte comme
précondition. Ne pas raccourcir la fenêtre d'inactivité par réflexe — ce serait
purger le cache de travail en cours pour un problème de course.

**S3 — contrôle négatif du zéro.** Un compte `build_lock_raced` **nul** sur 30
jours ne prouve **pas** que le défaut n'existait pas : il faut qu'un `cargo`
démarre pile dans la fenêtre, et R1 dit que cette fenêtre est de ~2 s pour le
premier candidat d'un tick. Avant de conclure, établir le contrôle positif — que
le bras a réellement purgé quelque chose :
`SELECT count(*) FROM audit_events WHERE tool_name = 'target_purged';`
**Zéro des deux ne prouve rien** (classe mika#2205).

**S4 — non-régression du régime nominal (7 jours).** La distribution des refus
doit rester dominée par `recently_active`, comme avant : c'est la fenêtre qui
protège le travail en cours.
**Halte H3 —** si `build_lock_unreadable` se met à dominer, le bras est
silencieusement inerte par fail-safe — il tourne et ne purge jamais rien, ce qui
se lit exactement comme un disque en bonne santé. Établir pourquoi l'énumération
échoue **avant** de toucher au prédicat.

---

## 9. Hors périmètre, délibérément

- **Le `rename` avant suppression** (R2) — refusé ici avec sa raison mesurée (il
  salit `git status`, donc casse T7 du faucheur, donc crée un worktree
  non-retirable portant un orphelin de 40 Go). **Suivi**, précondition : que S2
  montre une population `build_lock_raced` non négligeable.
- **Le `disposed += 1` en `observe` du bras faucheur** (:1459) — même motif que
  (c), mais il appartient à mika#2420/mika#2469 et change une surface opérateur
  d'un autre ticket. **Suivi.**
- **La veille (d)** — `TARGET_MTIME_SCAN_DEPTH` et la fenêtre de 4 h ne bougent
  pas (N1/N2), et le remède (a) ne touche ni `newest_mtime_bounded` ni
  `inspect_target_dir`. La question que (d) pose (« les écritures cargo profondes
  ne remontent pas toujours le mtime ») reste ouverte et reste couverte, en temps
  réel, par P5 — que ce ticket renforce précisément.
- **La veille (e)** — gestes hôte prescrits en prose (`df -h`, `du -sh`,
  `touch …/worktree-reap-stop`). Aucun correctif code n'est demandé et aucun n'est
  livré : ce sont des gestes opérateur, et un pilote qui les tente prend un deny
  de permission, ce qui est le comportement voulu.
- **Le pic de production simultanée** — ce bras borne l'accumulation, pas le pic.
  Limite déjà écrite par mika#2497, inchangée.
- **La sentinelle STOP, le kill-switch, la disposition, les trois clés
  numériques** — aucun réglage ne bouge (N1, N3).
- **`cargo_build_lock_is_free` en tant que filtre** — conservée avec sa
  sémantique. Le motif est celui de mika#2184 : *le proxy filtre d'abord, la
  mesure directe tranche ensuite*, à ceci près qu'ici les deux étages font la
  même mesure et que seul le second engage.

---

## 10. Definition of Done

- [ ] `cargo_lock_paths` extrait, consommé par `cargo_build_lock_is_free` **et**
      par `acquire_cargo_build_locks` ; sémantique du filtre inchangée.
- [ ] `CargoBuildLockGuard` (RAII, `#[must_use]`, `Drop` explicite) et
      `LockAcquisition` livrés, avec repli non-Linux qui conserve.
- [ ] La disposition acquiert avant de supprimer, relâche après, et n'a entre les
      deux ni `.await` ni opération non bornée.
- [ ] `budget -= 1` déplacé après l'acquisition ; `measure_tree_size` sous verrou.
- [ ] `build_lock_raced` ajouté en queue de `ALL_PURGE_REFUSAL_REASONS`.
- [ ] `should_stop_repo_loop` livré et branché ; bloc T7 + disposition du faucheur
      enveloppé dans `if budget > 0`.
- [ ] `TargetPurgeStats.would_purge` livré ; condition d'émission et champ du log
      d'agrégat mis à jour (B6).
- [ ] `mika2497_v5_observe_ne_supprime_rien` corrigé, avec sa raison écrite dans
      le test.
- [ ] `mika2497_les_motifs_de_purge_sont_un_format_de_fil` mis à jour, avec
      l'entrée CLAUDE.md qui date l'ajout.
- [ ] V1–V11 verts.
- [ ] Scan `mika2511_toute_suppression_est_precedee_de_lacquisition` livré, avec
      son allowlist vide, son assertion sœur, son contrôle de non-vacuité et ses
      deux fixtures de contrôle (une vue rouge, une vue verte).
- [ ] Pré-vol § 5.3 exécuté et son résultat rapporté dans le corps de la PR.
- [ ] Entrée CLAUDE.md § 7 écrite (cinq points).
- [ ] `cargo fmt`, `cargo clippy`, `cargo test -p mika-agent` verts.

---

## 11. Acceptance criteria

1. **AC1** — Le `flock` de chaque `.cargo-lock` du `target/` visé est **tenu**
   pendant `remove_dir_all` et relâché après ; l'acquisition est le dernier acte
   avant la suppression, sans `.await` intermédiaire. *(B1 ; V4, V7, scan § 6.)*
2. **AC2** — Un verrou tenu au moment de l'acquisition **conserve** l'arbre et
   écrit un refus sous le motif `build_lock_raced`, distinct de
   `build_lock_held`. *(B2 ; V1, V7, V11.)*
3. **AC3** — Une acquisition inévaluable conserve l'arbre (fail-safe), y compris
   hors Linux. *(B2, N5 ; V3.)*
4. **AC4** — Le budget du faucheur épuisé n'empêche plus la purge de tourner sur
   les dépôts suivants, ni la sonde `probe_main_checkout` de s'y exécuter.
   *(B3, R3 ; V8, S1.)*
5. **AC5** — La boucle des dépôts cesse toujours quand **les deux** bras sont
   épuisés, ou quand le faucheur est épuisé et la purge désarmée. *(B4 ; V8.)*
6. **AC6** — En `observe`, `stats.purged` vaut zéro et `stats.would_purge`
   compte la population éligible ; en `armed`, l'inverse. *(B5 ; V9, V10.)*
7. **AC7** — `target_purge_tick` continue d'être émis en `observe` et porte
   `would_purge`. *(B6 ; V9.)*
8. **AC8** — Aucune valeur de réglage, aucune variable d'environnement, aucune
   clé d'audit et aucun `tool_name` ne change. *(N1, N3, N4.)*
9. **AC9** — La profondeur de marche mtime, la fenêtre d'inactivité et
   `inspect_target_dir` sont inchangées : le remède (a) ne modifie pas la sonde
   visée par la veille (d). *(N2.)*
10. **AC10** — Le scan structurel du § 6 est livré **armé**, allowlist vide, avec
    son assertion sœur, son contrôle de non-vacuité et ses deux contrôles
    négatifs. *(§ 6.)*
11. **AC11** — L'entrée CLAUDE.md porte la borne exacte de la protection (R2), le
    nouveau motif avec son régime attendu, et le changement de sens de `purged`
    en `observe` au déploiement. *(§ 7.)*
