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

**Il y a un second appel, et c'est celui de `startup.rs:80` qui porte la
garantie.** `seed_bundled_skill_library` rappelle `seed_support_dirs` à
`bundled_skills.rs:486`, c'est-à-dire **après** son retour anticipé de porte de
hash (`470-483`) : redondant sur le chemin d'extraction, absent sur le chemin de
confirmation. T4 tient donc par l'appel de `startup.rs:80` seul, jamais par
celui-ci. Dit ici parce que l'implémenteur de B2 pose son écriture dans cette
même fonction et doit savoir lequel des deux appels garantit quoi.

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
- **R5** — La library porte une trace durable du binaire qui a produit son état,
  lisible sans lancer de commande (`cat`), rafraîchie par **toute** passe de seed
  — y compris celle qui se contente de confirmer — et une régression de version
  est **dite**.
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

`seed_bundled_skills_if_needed` compose déjà, dans cet ordre : réparation des
variantes orphelines, création de la library, `seed_support_dirs`
inconditionnel, garde `disabled` (avec détection de drift),
`seed_bundled_skill_library` (gardé par hash, sync-shape, élagage des
orphelins), puis `materialize_agent_skill_links` avec l'allowlist d'identité.

**La précondition qui décide de la sûreté de cette bascule, vérifiée plutôt que
supposée : la cible des symlinks ne bouge pas.** L'appel actuel est *paramétré*
— `materialize_agent_skill_links(&library_dir, skills_dir, allowlist)`, où
`skills_dir` arrive par la signature d'`update_skills` — tandis que le composite
**recalcule** sa cible en interne (`agent_skills_dir = agent_home.join("skills")`,
`startup.rs:66`). Remplacer un appel paramétré par un appel auto-résolvant n'est
sûr que si les deux chemins coïncident, et ils coïncident **par construction** :
`commands::skills::run` pose `let skills_dir = agent_home.join("skills")`
(`skills.rs:25`), la même expression, à partir du même `agent_home` qu'il
transmet. Sans cette vérification la bascule pourrait déplacer les symlinks de
tous les skills bundled vers un autre répertoire — un défaut silencieux de la
même famille que celui qu'on ferme, et il n'aurait été découvert qu'en
production. **Corollaire pour l'implémenteur :** le paramètre `skills_dir` ne
devient pas mort et ne doit pas être supprimé — il reste lu plus bas par
`install::update_skill(agent_home, skills_dir, …)`, sur la branche marketplace.

**Une différence de comportement à nommer, pour qu'elle n'arrête pas
l'implémenteur : le layout legacy single-agent.** `seed_bundled_skills_if_needed`
retourne **avant** `materialize_agent_skill_links` quand `resolve_global_home`
(`startup.rs:129-137`) rend `is_multi_agent = false` — c'est-à-dire quand le
parent d'`agent_home` ne s'appelle pas `agents`. Le code actuel d'`update_skills`
appelle `materialize_agent_skill_links` **toujours**. La bascule supprime donc
cet appel dans ce seul layout, et **c'est correct** : en legacy la library *est*
le répertoire de skills de l'agent, donc la passe s'exécuterait contre elle-même
— le commentaire de `startup.rs:108-111` le dit dans ces termes. La topologie du
ticket (`~/.mika/agents/mika-arch/skills`) est multi-agent, donc le chemin du
défaut est inchangé.

Cette borne est en outre **plus étroite que sa formulation** : `commands::skills::run`
appelle `home::migrate_to_multi_agent(&global_home)?` (`skills.rs:23`) *avant* de
résoudre `agent_home`, donc sur ce chemin précis le layout est déjà migré quand
la question se pose. Et en legacy `resolve_agent_home` rend `global_home`
lui-même (`home.rs:344-350`), si bien que `skills_dir` et la library désignent
littéralement le même répertoire — le chemin confirme ce que rev 4 déduisait du
commentaire.

**Ce que la bascule ajoute par ailleurs, nommé plutôt que découvert.** Le
composite ouvre par `migrate_generated_variant_provider_dirs(agent_home)`
(`startup.rs:62`), qui **renomme physiquement** des répertoires de variantes
générées (mika#1663) — et, les dossiers de skills étant des symlinks vers la
library, ce renommage atterrit dans la library partagée (son doc-comment le dit,
`startup.rs:151-156`). `mika skills update` n'effectuait pas cette mutation
jusqu'ici. Elle est idempotente, warn-and-continue, et bornée aux répertoires
`generated/<provider>/` — mais c'est un élargissement réel du périmètre d'effet
de la commande, et un plan qui route par un composite doit énoncer tout ce que ce
composite fait, pas seulement la partie qu'il vient chercher.

**Nombre d'appelants, pour la même raison.** Le composite a **quatre** sites de
production (`init.rs:68`, `agents.rs:126`, `server/mod.rs:486`,
`create_agent.rs:119`) plus quatre appels dans
`tests/bundled_skill_library_e2e.rs`. C'est ce nombre qui justifie la lecture du
sidecar par le CLI plutôt qu'un élargissement de signature (voir B2, *Lecture par
le CLI*), et il n'a pas besoin d'être plus grand pour le justifier.

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

### B2 — La library dit quel binaire a produit son état (`.manifest-writer`)

`seed_bundled_skill_library` — dans **`crates/mika-agent/src/bundled_skills.rs`**,
à la racine de `src/`, *pas* sous `src/skills/` où la proximité avec le reste du
sous-système le ferait chercher — écrit un sidecar JSON `.manifest-writer` à côté
de `.manifest-hash` (`MANIFEST_HASH_FILE`, `bundled_skills.rs:32`) :

```json
{"version":"0.12.2","git_hash":"968dbe94","attested_at":"2026-09-18T09:14:02Z","manifest_hash":"a1b2c3d4e5f60718","extracted":true}
```

`version` et `git_hash` viennent de `mika_common::build_info` ; `attested_at` de
`crate::timestamp::now()`.

**Il est écrit sur TOUTES les passes de seed, y compris celle qui n'extrait
rien** — c'est la décision centrale de ce bloc, et elle est imposée par une
lecture du code plutôt que par goût. `seed_bundled_skill_library`
(`bundled_skills.rs:470-483`) **retourne tôt** quand le `.manifest-hash` présent
égale celui du binaire, et `compute_manifest_hash` (`bundled_skills.rs:427-439`)
ne hache **que** les noms, `content_hash` et chemins de fichiers des skills.
Deux binaires séparés par des semaines de commits Rust, sans changement sous
`skills/bundled/`, ont donc le **même** hash de manifeste.

Conséquence si le sidecar n'était écrit que sur le chemin d'extraction : un
opérateur qui vient de reconstruire et dont le PR ne touche aucun prompt bundled
lirait un `git_hash` antérieur sur une library pourtant parfaitement conforme —
et conclurait à un défaut de déploiement. **C'est le symptôme même du ticket,
retourné en faux positif.** Un instrument posé pour clore une lecture fausse ne
doit pas en ouvrir la réciproque.

D'où le sens exact du fichier, à écrire dans le code comme ici : *quel binaire a
produit l'état actuel de cette library, et quand l'a-t-il attesté.* Le champ
`extracted` distingue les deux passes (`true` : cette passe a réellement écrit du
contenu ; `false` : la porte de hash a confirmé la conformité sans réécrire).
Après `make deploy` suivi de n'importe quel seed, le sha inscrit est celui du
binaire déployé **dans les deux cas** — ce qui est précisément la propriété que
R4 et R5 demandent.

**Ordre d'écriture, et pourquoi il diffère de celui de `.manifest-hash`.** Sur le
chemin d'extraction, le sidecar est écrit **en dernier, après `.manifest-hash`**,
pour la raison déjà inscrite là : une extraction partiellement échouée ne doit pas
laisser une attestation qui masque un état périmé. Sur le chemin de confirmation,
il est écrit avant le retour anticipé — il n'y a rien à faire échouer.

**Écriture atomique (tmp + `rename`), au motif que deux processus écrivent.**
mika-spirit et le CLI `mika` peuvent semer en même temps (c'est T3) ; une
écriture en place exposerait un JSON tronqué à un lecteur concurrent, et le
lecteur de B3 est justement une seconde passe du CLI. Le motif tmp-dans-le-même-
répertoire-puis-`rename` est celui qu'emploient déjà `marketplace.rs:87`,
`oauth.rs:285` et `well_known_agents.rs:681`. Un échec d'écriture du sidecar est
un WARN, jamais un abandon du seed (même hiérarchie que V4 : le fait prime sur sa
trace).

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

**Seconde borne, qui tient au champ lui-même : `git_hash` peut valoir
`"unknown"`.** `build_info::GIT_HASH` (`build_info.rs:14-18`) est
`option_env!("GIT_HASH")` avec repli sur la chaîne `"unknown"` — le repli est le
cas d'un binaire construit hors checkout git (couche Docker, tarball source), et
son doc-comment l'énonce déjà. Trois conséquences, écrites ici pour qu'aucune ne
soit découverte en production :

- Sur le poste opérateur, où le geste est `make deploy` depuis le checkout, le
  sha est réel. **C'est le cas nominal du ticket et il est couvert.**
- En container, la ligne `attested by` affichera `mika 0.12.2 (unknown)`. Ce
  n'est pas un défaut de déploiement et la sonde V5 ne s'y applique pas : dire
  `unknown` est la réponse honnête, et la laisser lire comme une panne serait
  rouvrir la classe même que ce plan ferme. La documentation de B4 le dit d'une
  phrase.
- La garde `bundled_library_downgrade` **ne dépend pas de ce champ** : elle
  compare les `version`, et `VERSION` est `env!("CARGO_PKG_VERSION")`, toujours
  présent. Un sha `unknown` n'affaiblit donc pas la garde ; il n'affaiblit que la
  précision de l'attestation.

Aucun cas particulier n'est écrit dans le code pour `"unknown"` : le sidecar
enregistre la constante telle quelle. Une valeur sentinelle réécrite par
l'écrivain serait une seconde source de vérité sur la provenance du binaire.

**Le WARN atteint bien la surface que le ticket vise — vérifié, pas supposé.**
La garde ci-dessus n'a de valeur que si son `warn!` est rendu sur la commande
qu'un opérateur tape. `main.rs:222-238` : `suppress_stderr` ne couvre que `Chat`
et `Ask`, donc `mika skills` initialise `init_pretty` en `LogOutput::PrettyAndFile`
et le WARN part sur stderr en plus du fichier. Prescrire un signal sur une
commande qui n'installe pas de subscriber — ou qui filtre son propre niveau —
serait prescrire un signal invisible, c'est-à-dire reproduire en petit la classe
de défaut du ticket : un compte-rendu rassurant qui ne dit rien. La même
vérification vaut pour AC5.

**Lecture par le CLI, pas retour de fonction.** `seed_bundled_skills_if_needed`
rend `()` et a quatre sites de production. Élargir sa signature pour qu'un seul
imprime un résumé coûterait quatre sites pour un afficheur. Le CLI **relit**
`.manifest-writer` après l'appel et imprime ce qu'il y trouve — ce qui a en plus
la propriété d'être honnête : il rapporte le fait inscrit, pas une intention en
mémoire.

### B3 — La sortie de `update` cesse de parler du lien seul

```
  Refreshed bundled-skill library and symlinks.
    library: ~/.mika/skills
    manifest: a1b2c3d4e5f60718
    attested by: mika 0.12.2 (968dbe94) at 2026-09-18T09:14:02Z
```

La ligne `attested by` est celle qui répond à la question du ticket. Un opérateur
qui vient de fusionner #2339 et lit un sha antérieur a sa réponse dans la ligne
qu'il est déjà en train de lire, sans `diff` ni `stat`.

Le verbe est « attested », pas « written », **parce que c'est ce que le fichier
sait dire** : par B2 il est réécrit aussi quand la porte de hash confirme sans
extraire, et écrire « written by » là serait faux à la lettre. Le champ
`extracted` n'est **pas** imprimé — il sert au diagnostic et à V4 ; l'afficher
inviterait à lire `false` comme « rien n'a été fait », c'est-à-dire exactement la
confusion lien-contre-contenu que ce plan ferme.

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
- Dans les deux endroits, **une phrase sur `git_hash: "unknown"`** : c'est un
  binaire construit hors checkout git, pas un déploiement manqué (R-f).

---

## Verification contract

### V0 — Où vivent ces tests, et le harnais qui existe déjà

Les cinq vérifications ne portent pas sur la même unité, donc elles ne vivent pas
au même endroit — et l'une des deux maisons est déjà construite.

- **V1 et V1.5** portent sur `update_skills`, **privé au binaire** `mika-cli`.
  Un module `#[cfg(test)]` inline dans `crates/mika-cli/src/commands/skills.rs`
  est la surface minimale (`lib.rs` doit rester minimal par la consigne du
  crate) : aucun autre emplacement ne peut appeler la fonction.
- **V2, V3 et V4** portent sur le **semeur** (`mika-agent`), pas sur la commande.
  `crates/mika-agent/tests/bundled_skill_library_e2e.rs` existe déjà pour
  exactement cette unité : il fournit un helper `provision_agent(tmp, name,
  allowlist)` qui monte le layout multi-agent temporaire, et un test
  `second_seed_is_a_noop_via_hash_gate` qui exerce **déjà** le chemin de
  confirmation — c'est-à-dire le chemin dont V4 doit prouver qu'il rafraîchit
  malgré tout le sidecar. Y ajouter les cas plutôt que recréer un montage évite
  une seconde définition de « un home d'agent de test », qui est la même classe
  de duplication que B1 refuse côté production.

Cette répartition n'ajoute pas de fichier de test : elle en réutilise un et en
crée un module inline. Elle est nommée ici parce qu'un implémenteur qui suit V1 à
la lettre sans lire ce paragraphe écrirait les cinq au même endroit — et devrait
alors rendre `update_skills` publique pour trois tests qui ne l'appellent pas.

### V1 — Test d'intégration : la library converge (R1, R7)

Module `#[cfg(test)]` inline dans `crates/mika-cli/src/commands/skills.rs`
(voir V0 pour le motif).

1. Home multi-agents temporaire : `<tmp>/agents/<a>/identity.toml` avec une
   allowlist d'un skill bundled connu.
2. `seed_bundled_skills_if_needed` une fois → library peuplée.
3. **Périmer** : écrire `STALE` dans
   `<tmp>/skills/<skill>/system_prompt.md`, et `stale` dans `.manifest-hash`.
4. Appeler `update_skills(global_home, agent_home, skills_dir, None, None)`.
5. Asserter : le fichier de library **et** le fichier résolu via le symlink de
   l'agent sont revenus au contenu du manifeste du binaire.
6. Asserter que le symlink lu à l'étape 5 est bien celui de
   `<agent_home>/skills/<skill>` — c'est-à-dire du `skills_dir` passé à
   `update_skills`, et non d'un répertoire que le composite aurait recalculé
   ailleurs. C'est la précondition de B1 (*la cible des symlinks ne bouge pas*)
   rendue mesurable : sans elle, une bascule qui resynchroniserait correctement
   la library tout en matérialisant les liens dans un autre répertoire passerait
   les étapes 1 à 5 en laissant l'agent sur ses anciens liens. Un montage de test
   où les deux chemins coïncident déjà ne prouve rien tout seul — l'assertion
   doit nommer `skills_dir` explicitement.

**Fidélité de la simulation, dite explicitement.** L'étape 3 simule l'état
*post-reconstruction* — une library écrite par un manifeste antérieur à celui du
binaire courant — et non l'incident littéral (où le binaire CLI était lui-même
périmé et son hash cohérent). C'est le bon état à tester : c'est celui qu'un
`skills update` doit désormais réparer. L'incident littéral n'est pas réparable
par du code, il l'est par une reconstruction, et c'est B4 qui le dit.

**Pas de scan de source en plus, et voici pourquoi.** Une régression vers
`materialize_agent_skill_links` seule fait **échouer V1** : le fichier `STALE`
survit. C'est la différence avec les classes d'observabilité de ce dépôt (où une
régression ne rend aucune décision fausse et ne peut être vue que par un scan de
source) — ici le comportement bouge, donc un test de comportement suffit. Mais
cette phrase est une affirmation **sur** V1, faite dans la prose du plan ; la
mesurer demande un second test. C'est V1.5, et c'est tout ce que porte AC7.

### V1.5 — Test négatif : le symlink seul ne répare rien (AC7)

Test **séparé** de V1, même module. Montage identique jusqu'à l'étape 3 (library
périmée, `.manifest-hash` à `stale`), puis appel de
`materialize_agent_skill_links` **seule** — jamais `update_skills` — et
assertion que le contenu `STALE` **survit** des deux côtés (fichier de library et
fichier résolu via le symlink).

Ce qu'il épingle n'est pas `update_skills` : c'est le **pouvoir discriminant de
V1**. Il mesure que le composant vers lequel une régression retomberait est bien
incapable de produire le résultat que V1 exige. Sans lui, AC7 restait une
affirmation sur un test, invérifiable — la circularité relevée en première passe
(F2) : V1 ne peut pas être à la fois la preuve du correctif et la preuve de sa
propre sensibilité.

**Le cas qui le fait rougir légitimement, et c'est voulu.** Si
`materialize_agent_skill_links` apprenait un jour à réécrire le contenu de la
library, V1.5 rougirait alors que rien ne serait cassé. Ce n'est pas un faux
positif : c'est le seul signal possible que V1 a cessé d'être discriminant — V1
resterait vert en prouvant strictement moins qu'on ne croit. Le message d'échec
doit le dire dans ces termes, sinon le prochain lecteur le « réparera » en
supprimant le test.

*Citation : review-guide.md § Single Responsibility (un test, un invariant) —
V1 atteste le comportement, V1.5 atteste la sensibilité de V1.*

### V2 — `_shared/dispatch-lib.sh` (R2)

Dans `bundled_skill_library_e2e.rs` (V0), sur un home monté par
`provision_agent` : corrompre `<tmp>/skills/_shared/dispatch-lib.sh`, re-semer,
asserter le retour au contenu du manifeste. Couvre le second contournement du
ticket.

### V3 — `MIKA_DISABLE_BUNDLED_SKILLS` (R3)

Même fichier. Test asserant que sous `disabled = true` le contenu de skill
corrompu **survit**
(la garde est honorée) tandis que `_shared/` est **quand même** réécrit — la
composition exacte de `startup.rs:68-103`. Le flag est passé en paramètre, donc
le test ne mute aucun état global de processus.

### V4 — Écrivain et régression (R4, R5)

Même fichier que V2/V3 (V0). Le cas central ci-dessous est le jumeau de
`second_seed_is_a_noop_via_hash_gate`, qui y exerce déjà la porte de hash.

- `.manifest-writer` existe après un seed, parse en JSON, porte
  `build_info::VERSION` et `build_info::GIT_HASH`.
- **Le champ `git_hash` est asserté égal à la constante, jamais à un motif de
  sha.** `GIT_HASH` vaut `"unknown"` hors checkout git (`build_info.rs:14-18`),
  donc une assertion de forme (« 8 caractères hexadécimaux ») rougirait en CI
  container sans qu'aucun comportement soit cassé — un test qui échoue là où le
  produit est sain est un test qu'on finit par désarmer.
- **Le sidecar est rafraîchi par la passe qui n'extrait rien** (le cœur de B2).
  Semer une fois, altérer `attested_at` et `git_hash` dans le sidecar **sans
  toucher au contenu ni à `.manifest-hash`**, re-semer : le sidecar est revenu
  aux constantes du binaire et porte `extracted: false`, alors que le contenu des
  skills n'a pas été réécrit. Ce test est le seul qui distingue la conception
  retenue de celle qui produirait le faux positif décrit en B2 — sans lui, la
  variante « écrire seulement sur extraction » passerait tous les autres.
- Planter un `.manifest-writer` avec une `version` future → un seed émet
  `bundled_library_downgrade` et **écrit quand même**.
- Un `.manifest-writer` illisible ou malformé n'empêche pas le seed (fail-open :
  un sidecar d'observabilité ne doit jamais bloquer une écriture de contenu).
- Aucun fichier temporaire d'écriture atomique ne subsiste dans la library après
  un seed, et le sidecar n'est jamais observé tronqué.

### V5 — Sonde post-déploiement, avec sa halte

Après `make deploy` :

```bash
cat ~/.mika/skills/.manifest-writer          # le sha doit être celui qu'on vient de déployer
mika skills --agent mika-arch update         # même sha sur la ligne « attested by »
diff ~/.mika/agents/mika-arch/skills/mika-arch-groom-ticket/system_prompt.md \
     skills/bundled/mika-arch-groom-ticket/system_prompt.md   # vide
```

**Halte.** Si le sha de `.manifest-writer` est bien celui du HEAD déployé **et**
que le `diff` est non vide, le défaut n'est pas ici : il est dans la découverte
`build.rs` ou dans l'extraction. Ne pas relancer `update` — c'est la
reconstruction qu'il faut examiner.

**Condition d'applicabilité de cette sonde.** Elle compare un sha, donc elle
suppose un binaire construit dans un checkout git. Sur un binaire de container ou
de tarball, `git_hash` est `"unknown"` (B2) et **les deux premières lignes ne
décident rien** — seul le `diff` reste probant. Lire `unknown` comme un échec de
déploiement serait le faux positif symétrique de celui que ce plan ferme.

---

## Fire-Disposition

Requis par le Fire-Disposition Gate (mika#1574), soulevé par mika-arch en
première passe (F1). Ce plan porte deux livrables de classe détecteur — V1–V4
(et V1.5) d'un côté, la garde `bundled_library_downgrade` de B2 de l'autre. Ils
ne tirent pas sur la même population, donc la disposition est dite par livrable
plutôt qu'une fois pour le plan.

### V1–V4 et V1.5 — Option (a), allowlist nommée, aujourd'hui vide

Ces tests s'exécutent **intégralement sur un home temporaire que le test
fabrique** (`<tmp>/agents/…`, `<tmp>/skills/…`) et comparent au manifeste
compilé dans le binaire de test. Aucune donnée pré-existante du dépôt ni du poste
de l'opérateur n'entre dans leur population, donc aucune violation antérieure ne
peut être surfacée : **l'allowlist naît vide, et c'est un fait sur le montage du
test, pas une espérance sur les données.**

L'engagement est la moitié utile de l'option (a). Si l'implémentation découvre
malgré tout un échec — le cas plausible étant un skill bundled dont l'extraction
ne reproduit pas son manifeste à l'octet près — il est traité ainsi :

1. **Donnée nommée** — le skill exact, jamais une tolérance générale sur le
   prédicat. Un test rendu permissif pour passer aurait exactement la propriété
   que ce plan reproche à la ligne « Refreshed » : dire vert sans rien garantir.
2. **Ticket de suivi** déposé sur la cause d'extraction.
3. **Assertion auto-nettoyante** — l'entrée d'exception rougit quand le suivi se
   ferme, avec pour message « retirer cette entrée ».

L'exception vivrait dans `#[cfg(test)] mod tests`, jamais sur un chemin que le
semeur de production puisse consulter au runtime.

**Ce qui est explicitement refusé ici : l'option (b)** (atterrir sous
`#[ignore]`). V1 est le seul détecteur qui tienne R7 ; le désarmer laisserait
vivre précisément la régression pour laquelle il existe. L'option (c) ne
s'applique pas non plus : la forme de la résolution n'est pas une question de
cadrage opérateur, c'est un défaut d'extraction avec une réponse technique.

### `bundled_library_downgrade` — avertir et procéder, par conception

C'est le seul détecteur de ce plan qui tire sur des **données de production
réelles** : la library du poste de l'opérateur. Sa disposition est celle déjà
écrite en B2 et vaut ici comme fire-disposition — **il émet un WARN et écrit
quand même**. Ce n'est aucune des trois options canoniques, et pour une raison
structurelle : les trois supposent un détecteur dont le tir empêche quelque
chose. Celui-ci n'empêche rien, par décision. Un rollback délibéré est un geste
légitime, et une garde qui le refuserait serait un mode de panne pire que celui
qu'elle signale — même arbitrage que la garde mika#2293, où refuser de démarrer
sur un réglage sous-optimal mais fonctionnel coucherait la flotte.

Deux bornes de population, dites plutôt que découvertes à l'implémentation :

- **Au déploiement de ce correctif lui-même, la garde ne peut pas tirer.**
  `.manifest-writer` n'existe encore sur aucun poste ; une absence n'est pas une
  comparaison. Le premier seed l'écrit, et le premier tir possible est le
  suivant. Il n'y a donc pas de rafale de WARN à prévoir le jour du déploiement.
- **Un `.manifest-writer` illisible, malformé, ou sans `version` parsable ne
  tire pas et ne bloque pas** (V4). Un sidecar d'observabilité qui empêcherait
  une écriture de contenu inverserait la hiérarchie entre le fait et sa trace.

Répond aussi à S1 : c'est la ligne WARN qui porte la trace, et elle nomme les
deux versions, les deux sha et l'agent — la « donnée spécifique » que l'option
(a) exige d'une exception, portée ici par l'événement plutôt que par une entrée
d'allowlist, faute de population à exempter.

*Citation : review-guide.md § Fire-Disposition Gate (mika#1574) ;
`docs/solutions/best-practices/fire-disposition-doctrine.md`.*

---

## Definition of Done

- `mika skills update` (sans argument) resynchronise library + `_shared/` +
  symlinks via `seed_bundled_skills_if_needed`, et rien n'est recomposé sur place.
- `MIKA_DISABLE_BUNDLED_SKILLS` est lu via `Settings` et honoré à l'identique.
- `.manifest-writer` est écrit atomiquement par le semeur sur **toute** passe
  (extraction comme confirmation) et imprimé par la commande.
- Une régression de version émet `bundled_library_downgrade` sans refuser.
- V1, V1.5 et V2–V4 passent ; `cargo test`, `cargo clippy`, `cargo fmt --check`
  verts.
- `docs/skills.md` et la racine `CLAUDE.md` portent la chaîne de déploiement
  réelle.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` : il énonce
ses exigences sous « Requirements » et « Verification contract ». **La dérive de
gabarit est confirmée comme acceptée, sans gap fonctionnel** (F3) — et la
confirmation est rendue vérifiable plutôt que déclarative par la table de
traçabilité ci-dessous, où chaque critère remonte à une exigence du ticket ou est
nommé comme un dépassement assumé.

| AC | Exigence du plan | Origine dans le ticket |
|----|------------------|------------------------|
| AC1 | R1 | L'exigence centrale : `update` doit resynchroniser le contenu de la library (lecture T1) |
| AC2 | R2 | Le second contournement manuel décrit par le ticket : `_shared/dispatch-lib.sh` (T4) |
| AC3 | R3 | `MIKA_DISABLE_BUNDLED_SKILLS`, que le ticket demande de laisser honoré tel quel |
| AC4 | R4 | Le symptôme fondateur : la commande rend un compte-rendu que l'opérateur lit à faux |
| AC5 | R5 | **Dépassement assumé**, dérivé de la lecture T5 : le ticket ne nomme pas `.manifest-writer`, il pose la question (« cette library est-elle à jour ? ») à laquelle le hash seul ne sait pas répondre |
| AC6 | R6 | Verification contract du ticket, plus la correction T2 (la library est une projection du binaire) |
| AC7 | R7 | « Un test automatique échoue si `update` retombe sur le symlink seul » |

**Aucune exigence du ticket ne reste sans critère, et le seul critère qui dépasse
le ticket est nommé comme tel.** C'est la forme du gap que la Acceptance-Criteria
Gate cherche, et elle est vide dans les deux sens.

Deux précisions d'honnêteté. (a) Cette confirmation s'appuie sur la transcription
du corps du ticket faite en première passe de ce plan (§ *Le besoin*, T1–T5) : la
session de révision n'a pas de jeton GitHub et n'a pas pu relire le corps. Un
architecte de seconde passe, qui l'a sous les yeux, peut contredire une ligne de
la table d'un mot. (b) **Écrire ces critères dans le corps du ticket est un geste
GitHub, hors du périmètre content-only de cette révision** (`/mika-revise-plan`
interdit `gh issue edit`) ; il appartient au pas de grooming qui attache le plan
au ticket, s'il est jugé souhaitable.

*Citation : review-guide.md § Acceptance-Criteria Gate (mika#1559).*

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
  la version + le sha du binaire qui a attesté l'état — de sorte qu'une
  resynchronisation faite par un binaire périmé soit lisible comme telle.
- **AC5** — `~/.mika/skills/.manifest-writer` existe après tout seed et porte
  `version`, `git_hash`, `attested_at`, `manifest_hash`, `extracted` ; il est
  rafraîchi **y compris par une passe que la porte de hash court-circuite**, de
  sorte qu'un binaire reconstruit sans changement de skill n'affiche jamais un
  sha antérieur sur une library conforme ; un seed par un binaire de version
  strictement inférieure à celle inscrite émet `bundled_library_downgrade` (WARN)
  et procède. Attesté par V4.
- **AC6** — `docs/skills.md` et la racine `CLAUDE.md` § `make deploy` énoncent
  que la library est une projection du binaire, que la chaîne de déploiement
  d'un prompt bundled passe par une reconstruction, et nomment
  `.manifest-writer` comme sonde.
- **AC7** — Le pouvoir discriminant de V1 est **mesuré, pas affirmé** : un appel
  à `materialize_agent_skill_links` seule, sur la même library périmée, laisse le
  contenu `STALE` en place des deux côtés. Attesté par V1.5, test distinct de V1.
  *Corollaire, qui est la formulation initiale de ce critère :* un retour
  d'`update_skills` au seul symlink fait donc échouer V1 — mais c'est une
  conséquence de la mesure, plus une affirmation que V1 porterait sur lui-même
  (F2).

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
- **R-d — La ligne de sortie change de texte, et rien dans le dépôt ne la lit.**
  `Refreshed bundled-skill symlinks.` devient `Refreshed bundled-skill library
  and symlinks.`. Recherche faite sur `*.rs`, `*.sh`, `*.md`, `Makefile`,
  `.github/`, `scripts/` et `skills/` : **un seul producteur**
  (`crates/mika-cli/src/commands/skills.rs:1443`) et **aucun consommateur** —
  aucun script n'appelle `mika skills update` ni ne filtre sa sortie. La ligne
  n'est lue que par un humain, ce qui est exactement le défaut que B3 corrige.
  Répond à la vérification de compatibilité de format soulevée en première passe.
- **R-e — Le sidecar rapporte le _dernier_ attesteur, pas le plus récemment
  construit.** Écrire sur toute passe (B2) ferme le faux positif « sha ancien sur
  library conforme » dans le cas nominal, mais l'ouvre dans un cas rare et
  symétrique : un binaire `mika` périmé dont le manifeste de skills est identique
  à celui du binaire courant passe la porte de hash, n'altère **aucun** contenu,
  et inscrit pourtant son propre sha. La library reste juste, l'attestation
  recule. La garde `bundled_library_downgrade` le dit dès que la *version*
  diffère ; à version égale elle reste muette — c'est la même borne que R-a,
  héritée du fait qu'aucun ordre total sur les builds n'existe côté CLI. Le
  remède opérateur est celui de V5 : l'attestation se corrige en re-semant depuis
  le binaire attendu, et le champ `extracted: false` dit que rien n'a été
  réécrit entre-temps. **Ce n'est pas un échange de défaut mais une réduction :**
  le cas fermé est le geste nominal (reconstruire, puis lire), le cas ouvert
  demande un binaire périmé exécuté après le neuf, qui est déjà la situation que
  R-a déclare non fermée.
  Si un parseur hors dépôt existe, le changement lui apparaît comme un échec de
  correspondance franc, pas comme un silence — la bonne direction pour un défaut
  dont le sujet est précisément une phrase trop rassurante.
- **R-f — L'attestation est muette sur un binaire construit hors git.**
  `GIT_HASH` vaut alors `"unknown"` (`build_info.rs:14-18`) et la ligne
  `attested by` perd sa moitié discriminante, sans que rien ne soit cassé. La
  borne est **héritée, pas créée** : elle appartient à `build_info` depuis
  mika#2066 et vaut pour toute sonde de déploiement du dépôt. Elle est sans
  effet sur le geste du ticket (`make deploy` depuis le checkout) et sans effet
  sur la garde de régression, qui compare les `version`. Elle est écrite ici,
  dans B2 et dans la halte de V5 parce que l'unique danger qu'elle porte est de
  *se lire* comme le défaut : un opérateur en container qui prendrait `unknown`
  pour une preuve de non-déploiement referait le contournement `cp -f` du
  ticket.

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

---

## Revision history

- **rev 5 (2026-09-18)** — passe de préconditions d'implémentation. Aucune
  conception n'est changée, aucun critère d'acceptation affaibli ; cinq faits
  relus dans le code, dont un qui décide de la sûreté de B1 et que ni la rev 3
  ni la rev 4 n'avaient posé.
  - **B1 vérifie que la bascule ne déplace pas les symlinks.** L'appel actuel est
    *paramétré* (`materialize_agent_skill_links(&library_dir, skills_dir, …)`)
    et le composite **recalcule** sa cible (`agent_home.join("skills")`,
    `startup.rs:66`). La substitution n'est sûre que si les deux chemins
    coïncident : `skills.rs:25` pose exactement la même expression à partir du
    même `agent_home`. C'est la seule précondition du plan dont la fausseté
    produirait un défaut *silencieux* — library correctement resynchronisée,
    agent laissé sur ses anciens liens — donc la seule qui ne pouvait pas rester
    implicite. Rendue mesurable par une sixième assertion de V1, qui nomme
    `skills_dir` plutôt que de se reposer sur un montage où les deux chemins
    coïncident déjà. Corollaire ajouté pour éviter une suppression de bonne foi :
    `skills_dir` ne devient pas un paramètre mort (`install::update_skill` le lit
    encore, sur la branche marketplace).
  - **B2 vérifie que sa garde de régression est visible sur la surface visée.**
    `main.rs:222-238` : `suppress_stderr` ne couvre que `Chat` et `Ask`, donc
    `mika skills` initialise `LogOutput::PrettyAndFile` et le `warn!` de
    `bundled_library_downgrade` atteint stderr. Prescrire un WARN sur une
    commande qui n'installe pas de subscriber aurait reproduit en petit la classe
    du ticket : un compte-rendu qui ne dit rien. Vaut aussi pour AC5.
  - **T4 nomme le second appel à `seed_support_dirs`** (`bundled_skills.rs:486`),
    postérieur au retour anticipé de la porte de hash — donc redondant sur le
    chemin d'extraction et **absent** sur le chemin de confirmation. T4 tient par
    l'appel de `startup.rs:80` seul ; l'implémenteur de B2 pose son écriture dans
    cette même fonction et doit savoir lequel des deux garantit quoi.
  - **Le chemin exact du fichier est donné :**
    `crates/mika-agent/src/bundled_skills.rs`, à la racine de `src/` et non sous
    `src/skills/` où le sous-système le fait chercher.
  - **B1 nomme la mutation de disque que la bascule ajoute** —
    `migrate_generated_variant_provider_dirs` (`startup.rs:62`), un renommage
    physique de répertoires de variantes qui atterrit dans la library partagée.
    Idempotent, warn-and-continue, borné — mais réel, et un plan qui route par un
    composite doit énoncer tout ce que ce composite fait. La borne
    `is_multi_agent` de la rev 4 est par ailleurs confirmée **plus étroite**
    qu'annoncée : `skills.rs:23` migre le layout avant de résoudre `agent_home`,
    et en legacy `resolve_agent_home` rend `global_home` lui-même
    (`home.rs:344-350`), donc les deux répertoires sont littéralement le même.
- **rev 4 (2026-09-18)** — passe de fidélité au code. Aucun changement de
  conception, aucun critère d'acceptation affaibli ; trois écarts entre ce que le
  plan affirmait du code et ce que le code fait, tous relevés par relecture
  directe des fichiers cités.
  - **B1 nomme la borne `is_multi_agent`.** `seed_bundled_skills_if_needed`
    retourne **avant** `materialize_agent_skill_links` quand
    `resolve_global_home` (`startup.rs:129-137`) rend `false`, alors
    qu'`update_skills` appelle cette passe inconditionnellement aujourd'hui. La
    bascule la supprime donc en layout legacy single-agent — sans régression (la
    library *est* alors le répertoire de l'agent, l'appel s'exécuterait contre
    lui-même) et sans toucher la topologie du ticket, qui est multi-agent. Non
    dit, cet écart aurait arrêté l'implémenteur au moment précis où il compare
    l'ancien appel au nouveau. Le compte d'appelants est corrigé de « cinq » à
    **quatre sites de production**, ce qui justifie toujours la lecture du
    sidecar par le CLI plutôt qu'un élargissement de signature.
  - **B2 nomme la seconde borne du sidecar :** `build_info::GIT_HASH` vaut
    `"unknown"` hors checkout git (`build_info.rs:14-18`, repli documenté depuis
    mika#2066). Propagé en V4 (le test asserte l'égalité à la constante, jamais
    un motif de sha — une assertion de forme rougirait en CI container sur un
    produit sain), dans la halte de V5 (la sonde par sha n'y décide rien, seul le
    `diff` reste probant), en B4 (une phrase de documentation) et au risque
    **R-f**. La borne est héritée, pas créée, et sans effet sur la garde
    `bundled_library_downgrade`, qui compare les `version` — toujours présentes.
    Son seul danger est de *se lire* comme le défaut, ce qui ferait refaire le
    contournement `cp -f` du ticket.
  - **V0 ajouté : le harnais de test existe déjà.**
    `crates/mika-agent/tests/bundled_skill_library_e2e.rs` fournit
    `provision_agent` et un test `second_seed_is_a_noop_via_hash_gate` qui
    exerce exactement le chemin de confirmation dont V4 doit prouver qu'il
    rafraîchit tout de même le sidecar. V2/V3/V4 y vivent (ils portent sur le
    semeur, crate `mika-agent`) ; V1/V1.5 restent inline dans `mika-cli`
    (`update_skills` est privé au binaire). Sans cette répartition, un
    implémenteur suivant V1 à la lettre aurait écrit les cinq au même endroit et
    aurait dû rendre `update_skills` publique pour trois tests qui ne l'appellent
    pas.
- **rev 3 (2026-09-18)** — révision issue d'une relecture du code contre les
  affirmations du plan. Les cinq lectures T1–T5 sont confirmées à la ligne près
  (`skills.rs:1424-1443` n'appelle que `materialize_agent_skill_links` et
  `skills::run` ne passe pas par `init_base_for_agent` ; `startup.rs:68` appelle
  `seed_support_dirs` avant la garde `disabled` ; `all_bundled_skills` ne lit que
  la constante compilée ; `server/mod.rs:486` et `init.rs:68` sont bien les deux
  écrivains). **Un défaut de conception est en revanche apparu dans B2**, non
  relevé en première passe :
  - `seed_bundled_skill_library` **retourne tôt** quand `.manifest-hash`
    correspond (`bundled_skills.rs:470-483`), et `compute_manifest_hash`
    (`427-439`) ne hache que le contenu des skills. Écrire le sidecar uniquement
    sur le chemin d'extraction aurait donc laissé un `git_hash` antérieur sur une
    library parfaitement conforme dès que le PR déployé ne touche aucun prompt
    bundled — **le symptôme du ticket retourné en faux positif**, sur
    l'instrument même posé pour le clore.
  - B2 écrit désormais le sidecar sur **toute** passe de seed, avec un champ
    `extracted` distinguant extraction et confirmation, et `written_at` devient
    `attested_at` (le fichier ne peut plus prétendre décrire une écriture).
    Écriture atomique tmp + `rename`, au motif explicite des deux écrivains
    concurrents de T3, sur le motif déjà employé par `marketplace.rs:87`,
    `oauth.rs:285` et `well_known_agents.rs:681`.
  - Propagé en B3 (« attested by », et le refus argumenté d'imprimer
    `extracted`), R5, V4 (un cas de test dédié, seul à séparer la conception
    retenue de la variante fautive), AC5 et la Definition of Done.
  - **R-e** nomme la borne symétrique que ce choix ouvre — un binaire périmé au
    manifeste identique fait *reculer* l'attestation sans rien dégrader — et
    pourquoi c'est une réduction du défaut plutôt qu'un échange (le cas fermé est
    le geste nominal, le cas ouvert est déjà couvert par la non-fermeture
    déclarée en R-a).
  - Aucun critère d'acceptation n'est affaibli ; AC5 est renforcé d'une clause.
- **rev 2 (2026-09-18)** — révision adressant la première passe architecte
  (`Disposition: ITERATE`, findings F1–F3, sharpening S1 + vérification de
  compatibilité de format).
  - **F1 adressé** par l'ajout d'une section `## Fire-Disposition` qui traite
    séparément les deux populations de détecteurs : option (a) à allowlist vide
    pour V1–V4/V1.5 (montage intégralement en home temporaire, donc aucune
    violation pré-existante possible) avec l'engagement de nommage + suivi +
    assertion auto-nettoyante si l'implémentation en découvre une, et un refus
    argumenté de l'option (b) ; puis la disposition « avertir et procéder » de
    `bundled_library_downgrade`, seul détecteur tirant sur des données de
    production, avec ses deux bornes de population (le sidecar absent au
    déploiement ne peut pas tirer ; un sidecar illisible ne bloque pas).
    Citation : review-guide.md § Fire-Disposition Gate (mika#1574).
  - **S1 adressé dans le même mouvement** : l'émission du WARN y est documentée
    comme portant la « donnée spécifique » exigée par l'option (a), via
    l'événement plutôt qu'une entrée d'allowlist faute de population à exempter.
  - **F2 adressé** par la branche « test négatif séparé » que le finding laissait
    au choix : ajout de **V1.5**, qui appelle `materialize_agent_skill_links`
    seule sur la library périmée et asserte la survie de `STALE`. AC7 est
    reformulé pour porter cette mesure au lieu d'une affirmation de V1 sur
    lui-même, l'énoncé initial devenant un corollaire explicite. Le paragraphe
    correspondant de V1 est ajusté (« pas de scan de source » plutôt que « pas de
    garde structurelle ») et le cas où V1.5 rougit légitimement — si
    `materialize_agent_skill_links` apprenait à réécrire le contenu — est écrit
    comme le seul signal possible de la perte du pouvoir discriminant, pour qu'il
    ne soit pas « réparé » par suppression. Citations : review-guide.md § YAGNI,
    § Single Responsibility.
  - **F3 adressé** par la première branche du finding : la dérive de gabarit du
    corps du ticket est **confirmée acceptée, sans gap fonctionnel**, et la
    confirmation est rendue vérifiable par une table de traçabilité AC →
    exigence du plan → origine dans le ticket. AC5 y est nommé comme le seul
    dépassement assumé. Deux précisions d'honnêteté accompagnent la table : la
    session de révision n'a pas de jeton GitHub et s'appuie sur la transcription
    de première passe (contredisible d'un mot en seconde passe), et l'écriture
    des AC dans le corps du ticket est un geste GitHub hors du périmètre
    content-only de `/mika-revise-plan`. Citation : review-guide.md §
    Acceptance-Criteria Gate (mika#1559).
  - **Vérification de compatibilité de format adressée** par le risque **R-d** :
    recherche faite sur le dépôt (`*.rs`, `*.sh`, `*.md`, `Makefile`, `.github/`,
    `scripts/`, `skills/`) — un seul producteur de la ligne `Refreshed
    bundled-skill symlinks.` (`skills.rs:1443`) et **aucun consommateur**, aucun
    script n'appelant `mika skills update` ni ne filtrant sa sortie.
  - Cohérence : la Definition of Done nomme désormais V1.5.
  - Aucun critère d'acceptation n'a été affaibli ; AC7 est renforcé (une mesure
    remplace une affirmation) et aucun autre n'a changé de portée.
