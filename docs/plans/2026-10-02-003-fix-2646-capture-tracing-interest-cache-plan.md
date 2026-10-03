# mika#2646 — la capture `tracing` cesse de dépendre de l'ordre des tests

**Ticket :** senara-solutions/mika#2646 (`bug`, `dispatch:loop`)
**Type :** fix (hygiène de harnais de test)
**Date :** 2026-10-02

---

## Constat, et il est entièrement établi par le dépôt

`teams::engine::tests::mika2633_la_ligne_de_refus_nomme_lequipe_et_le_redacteur`
(arrivé avec #2640 / mika#2633) a rougi en CI sur une PR qui ne touche pas ce
module : PR #2643, tête `26e70b43`, run `37037943870`, job « Test with
telemetry feature », 2026-10-02 17:24Z, `panicked at
crates/mika-agent/src/teams/engine.rs:3387` — c'est
`assert_eq!(refusals.len(), 2, "la ligne nominale et son résidu")`, donc **la
capture n'a vu aucun événement**. `5838 passed; 1 failed`, et la CI de `main`
est verte sur `d1a6ff6c`.

Le mécanisme est déjà écrit dans le dépôt, à la ligne près, par
`crates/mika-common/tests/llm_retry.rs` (l. 633-656) :

> a `tracing` callsite caches its `Interest` **globally**, decided by whichever
> thread reaches it first. In a test binary that is routinely a test with no
> subscriber installed, which answers `never` — after which the capturing test
> on another thread observes nothing at all, and the failure looks like "the
> rail did not emit" rather than "the callsite was disabled before we got
> there".

Et le poison est **nommable ici** : les deux callsites capturés vivent tous
deux dans `commit_deliverable` — `crates/mika-agent/src/teams/engine.rs:848`
pour la ligne nominale (`guard.testimony_access_proposal`) et le `warn!` de la
l. 869 pour son résidu (`…_uncorrected`, champ `event` à la l. 882) — et une
dizaine de tests voisins du **même module** appellent
`commit_deliverable(PROPOSAL, …)` **sans aucun abonné installé** :
`mika2633_v4_…` (l. 3068), `mika2633_v3_…` (l. 3105),
`mika2633_v2_une_redaction_encore_sale…` (l. 3174), `mika2633_la_ligne_neutre…`
(l. 3636), entre autres. Chacun est un poisonneur suffisant.

Les deux autres émetteurs de la même famille — `tools/mod.rs:743` et
`agent_loop/mod.rs:3069`, pour les canaux `send_message` et `end_turn` — sont
des callsites **distincts**, donc des caches distincts : ils ne peuvent ni
empoisonner ni être empoisonnés par celui-ci, et sont écartés à ce titre.

**Le défaut est donc une course, et le dépôt porte déjà son remède mesuré.** Il
n'y a rien à établir sur l'existence du mécanisme : il reste à l'appliquer, et à
le rendre impossible à reperdre.

### Pourquoi c'est prioritaire

`Check` est un check **requis** sur `main` depuis le 2026-10-02 (pont (b) de
mika#2617). Un test de capture instable peut donc bloquer n'importe quelle PR,
sans rapport avec elle — et c'est exactement ce qui s'est produit sur #2643.

---

## Rectifications que la lecture du code impose au ticket

Elles sont le **premier livrable** : trois d'entre elles changent le périmètre,
et la quatrième change la méthode de vérification.

### R1 — AC1 nomme un test ; le fichier en porte **deux**, à exposition identique

`crates/mika-agent/src/teams/engine.rs` contient **deux** `set_default`
(l. 3363 et l. 3415), pas un :

| ligne | test | forme de l'échec sur capture vide |
|---|---|---|
| 3363 | `mika2633_la_ligne_de_refus_nomme_lequipe_et_le_redacteur` | `assert_eq!(refusals.len(), 2)` → **le rouge mesuré** |
| 3415 | `mika2633_sans_redacteur_aucun_writer_agent_nest_invente` | `.expect("la ligne de refus est émise")` → **panique identique** |

Le second capture le **même** callsite, est poisonné par les **mêmes** voisins,
et n'est pas nommé par AC1. **Corriger le seul test nommé laisserait le défaut
identique une fonction plus loin** — et c'est le contrôle négatif de mika#2633
(« aucun `writer_agent` n'est inventé »), donc une garde de correctness qui
rougirait au hasard.

### R2 — la population est de **6 sites** dans 4 fichiers, et sa frontière est le **binaire**, pas le fichier

Le compte du ticket (« 4 fichiers utilisent `set_default` ») est **exact** et
confirmé. Mais `panic_hook.rs` en porte **trois**, donc la population est de
six sites :

| # | fichier:ligne | porteur | forme |
|---|---|---|---|
| 1 | `teams/engine.rs:3363` | test (inline) | `CapturingLayer(Arc<Mutex<Vec<HashMap>>>)` |
| 2 | `teams/engine.rs:3415` | test (inline) | idem |
| 3 | `skills/builtin_handlers.rs:11722` | helper `capture_tracing_events()` | `CapturingLayer { events }` + `CapturedEvent` |
| 4 | `kg/resolver_tick.rs:748` | helper `capture()` | `CapturingLayer { events }` + son propre `CapturedEvent` |
| 5-7 | `panic_hook.rs:123, 152, 180` | tests (inline) | `fmt::layer()` vers un `Vec<u8>` |

Et surtout : **le cache d'`Interest` est global au processus**, donc l'unité
d'analyse est le **binaire de test**. Les six sites vivent dans le binaire des
tests unitaires de la lib `mika-agent` : ils partagent un seul cache et doivent
être traités ensemble. Les neuf fichiers de `crates/mika-agent/tests/` qui
utilisent `set_default` sont des binaires **distincts** (un processus chacun) —
hors population, nommés au § *Hors périmètre*.

### R3 — le `HOOK_MUTEX` de `panic_hook.rs` n'est **pas** un substitut, et ce fichier porte une exposition qui lui est propre

`panic_hook.rs:52` déclare un `HOOK_MUTEX` et son commentaire dit ce qu'il
garde : `std::panic::set_hook` est **process-global**, donc les trois tests se
sérialisent pour ne pas s'écraser mutuellement le hook. C'est un autre danger,
et il ne touche **ni** le cache d'`Interest` **ni** les tests des autres
modules — exactement ce que `wip_rescue.rs:2289-2292` a déjà dû écrire pour ce
même attribut :

> it serialises these tests against *other `#[serial]` tests*, not against the
> whole binary. Tests without the attribute still run in parallel.

Un mutex **module-local** est plus faible encore : il est invisible même aux
autres tests `#[serial]`.

**Et l'exposition propre à ce fichier est structurelle :** le hook est
**process-global** pendant que l'abonné est **thread-local**. Une panique sur un
**autre** thread, dans la fenêtre où le hook est installé, émet `process_panic`
sans aucun abonné en vue → le callsite est mis en cache à `never` → les trois
tests voient ensuite un tampon vide. La population de telles paniques est
**mince mais réelle** (un `should_panic` mesuré dans le binaire,
`calibration/providers.rs`, plus toute assertion qui échoue réellement).

Le raisonnement qui **exempterait** `panic_hook.rs` est donc un raisonnement
d'exclusivité (« ce callsite n'est atteint que sous l'abonné »), et il est faux
dès qu'un thread parallèle panique. **Verdict : les trois sites reçoivent le
même traitement que les autres** — le correctif coûte un appel et supprime le
besoin de tenir ce raisonnement.

### R4 — AC3 : la reproduction par la course est probabiliste **par construction**, et une reproduction déterministe existe sans le harnais

Le harnais `libtest` ordonne les tests **par nom**. Dans
`teams::engine::tests::`, le test capturant trie **avant** ses poisonneurs :

```
mika2633_la_ligne_de_refus_…     ← capture (d)
mika2633_la_ligne_neutre_…       ← poisonneur (n)
mika2633_sans_redacteur_…        ← capture
mika2633_v2_…  v3_…  v4_…        ← poisonneurs
```

Donc `--test-threads=1` est **déterministement vert**, et c'est *l'explication
complète* du régime observé : vert en local, vert sur `main`, rouge une fois sur
#2643 quand la course a été perdue. Un filtre ne peut pas inverser cet ordre
(un filtre sélectionne, il n'ordonne pas), donc **chercher un rouge en rejouant
le binaire en parallèle est un tirage**, pas une mesure.

La reproduction déterministe ne passe pas par le harnais : elle tient dans
**un seul test**, et c'est ce que l'AC3 obtiendra (§ *Contrat de vérification*,
V3).

### R5 — la feature `telemetry` n'est pas causale

Le rouge a été observé dans le job « Test with telemetry feature »
(`cargo test --workspace --features telemetry`, `ci.yml:126`). Le mécanisme —
un cache d'`Interest` décidé par le premier thread — ne dépend d'aucune feature,
et aucun abonné global n'est installé dans un binaire de test quelle que soit la
feature. **`telemetry` est le job qui a perdu la course, pas la cause.** À
écrire, sinon la prochaine lecture conclura que la feature doit être désarmée.

---

## Exigences

1. Les six sites de capture de `crates/mika-agent/src` voient les événements
   qu'ils assertent, **indépendamment de l'ordre et du parallélisme** des tests
   du binaire.
2. Le **septième** site, écrit demain, ne peut pas reperdre le correctif — et la
   garde qui le tient ne doit pas être une consigne en prose : `llm_retry.rs`
   documentait déjà ce piège de façon exemplaire, et `teams/engine.rs` l'a
   reperdu quand même. C'est le motif
   `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
   appliqué à l'hygiène de harnais : la prose n'a pas tenu, mesuré.
3. Aucun comportement de production n'est modifié. Aucun `Cargo.toml` n'est
   touché (`serial_test` est **déjà** dev-dependency de `mika-agent`,
   `Cargo.toml:85`, et `tracing-subscriber` aussi).
4. Aucune assertion existante n'est affaiblie pour faire passer un test.

---

## Approche : **un seul site d'installation**, et non six rebuilds co-localisés

### Ce qui est livré

Un installateur unique dans le module de support déjà existant
(`crates/mika-agent/src/test_utils.rs`, `#[cfg(test)] pub mod test_helpers`,
déclaré `lib.rs:53`) :

```rust
/// Installe un abonné de capture pour le thread courant, et **ré-interroge
/// tous les callsites** (mika#2646).
///
/// `rebuild_interest_cache` n'est pas optionnel, et la raison est celle que
/// `mika-common/tests/llm_retry.rs` a déjà dû écrire : un callsite `tracing`
/// met son `Interest` en cache **globalement**, décidé par le premier thread
/// qui l'atteint. Dans ce binaire c'est couramment un test sans abonné, qui
/// répond `never` — après quoi le test capturant ne voit plus rien, et l'échec
/// ressemble à « le rail n'a pas émis » plutôt qu'à « le callsite était
/// désactivé avant notre arrivée ». Mesuré sur mika#2646 : PR #2643, run
/// 37037943870.
///
/// **Site unique volontairement** : six appels nus à `set_default` vivaient
/// dans ce crate et aucun ne ré-interrogeait le cache, alors que le piège était
/// documenté à la ligne près ailleurs dans le dépôt. Un rebuild qu'on peut
/// oublier est un rebuild qu'on oublie ; celui-ci n'a pas de site où être
/// oublié. Tenu par `mika2646_set_default_a_un_site_dinstallation_unique`.
pub fn install_capturing_subscriber<S>(subscriber: S) -> tracing::subscriber::DefaultGuard
where
    S: tracing::Subscriber + Send + Sync + 'static,
{
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    guard
}
```

Les six sites deviennent un appel. Chaque module **garde son layer et son
visiteur** — les quatre diffèrent (trois formes de `CapturedEvent`, plus un
`fmt::layer()` qui écrit des octets), et les unifier serait un refactor que le
ticket ne demande pas. Seule l'**installation** est partagée, et c'est la seule
moitié où le défaut vit.

### Pourquoi pas « ajouter `rebuild_interest_cache()` aux six sites »

C'est la lettre d'AC1/AC2, et c'est l'option la plus faible disponible. La
garde correspondante serait une règle **textuelle de co-localisation** (« tout
corps de fonction portant `set_default` porte aussi
`rebuild_interest_cache` ») : fragile, et surtout elle laisse le septième site
se faire oublier puis *détecter*, là où le site unique le rend
**inoubliable**. Le dépôt emploie partout le motif du lecteur / écrivain unique
pour exactement cette raison — `grooming_marker` (mika#2158),
`LlmUsage::accumulate` (mika#1883), `discover_build_dirs` (mika#2619),
`FIRED_AT_STAMP_IF_NULL` (mika#2133). C'est le même geste, et il est ici moins
coûteux que la règle textuelle qu'il remplace.

### `#[serial_test::serial]` : ce qu'il achète, et ce qu'il n'achète pas

Les **deux** moitiés sont posées, et elles ne couvrent pas la même fenêtre :

- `rebuild_interest_cache()` répare un cache empoisonné **avant** notre
  capture — c'est la moitié **porteuse**, et la seule qui ferme le défaut
  mesuré.
- `#[serial_test::serial]` empêche un **autre test capturant** de re-décider le
  cache **pendant** notre capture (tout site qui appelle `set_default`
  enregistre un dispatcher). C'est la moitié de **composition**, et c'est
  littéralement ce que `llm_retry.rs` écrit : *« the rebuild fixes the cache,
  and the serialization is what stops a parallel test from re-deciding it mid
  capture »*.

**Le résidu est nommé, pas fermé** : `#[serial]` ne sérialise que contre les
autres tests `#[serial]` (`wip_rescue.rs:2289`). Un test **non annoté** continue
de tourner en parallèle — mais il ne peut pas re-décider le cache après notre
rebuild, parce qu'il n'appelle pas `set_default`. La fenêtre résiduelle est donc
« un site de capture non annoté », et c'est précisément ce que le site unique
rend recensable : les porteurs de `#[serial]` sont les appelants de
l'installateur, énumérables par `grep`.

Convention du crate, vérifiée : `#[tokio::test]` **puis**
`#[serial_test::serial]` (chemin complet, attribut de test en premier) —
`wip_rescue.rs:2393-2395` compile ainsi aujourd'hui.

---

## Étapes d'implémentation

### U1 — l'installateur

Ajouter `install_capturing_subscriber` à
`crates/mika-agent/src/test_utils.rs::test_helpers`, avec le doc-comment
ci-dessus (il porte la mesure : run, PR, mécanisme — c'est le site où la
prochaine lecture le cherchera).

### U2 — router les six sites

| fichier | geste |
|---|---|
| `teams/engine.rs:3363` et `:3415` | remplacer `set_default(subscriber)` par `crate::test_utils::test_helpers::install_capturing_subscriber(subscriber)` ; ajouter `#[serial_test::serial]` sous `#[tokio::test]` |
| `skills/builtin_handlers.rs:11722` | idem dans `capture_tracing_events()` ; `#[serial_test::serial]` sur **ses appelants** (les tests), pas sur le helper |
| `kg/resolver_tick.rs:748` | idem dans `capture()` ; `#[serial_test::serial]` sur ses appelants |
| `panic_hook.rs:123, 152, 180` | idem ; **conserver `HOOK_MUTEX`** (il garde le hook process-global, un danger distinct — R3) ; ajouter `#[serial_test::serial]` sous `#[test]` |

Énumérer les appelants des deux helpers par `grep -n 'capture_tracing_events()\|capture()'`
dans les deux fichiers, et annoter **chacun** : un appelant oublié est un test
capturant non sérialisé, c'est-à-dire la fenêtre résiduelle ci-dessus.

### U3 — la garde structurelle (détecteur — voir `## Fire-Disposition`)

`test_utils::tests::mika2646_set_default_a_un_site_dinstallation_unique` :
scanner `crates/mika-agent/src/**/*.rs` et asserter que
`tracing::subscriber::set_default` n'apparaît **qu'à un seul site**,
l'installateur.

Trois détails d'implémentation, dont deux sont des pièges :

1. **Commentaires retirés d'abord** (`crate::source_scan::strip_comment_lines`) :
   sans quoi un doc-comment qui *mentionne* `set_default` — et celui de
   l'installateur le fait — compte comme un site.
2. **PIÈGE — ne pas réutiliser `production_sources()`.** Toutes les gardes
   voisines écartent le code de test (`source_scan::is_test_source_path`,
   `production_half`) parce qu'elles cherchent dans la production un motif que
   la production ne doit pas porter. **Ici c'est l'inverse : les six sites
   vivent tous dans du code de test.** Une garde bâtie sur `production_sources()`
   trouverait **zéro** site et se lirait comme un arbre propre — la classe
   mika#2205 appliquée à la garde elle-même. Le scan énumère `src/**/*.rs`
   **sans** filtre de test.
3. **Anti-vacuité** : asserter que le site de l'installateur **a été trouvé**.
   Un scan qui vise un nom mort (module renommé, helper déplacé) rend zéro
   infraction et se lit exactement comme un arbre sain — motif de la cardinalité
   de mika#2496 et du `!declared.is_empty()` de mika#2201.

Le message d'échec nomme la **résolution** : router le site vers
`install_capturing_subscriber`, jamais ajouter une entrée d'exception
(doctrine mika#2201 : *« on déclare, on n'allowliste pas »*).

### U4 — ce qui n'est **pas** couvert par U3, dit plutôt que découvert

Le scan tient la moitié **porteuse** (`set_default` ⇒ passage par
l'installateur ⇒ rebuild). Il **ne tient pas** `#[serial]`, et c'est une
décision : l'attribut vit sur le **test appelant**, pas sur le helper, donc le
couvrir demanderait de résoudre un graphe d'appels dans un scan textuel. Le
recensement des appelants est à la place rendu trivial par le site unique
(un `grep` sur `install_capturing_subscriber`). Nommé ici pour que personne ne
lise le vert du scan comme « les deux moitiés sont tenues ».

---

## Fire-Disposition

Ce plan livre **un** détecteur : la garde structurelle U3
(`mika2646_set_default_a_un_site_dinstallation_unique`), dont le chemin de
succès est « aucun `set_default` hors de l'installateur ».

**Disposition retenue : (a) exception nommée en allowlist — livrée VIDE.**

- L'allowlist est une constante `SET_DEFAULT_SITES_ALLOWED: &[&str] = &[]`,
  grep-visible, **livrée vide** et **épinglée vide** par un test frère
  (`mika2646_lallowlist_du_scan_est_livree_vide`, motif des allowlists vides de
  mika#2496 U3, mika#2520 et mika#2633).
- Elle est vide parce qu'**il n'y a rien à excepter** : les six violations
  existantes sont routées vers l'installateur **dans le même commit** (U2), donc
  le détecteur atterrit sur un arbre déjà conforme. Aucune entrée ne référence
  donc de ticket de suivi, et aucune assertion auto-nettoyante n'a d'entrée à
  surveiller — l'épinglage à vide **est** l'assertion auto-nettoyante : il rougit
  dès que quelqu'un y ajoute une ligne.
- **Quand le scan tire, la résolution est de router le site**, jamais
  d'allowlister. C'est écrit dans le message d'échec et dans le doc-comment du
  scan.

Les options (b) *livrer désarmé* et (c) *halte-et-remontée* sont écartées : (b)
n'a pas de sens pour un détecteur qui naît vert sur un arbre conforme — un
`#[ignore]` ici serait une garde que personne n'exerce, la classe mika#2205 ;
(c) n'a aucun arbitrage à remonter, les six sites étant corrigés par le même
geste mécanique.

---

## Contrat de vérification

### V1 — le défaut mesuré ne se rejoue plus (déterministe)

```bash
cargo test -p mika-agent --features telemetry teams::engine::tests::mika2633
cargo test -p mika-agent --features telemetry kg::resolver_tick
cargo test -p mika-agent --features telemetry panic_hook
cargo test -p mika-agent --features telemetry skills::builtin_handlers::tests::test_spawn_and_collect
```

Chacune verte, **et** la suite complète du job CI qui a rougi :

```bash
cargo test --workspace --features telemetry
```

### V2 — la garde structurelle mord, et elle regarde quelque chose

```bash
cargo test -p mika-agent mika2646_
```

Deux assertions, et la seconde est celle qui compte : le scan doit **trouver**
le site de l'installateur. Contrôle négatif à **voir rouge** avant de livrer :
remettre un `set_default` nu dans `teams/engine.rs` et constater que le scan
l'accuse en nommant le fichier et la résolution.

### V3 — AC3, la démonstration **déterministe** du mécanisme

La course n'est pas reproductible à la demande (R4), donc le mécanisme est
démontré **sans** le harnais, dans un seul test, sous `.pilot-scratch/` :

```rust
// 1. empoisonner : atteindre le callsite SANS abonné
let mut e1 = engine_with_mock(tmp.path(), vec![]);
let _ = e1.commit_deliverable(PROPOSAL.to_string(), DeliverableSource::NoDelegation).await;

// 2. installer la capture SANS rebuild (l'état d'avant ce correctif)
let events = Arc::new(Mutex::new(Vec::new()));
let _g = tracing::subscriber::set_default(
    tracing_subscriber::registry().with(CapturingLayer(Arc::clone(&events))));

// 3. émettre de nouveau
let mut e2 = engine_with_mock(tmp.path(), vec![]);
let _ = e2.commit_deliverable(PROPOSAL.to_string(), DeliverableSource::NoDelegation).await;

assert_eq!(events.lock().unwrap().len(), 0, "callsite empoisonné — le défaut de mika#2646");
```

Puis le **contrôle positif** : la même séquence avec
`tracing::callsite::rebuild_interest_cache()` après le `set_default` doit
capturer les événements. Les deux sorties sont recopiées dans le corps de PR.

**Cette démonstration ne part PAS dans le commit**, pour deux raisons : elle
asserte que le défaut existe (elle rougirait le jour où `tracing` changerait sa
politique de cache), et elle est elle-même fragile à l'ordre (un `set_default`
parallèle rebuild sous elle). Elle vit sous `.pilot-scratch/mika2646/` et, par
mika#2548, **n'est pas supprimée**.

Si la démonstration déterministe échoue à produire la capture vide — c'est-à-dire
si le cache n'est pas empoisonnable comme décrit — **halte** : le diagnostic du
ticket serait faux pour la version de `tracing-core` en vigueur (0.1.36, épinglé
dans `Cargo.lock`), et il faut relire le mécanisme **avant** de livrer quoi que
ce soit. Le correctif resterait inoffensif, mais le plan aurait menti sur sa
cause.

### V4 — l'exhaustivité du recensement (AC2)

```bash
grep -rn 'tracing::subscriber::set_default' crates/mika-agent/src/
```

Doit rendre **exactement une** ligne après U2 : l'installateur.

### V5 — contrôle négatif de non-régression

`cargo clippy --workspace --all-targets --features telemetry` et
`cargo fmt --check` propres ; **aucun** fichier de production modifié :

```bash
git diff --name-only main... | grep -v '^docs/plans/'
```

Ne doit rendre que `test_utils.rs` et les quatre fichiers du recensement — et
dans ces quatre, le diff ne doit toucher que des lignes sous `#[cfg(test)]`.

---

## Acceptance criteria

- **AC1.** `mika2633_la_ligne_de_refus_nomme_lequipe_et_le_redacteur` ré-interroge
  le cache de callsites après l'installation de son abonné, et est sérialisé
  (`#[serial_test::serial]`), comme les tests de capture de `llm_retry.rs`.
  **Étendu par R1 :** le test frère
  `mika2633_sans_redacteur_aucun_writer_agent_nest_invente` (l. 3409), à
  exposition identique et non nommé par le ticket, reçoit le même traitement.
- **AC2.** Les autres sites de `crates/mika-agent/src` capturant par
  `set_default` sont recensés — **4 fichiers, 6 sites** (R2) — et reçoivent tous
  le même traitement. Aucun n'est exempté ; en particulier le `HOOK_MUTEX` de
  `panic_hook.rs` est établi comme **non substituable** (R3), avec sa raison
  écrite au site.
- **AC3.** Le mécanisme est démontré **déterministement** (V3 : empoisonnement
  puis capture vide, plus le contrôle positif avec rebuild), et l'absence de
  reproduction fiable *par la course* est documentée avec sa cause — l'ordre
  alphabétique de `libtest` place le test capturant avant ses poisonneurs, donc
  `--test-threads=1` est déterministement vert (R4).
- **AC4.** Une garde structurelle refuse un septième `set_default` nu dans
  `crates/mika-agent/src`, allowlist livrée **vide** et épinglée vide, message
  d'échec nommant la résolution (router, pas excepter).
- **AC5.** Aucun comportement de production modifié ; aucun `Cargo.toml`
  touché ; aucune assertion existante affaiblie.

---

## Ce que ce travail n'achète PAS

- **Il ne rend pas la capture `tracing` robuste en général.** Il supprime la
  fenêtre d'empoisonnement **antérieure** à la capture et sérialise les
  capturants entre eux. La fenêtre résiduelle — un site de capture non annoté
  `#[serial]` — est nommée au § `#[serial_test::serial]` et rendue recensable
  par le site unique, pas fermée.
- **Il ne couvre pas les deux autres crates.** `crates/mika-common/src` porte
  2 fichiers avec `set_default` (`llm/budget_provenance.rs`,
  `llm/config_freshness.rs`) et `crates/mika-gateway/src` en porte **5**
  (`egress_search/{mod,brave,tests_e4_no_log}.rs`,
  `egress_fetch/gouv_fr.rs`). Même classe, binaires distincts, donc **aucun**
  n'est affecté par ce correctif ni ne peut affecter le test mesuré.
  `crates/mika-common/tests/llm_retry.rs` est déjà correct — c'est la référence.
  **Suivi**, et sa précondition est écrite : un rouge mesuré dans l'un de ces
  binaires, ou le même geste posé par qui y touchera ensuite. Les étendre ici
  triplerait le rayon de souffle d'un correctif de flake, et chaque crate
  demande son propre installateur (un helper `#[cfg(test)]` ne traverse pas une
  frontière de crate).
- **Il ne couvre pas les 9 fichiers de `crates/mika-agent/tests/`** qui
  utilisent `set_default`. Chacun est un binaire à soi, donc hors de la
  population du cache partagé mesuré — nommés plutôt que masqués, même suivi.
- **Il ne rattrape pas le rouge du 2026-10-02.** Le run `37037943870` reste ce
  qu'il est ; la sonde est la **prochaine** exécution du job
  `cargo test --workspace --features telemetry`.
- **Il n'ajoute aucune surface d'observabilité** : ni compteur, ni événement, ni
  ligne d'audit. Le défaut est un test instable, et son instrument est la CI.

---

## Hors périmètre, délibérément

- **Unifier les quatre layers de capture** en un seul `CapturedEvent`. Les
  formes diffèrent réellement (trois visiteurs, plus un `fmt::layer()` qui écrit
  des octets), et le ticket ne demande pas ce refactor. Seule l'**installation**
  est partagée — la seule moitié où le défaut vit.
- **Retirer le `HOOK_MUTEX`** de `panic_hook.rs` au motif que `#[serial]` le
  remplacerait. Il ne le remplace pas : il garde `std::panic::set_hook`, qui est
  process-global, contre des tests que `#[serial]` ne contraint pas (R3).
- **Les callsites de `agent_loop/mod.rs:3142`** (le frère du même événement sur
  le canal `end_turn`) : callsite distinct, cache distinct, et ses tests ne sont
  pas dans la population mesurée.
- **Désarmer ou interroger la feature `telemetry`** : elle n'est pas causale
  (R5).
- **Le contenu de mika#2633** : aucune assertion de ses tests n'est modifiée,
  seulement l'installation de leur abonné. Si une assertion devait bouger pour
  faire passer un test, ce serait un défaut distinct et il faudrait l'instruire
  comme tel.

---

## Definition of Done

- [ ] `install_capturing_subscriber` livré dans `test_utils::test_helpers` avec
      son doc-comment portant la mesure (PR #2643, run `37037943870`, mécanisme).
- [ ] Les 6 sites routés ; `grep -rn 'tracing::subscriber::set_default' crates/mika-agent/src/`
      rend exactement une ligne.
- [ ] `#[serial_test::serial]` sur les deux tests inline de `teams/engine.rs`,
      les trois de `panic_hook.rs`, et **tous** les appelants des deux helpers.
- [ ] Garde structurelle U3 livrée, allowlist vide, épinglée vide, contrôle
      négatif **vu rouge**.
- [ ] V1 à V5 exécutés et verts ; sortie de V3 (démonstration + contrôle
      positif) recopiée dans le corps de PR.
- [ ] `cargo clippy --workspace --all-targets --features telemetry` et
      `cargo fmt --check` propres.
- [ ] Le corps de PR nomme les cinq rectifications (R1-R5) et le résidu
      `#[serial]`.
