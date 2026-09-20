# mika#2230 — `mika agents reprovision` : la re-provision d'identité a un geste outillé

> **Plan:** docs/plans/2026-09-20-002-feat-2230-cli-reprovision-identite-agent-plan.md
> **Issue:** senara-solutions/mika#2230
> **Type:** feat
> **Parent:** extrait de mika#2027 (F3, scope creep) — le fail-closed est clos, le chemin outillé ne l'est pas

---

## 1. Ce que la lecture du code déplace dans le ticket

Le ticket est juste sur son constat central et **incomplet sur deux points**, dont
un qui change la forme du livrable. La rectification est le premier livrable.

### 1.1 Confirmé : les trois portes existent bien, et la suppression est définitive

Vérifié à HEAD `75cd9288` :

| porte | site | ce qu'elle fait |
|---|---|---|
| `is_initialized` | `home.rs:295` | rend `true` dès que `data/mika.db` existe → `bootstrap_fresh_install` (`home.rs:320`) ne re-tourne **jamais** |
| `write_default_if_missing` | `home.rs:483` | `if !path.exists()` — `bootstrap` (`home.rs:464`) restaurerait le fichier, mais rien ne l'appelle pour un agent déjà provisionné |
| `ensure_initialized_for_agent` | `mika-cli/src/init.rs:141` | teste `config.toml`, **pas** `identity.toml` — un agent à qui il ne manque que l'identité passe pour provisionné |

Et `agent_exists` (`mika-common/src/agent.rs:48`) teste lui aussi `config.toml`.
Donc **la forme exacte du sinistre #2027 — `identity.toml` absent, `config.toml`
présent — est invisible à tous les prédicats d'existence du CLI.** C'est le
premier détail d'implémentation qui décide du prédicat de population (§3, D5).

### 1.2 Rectification 1 — mika#2330 a livré un demi-chemin, et il faut dire lequel

Le corps du ticket (et le §7 du runbook) datent d'avant mika#2330. Depuis,
`reconcile_well_known_identity` (`well_known_agents.rs:618`) ré-applique les
sections `CODE_OWNED_IDENTITY_SECTIONS` sur l'`identity.toml` **des agents
bien connus déjà sur disque**, à chaque démarrage, **y compris sous
`MIKA_DISABLE_AGENT_PROVISIONING`** (`well_known_agents.rs:883-888`).

Donc pour un agent bien connu dont l'identité est **présente mais dérivée**, un
chemin outillé existe déjà : redémarrer. Ce qui n'en a **aucun** :

| population | état | chemin aujourd'hui |
|---|---|---|
| agent bien connu, `identity.toml` **présent** mais dérivé | couvert | redémarrage (mika#2330) |
| agent bien connu, `identity.toml` **absent** | **trou** | le réconciliateur lit le fichier **en premier** et sort en `identity_reconcile.skipped` / `read_failed` (`well_known_agents.rs:636-648`) — *la réconciliation a besoin d'un fichier à réconcilier, elle n'en crée pas* |
| agent client/personnel (tier), identité absente ou dérivée | **trou** | aucun : `WELL_KNOWN_AGENTS` ne le contient pas, il n'y a pas de spec à réconcilier |
| **`soul.md`, toutes populations** | **trou** | `reconcile_well_known_identity` ne touche **que** `identity.toml` |

Le périmètre du ticket se resserre sur ces trois lignes et **elles sont toutes
réelles**. Ce plan ne ré-implémente pas mika#2330 : il le complète par en bas
(créer ce qui est absent) et par le côté (l'axe persona).

### 1.3 Rectification 2 — un tier a DEUX axes, et la garde lit les deux

Le ticket dit « ré-écrire l'identité ». Mais depuis mika#2023 un tier est deux
décisions indépendantes : `identity_toml()` lit `tools_profile()`,
`soul_md()` lit `persona_profile()` (`home.rs:177-186`). Et la garde de
démarrage mika#1962 détecte le provisionnement famille sur **deux axes
OR-combinés** : le sentinelle `FAMILY_SOUL_MARKER` dans `soul.md`
(`soul_has_family_marker`) **et** la comparaison d'ensemble de l'allowlist
(`identity_allowlist_matches_family`).

**Conséquence portante : un outil qui n'écrirait que `identity.toml` fabrique
l'état de dérive que la garde existe pour attraper.** Ré-appliquer un tier
famille sur l'identité seule laisse un `soul.md` opérateur sans sentinelle ;
l'inverse laisse une allowlist famille sous une persona opérateur. Le §6 du
runbook le dit déjà en toutes lettres — « write the target tier's
`identity.toml` **and** `soul.md` » — et c'est ce que le verbe doit exécuter.

### 1.4 Rectification 3 — l'alternative « re-bootstrap per-agent » est refusée

Le ticket propose « une sous-commande CLI **OU** un re-bootstrap per-agent
explicite ». La seconde branche est écartée, et la raison est mesurable :
`write_default_if_missing` a **neuf** appelants entre `bootstrap`,
`bootstrap_fresh_install` et `migrate_to_multi_agent`, et sa sémantique
« ne jamais écraser » est exactement ce qui **préserve** la persona d'un
opérateur à chaque démarrage. Lui ajouter un mode écrasant, même paramétré,
met cette garantie à la merci d'un booléen mal passé sur le chemin du
démarrage de tous les agents. Le verbe CLI est purement **additif** : aucun
appelant existant ne change de comportement.

---

## 2. Exigences

- **R1** — `mika agents reprovision <name>` ré-applique sur disque le template
  d'identité **et** de persona faisant autorité pour cet agent.
- **R2** — Une commande, deux populations, dispatchées par
  `find_well_known_agent(name)` : agent bien connu → spec
  (`render_identity_content` + `spec.soul`) ; sinon → tier
  (`tier.identity_toml()` + `tier.soul_md()`).
- **R3** — Le tier appliqué est **explicite** (`--tier`) ou résolu depuis
  l'environnement, et il est **dit** dans la sortie avec sa provenance. `--tier`
  est **refusé** sur un agent bien connu.
- **R4** — Aucun fichier n'est écrasé sans sauvegarde horodatée à côté, en
  `0600`.
- **R5** — Idempotent : un fichier déjà identique au template n'est ni
  sauvegardé ni réécrit, et la sortie le dit.
- **R6** — `--dry-run` montre exactement ce qui serait écrit sans rien écrire.
  Hors dry-run, confirmation par saisie du nom de l'agent ; `--yes` la saute ;
  un terminal non interactif sans `--yes` est **refusé**, jamais répondu à la
  place de l'opérateur.
- **R7** — Fail-closed sur le rendu : un template qui ne parse pas en TOML, ou
  dont `[skills].allowlist` est absente ou vide, **refuse l'écriture**.
- **R8** — `config.toml` n'est **jamais** touché. La base n'est **jamais**
  touchée. Aucun symlink de skill n'est re-matérialisé.
- **R9** — Après écriture, auto-contrôle par `tier_guard::check_agent_tier_consistency`
  contre le tier résolu par **ce process**, et avertissement nommé sur
  l'environnement du **service**, qui est l'autre moitié et que le CLI ne peut
  pas lire.
- **R10** — Documentation : le §7 « Tooled path » du runbook cesse de dire
  « il n'y a pas encore de verbe », `docs/configuration.md`,
  `crates/mika-cli/CLAUDE.md`, et le root `CLAUDE.md`.

---

## 3. Décisions de conception

### D1 — Écriture différentielle, et `--identity-only` comme échappatoire nommée

Le verbe compare chaque fichier au template et **n'écrit que ce qui diffère**.
C'est ce qui rend R5 vrai par construction plutôt que par une branche, et c'est
ce qui fait que le cas mesuré (#2027 : identité absente, persona intacte) touche
**exactement un fichier**.

Mais l'écriture différentielle réécrit aussi une persona **volontairement
retouchée** — elle diffère, donc elle est écrasée (avec sauvegarde). D'où
`--identity-only`, pour « restaure ma frontière de skills, ne touche pas à ma
voix ». **Il porte un avertissement, pas un silence** : après un
`--identity-only` qui change de tier, les deux axes de détection de la garde
mika#1962 sont en désaccord, et c'est exactement l'état que la garde signale.
Le dire au moment du geste coûte une ligne ; le laisser découvrir au prochain
redémarrage coûte un incident.

**Refusé : `--soul-only`.** Aucun besoin mesuré, et il produit la moitié
*dangereuse* de la dérive — une persona famille sous une allowlist opérateur,
c'est-à-dire la forme qui *ressemble* à un tenant famille en gardant
`shell-exec`.

### D2 — Le tier est dit, jamais deviné en silence

`AgentTier::from_env()` lit `MIKA_AGENT_TIER` dans l'environnement **du shell
qui lance le CLI**. Or tout le runbook mika#1962 répète que le tier se pose
« in the service EnvironmentFile … never in an interactive shell ». Un verbe qui
lirait silencieusement la variable du shell ré-instaurerait la confusion
deux-process que mika#1962 a dû écrire.

Donc : `--tier <default|family|champion>` est la voie recommandée, l'absence
retombe sur `AgentTier::from_env()` (même règle que `bootstrap`, pour que le
comportement soit prévisible), et **la sortie nomme toujours le tier résolu et
sa provenance** (`--tier` / `MIKA_AGENT_TIER` / défaut). Modèle :
`llm_budget_resolved` (mika#2293) — *un réglage qu'on ne peut pas observer n'est
pas un réglage.*

`--tier` sur un agent bien connu est une **erreur**, pas un no-op : l'identité
d'un `mika-arch` ne dérive d'aucun tier, et accepter le drapeau laisserait
l'opérateur croire qu'il a posé quelque chose.

### D3 — Le rayon de souffle du tier famille est nommé AVANT l'écriture

`assert_family_tier_env_consistency` (`server/tier_guard.rs:65`) **refuse le
démarrage du process entier** quand un agent est provisionné famille sur disque
alors que le tier du process n'écrit pas les templates famille. Ce n'est pas
un refus par agent : c'est `run_server` qui `bail!`, donc **tous** les agents
tombent.

Conséquence concrète : `reprovision <tenant> --tier family` sur un hôte dont le
service tourne en tier par défaut **couche mika-spirit au prochain
redémarrage**. La garde fait son travail (c'est la différence entre un arrêt
franc et un agent servi sous la mauvaise politique), mais un outil qui amène
l'opérateur là sans le dire est un piège.

Donc, quand le tier résolu attend un provisionnement famille
(`tier.expects_family_provisioning()`), le pré-digest de confirmation porte la
phrase et la commande : poser `MIKA_AGENT_TIER=family` dans l'EnvironmentFile /
ConfigMap du service **avant** le redémarrage.

**La direction inverse est nommée et non gardée**, parce qu'elle n'est pas
gardable ici : `reprovision --tier default` sur un agent famille retire les
marqueurs, et la garde ne dit rien (elle ne détecte que le provisionnement
*famille*, cf. son propre commentaire `well_known_agents`/`tier_guard` : *« the
reverse direction — out of scope here »*). L'agent tournera alors en sémantique
opérateur sous un process famille. Le pré-digest le dit ; aucune ligne de code
ne peut l'empêcher sans inventer un sentinelle opérateur, ce qui est un autre
ticket.

### D4 — Fail-closed au rendu, et c'est le piège du §3d du runbook rendu structurel

Trois refus **avant** toute écriture :

1. `render_identity_content` rend `Err` — c'est le cas de `mika-arch`, dont
   `build_mika_arch_identity` a besoin de `MIKA_KG_DOCS_ROOTS` pour produire des
   chemins absolus. Écrire malgré l'erreur donnerait une identité arch sans
   corpus, c'est-à-dire un agent qui démarre et ne trouve rien.
2. Le rendu ne parse pas en `toml::Value`.
3. `[skills].allowlist` est absente **ou vide**. C'est le piège que le runbook
   §3b écrit en encadré : `apply_identity_allowlist` sort tôt sur une liste
   vide, donc `allowlist = []` signifie « aucun filtre », c'est-à-dire **toutes
   les skills**. Un outil de réparation qui peut écrire la configuration la plus
   permissive du système sans rien dire est pire que le geste manuel qu'il
   remplace.

Le refus nomme le fichier, la cause et le geste — jamais un code de sortie nu.

### D5 — Le prédicat de population vient du serveur, il n'est pas réécrit

`agent_exists` teste `config.toml` ; la population que le serveur **sert** est
`tier_guard::servable_agent_names` (`server/tier_guard.rs:211`) = l'union de
`list_agents` (qui exige `config.toml`) et de tout répertoire portant un
`identity.toml`. Les deux prédicats divergent **par conception**, et le verbe
répare précisément les répertoires où l'un des deux fichiers manque.

Donc : `servable_agent_names` passe de `pub(crate)` à `pub` et devient le
lecteur unique de « cet agent existe-t-il ? » pour ce verbe. Écrire un second
prédicat local est la duplication que `grooming_marker` (mika#2158) a dû
refermer une fois — deux lecteurs d'une même question finissent par répondre
différemment, et personne ne le voit le jour où ça arrive.

### D6 — Ce que le verbe ne fait pas, et ce que ça coûte

Rien sur `config.toml` (c'est là que vivent `llm_provider` / `*_model` d'un
opérateur, et mika#2330 documente que les écraser est le défaut que
`MIKA_DISABLE_AGENT_PROVISIONING` existe pour éviter). Rien en base. Aucun appel
à `seed_bundled_skills_if_needed` : re-matérialiser les symlinks est le travail
du prochain démarrage (note encadrée du runbook §3c), et l'élargir ici étendrait
le rayon de souffle au-delà des deux fichiers pour une opération que le
redémarrage refait de toute façon.

**Coût nommé, et il est plus faible qu'il n'y paraît :** `mika skills list
--agent <name>` applique l'allowlist d'identité **en direct depuis le disque**
(`mika-cli/src/commands/skills.rs:313`), donc il montre la frontière réparée
immédiatement, avant tout redémarrage. Ce qui attend le redémarrage, ce sont les
symlinks sous `~/.mika/agents/<name>/skills/`. La sortie distingue les deux,
sans quoi un opérateur lit un répertoire vide comme une réparation ratée.

### D7 — Aucune variable d'environnement nouvelle, aucun événement de journal nouveau

Le geste est synchrone, opérateur, en avant-plan : sa sortie **est** sa
télémétrie. Ajouter un `audit_events` supposerait d'ouvrir la base, ce que D6
refuse. La trace durable de la réparation est le fichier de sauvegarde horodaté
et, au redémarrage suivant, la disparition des lignes
`identity_toml_absent|unreadable|malformed` que le runbook §1 fait déjà grepper.

---

## 4. Implémentation

### 4.1 `crates/mika-agent/src/well_known_agents.rs`

- `render_identity_content` : `fn` → `pub fn`. Aucun changement de corps. C'est
  le rendu faisant autorité de l'identité d'un agent bien connu, y compris le
  chemin `Computed` de mika-arch ; le CLI doit l'appeler plutôt que d'en écrire
  un second.

### 4.2 `crates/mika-agent/src/server/tier_guard.rs`

- `servable_agent_names` : `pub(crate) fn` → `pub fn` (D5). Le doc-comment gagne
  une phrase disant qu'il est aussi le prédicat de population de
  `mika agents reprovision`, pour qu'un futur resserrage sache qui il casse.

### 4.3 `crates/mika-cli/src/cli.rs`

Nouvelle variante `AgentsCommand::Reprovision` :

```rust
/// Re-apply the authoritative identity (and persona) template for an agent
Reprovision {
    /// Agent name to re-provision
    name: String,
    /// Tier template to apply (customer agents only): default | family | champion.
    /// Defaults to MIKA_AGENT_TIER. Refused for well-known agents.
    #[arg(long)]
    tier: Option<String>,
    /// Re-apply identity.toml only, leaving soul.md untouched
    #[arg(long)]
    identity_only: bool,
    /// Show what would be written without writing anything
    #[arg(long)]
    dry_run: bool,
    /// Skip the typed-name confirmation
    #[arg(long, short)]
    yes: bool,
},
```

Le tier est une `Option<String>` et non un `ValueEnum` : `AgentTier::from_str`
n'existe pas aujourd'hui, `from_env` porte la règle (insensible à la casse,
trimée, **fail-closed** sur une valeur non reconnue — mika#2023 AC2). Un
`ValueEnum` clap dupliquerait cette règle et divergerait le jour où un quatrième
tier arrive. → §4.4, `resolve_tier_arg`.

### 4.4 `crates/mika-cli/src/commands/agents.rs`

Branche de `run()` + une fonction `reprovision` et ses aides :

1. **Normaliser et valider le nom** (`normalize_agent_name`, `validate_agent_name`).
2. **Population** — `mika_agent::server::tier_guard::servable_agent_names(&global_home)`
   contient-il le nom ? Sinon : refus nommant `mika agents create`.
3. **Résoudre la source** :
   - `find_well_known_agent(&name)` → `Some(spec)` : `--tier` présent ⇒ **erreur**
     (D2). Charger `Settings::load(&global_home)`, rendre
     `render_identity_content(spec, &settings)?`, persona = `spec.soul`.
   - `None` : `resolve_tier_arg(args.tier)?` — `Some(s)` pose `MIKA_AGENT_TIER`
     pour la durée de la résolution puis appelle `AgentTier::from_env()` afin que
     **la règle de mika#2023 soit lue une seule fois, à son site**, ou bien (variante
     préférée si elle est disponible sans `unsafe`) un petit `AgentTier::parse(&str)`
     extrait de `from_env` et réutilisé par lui. `edition 2024` exige `unsafe` pour
     `std::env::set_var` : **on extrait donc `parse`** et `from_env` l'appelle. Un
     lecteur, deux appelants.
     Identité = `tier.identity_toml()`, persona = `tier.soul_md()`.
4. **Valider le rendu** (D4) : parse TOML + `[skills].allowlist` non vide.
5. **Calculer le plan** : pour chacun des deux fichiers, `Absent` / `Identique` /
   `Diffère`. `--identity-only` retire `soul.md` du plan et arme
   l'avertissement de désaccord d'axes.
6. **Pré-digest** : agent, source (`well-known <name>` ou `tier <T> (via …)`),
   tableau par fichier, chemin des sauvegardes, et — si
   `tier.expects_family_provisioning()` — le bloc D3 sur l'environnement du
   service.
7. **`--dry-run`** : sortir ici, code 0.
8. **Confirmation** : saisie du nom, sauf `--yes` ; non-TTY sans `--yes` ⇒ refus
   (forme exacte de `reset`, `agents.rs:412-438`).
9. **Écrire** : pour chaque fichier qui diffère, sauvegarder en
   `<fichier>.bak.<YYYYMMDDTHHMMSSZ>` (`0600`) puis écrire en `.tmp` + `rename`
   (écriture atomique, même forme que `reconcile_well_known_identity`), puis
   `0600` sur la cible.
10. **Auto-contrôle** (R9) : `tier_guard::check_agent_tier_consistency(&agent_home,
    &name, resolved_tier)`. En `Err`, **ne pas revenir en arrière** — le disque est
    désormais cohérent avec le tier demandé, c'est le *process* qui ne l'est pas —
    mais afficher le message de la garde tel quel : il nomme déjà l'agent et le
    correctif.
11. **Sortie finale** : ce qui a été écrit, où sont les sauvegardes, et les trois
    gestes de vérification du runbook §3c (redémarrage, grep des trois événements,
    `mika skills list`), avec la distinction D6 entre la liste (immédiate) et les
    symlinks (au redémarrage).

Aides privées : `render_sources`, `plan_writes`, `validate_rendered_identity`,
`backup_path`, `write_atomic_0600`.

### 4.5 Documentation (R10)

- `docs/operator/agent-identity-reprovision.md` : §7 « Tooled path » réécrit —
  la commande devient le geste supporté, le §3 reste la voie de secours quand le
  binaire `mika` n'est pas disponible. §6 (changement de tier) pointe sur
  `--tier` et garde son encadré sur l'environnement du service. §1 gagne une
  ligne : pour un agent **bien connu**, une identité *présente mais dérivée* se
  répare par un simple redémarrage (mika#2330) — c'est l'absence qui demande ce
  verbe.
- `docs/configuration.md` : la mention `identity.toml` nomme la commande.
- `crates/mika-cli/CLAUDE.md` § *Key Commands* : une entrée, sur le modèle de
  `agents reset`.
- Root `CLAUDE.md` : la commande dans la liste, et une phrase sous le bloc
  `MIKA_AGENT_TIER` disant que la re-provision a désormais un verbe — le
  paragraphe « **What this does NOT retrofit** » y décrit littéralement la
  population que ce verbe sert (« it is a re-provisioning gesture »).

---

## 5. Contrat de vérification

Tests unitaires dans `crates/mika-cli/src/commands/agents.rs` (`#[cfg(test)] mod
tests`, `tempfile::TempDir`), sauf mention contraire.

### Population et refus

- **V1** — agent inexistant ⇒ `Err` nommant `mika agents create`.
- **V2** — agent avec `config.toml` mais **sans** `identity.toml` (la forme
  #2027) ⇒ accepté par le prédicat. *Contrôle négatif du choix D5 :
  `agent_exists` seul le voit aussi, mais un répertoire avec `identity.toml` et
  sans `config.toml` — servi par le serveur — ne serait pas vu ; V2b l'asserte.*
- **V2b** — agent avec `identity.toml` seul ⇒ accepté.
- **V3** — `--tier family` sur `mika-dev` ⇒ `Err` nommant le refus.
- **V4** — non-TTY sans `--yes` ⇒ `Err`, **et aucun fichier n'a changé**.

### Rendu et fail-closed (D4)

- **V5** — spec dont `render_identity_content` rend `Err` (mika-arch sans
  `MIKA_KG_DOCS_ROOTS`) ⇒ refus, `identity.toml` inchangé sur disque.
- **V6** — identité rendue avec `allowlist = []` ⇒ refus. *C'est le piège du
  runbook §3b ; sans ce test, l'outil peut écrire la configuration la plus
  permissive du système.*
- **V7** — identité rendue sans section `[skills]` ⇒ refus (même famille).

### Écriture

- **V8** — identité absente, persona intacte et identique au template ⇒
  `identity.toml` créé, **`soul.md` non réécrit**, **aucune sauvegarde de
  `soul.md`**. (Idempotence R5 — c'est le cas mesuré de #2027.)
- **V9** — les deux fichiers diffèrent ⇒ les deux écrits, deux sauvegardes,
  contenu des sauvegardes = contenu d'origine **octet pour octet**.
- **V10** — deux exécutions successives ⇒ la seconde n'écrit rien et ne
  sauvegarde rien.
- **V11** — `--identity-only` avec une persona divergente ⇒ `soul.md`
  strictement inchangé (mtime et contenu), et l'avertissement de désaccord
  d'axes est présent dans la sortie.
- **V12** — `--dry-run` ⇒ aucune écriture, aucune sauvegarde, sortie non vide.
- **V13** — permissions : les deux cibles et les deux sauvegardes sont en
  `0600` (`#[cfg(unix)]`).
- **V14** — `config.toml` est identique octet pour octet après un reprovision
  qui écrit les deux fichiers. *Contrôle négatif de D6 : sans lui, « ne touche
  pas config.toml » est une intention et pas une propriété.*

### Tier (D2/D3)

- **V15** — `--tier family` sur un agent client ⇒ l'identité écrite est
  `FAMILY_IDENTITY` et la persona `FAMILY_SOUL`, sentinelle
  `FAMILY_SOUL_MARKER` comprise.
- **V16** — après V15, `soul_has_family_marker` **et**
  `identity_allowlist_matches_family` rendent tous deux `true`. *C'est
  l'assertion qui atteste la rectification §1.3 : les deux axes de la garde
  mika#1962 s'accordent après un reprovision complet.*
- **V17** — après un `--identity-only --tier family` sur un agent opérateur, les
  deux axes **divergent** — assertion explicite du coût, pas de son absence.
- **V18** — `--tier zorglub` ⇒ résolu `Family` (règle fail-closed mika#2023
  AC2) et la sortie nomme la valeur entre guillemets. *Ce test existe pour que
  le drapeau hérite de la règle du tier au lieu d'en inventer une.*
- **V19** — `--tier` absent et `MIKA_AGENT_TIER` non posé ⇒ `Default`, et la
  provenance affichée est « défaut ».

### Structurel

- **V20** — `mika2230_le_tier_a_un_seul_analyseur` : scan de source refusant un
  second site qui compare `"family"` / `"champion"` littéralement hors de
  `AgentTier::parse`. *Un test comportemental ne peut pas voir cette classe : un
  second analyseur ne rendrait aucune décision fausse le jour où il est écrit,
  il divergerait au quatrième tier, en silence.*

### Non-régression

- **V21** — `cargo test -p mika-agent` : `render_identity_content` et
  `servable_agent_names` élargis ne cassent aucun appelant (changement de
  visibilité seul).
- **V22** — `make verify-bundled-skills`, `cargo clippy`, `cargo fmt --check`.

---

## 6. Definition of Done

- [ ] `mika agents reprovision <name>` existe, avec `--tier`, `--identity-only`,
      `--dry-run`, `--yes`.
- [ ] Les deux populations (bien connue / tier) sont servies par la même
      commande, via `find_well_known_agent`.
- [ ] `render_identity_content` et `servable_agent_names` sont `pub` ; aucun
      second lecteur de ces deux questions n'est introduit.
- [ ] `AgentTier::parse` est extrait et `from_env` l'appelle ; aucun second
      analyseur de tier.
- [ ] Les trois refus de D4 sont implémentés et testés.
- [ ] Sauvegardes horodatées `0600`, écriture atomique `.tmp` + `rename`.
- [ ] `config.toml`, la base et les symlinks de skills sont intacts — asserté.
- [ ] Le pré-digest nomme le tier résolu **et sa provenance**, et le bloc
      environnement-du-service quand le tier attend un provisionnement famille.
- [ ] Auto-contrôle `check_agent_tier_consistency` après écriture, sans retour
      arrière, message de la garde affiché tel quel.
- [ ] Les 22 points du §5 passent.
- [ ] Les quatre surfaces documentaires de R10 sont à jour, §7 du runbook
      compris.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria`. Les
critères ci-dessous sont dérivés de sa **Demande** (« une sous-commande CLI …
qui ré-applique le template du tier, respectant `MIKA_AGENT_TIER` + les
invariants tier (tier_guard mika#1962). Documenté dans docs/operator/ ») et du
§5 de ce plan.

- **AC1** — Une sous-commande `mika agents reprovision <name>` existe et
  ré-applique sur disque le template faisant autorité pour l'agent nommé, sans
  redémarrage préalable ni édition manuelle de constantes. *(V8, V9)*
- **AC2** — Elle respecte `MIKA_AGENT_TIER` : en l'absence de `--tier`, le tier
  appliqué est celui que `AgentTier::from_env()` résout, règle fail-closed
  mika#2023 comprise. *(V18, V19)*
- **AC3** — Elle respecte les invariants tier de mika#1962 : après un
  reprovision complet vers un tier famille, les **deux** axes de détection de la
  garde s'accordent ; l'auto-contrôle per-agent est exécuté et son verdict
  affiché. *(V15, V16, R9)*
- **AC4** — Elle sert aussi les agents **bien connus**, dont l'identité est
  rendue par la spec (chemin `Computed` de mika-arch compris) et refusée plutôt
  qu'écrite à moitié quand le rendu échoue. *(V5)*
- **AC5** — Elle ne peut pas élargir une frontière de skills en silence : une
  allowlist vide ou absente refuse l'écriture. *(V6, V7)*
- **AC6** — Elle est non destructrice et réversible : sauvegarde horodatée en
  `0600` de tout fichier écrasé, écriture atomique, idempotence sur ré-exécution.
  *(V9, V10, V13)*
- **AC7** — Elle est bornée : `config.toml`, la base et les symlinks de skills
  sont intacts. *(V14, D6)*
- **AC8** — Le chemin est documenté dans `docs/operator/` : le §7 « Tooled path »
  du runbook nomme la commande au lieu de dire qu'elle n'existe pas, et le §6
  (changement de tier) l'utilise. *(R10)*

---

## 8. Hors périmètre, délibérément

- **Le fail-closed lui-même** (mika#2027) : déjà livré, non retouché.
- **Le tier champion** (mika#2023) : `CHAMPION_PERSONA_PLACEHOLDER` n'est pas
  remplacé ici. `--tier champion` écrit ce que le code pose aujourd'hui
  (outils famille + persona placeholder famille) — ce verbe **applique** la
  décision produit, il n'en prend aucune.
- **La re-provision en masse** (`--all`, un sélecteur de tenants). La population
  mesurée est d'un tenant à la fois ; un geste de masse sur une écriture
  d'identité est un rayon de souffle qu'aucune mesure ne demande aujourd'hui.
- **Un sentinelle de provisionnement opérateur**, qui rendrait la direction
  inverse de D3 détectable. Réel, non mesuré, et il change la garde mika#1962 :
  ticket de suivi.
- **La retouche de `config.toml`** : c'est le fichier que
  `MIKA_DISABLE_AGENT_PROVISIONING` existe pour protéger (mika#2330).
- **Un rechargement à chaud** : `AgentState` met en cache le tier (mika#1962) et
  le registre de skills, et `startup::init_agent` charge l'identité une fois. Le
  redémarrage reste le contrat, et la sortie le dit au lieu de le laisser
  supposer.
- **La population « identité présente mais dérivée sur un agent bien connu » **
  reste servie par mika#2330 au redémarrage ; ce verbe la sert aussi, mais ce
  n'est pas lui qui la ferme.

---

## 9. Risques et haltes

- **Halte 1 — mika-spirit refuse de démarrer après un reprovision.** C'est
  `assert_family_tier_env_consistency` qui mord, et il nomme l'agent. **Ne pas
  supprimer le fichier qu'on vient d'écrire** : poser `MIKA_AGENT_TIER` dans
  l'environnement du **service** (pas dans un shell), ou restaurer la sauvegarde
  horodatée. Le verbe a fait exactement ce qu'on lui a demandé ; c'est la moitié
  environnement qui manque, et le CLI ne peut structurellement pas la lire.
- **Halte 2 — l'agent reste sans skills après redémarrage.** Le grep du runbook
  §1 (`identity_toml_absent|unreadable|malformed`) doit être vide. S'il ne l'est
  pas, ce n'est pas le verbe qu'il faut relancer — relire le §1, les trois
  événements ont trois remèdes différents et un second reprovision n'en traite
  qu'un.
- **Halte 3 — `~/.mika/agents/<name>/skills/` paraît vide juste après.** Attendu
  avant redémarrage (D6) : vérifier avec `mika skills list --agent <name>`, qui
  lit l'identité en direct. **Ne pas réinstaller de skills pour « réparer »
  ça** — l'encadré du runbook §3c le dit déjà, et le faire masque l'état réel.
- **Risque — une persona retouchée écrasée.** Borné par la sauvegarde
  horodatée et par `--identity-only`, et le pré-digest liste les fichiers qui
  vont changer avant la confirmation. La confirmation par saisie du nom existe
  pour cette raison précise.
- **Risque — la direction inverse de D3 (famille → défaut) n'est pas gardée.**
  Nommée dans le pré-digest, non détectable par le code aujourd'hui, ticket de
  suivi (§8).
