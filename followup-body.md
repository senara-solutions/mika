Suivi de mika#2073, ouvert avec sa PR. mika#2073 a fermé la course sur
`crates/mika-common/src/home.rs` ; **six sites de `crates/mika-agent/src/well_known_agents.rs`
portent exactement la même mine, armée de la même façon, et restent intacts.**

## Pourquoi ce n'était pas dans mika#2073

L'AC2 de mika#2073 dit « audit du **même fichier** ». Sa seconde phrase (« le
défaut est la classe, pas la ligne 790 ») justifiait une lecture plus large, mais
étendre l'audit à un autre crate est une divergence de spec — et une divergence
de spec se ratifie par l'opérateur, jamais par l'architecte ni par le plan qui en
bénéficie. La mesure a donc été conservée et reportée ici plutôt qu'exécutée sans
mandat.

## La mesure, re-vérifiée à l'implémentation de mika#2073 (HEAD `bfd8ad0d`)

Six appels de test à `bootstrap_agent()` dans `crates/mika-agent/src/well_known_agents.rs` :

| ligne | test |
|---|---|
| 2252 | `test_provision_skips_existing_agent` |
| 2273 | `test_provision_partial_state` |
| 3210 | `pre_seed_identity` (helper — la conversion couvre ses appelants d'un coup) |
| 3660 | `test_reconcile_config_toml_for_mika_dev` |
| 3681 | `test_reconcile_config_toml_idempotent` |
| 3705 | `test_reconcile_config_skips_agents_without_spec_config` |

`bootstrap_agent` appelle `bootstrap`, donc lit `AgentTier::from_env()` sur
l'environnement du **processus**. Les six sont des `#[test]` **nus**, et ils
tournent dans le même binaire (`-p mika-agent --lib`) que deux poseurs de
`MIKA_AGENT_TIER=family`, tous deux sériels :

- `crates/mika-agent/src/server/mod.rs:1952`
- `crates/mika-agent/src/server/tier_guard.rs:464`

`#[serial]` ne sérialise **que ses porteurs** : un `#[test]` nu tourne en
parallèle d'eux. C'est la course de mika#2073, à la ligne près.

**Le septième appel, `:927`, est de production** — `provision_agent` crée un agent
bien connu. Il doit **rester** un lecteur d'environnement : c'est le chemin par
lequel `MIKA_AGENT_TIER` atteint légitimement un agent au premier démarrage
(mika#1778). Le convertir inverserait un comportement de production.

## Pourquoi c'est silencieux aujourd'hui — et ce que ça vaut

**Vérifié, et c'est le résultat qui fixe la priorité : aucun des six n'assère le
contenu d'un gabarit.** Ils écrasent `identity.toml` / `soul.md` juste après le
bootstrap, ou n'assèrent que de l'existence. Ce sont donc des **mines armées**, pas
un second défaut vivant — exactement la chance qui a tenu `home.rs` jusqu'au
29/08/2026, où une assertion de contenu a fini par tirer sur une PR qui ne touchait
rien de tout ça.

## Ce qu'il y a à faire — deux gestes indissociables, dans cet ordre, au même commit

1. **Convertir les six** vers `bootstrap_agent_with_tier(home, name, AgentTier::Default)`
   (la variante existe depuis mika#2073, publique et non gated précisément pour
   être atteignable depuis `mika-agent`).
2. **Élargir au workspace le scan de `mika2073_no_bare_test_reads_the_tier_from_the_environment`**
   (`crates/mika-common/src/home.rs`), aujourd'hui borné à `home.rs`.

L'ordre est contraint : élargi **avant** la conversion, le scan est rouge sur six
sites et la PR est inlivrable ; élargi **après**, il démarre vert — donc la
conversion doit le voir rouge d'abord, exactement comme mika#2073 a fait voir le
sien rouge sur ses huit sites avant de convertir.

Deux branches du prédicat ne sont aujourd'hui attestées que par le contrôle de
bonne foi, sur lignes fabriquées, faute de se rencontrer dans `home.rs` :
`#[tokio::test…]` et `#[serial_test::serial]` en position d'attribut. **Elles se
rencontrent toutes les deux dans `mika-agent`** (`tier_guard.rs:443` porte la
seconde), donc cet élargissement est aussi ce qui les exercera pour de bon sur du
source réel.

## Critères d'acceptation

- **AC1** — Les six appels de test de `well_known_agents.rs` passent le tier
  explicitement. `:927` (production) est inchangé.
- **AC2** — Le scan de `mika2073_no_bare_test_reads_the_tier_from_the_environment`
  couvre le workspace, allowlist **vide**, disposition halt-and-surface inchangée.
  Son garde-fou « le scan a-t-il vraiment lu quelque chose ? » est adapté au
  nouveau périmètre.
- **AC3** — Le scan élargi a été **vu rouge** sur les six sites avant leur
  conversion, et la PR le dit.
- **AC4** — `crates/mika-common/CLAUDE.md` retire la mention du périmètre borné à
  `home.rs` et du présent ticket de suivi.
- **AC5** — `cargo test -p mika-agent --lib` et `cargo test -p mika-common --lib`
  verts.

## Hors périmètre

- Le site de production `:927`.
- `MIKA_HOME` et `MIKA_DEPLOYMENT` : la même classe de course existe en principe,
  aucune n'a produit d'échec mesuré, et élargir la garde à une variable sans
  défaut mesuré serait généraliser depuis un point.

## Provenance

Mesuré pendant le grooming de mika#2073 (§1.3 de
`docs/plans/2026-09-20-003-fix-2073-mika-common-test-bootstrap-creates-plan.md`),
re-vérifié à l'implémentation, et reporté ici sur le finding F3 de la revue
architecte (divergence de spec non ratifiable par l'architecte).
