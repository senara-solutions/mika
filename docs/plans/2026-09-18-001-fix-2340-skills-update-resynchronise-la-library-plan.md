# Plan — mika#2340 : `mika skills update` resynchronise la library bundled

- **Ticket :** senara-solutions/mika#2340
- **Priorité :** p2
- **Classe :** deploy-gap (DÉPLOYÉ≠EFFECTIF, silencieux)
- **Symptôme fondateur :** 2026-09-16, déploiement de #2339 — `mika skills --agent
  mika-arch update` rend « Refreshed bundled-skill symlinks. / Linked (no-op): 1 »
  pendant que le `system_prompt.md` résolu de mika-arch porte encore le texte du
  2026-09-10. Contourné à la main par `cp -f` du repo vers `~/.mika/skills/`.

---

## Le besoin, et ce que la lecture du code y déplace

Le ticket énonce trois choses. Deux sont exactes à la ligne près ; la troisième
est fausse, et c'est elle qui décide de la forme du correctif.

### T1 — La moitié exacte : `mika skills update` n'appelle jamais le semeur de library

`commands::skills::run` (`crates/mika-cli/src/commands/skills.rs:21-25`) résout
`global_home` / `agent_home` **directement** et ne passe pas par
`init::init_base_for_agent`, qui est la porte CLI où vit
`startup::seed_bundled_skills_if_needed` (`crates/mika-cli/src/init.rs:68`).

`update_skills` (`skills.rs:1424-1443`) appelle `materialize_agent_skill_links`
**seule**, puis imprime :

```
  Refreshed bundled-skill symlinks.
```

Une phrase sur le **lien**, lue par l'opérateur comme une phrase sur le
**contenu**. La suite du rapport (« Linked (no-op): 1 ») parle des skills
marketplace et renforce la lecture.

**Toutes les autres sous-commandes `mika` qui construisent un `AppContext` /
`DbContext` sèment la library** (`init.rs:68`) ; `skills` est la seule qui ne le
fait pas — et c'est celle que la procédure de déploiement nomme. Le trou est
exactement à l'endroit où le geste documenté le traverse.

### T2 — La moitié fausse : la library n'est PAS une projection du repo

Le ticket demande que `update` resynchronise « le contenu library ← repo
`skills/bundled/` ». **Aucun chemin d'exécution ne lit `skills/bundled/` sur
disque.** `crates/mika-agent/build.rs` parcourt cet arbre **à la compilation** et
génère `BUNDLED_SKILL_MANIFESTS` ; `seed_bundled_skill_library`
(`bundled_skills.rs:460`) extrait depuis cette constante embarquée.

Conséquence, et c'est la phrase qui gouverne tout le reste :

> **La library est une projection du binaire, pas du dépôt. Un `git pull` seul ne
> peut changer ce qu'aucun binaire en cours d'exécution est capable d'écrire.**

Le geste qui rafraîchit un prompt bundled est donc une **reconstruction**
(`make deploy` : build + install + restart), et le semeur est ce qui la porte sur
le disque. `docs/skills.md:930` le dit déjà correctement — « re-synced from
compiled-in templates on every startup » — mais rien ne relie cette phrase à la
commande que l'opérateur tape.

Cela réinterprète le symptôme fondateur sans le contredire : le fichier daté du
09-10 n'est pas un fichier que `update` a refusé de rafraîchir depuis le repo,
c'est un fichier que le binaire `mika` installé — lui-même du 09-10 — aurait
réécrit à l'identique s'il avait seulement appelé le semeur. **Les deux causes
sont réelles et se composent :** la commande ne sème pas (T1), et même si elle
semait, elle sèmerait ce que son binaire porte (T2).

### T3 — Le danger que le correctif ne doit pas créer : la resynchronisation-régression

Le contrat d'écriture de la library est unique (`seed_bundled_skill_library`,
gardé par le hash `.manifest-hash`), mais les **processus écrivains sont deux** :
mika-spirit au démarrage (`server/mod.rs:486`) et le CLI `mika` via
`init_base_for_agent`. Aucun des deux ne connaît le build de l'autre, et la porte
de hash n'est pas un ordre : elle compare « le hash inscrit » à « mon manifeste »
et réécrit dès qu'ils diffèrent — **dans les deux sens**.

Donc un binaire `mika` plus ancien que le mika-spirit en cours extrait son propre
manifeste et enregistre son hash, dégradant silencieusement les prompts de tous
les agents. Ce trou existe déjà aujourd'hui (un `mika status` avec un CLI périmé
suffit) ; faire écrire `skills update` l'élargit et, surtout, **l'installe dans
le geste documenté**. Un correctif qui remplacerait « Refreshed » par
« Resynced ! » sans rien d'autre échangerait un silence contre un plus joli.

### T4 — Le second contournement du ticket est couvert par le même appel

Le ticket a dû recopier `~/.mika/skills/_shared/dispatch-lib.sh` à la main.
`seed_bundled_skills_if_needed` appelle `seed_support_dirs(&library_dir)`
**inconditionnellement** (`startup.rs:80`) — avant le retour anticipé
`disabled`, et hors de la porte de hash. Router `update` par la fonction
canonique couvre donc `_shared/` sans second mécanisme.

### T5 — Ce que le hash sait dire, et ce qu'il ne sait pas dire

`.manifest-hash` répond à « cette library correspond-elle au binaire qui l'a
écrite en dernier ? » — tautologiquement oui juste après une écriture. La
question de l'opérateur est « cette library correspond-elle au commit que je
viens de tirer ? », et **seule la provenance de l'écrivain peut l'approcher**.

`mika_common::build_info` (mika#2066) existe précisément pour ça : `GIT_HASH` et
`VERSION` sont injectés à la compilation « so a deploy is verified by
interrogating the binary, not only by reasoning about provenance ». Rien ne les
inscrit à côté de la library, et rien ne les imprime sur le chemin `skills`.

---

## Requirements

- **R1** — `mika skills update` (sans nom de skill) resynchronise le contenu de
  la library bundled depuis le manifeste du binaire, avant de rafraîchir les
  symlinks par agent.
- **R2** — Le même appel rafraîchit les répertoires de support (`_shared/`,
  donc `dispatch-lib.sh`), y compris sous `MIKA_DISABLE_BUNDLED_SKILLS`.
- **R3** — `MIKA_DISABLE_BUNDLED_SKILLS` est honoré sur ce chemin exactement
  comme au démarrage, **lu via `Settings`** et jamais ré-interprété localement.
- **R4** — La sortie de la commande nomme ce qui a été fait, et **quel binaire**
  l'a fait : impossible de lire « à jour » d'une resynchronisation opérée par un
  binaire périmé.
- **R5** — La library porte une trace durable de son écrivain, lisible sans
  lancer de commande (`cat`), et une régression de version est **dite**.
- **R6** — La documentation nomme la chaîne réelle de déploiement d'un prompt
  bundled, et dit que `mika skills update` n'est pas, seule, cette chaîne.
- **R7** — Un test automatique échoue si `update` retombe sur le symlink seul.

---

## Conception

### B1 — `update_skills` appelle le composite canonique, jamais les morceaux

Dans `update_skills` (`crates/mika-cli/src/commands/skills.rs:1438-1443`),
remplacer l'appel isolé à `materialize_agent_skill_links` par :

```rust
let settings = mika_common::config::Settings::load_for_agent(global_home, agent_home).ok();
let disabled = settings.as_ref().is_some_and(|s| s.disable_bundled_skills);
mika_agent::startup::seed_bundled_skills_if_needed(agent_home, disabled);
```

`seed_bundled_skills_if_needed` compose déjà, dans cet ordre : création de la
library, `seed_support_dirs` inconditionnel, garde `disabled` (avec détection de
drift), `seed_bundled_skill_library` (gardé par hash, sync-shape, élagage des
orphelins), puis `materialize_agent_skill_links` avec l'allowlist d'identité.

**Pourquoi le composite et pas les deux appels côte à côte.** Recomposer ici
produirait une seconde définition de « que veut dire rafraîchir les skills
bundled », libre de diverger de la première au prochain changement — la classe
que ce dépôt a déjà dû défaire deux fois (`grooming_marker`, mika#2158 ; la
requête morte du supersede, mika#2335). La règle y est écrite : *un résolveur
écrit une seconde fois est un résolveur qui peut contredire le premier.*

**Pourquoi `Settings` et pas `std::env::var`.** Lire `MIKA_DISABLE_BUNDLED_SKILLS`
à la main rouvrirait la table de vérité divergente mesurée par mika#2220
(`MIKA_LOG_LLM_BODIES=True` armait le démon et était un no-op silencieux côté
CLI). `skills.rs:288` charge déjà `Settings::load_for_agent` pour le token git :
le chargeur est à portée, il n'y a pas de dépendance nouvelle.

**Échec de chargement → `disabled = false` + WARN.** C'est le défaut de
production, et refuser de rafraîchir parce que la config est illisible
rétablirait exactement le silence qu'on ferme.

**Portée : `name.is_none()` seulement.** `mika skills update <nom>` vise un skill
marketplace nommé ; la garde `if name.is_none()` déjà présente reste, inchangée.

### B2 — La library dit qui l'a écrite (`.manifest-writer`)

`seed_bundled_skill_library` écrit, **au même endroit et au même moment** que
`.manifest-hash` (donc en dernier, pour la raison déjà écrite là : une extraction
partiellement échouée ne doit pas masquer un état périmé), un sidecar JSON
`.manifest-writer` :

```json
{"version":"0.12.2","git_hash":"968dbe94","written_at":"2026-09-18T09:14:02Z","manifest_hash":"a1b2c3d4e5f60718"}
```

`version` et `git_hash` viennent de `mika_common::build_info` ; `written_at` de
`crate::timestamp::now()`.

**Garde de régression (R5).** Avant d'écrire, si le `.manifest-writer` présent
porte une `version` sémantique **strictement supérieure** à celle du binaire qui
écrit, émettre un WARN `bundled_library_downgrade` nommant les deux versions, les
deux sha, et l'agent. **Elle ne refuse pas** : un rollback délibéré est un geste
légitime, et une garde qui bloquerait un rollback serait un mode de panne pire
que celui qu'elle signale (le précédent est écrit noir sur blanc pour la garde
mika#2293 : refuser de démarrer sur un réglage sous-optimal coucherait la flotte).

**Borne assumée, dite plutôt que cachée :** cette garde ne voit **pas** une
régression à version égale (deux builds différents de `0.12.2`). Elle attrape la
classe qui traverse une release et laisse passer celle qui ne la traverse pas.
Un ordre total sur les commits n'existe pas côté CLI — il faudrait interroger un
dépôt que le binaire, lancé depuis `~/.local/bin` avec un CWD arbitraire, n'a
aucun moyen fiable de localiser.

**Lecture par le CLI, pas retour de fonction.** `seed_bundled_skills_if_needed`
rend `()` et a cinq appelants. Élargir sa signature pour qu'un seul imprime un
résumé coûterait cinq sites pour un afficheur. Le CLI **relit**
`.manifest-writer` après l'appel et imprime ce qu'il y trouve — ce qui a en plus
la propriété d'être honnête : il rapporte le fait inscrit, pas une intention en
mémoire.

### B3 — La sortie de `update` cesse de parler du lien seul

```
  Refreshed bundled-skill library and symlinks.
    library: ~/.mika/skills
    manifest: a1b2c3d4e5f60718
    written by: mika 0.12.2 (968dbe94) at 2026-09-18T09:14:02Z
```

La ligne `written by` est celle qui répond à la question du ticket. Un opérateur
qui vient de fusionner #2339 et lit un sha antérieur a sa réponse dans la ligne
qu'il est déjà en train de lire, sans `diff` ni `stat`.

Sous `MIKA_DISABLE_BUNDLED_SKILLS`, la sortie le dit explicitement plutôt que
d'afficher un couple manifeste/écrivain périmé sans commentaire.

### B4 — Documentation (R6)

- `docs/skills.md` — un bloc court **« Deploying a change to a bundled skill »**
  à côté de § *Customizing Built-in Skills* : la library est une projection du
  **binaire** ; la chaîne est `edit → make deploy → vérifier .manifest-writer` ;
  `mika skills update` porte la projection du binaire installé sur le disque et
  ne remplace pas la reconstruction.
- Racine `CLAUDE.md` § `make deploy` — une phrase : un changement de
  `skills/bundled/**` n'est effectif qu'après reconstruction, et
  `~/.mika/skills/.manifest-writer` est la sonde de vérification.

---

## Verification contract

### V1 — Test d'intégration : la library converge (R1, R7)

Module `#[cfg(test)]` inline dans `crates/mika-cli/src/commands/skills.rs`
(`update_skills` est privé au binaire ; un test inline est la surface minimale —
`lib.rs` doit rester minimal par la consigne du crate).

1. Home multi-agents temporaire : `<tmp>/agents/<a>/identity.toml` avec une
   allowlist d'un skill bundled connu.
2. `seed_bundled_skills_if_needed` une fois → library peuplée.
3. **Périmer** : écrire `STALE` dans
   `<tmp>/skills/<skill>/system_prompt.md`, et `stale` dans `.manifest-hash`.
4. Appeler `update_skills(global_home, agent_home, skills_dir, None, None)`.
5. Asserter : le fichier de library **et** le fichier résolu via le symlink de
   l'agent sont revenus au contenu du manifeste du binaire.

**Fidélité de la simulation, dite explicitement.** L'étape 3 simule l'état
*post-reconstruction* — une library écrite par un manifeste antérieur à celui du
binaire courant — et non l'incident littéral (où le binaire CLI était lui-même
périmé et son hash cohérent). C'est le bon état à tester : c'est celui qu'un
`skills update` doit désormais réparer. L'incident littéral n'est pas réparable
par du code, il l'est par une reconstruction, et c'est B4 qui le dit.

**Pas de garde structurelle en plus, et voici pourquoi.** Une régression vers
`materialize_agent_skill_links` seule fait **échouer V1** : le fichier `STALE`
survit. C'est la différence avec les classes d'observabilité de ce dépôt (où une
régression ne rend aucune décision fausse et ne peut être vue que par un scan de
source) — ici le comportement bouge, donc un test de comportement suffit.

### V2 — `_shared/dispatch-lib.sh` (R2)

Dans le même test : corrompre `<tmp>/skills/_shared/dispatch-lib.sh`, relancer,
asserter le retour au contenu du manifeste. Couvre le second contournement du
ticket.

### V3 — `MIKA_DISABLE_BUNDLED_SKILLS` (R3)

Test asserant que sous `disabled = true` le contenu de skill corrompu **survit**
(la garde est honorée) tandis que `_shared/` est **quand même** réécrit — la
composition exacte de `startup.rs:68-103`. Le flag est passé en paramètre, donc
le test ne mute aucun état global de processus.

### V4 — Écrivain et régression (R4, R5)

- `.manifest-writer` existe après un seed, parse en JSON, porte
  `build_info::VERSION` et `build_info::GIT_HASH`.
- Planter un `.manifest-writer` avec une `version` future → un seed émet
  `bundled_library_downgrade` et **écrit quand même**.
- Un `.manifest-writer` illisible ou malformé n'empêche pas le seed (fail-open :
  un sidecar d'observabilité ne doit jamais bloquer une écriture de contenu).

### V5 — Sonde post-déploiement, avec sa halte

Après `make deploy` :

```bash
cat ~/.mika/skills/.manifest-writer          # le sha doit être celui qu'on vient de déployer
mika skills --agent mika-arch update         # même sha sur la ligne « written by »
diff ~/.mika/agents/mika-arch/skills/mika-arch-groom-ticket/system_prompt.md \
     skills/bundled/mika-arch-groom-ticket/system_prompt.md   # vide
```

**Halte.** Si le sha de `.manifest-writer` est bien celui du HEAD déployé **et**
que le `diff` est non vide, le défaut n'est pas ici : il est dans la découverte
`build.rs` ou dans l'extraction. Ne pas relancer `update` — c'est la
reconstruction qu'il faut examiner.

---

## Definition of Done

- `mika skills update` (sans argument) resynchronise library + `_shared/` +
  symlinks via `seed_bundled_skills_if_needed`, et rien n'est recomposé sur place.
- `MIKA_DISABLE_BUNDLED_SKILLS` est lu via `Settings` et honoré à l'identique.
- `.manifest-writer` est écrit par le semeur et imprimé par la commande.
- Une régression de version émet `bundled_library_downgrade` sans refuser.
- V1–V4 passent ; `cargo test`, `cargo clippy`, `cargo fmt --check` verts.
- `docs/skills.md` et la racine `CLAUDE.md` portent la chaîne de déploiement
  réelle.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés des Requirements et du Verification contract.

- **AC1** — Partant d'une library dont le contenu d'un skill bundled diverge du
  manifeste du binaire, `mika skills --agent <a> update` rend le fichier résolu
  via `~/.mika/agents/<a>/skills/<skill>/system_prompt.md` identique au contenu
  du manifeste. Attesté par V1.
- **AC2** — Le même appel rafraîchit `~/.mika/skills/_shared/dispatch-lib.sh`.
  Attesté par V2.
- **AC3** — Sous `MIKA_DISABLE_BUNDLED_SKILLS=true`, l'écriture de contenu de
  skill est supprimée et `_shared/` est tout de même semé ; la valeur est lue via
  `Settings`, aucune ré-interprétation locale de la variable n'est introduite.
  Attesté par V3 + revue.
- **AC4** — La sortie de la commande nomme la library, le hash de manifeste, et
  la version + le sha du binaire qui a écrit — de sorte qu'une resynchronisation
  faite par un binaire périmé soit lisible comme telle.
- **AC5** — `~/.mika/skills/.manifest-writer` existe après tout seed et porte
  `version`, `git_hash`, `written_at`, `manifest_hash` ; un seed par un binaire
  de version strictement inférieure à celle inscrite émet
  `bundled_library_downgrade` (WARN) et procède. Attesté par V4.
- **AC6** — `docs/skills.md` et la racine `CLAUDE.md` § `make deploy` énoncent
  que la library est une projection du binaire, que la chaîne de déploiement
  d'un prompt bundled passe par une reconstruction, et nomment
  `.manifest-writer` comme sonde.
- **AC7** — Un retour de `update_skills` au seul `materialize_agent_skill_links`
  fait échouer V1.

---

## Risques et hors périmètre

### Risques

- **R-a — La resynchronisation-régression n'est pas fermée, elle est rendue
  lisible.** Deux processus écrivent la library et rien ne les ordonne. B2
  prévient sur une régression de version et reste muet sur une régression à
  version égale. Fermer la classe demanderait un verrou ou un ordre total sur
  les builds : un autre ticket, avec sa propre mesure.
- **R-b — La commande ne peut pas dire « à jour par rapport au dépôt ».** Elle
  dit quel binaire a écrit ; c'est à l'opérateur de comparer ce sha au HEAD qu'il
  vient de tirer. Faire mieux supposerait que le CLI sache localiser « le
  dépôt », ce qu'il ne sait pas depuis `~/.local/bin`.
- **R-c — Un prompt peut rester périmé pour une cause en amont** (découverte
  `build.rs`, extraction). V5 porte la halte correspondante : ne pas relancer
  `update` en boucle.

### Hors périmètre, délibérément

- **Une sous-commande `mika skills sync-library`.** Le ticket la propose en
  alternative (« soit… soit… »), pas en supplément. Livrer les deux donnerait
  deux surfaces pour un geste et laisserait `update` comme piège pour quiconque
  ne connaît pas la nouvelle. Le correctif répare la commande que la procédure
  nomme déjà.
- **Faire lire `skills/bundled/` au runtime.** Ce serait renverser le modèle
  compile-time de `build.rs` (et le contrat « engine-coupled = lockstep avec le
  moteur » qui en dépend) pour un défaut de déploiement. Hors sujet.
- **La sémantique de `MIKA_DISABLE_BUNDLED_SKILLS`.** Honorée telle quelle.
- **Le trou pré-existant `mika status` (et tout appelant d'`init_base_for_agent`)
  qui sème déjà la library sans rien dire.** B2 le rend lisible a posteriori via
  `.manifest-writer` ; décider si ces chemins doivent aussi *imprimer* quelque
  chose est une question de surface CLI, pas de ce défaut.
