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

### B2 — La library dit quel binaire a produit son état (`.manifest-writer`)

`seed_bundled_skill_library` écrit un sidecar JSON `.manifest-writer` à côté de
`.manifest-hash` :

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
